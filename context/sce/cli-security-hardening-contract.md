# SCE CLI security hardening contract

## Scope

Task `sce-cli-agent-friendly-reliability-baseline` `T06` adds baseline security hardening for CLI diagnostics/logging and setup filesystem interfaces.

## Implemented behavior

- Sensitive values in user-facing diagnostics/log lines are redacted before emission.
- `sce setup --hooks --repo <path>` now canonicalizes and validates the supplied repository path before hook installation.
- Setup install flows now run explicit write-permission probes on target directories before staging/swap writes.

## Redaction contract

`cli/src/services/security.rs` provides `redact_sensitive_text(...)` and is applied to:

- top-level CLI error emission in `cli/src/app.rs`
- observability stderr/file sink output in `cli/src/services/observability.rs`
- git-command stderr diagnostics surfaced by setup hook flows in `cli/src/services/setup.rs`

Current redaction coverage includes:

- assignment-style secrets (`password=...`, `token=...`, `api_key=...`)
- JSON key/value secrets (for the same key set)
- `Authorization`/`Bearer` token forms

## Path and permission safety contract

`cli/src/services/setup.rs` enforces:

- `--repo` path must resolve to an existing directory
- repository path is canonicalized before hook setup operations
- setup install roots and hooks directories must pass a deterministic write probe before writes

Write probe behavior is owned by `ensure_directory_is_writable(...)` in `cli/src/services/security.rs`.

## Verification anchors

- `cargo test --manifest-path cli/Cargo.toml services::security::tests`
- `cargo test --manifest-path cli/Cargo.toml services::setup::tests`
- `cargo test --manifest-path cli/Cargo.toml services::observability::tests`
- `cargo test --manifest-path cli/Cargo.toml app::tests`
- `cargo check --manifest-path cli/Cargo.toml`
- `cargo build --manifest-path cli/Cargo.toml`
