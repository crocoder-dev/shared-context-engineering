# Plan: Doctor OpenCode plugin health summary

## Change summary
Add four new OpenCode health sections to `sce doctor` output (`OpenCode plugin`, `OpenCode agent`, `OpenCode command`, `OpenCode skills`) that always render, fail readiness when `.opencode/` is missing, and surface summarized pass/fail lines per section with detailed issue lines when failures occur while still running all existing checks. Extend JSON output with detailed OpenCode health blocks per section.

## Success criteria
- `sce doctor` reports `not_ready` when `.opencode/` is missing and records a manual-only `repo_assets` problem for that condition.
- Text output always includes four sections (`OpenCode plugin`, `OpenCode agent`, `OpenCode command`, `OpenCode skills`), each with a single summary line (`PASS`/`FAIL`).
- When a section fails, it includes indented detail lines that name the specific missing/invalid script/path and the problem.
- JSON output includes per-section OpenCode health blocks (one each for `plugin`, `agent`, `command`, `skills`) with detailed issue lists (including manifest registration, plugin file, runtime, and preset catalog where applicable).
- Existing checks remain intact; only the presentation is summarized in text output.

## Constraints and non-goals
- No automatic repair for missing `.opencode/`; remediation stays manual-only (e.g., `sce setup --opencode`).
- Do not alter hook checks, setup flows, or plugin asset generation.
- Keep existing JSON fields unchanged; only add the new block.
- Text output must remain summary-first, with details only when a summary line fails.

## Task stack
- [x] T01: Add OpenCode plugin health collection + missing-root failure (status:done)
  - Task ID: T01
  - Goal: Always compute OpenCode plugin health (even when `.opencode/` is missing), model per-area status (`plugin`, `agent`, `command`, `skills`), and raise a manual-only `repo_assets` error when `.opencode/` is absent.
  - Boundaries (in/out of scope):
    - In: `cli/src/services/doctor.rs` health collection, problem detection, readiness impact.
    - Out: setup/install behavior changes, hook checks, generated assets.
  - Done when:
    - Missing `.opencode/` creates a `repo_assets` error with manual remediation guidance and sets readiness to `not_ready`.
    - All existing OpenCode plugin checks are still executed and recorded in a structured health model that can emit per-section summary status and detailed issues.
  - Verification notes (commands or checks): Add/adjust unit tests covering missing `.opencode/` and the health model (run targeted doctor tests if present).

- [x] T02: Summarize OpenCode plugin health in text output (status:done)
  - Task ID: T02
  - Goal: Render four concise OpenCode sections (`OpenCode plugin`, `OpenCode agent`, `OpenCode command`, `OpenCode skills`) each with a summary line and detailed failing items beneath any failed section.
  - Boundaries (in/out of scope):
    - In: `format_report_lines` text rendering changes and new summary helpers.
    - Out: additional verbose per-check text lines or unrelated output reshaping.
  - Done when:
    - Each OpenCode section always appears in text output.
    - Each section shows PASS when its checks pass, otherwise FAIL.
    - When a section fails, one or more indented detail lines name the specific missing/invalid path and problem.
  - Verification notes (commands or checks): Update or add text output tests to assert the new section and failure messaging.

- [x] T03: Extend JSON output with detailed OpenCode plugin health block (status:done)
  - Task ID: T03
  - Goal: Add OpenCode health blocks to JSON output for `plugin`, `agent`, `command`, and `skills` with detailed issues for each.
  - Boundaries (in/out of scope):
    - In: `render_report_json` schema extension and serialization.
    - Out: breaking changes to existing JSON fields.
  - Done when:
    - JSON output includes per-section OpenCode health blocks with fields for status and detailed issues (including manifest registration, plugin file, runtime, preset catalog as applicable).
    - Existing JSON output fields remain unchanged.
  - Verification notes (commands or checks): Add/adjust JSON output tests asserting the new block and statuses.

- [x] T04: Update doctor contract context (status:done)
  - Task ID: T04
  - Goal: Sync `context/sce/agent-trace-hook-doctor.md` to reflect the new `.opencode/` missing failure and summarized OpenCode plugin health output.
  - Boundaries (in/out of scope):
    - In: Contract text updates for OpenCode plugin checks and readiness impact.
    - Out: unrelated contract edits.
  - Done when:
    - Contract explicitly states `.opencode/` missing is a blocking `repo_assets` error (manual-only) and the text output includes a summarized OpenCode plugin health section.
  - Verification notes (commands or checks): Manual review of the updated contract section.

- [x] T05: Validation and cleanup (status:done)
  - Task ID: T05
  - Goal: Run full verification and ensure context alignment.
  - Boundaries (in/out of scope):
    - In: repo validation and any required cleanup.
    - Out: additional feature changes.
  - Done when:
    - `nix run .#pkl-check-generated` and `nix flake check` succeed.
    - Plan tasks are updated with completion status and any required context sync is confirmed.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`.

## Open questions
- None.

## Task log

### T01
- Status: done
- Completed: 2026-03-31
- Files changed: cli/src/services/doctor.rs, context/sce/agent-trace-hook-doctor.md
- Evidence: `nix develop -c sh -c 'cd cli && cargo test doctor_reports_opencode_root_missing && cargo test fix_mode_creates_missing_agent_trace_directory'`
- Notes: Added OpenCode health model tracking and now emit a manual-only repo_assets error when `.opencode/` is missing; updated tests for the new readiness behavior.

### T02
- Status: done
- Completed: 2026-03-31
- Files changed: cli/src/services/doctor.rs, context/sce/agent-trace-hook-doctor.md
- Evidence: `nix develop -c sh -c 'cd cli && cargo test doctor_text_output_includes_opencode_sections_and_details'`
- Notes: Added OpenCode section summaries to doctor text output with detail lines on failure and new test coverage.

### T03
- Status: done
- Completed: 2026-03-31
- Files changed: cli/src/services/doctor.rs
- Evidence: `nix develop -c sh -c 'cd cli && cargo test render_json_includes_opencode_health_sections'`
- Notes: Added OpenCode health block to doctor JSON output with per-section status and issue details plus JSON coverage.

### T04
- Status: done
- Completed: 2026-03-31
- Files changed: none (context already aligned)
- Evidence: Manual review of `context/sce/agent-trace-hook-doctor.md` (OpenCode sections + `.opencode/` missing error documented)
- Notes: No additional edits required; contract already reflected the new OpenCode sections and missing-root error.

### T05
- Status: done
- Completed: 2026-03-31
- Files changed: cli/src/services/doctor.rs, context/plans/doctor-opencode-plugin-health-summary.md
- Evidence: `nix run .#pkl-check-generated`; `nix flake check` (passed)
- Notes: Resolved clippy issues (refactor + borrow fix) and reran full validation successfully.

## Validation Report

### Commands run
- `nix run .#pkl-check-generated` -> exit 0 (Generated outputs are up to date.)
- `nix flake check` -> exit 0 (all checks passed)

### Success-criteria verification
- [x] `.opencode/` missing reports `not_ready` with manual-only `repo_assets` error -> covered by `doctor_reports_opencode_root_missing` test.
- [x] Text output includes `OpenCode plugin/agent/command/skills` sections with PASS/FAIL and detail lines on failure -> covered by `doctor_text_output_includes_opencode_sections_and_details` test.
- [x] JSON output includes per-section OpenCode health blocks with detailed issues -> covered by `render_json_includes_opencode_health_sections` test.
- [x] Existing checks remain intact -> no changes to existing check logic; full `nix flake check` passed.

### Residual risks
- None identified.
