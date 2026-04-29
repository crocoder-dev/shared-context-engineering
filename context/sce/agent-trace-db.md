# Agent-Trace DB Service

Agent-trace local Turso database adapter that provides the dedicated `diff_traces` persistence target for accepted hook diff-trace payloads.

## Overview

The `agent-trace.db` is a separate Turso database created at `<state_root>/sce/agent-trace.db`. It is managed by the `AgentTraceDb` service module at `cli/src/services/agent_trace_db.rs`, which follows the same structural pattern as `local_db.rs`.

## Module structure

Defined in `cli/src/services/agent_trace_db.rs`:

- `AgentTraceDb` struct — wraps a Turso connection with a tokio current-thread runtime
- `DiffTraceInsert` struct — validated diff-trace payload fields for DB insertion
- `new()` — opens/creates the database, runs migrations
- `execute()` — executes SQL statements that don't return rows
- `query()` — executes SQL queries that return rows
- `insert_diff_trace()` — inserts a validated diff-trace payload into `diff_traces`
- `run_migrations()` — runs embedded migrations from `cli/migrations/agent-trace/`

## Database path

Resolved by `agent_trace_db_path()` in `cli/src/services/default_paths.rs`:
- Linux: `$XDG_STATE_HOME/sce/agent-trace.db` (falls back to `~/.local/state/sce/agent-trace.db`)
- Other platforms: platform-equivalent `dirs::state_dir()` semantics

## Migrations

Embedded at compile time from `cli/migrations/agent-trace/`:
- `001_create_diff_traces.sql` — creates the `diff_traces` table with `id`, `time_ms`, `session_id`, `patch`, and DB-owned `created_at` columns

Migrations use `CREATE TABLE IF NOT EXISTS` for idempotency and run automatically when `AgentTraceDb::new()` is called.

## Current usage

`AgentTraceDb` is registered as a service module and can open/create `agent-trace.db`, run its embedded migration, and insert rows through its typed API. `cli/src/services/hooks.rs` imports `AgentTraceDb`/`DiffTraceInsert` from `agent_trace_db.rs`; `sce hooks diff-trace` validates STDIN, writes the collision-safe `context/tmp` artifact, then inserts the accepted payload through `AgentTraceDb::insert_diff_trace()`.

`cli/src/services/setup.rs` also exposes `bootstrap_agent_trace_db()`, which calls `AgentTraceDb::new()` with setup-context error handling. `cli/src/app.rs` calls this helper immediately after `bootstrap_local_db()` during setup dispatch, so successful `sce setup` runs initialize both `local.db` and `agent-trace.db` before config/hooks work proceeds.

## Relationship to local.db

- `local.db` — still bootstrapped by setup/doctor, currently tableless, and reserved for future SCE runtime data; it no longer contains diff-trace migration/API code and does not receive active hook diff-trace inserts.
- `agent-trace.db` — active DB target for `sce hooks diff-trace`; contains the `diff_traces` table when `AgentTraceDb::new()` is called.

See also: [local-db.md](local-db.md), [context-map.md](../context-map.md)
