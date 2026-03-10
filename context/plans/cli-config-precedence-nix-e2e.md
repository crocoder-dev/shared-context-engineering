# Plan: CLI Config Precedence Nix End-to-End Tests

## Change Summary

Add compiled-binary end-to-end coverage for CLI config precedence so the existing Nix-driven CLI integration path proves both implemented resolution chains: `flags > env > config file > defaults` for runtime config keys and `env > config file > baked default` for `workos_client_id`. Current code truth already documents and unit-tests these contracts, but the Nix integration entrypoint still centers on setup-only binary scenarios. This plan extends the binary integration surface without introducing a non-binary harness.

## Success Criteria

- [ ] Nix-driven CLI integration coverage executes compiled-binary end-to-end scenarios for `sce config show` and/or `sce config validate`
- [ ] Binary end-to-end tests prove `--log-level` / `--timeout-ms` flags override env, config-file values, and defaults
- [ ] Binary end-to-end tests prove `SCE_LOG_LEVEL` / `SCE_TIMEOUT_MS` override config-file values and defaults when flags are absent
- [ ] Binary end-to-end tests prove discovered or explicit config-file values are used when higher-precedence overrides are absent
- [ ] Binary end-to-end tests prove default fallback remains `log_level=info` and `timeout_ms=30000` when no higher-precedence sources are present
- [ ] Binary end-to-end tests prove `WORKOS_CLIENT_ID` overrides config-file and baked default values, config-file overrides baked default when env is absent, and baked default is used when env/config are absent
- [ ] The new config-precedence binary end-to-end coverage is available through a canonical Nix entrypoint without being added to the default `nix flake check` path
- [ ] Current-state context reflects the expanded CLI integration scope and config-precedence end-to-end contract without leaving setup-only wording where it is no longer true

## Constraints and Non-Goals

**In Scope:**
- Add CLI integration tests that invoke the compiled `sce` binary in isolated temp repositories/state roots
- Reuse or extract compiled-binary integration-test helpers only as needed to keep the new coverage deterministic
- Assert precedence using stable stdout/JSON payloads, exit codes, and isolated config/env setup
- Extend the existing Nix integration surface so the new binary tests run through a canonical repo entrypoint while remaining out of the default `nix flake check` path
- Sync focused and root context where the integration-test scope changes are now important current-state behavior

**Out of Scope:**
- Changing the underlying config precedence implementation unless end-to-end gaps reveal a real defect
- Adding non-binary test harnesses, shell-script fixtures, or a second independent Nix test app when the existing entrypoint can be extended
- Broad redesign of config command UX beyond deterministic assertions needed for the new end-to-end coverage
- Live auth or networked WorkOS flows

**Non-Goals:**
- Replacing existing unit tests for config resolution
- Reworking setup integration scenarios unrelated to config precedence
- Changing CI trigger filters or branch policy

## Assumptions

- The existing canonical repository entrypoint `nix run .#cli-integration-tests` should continue to cover current setup integration behavior; config-precedence E2E coverage may use that entrypoint or a sibling canonical Nix app as long as it stays out of default `nix flake check`
- "End-to-end binary test only" means Rust integration tests may drive the compiled `sce` binary directly, but should not rely on internal function-level assertions as the primary verification signal
- JSON output from `sce config show` / `sce config validate` is the most stable assertion surface for precedence-source checks

## Task Stack

- [x] T01: Introduce shared compiled-binary config integration harness support (status:done)
  - Task ID: T01
  - Goal: Add or extract the minimal test-support surface needed for config-precedence end-to-end scenarios to run against the compiled `sce` binary with isolated repo/state roots.
  - Boundaries (in/out of scope):
    - IN: Reuse or refactor integration-test temp-dir, repo, env, and command helpers so config E2E scenarios can be added without duplicating fragile setup logic
    - IN: Preserve existing setup integration behavior while making compiled-binary support usable by an additional config-precedence test target or shared support module
    - OUT: Adding actual precedence assertions, changing runtime config logic, updating context files
  - Done when:
    - There is one deterministic compiled-binary integration support path that can provision isolated repo/state/config environments for config scenarios
    - Existing setup integration tests still fit the same binary-driven model after the harness change
    - The support surface is narrow enough that later config-precedence tasks can land as behavior-only commits
  - Verification notes (commands or checks):
    - Run `cargo test --manifest-path cli/Cargo.toml --test setup_integration`
    - If a new shared test-support module or second integration target is introduced, verify it still resolves the compiled `sce` binary path rather than calling internal library APIs

- [x] T02: Add end-to-end tests for `flags > env > config file > defaults` runtime precedence (status:done)
  - Task ID: T02
  - Goal: Add compiled-binary integration scenarios that prove the implemented precedence chain for `log_level` and `timeout_ms` through `sce config` command output.
  - Boundaries (in/out of scope):
    - IN: Cover flag-over-env-over-config-over-default behavior for `log_level` and `timeout_ms`
    - IN: Use isolated config files and env setup for deterministic assertions against stdout/JSON output and resolved source metadata
    - IN: Cover both a fully layered override case and no-override/default fallback case
    - OUT: `workos_client_id` auth-key precedence, Nix/flake entrypoint changes, context sync
  - Done when:
    - Binary E2E tests show flags win over env/config/defaults for `log_level` and `timeout_ms`
    - Binary E2E tests show env wins over config/defaults when flags are absent
    - Binary E2E tests show config values are used when flags/env are absent
    - Binary E2E tests show defaults remain `info` and `30000` when no higher-precedence source is present
  - Verification notes (commands or checks):
    - Run `cargo test --manifest-path cli/Cargo.toml --test config_precedence_integration`
    - Verify assertions inspect compiled-binary command output rather than internal structs

