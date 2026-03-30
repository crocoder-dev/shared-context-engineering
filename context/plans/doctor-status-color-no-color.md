# Plan: Doctor status colors respect NO_COLOR

## Change summary
Update `sce doctor` status-tag prefix colorization to respect the shared `NO_COLOR`/TTY policy by routing status-tag styling through `cli/src/services/style.rs`. Keep `[PASS]/[FAIL]/[WARN]/[MISS]` prefixes and layout unchanged; emit ANSI color only when `supports_color()` allows it. Update tests to cover `NO_COLOR` behavior and sync doctor contract documentation.

## Success criteria
- Status-tag prefixes are colorized only when `supports_color()` allows it; `NO_COLOR` disables ANSI for prefixes.
- Prefix labels and text layout remain unchanged.
- JSON output remains unchanged.
- Doctor tests cover `NO_COLOR` behavior (non-colored prefixes).
- Doctor contract documentation notes colorization is gated by `supports_color()`/`NO_COLOR`.

## Constraints and non-goals
- Do not change JSON output or schema.
- Do not change non-doctor commands.
- No new dependencies.
- Only status prefix colorization changes; the rest of each line remains uncolored.
- Non-TTY coverage is not required in this change (only `NO_COLOR`).

## Task stack
- [x] T01: Route status-tag styling through shared style helpers (status:done)
  - Task ID: T01
  - Goal: Replace direct `OwoColorize` usage for doctor status-tag prefixes with shared style helpers so `NO_COLOR`/TTY policy is respected.
  - Boundaries (in/out of scope):
    - In scope: `cli/src/services/doctor.rs`, `cli/src/services/style.rs` (add helper if needed).
    - Out of scope: JSON changes, other commands.
  - Done when: Status tags are colored only when `supports_color()` allows it; prefix text and layout are unchanged.
  - Verification notes: Update/add unit tests for `NO_COLOR` behavior; run doctor tests.

- [x] T02: Update doctor tests for NO_COLOR prefix behavior (status:done)
  - Task ID: T02
  - Goal: Ensure tests validate that `NO_COLOR` disables ANSI on status-tag prefixes.
  - Boundaries (in/out of scope):
    - In scope: doctor unit tests in `cli/src/services/doctor.rs`.
    - Out of scope: non-TTY behavior tests.
  - Done when: Tests assert non-colored prefixes under `NO_COLOR` and keep existing JSON expectations stable.
  - Verification notes: `nix develop -c sh -c 'cd cli && cargo test doctor'`.

- [x] T03: Sync doctor contract documentation (status:done)
  - Task ID: T03
  - Goal: Document that status-tag colorization respects `supports_color()`/`NO_COLOR`.
  - Boundaries (in/out of scope):
    - In scope: `context/sce/agent-trace-hook-doctor.md`.
    - Out of scope: other context files.
  - Done when: Doctor contract mentions `NO_COLOR`/TTY gating for status-tag colors.
  - Verification notes: Manual review.

- [x] T04: Validation and cleanup (status:done)
  - Task ID: T04
  - Goal: Run repo checks appropriate to the change and update the plan with results.
  - Boundaries (in/out of scope):
    - In scope: verification commands and cleanup only.
    - Out of scope: functional changes.
  - Done when: `nix flake check` passes and the plan is updated with evidence.
  - Verification notes: `nix flake check`.

## Open questions
None.

## Task log

### T01
- Status: done
- Completed: 2026-03-30
- Files changed: cli/src/services/doctor.rs, cli/src/services/style.rs
- Evidence: `nix develop -c sh -c 'cd cli && cargo test doctor'`
- Notes: Status-tag prefixes now use shared style helpers and respect `NO_COLOR`/TTY gating.

### T02
- Status: done
- Completed: 2026-03-30
- Files changed: cli/src/services/doctor.rs
- Evidence: `nix develop -c sh -c 'cd cli && cargo test doctor'`
- Notes: Added NO_COLOR-specific test to assert uncolored prefixes and no ANSI codes.

### T03
- Status: done
- Completed: 2026-03-30
- Files changed: context/sce/agent-trace-hook-doctor.md
- Evidence: Manual review
- Notes: Documented NO_COLOR/TTY gating for status-tag prefix colorization.

### T04
- Status: done
- Completed: 2026-03-30
- Files changed: cli/src/services/doctor.rs, context/plans/doctor-status-color-no-color.md
- Evidence: `nix flake check`
- Notes: Fixed clippy warning in ANSI stripping helper and confirmed full flake checks pass.

## Validation Report

### Commands run
- `nix flake check` -> exit 0 (all checks passed)

### Failed checks and follow-ups
- Initial `nix flake check` failed `cli-clippy` due to `clippy::while_let_on_iterator` in ANSI stripping helper; updated loop to `for` and reran `nix flake check` successfully.

### Success-criteria verification
- [x] Status-tag prefixes respect `supports_color()` / `NO_COLOR` -> `services::doctor::tests::doctor_text_output_disables_prefix_colors_when_no_color_set`.
- [x] Prefix labels and layout unchanged -> text-output tag tests normalize ANSI and assert `[PASS]/[FAIL]/[WARN]/[MISS]`.
- [x] JSON output unchanged -> `services::doctor::tests::render_json_includes_stable_fields_without_filesystem`.
- [x] Doctor contract updated to note NO_COLOR/TTY gating -> `context/sce/agent-trace-hook-doctor.md`.

### Residual risks
- None identified.
