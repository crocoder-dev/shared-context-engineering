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
- `lifecycle.rs`: service lifecycle provider for setup/doctor integration.

## Database path

The Agent Trace DB path is resolved from the shared default-path catalog:

- Function: `agent_trace_db_path()` in `cli/src/services/default_paths.rs`
- Path template: `<state_root>/sce/agent-trace.db`
- Linux: `$XDG_STATE_HOME/sce/agent-trace.db` (defaults to `~/.local/state/sce/agent-trace.db`)
- Other platforms: platform-equivalent user state root

## Migrations

`AgentTraceDbSpec::migrations()` embeds `cli/migrations/agent-trace/001_create_diff_traces.sql`.

The current migration creates `diff_traces` with:

- `id INTEGER PRIMARY KEY AUTOINCREMENT`
- `time_ms INTEGER NOT NULL`
- `session_id TEXT NOT NULL`
- `patch TEXT NOT NULL`
- `created_at TEXT NOT NULL DEFAULT (...)`

## Lifecycle integration

`AgentTraceDbLifecycle` is registered in `cli/src/services/lifecycle.rs` after `LocalDbLifecycle` and before optional `HooksLifecycle`.

- `diagnose()` reports canonical Agent Trace DB path and parent-directory readiness problems through the shared DB path-health helper.
- `fix()` can bootstrap the canonical Agent Trace DB parent directory for auto-fixable parent-readiness problems.
- `setup()` initializes the database with `AgentTraceDb::new()`, including the `diff_traces` migration.

## Runtime writers

`sce hooks diff-trace` is the current runtime writer for `diff_traces`.

- The hook path validates STDIN `{ sessionID, diff, time }` before persistence.
- `time` is accepted as a `u64` Unix epoch millisecond input and must fit the signed `i64` `time_ms` column before any persistence starts.
- The hook writes the existing collision-safe `context/tmp/<timestamp>-000000-diff-trace.json` artifact and inserts the same payload through `AgentTraceDb::insert_diff_trace()`.
- Command success requires both artifact and database persistence to succeed.
- Existing artifact files are not backfilled into the database.

Query/retrieval APIs remain follow-up scope.

See also: [shared-turso-db.md](shared-turso-db.md), [local-db.md](local-db.md), [agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md), [context-map.md](../context-map.md)
