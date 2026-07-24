# Agent Trace Database Adapter

`cli/src/services/agent_trace_db/mod.rs` defines the shared Agent Trace insert payloads and helpers consumed by the repository-scoped adapter. `RepositoryAgentTraceDb` (see [Repository-scoped adapter seam](#repository-scoped-adapter-seam)) is the sole Agent Trace DB adapter; the former checkout-scoped `AgentTraceDb` / `AgentTraceDbSpec` type, its `open_at` / `open_for_hooks_without_migrations` constructors, and its 15-file migration chain were removed by the `retire-legacy-agent-trace-db` plan (see [context/decisions/2026-07-17-retire-legacy-agent-trace-db.md](../decisions/2026-07-17-retire-legacy-agent-trace-db.md)).

## Shared insert/query payloads

`mod.rs` owns the typed payloads and SQL constants that `RepositoryAgentTraceDb` delegates to:

- `ensure_schema_ready_for_hooks()`: non-mutating hook-readiness check that delegates to the shared `TursoDb::ensure_schema_ready()` method with the Agent Trace–specific `AGENT_TRACE_SCHEMA_SETUP_GUIDANCE` constant (`"Run 'sce setup'."`); verifies the repository Agent Trace DB has the expected applied migration metadata in `__sce_migrations` for every ID in `AGENT_TRACE_REPOSITORY_MIGRATIONS`; missing/incomplete metadata fails with `Run 'sce setup'.` guidance instead of running migrations.
- `DiffTraceInsert<'a>`: insert payload with `time_ms: i64`, `session_id: &'a str`, `patch: &'a str`, `model_id: Option<&'a str>`, `tool_name: &'a str`, nullable `tool_version: Option<&'a str>`, and `payload_type: &'a str` (using `PAYLOAD_TYPE_PATCH` or `PAYLOAD_TYPE_STRUCTURED` constants).
- `PAYLOAD_TYPE_PATCH` / `PAYLOAD_TYPE_STRUCTURED`: string constants (`"patch"` / `"structured"`) for the `diff_traces.payload_type` discriminator column; `OpenCode` normalized diff-trace payloads use `patch`, `Claude` structured `PostToolUse` payloads use `structured`.
- `insert_diff_trace()`: domain-specific insert helper using parameterized SQL.
- `RecentDiffTracePatches`: parsed recent `diff_traces` query result containing valid parsed patches plus skipped-row reports.
- `recent_diff_trace_patches(cutoff_time_ms, end_time_ms)`: chronological `diff_traces` read helper for rows in the inclusive window `time_ms >= cutoff_time_ms AND time_ms <= end_time_ms`; parses raw patch text through `parse_patch` and skips malformed rows without failing the query.
- `PostCommitPatchIntersectionInsert<'a>`: insert payload for post-commit intersection results with commit metadata, window bounds, loaded/skipped counts, and serialized patch JSON.
- `insert_post_commit_patch_intersection()`: domain-specific insert helper using parameterized SQL.
- `AgentTraceInsert<'a>`: insert payload for built Agent Trace rows with `commit_id`, `commit_time_ms`, serialized `trace_json`, `agent_trace_id`, non-null `url`, and required `remote_url: &'a str` (Rust-API-only; DB column stays nullable).
- `insert_agent_trace()`: domain-specific insert helper for `agent_traces` using parameterized SQL.
- `MessageRole` enum: `User` / `Assistant` — maps to `messages.role` DB constraint.
- `InsertMessageInsert`: owned payload struct with insertable parent `messages` columns (`session_id`, `message_id`, `role`, `generated_at_unix_ms`); message body text belongs to `parts.text`, not the parent message row.
- `INSERT_MESSAGE_SQL`: parameterized single-row SQL using `INSERT ... ON CONFLICT (session_id, message_id) DO NOTHING` — leverages the unique index `idx_messages_session_message` so duplicate parent-message events remain non-failing without mutating the existing row.
- `insert_message(input)`: typed single-row helper that executes the duplicate-ignore parent-message insert; retained as part of the adapter surface.
- `insert_messages(inputs)`: typed batch helper that generates and executes one parameterized multi-row `messages` insert for valid conversation-trace `message` batches while preserving duplicate-ignore semantics.
- `PartType` enum: `Text` / `Reasoning` / `Patch` / `Question` — serializes known conversation part kinds for typed inserts. `parts.type` is stored as `TEXT NOT NULL` without a database-level enum `CHECK` constraint.
- `InsertPartInsert`: owned payload struct with `part_type`, `text`, `session_id`, `message_id`, and `generated_at_unix_ms`.
- `INSERT_PART_SQL`: parameterized single-row append-only INSERT into `parts` (no upsert; multiple rows per `(session_id, message_id)` allowed).
- `insert_part(input)`: typed single-row helper that inserts a part row without requiring a matching `messages` row (supports out-of-order writes); retained as part of the adapter surface.
- `insert_parts(inputs)`: typed batch helper that generates and executes one parameterized multi-row append-only `parts` insert for valid conversation-trace `message.part` batches.
- `lifecycle.rs`: service lifecycle provider for setup/doctor integration.

## Repository-scoped adapter seam

`cli/src/services/agent_trace_db/repository.rs` defines the repository-scoped Agent Trace DB adapter introduced by the `repository-scoped-agent-trace-db` plan:

```rust
pub type RepositoryAgentTraceDb = TursoDb<RepositoryAgentTraceDbSpec>;
```

This adapter has no canonical `DbSpec::db_path()`; callers must resolve `<state_root>/sce/repos/<repository-id>/agent-trace.db` first and use explicit-path `TursoDb` constructors. Its migration list is `generated_migrations::AGENT_TRACE_REPOSITORY_MIGRATIONS`, currently one fresh multi-statement SQL file at `cli/migrations/agent-trace-repository/001_repository_schema.sql`. The schema includes `repository_metadata` plus the existing repository-level Agent Trace tables, indexes, and triggers, and intentionally has no `checkout_id` columns on trace tables. `RepositoryAgentTraceDb::verify_or_initialize_repository_metadata(repository_id)` inserts the singleton metadata row on first initialization and errors if an existing DB stores a different repository ID. `RepositoryAgentTraceDb::repair_missing_repository_schema_migration_metadata()` is a narrow concurrent-first-open repair seam: it never creates trace tables, but if every required repository schema table already exists and only the one-file baseline migration record is missing, it records `001_repository_schema` and rechecks readiness.

`RepositoryAgentTraceDb` exposes repository-level write helpers for the current row families by delegating to the same typed insert payloads and parameterized SQL used by the checkout-scoped adapter: `insert_diff_trace`, `insert_post_commit_patch_intersection`, `insert_agent_trace`, `insert_message`, `insert_messages`, `insert_part`, and `insert_parts`. It also exposes `recent_diff_trace_patches(cutoff_time_ms, end_time_ms)` by delegating to the shared recent diff-trace query/parser helper, so repository-scoped attribution reads use the same chronological inclusive window semantics without a checkout filter. These methods preserve the existing row shapes and do not add checkout provenance columns or checkout-scoped write/query APIs.

The repository-scoped adapter is consumed by `agent_trace_storage`, active hook runtime opening, Agent Trace setup/doctor lifecycle, and `sce trace` status/list/shell flows. Hook writers/readers resolve the current repository storage context before using `RepositoryAgentTraceDb`. It also exposes `open_for_hooks_without_migrations_at(path)` — the explicit-path no-migration runtime-open used by the trace read paths (`stats`, `discovery`/readiness, `shell`) — plus the migration-running `new_at(path)` constructor used by setup and hook-runtime fallback initialization. There is no longer a checkout-scoped adapter.

## Non-goals

- No read/query helper for loading messages with their joined parts exists in the current runtime; the typed write helpers (`insert_message`, `insert_messages`, `insert_part`, `insert_parts`) are the only exposed message/part API surface. Message/part query helpers are deferred to a future task.
- No part upsert/deduplication; `parts` uses only the internal integer `id` for row identity (append-only per the `INSERT_PART_SQL` contract).

## Database path

The active repository-scoped Agent Trace DB path is resolved from repository identity through the shared default-path catalog:

- Function: `agent_trace_db_path_for_repository(repository_id)` in `cli/src/services/default_paths.rs`
- Path template: `<state_root>/sce/repos/<repository-id>/agent-trace.db`
- Repository ID source: explicit `agent_trace.repository_id` config, otherwise canonicalized configured Git remote URL (`agent_trace.repository_remote`, default `origin`)
- Checkout ID source: `<git-dir>/sce/checkout-id`, where `<git-dir>` comes from `git rev-parse --git-dir`; checkout ID is diagnostic metadata only and is not stored on Agent Trace rows.

Example state layout:

```text
<state-root>/sce/
├── repos/
│   └── <repository-id>/
│       └── agent-trace.db       # active DB shared by clones/worktrees of this logical repo
├── agent-trace-<checkout-id>.db # pre-migration on-disk file; never touched, no longer inspectable via the CLI
└── agent-trace.db               # pre-migration on-disk file; never touched, no longer inspectable via the CLI
```

SCE creates one Agent Trace DB per logical Git repository on demand through setup, hook, doctor, or trace commands. It does not run a daemon, background service, scheduler, watcher, external lock service, registry service, or data migration worker.

`agent_trace_db_path_for_repository(repository_id)` is the only Agent Trace DB path helper. The former global sentinel helper `agent_trace_db_path()` and the per-checkout helper `agent_trace_db_path_for_checkout(checkout_id)` were removed by the `retire-legacy-agent-trace-db` plan. Any pre-migration `agent-trace-<checkout-id>.db` / global `agent-trace.db` files left on disk are never migrated, imported, copied, renamed, archived, deleted, or backfilled by SCE, and are no longer inspectable through the CLI. When repository identity cannot be resolved (including outside a Git repository), lifecycle code returns an actionable "requires a Git repository" diagnostic instead of falling back to a sentinel path.

## Migrations

`RepositoryAgentTraceDbSpec::migrations()` returns `generated_migrations::AGENT_TRACE_REPOSITORY_MIGRATIONS`, generated from `cli/migrations/agent-trace-repository/` at build time. It is currently one fresh multi-statement baseline file:

- `001_repository_schema.sql` (migration ID `001_repository_schema`) — creates `repository_metadata`, `diff_traces` (including `payload_type TEXT NOT NULL DEFAULT 'patch'`), `post_commit_patch_intersections`, `agent_traces`, `messages`, and `parts`, plus the lookup indexes and `updated_at` triggers, in one `execute_batch` statement recorded as a single migration ID.

The former checkout-scoped `AGENT_TRACE_MIGRATIONS` constant and its 15-file `cli/migrations/agent-trace/` chain (`001_create_diff_traces` … `015_add_diff_traces_payload_type`) were removed by the `retire-legacy-agent-trace-db` plan; `build.rs` auto-discovers migration directories, so deleting the directory dropped the constant on regeneration. The repository schema captures the same tables/columns/indexes/triggers that the old incremental chain produced.

The shared `TursoDb` runner records applied IDs in the database-local `__sce_migrations` table. Migration SQL is executed with `execute_batch`, so the one-file repository baseline can contain multiple statements while still recording one migration ID.

Repository-scoped hook DB resolution first resolves `agent_trace.repository_id` / `agent_trace.repository_remote` through config, then uses `agent_trace_storage::resolve_agent_trace_storage(...)`. The storage resolver tries `RepositoryAgentTraceDb::open_without_migrations_at(path)` + `ensure_schema_ready_for_hooks()` + repository metadata validation first. If setup has not initialized the repository DB, metadata is absent, or migrations are incomplete, it falls back to migration-running `RepositoryAgentTraceDb::new_at(path)` and validates/seeds `repository_metadata.repository_id` before returning. The resolver retries the fast-path/migration sequence for a bounded window during concurrent first opens; if another opener completed the one-file schema but the baseline migration record is missing, the repository adapter records that metadata only after verifying all required repository schema tables already exist. When the fallback also fails, the error context includes the fast-path failure reason (`(fast-path attempt: {fast_error})`) so both failure causes are visible in diagnostics. Normal readiness is based on exact migration metadata parity with `AGENT_TRACE_REPOSITORY_MIGRATIONS`; table introspection is used only by the narrow concurrent-first-open metadata repair seam.

The `diff_traces` baseline migration creates:

- `id INTEGER PRIMARY KEY`
- `time_ms INTEGER NOT NULL`
- `session_id TEXT NOT NULL`
- `patch TEXT NOT NULL`
- `created_at TEXT NOT NULL DEFAULT (...)`
- `model_id TEXT`
- `tool_name TEXT`
- `tool_version TEXT`

Migration `015_add_diff_traces_payload_type` adds:

- `payload_type TEXT NOT NULL DEFAULT 'patch'` — discriminator for source payload format; `patch` for `OpenCode` unified-diff payloads, `structured` for `Claude` `PostToolUse` structured payloads.

The `post_commit_patch_intersections` baseline migration creates:

- `id INTEGER PRIMARY KEY`
- `commit_id TEXT NOT NULL`
- `post_commit_time_ms INTEGER NOT NULL`
- `recent_window_cutoff_ms INTEGER NOT NULL`
- `recent_window_end_ms INTEGER NOT NULL`
- `loaded_diff_trace_count INTEGER NOT NULL CHECK (loaded_diff_trace_count >= 0)`
- `skipped_diff_trace_count INTEGER NOT NULL CHECK (skipped_diff_trace_count >= 0)`
- `intersection_patch TEXT NOT NULL`
- `created_at TEXT NOT NULL DEFAULT (...)`

The `agent_traces` baseline migration creates:

- `id INTEGER PRIMARY KEY`
- `commit_id TEXT NOT NULL`
- `commit_time_ms INTEGER NOT NULL`
- `url TEXT NOT NULL`
- `remote_url TEXT` (nullable)
- `trace_json TEXT NOT NULL`
- `agent_trace_id TEXT NOT NULL UNIQUE`
- `created_at TEXT NOT NULL DEFAULT (...)`

The `messages` migration creates:

- `id INTEGER PRIMARY KEY`
- `session_id TEXT NOT NULL`
- `message_id TEXT NOT NULL`
- `role TEXT NOT NULL CHECK (role IN ('user', 'assistant'))`
- `generated_at_unix_ms INTEGER NOT NULL CHECK (generated_at_unix_ms >= 0)`
- `created_at TEXT NOT NULL DEFAULT (...)`
- `updated_at TEXT NOT NULL DEFAULT (...)`

The `parts` migration creates:

- `id INTEGER PRIMARY KEY`
- `type TEXT NOT NULL`
- `text TEXT NOT NULL`
- `message_id TEXT NOT NULL`
- `session_id TEXT NOT NULL`
- `generated_at_unix_ms INTEGER NOT NULL CHECK (generated_at_unix_ms >= 0)`
- `created_at TEXT NOT NULL DEFAULT (...)`
- `updated_at TEXT NOT NULL DEFAULT (...)`

No foreign keys exist between `messages` and `parts`; rows may be written out of order. The data model uses natural identifiers (`session_id`, `message_id`) for joins rather than DB-level referential integrity.

Lookup indexes created by the baseline migration set:

- `idx_diff_traces_time_ms_id` on `(time_ms, id)`
- `idx_agent_traces_agent_trace_id` on `(agent_trace_id)`
- `idx_agent_traces_remote_url` on `(remote_url)`
- `idx_messages_session_message` unique index on `(session_id, message_id)` — enables duplicate-ignore parent message inserts by natural key
- `idx_messages_session_order` on `(session_id, generated_at_unix_ms, id)` — enables chronological session-scoped message retrieval
- `idx_parts_session_message_order` on `(session_id, message_id, generated_at_unix_ms, id)` — enables ordered part joins per message

`updated_at` triggers defined by the migration set:

- `trg_messages_updated_at`: fires on `UPDATE` for non-`updated_at` column changes on `messages`
- `trg_parts_updated_at`: fires on `UPDATE` for non-`updated_at` column changes on `parts`

Both triggers compare `OLD.*` vs `NEW.*` for all mutable columns (excluding `updated_at` itself) and refresh the timestamp only when a real change occurred.

## Lifecycle integration

`AgentTraceDbLifecycle` is registered in `cli/src/services/lifecycle.rs` after `LocalDbLifecycle` and before optional `HooksLifecycle`.

- `diagnose()` resolves repository identity from config or the configured Git remote and reports the repository-scoped Agent Trace DB path and parent-directory readiness. When the DB file exists, it opens the file via `RepositoryAgentTraceDb::open_without_migrations_at` and verifies schema readiness via `ensure_schema_ready_for_hooks`, reporting `AgentTraceDbConnectionFailed` if open fails or `AgentTraceDbSchemaNotReady` if the schema is incomplete. Missing repository identity is a manual problem with `.sce/config.json` / remote guidance. Outside repository context there is no repository identity to select a DB and no global/checkout fallback path; the lifecycle returns an actionable "requires a Git repository" diagnostic, surfaced by `diagnose_agent_trace_db_health` as a manual-only `UnableToResolveStateRoot` problem.
- `fix()` bootstraps the resolved repository DB parent directory for auto-fixable parent-readiness problems, with the same global parent fallback outside repository context.
- `setup()` resolves repository storage through `agent_trace_storage`, creates/reuses the current checkout identity for diagnostics, opens/creates `<state_root>/sce/repos/<repository-id>/agent-trace.db` with the repository schema, validates `repository_metadata.repository_id`, and emits setup messaging with the repository ID, checkout ID, and initialized DB path. Hook runtime lazy initialization remains available for repositories where setup has not run or schema metadata is incomplete.
- `sce doctor` surfaces checkout identity facts where available and lifecycle-owned repository Agent Trace DB health in the `Configuration` section, with `[PASS]`/`[FAIL]`/`[MISS]` status tokens. Outside repository context the lifecycle reports the actionable "requires a Git repository" diagnostic instead of probing a sentinel path. JSON output includes `checkout_identity` when available plus the resolved `agent_trace_db` field.
- `sce trace db list` discovers repository DBs under `<state_root>/sce/repos/<repository-id>/agent-trace.db`, reporting text or JSON sorted by mtime descending. See [context/cli/trace-command.md](../cli/trace-command.md).

## Runtime writers

`sce hooks diff-trace` is the current runtime writer for `diff_traces`.

- The hook path validates required STDIN `{ sessionID, diff, time, tool_name, tool_version }` before persistence, with `model_id` accepted as optional (absent or `null`) and `tool_version` accepted as nullable. Missing attribution remains `None`; `diff_traces.model_id` is the only active model-attribution storage for diff traces and there is no session-level fallback lookup.
- Direct payload `model_id` and `tool_version` values pass into `DiffTraceInsert` as-is. The stored `session_id` is tool-prefixed before insert construction: `opencode` payloads store `oc_<sessionID>`, `claude` structured payloads store `cc_<session_id/sessionID>`, `pi` normalized payloads store `pi_<sessionID>`, and same-tool-prefixed values are not double-prefixed. The `payload_type` field is set to `PAYLOAD_TYPE_PATCH` for `OpenCode` normalized diff-trace payloads and `PAYLOAD_TYPE_STRUCTURED` for Claude structured `PostToolUse` payloads. Claude structured intake best-effort extracts direct `model`/`model_id`/`modelId` metadata, including nested `model.id` / `model.model` / `model.name`, normalizes values with the `claude/` prefix when present, and leaves `model_id` nullable when metadata is absent.
- `time` is accepted as a `u64` Unix epoch millisecond input and must fit the signed `i64` `time_ms` column before any persistence starts.
- The hook inserts the parsed payload fields plus nullable direct attribution through `RepositoryAgentTraceDb::insert_diff_trace()` without writing a parsed-payload artifact under `context/tmp`.
- AgentTraceDb open failures are logged at error level through `sce.hooks.diff_trace.agent_trace_db_open_failed`; later conversion/insert failures retain `sce.hooks.diff_trace.agent_trace_db_write_failed`. Both failure classes preserve deterministic failed-persistence success text and create no artifact fallback. Open failures use the producer-native unprefixed session and do not also emit the write-failure event.
- Existing artifact files are not backfilled into the database.

Post-commit intersection rows are written by the active `post-commit` hook flow through repository-scoped Agent Trace DB access, and the same flow inserts built Agent Trace payloads into `agent_traces` via `RepositoryAgentTraceDb::insert_agent_trace()` (see [agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md)). The persisted `trace_json` is the schema-validated `build_agent_trace(...)` output and includes top-level `metadata.sce.version` from the compiled `sce` CLI package version plus `content_hash` on every emitted range. Range `content_hash` values are computed from the touched-line kind/content of the post-commit hunk that produced the persisted range, not from DB IDs, paths, line positions, or runtime metadata.

`sce hooks conversation-trace` is the current runtime writer for `messages` and `parts`.

- The hook accepts normalized snake_case mixed-batch STDIN envelopes: top-level `payloads` is an array and each persisted item owns its own `type`; any top-level `type` is ignored and does not classify same-kind old-envelope items.
- `message` items validate and map payloads without message-level `text`, `agent`, or `summary_diffs` to `InsertMessageInsert`; valid rows are inserted through at most one multi-row `RepositoryAgentTraceDb::insert_messages(...)` call per invocation so repeated `(session_id, message_id)` events are ignored without failing.
- `message.part` items validate and map payloads with required part `text` to `InsertPartInsert`; valid rows are inserted through at most one multi-row `RepositoryAgentTraceDb::insert_parts(...)` call per invocation so parts remain append-only and do not require a pre-existing message row.
- Unsupported item types, missing/non-string item types, non-object items, and event-specific parser validation failures are retained as skipped-item diagnostics, logged, and counted as skipped while valid sibling items remain eligible for persistence.
- The hook opens one repository-scoped `RepositoryAgentTraceDb` per invocation through lazy repository storage resolution before insertion; all clones/worktrees of the same logical repository share the same repository-level message/part rows. DB open/initialization failures are logged at error level through `sce.hooks.conversation_trace.agent_trace_db_open_failed` and returned as hook success because conversation-trace intake is fail-open to producers. The event uses the existing best-effort producer-native session route and does not also emit `sce.hooks.conversation_trace.error` for the same open failure.
- Multi-row insert failures are logged once and count the whole valid-item batch as skipped without failing the command; the hook does not fall back to row-by-row insertion after a batch failure. Successful inserts contribute to deterministic success output counts (`attempted`, `persisted_messages`, `persisted_parts`, `skipped`). Duplicate parent message inserts preserve the existing `ON CONFLICT DO NOTHING` affected-row semantics.
- No `context/tmp` artifact is written for conversation traces.
- The generated OpenCode agent-trace plugin sends mixed-batch envelopes for conversation traces: regular `message` and `message.part` events each carry one per-item `type`, while diff-backed `message` events send one envelope containing the synthetic parent message item plus patch part items.

`sce hooks session-model` is no longer a supported command route, generated Claude settings no longer produce `SessionStart` model-attribution events, and the Agent Trace DB adapter no longer exposes a `session_models` API or fresh-schema table. See [agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md).

## Recent patch reads

`RepositoryAgentTraceDb::recent_diff_trace_patches(cutoff_time_ms, end_time_ms)` supports the post-commit comparison flow without changing `diff_traces` writes:

- SQL reads `id`, `time_ms`, `session_id`, `patch`, nullable `model_id` + `tool_name` + `tool_version`, and `payload_type` from `diff_traces` where `time_ms >= cutoff_time_ms AND time_ms <= end_time_ms`.
- Rows are ordered by `time_ms ASC, id ASC` for deterministic chronological processing.
- Valid row patches are parsed through `cli/src/services/patch.rs` `parse_patch` for `payload_type="patch"` rows (OpenCode unified-diff payloads), while `payload_type="structured"` rows (Claude `PostToolUse` structured payloads) are parsed from stored JSON through `cli/src/services/structured_patch.rs` `derive_claude_structured_patch` at read time to produce `ParsedPatch` without pre-rendered unified-diff text.
- Each produced `PatchHunk` is annotated with the originating row `model_id` (`Some(value)` propagated verbatim, `NULL` propagated as `None`) for both patch and structured paths; parsed row records also carry nullable `tool_name`/`tool_version` and `payload_type` from the same source row and are returned as `ParsedDiffTracePatch` records.
- Malformed recent row patches (invalid unified-diff text, invalid structured JSON, unsupported payload types, or unsupported Claude structured payloads) are returned as `SkippedDiffTracePatch` records with deterministic parse-error or derivation-skip reasons; malformed historical rows do not fail the operation.
- `RecentDiffTracePatches::loaded_count()` and `skipped_count()` expose accounting for later hook output and persistence metadata.

## Staged-diff AI-overlap evidence gate

`cli/src/services/agent_trace.rs` owns the pure patch-overlap helper `patches_have_overlap`, which is consumed by the commit-msg staged-diff AI-overlap evidence gate in `cli/src/services/hooks/mod.rs`:

- `patches_have_overlap(staged_patch, recent_patch)` returns `true` when the staged diff and a recent AI/editor diff trace share at least one touched line, and `false` otherwise (including empty/untouched patches). This is the pure boolean predicate used by the commit-msg evidence gate.
- `StagedDiffAiOverlapResult` (`Overlap`/`NoOverlap`/`Error`) is the three-valued result from the injectable `staged_diff_has_ai_overlap_with` variant, enabling testable branch coverage and caller-side error logging.
- `staged_diff_has_ai_overlap` is the live wrapper that opens Agent Trace DB through the no-migration hook path, delegates to `_with`, and logs `sce.hooks.commit_msg.ai_overlap_error` on `Error` results.
- The commit-msg evidence gate invokes the preflight only when the attribution gate passes (`attribution_hooks_enabled && !sce_disabled`); both `NoOverlap` and `Error` map to `ai_contribution_present = false`, suppressing the trailer. There is no fail-open mode.
- Fixture-backed unit coverage for `patches_have_overlap` lives in `cli/src/services/agent_trace/tests.rs`, covering overlap, no-overlap, empty/untouched patches, and Claude structured-patch-derived input.

See also: [shared-turso-db.md](shared-turso-db.md), [local-db.md](local-db.md), [agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md), [agent-trace-commit-msg-coauthor-policy.md](agent-trace-commit-msg-coauthor-policy.md), [context-map.md](../context-map.md)
