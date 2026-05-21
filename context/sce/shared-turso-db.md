# Shared Turso Database Infrastructure

`cli/src/services/db/mod.rs` provides the shared Turso database adapter seam for CLI services that need Turso-backed persistence.

## Contract

- `DbSpec`: service-specific database metadata.
  - `db_name()` returns a human-readable diagnostic name.
  - `db_path()` resolves the canonical database file path.
  - `migrations()` returns ordered embedded migration `(id, sql)` pairs.
- `TursoDb<M: DbSpec>`: generic adapter that supports two modes:
  - **Local mode** (default): opens a plain Turso file database via `turso::Builder::new_local()`.
  - **Sync mode**: opens a Turso Cloud synced database via `turso::sync::Builder::new_remote()` when both `SCE_SYNC_URL` and `SCE_SYNC_TOKEN` env vars are set.
  - Owns tokio current-thread runtime creation, parent-directory creation, synchronous `execute()`, `query()`, and row-mapping `query_map()` wrappers, and generic embedded migration execution through `run_migrations()` with per-database `__sce_migrations` metadata.
  - Sync-mode methods:
    - `push()` — pushes local changes to the remote (no-op in local mode).
    - `pull()` — pulls remote changes, returns `true` if changes applied (returns `false` in local mode).
    - `checkpoint()` — forces a WAL checkpoint (no-op in local mode).
    - `stats()` — returns `Option<DatabaseSyncStats>` (returns `None` in local mode).
  - Explicit sync: Sync operations are never triggered automatically from `execute()`. Callers use `push()`/`pull()` directly, or the `sce sync push|pull` CLI command. Push failures from explicit calls propagate to the caller.
- Shared lifecycle helpers:
  - `collect_db_path_health()` emits common parent/path health problems for DB-backed services (sync mode uses the same local file path, so health checks are identical).
  - `bootstrap_db_parent()` creates the resolved DB parent directory for repair/setup flows.

## Sync env var reference

| Env var | Purpose | Default |
|---|---|---|
| `SCE_SYNC_URL` | Remote Turso Cloud URL | unset (triggers local mode) |
| `SCE_SYNC_TOKEN` | Auth token for remote access | unset (triggers local mode) |

When both `SCE_SYNC_URL` and `SCE_SYNC_TOKEN` are set, the database opens in sync mode. When either is absent, local-only mode is used.

## Internal struct fields

- `conn: turso::Connection` — active database connection (shared between modes).
- `sync_db: Option<turso::sync::Database>` — sync handle for push/pull/checkpoint/stats; `None` in local mode.

## Current integration state

The shared module is exported from `cli/src/services/mod.rs` and compile-checked. Current concrete wrappers:

- `cli/src/services/local_db/mod.rs`: `LocalDb = TursoDb<LocalDbSpec>`, with `LocalDbSpec` resolving `local_db_path()` and declaring zero migrations.
- `cli/src/services/agent_trace_db/mod.rs`: `AgentTraceDb = TursoDb<AgentTraceDbSpec>`, with `AgentTraceDbSpec` resolving `agent_trace_db_path()` and loading ordered Agent Trace migrations for `diff_traces` and `post_commit_patch_intersections`.

Both database wrappers now have lifecycle providers. `lifecycle_providers(include_hooks)` registers database providers in order `LocalDbLifecycle` → `AgentTraceDbLifecycle` before optional hooks, so setup initializes both databases and doctor diagnoses/fixes both canonical DB paths.

## Migration metadata

`TursoDb<M>::run_migrations()` creates a service-local `__sce_migrations` table before applying migrations. Each migration is skipped only when its ID is already recorded in that table; otherwise the SQL is executed and the ID is recorded after success.

Existing databases created before migration metadata are upgraded by re-applying the current idempotent migration list and recording each migration ID. This lets later `sce setup` / lifecycle initialization runs apply migrations added after the database file already existed, including Agent Trace DB schema/index additions.

See also: [local-db.md](local-db.md), [agent-trace-db.md](agent-trace-db.md), [overview.md](../overview.md), [architecture.md](../architecture.md), [glossary.md](../glossary.md)
