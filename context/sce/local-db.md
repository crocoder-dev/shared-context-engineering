# Local Turso Database Adapter

Provides a `LocalDb` struct in `cli/src/services/local_db.rs` that wraps a Turso connection with a tokio current-thread runtime for blocking operations.
The embedded `001_create_agent_traces.sql` migration defines normalized local Agent Trace persistence tables, and `LocalDb::new()` now handles the retired placeholder schema deterministically before migrations run.
`LocalDb::insert_agent_trace(&AgentTrace)` stores the complete payload plus normalized query rows without wiring persistence into any CLI or hook runtime.

## Module Structure

- `LocalDb` struct: Holds a `turso::Connection` and `tokio::runtime::Runtime`
- `LocalDb::new()`: Opens/creates a local Turso database at the canonical path, resolves retired placeholder-schema compatibility, then runs embedded migrations
- `LocalDb::execute()`: Executes SQL statements that don't return rows
- `LocalDb::query()`: Executes SQL queries that return rows
- `LocalDb::insert_agent_trace(&AgentTrace)`: Serializes the complete trace and inserts normalized rows for files, conversations, and ranges

## Database Path

The local DB path is resolved from the shared default-path catalog:
- Function: `local_db_path()` in `cli/src/services/default_paths.rs`
- Path template: `<state_root>/sce/local.db`
- Linux: `$XDG_STATE_HOME/sce/local.db` (defaults to `~/.local/state/sce/local.db`)
- Other platforms: platform-equivalent user state root

## Embedded Migrations

Migrations are embedded at compile time via `include_str!` from `cli/migrations/`:

```rust
const MIGRATION_001: &str = include_str!("../../migrations/001_create_agent_traces.sql");
const MIGRATIONS: &[(&str, &str)] = &[
    ("001", MIGRATION_001),
];
```

Migrations run automatically when `LocalDb::new()` is called. They execute through Turso `execute_batch` because `001` contains multiple SQL statements, and they use `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT EXISTS` for idempotency.

`001_create_agent_traces.sql` defines:

- `agent_traces`: one row per trace, keyed by `trace_id`, with `version`, `timestamp`, full serialized `trace_json`, and `created_at`.
- `agent_trace_files`: normalized trace files with `(trace_id, file_index)` uniqueness and cascading delete from `agent_traces`.
- `agent_trace_conversations`: normalized per-file conversations with `(file_id, conversation_index)` uniqueness and `contributor_type`.
- `agent_trace_ranges`: normalized conversation ranges with `(conversation_id, range_index)` uniqueness and `start_line` / `end_line` values.
- Indexes for trace timestamp lookup and file path lookup.

## Replaced Placeholder Schema Handling

Before running migrations, `LocalDb::new()` inspects any existing `agent_traces` table:

- Missing table: normal fresh bootstrap path.
- Normalized table shape (`trace_id`, `version`, `timestamp`, `trace_json`, `created_at`): migrations rerun idempotently.
- Retired placeholder shape (`id`, `trace_json`, `created_at`): all Agent Trace tables are dropped in child-to-parent order and recreated from `001`; local placeholder data is intentionally disposable.
- Unknown incompatible shape: bootstrap fails early with remediation to move or remove the local SCE database file, instead of silently leaving a table that will fail later inserts.

## Agent Trace Insertion

`LocalDb::insert_agent_trace(&AgentTrace)` accepts the existing domain struct from `cli/src/services/agent_trace.rs` and performs persistence internally:

- Serializes the complete `AgentTrace` with `serde_json` into `agent_traces.trace_json`.
- Inserts `agent_trace_files`, `agent_trace_conversations`, and `agent_trace_ranges` rows in the source vector order using zero-based `file_index`, `conversation_index`, and `range_index` values.
- Stores contributor classifications as the current serialized names: `ai`, `mixed`, or `unknown`.
- Runs inside an explicit `BEGIN IMMEDIATE` / `COMMIT` transaction and rolls back on insert or conversion failures so parent/child rows do not persist partially.
- Duplicate `trace_id` values fail deterministically through the `agent_traces.trace_id` primary key and roll back without adding extra child rows.

This API is library-only. No CLI command, hook runtime, `diff-trace` intake path, or `sce sync` command currently calls it.

## Usage Pattern

```rust
use crate::services::agent_trace::AgentTrace;
use crate::services::local_db::LocalDb;

// Open or create the database
let db = LocalDb::new()?;

// Persist an existing AgentTrace value
let trace: AgentTrace = build_trace_somewhere()?;
db.insert_agent_trace(&trace)?;

// Query rows
let mut rows = db.query("SELECT * FROM agent_traces", ())?;
```

## Error Handling

All methods return `anyhow::Result` with context attached for actionable diagnostics:
- Failed to open database: "failed to open local database at {path}: {error}"
- Failed to connect: "failed to connect to local database: {error}"
- Failed migration: "migration {id} failed: {error}"
- Failed trace insert with rollback: "failed to insert Agent Trace into local DB; transaction rolled back"

## Dependencies

- `turso` crate (async Turso/SQLite API)
- `tokio` runtime (current-thread, enable_io, enable_time)
- `anyhow` for error handling with context

See also: [overview.md](../overview.md), [glossary.md](../glossary.md), [context-map.md](../context-map.md)
