# Shared Turso Database Infrastructure

`cli/src/services/db/mod.rs` provides the shared Turso database adapter seam for CLI services that need local Turso-backed persistence.

## Contract

- `DbSpec`: service-specific database metadata.
  - `db_name()` returns a human-readable diagnostic name.
  - `db_path()` resolves the canonical database file path.
  - `migrations()` returns ordered embedded migration `(id, sql)` pairs.
- `cli/build.rs` scans immediate `cli/migrations/<db-name>/*.sql` directories at compile time, copies them into `OUT_DIR/static/migrations`, and writes `OUT_DIR/generated_migrations.rs` with database-named migration constants (`AGENT_TRACE_REPOSITORY_MIGRATIONS`, `AUTH_MIGRATIONS`, etc.). Generated entries use the filename stem as the migration ID, embed the staged SQL with `include_str!`, and are sorted by the numeric prefix before the first `_`.
- `TursoDb<M: DbSpec>`: generic unencrypted adapter that owns:
  - tokio current-thread runtime creation
  - Turso local database open/connect flow using `turso::Builder::new_local()` with `experimental_multiprocess_wal(true)` so concurrent `sce` processes can safely access the same local database without WAL lock contention
  - config-driven connection-open retry around only the `build().await.connect()` block using `run_with_retry_sync` (resolved from `policies.database_retry.<db>.connection_open` via `DATABASE_RETRY_CONFIG` `OnceLock` with fallback to hardcoded defaults `3` attempts, `1s` timeout, `25ms..200ms` backoff)
  - config-driven operation retry for `execute()`, `query()`, and `query_map()` using `run_with_retry_sync` (resolved from `policies.database_retry.<db>.query` via the same `OnceLock` with fallback to hardcoded defaults `5` attempts, `200ms` timeout, `25ms..100ms` backoff, with default worst-case failure budget `<= 2_000ms`)
  - parent-directory creation
  - retry-backed synchronous `execute()`, `query()`, raw-value `query_values()`, and row-mapping `query_map()` wrappers via the public adapter methods, with config-driven query retry resolved from `policies.database_retry.<db>.query`
  - migration-running initialization through `new()` and generic embedded migration execution through `run_migrations()` delegated to the shared internal `TursoConnectionCore<M>` with per-database `__sce_migrations` metadata
  - explicit-path migration-running initialization through `new_at(path)`, preserving the same service-specific retry/migration behavior while letting callers supply a database path outside `DbSpec::db_path()`
  - no-migration opening through `open_without_migrations()`, which preserves parent-directory creation and connection-open retry but does not create `__sce_migrations` or apply embedded migrations
  - explicit-path no-migration opening through `open_without_migrations_at(path)` for path-resolved hot runtime callers
  - `migration_metadata_problems(&self) -> Result<Vec<String>>`: non-mutating readiness check that queries `__sce_migrations` metadata and compares applied migration IDs against `M::migrations()`; returns a list of problems (missing metadata table, incomplete applied migrations, unexpected extra migrations) or an empty list when the schema is ready
  - `ensure_schema_ready(&self, setup_guidance: &str) -> Result<()>`: non-mutating hook-readiness gate that calls `migration_metadata_problems()` and bails with a formatted error including `M::db_name()` and the caller-provided guidance string when problems are found; returns `Ok(())` when the schema is ready
  - `pub(crate) fn from_connection(conn: turso::Connection, runtime: tokio::runtime::Runtime) -> Self`: wraps an already-open connection and the runtime that opened it as a `TursoDb<M>`, without opening a new connection, creating `__sce_migrations`, or running migrations. For callers that open a connection through a non-local-path builder — currently only `AgentTraceDwhReplica`'s Turso Sync open (see [agent-trace-dwh-replica.md](agent-trace-dwh-replica.md)) — and want to reuse this adapter's synchronous SQL surface and schema-readiness checks instead of duplicating them.
  - `pub(crate) fn block_on<F: Future>(&self, future: F) -> F::Output`: runs a future to completion on the runtime backing this connection, so a companion async handle opened alongside it (e.g. a Turso Sync `Database` used for `pull`/`push`) can be driven without owning a second runtime.
  - `pub fn transaction<T>(&self, f: impl FnOnce(&TursoTransaction<M>) -> Result<T>) -> Result<T>`: runs `f` inside an explicit `BEGIN IMMEDIATE` transaction on the same connection. Commits only when `f` returns `Ok`; any closure error, or a failed `COMMIT`, triggers a best-effort `ROLLBACK` before the original error is returned. `TursoTransaction<'a, M: DbSpec>` is the transaction-scoped handle passed to `f`, exposing non-retried synchronous `execute()`/`query_map()` over the same connection/runtime; no raw `turso::Transaction` is ever exposed outside this module. This is a single-level seam: no nested transactions, savepoints, or transaction-level retry. It works for any `TursoDb<M>`, including `AgentTraceDwhDb` opened through `new_at()` or through `AgentTraceDwhReplica`'s `from_connection` seam.
  - `pub fn read_transaction<T>(&self, f: impl FnOnce(&TursoTransaction<M>) -> Result<T>) -> Result<T>`: same commit/rollback shape as `transaction()`, but issues a plain `BEGIN` instead of `BEGIN IMMEDIATE`, so the transaction never reserves the database's write lock and concurrent writers on other connections are not blocked while it is open. Both methods share a private `run_transaction(begin_sql, f)` helper. Intended for read-only callers such as Agent Trace ETL source extraction (see [agent-trace-etl.md](agent-trace-etl.md)), which must not interfere with concurrent multiprocess-WAL hook writers on `agent-trace.db`.
  - `pub(crate) fn rollback_best_effort(&self)`: issues a best-effort `ROLLBACK`, ignoring failures. Used internally by `transaction()`/`read_transaction()` after a closure error or failed commit, and available to same-crate callers that retry a whole transaction attempt (e.g. Agent Trace ETL source contention retries) to clear a stale failed transaction before issuing a new `BEGIN`.
