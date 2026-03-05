# Plan: sce-nix-setup-hooks-integration-tests

## 1) Change summary
Add a Nix-driven Rust integration-test slice that builds the `sce` CLI and validates setup target installation plus required hook installation by invoking the built binary directly in ephemeral repositories, including rerun idempotency and supported hook-path modes, with Turso local state isolated to each test directory.

## 2) Success criteria
- A deterministic Nix test entrypoint runs setup integration tests without ad-hoc local scripting.
- Integration tests are implemented in Rust and execute the compiled `sce` binary (not `cargo run`) for setup and hook scenarios.
- Integration coverage verifies `sce setup --opencode`, `--claude`, and `--both` install outcomes in temporary repositories.
- Integration coverage verifies `sce setup --hooks` for default `.git/hooks` and custom `core.hooksPath` modes.
- Integration coverage verifies rerun idempotency semantics (`skipped` outcomes where applicable) for both target-asset and hook installation flows.
- Assertions prefer structured signals: filesystem/git state as canonical truth for setup outcomes, and JSON output only where command contracts already support it.
- Test runtime ensures Turso local instance/state used by invoked CLI paths is created under the per-test temporary directory (no shared global state).
- The new integration slice is wired into repository verification flow and remains discoverable in context docs.

## 3) Constraints and non-goals
- In scope: integration-test harness and fixtures for setup/hook scenarios, Nix wiring to execute the suite, and context discoverability updates for the new test entrypoint.
- In scope: filesystem and git-backed temporary repository scenarios that exercise real setup command paths.
- In scope: Rust integration tests that build once and invoke the resulting `sce` binary path for all scenario assertions.
- In scope: JSON output assertions only for existing JSON-capable commands used as setup-adjacent checks (for example `doctor --format json`), without changing setup output contracts.
- In scope: deterministic per-test Turso local-state placement inside test temp directories via explicit environment/runtime setup.
- Out of scope: changes to setup runtime semantics, hook template content, or command UX beyond what tests require.
- Out of scope: expanding coverage to non-setup command domains (`doctor`, `sync`, `mcp`) except where setup verification depends on them.
- Non-goal: introducing network-dependent or flaky integration behavior.

## 4) Task stack (T01..T06)
- [x] T01: Define Nix setup integration-test contract and scenario matrix (status:done)
  - Task ID: T01
  - Goal: Specify canonical integration scenarios, expected outcomes, and test boundaries for target install + hooks install coverage using Rust tests that execute the compiled binary.
  - Boundaries (in/out of scope):
    - In: scenario inventory for `--opencode|--claude|--both`, hooks default/custom path modes, rerun idempotency expectations, and assertion-source policy (filesystem/git truth vs JSON output where available).
    - Out: implementation of tests or Nix wiring.
  - Done when:
    - A focused context contract doc records scenario matrix, expected result signals, and deterministic fixture assumptions.
    - Scenario IDs map 1:1 to planned integration tests for implementation continuity.
  - Verification notes (commands or checks):
    - Contract parity review against existing setup behavior docs: `context/sce/setup-githooks-cli-ux.md` and `context/sce/setup-githooks-install-flow.md`.

- [ ] T02: Add integration-test harness for ephemeral git repositories (status:todo)
  - Task ID: T02
  - Goal: Implement reusable Rust integration-test support that provisions isolated repos, compiles `sce`, runs the built binary for setup invocations, and captures deterministic assertions.
  - Boundaries (in/out of scope):
    - In: temp repo lifecycle helpers, binary-path resolution helpers, invocation wrappers around compiled `sce`, Turso-local-state directory setup under each test temp root, and common assertion utilities for install outcomes and filesystem state.
    - Out: test scenario coverage details and Nix entrypoint wiring.
  - Done when:
    - Integration tests can create/teardown isolated repositories and execute the compiled `sce` binary with deterministic stderr/stdout capture.
    - Harness guarantees Turso local state for each test run is rooted under that test's temporary directory.
    - Harness supports both default and custom hooks-path repository preparation.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml --test setup_integration -- --nocapture`.
    - Inspect fixture assertions/logging to confirm Turso paths resolve under test temp roots.

- [ ] T03: Implement OpenCode/Claude/Both setup integration scenarios (status:todo)
  - Task ID: T03
  - Goal: Add Rust integration tests that validate target asset installation for `--opencode`, `--claude`, and `--both`, including rerun idempotency outcomes via compiled-binary invocations.
  - Boundaries (in/out of scope):
    - In: per-target install assertions (expected directories/files), deterministic status lines, and second-run idempotency checks from compiled binary execution.
    - Out: hook installation assertions (covered in T04).
  - Done when:
    - Integration tests cover all three target-selection modes with stable assertions.
    - Rerun checks confirm deterministic no-op-or-skipped style outcomes per current setup contract.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml --test setup_integration setup_targets -- --nocapture`.

- [ ] T04: Implement hook setup integration scenarios for default and custom hooks paths (status:todo)
  - Task ID: T04
  - Goal: Add Rust integration tests for `sce setup --hooks` across default `.git/hooks` and per-repo `core.hooksPath`, including rerun idempotency and executable-state checks via compiled-binary execution.
  - Boundaries (in/out of scope):
    - In: required-hook presence checks (`pre-commit`, `commit-msg`, `post-commit`), status-line assertions, executable-bit assertions, and rerun verification.
    - Out: doctor command behavioral testing beyond optional post-setup JSON sanity checks needed by setup outcomes.
  - Done when:
    - Both hook-path modes are covered by integration tests with deterministic assertions.
    - Rerun behavior confirms stable `installed/updated/skipped` outcome semantics per mode.
    - Optional post-setup `doctor --format json` checks remain deterministic where included.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml --test setup_integration setup_hooks -- --nocapture`.

- [ ] T05: Wire Nix entrypoint and check integration for setup test suite (status:todo)
  - Task ID: T05
  - Goal: Expose and integrate a Nix-runner path for the Rust setup integration suite so contributors and CI-style flows can deterministically build `sce` and execute binary-driven integration tests.
  - Boundaries (in/out of scope):
    - In: flake app/check wiring, invocation command contract, and verification-flow documentation updates for compiled-binary integration execution.
    - Out: unrelated Nix refactors or broader CI workflow additions unless directly required by entrypoint wiring.
  - Done when:
    - A documented Nix command runs the new setup integration suite from repo root.
    - `nix flake check` includes or validates the new setup integration slice according to repo check conventions.
  - Verification notes (commands or checks):
    - `nix run .#<setup-integration-entrypoint>`.
    - `nix flake check`.

- [ ] T06: Validation and cleanup (status:todo)
  - Task ID: T06
  - Goal: Run final verification, clean temporary artifacts, and sync context to current-state behavior for the new Nix integration-test contract.
  - Boundaries (in/out of scope):
    - In: final command validation, artifact cleanup, and context sync confirmation.
    - Out: net-new feature expansion beyond approved tasks.
  - Done when:
    - Verification evidence confirms all success criteria.
    - Temporary test artifacts are removed or explicitly documented.
    - Verification confirms no Turso local-state artifacts leak outside test temp directories.
    - Context discoverability reflects the new test entrypoint and no setup-test drift remains.
  - Verification notes (commands or checks):
    - `nix run .#pkl-check-generated`.
    - `nix flake check`.
    - `nix run .#<setup-integration-entrypoint>`.

## 5) Open questions
- None.
