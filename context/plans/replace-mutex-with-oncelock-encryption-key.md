# Replace Mutex<bool> with OnceLock<bool> in encryption_key.rs

## 1) Change summary

Replace the `Mutex<bool>` one-time credential-store initialization guard in `cli/src/services/db/encryption_key.rs` with `std::sync::OnceLock<bool>` plus a stable retrying initialization guard. The current `Mutex<bool>` is race-prone: if `set_default_store` panics while the lock is held, the `Mutex` is poisoned, and every subsequent call to `ensure_default_store()` returns a poison error — permanently breaking encryption key access for the lifetime of the process.

The original plan targeted `OnceLock::get_or_try_init`, but implementation showed it remains unstable in the effective stable toolchain. The approved implementation uses stable `OnceLock<bool>` with an atomic in-progress guard: on both panic and error, the `OnceLock` stays uninitialized so the **next caller retries** — no poison risk. It also preserves the current fallible error-propagation pattern (errors from credential store setup flow to the caller).

## 2) Success criteria

- No `Mutex<bool>` in `ensure_default_store()` or anywhere in `encryption_key.rs`.
- Stable `OnceLock<bool>` is used for one-time credential store registration, with a retrying in-progress guard while `OnceLock::get_or_try_init` remains unstable in the effective stable toolchain.
- Fallible error propagation preserved: credential store init errors flow to `get_or_create_encryption_key` callers (i.e., `EncryptedTursoDb::new()`).
- Panic during init no longer permanently breaks subsequent calls — the `OnceLock` stays uninitialized and retries on next invocation.
- Outdated comment about `OnceLock::get_or_try_init` instability updated (both the API and toolchain version).
- `nix flake check` passes (CLI tests, clippy, fmt).
- All existing unit tests in `encryption_key.rs` pass (`hex_encode`, `generate_key`).

## 3) Constraints and non-goals

**Constraints:**
- Use only `std::sync::OnceLock` — no new crate dependencies.
- No behavioral change to `get_or_create_encryption_key` public API.
- Thread safety and one-time-initialization semantics preserved.
- Must compile on the existing Rust 1.93.1 toolchain.

**Non-goals:**
- Not changing `get_or_create_encryption_key` or any other public API.
- Not touching callers (`EncryptedTursoDb::new()`, `mod.rs`).
- Not adding new error paths or changing existing error messages.

## 4) Task stack

- [x] T01: `Replace Mutex<bool> with OnceLock<bool> in ensure_default_store` (status:done)
  - Task ID: T01
  - Goal: Replace `static DEFAULT_STORE: Mutex<bool>` with `static DEFAULT_STORE: OnceLock<bool>` and rewrite `ensure_default_store()` to use stable retrying initialization semantics after `OnceLock::get_or_try_init` proved unstable in the effective toolchain. Update the doc comment to reflect the current approach.
  - Boundaries (in/out of scope):
    - **In scope:**
      - `encryption_key.rs`: replace the `DEFAULT_STORE` static, rewrite `ensure_default_store()`, update the doc comment on `DEFAULT_STORE`.
      - Ensure the file compiles, passes `cargo clippy`, and passes its existing unit tests.
    - **Out of scope:**
      - `cli/src/services/db/mod.rs`, callers of `get_or_create_encryption_key`, `AuthDb`, or any other module.
      - Changing error messages or error propagation behavior.
      - Adding new tests beyond verifying the refactor doesn't break existing ones.
  - Done when:
    - `DEFAULT_STORE` is `static DEFAULT_STORE: OnceLock<bool> = OnceLock::new();`.
    - `ensure_default_store()` uses stable `OnceLock<bool>` plus an atomic in-progress guard, with the platform-specific `set_default_store` calls in `register_default_store()` and retry-after-error/panic semantics preserved.
    - The `use std::sync::Mutex;` import is removed (or replaced with `use std::sync::OnceLock;` if not already imported).
    - The doc comment on `DEFAULT_STORE` accurately describes the current approach (no mention of unstable APIs or outdated toolchain versions).
    - `cargo clippy` and `cargo fmt` produce no warnings in the changed file.
    - Existing unit tests (`test_hex_encode_*`, `test_generate_key_*`) pass.
  - Verification notes (commands or checks):
    - `nix develop -c sh -c 'cd cli && cargo test encryption_key -- --exact'`
    - `nix develop -c sh -c 'cd cli && cargo clippy'`
    - `nix develop -c sh -c 'cd cli && cargo fmt -- --check'`
  - Completion evidence:
    - Completed: 2026-06-07
    - Files changed: `cli/src/services/db/encryption_key.rs`
    - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt -- --check'` passed; `nix flake check` passed, including CLI tests, clippy, and fmt.
    - Notes: Direct `cargo test encryption_key` was blocked by repo bash policy preferring `nix flake check`; direct `cargo clippy` exceeded the shell timeout while dependency checking, but the flake `cli-clippy` derivation passed.

