# Auth DB

The encrypted auth DB foundation provides a thin Rust wrapper, path and migration assets, and is consumed by `cli/src/services/token_storage.rs` for runtime auth-token persistence.

## Implemented surface

- Canonical path resolver: `cli/src/services/default_paths.rs::auth_db_path()`.
- Database file path: `<state_root>/sce/auth.db`.
- Service wrapper: `cli/src/services/auth_db/mod.rs`.
- `AuthDbSpec` implements `DbSpec` with diagnostic name `auth DB`, `auth_db_path()`, and `generated_migrations::AUTH_MIGRATIONS`.
- `AuthDb` is a type alias for `EncryptedTursoDb<AuthDbSpec>`, inheriting retry-backed constructor and operation behavior and consumed by the lifecycle provider at `cli/src/services/auth_db/lifecycle.rs`.
- Migration directory: `cli/migrations/auth/`.
- Ordered migrations generated from filename stems:
  - `001_create_auth_tokens.sql`
  - `002_create_auth_credentials_updated_at_trigger.sql`
  - `003_create_telemetry_credentials.sql`
  - `004_bind_telemetry_credentials.sql`

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

## Telemetry credential table

`003_create_telemetry_credentials.sql` adds `telemetry_credentials`, independent of `auth_credentials`:

- `id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1)` (one fixed row)
- `token TEXT NOT NULL`, `accessor TEXT NOT NULL`
- `expires_at_unix_seconds INTEGER NOT NULL`, `stored_at_unix_seconds INTEGER NOT NULL`
- `created_at` / `updated_at` with the same defaults as `auth_credentials`, plus the `telemetry_credentials_set_updated_at` trigger

`004_bind_telemetry_credentials.sql` adds `user_id` and `workspace_id` (`TEXT NOT NULL DEFAULT ''`) to `telemetry_credentials` and creates `telemetry_repository_bindings` (`repository_id TEXT PRIMARY KEY`, `user_id`, `workspace_id`, `bound_at_unix_seconds`). Rows predating `004` keep empty identity values and are never returned.

Storage operations live in `cli/src/services/telemetry_credential_storage.rs`: `save_telemetry_credential` is a single-statement `INSERT OR REPLACE` of the fixed row (last complete writer wins), `load_telemetry_credential` returns the row only when token, accessor, and identity are non-empty, the expiry is non-negative and in the future, and the stored WorkOS session's access-token `(sub, workspace_id)` claims equal the row's identity (it never mints or refreshes), `load_repository_telemetry_credential` additionally requires a matching repository binding, `bind_repository_telemetry` records a binding for the current credential identity, and `delete_telemetry_credential` removes the row. `TelemetryCredential` has a redacted `Debug`, and storage errors use fixed messages without secret values. Saving or deleting never touches `auth_credentials`. Managed OTLP export reads this row at CLI startup through `load_telemetry_credential` and uses only the token as bearer. The shared `AuthDb` singleton comes from `token_storage::get_auth_db`. `sce auth logout` deletes both the `auth_credentials` row and the telemetry row; see [telemetry-credential.md](telemetry-credential.md) for lifecycle and security boundaries.

## Concurrency and schema readiness

- Many short-lived `sce` processes read `auth.db` at once: every admitted Claude hook resolves the telemetry credential at startup. `EncryptedTursoDb::new()` therefore opens with Turso multiprocess WAL, alongside encryption, so concurrent readers share the file. Multiprocess WAL creates `auth.db-wal` and the `auth.db-tshm` coordination file next to the database.
- A current schema is opened read-only: no `CREATE TABLE IF NOT EXISTS __sce_migrations` or other DDL runs when every embedded migration ID is already recorded. A missing or outdated DB is migrated in one `BEGIN IMMEDIATE` transaction, with a retry only for genuine `Busy`. A partially migrated schema fails with the failing migration ID rather than being treated as current. Details: [shared-turso-db.md](shared-turso-db.md).
- `load_telemetry_credential` (the managed-telemetry lookup used by every command and admitted hook) is an existing-DB read path: when `auth.db` does not exist it returns no credential without creating the file or an OS-keyring key. Only `sce auth`, `sce setup`, and other writers create the DB. This also keeps concurrent hooks on a never-logged-in machine from racing to create it.
- The auth DB busy timeout is 1000 ms. Hook reads stay bounded by `CREDENTIAL_LOOKUP_DEADLINE` (2s), which aborts the lookup regardless of busy waits or retries.
- Regression coverage (`cli/src/services/auth_db/multiprocess_tests.rs`, child processes of the test binary with sandboxed `HOME`/`XDG_*` and an env-provided `SCE_AUTH_DB_ENCRYPTION_KEY`; never the real DB or keyring): a reader succeeds while another process holds the DB open; 32 reader processes × 3 rounds all resolve the credential; a current-schema reader succeeds while another process holds `BEGIN IMMEDIATE`, which proves it performs no migration write; a telemetry lookup on a missing DB returns none and creates nothing; 4 processes concurrently migrating an existing schemaless DB record each migration once and pass `PRAGMA integrity_check`; an outdated schema upgrades and a partial one fails clearly and rolls back; 32 concurrent hook processes export correlated `sce.hook` spans.

