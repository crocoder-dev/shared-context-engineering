# Plan: sce-cli-colorize-human-readable-output

## Change summary

Investigate why human-readable `sce` command output is not consistently colorful, then standardize CLI rendering so human-facing text surfaces use the shared styling service whenever color is enabled.

The target operating model for this plan is:

- Human-readable help and text-mode command output use the shared styling helpers from `cli/src/services/style.rs` instead of raw unstyled strings.
- Interactive prompt-adjacent human-facing text and stderr diagnostics also follow the same shared color policy when they are text-mode surfaces.
- Machine-readable output stays unstyled: JSON, completion scripts, piped/non-TTY output, and `NO_COLOR` flows remain plain text.
- The implementation should preserve existing wording/contracts unless color application requires localized rendering refactors.

## Success criteria

- All human-facing text-mode command/help surfaces consistently use the shared styling service when color is enabled.
- Top-level help and command-local help render with the intended heading/command/example/placeholder styling.
- Human-readable text outputs from command services use existing success/label/value/prompt/error styling helpers where appropriate.
- Human-readable stderr diagnostics are colorized consistently with the existing stderr color policy.
- JSON output, completion output, non-TTY output, and `NO_COLOR` flows remain unstyled.
- Tests and durable context are updated to reflect the final current-state colorization contract.

## Constraints and non-goals

- In scope: auditing human-readable CLI render paths, wiring missing calls to the shared styling service, localized refactors needed to keep color application consistent, and directly affected tests/context.
- In scope: preserving the current `NO_COLOR` and TTY-detection policy as the source of truth for whether colors appear.
- In scope: fixing inconsistencies across stdout help/text surfaces, stderr diagnostics, and prompt-adjacent human-readable text.
- Out of scope: changing JSON schemas or machine-readable payload fields.
- Out of scope: altering completion-script output styling behavior.
- Out of scope: broad wording rewrites unrelated to colorization consistency.
- Every executable task must remain one coherent commit unit.

## Task stack

- [x] T01: `Audit CLI human-readable render paths for missing styling` (status:done)
  - Task ID: T01
  - Goal: Identify which help, command-output, stderr-diagnostic, and prompt-adjacent human-readable render paths bypass `cli/src/services/style.rs` and define the exact in-scope remediation list.
  - Boundaries (in/out of scope): In - code-path audit, gap inventory, and any test expectations needed to lock the identified target surfaces. Out - implementing the colorization fixes themselves beyond minimal audit-driven test scaffolding.
  - Done when: The implementation surface is enumerated in code/tests with a clear list of missing styled paths covering help, stdout text output, stderr text diagnostics, and prompt-adjacent text surfaces.
  - Verification notes (commands or checks): Review render call sites in `cli/src/app.rs` and `cli/src/services/*.rs`; confirm each human-readable surface is classified as styled vs missing-style and exclude JSON/completion paths from the remediation list.
  - Completed: 2026-03-31
  - Files changed: `cli/src/main.rs`, `cli/src/styling_audit.rs`, `context/plans/sce-cli-colorize-human-readable-output.md`
  - Evidence: Added a test-owned audit inventory covering help/stdout/stderr/prompt surfaces plus machine-readable exclusions; targeted Rust tests pass.
  - Notes: Current missing shared-styling targets are command-local help, auth help base text, top-level stderr diagnostic body, observability log-file fallback stderr, setup prompt title, and setup prompt choice labels.