- [x] T03: Add end-to-end tests for `env > config file > baked default` auth precedence (status:done)
  - Task ID: T03
  - Goal: Add compiled-binary integration scenarios that prove `workos_client_id` resolves through the implemented auth precedence chain.
  - Boundaries (in/out of scope):
    - IN: Cover env override, config-file fallback, and baked-default fallback for `WORKOS_CLIENT_ID` / `workos_client_id`
    - IN: Assert resolved value/source metadata via stable `sce config` output
    - OUT: Expanding auth behavior beyond precedence inspection, changing login/network flows, root Nix wiring, context sync
  - Done when:
    - Binary E2E tests prove `WORKOS_CLIENT_ID` wins over config-file and baked-default values
    - Binary E2E tests prove config-file value wins over baked default when env is absent
    - Binary E2E tests prove baked default is reported when env/config inputs are absent
  - Verification notes (commands or checks):
    - Run `cargo test --manifest-path cli/Cargo.toml --test config_precedence_integration`
    - Verify assertions check stable source markers such as `env`, `config_file`, and `default`

- [ ] T04: Add opt-in canonical Nix entrypoint for config-precedence binary tests (status:todo)
  - Task ID: T04
  - Goal: Update flake-managed CLI integration execution so config-precedence binary tests are runnable through a canonical Nix entrypoint without being pulled into the default `nix flake check` path.
  - Boundaries (in/out of scope):
    - IN: Update root and nested flake wiring plus app/help text as needed so config-precedence binary tests have a canonical Nix run path
    - IN: Preserve existing `nix flake check` behavior unless explicitly required for current setup integration coverage already in place
    - IN: Keep naming and help text clear about which entrypoint runs setup integration vs config-precedence E2E coverage
    - OUT: New CI trigger policies, unrelated flake refactors, context-file edits
  - Done when:
    - There is one documented canonical Nix command for running config-precedence binary E2E coverage
    - The config-precedence binary tests do not run as part of default `nix flake check`
    - Help text and command descriptions accurately distinguish the available integration entrypoints
  - Verification notes (commands or checks):
    - Run the canonical Nix command for config-precedence binary tests
    - Run `nix flake check`
    - Verify `nix flake check` does not pick up the config-precedence binary test slice by default
    - Verify flake help/output text accurately describes the available integration commands

- [ ] T05: Sync context contracts for CLI config-precedence binary integration coverage (status:todo)
  - Task ID: T05
  - Goal: Update current-state context so future sessions understand which Nix entrypoint covers setup integration and which opt-in path covers config-precedence binary end-to-end coverage.
  - Boundaries (in/out of scope):
    - IN: Update focused context covering config precedence and CLI integration-test contracts
    - IN: Update root context files only where the integration-entrypoint scope change is an important current-state contract
    - OUT: Historical rollout notes or completed-work narration
  - Done when:
    - Focused context documents the new config-precedence binary scenarios, stable assertion policy, and canonical opt-in Nix command
    - Root context accurately distinguishes setup integration coverage from config-precedence E2E coverage and does not imply the latter runs in default `nix flake check`
    - Context statements match code truth and verification entrypoints exactly
  - Verification notes (commands or checks):
    - Verify `context/cli/config-precedence-contract.md` reflects the intended E2E assertion surface if needed
    - Verify `context/overview.md`, `context/glossary.md`, `context/architecture.md`, and `context/patterns.md` stay aligned with the final split between default and opt-in integration entrypoints where touched

- [ ] T06: Validation and cleanup (status:todo)
  - Task ID: T06
  - Goal: Validate binary integration coverage, flake wiring, and context alignment for the CLI config-precedence Nix end-to-end change.
  - Boundaries (in/out of scope):
    - IN: Run focused config-precedence and existing setup integration tests
    - IN: Run canonical Nix integration and repo lightweight validation baseline
    - IN: Confirm context sync accuracy after implementation
    - OUT: Additional product changes beyond the planned coverage and context updates
  - Done when:
    - The compiled-binary config-precedence integration tests pass locally through both focused cargo execution and the canonical opt-in Nix entrypoint
    - Existing setup integration coverage remains green
    - Root and focused context accurately reflect the final integration-test scope, entrypoint split, and precedence contract
  - Verification notes (commands or checks):
    - Run `cargo test --manifest-path cli/Cargo.toml --test setup_integration`
    - Run `cargo test --manifest-path cli/Cargo.toml --test config_precedence_integration`
    - Run the canonical Nix command for config-precedence binary tests
    - Run `nix run .#pkl-check-generated`
    - Run `nix flake check`

## Open Questions

None.
