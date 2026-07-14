# Agent Trace Database Adapter

`cli/src/services/agent_trace_db/mod.rs` defines the Agent Trace persistence adapter as a thin alias over the shared Turso adapter:

```rust
pub type AgentTraceDb = TursoDb<AgentTraceDbSpec>;
```

## Module structure

- `AgentTraceDbSpec`: `DbSpec` implementation for Agent Trace persistence.
- `AgentTraceDb`: type alias for `TursoDb<AgentTraceDbSpec>`, inheriting shared constructor and operation retry behavior.
- `open_at(path)`: migration-running explicit-path constructor for per-checkout Agent Trace databases.
- `open_for_hooks_without_migrations()`: Agent Trace-specific runtime-open API for high-frequency hook paths; opens/connects via `TursoDb::open_without_migrations()` and does not run embedded migrations.
- `open_for_hooks_without_migrations_at(path)`: explicit-path no-migration runtime-open API used by per-checkout hook resolution.
- `ensure_schema_ready_for_hooks()`: non-mutating hook-readiness check that delegates to the shared `TursoDb::ensure_schema_ready()` method with the Agent Trace–specific `AGENT_TRACE_SCHEMA_SETUP_GUIDANCE` constant (`"Run 'sce setup'."`); verifies the Agent Trace DB has the expected applied migration metadata in `__sce_migrations` for every ID in `AGENT_TRACE_MIGRATIONS`; missing/incomplete metadata fails with `Run 'sce setup'.` guidance instead of running migrations.
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

## Non-goals

- No read/query helper for loading messages with their joined parts exists in the current runtime; the typed write helpers (`insert_message`, `insert_messages`, `insert_part`, `insert_parts`) are the only exposed message/part API surface. Message/part query helpers are deferred to a future task.
- No part upsert/deduplication; `parts` uses only the internal integer `id` for row identity (append-only per the `INSERT_PART_SQL` contract).

## Database path

The legacy/global Agent Trace DB path is resolved from the shared default-path catalog and retained as a lifecycle fallback when no checkout context or checkout ID is available:

- Function: `agent_trace_db_path()` in `cli/src/services/default_paths.rs`
- Path template: `<state_root>/sce/agent-trace.db`
- Linux: `$XDG_STATE_HOME/sce/agent-trace.db` (defaults to `~/.local/state/sce/agent-trace.db`)
- Other platforms: platform-equivalent user state root

Active hook runtime resolves per-checkout Agent Trace DB files:

- Function: `agent_trace_db_path_for_checkout(checkout_id)` in `cli/src/services/default_paths.rs`
- Path template: `<state_root>/sce/agent-trace-{checkout_id}.db`
- Checkout ID source: `<git-dir>/sce/checkout-id`, where `<git-dir>` comes from `git rev-parse --git-dir`
- Checkouts are discovered by `sce trace db list` via filesystem scan of `<state_root>/sce/agent-trace-*.db` files; there is no central registry file.

## Migrations

`AgentTraceDbSpec::migrations()` returns `generated_migrations::AGENT_TRACE_MIGRATIONS`, generated from `cli/migrations/agent-trace/` at build time. Setup-time `AgentTraceDb::open_at(path)` and hook-runtime fallback initialization both apply this migration set. Migration IDs are the SQL filename stems, sorted by numeric prefix:

- `001_create_diff_traces.sql`
- `002_create_post_commit_patch_intersections.sql`
- `003_create_agent_traces.sql`
- `004_create_diff_traces_time_ms_id_index.sql`
- `005_create_agent_traces_agent_trace_id_index.sql`
- `006_add_agent_traces_vcs_remote_url.sql`
- `007_create_agent_traces_vcs_remote_url_index.sql`
- `008_create_messages.sql`
- `009_create_parts.sql`
- `010_create_messages_session_message_unique_index.sql`
- `011_create_messages_session_order_index.sql`
- `012_create_parts_session_message_order_index.sql`
- `013_create_messages_updated_at_trigger.sql`
- `014_create_parts_updated_at_trigger.sql`
- `015_add_diff_traces_payload_type.sql` (migration ID `015_add_diff_traces_payload_type`; adds `payload_type TEXT NOT NULL DEFAULT 'patch'` to `diff_traces`)