- `EncryptedTursoDb<M: DbSpec>`: encrypted-adapter seam parallel to `TursoDb<M>` with the same structural shape (connection, runtime bridge, and spec marker). `EncryptedTursoDb::new()` resolves the encryption key via `encryption_key::get_or_create_encryption_key()` (environment variable `SCE_AUTH_DB_ENCRYPTION_KEY` with OS credential-store fallback), enables Turso experimental local encryption, applies strict `aegis256` cipher selection through `turso::EncryptionOpts` during local DB open/connect, wraps that open/connect block in the same connection-open retry policy resolved from `policies.database_retry.<db>.connection_open`, and runs embedded migrations after connect.
- `EncryptedTursoDb<M>` exposes the same public synchronous `execute()`, `query()`, `query_map()`, and `run_migrations()` methods; operation methods use the same config-driven query retry policy as `TursoDb<M>`.
- `TursoConnectionCore<M>` is internal to `cli/src/services/db/mod.rs` and owns the shared Turso connection plus tokio current-thread runtime bridging used by the public adapter methods; generic embedded migration execution with per-database `__sce_migrations` metadata is delegated to `run_embedded_migrations` helpers; encryption vs unencrypted behavior remains constructor-only at the public adapter layer.
- Shared lifecycle helpers:
  - `collect_db_path_health()` emits common parent/path health problems for DB-backed services.
  - `bootstrap_db_parent()` creates the resolved DB parent directory for repair/setup flows.

## Encryption key management

`cli/src/services/db/encryption_key.rs` exposes
`get_or_create_encryption_key(db_path: &Path, db_name: &str) -> Result<String>`.
It first checks `SCE_AUTH_DB_ENCRYPTION_KEY`; when that env var is present,
the raw non-empty secret text is deterministically SHA-256 hashed and rendered
as the 64-character lowercase hex key required by Turso encryption. Empty or
whitespace-only env values fail before any keyring initialization. When the env
var is absent, the module uses OS-credential-store-backed get-or-create logic
through `keyring-core` v1. Credential-store default registration is guarded by a
stable `OnceLock<bool>` plus an atomic in-progress flag: successful registration
is recorded once, while panic or error leaves the cell uninitialized so a later
caller can retry without mutex-poisoning the process. On first keyring-backed use
for a given database (file does not exist), a 32-byte random key is generated,
hex-encoded to 64 characters, and persisted in the platform credential store
(macOS Keychain, Linux Secret Service via zbus, Windows Credential Store). On
subsequent keyring-backed use, the key is read from the credential store.
Credential-store initialization, entry creation/storage, and existing-DB/missing-key
errors return platform-specific remediation for Linux system keyring/Secret
Service, macOS Keychain, Windows Credential Store, or unsupported platforms, and
always mention `SCE_AUTH_DB_ENCRYPTION_KEY` for headless/CI use without printing
the secret value. No plaintext fallback exists.

## Current integration state

The shared module is exported from `cli/src/services/mod.rs` and compile-checked. Current concrete wrappers:

