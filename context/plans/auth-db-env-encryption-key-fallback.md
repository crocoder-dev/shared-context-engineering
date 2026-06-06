# Auth DB env encryption key fallback

## 1) Change summary

When `sce setup` initializes the encrypted auth database on a machine without an accessible OS credential store/keyring, it currently fails because `AuthDb` relies exclusively on `encryption_key::get_or_create_encryption_key(...)`. Add an environment-variable fallback so headless/CI users can provide an auth DB encryption secret directly.

The new env var is `SCE_AUTH_DB_ENCRYPTION_KEY`. When present, SCE must use that secret instead of the OS keyring for auth DB encryption. When absent, SCE keeps the current keyring-backed behavior. If neither path works, setup/auth DB initialization should fail with platform-specific guidance, for example: `Could not access the Linux system keyring. For headless or CI use, set SCE_AUTH_DB_ENCRYPTION_KEY to provide the auth DB encryption secret.` Equivalent wording should mention macOS Keychain or Windows Credential Store on those platforms.

## 2) Success criteria

- `SCE_AUTH_DB_ENCRYPTION_KEY` is the canonical env var for bypassing keyring-backed auth DB encryption.
- When `SCE_AUTH_DB_ENCRYPTION_KEY` is present, auth DB encryption uses the env secret and does not initialize or read/write the OS keyring.
- The env secret accepts an arbitrary string; implementation deterministically derives the 64-character hex key required by Turso local encryption.
- A DB encrypted with the env-derived key can be reopened when the same env secret is present.
- Existing keyring-backed behavior remains unchanged when the env var is absent and keyring is available.
- If the env var is absent and keyring access fails, setup/auth DB initialization fails with platform-specific remediation that mentions `SCE_AUTH_DB_ENCRYPTION_KEY` for headless/CI use.
- No plaintext auth DB fallback is introduced.
- Full repo validation passes.

## 3) Constraints and non-goals

**Constraints:**

- Keep the auth DB encrypted in all modes.
- Use `SCE_AUTH_DB_ENCRYPTION_KEY` exactly as the env var name.
- Accept arbitrary env-secret text and derive a Turso-compatible 64-character hex key deterministically.
- Prefer env secret over keyring when the env var is present.
- Preserve the canonical auth DB path from `default_paths::auth_db_path()`.
- Keep auth DB migrations owned by `AuthDbSpec` and applied through the existing encrypted adapter.

**Non-goals:**

- No plaintext auth DB mode.
- No migration from keyring-backed encrypted DBs to env-secret-backed encrypted DBs, or vice versa.
- No interactive prompt for entering the secret.
- No storage of the env secret in config files, keyring, logs, setup output, or context artifacts.
- No changes to local DB or Agent Trace DB encryption status.
- No redesign of auth token schema or WorkOS auth behavior.

## Assumptions

- If an existing auth DB was created with one secret source and later opened with a different secret source/value, Turso open/connect may fail and should surface as an actionable auth DB encryption/key mismatch error rather than attempting recovery.
- The derivation can use existing crate dependencies where practical; if a new dependency is needed, it must be small, deterministic, and justified in the implementation task.
- Empty or whitespace-only `SCE_AUTH_DB_ENCRYPTION_KEY` values should be treated as invalid rather than silently falling back to keyring.

## 4) Task stack

- [x] T01: `Add env-secret auth DB encryption key source` (status:done)
  - Task ID: T01
  - Goal: Extend encryption-key resolution so `SCE_AUTH_DB_ENCRYPTION_KEY` takes precedence over keyring and arbitrary secret text is converted into Turso's required 64-character hex key.
  - Boundaries (in/out of scope):
    - **In scope:** `cli/src/services/db/encryption_key.rs`, focused unit tests for env precedence/validation/derivation, and minimal exported constants/helpers needed by callers or tests.
    - **Out of scope:** setup rendering, token storage changes, plaintext DB support, keyring behavior changes when the env var is absent.
  - Done when:
    - `SCE_AUTH_DB_ENCRYPTION_KEY` is read before keyring initialization.
    - Non-empty arbitrary env values deterministically produce a valid 64-character hex encryption key.
    - Empty or whitespace-only env values fail with actionable remediation.
    - Tests prove env-present code does not require keyring initialization and returns stable derived keys.
    - Existing key-generation/keyring unit tests still pass.
  - Verification notes (commands or checks):
    - `nix develop -c sh -c 'cd cli && cargo test encryption_key'`
    - `nix develop -c sh -c 'cd cli && cargo fmt -- --check'`
  - Completion evidence (2026-06-06):
    - Files changed: `cli/src/services/db/encryption_key.rs`
    - Implemented `SCE_AUTH_DB_ENCRYPTION_KEY` precedence before keyring initialization, deterministic SHA-256-to-hex derivation, and empty/whitespace validation.
    - User-requested follow-up removed generated T01 unit tests, so the plan's focused-test done check is not represented by new task-local tests.
    - Verification: `nix develop -c sh -c 'cd cli && cargo fmt'`; `nix flake check` passed. The planned targeted `cargo test encryption_key` command was blocked by the repository bash policy preferring `nix flake check`.

