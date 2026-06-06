# Auth DB

The encrypted auth DB foundation provides a thin Rust wrapper, path and migration assets, and is consumed by `cli/src/services/token_storage.rs` for runtime auth-token persistence.

## Implemented surface

- Canonical path resolver: `cli/src/services/default_paths.rs::auth_db_path()`.
- Database file path: `<state_root>/sce/auth.db`.
- Service wrapper: `cli/src/services/auth_db/mod.rs`.
- `AuthDbSpec` implements `DbSpec` with diagnostic name `auth DB`, `auth_db_path()`, and ordered embedded auth migrations.
- `AuthDb` is a type alias for `EncryptedTursoDb<AuthDbSpec>`, consumed by the lifecycle provider at `cli/src/services/auth_db/lifecycle.rs`.
- Migration directory: `cli/migrations/auth/`.
- Ordered migrations:
  - `001_create_auth_tokens.sql`
  - `002_create_auth_credentials_updated_at_trigger.sql`

## Schema baseline

`auth_credentials` is created idempotently with:

- `id INTEGER PRIMARY KEY NOT NULL`
- `access_token TEXT NOT NULL`
- `token_type TEXT NOT NULL`
- `expires_in INTEGER NOT NULL`
- `refresh_token TEXT NOT NULL`
- `scope TEXT` (nullable)
- `stored_at_unix_seconds INTEGER NOT NULL`
- `created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))`
- `updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))`

Current migration baseline:

- `001_create_auth_tokens.sql` creates `auth_credentials` without `user_id`, with `updated_at`.
- `002_create_auth_credentials_updated_at_trigger.sql` creates `auth_credentials_set_updated_at` trigger to auto-refresh `updated_at` on row updates.

## Lifecycle integration

`AuthDbLifecycle` is registered in `cli/src/services/auth_db/lifecycle.rs` following the existing DB lifecycle pattern:
- `diagnose` collects auth DB path health problems.
- `fix` bootstraps missing auth DB parent directory.
- `setup` calls `AuthDb::new()` to initialize the encrypted database.
- `LifecycleProviderId::AuthDb` is the provider identifier.
- The lifecycle provider is registered in deterministic order: config → local_db → auth_db → agent_trace_db → hooks.

## Token storage integration

- `cli/src/services/token_storage.rs` now uses `AuthDb` for all persistence operations (`save_tokens`, `load_tokens`, `delete_tokens`) via a `OnceLock<Result<AuthDb, String>>` lazy singleton.
- `token_file_path()` returns the auth DB path from `auth_db_path()` instead of a JSON file path.
- `TokenStorageError` exposes `PathResolution` and `Database` variants; former `Io`, `Serialization`, `CorruptedTokenFile`, and `Permission` variants have been removed.
- No JSON file I/O remains in `token_storage.rs`.
- The `auth_credentials` row uses constant integer ID `1` for single-row token storage.
- Encryption is required: `encryption_key::get_or_create_encryption_key()` first honors `SCE_AUTH_DB_ENCRYPTION_KEY` by deriving a deterministic 64-character hex Turso key from the non-empty env-secret text, then falls back to OS credential-store keyring get-or-create behavior when the env var is absent. No plaintext auth DB fallback exists; failures surface as `TokenStorageError::Database`.

See also: [shared-turso-db.md](shared-turso-db.md), [../cli/default-path-catalog.md](../cli/default-path-catalog.md), [../context-map.md](../context-map.md), [../../context/plans/token-storage-db-migration.md](../../context/plans/token-storage-db-migration.md)
