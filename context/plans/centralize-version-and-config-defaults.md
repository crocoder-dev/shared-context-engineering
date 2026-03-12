# Plan: Centralize Versioning and Config Defaults

## Change summary

Align three independent cleanup improvements:
1. Make `cli/src/services/config.rs` use `dirs` for platform paths (already a dependency, used in `token_storage.rs`).
2. Change default `log_level` from `info` to `error` across config resolution, observability, and documentation.
3. Centralize app and schema versioning via `.version` file and `Cargo.toml`, eliminating hardcoded `0.1.0` strings in code, build, and docs.

## Success criteria

- `cli/src/services/config.rs` uses `dirs::state_dir` / `dirs::data_dir` for global config path resolution, matching `token_storage.rs` style.
- Default `log_level` is `error` in runtime behavior and all contracts/docs reflect this.
- `.version` file exists at repo root with content `0.1.0`.
- `Cargo.toml` version is the single Rust source of truth; `cli/flake.nix` reads version from `.version`.
- `cli/src/services/version.rs` continues using `CARGO_PKG_VERSION` (no change needed).
- `cli/src/services/agent_trace.rs` reads app version at compile time via `env!("CARGO_PKG_VERSION")` instead of hardcoding `TRACE_VERSION`.
- All context files and contracts referencing `0.1.0` for app/schema version are updated or removed.
- `nix flake check` and `nix run .#pkl-check-generated` pass.

## Constraints and non-goals

- No changes to runtime behavior beyond log-level default and path-resolution source.
- No changes to CLI command surface or output schemas.
- Do not modify CI workflows beyond version-source alignment.
- Do not introduce runtime file reads for `.version`; keep it compile-time where possible.

## Task stack

- [x] T01: Use `dirs` for config global path resolution (status:done)
  - Task ID: T01
  - Goal: Replace manual platform-specific path logic in `config.rs` with `dirs` crate calls, matching `token_storage.rs` patterns.
  - Boundaries (in/out of scope):
    - In: `cli/src/services/config.rs` `resolve_default_global_config_path`, related test helpers.
    - In: Update `context/cli/config-precedence-contract.md` if path-resolution wording changes.
    - Out: No changes to `token_storage.rs` (already uses `dirs`).
  - Done when: `config.rs` compiles, tests pass, `nix flake check` green.
  - Verification notes (commands or checks): `cargo test --manifest-path cli/Cargo.toml`, `nix flake check`.
  - Completed: 2026-03-12
  - Evidence:
    - `cargo test --manifest-path cli/Cargo.toml config` passed (28 tests)
    - `cargo clippy --manifest-path cli/Cargo.toml --bin sce --tests -- -D warnings` passed
    - `nix flake check` passed after switching config global path resolution to direct `dirs`-based platform lookup

- [x] T02: Change default `log_level` to `error` (status:done)
  - Task ID: T02
  - Goal: Update default log level from `info` to `error` in code, tests, and docs.
  - Boundaries (in/out of scope):
    - In: `cli/src/services/observability.rs` default, `cli/src/services/config.rs` default, related tests.
    - In: `context/cli/config-precedence-contract.md`, `context/glossary.md`, `context/overview.md` wording.
    - In: Test assertions that assert `info` default.
    - Out: No changes to env-var names or allowed level values.
  - Done when: All code/tests/docs say default is `error`, tests pass, `nix flake check` green.
  - Verification notes (commands or checks): `grep -R 'default.*info' cli/src/services/` returns no hits; `nix flake check`.
  - Completed: 2026-03-12
  - Evidence:
    - `cargo test --manifest-path cli/Cargo.toml` passed after updating config + observability defaults and file-sink tests
    - `cargo build --manifest-path cli/Cargo.toml` passed
    - `cargo clippy --manifest-path cli/Cargo.toml --bin sce --tests` passed
    - `nix run .#pkl-check-generated` passed
    - `nix flake check` passed

- [x] T03: Add `.version` file and align `cli/flake.nix` (status:done)
  - Task ID: T03
  - Goal: Create `.version` with `0.1.0`, update `cli/flake.nix` to read version from file instead of hardcoding.
  - Boundaries (in/out of scope):
    - In: New file `.version` at repo root.
    - In: `cli/flake.nix` `version` fields (lines ~45, ~65) read from `.version`.
    - Out: No changes to `Cargo.toml` version value yet (T04).
  - Done when: `nix eval .#packages.<system>.sce.version` matches `.version`, `nix flake check` passes.
  - Verification notes (commands or checks): `cat .version` shows `0.1.0`; `nix flake check`.
  - Completed: 2026-03-12
  - Evidence:
    - `.version` added at repo root with `0.1.0`
    - `nix eval --raw 'path:/home/davidabram/repos/shared-context-engineering/master?dir=cli#packages.x86_64-linux.sce.version'` returned `0.1.0`
    - `nix run .#pkl-check-generated` passed
    - User confirmed `nix flake check` passed

