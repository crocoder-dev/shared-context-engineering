# Agent Trace Database Adapter

`cli/src/services/agent_trace_db/mod.rs` defines the Agent Trace persistence adapter as a thin alias over the shared Turso adapter:

```rust
pub type AgentTraceDb = TursoDb<AgentTraceDbSpec>;
```

## Module structure

- `AgentTraceDbSpec`: `DbSpec` implementation for Agent Trace persistence.
- `AgentTraceDb`: type alias for `TursoDb<AgentTraceDbSpec>`, inheriting shared constructor and operation retry behavior.
- `open_for_hooks_without_migrations()`: Agent Trace-specific runtime-open API for high-frequency hook paths; opens/connects via `TursoDb::open_without_migrations()` and does not run embedded migrations.
- `ensure_schema_ready_for_hooks()`: non-mutating hook-readiness check that delegates to the shared `TursoDb::ensure_schema_ready()` method with the Agent Trace–specific `AGENT_TRACE_SCHEMA_SETUP_GUIDANCE` constant (`"Run 'sce setup'."`); verifies the Agent Trace DB has the expected applied migration metadata in `__sce_migrations` for every ID in `AGENT_TRACE_MIGRATIONS`; missing/incomplete metadata fails with `Run 'sce setup'.` guidance instead of running migrations.
- `DiffTraceInsert<'a>`: insert payload with `time_ms: i64`, `session_id: &'a str`, `patch: &'a str`, `model_id: &'a str`, `tool_name: &'a str`, and nullable `tool_version: Option<&'a str>`.
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
- `insert_messages(inputs)`: typed batch helper that generates and executes one parameterized multi-row `messages` insert for valid conversation-trace `message.updated` batches while preserving duplicate-ignore semantics.
- `PartType` enum: `Text` / `Reasoning` / `Patch` — maps to `parts.type` DB constraint.
- `InsertPartInsert`: owned payload struct with `part_type`, `text`, `session_id`, `message_id`, and `generated_at_unix_ms`.
- `INSERT_PART_SQL`: parameterized single-row append-only INSERT into `parts` (no upsert; multiple rows per `(session_id, message_id)` allowed).
- `insert_part(input)`: typed single-row helper that inserts a part row without requiring a matching `messages` row (supports out-of-order writes); retained as part of the adapter surface.
- `insert_parts(inputs)`: typed batch helper that generates and executes one parameterized multi-row append-only `parts` insert for valid conversation-trace `message.part.updated` batches.
- `lifecycle.rs`: service lifecycle provider for setup/doctor integration.

## Non-goals

- No read/query helper for loading messages with their joined parts exists in the current runtime; the typed write helpers (`insert_message`, `insert_messages`, `insert_part`, `insert_parts`) are the only exposed message/part API surface. Message/part query helpers are deferred to a future task.
- No part upsert/deduplication; `parts` uses only the internal integer `id` for row identity (append-only per the `INSERT_PART_SQL` contract).

## Database path

The Agent Trace DB path is resolved from the shared default-path catalog:

- Function: `agent_trace_db_path()` in `cli/src/services/default_paths.rs`
- Path template: `<state_root>/sce/agent-trace.db`
- Linux: `$XDG_STATE_HOME/sce/agent-trace.db` (defaults to `~/.local/state/sce/agent-trace.db`)
- Other platforms: platform-equivalent user state root

## Migrations

`AgentTraceDbSpec::migrations()` embeds an ordered split fresh-start baseline migration set from `cli/migrations/agent-trace/`:

- `001_create_diff_traces.sql`
- `002_create_post_commit_patch_intersections.sql`
- `003_create_agent_traces.sql`
- `004_create_diff_traces_time_ms_id_index.sql`
- `005_create_agent_traces_agent_trace_id_index.sql`
- `006_add_agent_traces_vcs_remote_url.sql` (historical filename; migration ID `006_add_agent_traces_remote_url` adds the `remote_url` column)
- `007_create_agent_traces_vcs_remote_url_index.sql` (historical filename; migration ID `007_create_agent_traces_remote_url_index` creates `idx_agent_traces_remote_url`)
- `008_create_messages.sql`
- `009_create_parts.sql`
- `010_create_messages_session_message_unique_index.sql`
- `011_create_messages_session_order_index.sql`
- `012_create_parts_session_message_order_index.sql`
- `013_create_messages_updated_at_trigger.sql`
- `014_create_parts_updated_at_trigger.sql`

The shared `TursoDb` runner records applied IDs in the database-local `__sce_migrations` table. Existing Agent Trace DB files without metadata are brought forward by re-applying the idempotent migration set and recording each ID, so rerunning `sce setup` / `AgentTraceDb::new()` applies later Agent Trace migrations to an already-created `~/.local/state/sce/agent-trace.db`.

`AgentTraceDb::open_for_hooks_without_migrations()` is the named no-migration Agent Trace open path for hook runtime code. It preserves Turso open/connect retry behavior from the shared adapter but intentionally skips `run_migrations()`, so it neither creates `__sce_migrations` nor applies Agent Trace schema SQL. Active hook callers (`conversation-trace`, `diff-trace`, and both post-commit Agent Trace DB flows) use this path and call `ensure_schema_ready_for_hooks()` before reads/writes; readiness is based on exact migration metadata parity with `AGENT_TRACE_MIGRATIONS`, not table/index/column introspection.

The `diff_traces` baseline migration creates:

