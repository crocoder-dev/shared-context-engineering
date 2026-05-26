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
  - synchronous `execute()`, `query()`, and row-mapping `query_map()` wrappers
  - generic embedded migration execution through `run_migrations()` with per-database `__sce_migrations` metadata
- `EncryptedTursoDb<M: DbSpec>`: encrypted-adapter seam parallel to `TursoDb<M>` with the same structural shape (connection, runtime bridge, and spec marker). `EncryptedTursoDb::new()` resolves the encryption key from the OS credential store via `encryption_key::get_or_create_encryption_key()`, enables Turso experimental local encryption, applies strict `aegis256` cipher selection through `turso::EncryptionOpts` during local DB open/connect, and runs embedded migrations after connect.
- `EncryptedTursoDb<M>` also exposes synchronous `execute()`, `query()`, and `query_map()` wrappers plus generic `run_migrations()` with the same `__sce_migrations` metadata flow used by `TursoDb<M>`.
- Shared lifecycle helpers:
  - `collect_db_path_health()` emits common parent/path health problems for DB-backed services.
  - `bootstrap_db_parent()` creates the resolved DB parent directory for repair/setup flows.

## Encryption key management

`cli/src/services/db/encryption_key.rs` provides OS-credential-store-backed encryption key
get-or-create logic using `keyring-core` v1. Exposes
`get_or_create_encryption_key(db_path: &Path, db_name: &str) -> Result<String>`.
This module is consumed by `EncryptedTursoDb::new()` to replace the previous
`SCE_DB_ENCRYPTION_KEY` environment variable approach. On first use for a given
database (file does not exist), a 32-byte random key is generated, hex-encoded to
64 characters, and persisted in the platform credential store (macOS Keychain,
Linux keyutils, Windows Credential Store). On subsequent use, the key is read
from the credential store. If the DB file exists but the key is missing (e.g.
Linux keyutils expiry), a clear remediation error is returned.

## Current integration state

The shared module is exported from `cli/src/services/mod.rs` and compile-checked. Current concrete wrappers:

- `cli/src/services/local_db/mod.rs`: `LocalDb = TursoDb<LocalDbSpec>`, with `LocalDbSpec` resolving `local_db_path()` and declaring zero migrations.
- `cli/src/services/agent_trace_db/mod.rs`: `AgentTraceDb = TursoDb<AgentTraceDbSpec>`, with `AgentTraceDbSpec` resolving `agent_trace_db_path()` and loading ordered Agent Trace migrations for `diff_traces` and `post_commit_patch_intersections`.
- `cli/src/services/auth_db/mod.rs`: `AuthDb = EncryptedTursoDb<AuthDbSpec>`, with `AuthDbSpec` resolving `auth_db_path()` and loading ordered auth migrations where baseline SQL creates `auth_credentials` without `user_id`, with `updated_at`, and a trigger that auto-refreshes `updated_at` on row updates.

All three database wrappers (local DB, auth DB, Agent Trace DB) have lifecycle providers. `lifecycle_providers(include_hooks)` registers database providers in order `LocalDbLifecycle` → `AuthDbLifecycle` → `AgentTraceDbLifecycle` before optional hooks, so setup initializes all three databases and doctor diagnoses/fixes all three canonical DB paths.

## Migration metadata

`TursoDb<M>::run_migrations()` creates a service-local `__sce_migrations` table before applying migrations. Each migration is skipped only when its ID is already recorded in that table; otherwise the SQL is executed and the ID is recorded after success.

Existing databases created before migration metadata are upgraded by re-applying the current idempotent migration list and recording each migration ID. This lets later `sce setup` / lifecycle initialization runs apply migrations added after the database file already existed, including Agent Trace DB schema/index additions.

See also: [local-db.md](local-db.md), [agent-trace-db.md](agent-trace-db.md), [auth-db.md](auth-db.md), [overview.md](../overview.md), [architecture.md](../architecture.md), [glossary.md](../glossary.md)
