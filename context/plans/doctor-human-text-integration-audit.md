# Plan: doctor-human-text-integration-audit

## Change summary

Update the human-facing `sce doctor` text output so it renders the approved sectioned layout, reports repo-root integration health by presence only, prints bracketed status tokens (`[PASS]`, `[FAIL]`, `[MISS]`), colorizes `[PASS]` green plus `[MISS]`/`[FAIL]` red in human text mode, and adopts the simplified label/path presentation shown by the approved target example. Keep JSON output and current `--fix` behavior unchanged.

## Success criteria

- `sce doctor` text output uses the approved human-only layout with these sections in order: `Environment`, `Configuration`, `Repository`, `Git Hooks`, `Integrations`.
- The header line renders as `SCE doctor diagnose` in diagnose mode, not `SCE doctor (diagnose) PASS`.
- Human text statuses use the exact bracketed vocabulary `[PASS]`, `[FAIL]`, and `[MISS]`.
- Status meaning is fixed as: `[PASS]` = healthy, `[FAIL]` = SCE will not work unless fixed, `[MISS]` = required file is missing.
- Human text renders `[PASS]` in green and `[MISS]`/`[FAIL]` in red when color output is enabled by the shared styling policy; non-color environments still render the exact bracketed tokens without ANSI noise.
- Human text rows use the simplified `label (path)` form instead of the current `label: state (path)` form when a path is present.
- Environment/configuration rows drop redundant state words such as `present` and `expected` from the human text line when the status token already communicates health.
- Repository rows use the simplified labels `Repository` and `Hooks`; the current split `Repository root`, `Hooks path source`, and `Effective hooks directory` wording does not remain in the approved text output.
- Text-mode integrations use exactly these groups: `OpenCode plugins`, `OpenCode agents`, `OpenCode commands`, `OpenCode skills`.
- Integration checks inspect repo-root installed artifacts only and validate file presence only; they do not inspect file contents.
- A single missing file under any integration group causes that group to render `[FAIL]`, with missing child rows rendered as `[MISS]`.
- Integration parent rows render only the group name in healthy cases; they do not append prose such as `all required files present`.
- Integration child rows render as `[STATUS] relative/path (absolute/path)` in text mode.
- Git hook text output is simplified to top-level hook presence rows only; no nested text rows for `content` or `executable` remain in the human output.
- Existing JSON output shape and semantics remain unchanged.
- Existing `sce doctor --fix` behavior remains unchanged.

## Constraints and non-goals

- In scope: human text rendering, text-only status classification/styling for the approved sections, header/label/path formatting cleanup, repo-root integration presence inventory, and regression coverage for unchanged JSON/`--fix` behavior.
- Out of scope: changing JSON output, broadening `--fix`, adding content-drift inspection for integrations, changing hook repair semantics, or introducing new integration group names.
- Repo-root artifacts are the only source of truth for integration checks in this change; generated `config/.opencode/**` trees are not inspected by doctor for this task.
- For `agents`, `commands`, and `skills`, doctor should treat the installed repo-root trees as required inventory and fail the group if any expected installed file is missing.
- Assumption: the implementation will derive the required integration inventory from the installed repo-root artifact trees and/or the existing embedded setup asset catalog without changing the JSON contract.
- Assumption: text colorization will reuse the existing shared styling service and its TTY/`NO_COLOR` behavior instead of introducing doctor-specific color toggles.
- Assumption: the simplified `Hooks` repository row should display the effective hooks directory path only, not the hooks-path source metadata, in human text mode.

## Task stack

- [x] T01: `Lock doctor text-mode contract for human layout and status rules` (status:done)
  - Task ID: T01
  - Goal: Update the doctor service contract and rendering rules so text mode has the approved section order, exact `PASS`/`FAIL`/`MISS` vocabulary, simplified hook rows, and fixed parent/child status semantics.
  - Boundaries (in/out of scope): In - text-mode layout rules, status meaning, parent-group failure rules, hook text simplification. Out - JSON shape changes, `--fix` behavior changes, new machine-readable fields.
  - Done when: doctor text rendering requirements are encoded clearly enough that implementation can update the formatter without ambiguity, including the rule that integrations fail if any child file is missing and hook nested text rows are removed.
  - Verification notes (commands or checks): Review `context/sce/agent-trace-hook-doctor.md` and implementation plan acceptance checks for exact section names, status vocabulary, and explicit JSON/`--fix` non-goals.
  - Completed: 2026-04-02
  - Files changed: `context/sce/agent-trace-hook-doctor.md`, `context/context-map.md`
  - Evidence: Contract file now captures exact human text section order, `PASS`/`FAIL`/`MISS` semantics, simplified hook rows, fixed integration group names, parent/child missing-file behavior, and explicit JSON/`--fix` non-goals.
  - Notes: This task intentionally locks the downstream implementation contract without changing Rust runtime behavior; later tasks own formatter/runtime updates and regression coverage.

