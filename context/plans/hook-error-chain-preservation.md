# hook-error-chain-preservation

## Change summary

Fix `error.to_string()` truncation of anyhow error chains in command dispatchers. The `hooks/command.rs` (and 4 other command files) use `error.to_string()` which renders only the outermost anyhow error via `{}` Display, silently discarding the full cause chain. The repo convention (established in `setup/command.rs`) is `format!("{error:#}")` which renders the full chain with `: ` separators per `anyhow-1.0.102/src/fmt.rs:8-14`. This prevents operators from seeing the underlying failure reason (e.g., git resolution, DB connection, permissions) behind errors like `"Failed to open Agent Trace DB for conversation-trace persistence."`.

## Success criteria

- `hooks/command.rs:12` uses `format!("{error:#}")` instead of `error.to_string()`, exposing the full anyhow chain in hook error diagnostics.
- The same fix is applied to `auth_command/command.rs`, `config/command.rs`, `doctor/command.rs`, and `version/command.rs` for consistency with the established pattern in `setup/command.rs`.
- `nix flake check` passes (all 13 checks: cli-tests, cli-clippy, cli-fmt, integrations-*, pkl-parity, npm-*, config-lib-*).
- No behavioral changes to hook runtime, doctor output, setup, or any other service — only richer diagnostic text when errors occur.

## Constraints and non-goals

- In scope: 5 command files, one-line pattern change each (`error.to_string()` → `format!("{error:#}")`).
- Out of scope: hardening the OpenCode plugin's `repoRoot` resolution, investigating the root cause of the Agent Trace DB open failure, fixing error truncation outside of `cli/src/services/*/command.rs`, changing error classification or rendering in `app_support.rs`.
- Out of scope: adding test coverage for error chain rendering.

## Assumptions

- The underlying Agent Trace DB open failure in OpenCode is a pre-existing environmental issue; this fix enables diagnosis by surfacing the full error.
- `anyhow::Error` `{:#}` alternate format preserves the chain for all error types produced in these command paths (ContextError chains, Turso/libsql errors, I/O errors).

## Tasks

- [x] T01: `Fix hooks/command.rs error chain truncation` (status: done)
  - Task ID: T01
  - Goal: Change `error.to_string()` to `format!("{error:#}")` in `cli/src/services/hooks/command.rs:12` so the full anyhow error chain (including the fast-path attempt failure added by the staged fix in `checkout/mod.rs`) is visible in hook error diagnostics.
  - Boundaries (in/out of scope): In — `cli/src/services/hooks/command.rs` line 12 only. Out — all other files, hook runtime logic, error classification.
  - Done when: Line 12 reads `.map_err(|error| ClassifiedError::runtime(format!("{error:#}")))`; `nix flake check` passes.
  - Completed: 2026-06-16
  - Files changed: `cli/src/services/hooks/command.rs` (line 12)
  - Evidence: `nix flake check` passed (4/4: cli-tests, cli-clippy, cli-fmt, pkl-parity)
  - Verification notes: `nix flake check`; review the changed line to confirm `format!("{error:#}")`.

- [x] T02: `Fix remaining command file error chain truncations` (status: done)
  - Task ID: T02
  - Goal: Apply the same `error.to_string()` → `format!("{error:#}")` fix to `auth_command/command.rs:11`, `config/command.rs:11`, `doctor/command.rs:12`, and `version/command.rs:11` for consistency with `setup/command.rs`.
  - Boundaries (in/out of scope): In — the 4 listed command files, one line each. Out — any other files, changes to command logic or error handling patterns.
  - Done when: All 4 files use `format!("{error:#}")`; `nix flake check` passes.
  - Completed: 2026-06-16
  - Files changed: `cli/src/services/auth_command/command.rs`, `cli/src/services/config/command.rs`, `cli/src/services/doctor/command.rs`, `cli/src/services/version/command.rs` (line 11-12 each)
  - Evidence: `nix flake check` passed (4/4: cli-tests, cli-clippy, cli-fmt, pkl-parity); grep confirms zero remaining `error.to_string()` in `cli/src/services/*/command.rs`.
  - Verification notes: `nix flake check`; review each file to confirm the pattern change.