The former `015_create_session_models` migration was removed from the fresh schema when the `remove-session-models-direct-claude-model-id` plan cleaned up session-models support. The `retired_migration_ids()` compat mechanism and `RETIRED_AGENT_TRACE_MIGRATION_IDS` constant that previously accommodated upgraded databases with that migration ID were subsequently removed in the `remove-retired-migration-ids` plan, since all development databases have been recreated and no ongoing compatibility is needed. Current migration IDs go directly from `014_create_parts_updated_at_trigger` to `015_add_diff_traces_payload_type`.

The shared `TursoDb` runner records applied IDs in the database-local `__sce_migrations` table. Existing Agent Trace DB files without metadata are brought forward by re-applying the idempotent migration set and recording each ID, so rerunning `sce setup` / `AgentTraceDb::open_at(path)` applies later Agent Trace migrations to an already-created per-checkout DB.

Per-checkout hook DB resolution first tries `AgentTraceDb::open_for_hooks_without_migrations_at(path)` and `ensure_schema_ready_for_hooks()`. If setup has not initialized the DB, metadata is absent, or migrations are incomplete, the checkout resolver falls back to `AgentTraceDb::open_at(path)` so hook invocation lazily creates or upgrades the per-checkout DB before continuing. When the fallback also fails, the error context includes the fast-path failure reason (`(fast-path attempt: {fast_error})`) so both failure causes are visible in diagnostics. Readiness is based on exact migration metadata parity with `AGENT_TRACE_MIGRATIONS`, not table/index/column introspection.

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

- `diagnose()` reports per-checkout Agent Trace DB path and parent-directory readiness when a repo root has a checkout ID; otherwise it falls back to the legacy global Agent Trace DB path. When the DB file exists, it also performs a deep health check: opens the file via `open_for_hooks_without_migrations_at` and verifies schema readiness via `ensure_schema_ready_for_hooks`, reporting `AgentTraceDbConnectionFailed` if open fails or `AgentTraceDbSchemaNotReady` if the schema is incomplete. These deep-check problems are `ManualOnly` (not auto-fixable by `sce doctor --fix`); the remediation directs the operator to re-run `sce setup` or fix file permissions.
- `fix()` bootstraps the resolved per-checkout DB parent directory for auto-fixable parent-readiness problems, with the same global fallback outside checkout context.
- `setup()` creates/reuses the current checkout identity when a repo root is available, resolves `<state_root>/sce/agent-trace-{checkout_id}.db` through `agent_trace_db_path_for_checkout(checkout_id)`, opens/creates it with `AgentTraceDb::open_at(&db_path)` to apply embedded migrations, and emits setup messaging with the checkout ID plus initialized DB path. Hook runtime lazy initialization remains available for checkouts where setup has not run or schema metadata is incomplete.
- `sce doctor` surfaces checkout identity and per-checkout Agent Trace DB health in the `Configuration` section when a checkout ID exists, with `[PASS]`/`[FAIL]`/`[MISS]` status tokens. Outside checkout context it falls back to the legacy/global Agent Trace DB row. JSON output includes `checkout_identity` when available plus the resolved `agent_trace_db` field.
- `sce trace db list` discovers checkouts by scanning `<state_root>/sce/agent-trace-*.db` files on disk, reporting them in text or JSON sorted by mtime descending. See [context/cli/trace-command.md](../cli/trace-command.md).

## Runtime writers

`sce hooks diff-trace` is the current runtime writer for `diff_traces`.