- [x] T02: `Implement text-only doctor layout and status mapping` (status:done)
  - Task ID: T02
  - Goal: Change the text formatter in `cli/src/services/doctor.rs` to emit the approved human layout, exact status labels, simplified hook rows, and final summary line while preserving current diagnosis data sources.
  - Boundaries (in/out of scope): In - text rendering code, summary counting, section ordering, and text status mapping. Out - changing doctor JSON serialization, altering check execution logic unrelated to text-mode needs.
  - Done when: text-mode `sce doctor` renders the approved section stack and footer, hook rows are top-level-only in text mode, and PASS/FAIL/MISS labels match the approved semantics without changing JSON output.
  - Verification notes (commands or checks): Add/update doctor text rendering tests in the CLI test suite; verify expected snapshots/strings for section order, hook rows, and footer problem count.
  - Completed: 2026-04-02
  - Files changed: `cli/src/services/doctor.rs`, `context/overview.md`, `context/glossary.md`, `context/cli/cli-command-surface.md`, `context/sce/agent-trace-hook-doctor.md`
  - Evidence: Added formatter + unit coverage for section order, top-level-only hook rows, and `PASS`/`FAIL`/`MISS` labels; `nix flake check` passed.
  - Notes: Text mode now uses the approved sectioned layout and summary footer while leaving JSON output and fix-mode semantics unchanged; repo-root multi-group integration inventory remains deferred to `T03`.

- [x] T03: `Add repo-root integration presence inventory checks` (status:done)
  - Task ID: T03
  - Goal: Teach doctor to inspect repo-root installed OpenCode integrations for `plugins`, `agents`, `commands`, and `skills`, reporting presence-only child rows and group failure when any required file is missing.
  - Boundaries (in/out of scope): In - repo-root `.opencode/**` inventory resolution, exact four integration groups, missing-file detection, and text-mode child/group classification. Out - content validation, generated `config/.opencode/**` inspection, Claude assets, or new repair actions.
  - Done when: doctor identifies missing repo-root installed integration files, renders missing children as `MISS`, renders the affected integration group as `FAIL`, and leaves groups as `PASS` only when every required file is present.
  - Verification notes (commands or checks): Add focused service tests with temporary repo-root `.opencode/` fixtures covering all-present, single-missing, and multi-missing cases across all four integration groups.
  - Completed: 2026-04-02
  - Files changed: `cli/src/services/doctor.rs`
  - Evidence: Added repo-root `.opencode/` integration inventory + text rendering/tests for plugins, agents, commands, and skills; `nix flake check` passed.
  - Notes: Doctor now derives required repo-root OpenCode inventory from installed asset expectations, treats missing required files as blocking `RepoAssets` errors, and keeps checks presence-only without changing JSON or `--fix` behavior.

- [x] T04: `Bracket and colorize doctor text status tokens` (status:done)
  - Task ID: T04
  - Goal: Update doctor human text rendering so every status token appears as `[PASS]`, `[MISS]`, or `[FAIL]`, with `[PASS]` styled green and `[MISS]`/`[FAIL]` styled red through the shared styling service, while also switching rows to the approved simplified label/path presentation.
  - Boundaries (in/out of scope): In - text-mode token formatting, color application, diagnose header text, simplified row labels, parent-row wording cleanup, and tests covering color-enabled and color-disabled rendering. Out - JSON output changes, new doctor status categories, or styling changes outside doctor text status tokens.
  - Done when: human text doctor output emits bracketed status tokens everywhere, applies green/red styling only when the shared color policy allows it, preserves exact plain-text bracketed tokens when color is disabled, renders `SCE doctor diagnose` as the diagnose header, removes redundant prose like `present`, `expected`, and `all required files present`, and formats rows as `label (path)` / `relative/path (absolute/path)` per the approved example.
  - Verification notes (commands or checks): Add/update doctor text rendering tests for bracketed tokens, color-enabled ANSI styling expectations, no-color/plain-text expectations, diagnose header text, simplified repository/config labels, and integration parent/child row formatting under the shared styling helpers.
  - Completed: 2026-04-02
  - Files changed: `cli/src/services/doctor.rs`
  - Evidence: `nix flake check`; `nix run .#pkl-check-generated`
  - Notes: Human doctor text now renders bracketed status tokens, colorizes them through the shared styling policy when enabled, removes redundant state prose from healthy rows, simplifies repository labels to `Repository` and `Hooks`, and renders integration parent rows without appended success prose.