- `cli/src/services/local_db/mod.rs`: `LocalDb = TursoDb<LocalDbSpec>`, with `LocalDbSpec` resolving `local_db_path()` and declaring zero migrations.
- `cli/src/services/agent_trace_db/mod.rs`: owns the shared Agent Trace insert payloads/helpers. Active hook/runtime paths use the sole `RepositoryAgentTraceDb = TursoDb<RepositoryAgentTraceDbSpec>` adapter from `cli/src/services/agent_trace_db/repository.rs`, selected by `agent_trace_storage` at `<state_root>/sce/repos/<repository-id>/agent-trace.db`, with a one-file repository schema containing repository metadata plus repository-level `diff_traces`, `post_commit_patch_intersections`, `agent_traces`, `messages`, and `parts` tables. The checkout-scoped `AgentTraceDb`/`AgentTraceDbSpec` adapter, its `agent_trace_db_path()` global fallback, and the 15-file migration chain were removed by the `retire-legacy-agent-trace-db` plan.
- `cli/src/services/auth_db/mod.rs`: `AuthDb = EncryptedTursoDb<AuthDbSpec>`, with `AuthDbSpec` resolving `auth_db_path()` and loading ordered auth migrations where baseline SQL creates `auth_credentials` without `user_id`, with `updated_at`, and a trigger that auto-refreshes `updated_at` on row updates.
- `cli/src/services/agent_trace_dwh_db/mod.rs`: `AgentTraceDwhDb = TursoDb<AgentTraceDwhDbSpec>`, a separate append-oriented destination-schema adapter for the CLI-independent Agent Trace ETL consumer, distinct from the repository-scoped source schema above. Explicit-path only (no canonical `db_path()`), reuses the `"agent_trace_db"` retry config key, and is not wired into any lifecycle provider, doctor/setup flow, or CLI command. See [agent-trace-dwh-db.md](agent-trace-dwh-db.md).
- `cli/src/services/agent_trace_dwh_replica/mod.rs`: `AgentTraceDwhReplica`, the sole owner of a Turso Sync connection to the repository-scoped `agent-trace-sync.db` replica. Built by opening a connection through `turso::sync::Builder` (the `sync` Cargo feature) and wrapping it as an `AgentTraceDwhDb` via the `from_connection`/`block_on` seam above, rather than through `TursoDb::new`/`new_at`. See [agent-trace-dwh-replica.md](agent-trace-dwh-replica.md).
- `cli/src/services/agent_trace_etl/mod.rs`: the first `TursoDb::read_transaction` consumer and the incremental `AgentTraceEtl` bridge. It extracts bounded, ordered `agent_traces` batches without reserving the source write lock, then loads facts and the lineage watermark atomically through `AgentTraceDwhReplica`. See [agent-trace-etl.md](agent-trace-etl.md).

All three database areas (local DB, auth DB, Agent Trace DB) have lifecycle providers. `lifecycle_providers(include_hooks)` registers database providers in order `LocalDbLifecycle` → `AuthDbLifecycle` → `AgentTraceDbLifecycle` before optional hooks. Setup initializes local/auth DBs, establishes Agent Trace checkout identity for diagnostics, initializes the repository-scoped Agent Trace DB with migrations/metadata, and reports credential-safe repository identity metadata. Hook runtime (`open_repository_db_for_hook_runtime` in `cli/src/services/agent_trace_storage/mod.rs`) never creates, migrates, or repairs schema/migration metadata: it opens through `open_without_migrations_at` and calls `ensure_schema_ready_for_hooks()`, a non-mutating readiness check that fails with `sce setup` guidance when the schema is missing or migration-incomplete; only source-instance metadata initialization (an atomic, race-safe `UPDATE` of an existing column) still runs on the hook path. Doctor diagnoses/fixes DB parent/path readiness through lifecycle providers.

## Migration metadata

`TursoDb<M>::new()` opens the database through the same connection path as `open_without_migrations()`, then calls `run_migrations()`. The shared `TursoConnectionCore<M>` migration path creates a service-local `__sce_migrations` table before applying migrations. Each migration is skipped only when its ID is already recorded in that table; otherwise the SQL is executed and the ID is recorded after success.

`TursoDb<M>::open_without_migrations()` is the explicit runtime-open seam for high-frequency callers that must not perform schema changes on their hot path. It still creates the parent directory and uses the configured connection-open retry policy; callers must verify schema readiness before query/write work.

Migrations are deliberately outside the connection-open retry block. The generic constructors retry only local Turso open/connect; schema changes are not generically retried because migration SQL must not be replayed after partial execution. Service-specific resolvers may add bounded recovery around their own initialization contracts when they can prove safety; the repository-scoped Agent Trace resolver does this only for its one-file fresh schema and only repairs missing migration metadata after all required repository schema tables already exist.

`TursoDb<M>` and `EncryptedTursoDb<M>` operation methods use the same config-driven query retry policy, resolved from `policies.database_retry.<db>.query` via `DATABASE_RETRY_CONFIG` `OnceLock` with fallback to hardcoded defaults (`5` attempts, `200ms` timeout, `25ms..100ms` backoff; default worst-case failure budget `<= 2_000ms`). `execute()`, `query()`, and `query_values()` convert caller parameters to owned Turso params before retry so each attempt can clone the same values. `query_values()` returns fully fetched column names plus raw `turso::Value` rows for deterministic rendering by operator-facing services. `query_map()` retries the initial query and full row-fetch loop, then runs caller-provided row mapping after retry completion so mapping failures are surfaced as logic errors and are not retried.

Existing databases created before migration metadata are upgraded by re-applying the current idempotent migration list and recording each migration ID. This lets later `sce setup` / lifecycle initialization runs apply migrations added after the database file already existed, including Agent Trace DB schema/index additions.

See also: [local-db.md](local-db.md), [agent-trace-db.md](agent-trace-db.md), [agent-trace-dwh-db.md](agent-trace-dwh-db.md), [agent-trace-dwh-replica.md](agent-trace-dwh-replica.md), [agent-trace-etl.md](agent-trace-etl.md), [auth-db.md](auth-db.md), [overview.md](../overview.md), [architecture.md](../architecture.md), [glossary.md](../glossary.md)