- The hook path validates required STDIN `{ sessionID, diff, time, tool_name, tool_version }` before persistence, with `model_id` accepted as optional (absent or `null`) and `tool_version` accepted as nullable. Missing attribution remains `None`; `diff_traces.model_id` is the only active model-attribution storage for diff traces and there is no session-level fallback lookup.
- Direct payload `model_id` and `tool_version` values pass into `DiffTraceInsert` as-is. The stored `session_id` is tool-prefixed before insert construction: `opencode` payloads store `oc_<sessionID>`, `claude` structured payloads store `cc_<session_id/sessionID>`, `pi` normalized payloads store `pi_<sessionID>`, and same-tool-prefixed values are not double-prefixed. The `payload_type` field is set to `PAYLOAD_TYPE_PATCH` for `OpenCode` normalized diff-trace payloads and `PAYLOAD_TYPE_STRUCTURED` for Claude structured `PostToolUse` payloads. Claude structured intake best-effort extracts direct `model`/`model_id`/`modelId` metadata, including nested `model.id` / `model.model` / `model.name`, normalizes values with the `claude/` prefix when present, and leaves `model_id` nullable when metadata is absent.
- `time` is accepted as a `u64` Unix epoch millisecond input and must fit the signed `i64` `time_ms` column before any persistence starts.
- The hook inserts the parsed payload fields plus nullable direct attribution through `AgentTraceDb::insert_diff_trace()` without writing a parsed-payload artifact under `context/tmp`.
- AgentTraceDb open/insert failures are logged and reflected in deterministic success text as failed DB persistence; no artifact fallback is created.
- Existing artifact files are not backfilled into the database.

Post-commit intersection rows are written by the active `post-commit` hook flow through per-checkout lazy AgentTraceDb access, and the same flow now also inserts built Agent Trace payloads into `agent_traces` via `AgentTraceDb::insert_agent_trace()` (see [agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md)). The persisted `trace_json` is the schema-validated `build_agent_trace(...)` output and includes top-level `metadata.sce.version` from the compiled `sce` CLI package version plus `content_hash` on every emitted range. Range `content_hash` values are computed from the touched-line kind/content of the post-commit hunk that produced the persisted range, not from DB IDs, paths, line positions, or runtime metadata.

`sce hooks conversation-trace` is the current runtime writer for `messages` and `parts`.

- The hook accepts normalized snake_case mixed-batch STDIN envelopes: top-level `payloads` is an array and each persisted item owns its own `type`; any top-level `type` is ignored and does not classify same-kind old-envelope items.
- `message` items validate and map payloads without message-level `text`, `agent`, or `summary_diffs` to `InsertMessageInsert`; valid rows are inserted through at most one multi-row `AgentTraceDb::insert_messages(...)` call per invocation so repeated `(session_id, message_id)` events are ignored without failing.
- `message.part` items validate and map payloads with required part `text` to `InsertPartInsert`; valid rows are inserted through at most one multi-row `AgentTraceDb::insert_parts(...)` call per invocation so parts remain append-only and do not require a pre-existing message row.
- Unsupported item types, missing/non-string item types, non-object items, and event-specific parser validation failures are retained as skipped-item diagnostics, logged, and counted as skipped while valid sibling items remain eligible for persistence.
- The hook opens one per-checkout `AgentTraceDb` per invocation through lazy checkout DB resolution before insertion; DB open/initialization failures are logged through `sce.hooks.conversation_trace.error` and returned as hook success because conversation-trace intake is fail-open to producers.
- Multi-row insert failures are logged once and count the whole valid-item batch as skipped without failing the command; the hook does not fall back to row-by-row insertion after a batch failure. Successful inserts contribute to deterministic success output counts (`attempted`, `persisted_messages`, `persisted_parts`, `skipped`). Duplicate parent message inserts preserve the existing `ON CONFLICT DO NOTHING` affected-row semantics.
- No `context/tmp` artifact is written for conversation traces.
- The generated OpenCode agent-trace plugin sends mixed-batch envelopes for conversation traces: regular `message` and `message.part` events each carry one per-item `type`, while diff-backed `message` events send one envelope containing the synthetic parent message item plus patch part items.

`sce hooks session-model` is no longer a supported command route, generated Claude settings no longer produce `SessionStart` model-attribution events, and the Agent Trace DB adapter no longer exposes a `session_models` API or fresh-schema table. See [agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md).

## Recent patch reads

`AgentTraceDb::recent_diff_trace_patches(cutoff_time_ms, end_time_ms)` supports the post-commit comparison flow without changing `diff_traces` writes:

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