- [x] T02: `Improve missing-keyring remediation for auth DB setup` (status:done)
  - Task ID: T02
  - Goal: Update keyring failure diagnostics so setup/auth DB initialization tells users which platform credential store could not be accessed and how to use `SCE_AUTH_DB_ENCRYPTION_KEY` for headless/CI use.
  - Boundaries (in/out of scope):
    - **In scope:** platform-specific error text in `encryption_key.rs` and auth DB lifecycle/setup error propagation tests where practical.
    - **Out of scope:** changing setup command structure, adding prompts, changing non-auth DB lifecycle providers.
  - Done when:
    - Linux failure text mentions Linux system keyring/Secret Service and `SCE_AUTH_DB_ENCRYPTION_KEY`.
    - macOS failure text mentions macOS Keychain and `SCE_AUTH_DB_ENCRYPTION_KEY`.
    - Windows failure text mentions Windows Credential Store and `SCE_AUTH_DB_ENCRYPTION_KEY`.
    - Unsupported-platform text remains actionable and mentions the env var fallback if applicable.
    - Error messages do not print the env-secret value.
  - Verification notes (commands or checks):
    - `nix develop -c sh -c 'cd cli && cargo test encryption_key'`
    - `nix develop -c sh -c 'cd cli && cargo test auth_db'`
    - `nix develop -c sh -c 'cd cli && cargo fmt -- --check'`
  - Completion evidence (2026-06-06):
    - Files changed: `cli/src/services/db/encryption_key.rs`
    - Added shared platform-specific credential-store remediation text for Linux system keyring/Secret Service, macOS Keychain, Windows Credential Store, and unsupported platforms; all remediation mentions `SCE_AUTH_DB_ENCRYPTION_KEY` for headless/CI use and does not include env-secret values.
    - Applied the remediation text to credential-store initialization, keyring entry creation, key storage, and existing-DB/missing-keyring-entry errors.
    - User-requested follow-up removed generated T02 helper tests; non-current platform enum variants are compiled only for their target platform rather than using a production dead-code allowance.
    - Verification: targeted `cargo test encryption_key` and `cargo test auth_db` commands were blocked by the repository bash policy preferring `nix flake check`; `nix develop -c sh -c 'cd cli && cargo fmt -- --check'` passed after formatting and after test removal; `nix flake check` passed after implementation and after test removal.

- [ ] T03: `Verify auth/token storage with env-key encrypted DBs` (status:todo)
  - Task ID: T03
  - Goal: Add focused coverage proving auth DB initialization and token persistence work with `SCE_AUTH_DB_ENCRYPTION_KEY` set.
  - Boundaries (in/out of scope):
    - **In scope:** `cli/src/services/auth_db/` tests, `cli/src/services/token_storage.rs` tests if needed, and deterministic env-var test setup/cleanup.
    - **Out of scope:** WorkOS auth protocol changes, token schema changes, multi-account token rows, plaintext support.
  - Done when:
    - Auth DB can be created/opened with an env secret.
    - The same env secret reopens the DB and migrations remain idempotent.
    - Token save/load/delete works through the auth DB wrapper when the env secret is present.
    - Tests isolate env var state and do not leak `SCE_AUTH_DB_ENCRYPTION_KEY` across cases.
  - Verification notes (commands or checks):
    - `nix develop -c sh -c 'cd cli && cargo test auth_db'`
    - `nix develop -c sh -c 'cd cli && cargo test token_storage'`
    - `nix develop -c sh -c 'cd cli && cargo fmt -- --check'`

- [ ] T04: `Validation and context sync` (status:todo)
  - Task ID: T04
  - Goal: Run full validation and update durable context to describe the env-secret fallback for encrypted auth DB initialization.
  - Boundaries (in/out of scope):
    - **In scope:** full repo validation, generated-output parity check, updating relevant context files such as `context/sce/shared-turso-db.md`, `context/sce/auth-db.md`, `context/sce/setup-repo-local-config-bootstrap.md`, `context/overview.md`, `context/glossary.md`, and `context/context-map.md` if code truth changes require it.
    - **Out of scope:** new feature work or broad documentation rewrites unrelated to auth DB encryption key sourcing.
  - Done when:
    - `nix flake check` passes.
    - `nix run .#pkl-check-generated` passes.
    - Context files accurately state that auth DB remains encrypted and can source its encryption key from `SCE_AUTH_DB_ENCRYPTION_KEY` before falling back to the OS keyring.
    - Context files accurately state that no plaintext auth DB fallback exists.
    - Plan checklist is fully checked off.
  - Verification notes (commands or checks):
    - `nix flake check`
    - `nix run .#pkl-check-generated`

## 5) Open questions

(None — clarification resolved the env var name, arbitrary secret format, and keyring-unavailable remediation behavior.)
