# Plan: sce-cli-setup-integration-test-improvements

## 1) Change summary
Expand the Rust binary-driven setup integration suite to cover `--repo` path canonicalization behavior, setup failure contracts, true interactive setup behavior through a PTY harness, hook update/backup edge cases, deterministic permission failures, and cross-platform validation improvements, while explicitly excluding CI trigger changes.

## 2) Success criteria
- Integration coverage validates `sce setup --hooks --repo <path>` for both relative and absolute repository paths, including canonical path handling and reported repository/hooks directory output.
- Binary-level integration tests verify exit code and stderr contracts for `sce setup --hooks --repo /missing`, `sce setup --repo <path>` (without `--hooks`), and `sce setup --non-interactive` (without target).
- Interactive setup integration tests run through a PTY harness and cover OpenCode selection, cancellation flow, and non-TTY failure messaging.
- Integration tests verify hook update behavior by mutating an installed hook, rerunning setup, asserting `updated` status, and asserting backup creation.
- Integration tests verify backup suffix collision behavior by pre-creating backup targets and asserting deterministic next-suffix selection.
- Integration tests verify deterministic writability/permission failures for repo-root and hooks-directory write probes, including unix-only guarded read-only directory scenarios.
- Cross-platform validation runs on more than one OS and keeps assertions platform-appropriate for executable-bit and path handling differences.
- CI workflow trigger definitions remain unchanged.

## 3) Constraints and non-goals
- In scope: `cli/tests/setup_integration.rs` scenario/harness expansion, setup-integration contract updates under `context/sce/`, and CI matrix/job updates needed for multi-OS validation without changing workflow trigger events.
- In scope: preserving existing user-facing setup/error wording and asserting against current contract text (without rewording runtime messages as part of this change).
- In scope: unix-only guards for permission scenarios that rely on POSIX read-only semantics.
- Out of scope: setup runtime behavior changes unrelated to making existing contracts testable.
- Out of scope: changing workflow trigger conditions (`on:` push/pull_request filters, branch filters).
- Out of scope: unrelated command domains outside setup/hook integration coverage.

## 4) Task stack (T01..T09)
- [x] T01: Update setup integration-test contract for new scenario matrix (status:done)
  - Task ID: T01
  - Goal: Refresh `context/sce/setup-nix-integration-test-contract.md` with canonical scenario IDs and assertion policy for repo-path, failure-contract, PTY-interactive, hook-update, backup-collision, permission, and cross-platform coverage.
  - Boundaries (in/out of scope):
    - In: scenario IDs, expected canonical signals, OS-guard guidance, and explicit note that CI triggers are unchanged.
    - Out: test code implementation and CI YAML edits.
  - Done when:
    - Contract doc enumerates all new scenario classes with deterministic assertion anchors.
    - Contract parity references include current setup UX/security behavior docs.
  - Verification notes (commands or checks):
    - Manual parity review against `context/sce/setup-githooks-cli-ux.md` and `context/sce/cli-security-hardening-contract.md`.

- [ ] T02: Add `--repo` relative/absolute path integration scenarios (status:todo)
  - Task ID: T02
  - Goal: Add binary-driven integration tests that execute `sce setup --hooks --repo <path>` using relative and absolute paths, then assert canonical repo/hooks output and correct install location behavior.
  - Boundaries (in/out of scope):
    - In: test fixtures for relative/absolute path invocation, canonicalized output assertions, and filesystem truth checks.
    - Out: runtime canonicalization logic changes.
  - Done when:
    - Both relative and absolute `--repo` scenarios pass with deterministic repository/hooks path assertions.
    - Assertions prove hooks are installed in git-resolved target path.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml --test setup_integration setup_hooks_repo_paths -- --nocapture`.

- [ ] T03: Add binary-level setup failure-contract integration tests (status:todo)
  - Task ID: T03
  - Goal: Add integration tests for invalid setup invocations that assert process exit class and stderr contract text for the three requested failure modes.
  - Boundaries (in/out of scope):
    - In: tests for `/missing` repo path, `--repo` without `--hooks`, and `--non-interactive` without target.
    - Out: changing error strings or failure-class mapping behavior.
  - Done when:
    - Each failure scenario asserts expected non-zero exit code and deterministic stderr content aligned with current user-facing text.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml --test setup_integration setup_failure_contracts -- --nocapture`.

