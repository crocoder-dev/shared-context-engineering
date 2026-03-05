# SCE setup Nix integration-test contract

## Scope

Task `sce-nix-setup-hooks-integration-tests` defines the canonical setup integration-test matrix and harness contracts for the suite that will run through a deterministic Nix entrypoint.

Current implementation state:

- `T01` is implemented as this scenario/assertion contract.
- `T02` is implemented in `cli/tests/setup_integration.rs` as reusable integration harness primitives (ephemeral repo setup, compiled-binary invocation wrappers, deterministic stdout/stderr capture, default/custom hooks-path prep helpers, and per-test Turso state-home isolation).
- `T03` is implemented in `cli/tests/setup_integration.rs` as target-install integration scenarios covering `sce setup --opencode`, `sce setup --claude`, and `sce setup --both`, each with first-run install assertions and deterministic rerun assertions.
- `T04` is implemented in `cli/tests/setup_integration.rs` as hook-install integration scenarios for default `.git/hooks` and custom per-repo `core.hooksPath`, including rerun idempotency and executable-state assertions.
- `T05` is implemented via root and nested flake wiring:
  - root app entrypoint: `nix run .#cli-integration-tests`
  - nested check derivation: `checks.<system>.cli-setup-integration` in `cli/flake.nix`
  - root check pass-through: `checks.<system>.cli-setup-integration` in `flake.nix`

## Required execution model

- Integration tests are written in Rust and run against the compiled `sce` binary path (not `cargo run`).
- Scenarios execute in isolated ephemeral repositories with deterministic fixture setup.
- Assertions use filesystem and git state as source of truth for setup outcomes.
- JSON assertions are allowed only where an existing command contract already supports JSON output (for example `doctor --format json`).
- Test runtime must isolate Turso local state under each test temp root (no shared user-global state).

## Scenario matrix (canonical IDs)

### Target install scenarios

- `SETUP-TARGET-OPENCODE-RUN1`: `sce setup --opencode` installs `.opencode/` assets in a fresh repo.
- `SETUP-TARGET-OPENCODE-RERUN`: rerun `sce setup --opencode` and assert idempotent/skipped behavior per current setup contract.
- `SETUP-TARGET-CLAUDE-RUN1`: `sce setup --claude` installs `.claude/` assets in a fresh repo.
- `SETUP-TARGET-CLAUDE-RERUN`: rerun `sce setup --claude` and assert idempotent/skipped behavior.
- `SETUP-TARGET-BOTH-RUN1`: `sce setup --both` installs both target trees in a fresh repo.
- `SETUP-TARGET-BOTH-RERUN`: rerun `sce setup --both` and assert idempotent/skipped behavior.

### Hook install scenarios

- `SETUP-HOOKS-DEFAULT-RUN1`: `sce setup --hooks` installs required hooks in default `.git/hooks` mode.
- `SETUP-HOOKS-DEFAULT-RERUN`: rerun `sce setup --hooks` and assert deterministic per-hook `installed|updated|skipped` semantics resolve to stable no-op/skipped outcomes.
- `SETUP-HOOKS-CUSTOM-RUN1`: configure per-repo `core.hooksPath`, run `sce setup --hooks`, and assert required hooks install in the resolved custom path.
- `SETUP-HOOKS-CUSTOM-RERUN`: rerun custom-path hooks setup and assert deterministic idempotency with executable-state preservation.

### Optional setup-adjacent sanity scenario

- `SETUP-HOOKS-DOCTOR-JSON-SANITY` (optional): after hook install scenarios, run `sce doctor --format json` and assert deterministic readiness fields only where they are already contract-defined.

## Assertion signal policy

- Canonical signals:
  - repository filesystem state (installed paths, required files, executable bit state)
  - git-resolved hooks directory state for default and custom `core.hooksPath`
- Secondary signals:
  - deterministic CLI status lines for setup and hook outcomes
  - JSON payload fields from JSON-capable commands only
- Non-canonical signals:
  - free-form stderr wording that is not contract-stable
  - environment-global side effects outside test temp roots

## Deterministic fixture assumptions

- Each scenario owns a unique temp directory with its own repo root.
- Repo initialization and hook-path configuration are explicit per scenario.
- CLI invocation environment pins state-home style variables so Turso local DB paths resolve under the scenario temp root.
- Scenario assertions never depend on execution order across unrelated scenarios.

## Implemented harness surface (T02)

- `SetupIntegrationHarness` creates a unique test temp root with explicit subpaths for repo, state home, and HOME isolation.
- `run_sce(...)` invokes the compiled `sce` binary path directly (prefers `CARGO_BIN_EXE_sce`, with deterministic target-profile fallback path resolution) and captures stdout/stderr/status for assertions.
- `run_git(...)` uses the same isolated environment (`XDG_STATE_HOME`, temp-scoped `HOME`, `GIT_CONFIG_GLOBAL` null device, `GIT_CONFIG_NOSYSTEM=1`) to avoid global-machine config drift.
- Harness helpers include repository bootstrap (`git init -q`) and per-repo custom hook-path setup (`git config core.hooksPath <relative-path>`).
- Harness validation includes proof that runtime local DB bootstrap for `sce hooks post-commit` lands at `${XDG_STATE_HOME}/sce/agent-trace/local.db` under each test temp root.

## Implemented target scenario coverage (T03)

- `setup_targets_opencode_install_and_rerun_are_deterministic` validates `sce setup --opencode` output markers, `.opencode/command/next-task.md` presence, `.claude` absence, and deterministic rerun backup-and-replace messaging.
- `setup_targets_claude_install_and_rerun_are_deterministic` validates `sce setup --claude` output markers, `.claude/commands/next-task.md` presence, `.opencode` absence, and deterministic rerun backup-and-replace messaging.
- `setup_targets_both_install_and_rerun_are_deterministic` validates `sce setup --both` output markers plus both target trees and deterministic rerun backup-and-replace messaging.

## Planned implementation mapping (1:1)

- `SETUP-TARGET-*` scenarios map to planned implementation task `T03`.
- `SETUP-HOOKS-*` scenarios map to planned implementation task `T04`.
- Harness-level temp-repo, compiled-binary invocation, and Turso-state helpers map to planned implementation task `T02`.
- Nix entrypoint and flake check wiring map to implemented task `T05`.

## Parity anchors

This contract aligns with the current setup behavior in:

- `context/sce/setup-githooks-cli-ux.md`
- `context/sce/setup-githooks-install-flow.md`
