# Agent Trace Database Adapter

`cli/src/services/agent_trace_db/mod.rs` defines the Agent Trace persistence adapter as a thin alias over the shared Turso adapter:

```rust
pub type AgentTraceDb = TursoDb<AgentTraceDbSpec>;
```

## Module structure

- `AgentTraceDbSpec`: `DbSpec` implementation for Agent Trace persistence.
- `AgentTraceDb`: type alias for `TursoDb<AgentTraceDbSpec>`.
- `DiffTraceInsert<'a>`: insert payload with `time_ms: i64`, `session_id: &'a str`, and `patch: &'a str`.
- `insert_diff_trace()`: domain-specific insert helper using parameterized SQL.
- `DiffTracePatchRow`: read DTO with `id: i64` and `patch: String` for raw diff trace rows.
- `latest_diff_trace_session_id()`: returns `Option<String>` using `time_ms DESC, id DESC` latest-row ordering.
- `diff_trace_patches_for_session()`: returns all `id` + `patch` rows for a session using `time_ms ASC, id ASC` ordering.
- `PatchIntersectionInsert<'a>`: insert payload with `commit_sha`, ordered `source_diff_trace_ids`, and `intersection_json` string fields.
- `insert_patch_intersection()`: domain-specific insert helper using parameterized SQL.
- `lifecycle.rs`: service lifecycle provider for setup/doctor integration.

## Database path

The Agent Trace DB path is resolved from the shared default-path catalog:

- Function: `agent_trace_db_path()` in `cli/src/services/default_paths.rs`
- Path template: `<state_root>/sce/agent-trace.db`
- Linux: `$XDG_STATE_HOME/sce/agent-trace.db` (defaults to `~/.local/state/sce/agent-trace.db`)
- Other platforms: platform-equivalent user state root

## Migrations

`AgentTraceDbSpec::migrations()` embeds ordered Agent Trace DB migrations:

- `cli/migrations/agent-trace/001_create_diff_traces.sql`
- `cli/migrations/agent-trace/002_create_patch_intersections.sql`

The first migration creates `diff_traces` with:

- `id INTEGER PRIMARY KEY AUTOINCREMENT`
- `time_ms INTEGER NOT NULL`
- `session_id TEXT NOT NULL`
- `patch TEXT NOT NULL`
- `created_at TEXT NOT NULL DEFAULT (...)`

The second migration creates `patch_intersections` with:

- `id INTEGER PRIMARY KEY AUTOINCREMENT`
- `commit_sha TEXT NOT NULL`
- `source_diff_trace_ids TEXT NOT NULL`
- `intersection_json TEXT NOT NULL`
- `created_at TEXT NOT NULL DEFAULT (...)`

## Lifecycle integration
 
`AgentTraceDbLifecycle` is registered in `cli/src/services/lifecycle.rs` after `LocalDbLifecycle` and before optional `HooksLifecycle`.
 
- `diagnose()` reports canonical Agent Trace DB path and parent-directory readiness problems through the shared DB path-health helper.
- `fix()` can bootstrap the canonical Agent Trace DB parent directory for auto-fixable parent-readiness problems.
- `setup()` initializes the database with `AgentTraceDb::new()`, including the `diff_traces` and `patch_intersections` migrations.
- `sce doctor` now surfaces Agent Trace DB health as a row within the `Configuration` section with `[PASS]`/`[FAIL]`/`[MISS]` status tokens (e.g., `Agent Trace DB (/path/to/agent-trace.db)`), and includes it in JSON output under the `agent_trace_db` field.

## Runtime writers

`sce hooks diff-trace` is the runtime writer for `diff_traces`.

- The hook path validates STDIN `{ sessionID, diff, time }` before persistence.
- `time` is accepted as a `u64` Unix epoch millisecond input and must fit the signed `i64` `time_ms` column before any persistence starts.
- The hook writes the existing collision-safe `context/tmp/<timestamp>-000000-diff-trace.json` artifact and inserts the same payload through `AgentTraceDb::insert_diff_trace()`.
- Command success requires both artifact and database persistence to succeed.
- Existing artifact files are not backfilled into the database.

`sce hooks post-commit` is the runtime writer for `patch_intersections` when usable latest-session raw diffs and a valid `HEAD` patch exist.

- The hook preserves the `SCE_DISABLED` no-op path.
- It selects the latest `session_id` from `diff_traces` by `time_ms DESC, id DESC`; no available diff-trace rows returns a deterministic skip message.
- It loads selected-session source rows by `time_ms ASC, id ASC`; those `id` values are JSON-serialized into `source_diff_trace_ids` in the same order used for patch combination.
- It captures `HEAD` SHA and patch from git, builds compact `ParsedPatch` intersection JSON via `build_patch_intersection_json(...)`, and inserts one row through `AgentTraceDb::insert_patch_intersection()`.
- Session provenance is recoverable via `source_diff_trace_ids` → `diff_traces.session_id`; no `session_id` column is stored in `patch_intersections`.
- Invalid stored patch data, invalid post-commit patch data, missing `HEAD`, and DB failures are command-failing runtime errors; existing `diff_traces` rows are not modified.
- Full `AgentTrace` JSON is not generated or persisted by this flow.

## Read helpers

AgentTraceDb exposes read helpers for post-commit intersection persistence:

- `latest_diff_trace_session_id()` returns the latest available `session_id` from `diff_traces`, ordering rows by `time_ms DESC, id DESC` and returning `None` when no rows exist.
- `diff_trace_patches_for_session(session_id)` returns `DiffTracePatchRow { id, patch }` values for the selected session, ordered by `time_ms ASC, id ASC`; a missing session returns an empty vector.

These helpers do not parse patches or choose between multiple concurrent sessions beyond the latest-session heuristic.

See also: [shared-turso-db.md](shared-turso-db.md), [local-db.md](local-db.md), [agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md), [context-map.md](../context-map.md)
