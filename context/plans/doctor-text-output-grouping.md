# Plan: Doctor text output grouping

## Change summary
Reshape `sce doctor` text output into grouped sections (Environment, Configuration, Repository, Git Hooks, Integrations) with a clear header and divider lines, while preserving existing status tag colors and `NO_COLOR` behavior. Keep JSON output unchanged and maintain all current checks; only the text presentation and wording change.

## Success criteria
- Text output for `sce doctor`, `sce doctor --fix`, and `sce doctor --all-databases` renders a header (`SCE Doctor`) plus divider lines, with section headings that are not prefixed by status tags.
- Section content lines remain prefixed with `[PASS]`, `[FAIL]`, `[WARN]`, or `[MISS]` using the same color rules as today (TTY-gated and `NO_COLOR` compliant).
- Grouping matches the requested structure: Environment, Configuration, Repository, Git Hooks, Integrations (with line ordering that keeps all existing data points).
- Problems and fix results remain reported; a concise final summary line (e.g., “No problems detected”) appears after the closing divider.
- JSON output schema and values remain unchanged.

## Constraints and non-goals
- Do not change the underlying doctor checks, readiness logic, or remediation behavior.
- Do not alter the current status tag color mapping or `NO_COLOR` handling.
- Do not add or remove JSON fields or rename JSON keys.
- Avoid removing existing information; rewording and grouping are the only changes.

## Assumptions
- The grouped layout applies to text output for all doctor modes (diagnose/fix/all-databases), while JSON output stays unchanged.
- Existing OpenCode line labels remain unless a rename is explicitly requested later (e.g., keep “OpenCode command” if that is the current label).

## Task stack
- [x] T01: Implement grouped text layout + tests (status:done)
  - Task ID: T01
  - Goal: Update `sce doctor` text rendering to include a header and divider, group lines into the five sections, and emit untagged section headings while keeping status-tagged content lines and existing information.
  - Boundaries (in/out of scope):
    - In: `cli/src/services/doctor.rs` text rendering helpers, status-tag formatting, and doctor text output tests.
    - Out: doctor check logic, JSON output, fix/repair behavior, or setup flows.
  - Done when:
    - Text output shows the new grouped layout with untagged headings/dividers and status-tagged content lines beneath each section.
    - Problems and fix results remain visible, with a final summary line after the closing divider.
    - All relevant doctor text-output tests are updated to reflect the new layout.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test doctor_text_output_'` (or the updated targeted test names for doctor text output).

- [x] T02: Update doctor contract documentation (status:done)
  - Task ID: T02
  - Goal: Sync `context/sce/agent-trace-hook-doctor.md` to document the grouped text layout and the new exception that headings/dividers are untagged while status-tagged content lines retain the existing color rules.
  - Boundaries (in/out of scope):
    - In: Text-output contract wording for doctor layout and section grouping.
    - Out: Unrelated doctor contracts, JSON schema details, or readiness taxonomy changes.
  - Done when:
    - The doctor contract reflects the grouped output layout and the untagged heading/divider lines.
  - Verification notes (commands or checks): Manual review of `context/sce/agent-trace-hook-doctor.md`.

- [x] T03: Validation and cleanup (status:done)
  - Task ID: T03
  - Goal: Run repository-level verification and confirm plan status updates.
  - Boundaries (in/out of scope):
    - In: Required validation commands and plan status updates.
    - Out: Additional feature changes.
  - Done when:
    - `nix run .#pkl-check-generated` and `nix flake check` succeed.
    - Plan checklist statuses reflect completed work.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`.

## Open questions
- None.

## Task log

### T01
- Status: done
- Completed: 2026-04-01
- Files changed: cli/src/services/doctor.rs
- Evidence: `nix flake check`
- Notes: Grouped doctor text output with header/divider lines and section headings while keeping status-tagged content lines.

### T02
- Status: done
- Completed: 2026-04-01
- Files changed: context/sce/agent-trace-hook-doctor.md
- Evidence: Manual review
- Notes: Documented grouped doctor text output layout and the untagged headings/dividers exception.

### T03
- Status: done
- Completed: 2026-04-01
- Files changed: context/plans/doctor-text-output-grouping.md
- Evidence: `nix run .#pkl-check-generated`; `nix flake check`
- Notes: Validation complete; plan status updated.

## Validation Report

### Commands run
- `nix run .#pkl-check-generated` -> exit 0 (Generated outputs are up to date.)
- `nix flake check` -> exit 0 (all checks passed)

### Success-criteria verification
- [x] Text output for `sce doctor`, `sce doctor --fix`, and `sce doctor --all-databases` renders header/dividers with untagged headings -> verified by updated text rendering in `cli/src/services/doctor.rs` and doctor text output tests.
- [x] Status tags/colors and `NO_COLOR` gating preserved -> verified by existing text output tests and unchanged styling service.
- [x] Grouping matches Environment/Configuration/Repository/Git Hooks/Integrations while preserving data points -> verified in `cli/src/services/doctor.rs` rendering logic.
- [x] Problems/fix results remain reported with final summary line -> verified in updated text rendering logic and tests.
- [x] JSON output unchanged -> no JSON rendering changes in `cli/src/services/doctor.rs`.

### Failed checks and follow-ups
- None.

### Residual risks
- None identified.