- `id INTEGER PRIMARY KEY`
- `time_ms INTEGER NOT NULL`
- `session_id TEXT NOT NULL`
- `patch TEXT NOT NULL`
- `created_at TEXT NOT NULL DEFAULT (...)`
- `model_id TEXT`
- `tool_name TEXT`
- `tool_version TEXT`

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
- `type TEXT NOT NULL CHECK (type IN ('text', 'reasoning', 'patch'))`
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
 
- `diagnose()` reports canonical Agent Trace DB path and parent-directory readiness problems through the shared DB path-health helper.
- `fix()` can bootstrap the canonical Agent Trace DB parent directory for auto-fixable parent-readiness problems.
- `setup()` initializes the database with `AgentTraceDb::new()`, including all ordered Agent Trace migrations and any later migrations not yet recorded in `__sce_migrations`.
- `sce doctor` now surfaces Agent Trace DB health as a row within the `Configuration` section with `[PASS]`/`[FAIL]`/`[MISS]` status tokens (e.g., `Agent Trace DB (/path/to/agent-trace.db)`), and includes it in JSON output under the `agent_trace_db` field.

## Runtime writers

`sce hooks diff-trace` is the current runtime writer for `diff_traces`.

- The hook path validates required STDIN `{ sessionID, diff, time, model_id, tool_name, tool_version }` before persistence (`tool_name` non-empty; `tool_version` present and either `null` or non-empty string) and passes parsed `model_id`, `tool_name`, and nullable `tool_version` into `DiffTraceInsert`.
- `time` is accepted as a `u64` Unix epoch millisecond input and must fit the signed `i64` `time_ms` column before any persistence starts.
- The hook writes the existing collision-safe `context/tmp/<timestamp>-000000-diff-trace.json` parsed-payload artifact and inserts the parsed payload fields through readiness-gated `AgentTraceDb::insert_diff_trace()`.
- Command success requires both artifact and database persistence to succeed.
- Existing artifact files are not backfilled into the database.

Post-commit intersection rows are written by the active `post-commit` hook flow through readiness-gated AgentTraceDb access, and the same flow now also inserts built Agent Trace payloads into `agent_traces` via `AgentTraceDb::insert_agent_trace()` (see [agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md)). The persisted `trace_json` is the schema-validated `build_agent_trace(...)` output and includes top-level `metadata.sce.version` from the compiled `sce` CLI package version plus `content_hash` on every emitted range. Range `content_hash` values are computed from the touched-line kind/content of the post-commit hunk that produced the persisted range, not from DB IDs, paths, line positions, or runtime metadata.

`sce hooks conversation-trace` is the current runtime writer for `messages` and `parts`.

- The hook accepts only normalized snake_case typed batch STDIN envelopes: top-level `type` is `message.updated` or `message.part.updated`, and top-level `payloads` is an array of same-kind item objects.
- `message.updated` batch items validate and map payloads without message-level `text`, `agent`, or `summary_diffs` to `InsertMessageInsert`; valid rows are inserted through one multi-row `AgentTraceDb::insert_messages(...)` call so repeated `(session_id, message_id)` events are ignored without failing.
- `message.part.updated` batch items validate and map payloads with required part `text` to `InsertPartInsert`; valid rows are inserted through one multi-row `AgentTraceDb::insert_parts(...)` call so parts remain append-only and do not require a pre-existing message row.
- Per-item parser validation failures are retained as skipped-item diagnostics, logged, and counted as skipped while valid sibling items remain eligible for persistence.
- The hook opens one no-migration `AgentTraceDb` per invocation and checks schema readiness before insertion; DB open or readiness failures remain command-failing because no rows can be attempted.
- Multi-row insert failures are logged once and count the whole valid-item batch as skipped without failing the command; the hook does not fall back to row-by-row insertion after a batch failure. Successful inserts contribute to deterministic success output counts (`attempted`, `persisted`, `skipped`). Duplicate parent message inserts preserve the existing `ON CONFLICT DO NOTHING` affected-row semantics.
- No `context/tmp` artifact is written for conversation traces.
- The generated OpenCode agent-trace plugin is a runtime caller for both conversation event variants and currently sends one-element typed batch envelopes for captured `message.updated` and `message.part.updated` events.

## Recent patch reads

`AgentTraceDb::recent_diff_trace_patches(cutoff_time_ms, end_time_ms)` supports the post-commit comparison flow without changing `diff_traces` writes:

- SQL reads `id`, `time_ms`, `session_id`, `patch`, and nullable `model_id` + `tool_name` + `tool_version` from `diff_traces` where `time_ms >= cutoff_time_ms AND time_ms <= end_time_ms`.
- Rows are ordered by `time_ms ASC, id ASC` for deterministic chronological processing.
- Valid row patches are parsed through `cli/src/services/patch.rs` `parse_patch`, then each produced `PatchHunk` is annotated with the originating row `model_id` (`Some(value)` propagated verbatim, `NULL` propagated as `None`); parsed row records also carry nullable `tool_name`/`tool_version` from the same source row and are returned as `ParsedDiffTracePatch` records.
- Malformed recent row patches are returned as `SkippedDiffTracePatch` records with deterministic parse-error reasons; malformed historical rows do not fail the operation.
- `RecentDiffTracePatches::loaded_count()` and `skipped_count()` expose accounting for later hook output and persistence metadata.

See also: [shared-turso-db.md](shared-turso-db.md), [local-db.md](local-db.md), [agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md), [context-map.md](../context-map.md)