- [x] T05: `Protect unchanged JSON and fix-mode behavior` (status:done)
  - Task ID: T05
  - Goal: Add regression coverage proving that the human text changes do not modify JSON output shape/semantics or current `sce doctor --fix` behavior.
  - Boundaries (in/out of scope): In - regression tests for JSON contract stability and unchanged fix behavior. Out - new JSON fields, new fix actions, or expanded remediation ownership.
  - Done when: automated coverage demonstrates that text-mode changes are isolated, JSON output remains byte-for-byte or semantically identical under the same fixtures, and existing fix flows still behave as before.
  - Verification notes (commands or checks): Add/update JSON-mode and `--fix` tests in the CLI suite; compare representative outputs before/after under the same controlled fixtures.
  - Completed: 2026-04-02
  - Files changed: `cli/src/services/doctor.rs`
  - Evidence: Added JSON diagnose/fix regression assertions plus `--fix` execution coverage for preserved auto-fix/manual-follow-up behavior; `nix flake check`; `nix run .#pkl-check-generated`.
  - Notes: This task was verify-only for durable context because it adds regression coverage without changing current-state doctor behavior.

- [x] T06: `Validate doctor changes and sync current-state context` (status:done)
  - Task ID: T06
  - Goal: Run final validation and update durable context so future sessions reflect the new human text doctor contract and repo-root integration presence rules.
  - Boundaries (in/out of scope): In - full verification, cleanup, and context sync for important behavior changes. Out - follow-on UX polish beyond the approved contract.
  - Done when: required verification passes, temporary scaffolding is removed, and context files reflect the resulting current-state doctor contract, including bracketed status tokens and approved color semantics.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; sync `context/overview.md`, `context/glossary.md`, and focused doctor context files if implementation changes the current-state contract.
  - Completed: 2026-04-02
  - Files changed: `context/sce/agent-trace-hook-doctor.md`, `context/plans/doctor-human-text-integration-audit.md`
  - Evidence: `nix flake check`; `nix run .#pkl-check-generated`.
  - Notes: Root shared files (`context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, `context/context-map.md`) already matched code truth; final sync corrected stale future-task wording in the focused doctor contract file so it is current-state oriented.

## Open questions

- None at plan time; blocking scope decisions have been resolved by the human for text layout, integration grouping, presence-only checks, bracketed status tokens, color intent, and unchanged JSON/`--fix` behavior.

## Validation Report

### Commands run

- `nix flake check` -> exit 0 (CLI tests, clippy, fmt, pkl parity, npm/config-lib JS checks passed)
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)

### Temporary scaffolding

- No temporary scaffolding was introduced for this plan's final task.

### Success-criteria verification

- [x] `sce doctor` text output uses the approved human-only layout with sections in order -> covered by doctor text rendering tests in `cli/src/services/doctor.rs` and retained in current-state context (`context/sce/agent-trace-hook-doctor.md`, `context/cli/cli-command-surface.md`, `context/overview.md`)
- [x] Diagnose header renders as `SCE doctor diagnose` -> covered by doctor text rendering tests in `cli/src/services/doctor.rs`
- [x] Human text statuses use exact `[PASS]`, `[FAIL]`, `[MISS]` vocabulary -> covered by doctor text rendering tests in `cli/src/services/doctor.rs` and documented in `context/sce/agent-trace-hook-doctor.md`
- [x] Human text colorizes `[PASS]` green and `[MISS]`/`[FAIL]` red when color is enabled -> covered by doctor text rendering tests in `cli/src/services/doctor.rs` and documented in `context/overview.md` / `context/glossary.md`
- [x] Human text rows use simplified `label (path)` presentation and remove redundant prose -> covered by doctor text rendering tests and documented in `context/sce/agent-trace-hook-doctor.md`
- [x] Repository rows use simplified `Repository` and `Hooks` labels -> covered by current text rendering implementation and documented in `context/overview.md`
- [x] Integrations use exactly `OpenCode plugins`, `OpenCode agents`, `OpenCode commands`, `OpenCode skills` -> covered by doctor integration tests in `cli/src/services/doctor.rs` and documented in `context/sce/agent-trace-hook-doctor.md`
- [x] Integration checks are repo-root installed-artifact presence only -> covered by doctor integration tests and documented in `context/overview.md`, `context/cli/cli-command-surface.md`, and `context/sce/agent-trace-hook-doctor.md`
- [x] Missing integration files render child `[MISS]` and parent `[FAIL]` -> covered by doctor integration tests in `cli/src/services/doctor.rs`
- [x] Git hook text output is simplified to top-level hook presence rows only -> covered by doctor text rendering tests in `cli/src/services/doctor.rs`
- [x] Existing JSON output shape and semantics remain unchanged -> covered by JSON regression tests in `cli/src/services/doctor.rs`
- [x] Existing `sce doctor --fix` behavior remains unchanged -> covered by fix-mode regression tests in `cli/src/services/doctor.rs`

### Context verification

- Verified `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, and `context/context-map.md` against code truth.
- Updated `context/sce/agent-trace-hook-doctor.md` to remove stale future-task framing and keep the file current-state oriented.

### Residual risks

- None identified within the approved scope.
