# Agent Trace Database Adapter

`cli/src/services/agent_trace_db/mod.rs` defines the Agent Trace persistence adapter as a thin alias over the shared Turso adapter:

```rust
pub type AgentTraceDb = TursoDb<AgentTraceDbSpec>;
```

## Module structure

- `AgentTraceDbSpec`: `DbSpec` implementation for Agent Trace persistence.
- `AgentTraceDb`: type alias for `TursoDb<AgentTraceDbSpec>`.
- `DiffTraceInsert<'a>`: insert payload with `time_ms: i64`, `session_id: &'a str`, `patch: &'a str`, `model_id: &'a str`, `tool_name: &'a str`, and nullable `tool_version: Option<&'a str>`.
- `insert_diff_trace()`: domain-specific insert helper using parameterized SQL.
- `RecentDiffTracePatches`: parsed recent `diff_traces` query result containing valid parsed patches plus skipped-row reports.
- `recent_diff_trace_patches(cutoff_time_ms, end_time_ms)`: chronological `diff_traces` read helper for rows in the inclusive window `time_ms >= cutoff_time_ms AND time_ms <= end_time_ms`; parses raw patch text through `parse_patch` and skips malformed rows without failing the query.
- `PostCommitPatchIntersectionInsert<'a>`: insert payload for post-commit intersection results with commit metadata, window bounds, loaded/skipped counts, and serialized patch JSON.
- `insert_post_commit_patch_intersection()`: domain-specific insert helper using parameterized SQL.
- `AgentTraceInsert<'a>`: insert payload for built Agent Trace rows with `commit_id`, `commit_time_ms`, serialized `trace_json`, `agent_trace_id`, and non-null `url`.
- `insert_agent_trace()`: domain-specific insert helper for `agent_traces` using parameterized SQL.
- `lifecycle.rs`: service lifecycle provider for setup/doctor integration.

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

The shared `TursoDb` runner records applied IDs in the database-local `__sce_migrations` table. Existing Agent Trace DB files without metadata are brought forward by re-applying the idempotent migration set and recording each ID, so rerunning `sce setup` / `AgentTraceDb::new()` applies later Agent Trace migrations to an already-created `~/.local/state/sce/agent-trace.db`.

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
- `trace_json TEXT NOT NULL`
- `agent_trace_id TEXT NOT NULL UNIQUE`
- `created_at TEXT NOT NULL DEFAULT (...)`

Lookup indexes created by the baseline migration set:

- `idx_diff_traces_time_ms_id` on `(time_ms, id)`
- `idx_agent_traces_agent_trace_id` on `(agent_trace_id)`

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
- The hook writes the existing collision-safe `context/tmp/<timestamp>-000000-diff-trace.json` parsed-payload artifact and inserts the parsed payload fields through `AgentTraceDb::insert_diff_trace()`.
- Command success requires both artifact and database persistence to succeed.
- Existing artifact files are not backfilled into the database.

Post-commit intersection rows are written by the active `post-commit` hook flow, and the same flow now also inserts built Agent Trace payloads into `agent_traces` via `AgentTraceDb::insert_agent_trace()` (see [agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md)). The persisted `trace_json` is the schema-validated `build_agent_trace(...)` output and includes top-level `metadata.sce.version` from the compiled `sce` CLI package version plus `content_hash` on every emitted range. Range `content_hash` values are computed from the touched-line kind/content of the post-commit hunk that produced the persisted range, not from DB IDs, paths, line positions, or runtime metadata.

## Recent patch reads

`AgentTraceDb::recent_diff_trace_patches(cutoff_time_ms, end_time_ms)` supports the post-commit comparison flow without changing `diff_traces` writes:

- SQL reads `id`, `time_ms`, `session_id`, `patch`, and nullable `model_id` + `tool_name` + `tool_version` from `diff_traces` where `time_ms >= cutoff_time_ms AND time_ms <= end_time_ms`.
- Rows are ordered by `time_ms ASC, id ASC` for deterministic chronological processing.
- Valid row patches are parsed through `cli/src/services/patch.rs` `parse_patch`, then each produced `PatchHunk` is annotated with the originating row `model_id` (`Some(value)` propagated verbatim, `NULL` propagated as `None`); parsed row records also carry nullable `tool_name`/`tool_version` from the same source row and are returned as `ParsedDiffTracePatch` records.
- Malformed recent row patches are returned as `SkippedDiffTracePatch` records with deterministic parse-error reasons; malformed historical rows do not fail the operation.
- `RecentDiffTracePatches::loaded_count()` and `skipped_count()` expose accounting for later hook output and persistence metadata.

See also: [shared-turso-db.md](shared-turso-db.md), [local-db.md](local-db.md), [agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md), [context-map.md](../context-map.md)
