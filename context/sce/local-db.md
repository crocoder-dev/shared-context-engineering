# Local Turso Database Adapter

Provides a `LocalDb` struct in `cli/src/services/local_db.rs` that wraps a Turso connection with a tokio current-thread runtime for blocking operations. `local.db` is currently tableless and reserved for future SCE runtime data; active hook diff-trace persistence uses `AgentTraceDb` and `agent-trace.db`.

## Module Structure

- `LocalDb` struct: Holds a `turso::Connection` and `tokio::runtime::Runtime`
- `LocalDb::new()`: Opens/creates a local Turso database at the canonical path, then runs the current no-op migration loop
- `LocalDb::execute()`: Executes SQL statements that don't return rows
- `LocalDb::query()`: Executes SQL queries that return rows

## Database Path

The local DB path is resolved from the shared default-path catalog:
- Function: `local_db_path()` in `cli/src/services/default_paths.rs`
- Path template: `<state_root>/sce/local.db`
- Linux: `$XDG_STATE_HOME/sce/local.db` (defaults to `~/.local/state/sce/local.db`)
- Other platforms: platform-equivalent user state root

## Migrations

The local DB migration list is intentionally empty:

```rust
const MIGRATIONS: &[(&str, &str)] = &[];
```

`LocalDb::new()` still calls `run_migrations()`, but the loop is currently a no-op. Future SCE runtime data can add migrations here without reintroducing Agent Trace tables into `local.db`.

## Current Runtime Consumers

- Setup and doctor continue to use `LocalDb::new()` for bootstrap and health/repair flows; there is still no user-invocable `sce sync` command.
- `sce hooks diff-trace` does not use `LocalDb`; it writes accepted payloads through `AgentTraceDb::insert_diff_trace()` while retaining the collision-safe `context/tmp/*-diff-trace.json` artifact.

## Usage Pattern

```rust
use crate::services::local_db::LocalDb;

// Open or create the database
let db = LocalDb::new()?;

// Query through the adapter when future local runtime tables are added
let mut rows = db.query("SELECT 1", ())?;
```

## Error Handling

All methods return `anyhow::Result` with context attached for actionable diagnostics:
- Failed to open database: "failed to open local database at {path}: {error}"
- Failed to connect: "failed to connect to local database: {error}"
- Failed migration: "migration {id} failed: {error}"

## Dependencies

- `turso` crate (async Turso/SQLite API)
- `tokio` runtime (current-thread, enable_io, enable_time)
- `anyhow` for error handling with context

See also: [overview.md](../overview.md), [glossary.md](../glossary.md), [context-map.md](../context-map.md)
