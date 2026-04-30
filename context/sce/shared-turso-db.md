# Shared Turso Database Infrastructure

`cli/src/services/db/mod.rs` provides the shared Turso database adapter seam for CLI services that need local Turso-backed persistence.

## Contract

- `DbSpec`: service-specific database metadata.
  - `db_name()` returns a human-readable diagnostic name.
  - `db_path()` resolves the canonical database file path.
  - `migrations()` returns ordered embedded migration `(id, sql)` pairs.
- `TursoDb<M: DbSpec>`: generic adapter that owns:
  - tokio current-thread runtime creation
  - Turso local database open/connect flow
  - parent-directory creation
  - synchronous `execute()` and `query()` wrappers
  - generic embedded migration execution through `run_migrations()`
- Shared lifecycle helpers:
  - `collect_db_path_health()` emits common parent/path health problems for DB-backed services.
  - `bootstrap_db_parent()` creates the resolved DB parent directory for repair/setup flows.

## Current integration state

The shared module is exported from `cli/src/services/mod.rs` and compile-checked. Current concrete wrappers:

- `cli/src/services/local_db/mod.rs`: `LocalDb = TursoDb<LocalDbSpec>`, with `LocalDbSpec` resolving `local_db_path()` and declaring zero migrations.
- `cli/src/services/agent_trace_db/mod.rs`: `AgentTraceDb = TursoDb<AgentTraceDbSpec>`, with `AgentTraceDbSpec` resolving `agent_trace_db_path()` and loading `cli/migrations/agent-trace/001_create_diff_traces.sql`.

Both database wrappers now have lifecycle providers. `lifecycle_providers(include_hooks)` registers database providers in order `LocalDbLifecycle` → `AgentTraceDbLifecycle` before optional hooks, so setup initializes both databases and doctor diagnoses/fixes both canonical DB paths.

See also: [local-db.md](local-db.md), [agent-trace-db.md](agent-trace-db.md), [overview.md](../overview.md), [architecture.md](../architecture.md), [glossary.md](../glossary.md)
