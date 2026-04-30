# Local Turso Database Adapter

`cli/src/services/local_db/mod.rs` defines the concrete local database spec and exposes `LocalDb` as a thin alias over the shared Turso adapter:

```rust
pub type LocalDb = TursoDb<LocalDbSpec>;
```

## Module structure

- `LocalDbSpec`: `DbSpec` implementation for the canonical per-user local database.
- `LocalDb`: type alias for `TursoDb<LocalDbSpec>`.
- `LocalDb::new()`, `execute()`, and `query()`: inherited from `TursoDb<M>`.
- `local_db/lifecycle.rs`: owns local DB health, parent-directory bootstrap, and setup initialization through `LocalDb::new()`.

## Database path

The local DB path is resolved from the shared default-path catalog:

- Function: `local_db_path()` in `cli/src/services/default_paths.rs`
- Path template: `<state_root>/sce/local.db`
- Linux: `$XDG_STATE_HOME/sce/local.db` (defaults to `~/.local/state/sce/local.db`)
- Other platforms: platform-equivalent user state root

## Migrations

`LocalDbSpec::migrations()` currently returns an empty slice. `LocalDb::new()` still invokes the shared `TursoDb` migration runner, but no local DB tables are created by this adapter.

Agent Trace-specific persistence is split out of the neutral local DB baseline and belongs to the dedicated `agent_trace_db` service.

## Usage pattern

```rust
use crate::services::local_db::LocalDb;

let db = LocalDb::new()?;
let affected = db.execute("VACUUM", ())?;
let mut rows = db.query("PRAGMA database_list", ())?;
```

## Error handling

The shared `TursoDb` adapter returns `anyhow::Result` with service-name-qualified diagnostics for path resolution, parent-directory creation, runtime creation, database open/connect, migration execution, and SQL execution/query failures.

See also: [shared-turso-db.md](shared-turso-db.md), [agent-trace-db.md](agent-trace-db.md), [overview.md](../overview.md), [glossary.md](../glossary.md), [context-map.md](../context-map.md)
