# Local Turso Database Adapter

Provides a `LocalDb` struct in `cli/src/services/local_db.rs` that wraps a Turso connection with a tokio current-thread runtime for blocking operations and keeps local DB SQL behind focused adapter methods.

## Module Structure

- `LocalDb` struct: Holds a `turso::Connection` and `tokio::runtime::Runtime`
- `LocalDb::new()`: Opens/creates a local Turso database at the canonical path, runs embedded migrations
- `LocalDb::execute()`: Executes SQL statements that don't return rows
- `LocalDb::query()`: Executes SQL queries that return rows
- `DiffTraceInsert`: Typed input for validated diff-trace `time_ms`, `session_id`, and `patch` values
- `LocalDb::insert_diff_trace()`: Inserts one diff-trace row with parameterized SQL and local-DB remediation context

## Database Path

The local DB path is resolved from the shared default-path catalog:
- Function: `local_db_path()` in `cli/src/services/default_paths.rs`
- Path template: `<state_root>/sce/local.db`
- Linux: `$XDG_STATE_HOME/sce/local.db` (defaults to `~/.local/state/sce/local.db`)
- Other platforms: platform-equivalent user state root

## Embedded Migrations

Migrations are embedded at compile time via `include_str!` from `cli/migrations/`:

```rust
const MIGRATION_001: &str = include_str!("../../migrations/001_create_diff_traces.sql");
const MIGRATIONS: &[(&str, &str)] = &[
    ("001", MIGRATION_001),
];
```

Migrations run automatically when `LocalDb::new()` is called. They use `CREATE TABLE IF NOT EXISTS` for idempotency.

Current embedded schema:

- `diff_traces`: stores accepted diff-trace payload data with `id`, incoming event `time_ms`, incoming `session_id`, incoming unified diff/patch text in `patch`, and DB-owned `created_at` insertion time.

## Current Runtime Consumers

- `sce hooks diff-trace` validates STDIN `{ sessionID, diff, time }`, retains the collision-safe `context/tmp/*-diff-trace.json` artifact, and inserts the accepted payload into `diff_traces` through `LocalDb::insert_diff_trace`.
- Setup and doctor continue to use `LocalDb::new()` for bootstrap and health/repair flows; there is still no user-invocable `sce sync` command.

## Usage Pattern

```rust
use crate::services::local_db::{DiffTraceInsert, LocalDb};

// Open or create the database
let db = LocalDb::new()?;

// Insert an accepted diff-trace payload through the typed seam
db.insert_diff_trace(DiffTraceInsert {
    time_ms: 1_777_403_999_227,
    session_id: "ses_123",
    patch: "Index: ...",
})?;

// Query rows
let mut rows = db.query("SELECT * FROM diff_traces", ())?;
```

## Error Handling

All methods return `anyhow::Result` with context attached for actionable diagnostics:
- Failed to open database: "failed to open local database at {path}: {error}"
- Failed to connect: "failed to connect to local database: {error}"
- Failed migration: "migration {id} failed: {error}"
- Failed diff-trace insertion: "failed to insert diff-trace payload into local DB. Try: run 'sce doctor --fix' to verify local DB health."

## Dependencies

- `turso` crate (async Turso/SQLite API)
- `tokio` runtime (current-thread, enable_io, enable_time)
- `anyhow` for error handling with context

See also: [overview.md](../overview.md), [glossary.md](../glossary.md), [context-map.md](../context-map.md)