- [ ] T04: Add true interactive setup PTY integration coverage (status:todo)
  - Task ID: T04
  - Goal: Introduce PTY-backed integration tests that validate real prompt behavior for `sce setup`, including OpenCode selection and cancel flow, and validate non-TTY failure messaging for interactive mode without a TTY.
  - Boundaries (in/out of scope):
    - In: PTY harness utilities (test-only), deterministic prompt interaction assertions, and explicit non-TTY invocation assertion.
    - Out: changing interactive UX semantics or prompt copy.
  - Done when:
    - PTY flow can select OpenCode and complete setup with expected outcomes.
    - PTY flow can cancel and assert non-destructive cancellation result.
    - Non-TTY scenario asserts actionable failure guidance.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml --test setup_integration setup_interactive_pty -- --nocapture`.

- [ ] T05: Add hook-update integration scenario with backup assertion (status:todo)
  - Task ID: T05
  - Goal: Extend hook scenarios to mutate one previously installed required hook, rerun setup, and assert `updated` outcome plus backup file creation.
  - Boundaries (in/out of scope):
    - In: one-hook mutation fixture, rerun output assertions, and backup artifact presence/content sanity checks.
    - Out: altering update/backup runtime implementation.
  - Done when:
    - Test deterministically demonstrates `updated` status for mutated hook.
    - Backup artifact path exists and is associated with replaced pre-rerun hook content.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml --test setup_integration setup_hooks_update_path -- --nocapture`.

- [ ] T06: Add backup suffix collision integration scenario (status:todo)
  - Task ID: T06
  - Goal: Add integration coverage for pre-existing backup path collisions (for example `.opencode.backup`, `.opencode.backup.1`) and assert next available suffix selection.
  - Boundaries (in/out of scope):
    - In: deterministic pre-created backup fixtures and rerun assertions for chosen backup target.
    - Out: changing backup naming algorithm.
  - Done when:
    - Collision scenario confirms setup selects the correct next backup suffix.
    - Assertions remain deterministic regardless of temp-root path randomness.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml --test setup_integration setup_backup_suffix_collision -- --nocapture`.

- [ ] T07: Add writability/permission failure integration scenarios (status:todo)
  - Task ID: T07
  - Goal: Add deterministic integration tests for setup failures when repo root or hooks directory is not writable, including unix-guarded read-only directory cases.
  - Boundaries (in/out of scope):
    - In: permission fixture prep, expected failure-class/stderr assertions, and cfg-guarded unix-only read-only checks.
    - Out: Windows ACL-specific simulation beyond portable deterministic test scope.
  - Done when:
    - Non-writable repo-root and hooks-dir failure paths are asserted with deterministic diagnostics.
    - Unix-only read-only tests are guarded and stable.
  - Verification notes (commands or checks):
    - `cargo test --manifest-path cli/Cargo.toml --test setup_integration setup_permission_failures -- --nocapture`.

- [ ] T08: Expand multi-OS validation and platform-aware assertions (status:todo)
  - Task ID: T08
  - Goal: Ensure setup integration validation runs on more than one OS by updating existing CI job matrix (without trigger changes) and tightening tests for platform-appropriate executable-bit/path assertions.
  - Boundaries (in/out of scope):
    - In: CI job matrix adjustments for `cli-integration-tests` workflow and OS-conditional assertion handling in setup integration tests.
    - Out: modifications to workflow trigger conditions or unrelated CI pipelines.
  - Done when:
    - Existing integration workflow validates setup suite on at least two OS targets.
    - Test assertions are explicit about platform differences and remain deterministic per OS.
  - Verification notes (commands or checks):
    - Local: `nix run .#cli-integration-tests`.
    - CI evidence: successful multi-OS `cli-integration-tests` workflow runs.

- [ ] T09: Validation and cleanup (status:todo)
  - Task ID: T09
  - Goal: Run final verification set, ensure no flaky/temporary scaffolding remains, and sync context to final current-state contracts.
  - Boundaries (in/out of scope):
    - In: full test/flake validation, cleanup, and context sync updates for durable behavior changes.
    - Out: new feature expansion beyond approved setup integration improvements.
  - Done when:
    - Setup integration suite passes with the added scenarios.
    - Required repo verification passes.
    - Context files reflect final behavior and verification entrypoints.
  - Verification notes (commands or checks):
    - `nix run .#cli-integration-tests`.
    - `nix run .#pkl-check-generated`.
    - `nix flake check`.

## 5) Open questions
- None.