- [x] T02: `Validation and context sync` (status:done)
  - Task ID: T02
  - Goal: Run full repo validation (`nix flake check`), verify no regressions, and sync `context/` files to reflect the updated code.
  - Boundaries (in/out of scope):
    - **In scope:** `nix flake check`, `nix run .#pkl-check-generated`, context-sync for relevant context files.
    - **Out of scope:** Any code changes beyond verification and context updates.
  - Done when:
    - `nix flake check` passes with no failures.
    - `nix run .#pkl-check-generated` passes.
    - Context files referencing the `Mutex<bool>` or outdated toolchain commentary in `encryption_key.rs` are updated if needed (`context/sce/shared-turso-db.md`, `context/glossary.md`, `context/architecture.md`).
    - Plan checklist is fully checked off.
  - Verification notes (commands or checks):
    - `nix flake check`
    - `nix run .#pkl-check-generated`
  - Completion evidence:
    - Completed: 2026-06-07
    - Files changed: `context/plans/replace-mutex-with-oncelock-encryption-key.md`
    - Evidence: `nix flake check` passed; `nix run .#pkl-check-generated` passed and reported generated outputs are up to date.
    - Notes: Context references were checked against code truth; `context/sce/shared-turso-db.md`, `context/glossary.md`, and `context/architecture.md` already describe the stable `OnceLock<bool>` plus atomic retry guard behavior with no unresolved drift for this task.

## 5) Open questions

(None — all ambiguities resolved.)

## Validation Report

### Commands run

- `nix flake check` -> exit 0. Key output: `all checks passed!` (with dirty-tree warning expected because this plan's changes are uncommitted).
- `nix run .#pkl-check-generated` -> exit 0. Key output: `Generated outputs are up to date.`

### Success-criteria verification

- [x] No `Mutex<bool>` remains in `cli/src/services/db/encryption_key.rs` or `ensure_default_store()`; code uses `static DEFAULT_STORE: OnceLock<bool> = OnceLock::new();`.
- [x] Stable `OnceLock<bool>` plus `AtomicBool` in-progress guard is present in `encryption_key.rs`; `OnceLock::get_or_try_init` is not used.
- [x] Fallible error propagation is preserved: `ensure_default_store()` calls `register_default_store()?`, and `get_or_create_keyring_encryption_key()` propagates through `ensure_default_store()?`.
- [x] Retry-after-panic/error semantics are represented by setting `DEFAULT_STORE` only after successful registration and clearing `DEFAULT_STORE_INITIALIZING` in `DefaultStoreInitGuard::drop()`.
- [x] Context is synced: `context/sce/shared-turso-db.md`, `context/glossary.md`, `context/architecture.md`, and `context/context-map.md` describe the current stable `OnceLock<bool>` plus atomic retry guard behavior.
- [x] Full validation passed via `nix flake check`.
- [x] Generated-output parity passed via `nix run .#pkl-check-generated`.
- [x] Plan checklist is fully checked off (`T01`, `T02`).

### Temporary scaffolding

- No task-specific temporary scaffolding was introduced during T02.

### Residual risks

- None identified.
