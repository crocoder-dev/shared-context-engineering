# Plan: agnix-config-ci-validate-report

## 1) Change summary
Add a CI workflow (modeled after `.github/workflows/pkl-generated-parity.yml`) that runs `agnix validate` with `config/` as the working directory, captures the validation report artifact when validation emits non-info findings, and fails the job when any non-info message appears.

## 2) Success criteria
- A GitHub Actions workflow exists for push/PR to `main` that includes checkout, environment setup, and an `agnix validate` step scoped to `config/`.
- The workflow saves the validation report to a deterministic path when `agnix validate` outputs non-info findings.
- CI fails when validation contains any non-info message (warning/error/fatal class as defined by `agnix validate` output contract).
- If validation is info-only/clean, the workflow passes without false failures.
- Workflow behavior and report path are documented in context for future sessions.

## 3) Constraints and non-goals
- In scope: workflow YAML changes under `.github/workflows/` and context updates required to describe current-state CI validation behavior.
- In scope: deterministic report file handling and explicit fail conditions tied to non-info findings.
- Out of scope: modifying `agnix` implementation, changing repository architecture, or introducing unrelated CI jobs.
- Out of scope: broad refactors of existing parity workflows beyond shared-pattern alignment.
- Non-goal: introducing additional third-party CI services; GitHub Actions remains the execution platform.

## 4) Task stack (T01..T05)
- [x] T01: Define workflow trigger and execution contract (status:done)
  - Task ID: T01
  - Goal: Lock the CI trigger/event scope and exact job contract for `agnix validate` in `config/`.
  - Boundaries (in/out of scope):
    - In: event triggers (`push`/`pull_request` on `main`), permissions, runner, timeout, and command working-directory contract.
    - Out: report parsing implementation details.
  - Done when:
    - Workflow contract is explicitly defined and mirrors existing repo CI conventions where applicable.
    - `agnix validate` invocation is pinned to execute from `config/`.
  - Verification notes (commands or checks):
    - Static YAML review confirms trigger parity with `.github/workflows/pkl-generated-parity.yml` and `working-directory: config` (or equivalent deterministic command form).
  - Evidence:
    - Added `.github/workflows/agnix-config-validate-report.yml` with trigger parity (`push`/`pull_request` on `main`), `permissions: contents: read`, `runs-on: ubuntu-latest`, and `timeout-minutes: 15`.
    - Pinned command execution scope with workflow `defaults.run.working-directory: config` and `nix develop -c agnix validate .`.

- [x] T02: Implement report capture and non-info failure detection (status:done)
  - Task ID: T02
  - Goal: Add deterministic report capture and gate logic that fails on non-info messages.
  - Boundaries (in/out of scope):
    - In: shell/step logic for running validation, capturing stdout/stderr/report output, and message-severity detection.
    - Out: changing `agnix validate` semantics or adding custom validators.
  - Done when:
    - Workflow writes report output to a stable path (for example under `context/tmp/ci-reports/` or another agreed deterministic location).
    - Job exits non-zero when report contains non-info findings.
    - Clean/info-only output exits zero.
  - Verification notes (commands or checks):
    - Workflow logic review demonstrates explicit severity filter and exit-code handling.
    - Path existence and upload preconditions are deterministic.
  - Evidence:
    - Implemented report capture in the validate step using deterministic path `context/tmp/ci-reports/agnix-validate-report.txt` (referenced from `config/` via `../context/tmp/ci-reports/agnix-validate-report.txt`) and explicit parent-directory creation.
    - Added explicit non-info severity detection with regex `\b(warning|error|fatal):` and surfaced `has_non_info` via step outputs.
    - Added explicit gate step that fails the job on non-zero validate exit or `has_non_info == true`.

- [x] T03: Add artifact upload behavior for failure investigation (status:done)
  - Task ID: T03
  - Goal: Preserve validation report as a CI artifact when non-info findings occur.
  - Boundaries (in/out of scope):
    - In: `actions/upload-artifact` (or equivalent) wiring, artifact name conventions, conditional upload semantics.
    - Out: long-term report retention policy changes outside workflow defaults.
  - Done when:
    - Non-info failure runs upload the captured report artifact.
    - Artifact naming and path conventions are stable and discoverable.
  - Verification notes (commands or checks):
    - Static workflow inspection confirms upload step condition aligns with failure/severity detection and references the deterministic report path.
  - Evidence:
    - Added `actions/upload-artifact@v4` step named "Upload agnix validation report artifact" with artifact name `agnix-validate-report`.
    - Conditional upload is wired to severity detection with `if: steps.validate.outputs.has_non_info == 'true'` and uploads `context/tmp/ci-reports/agnix-validate-report.txt`.

- [x] T04: Sync context for new CI validation pattern (status:done)
  - Task ID: T04
  - Goal: Update context files to reflect the new `agnix validate` CI behavior and report policy.
  - Boundaries (in/out of scope):
    - In: current-state updates in relevant `context/` files (`context/patterns.md`, `context/architecture.md`, and/or glossary entries as needed).
    - Out: historical narrative or completed-work logs in core context files.
  - Done when:
    - Context references the workflow path, command contract, fail condition, and report artifact behavior.
  - Verification notes (commands or checks):
    - Context/code consistency spot-check between workflow YAML and updated `context/` statements.
  - Evidence:
    - Updated `context/architecture.md` with the new workflow path and current-state execution/report/failure contract.
    - Updated `context/patterns.md` with trigger parity, working-directory contract, deterministic report-path usage, conditional artifact upload behavior, and non-info fail gating.
    - Updated `context/glossary.md` with terms for `agnix-config-validate-report` and the validation report artifact contract.

- [x] T05: Validation and cleanup (status:done)
  - Task ID: T05
  - Goal: Run final checks, confirm all success criteria, and remove temporary artifacts not meant for commit.
  - Boundaries (in/out of scope):
    - In: workflow lint/validation, repo status review for intended files, and temporary artifact cleanup.
    - Out: net-new feature additions.
  - Done when:
    - All success criteria have explicit evidence.
    - Workflow file and context updates are internally consistent and ready for review.
    - Temporary local report files (if any) are cleaned or intentionally ignored.
  - Verification notes (commands or checks):
    - Run repository CI/workflow validation checks used by this repo.
    - Confirm changed-file set is limited to planned workflow/context artifacts.
  - Evidence:
    - Ran `nix develop -c agnix validate config` (exit 0): validation completed with `Found 0 errors, 0 warnings` and `1 info messages`.
    - Ran `nix flake check` (exit 0): repository flake outputs evaluated successfully on host platform, with expected app metadata and incompatible-system warnings only.
    - Ran `nix develop -c ./config/pkl/check-generated.sh` (exit 0): generated parity check reported `Generated outputs are up to date.`.
    - Reviewed touched files for this task and kept implementation scope to planned workflow/context artifacts (`.github/workflows/agnix-config-validate-report.yml`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/plans/agnix-config-ci-validate-report.md`) while preserving unrelated pre-existing worktree changes.
    - No temporary local report artifacts were created during local verification, so no cleanup action was required.

## 5) Open questions
- None.
