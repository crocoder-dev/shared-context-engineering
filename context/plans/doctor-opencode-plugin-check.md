# Plan: Add OpenCode plugin checks to `sce doctor`

## Change summary
- Extend `sce doctor` to validate the OpenCode `sce-bash-policy` plugin when `.opencode/` exists in the target repo.
- Checks include plugin registration in `.opencode/opencode.json`, plugin file presence, and content drift against canonical embedded assets.
- Report issues as manual-only (no `--fix` repair), with severity mapping: registry missing = error, file missing = warning, content drift = warning.

## Success criteria
- When `.opencode/` does **not** exist, `sce doctor` does not report plugin-related issues.
- When `.opencode/` exists:
  - Missing `sce-bash-policy` registration in `.opencode/opencode.json` yields a `repo_assets` problem with **severity=error** and **fixability=manual_only**, and readiness becomes `not_ready`.
  - Missing plugin file (`.opencode/plugins/sce-bash-policy.ts`) yields a `repo_assets` problem with **severity=warning** and **fixability=manual_only`.
  - Content drift between the installed plugin file and canonical embedded asset yields a `repo_assets` problem with **severity=warning** and **fixability=manual_only`.
- Text and JSON outputs include deterministic plugin check reporting with actionable manual remediation steps.
- `sce doctor --fix` does not attempt plugin repairs and reports manual outcomes for plugin-related findings.
- Tests cover plugin check output shape and severity/fixability mapping.

## Constraints and non-goals
- No auto-fix or regeneration steps in `sce doctor --fix` for plugin issues.
- No changes to the plugin content itself or to generated config assets.
- Scope limited to the OpenCode `sce-bash-policy` plugin; no Claude hook/plugin checks.
- Keep existing doctor readiness semantics and output contracts stable outside the new plugin diagnostics.

## Task stack
- [x] T01: Add OpenCode plugin diagnostics to `sce doctor` (status:done)
  - Task ID: T01
  - Goal: Detect `sce-bash-policy` plugin registration, file presence, and content drift when `.opencode/` exists, and emit `repo_assets` problems with the agreed severity/fixability mapping.
  - Boundaries (in/out of scope): In - doctor service logic, problem records, remediation text/JSON fields, readiness impact for registry-missing errors. Out - any plugin repair logic, setup/regeneration flows, or non-OpenCode plugin checks.
  - Done when: `sce doctor` reports plugin issues only when `.opencode/` exists, uses `repo_assets` category, maps severities as specified, and keeps `--fix` read-only for plugin issues.
  - Verification notes (commands or checks): Update/execute doctor output-shape tests covering plugin states; `nix flake check`.

- [x] T02: Extend doctor output tests for plugin reporting (status:done)
  - Task ID: T02
  - Goal: Add/adjust tests to verify text/JSON output includes plugin findings and severity/fixability mapping.
  - Boundaries (in/out of scope): In - deterministic output fixtures and assertions for plugin states. Out - integration tests that touch real filesystem or git.
  - Done when: Tests assert presence/absence of plugin findings based on `.opencode/` existence and verify severity/fixability mappings.
  - Verification notes (commands or checks): `nix flake check`.

- [x] T03: Sync context docs with new doctor coverage (status:done)
  - Task ID: T03
  - Goal: Update `context/sce/agent-trace-hook-doctor.md` (and any related CLI context file) to record the new repo-assets plugin checks and manual remediation posture.
  - Boundaries (in/out of scope): In - current-state documentation updates for doctor coverage. Out - implementation changes or broad documentation rewrites.
  - Done when: Context files describe the OpenCode plugin check, severity/fixability mapping, and the “only if .opencode exists” gating rule.
  - Verification notes (commands or checks): Manual review for accuracy vs implemented behavior.

- [x] T04: Validation and cleanup (status:done)
  - Task ID: T04
  - Goal: Run required repo checks and ensure plan state is current.
  - Boundaries (in/out of scope): In - `nix flake check` (and `nix run .#pkl-check-generated` if any generated assets were touched). Out - additional tests not required by the change.
  - Done when: Validation passes and the plan reflects completed tasks.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated` (only if generated outputs were modified).

## Open questions
- None.

## Task log

### T01
- Status: done
- Completed: 2026-03-24
- Files changed: cli/src/services/doctor.rs
- Evidence: `nix flake check`
- Notes: Added repo-scoped OpenCode plugin registry/file/drift diagnostics gated on `.opencode/`, with error-vs-warning severity mapping and manual-only remediation; readiness now considers error severity only.

### T03
- Status: done
- Completed: 2026-03-24
- Files changed: context/sce/agent-trace-hook-doctor.md, context/cli/placeholder-foundation.md
- Evidence: Manual review for alignment with implemented doctor behavior.
- Notes: Documented OpenCode plugin registry/file/content-drift checks gated on `.opencode/` with error-vs-warning severity mapping and manual-only remediation posture.

### T02
- Status: done
- Completed: 2026-03-24
- Files changed: cli/src/services/doctor.rs
- Evidence: `nix flake check`
- Notes: Added JSON output-shape tests covering `.opencode/` gating plus registry-missing error, file-missing warning, and drift warning cases.

### T04
- Status: done
- Completed: 2026-03-24
- Files changed: context/plans/doctor-opencode-plugin-check.md
- Evidence: `nix flake check`
- Notes: No generated assets changed; skipped `nix run .#pkl-check-generated`.

## Validation Report

### Commands run
- `nix flake check` -> exit 0 (all checks passed; omitted incompatible systems: aarch64-darwin, aarch64-linux, x86_64-darwin)

### Success-criteria verification
- [x] Plugin checks gated on `.opencode/` existence -> covered by `doctor_skips_opencode_plugin_checks_without_opencode_root` test in `cli/src/services/doctor.rs`.
- [x] Missing registry yields `repo_assets` error/manual_only and readiness not_ready -> covered by `doctor_reports_opencode_plugin_registry_missing` test.
- [x] Missing plugin file yields `repo_assets` warning/manual_only -> covered by `doctor_reports_opencode_plugin_missing_file_warning` test.
- [x] Content drift yields `repo_assets` warning/manual_only -> covered by `doctor_reports_opencode_plugin_drift_warning` test.
- [x] `--fix` does not attempt plugin repair -> no auto-fix path added; problems render manual-only fix results.
- [x] Text/JSON output includes deterministic plugin reporting -> JSON output-shape tests assert problems array content.

### Residual risks
- None identified.