- [x] T03: `Validation and cleanup` (status: done)
  - Task ID: T03
  - Goal: Run full validation, confirm all 5 files are correct, the staged fix's fast-path error context is now reachable through `{:#}` format, and context is synced.
  - Boundaries (in/out of scope): In — `nix flake check`, `nix run .#pkl-check-generated`, review of changed files, context sync. Out — additional code changes, live hook testing.
  - Done when: All checks pass, the full anyhow chain is structurally reachable from hook error diagnostics (confirmed via code review of `hooks/command.rs:12` → `hooks/mod.rs:250` → `checkout/mod.rs:198` chain), and context files are updated.
  - Completed: 2026-06-16
  - Files changed: none (validation-only task)
  - Evidence: `nix flake check` passed (all 13 checks); `nix run .#pkl-check-generated` passed (generated outputs up to date); zero `error.to_string()` remaining in `cli/src/services/*/command.rs`; error chain review confirms `hooks/command.rs:12` → `hooks/mod.rs:294-300` → `checkout/mod.rs:194-201` structural reachability.
  - Verification notes: `nix flake check` && `nix run .#pkl-check-generated`; confirm that `hooks/mod.rs:250` error context string `"Failed to open Agent Trace DB for conversation-trace persistence."` would now be followed by `: failed to initialize Agent Trace DB for checkout {id} at '{path}' (fast-path attempt: {error})` when the DB open fails.

## Validation Report

### Commands run

| Command | Exit code | Key output |
|---|---|---|
| `nix flake check` | 0 | `all checks passed!` (all 13 checks: cli-tests, cli-clippy, cli-fmt, integrations-install-tests, integrations-install-clippy, integrations-install-fmt, pkl-parity, npm-bun-tests, npm-biome-check, npm-biome-format, config-lib-bun-tests, config-lib-biome-check, config-lib-biome-format) |
| `nix run .#pkl-check-generated` | 0 | `Generated outputs are up to date.` |
| grep `error.to_string()` in `cli/src/services/*/command.rs` | — | Zero matches (confirmed none remaining in the 6 command files) |

### Scaffolding cleanup

None needed — no temporary scaffolding, debug code, or intermediate artifacts were introduced.

### Context verification

- **Root context files**: Verified — `overview.md`, `architecture.md`, `glossary.md`, `patterns.md` all reflect current code truth. No drift. Change classified as verify-only.
- **Domain files**: No new domain concepts introduced; no domain files created.
- **Glossary**: No new SCE-specific terminology; `{:#}` is standard anyhow behavior.

### Success-criteria verification

- [x] `hooks/command.rs:12` uses `format!("{error:#}")` — confirmed line reads `.map_err(|error| ClassifiedError::runtime(format!("{error:#}")))`
- [x] Same fix applied to `auth_command/command.rs:11`, `config/command.rs:11`, `doctor/command.rs:12`, `version/command.rs:11` — confirmed all 4 files use `format!("{error:#}")`
- [x] `nix flake check` passes (all 13 checks) — confirmed exit 0
- [x] No behavioral changes — confirmed: only the format string changed from `error.to_string()` to `format!("{error:#}")`; same error types, same code paths, same `ClassifiedError::runtime(...)` classification

### Error chain reachability confirmation

The `{:#}` format ensures the full anyhow chain is visible. Example chain:

```
hooks/command.rs:12  →  ClassifiedError::runtime(format!("{error:#}"))
hooks/mod.rs:300     →  .context("Failed to open Agent Trace DB for conversation-trace persistence.")
hooks/mod.rs:298     →  checkout/mod.rs:resolve_or_create_agent_trace_db_for_checkout(...)
checkout/mod.rs:196  →  .with_context(|| format!(
                           "failed to initialize Agent Trace DB for checkout {id} at '{path}' \
                            (fast-path attempt: {fast_error})"))
```

With `{:#}`, any DB open failure renders as:
`Failed to open Agent Trace DB for conversation-trace persistence.: failed to initialize Agent Trace DB for checkout {id} at '{path}' (fast-path attempt: {original_error})`

### Residual risks

- None identified. The fix is a one-line format-string change (no logic, control flow, or error classification changes). The same pattern has been in production in `setup/command.rs` for prior releases.

## Open questions

- None at this time.
