# Plan: cli-help-about-clarity-refresh

## Goal

Make operator-facing help and about text in `cli/` easier to understand without changing command behavior.

## Scope

- In scope: top-level help text, Clap `about` strings, command-local help additions, usage examples, and tests that lock those help surfaces.
- In scope: wording cleanups that improve clarity, actionability, and consistency for implemented and placeholder commands.
- Out of scope: behavioral command changes, output-schema changes, non-help runtime messaging outside the targeted help/about surfaces, and context updates beyond plan execution artifacts unless implementation reveals an important change.

## Planning assumptions

- Treat `cli/src/command_surface.rs` and `cli/src/cli_schema.rs` as the primary help/about sources for this request.
- Keep placeholder-vs-implemented status accurate, but rewrite wording to be simpler and more operator-friendly.
- Preserve existing command surface and flags; this plan is a copy/UX pass, not a feature redesign.

## Task status

- [ ] T01
- [ ] T02
- [ ] T03
- [ ] T04

## Tasks

### T01 - Define the help-text rewrite contract and inventory target surfaces

- Goal: identify every operator-facing help/about surface in `cli/` that should be rewritten, then lock a concrete rewrite contract before text edits begin.
- In scope:
  - audit current help/about entrypoints in `cli/src/command_surface.rs` and `cli/src/cli_schema.rs`
  - capture rewrite rules for plain language, consistent terminology, and example quality
  - document the exact code/test surfaces that later tasks will touch
- Out of scope:
  - editing user-facing strings
  - changing command behavior or parser structure
- Done checks:
  - target help/about surfaces are explicitly listed in the plan or task notes
  - rewrite contract states what must stay stable versus what may be reworded
  - task boundaries for T02 and T03 are unambiguous
- Verification notes:
  - reviewer can map each planned rewrite task to concrete files and help surfaces before code changes begin

### T02 - Rewrite the top-level CLI help copy for clarity

- Goal: simplify the manually-authored top-level help text so first-time users can understand the command surface faster.
- In scope:
  - update `cli/src/command_surface.rs` help prose, usage labels, examples, and command-purpose wording
  - keep implemented/placeholder signaling accurate while making descriptions less jargon-heavy
  - update or add tests that assert the revised top-level help content
- Out of scope:
  - Clap-derived per-command `about` strings in `cli/src/cli_schema.rs`
  - runtime behavior changes
- Done checks:
  - top-level help text reads cleanly and uses consistent operator-facing terminology
  - examples reflect supported current behavior
  - top-level help tests pass with the new copy expectations
- Verification notes:
  - run targeted CLI tests covering `command_surface::help_text`

### T03 - Rewrite command and subcommand about/help strings for clarity

- Goal: improve command-local `--help` text so each command explains what it does in simpler, more direct language.
- In scope:
  - update Clap `about` strings in `cli/src/cli_schema.rs`
  - revise custom appended help content such as `auth_help_text()` examples
  - update or add tests that assert the revised command-local help output
- Out of scope:
  - top-level manual help text already handled in T02
  - non-help runtime error or success messages unless they are required to keep help tests coherent
- Done checks:
  - implemented commands and key subcommands have clearer `about` text
  - custom help additions are concise and example-driven
  - command-local help tests pass with updated wording
- Verification notes:
  - run targeted CLI tests covering `render_help_for_path`, `auth_help_text`, and command-local help rendering in `cli/src/app.rs` or `cli/src/cli_schema.rs`

### T04 - Validation and cleanup

- Goal: verify the rewritten help/about text is consistent, tested, and ready to ship.
- In scope:
  - run the relevant CLI test coverage for updated help/about surfaces
  - perform a consistency pass for terminology, implemented/placeholder labeling, and example formatting
  - confirm no additional context sync is needed beyond verify-only checks unless implementation changed durable contracts
- Out of scope:
  - introducing new copy scope beyond T02-T03 findings unless required to fix a failed verification
- Done checks:
  - relevant CLI help/about tests pass
  - wording is consistent across top-level and command-local help
  - task leaves the plan ready to close or archive after implementation sync
- Verification notes:
  - run targeted help-output tests, then the lightweight post-task verification baseline if the implementation session changes generated or shared artifacts

## Readiness

- ready_for_implementation: yes
- recommended_next_task: T01
- blockers: none
- ambiguity: none requiring plan stop; implementation should treat the current help/about surfaces in `cli/src/command_surface.rs` and `cli/src/cli_schema.rs` as the canonical rewrite target unless code review uncovers another directly connected help source in `cli/`
- missing_acceptance_criteria: none

## Handoff

- plan_name: `cli-help-about-clarity-refresh`
- plan_path: `context/plans/cli-help-about-clarity-refresh.md`
- next command: `/next-task cli-help-about-clarity-refresh T01`
