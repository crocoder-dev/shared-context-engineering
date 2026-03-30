# Plan: Doctor status tag colors

## Change summary
Colorize the `[PASS]`, `[FAIL]`, `[WARN]`, and `[MISS]` status tag prefixes in `sce doctor` text output (prefix only), using deterministic colors (PASS=green, FAIL=red, WARN=yellow, MISS=blue). Colorization should apply even when output is not a TTY or `NO_COLOR` is set (explicitly not disabling color).

## Success criteria
- `[PASS]`, `[FAIL]`, `[WARN]`, and `[MISS]` prefixes are colorized in `sce doctor` text output only; the rest of each line remains uncolored.
- Color mapping: PASS=green, FAIL=red, WARN=yellow, MISS=blue.
- Colorization is applied regardless of TTY/`NO_COLOR` (per requirement).
- JSON output is unchanged.
- Doctor text-output tests cover the presence of colored prefixes (or explicit ANSI sequences) while keeping JSON tests stable.
- Context contract for doctor output notes the status-tag colorization behavior.

## Constraints and non-goals
- Do not change JSON output or schema.
- Do not change non-doctor commands.
- No new dependencies.
- Only the status prefix is colored; the remainder of each line stays as-is.
- Do not respect `NO_COLOR`/TTY for this feature.

## Task stack
- [x] T01: Add colored status tag prefixes in doctor text rendering (status:done)
  - Task ID: T01
  - Goal: Colorize the `[PASS]`, `[FAIL]`, `[WARN]`, `[MISS]` prefixes in doctor text output using the specified color mapping.
  - Boundaries (in/out of scope):
    - In scope: `cli/src/services/doctor.rs` text rendering; prefix-only colorization; use existing styling utilities if applicable.
    - Out of scope: JSON output changes; other commands; colorization beyond the prefix.
  - Done when: Doctor text output prefixes render colored as specified even without TTY/`NO_COLOR`; remaining line content is uncolored.
  - Verification notes: Update/add unit tests covering colored prefixes in text output.

- [x] T02: Update doctor tests for colored prefixes (status:done)
  - Task ID: T02
  - Goal: Ensure doctor tests assert colorized status tag prefixes and maintain JSON output stability.
  - Boundaries (in/out of scope):
    - In scope: doctor unit tests in `cli/src/services/doctor.rs` (or existing doctor test module).
    - Out of scope: new integration tests or test frameworks.
  - Done when: Tests verify ANSI-colored prefixes for PASS/FAIL/WARN/MISS and JSON tests remain unchanged.
  - Verification notes: `nix develop -c sh -c 'cd cli && cargo test doctor'`.

- [x] T03: Sync doctor context documentation (status:done)
  - Task ID: T03
  - Goal: Document the status-tag colorization behavior in `context/sce/agent-trace-hook-doctor.md`.
  - Boundaries (in/out of scope):
    - In scope: doctor contract documentation only.
    - Out of scope: other context files unless required.
  - Done when: Doctor contract describes prefix-only status tag colors and the mapping.
  - Verification notes: Manual review.

- [x] T04: Validation and cleanup (status:done)
  - Task ID: T04
  - Goal: Run repo checks appropriate to the change and update the plan with results.
  - Boundaries (in/out of scope):
    - In scope: verification commands and cleanup only.
    - Out of scope: functional changes.
  - Done when: `nix flake check` passes and plan is updated with evidence.
  - Verification notes: `nix flake check`.

## Open questions
None.

## Task log

### T01
- Status: done
- Completed: 2026-03-30
- Files changed: cli/src/services/doctor.rs
- Evidence: `nix develop -c sh -c 'cd cli && cargo test doctor'`
- Notes: Colored status tag prefixes via `status_tag_prefix` with PASS/FAIL/WARN/MISS mapping; prefix-only coloring applied unconditionally.

### T02
- Status: done
- Completed: 2026-03-30
- Files changed: cli/src/services/doctor.rs
- Evidence: `nix develop -c sh -c 'cd cli && cargo test doctor'`
- Notes: Updated text-output tag tests to assert colored prefixes and preserve existing JSON checks.

### T03
- Status: done
- Completed: 2026-03-30
- Files changed: context/sce/agent-trace-hook-doctor.md
- Evidence: Manual review
- Notes: Doctor contract already documents prefix-only status tag colorization and PASS/FAIL/WARN/MISS mapping.

### T04
- Status: done
- Completed: 2026-03-30
- Files changed: context/plans/doctor-status-colors.md
- Evidence: `nix flake check`
- Notes: Validation checks passed.
