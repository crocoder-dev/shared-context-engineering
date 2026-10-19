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
- Telemetry credential lookups are existing-DB read paths. Both `load_telemetry_credential` (the managed-telemetry lookup used by every command and admitted hook) and `load_repository_telemetry_credential` (the `sce auth telemetry-headers` lookup that Claude Code's `otelHeadersHelper` runs) go through `token_storage::get_existing_auth_db`. That helper reuses the process `AuthDb` singleton when it is already open. Otherwise it calls `EncryptedTursoDb::open_existing_at`, which opens the database file without `O_CREAT` before resolving the encryption key. An absent file returns no credential, with no DB file, `-wal`/`-tshm` file, encryption key, or migration created. The existence check is the `open()` itself, not a separate `Path::exists()`. Path-resolution errors stay `TokenStorageError::PathResolution`. Only `sce auth`, `sce setup`, and other writers create the DB. This also keeps concurrent hooks and header-helper calls on a never-logged-in machine from racing to create it.
- The auth DB busy timeout is 1000 ms. Hook reads stay bounded by `CREDENTIAL_LOOKUP_DEADLINE` (2s), which aborts the lookup regardless of busy waits or retries.
- Regression coverage (`cli/src/services/auth_db/multiprocess_tests.rs`, child processes of the test binary with sandboxed `HOME`/`XDG_*` and an env-provided `SCE_AUTH_DB_ENCRYPTION_KEY`; never the real DB or keyring): a reader succeeds while another process holds the DB open; 32 reader processes × 3 rounds all resolve the credential; a current-schema reader succeeds while another process holds `BEGIN IMMEDIATE`, which proves it performs no migration write; both telemetry lookups on a missing DB return none and create nothing (the repository lookup runs with an empty `SCE_AUTH_DB_ENCRYPTION_KEY`, which fails any key resolution, so it also proves no key is resolved or created); 32 concurrent `sce auth telemetry-headers` runs (through `app::run`, inside a Git repository) on a missing DB all fail closed with `Telemetry credential is unavailable.` and leave no `auth.db*` file; the helper prints the bearer only for the bound repository; 4 processes concurrently migrating an existing schemaless DB record each migration once and pass `PRAGMA integrity_check`; an outdated schema upgrades and a partial one fails clearly and rolls back; 32 concurrent hook processes export correlated `sce.hook` spans. The mixed-version tests are listed under [Mixed-version compatibility lock](#mixed-version-compatibility-lock).

### Upstream Turso multiprocess race

Under heavy load, Turso 0.8.1 multiprocess WAL has an internal race on concurrent cold opens that involve writes. Stress runs (6 parallel copies of the suite, with 32 processes creating or migrating the same DB) occasionally failed: `short read on page 3`, a `turso_assert` panic `reader slot released by non-owner` in `shared_wal_coordination.rs`, or, in the Nix sandbox, a writer that stayed `database is locked` through all 10 migration attempts (about 49 s). The read-only paths never failed across about 3,800 cold-opening reader and hook processes, and that is all hooks do on a current schema. The migration regression therefore uses 4 concurrent migrators, which was 0 failures in 48 parallel stress runs. Concurrent creation and migration by many writers is left as a documented upstream limitation. It affects first-time initialization only, and the Agent Trace DB already runs in the same Turso mode.

### Mixed-version compatibility lock

SCE binaries from before multiprocess WAL (every release through `v0.4.0-pre-alpha-v11`, on Turso 0.8.1 or 0.7.0) open `auth.db` in legacy WAL mode. In that mode, Turso's `UnixFile::lock_file` takes a non-blocking classic POSIX record lock, `fcntl(F_SETLK, F_WRLCK)` over the whole file (`l_start = 0`, `l_len = 0`). It holds that lock while the database is open. Turso's own mixed-mode guards (`reject_live_*` in `turso_core::Database::resolve_default_storage`) never run for `turso::Builder`. `turso_sdk_kit` opens the file itself and calls `Database::open_async`, which skips them. Multiprocess mode opens the DB file with `NoLock`, so nothing stopped the two modes from overlapping. Without a guard, when an old and a new process both held the DB and both wrote, one process's committed rows were silently lost while `PRAGMA integrity_check` still reported `ok`. The regression test reproduced this: with the guard disabled, 24 of 72 committed rows were lost.

Every `EncryptedTursoDb` open now takes a shared compatibility lock (`cli/src/services/db/open_lock.rs`) on the `auth.db` file. It is acquired inside the connection-open retry loop, before Turso opens the file, and held for the life of the `EncryptedTursoDb` value:

- Linux: `fcntl(F_OFD_SETLK, F_RDLCK)`, an open-file-description lock on the whole file, on a descriptor the `EncryptedTursoDb` owns. Linux reports conflicts between OFD locks and classic POSIX record locks. So the new read lock and the legacy write lock exclude each other in both directions, while any number of new processes share the read lock. The lock belongs to SCE's own descriptor, so Turso opening or closing its descriptors for `auth.db` cannot release it. It is released when the `EncryptedTursoDb` is dropped or the process exits.
- macOS: `flock(LOCK_SH | LOCK_NB)` on the same SCE-owned `auth.db` descriptor. macOS has no public OFD lock, and a classic `F_SETLK` record lock is process-scoped: closing *any* descriptor for `auth.db` in the process released it, so the earlier `F_SETLK` `F_RDLCK` implementation could silently drop the guard while the `EncryptedTursoDb` was still open. A `flock` lock belongs to the open file description, so unrelated opens and closes of `auth.db` in the same process leave it in place. On macOS, `flock` and `fcntl` locks share one lock table and conflict across owners: the shared `flock` and the legacy whole-file `F_SETLK` `F_WRLCK` exclude each other in both directions (`EAGAIN`/`EWOULDBLOCK`), while any number of new processes share the `flock`. Turso's multiprocess open uses `NoLock` on the DB file and skips its legacy-WAL probe on the `turso::Builder` path, so it never requests a conflicting lock in the same process. Verified on macOS 15.4.1 (Darwin 24.4.0, arm64) with separate processes; see the macOS tests below.
- Other Unix (not Linux or macOS): `fcntl(F_SETLK, F_RDLCK)`, a classic process-scoped record lock. It conflicts with the legacy lock in the same way, but POSIX releases it when the process closes any descriptor for `auth.db`. Not built or tested on those platforms.
- Windows: no compatibility lock. Mixed versions on Windows remain unprotected.

Resulting coexistence on Linux (and on macOS through the test suite, without the real pre-fix binary), verified with separate processes, isolated encrypted databases, a legacy child that opens with the exact pre-fix builder configuration on the pinned Turso 0.8.1, and the real pre-fix `sce` binary:

| First holder | Second process | Result |
| --- | --- | --- |
| legacy | new read (telemetry lookup) or write (`AuthDb::new`) | Rejected after the connection-open retries (under 1 s): `auth DB database at <path> is open in an older SCE process that does not support shared access`. Commands surface the stable `Authentication storage is unavailable` error, and hooks fall back to Noop with reason `database_error`. Hook behavior is otherwise unchanged. |
| new | legacy read or write | Rejected immediately by Turso: `Failed locking file '<path>'. File is locked by another process` |
| new | new read or write | Shared. Writes stay serialized by Turso multiprocess WAL and the 1000 ms busy timeout. |
| legacy and new, started together | concurrent writes | Whichever opens first proceeds. The other is rejected or opens after the first exits. No committed row is lost. |
| none (all new processes exited) | legacy | Opens, decrypts, reads, and writes normally |

`/proc/locks` shows the legacy holder as `POSIX ADVISORY WRITE <pid> … 0 EOF` and a new holder as `OFDLCK ADVISORY READ -1 … 0 EOF`. Released binaries need no change. Unsafe combinations fail explicitly instead of losing data.

Mixed-version tests (`multiprocess_tests.rs`), all verifying persisted `telemetry_repository_bindings` rows and the `telemetry_credentials`/`auth_credentials` rows plus `integrity_check` afterwards:

- A legacy holder makes a new telemetry lookup and a new writer fail explicitly. The legacy holder's own later write persists.
- A new holder makes a legacy open fail with the Turso lock error.
- 3 rounds of 1 legacy writer and 2 new writers start together. Every row any writer reported committed is persisted.
- 4 concurrent new writers all commit all 32 rows.
- After every new process exits, a legacy process reopens the DB, reads the credential, and writes.

macOS-only tests (`#[cfg(target_os = "macos")]`, separate processes, raw `fcntl(F_SETLK, F_WRLCK)` children that take the exact legacy lock as well as legacy Turso opens):

- `macos_new_shared_lock_rejects_legacy_exclusive_lock_until_released`: a new holder makes the raw legacy lock fail with `EAGAIN` and the legacy Turso open fail; after the holder exits, both succeed.
- `macos_legacy_exclusive_lock_rejects_new_shared_lock_until_released`: a raw legacy lock holder makes a new lookup and a new writer fail with the shared-access error; after it exits, a new reader resolves the credential.
- `macos_new_shared_lock_survives_unrelated_descriptor_close`: the new holder opens and closes a second `auth.db` descriptor after `AuthDb::new`; legacy locks and opens are still rejected. Against the old `F_SETLK` `F_RDLCK` implementation this test fails because the legacy lock is granted.
- `macos_new_processes_share_the_lock_concurrently`: two new holders (one of them after an unrelated close) and 4 new writers run together, every write commits, and the legacy lock is still rejected; after they exit, a legacy process reopens and writes.

Remaining limits: a mixed old/new deployment is still not usable at the same time, because one side fails while the other holds the DB. Upgrade by replacing every `sce` binary used by the account. Old hook processes still in flight during the switch make new processes fail (and hooks fall back to Noop) until they exit; they no longer lose data. Turso reopens `auth.db` by path after the lock is taken, so a file that something outside SCE deletes in that window would be recreated empty. SCE never deletes `auth.db`.

## Lifecycle integration

`AuthDbLifecycle` is registered in `cli/src/services/auth_db/lifecycle.rs` following the existing DB lifecycle pattern:
- `diagnose` collects auth DB path health problems.
- `fix` bootstraps missing auth DB parent directory.
- `setup` only creates the auth DB parent directory. It never creates, opens, or migrates `auth.db` and never resolves an encryption key, so `sce setup` works offline, needs no login, and leaves an existing `auth.db` byte-for-byte unchanged. A missing `auth.db` is the normal unauthenticated state; only `sce auth login` creates it.
- `LifecycleProviderId::AuthDb` is the provider identifier.
- The lifecycle provider is registered in deterministic order: config → local_db → auth_db → agent_trace_db → hooks.

## Token storage integration

- `cli/src/services/token_storage.rs` uses `AuthDb` through async `save_tokens`, `load_tokens`, and `delete_tokens` operations and a `tokio::sync::OnceCell<AuthDb>`. Only a successful open is cached, so a failed non-interactive open never blocks a later interactive one in the same process.
- Open paths: `open_or_create_auth_db()` (`AuthDb::new()`, interactive key access) is used only to persist a new session or telemetry credential during `sce auth login`. Every other path uses `open_existing_auth_db(access)` (`EncryptedTursoDb::open_existing_at(path, access)`), which returns `None` for a missing `auth.db` and never creates the file or a key. `load_tokens(access)` and `save_tokens(token, access)` take the caller's `CredentialAccess`; `delete_tokens` is interactive and returns `false` for a missing DB.
- Auth commands and the generic async control-plane credential store directly await token persistence on the application runtime. Database operations do not cross a `spawn_blocking` boundary; the OS keyring adapter retains its platform-specific synchronous behavior where required.
- `token_file_path()` returns the auth DB path from `auth_db_path()` instead of a JSON file path.
- `TokenStorageError` exposes `PathResolution`, `Database`, and `CredentialAccessUnavailable` variants (the last one is mapped from the typed `encryption_key::CredentialAccessUnavailable` error and carries no secret); former `Io`, `Serialization`, `CorruptedTokenFile`, and `Permission` variants have been removed.
- No JSON file I/O remains in `token_storage.rs`.
- The `auth_credentials` row uses constant integer ID `1` for single-row token storage.
- Encryption is required: `encryption_key::get_or_create_encryption_key()` first honors `SCE_AUTH_DB_ENCRYPTION_KEY` by deriving a deterministic 64-character hex Turso key from the non-empty env-secret text, then falls back to the OS credential store when the env var is absent. No plaintext auth DB fallback exists.
- Credential access is explicit (`encryption_key::CredentialAccess`):
  - `Interactive`: used only by `sce auth login`, `logout`, and `whoami`. It may show the macOS Keychain authorization dialog. It generates and stores a new key only when the keyring reports `NoEntry` and `auth.db` does not exist. Any other read error is returned and never overwrites the stored key.
  - `NonInteractive`: used by Claude hooks, managed telemetry startup, `sce auth telemetry-headers`, and `sce sync` (including post-commit auto-sync). It only reads, never creates. On macOS it wraps the read in `SecKeychain::disable_user_interaction()` (`SecKeychainSetUserInteractionAllowed(false)`, the UI switch for the legacy file-based keychain API that `apple-native-keyring-store`'s `keychain` feature uses through `SecKeychainFindGenericPassword`). An item the binary is not authorized for, or a locked keychain, fails with `errSecInteractionNotAllowed` instead of prompting. Every failure becomes `CredentialAccessUnavailable`. `kSecUseAuthenticationUIFail` and `LAContext.interactionNotAllowed` do not apply, because they belong to the `SecItem`/data-protection keychain and LocalAuthentication-protected items, which this adapter does not use.
  - The interaction switch is process-wide, so all credential-store reads and writes in one process are serialized by a mutex. Interaction is turned back on before the next caller runs.
  - Linux Secret Service and Windows have no equivalent switch in the adapter. `NonInteractive` there only guarantees no key creation. Missing/unavailable credential-store failures surface as `TokenStorageError::Database` and include platform-specific remediation for Linux system keyring/Secret Service, macOS Keychain, Windows Credential Store, or unsupported platforms, always mentioning `SCE_AUTH_DB_ENCRYPTION_KEY` for headless/CI use without printing the secret value.

See also: [shared-turso-db.md](shared-turso-db.md), [../cli/default-path-catalog.md](../cli/default-path-catalog.md), [../context-map.md](../context-map.md)
