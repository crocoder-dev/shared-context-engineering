# Plan: Doctor OpenCode structure checks + status tags

## Change summary
Extend `sce doctor` so that when a repo-local `.opencode/` directory exists, the doctor validates that required subdirectories (`agent/`, `command/`, `skills/`) are present and reports missing items as errors with manual-only remediation. Update doctor’s **text** output to prefix every line with a standardized status tag: `[PASS]`, `[FAIL]`, `[MISS]`, or `[WARN]`. JSON output remains unchanged.

## Success criteria
- When `.opencode/` exists, missing `agent/`, `command/`, or `skills/` is reported as a **Problem** with `severity=error`, `fixability=manual_only`, and clear manual remediation guidance.
- When `.opencode/` does **not** exist, the new structure checks do **not** emit any problems.
- All text output lines from `sce doctor` are prefixed with exactly one of `[PASS]`, `[FAIL]`, `[MISS]`, `[WARN]` using a deterministic, documented mapping.
- JSON output shape and field values are unchanged.
- Doctor tests are updated/added to cover the new OpenCode structure checks and the status-tagged text output.
- Context contract for doctor reflects the new OpenCode structure checks and status-tagged text output.

## Constraints and non-goals
- No automatic fixes for OpenCode structure issues; all such issues are `manual_only`.
- Do not introduce new dependencies.
- Do not change the JSON output schema or field names.
- Keep scope limited to `sce doctor` behavior; no other commands should change.

## Task stack
- [x] T01: Add OpenCode structure checks when `.opencode/` exists (status:done)
  - Task ID: T01
  - Goal: Detect missing `.opencode/agent`, `.opencode/command`, and `.opencode/skills` when `.opencode/` exists, and surface each missing directory as a manual-only error problem.
  - Boundaries (in/out of scope):
    - In scope: doctor repo-asset checks in `cli/src/services/doctor.rs`, problem categorization/remediation text.
    - Out of scope: auto-fix behavior, setup changes, JSON schema changes.
  - Done when: Each missing required directory emits a `RepoAssets` (or appropriate) problem with `severity=error`, `fixability=manual_only`, and deterministic remediation text; no issue emitted when `.opencode/` is absent.
  - Verification notes: Add/update doctor unit tests covering `.opencode/` present vs absent cases.

- [x] T02: Implement status-tagged text rendering for doctor output (status:done)
  - Task ID: T02
  - Goal: Prefix every text output line with one of `[PASS]`, `[FAIL]`, `[MISS]`, `[WARN]` using a deterministic mapping aligned to doctor readiness and per-line state.
  - Boundaries (in/out of scope):
    - In scope: text-only rendering in `format_report`/`format_execution` and any shared formatting helpers needed.
    - Out of scope: JSON output changes, non-doctor commands.
  - Done when: All lines in text output carry a tag; tag mapping is consistent (e.g., PASS for healthy/informational lines, FAIL for error states, WARN for warnings, MISS for “not detected”/missing/none states), and no untagged lines remain.
  - Verification notes: Update tests asserting full-line tagging for representative outputs including ready/not-ready and fix-mode variants.

- [x] T03: Update doctor tests for new checks and tagged output (status:done)
  - Task ID: T03
  - Goal: Ensure test coverage validates OpenCode structure checks and status-tagged text formatting without altering JSON output expectations.
  - Boundaries (in/out of scope):
    - In scope: doctor unit tests in `cli/src/services/doctor.rs` (or existing doctor test module).
    - Out of scope: new integration tests or changes outside doctor.
  - Done when: Tests cover missing/ok OpenCode structure, tagged text output lines, and unchanged JSON output; all tests pass.
  - Verification notes: `nix develop -c sh -c 'cd cli && cargo test doctor'` (or the narrowest doctor-related test target used in the repo).

- [x] T04: Sync doctor contract context to reflect new behavior (status:done)
  - Task ID: T04
  - Goal: Update `context/sce/agent-trace-hook-doctor.md` to include the new OpenCode structure checks and the status-tagged text output requirement.
  - Boundaries (in/out of scope):
    - In scope: doctor contract documentation updates only.
    - Out of scope: changes to other context files unless required by the contract.
  - Done when: The doctor contract explicitly documents the `.opencode` required subdirectories and the `[PASS]/[FAIL]/[MISS]/[WARN]` text output convention.
  - Verification notes: Manual review of context file for accuracy vs implementation.

- [x] T05: Validation and cleanup (status:done)
  - Task ID: T05
  - Goal: Run repo checks appropriate to the change and ensure no leftover scaffolding or inconsistencies remain.
  - Boundaries (in/out of scope):
    - In scope: verification commands and cleanup only.
    - Out of scope: functional changes.
  - Done when: All verification commands pass and the plan is updated with results.
  - Verification notes: `nix flake check` (plus any narrower doctor tests if already used in T03).

## Open questions
None.

## Task log

### T01
- Status: done
- Completed: 2026-03-30
- Files changed: cli/src/services/doctor.rs
- Evidence: `nix develop -c sh -c 'cd cli && cargo test doctor'`
- Notes: Added required `.opencode` subdirectory checks gated on `.opencode/` presence; missing directories now emit manual-only repo-assets errors with updated tests.

### T02
- Status: done
- Completed: 2026-03-30
- Files changed: cli/src/services/doctor.rs
- Evidence: `nix develop -c sh -c 'cd cli && cargo test doctor'`
- Notes: Added status-tagged text output for all doctor lines with tag-mapping helpers and new text-output tag coverage tests.

### T03
- Status: done
- Completed: 2026-03-30
- Files changed: cli/src/services/doctor.rs
- Evidence: `nix develop -c sh -c 'cd cli && cargo test doctor'`
- Notes: Added warning/missing tag coverage in doctor text output tests.

### T04
- Status: done
- Completed: 2026-03-30
- Files changed: context/sce/agent-trace-hook-doctor.md
- Evidence: Manual review
- Notes: Doctor contract already documented OpenCode required directory checks and tagged text output; no content changes required.

### T05
- Status: done
- Completed: 2026-03-30
- Files changed: cli/src/services/doctor.rs, context/plans/doctor-opencode-structure-status-tags.md
- Evidence: `nix flake check`
- Notes: Fixed clippy match-same-arms warning and confirmed full flake checks pass.

## Validation Report

### Commands run
- `nix flake check` -> exit 0 (all checks passed)

### Failed checks and follow-ups
- Initial `nix flake check` failed `cli-clippy` due to `clippy::match_same_arms` in `tag_for_fix_result`; fixed by merging match arms and reran `nix flake check` successfully.

### Success-criteria verification
- [x] Missing `.opencode/agent`, `.opencode/command`, `.opencode/skills` reported as manual-only errors when `.opencode/` exists -> `services::doctor::tests::doctor_reports_opencode_structure_missing_directories`.
- [x] No `.opencode/` structure issues reported when `.opencode/` is absent -> `services::doctor::tests::doctor_skips_opencode_structure_checks_without_root`.
- [x] Text output lines are prefixed with `[PASS]/[FAIL]/[MISS]/[WARN]` -> `services::doctor::tests::doctor_text_output_tags_all_lines_for_*` and `doctor_text_output_includes_warn_and_miss_tags`.
- [x] JSON output shape unchanged -> `services::doctor::tests::render_json_includes_stable_fields_without_filesystem`.
- [x] Doctor contract documents OpenCode structure checks and tagged output -> `context/sce/agent-trace-hook-doctor.md`.

### Residual risks
- None identified.