### Upstream Turso multiprocess race

Under heavy load, Turso 0.8.1 multiprocess WAL has an internal race on concurrent cold opens that involve writes. Stress runs (6 parallel copies of the suite, with 32 processes creating or migrating the same DB) occasionally failed: `short read on page 3`, a `turso_assert` panic `reader slot released by non-owner` in `shared_wal_coordination.rs`, or, in the Nix sandbox, a writer that stayed `database is locked` through all 10 migration attempts (about 49 s). The read-only paths never failed across about 3,800 cold-opening reader and hook processes, and that is all hooks do on a current schema. The migration regression therefore uses 4 concurrent migrators, which was 0 failures in 48 parallel stress runs. Concurrent creation and migration by many writers is left as a documented upstream limitation. It affects first-time initialization only, and the Agent Trace DB already runs in the same Turso mode.

### Mixed-version rollout risk

Binaries from before this change open `auth.db` in legacy (exclusive-lock) WAL mode. A probe on the pinned Turso 0.8.1 with the same encrypted builder showed that Turso's mixed-mode guards do not fire on this path. With a legacy holder, a multiprocess process opens and writes, and with a multiprocess holder, a legacy process opens and writes; neither gets a `LockingError`. When both modes hold the DB and both write, the last committer's stale WAL view wins and the other process's committed rows are silently lost, while `PRAGMA integrity_check` still reports `ok`. A legacy holder running only the no-op `CREATE TABLE IF NOT EXISTS` lost nothing, and sequential use is safe: after all multiprocess processes exit, the legacy `sce` binary opens, decrypts, reads, and writes the DB, and integrity stays `ok`.

Hooks never write `auth.db`. Writers are `sce auth login`/`logout`, `sce setup` repository enrollment, telemetry-credential issuance, and the `sce sync` token refresh. So the hazard needs an old-version and a new-version `sce` process alive at the same time with at least one of them writing. Rollout: replace the binary while no `sce auth`, `sce setup`, or `sce sync` command is running, and do not keep two `sce` versions on `PATH` used by the same account. Short-lived old hook processes that are mid-flight during an upgrade only read. Closing the hazard in code would need a cross-mode lock guard (for example a shared `fcntl` lock held by new processes, which conflicts with the legacy exclusive lock). That needs a direct `rustix`/`libc` dependency and is not part of this change.

## Lifecycle integration

`AuthDbLifecycle` is registered in `cli/src/services/auth_db/lifecycle.rs` following the existing DB lifecycle pattern:
- `diagnose` collects auth DB path health problems.
- `fix` bootstraps missing auth DB parent directory.
- `setup` calls `AuthDb::new()` to initialize the encrypted database.
- `LifecycleProviderId::AuthDb` is the provider identifier.
- The lifecycle provider is registered in deterministic order: config → local_db → auth_db → agent_trace_db → hooks.

## Token storage integration

- `cli/src/services/token_storage.rs` uses `AuthDb` through async `save_tokens`, `load_tokens`, and `delete_tokens` operations and a `tokio::sync::OnceCell<Result<AuthDb, String>>`. Completed initialization success or failure is cached; cancellation before initialization completes leaves the cell uninitialized.
- Auth commands and the generic async control-plane credential store directly await token persistence on the application runtime. Database operations do not cross a `spawn_blocking` boundary; the OS keyring adapter retains its platform-specific synchronous behavior where required.
- `token_file_path()` returns the auth DB path from `auth_db_path()` instead of a JSON file path.
- `TokenStorageError` exposes `PathResolution` and `Database` variants; former `Io`, `Serialization`, `CorruptedTokenFile`, and `Permission` variants have been removed.
- No JSON file I/O remains in `token_storage.rs`.
- The `auth_credentials` row uses constant integer ID `1` for single-row token storage.
- Encryption is required: `encryption_key::get_or_create_encryption_key()` first honors `SCE_AUTH_DB_ENCRYPTION_KEY` by deriving a deterministic 64-character hex Turso key from the non-empty env-secret text, then falls back to OS credential-store keyring get-or-create behavior when the env var is absent. No plaintext auth DB fallback exists. Missing/unavailable credential-store failures surface as `TokenStorageError::Database` and include platform-specific remediation for Linux system keyring/Secret Service, macOS Keychain, Windows Credential Store, or unsupported platforms, always mentioning `SCE_AUTH_DB_ENCRYPTION_KEY` for headless/CI use without printing the secret value.

See also: [shared-turso-db.md](shared-turso-db.md), [../cli/default-path-catalog.md](../cli/default-path-catalog.md), [../context-map.md](../context-map.md)