- [x] T02: `Colorize help and shared human-readable stdout rendering` (status:done)
  - Task ID: T02
  - Goal: Apply shared styling helpers to top-level help, command-local help, and other stdout human-readable text rendering paths that currently emit raw strings.
  - Boundaries (in/out of scope): In - top-level help rendering, command-local help payloads, and stdout text-mode command output paths that should use shared style helpers. Out - stderr diagnostics, prompt-adjacent text, JSON/completion outputs, and unrelated wording changes.
  - Done when: Help/example/placeholder/heading styling is applied consistently on stdout human-readable surfaces while preserving existing plain-text behavior for non-TTY and `NO_COLOR` runs.
  - Verification notes (commands or checks): Targeted CLI/help rendering tests covering styled-vs-unstyled behavior boundaries; manual inspection of representative `sce --help` and text-mode command outputs under color-enabled vs color-disabled conditions.
  - Completed: 2026-03-31
  - Files changed: `cli/src/cli_schema.rs`, `cli/src/command_surface.rs`, `cli/src/main.rs`, `cli/src/services/style.rs`, `cli/src/services/style/tests.rs`, `context/cli/placeholder-foundation.md`, `context/cli/styling-service.md`, `context/plans/sce-cli-colorize-human-readable-output.md`
  - Evidence: `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Command-local clap help now runs through a shared stdout styling pass for headings/usage command tokens/placeholders, and top-level help examples now use shared example-command styling.

- [x] T03: `Colorize stderr diagnostics and prompt-adjacent text surfaces` (status:done)
  - Task ID: T03
  - Goal: Apply the shared stderr and prompt styling helpers to remaining human-facing text surfaces outside stdout help/output so diagnostics and interactive guidance follow the same policy.
  - Boundaries (in/out of scope): In - stderr diagnostics, interactive prompt labels/values, and any shared helper refactor needed to keep these surfaces consistent. Out - stdout help/output already covered in T02, JSON/completion output, and non-human-facing internal logging.
  - Done when: Human-readable stderr and prompt-adjacent text surfaces use the shared style helpers consistently and still degrade to plain text for non-TTY/`NO_COLOR` scenarios.
  - Verification notes (commands or checks): Targeted tests for stderr/helpful diagnostic rendering and prompt-adjacent output; manual review of representative auth/setup/validation flows that emit human-facing stderr or interactive guidance.
  - Completed: 2026-03-31
  - Files changed: `cli/src/app.rs`, `cli/src/services/observability.rs`, `cli/src/services/setup.rs`, `cli/src/services/setup/tests.rs`, `cli/src/services/style.rs`, `cli/src/services/style/tests.rs`, `context/plans/sce-cli-colorize-human-readable-output.md`
  - Evidence: `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Top-level stderr diagnostics now style the heading and message body through the shared stderr color policy, observability log-file write failures use the same stderr styling path, and interactive setup prompt titles/choice labels now use shared prompt styling while remaining plain in non-TTY/`NO_COLOR` flows.

- [x] T04: `Run validation and sync colorization context` (status:done)
  - Task ID: T04
  - Goal: Validate the CLI colorization changes end to end, remove stale context wording about unstyled human-readable output, and confirm durable context matches the implemented styling contract.
  - Boundaries (in/out of scope): In - required validation, direct context updates for the styling contract, and cleanup of stale references in context files. Out - new CLI feature work beyond the colorization scope.
  - Done when: Required validation passes, context reflects the current styling contract for human-readable text surfaces, and no in-scope stale wording remains about those surfaces being unstyled when color is enabled.
  - Verification notes (commands or checks): Run the repo validation baseline plus targeted CLI test coverage for touched renderers; review `context/overview.md`, `context/cli/styling-service.md`, `context/cli/placeholder-foundation.md`, and any affected command-contract context for parity.
  - Completed: 2026-03-31
  - Files changed: `context/overview.md`, `context/plans/sce-cli-colorize-human-readable-output.md`
  - Evidence: `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Reviewed `context/cli/styling-service.md` and `context/cli/placeholder-foundation.md` for parity; both already matched the implemented colorization contract, so only the root overview summary required a small current-state refinement.

## Open questions

- None. The user confirmed that the scope is human-readable surfaces only, that every human-facing text surface should be colorized consistently, and that the acceptance criterion is shared-styling coverage with JSON/non-TTY/`NO_COLOR` remaining unstyled.

## Validation Report

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (all evaluated flake checks passed; warning noted that incompatible non-local systems were omitted without `--all-systems`)

### Removed temporary scaffolding

- None

### Success-criteria verification

- [x] All human-facing text-mode command/help surfaces consistently use the shared styling service when color is enabled -> confirmed by implemented `cli/src/services/style.rs` helpers plus current-state context in `context/cli/styling-service.md` and `context/cli/placeholder-foundation.md`
- [x] Top-level help and command-local help render with the intended heading/command/example/placeholder styling -> documented in `context/cli/placeholder-foundation.md` and `context/cli/styling-service.md`; validation baseline passed
- [x] Human-readable text outputs from command services use existing success/label/value/prompt/error styling helpers where appropriate -> confirmed by current code contracts in `cli/src/services/style.rs`, `cli/src/app.rs`, and `cli/src/services/setup.rs`
- [x] Human-readable stderr diagnostics are colorized consistently with the existing stderr color policy -> confirmed by `cli/src/app.rs` shared stderr styling path and matching context
- [x] JSON output, completion output, non-TTY output, and `NO_COLOR` flows remain unstyled -> confirmed by styling-service policy docs and validation baseline with no drift detected
- [x] Tests and durable context are updated to reflect the final current-state colorization contract -> confirmed by updated `context/overview.md`, existing aligned domain files, completed task records, and successful validation commands

### Failed checks and follow-ups

- None

### Residual risks

- None identified within the approved colorization scope