- [x] T04: Remove hardcoded versions in Rust code (status:done)
  - Task ID: T04
  - Goal: Replace `TRACE_VERSION = "0.1.0"` with compile-time `CARGO_PKG_VERSION`, verify schema/docs alignment.
  - Boundaries (in/out of scope):
    - In: `cli/src/services/agent_trace.rs` constant, related tests.
    - In: Context files that reference `0.1.0` as the Agent Trace schema version.
    - In: `context/sce/agent-trace-implementation-contract.md`, `agent-trace-schema-adapter.md`, `agent-trace-payload-builder-validation.md`.
    - Out: No changes to JSON schema files (if any external).
  - Done when: No `0.1.0` hardcodes remain in `cli/src/`, tests pass, context docs say "follows app version".
  - Verification notes (commands or checks): `grep -R '0\.1\.0' cli/src/` shows no matches; `nix flake check`.
  - Completed: 2026-03-12
  - Evidence:
    - `rg "0\.1\.0" cli/src` returned no matches after switching `TRACE_VERSION` to `env!("CARGO_PKG_VERSION")`
    - `cargo test --manifest-path cli/Cargo.toml agent_trace` passed (7 tests)
    - `cargo clippy --manifest-path cli/Cargo.toml --bin sce --tests` passed
    - `cargo build --manifest-path cli/Cargo.toml` passed
    - `nix run .#pkl-check-generated && nix flake check` passed

- [x] T05: Remove hardcoded versions in CI/workflows and context (status:done)
  - Task ID: T05
  - Goal: Clean up remaining `0.1.0` references in workflows and non-Rust context files.
  - Boundaries (in/out of scope):
    - In: `.github/workflows/release-agents.yml` initial tag fallback.
    - In: `context/glossary.md` reference to `0.1.0` in schema validation patch note.
    - In: Any other context files with version hardcodes found by grep.
  - Out: No changes to test fixtures that intentionally test version parsing.
  - Done when: `grep -R '0\.1\.0' --include='*.yml' --include='*.md' context/ .github/` returns only intentional references (if any).
  - Verification notes (commands or checks): `grep -R '0\.1\.0' context/ .github/` audited; `nix flake check`.
  - Completed: 2026-03-12
  - Evidence:
    - `.github/workflows/release-agents.yml` now derives the initial release tag from `.version` instead of hardcoding `v0.1.0`
    - `0.1.0` audit across `context/` and `.github/` returned plan-history references only; no non-plan context or workflow matches remained
    - `nix run .#pkl-check-generated` passed
    - `nix flake check` passed

- [x] T06: Validation and context sync (status:done)
  - Task ID: T06
  - Goal: Final verification pass, update context-map if needed, confirm all checks green.
  - Boundaries (in/out of scope):
    - In: Run `nix run .#pkl-check-generated`, `nix flake check`.
    - In: Update `context/overview.md` and `context/glossary.md` if log-level default or version-centralization wording needs entry.
    - In: Update `context/context-map.md` only if new context files were created (none expected).
  - Out: No code changes.
  - Done when: All checks pass, context files reflect current state.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated && nix flake check`.
  - Completed: 2026-03-12
  - Evidence:
    - `nix run .#pkl-check-generated` passed (`Generated outputs are up to date.`)
    - `nix flake check` passed (all `cli-tests`, `cli-clippy`, `cli-fmt`, and `pkl-parity` checks evaluated and built successfully)
    - Root context sync corrected one stale dependency-baseline sentence in `context/architecture.md` so shared context matches current code truth
    - After the context fix, `nix run .#pkl-check-generated && nix flake check` passed again

## Final validation report

- Commands run:
  - `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
  - `nix flake check` -> exit 0 (`cli-tests`, `cli-clippy`, `cli-fmt`, `pkl-parity` built successfully)
  - `nix run .#pkl-check-generated && nix flake check` -> exit 0 after context-sync correction
- Failed checks and follow-ups:
  - None during final validation
- Context verification:
  - Shared files reviewed: `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, `context/context-map.md`
  - One stale duplicate dependency-baseline sentence in `context/architecture.md` was corrected; no new context files were needed
- Success criteria verification summary:
  - `config.rs` global config resolution uses `dirs`-backed platform paths
  - Default `log_level` is `error` in runtime contracts and context
  - Repo-root `.version` exists and `cli/flake.nix` reads version from it
  - Rust app/schema versioning follows `CARGO_PKG_VERSION` without hardcoded `0.1.0` in runtime code
  - Context/workflow references to version centralization and log-level defaults reflect current state
  - Required verification commands passed
- Temporary scaffolding:
  - None left behind
- Residual risks:
  - None identified beyond normal dirty-worktree warnings during local Nix evaluation

## Assumptions

- `.version` will be a single-line file with just the semver string (no newline or trailing whitespace).
- `cli/flake.nix` can read `.version` at eval time using `builtins.readFile` and `lib.strings.trim`.
- Agent Trace schema version is defined to equal the app version; no independent schema versioning is required.
