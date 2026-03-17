# Plan: sce-install-update-guidance

## Change summary

Add an info-only `sce update` CLI flow that inspects how the current `sce` binary is installed on the local machine and prints tailored update guidance instead of performing the upgrade automatically.

## Success criteria

- `sce update` is an implemented top-level command with command-local help and parser/dispatch coverage.
- The command detects the documented install surfaces this repo already supports: Cargo local install, Nix-packaged execution, and an explicit unknown fallback when the source cannot be identified confidently.
- Text output tells the user what installation source was detected and gives the exact next update command to run.
- JSON output returns a stable machine-readable report that includes the detected install source, confidence/fallback state, and the recommended update command or guidance.
- Tests cover command parsing, detection behavior, and output rendering for known and unknown install paths.

## Constraints and non-goals

- In scope: a new info-only update command, local install-source detection heuristics, deterministic text/JSON output, help text, and focused tests.
- In scope: guidance for the install methods already documented in current context and CLI help surfaces.
- Out of scope: self-updating behavior, downloading/installing binaries, network calls, release-channel management, package-manager integrations not already documented by the repo, and background update checks.
- Out of scope: changing existing `setup`, `doctor`, or `version` semantics beyond adding cross-references if implementation needs them.
- Constraint: detection must stay local, deterministic, and safe when the install method cannot be proven.

## Task stack

- [ ] T01: Add the `sce update` command contract and runtime wiring skeleton (status:todo)
  - Task ID: T01
  - Goal: Introduce `update` as a first-class CLI command with parser, help-surface, command-catalog, app-dispatch, and service-module wiring ready for a real implementation.
  - Boundaries (in/out of scope): In - `cli/src/cli_schema.rs`, `cli/src/command_surface.rs`, `cli/src/app.rs`, `cli/src/services/mod.rs`, and targeted parser/help tests. Out - real install-source detection and final output guidance logic.
  - Done when: `sce update` parses, appears in top-level help and command status listings, routes through app dispatch to a dedicated service entrypoint, and tests lock the new command surface.
  - Verification notes (commands or checks): run targeted Rust tests covering command parsing, top-level help text, and dispatch/help routing for `update`.

- [ ] T02: Implement deterministic install-source detection for documented install surfaces (status:todo)
  - Task ID: T02
  - Goal: Build the `update` service model and detection heuristics that classify the current `sce` runtime as Cargo local install, Nix-packaged execution, or unknown.
  - Boundaries (in/out of scope): In - new `cli/src/services/update.rs` detection/report types, current-executable/install-path inspection, and focused unit tests for known-path heuristics plus unknown fallback. Out - final user-facing rendering polish and broader app/help integration already covered by T01/T03.
  - Done when: the service can return a deterministic report for mocked/extracted runtime inputs, known install surfaces map to the correct classification, and ambiguous cases fall back to `unknown` rather than guessing.
  - Verification notes (commands or checks): run targeted Rust tests for detection helpers and report classification cases.

- [ ] T03: Render tailored update guidance for `sce update` text and JSON output (status:todo)
  - Task ID: T03
  - Goal: Turn the detection report into operator-facing update guidance that tells users exactly how to refresh their install for each supported source.
  - Boundaries (in/out of scope): In - `cli/src/services/update.rs` output rendering, any shared output-format parsing needed for `--format <text|json>`, dispatch integration, and tests for Cargo/Nix/unknown output contracts. Out - automatic upgrade execution or undocumented install channels.
  - Done when: text output states the detected source and recommended next command, JSON output exposes stable fields for status/source/guidance, and `sce update --help` plus runtime tests reflect the implemented behavior.
  - Verification notes (commands or checks): run targeted Rust tests for update text/JSON rendering and command execution paths.

- [ ] T04: Validation and cleanup (status:todo)
  - Task ID: T04
  - Goal: Confirm the new update command is fully verified and that context sync needs are handled before the plan closes.
  - Boundaries (in/out of scope): In - full relevant CLI test/build verification, final consistency pass across help/output/context references, and plan/context cleanup checks. Out - new feature work beyond fixes required by failed verification.
  - Done when: relevant CLI tests pass, lightweight post-task verification baseline is complete, and any important-change context updates required by the finished implementation are identified and applied.
  - Verification notes (commands or checks): run targeted CLI tests for `update`, then `nix run .#pkl-check-generated` and `nix flake check`; verify `context/` reflects the final command contract if implementation changes durable CLI behavior documentation.

## Open questions

- None. The user selected the v1 direction: info-only detection plus tailored update guidance, not self-updating behavior.

## Readiness

- ready_for_implementation: yes
- recommended_next_task: T01
- blockers: none
- ambiguity: none requiring plan stop; implementation should treat Cargo local install, Nix-packaged execution, and unknown fallback as the supported v1 detection surface unless code truth reveals another already-documented install path that clearly belongs in the same contract.
- missing_acceptance_criteria: none

## Handoff

- plan_name: `sce-install-update-guidance`
- plan_path: `context/plans/sce-install-update-guidance.md`
- next command: `/next-task sce-install-update-guidance T01`
