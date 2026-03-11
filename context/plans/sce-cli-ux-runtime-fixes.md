# Plan: SCE CLI UX and Runtime Fixes

## Change Summary
Address four current CLI defects/regressions in the Rust `sce` binary: add actionable `sce setup` guidance when the target folder is not a git repository, expand `sce doctor` location reporting for config files and the local database, make bare `sce auth` print subcommand guidance similar to help output, and fix the `sce auth login` Tokio runtime panic so device-login can perform network I/O safely.

## Success Criteria
- [ ] `sce setup` detects a missing `.git` context before repository-hook setup work and tells the user to run `git init` and then rerun `sce setup`.
- [ ] `sce doctor` reports where config files live or are expected to live, and where the local database lives or is expected to live.
- [ ] Running `sce auth` without a nested subcommand prints subcommand-oriented guidance comparable to help output instead of a poor/empty experience.
- [ ] `sce auth login` no longer panics with `A Tokio 1.x context was found, but IO is disabled`; it either completes the WorkOS device flow or fails with normal SCE runtime guidance.

## Constraints and Non-Goals

**In Scope:**
- CLI parser/dispatch, runtime wiring, and user-facing text/JSON output changes needed for the four retained reported behaviors.
- Runtime initialization fixes required for auth login network I/O in the existing async architecture.
- Tests covering release-observability behavior, setup non-git validation, doctor location reporting, bare-auth help output, and auth login runtime wiring.
- Context sync for any command-surface or contract changes that become current code truth.

**Out of Scope:**
- New auth providers, token storage redesign, or broader auth UX redesign beyond the reported `sce auth` invocation/help behavior.
- New setup flows beyond the missing-git guidance.
- Database schema changes unrelated to reporting its resolved/expected location.

**Non-Goals:**
- Adding new top-level commands.
- Changing existing exit-code classes unless required by an already-established validation/runtime contract.
- Expanding `doctor` into a full config validator beyond path/location reporting.

## Assumptions
- “Release builds” refers to the packaged/release execution paths already documented for `cargo build --release` and `nix run ./cli#sce`.
- The missing `.git` case is specifically for setup modes that require a git repository context (including current hook-install flows), and should be handled with actionable user guidance rather than a panic or opaque git error.
- `sce doctor` should surface both discovered current paths and deterministic expected default paths when files/databases are absent.

## Task Stack

- [x] T01: Add non-git setup guidance (status:done)
  - Task ID: T01
  - Goal: Detect setup invocations that need a git repository and return explicit `git init` remediation when `.git`/git-root context is missing.
  - Boundaries (in/out of scope):
    - IN: Setup validation and user-facing error text for repository-required flows.
    - IN: Tests covering current-directory and `--repo` invocations against non-git folders.
    - OUT: Automatic repository initialization or setup behavior changes for already-valid git repos.
  - Done when:
    - `sce setup` in a non-git folder fails gracefully with guidance to run `git init` and then rerun `sce setup`.
    - Existing setup success paths and option-validation rules remain unchanged for valid repos.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml setup`
    - `cargo clippy --manifest-path cli/Cargo.toml -- -D warnings`
    - Manual check in a temp directory without `.git`: `sce setup --hooks`

- [x] T02: Expand doctor path reporting for config and database locations (status:done)
  - Task ID: T02
  - Goal: Teach `sce doctor` to report config-file locations and local database location alongside existing rollout readiness details.
  - Boundaries (in/out of scope):
    - IN: Text and machine-readable doctor output updates for global/local config paths and Agent Trace local DB path.
    - IN: Tests for present/absent path reporting and stable field names/messages.
    - OUT: Broader doctor remediation logic unrelated to reporting locations.
  - Done when:
    - `sce doctor` output identifies where config files live or are expected.
    - `sce doctor` output identifies where the local database lives or is expected.
    - Existing hook-readiness reporting remains intact.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml doctor`
    - `cargo run --manifest-path cli/Cargo.toml -- doctor --format json`

- [x] T03: Improve bare `sce auth` subcommand guidance (status:done)
  - Task ID: T03
  - Goal: Make `sce auth` without a nested subcommand present subcommand-oriented guidance similar to help output.
  - Boundaries (in/out of scope):
    - IN: Parser/dispatch/help-text behavior for the bare `auth` command group.
    - IN: Tests covering `sce auth`, `sce auth --help`, and nested auth help output consistency.
    - OUT: New auth subcommands or changes to login/logout/status semantics.
  - Done when:
    - `sce auth` shows available subcommands and actionable next steps.
    - `sce auth --help` remains coherent with the bare-command experience.
    - Existing auth subcommand parsing still succeeds unchanged.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml auth`
    - `cargo run --manifest-path cli/Cargo.toml -- auth`

- [x] T04: Fix auth login Tokio runtime I/O enablement (status:done)
  - Task ID: T04
  - Goal: Ensure `sce auth login` runs under a Tokio runtime configuration that enables network I/O for the WorkOS device flow.
  - Boundaries (in/out of scope):
    - IN: Runtime-builder and auth command wiring changes needed to support reqwest/Tokio TCP usage.
    - IN: Regression coverage proving `auth login` no longer panics when it reaches async network setup.
    - OUT: WorkOS API redesign, token schema changes, or unrelated async refactors.
  - Done when:
    - `sce auth login` no longer panics with the Tokio I/O-disabled error.
    - Auth login reaches normal runtime behavior (successful flow or expected SCE runtime failure with guidance).
    - Runtime changes do not regress existing sync/local-DB async behavior.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml auth_command`
    - Manual check: `sce auth login`

- [ ] T05: Validation, cleanup, and context sync (status:todo)
  - Task ID: T05
  - Goal: Validate the full fix set, update current-state context/contracts, and leave the repo aligned for follow-on sessions.
  - Boundaries (in/out of scope):
    - IN: Run targeted CLI tests plus repo-level required checks.
    - IN: Update relevant current-state context files for observability/setup/doctor/auth contract changes.
    - IN: Remove stale assumptions uncovered by implementation.
    - OUT: New feature work beyond documenting the implemented fixes.
  - Done when:
    - All targeted checks for T01-T04 pass.
    - `nix run .#pkl-check-generated` and `nix flake check` pass.
    - Relevant `context/` files reflect current code truth.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml`
    - `nix run .#pkl-check-generated`
    - `nix flake check`

## Open Questions
None.
