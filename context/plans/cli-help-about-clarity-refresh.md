# Plan: cli-help-about-clarity-refresh

## Change summary

Refresh the CLI's user-facing wording so `sce` communicates more clearly to both humans and LLM agents without changing command behavior, flags, exit codes, or machine-readable payload contracts.

This plan treats "wording surfaces" broadly and starts by inventorying the current communication surfaces inside `cli/`, then rewrites them in atomic slices. The current known surface map is:

- top-level manually authored help and command catalog text in `cli/src/command_surface.rs`
- Clap `about` text and command-local `--help` output in `cli/src/cli_schema.rs`
- custom help appendices such as `auth_help_text()` in `cli/src/cli_schema.rs`
- app-level parse/validation/runtime/dependency diagnostics and default `Try:` guidance in `cli/src/app.rs`
- text-mode success, placeholder, and status messages rendered by CLI service modules under `cli/src/services/`
- tests that lock current wording or help-shape expectations

The rewrite should follow the agent-friendly CLI principles from the referenced article where they fit the existing product shape: predictable wording, direct action-oriented phrasing, explicit machine-readable paths, small-context examples, and defensive clarity for agent mistakes. This plan does not add new CLI surfaces such as schema introspection or `--help --json`; it improves the current communication contract first.

## Success criteria

- Every current user-facing wording surface in `cli/` is explicitly reviewed and assigned to a task before implementation begins.
- Top-level help and command-local help use clearer, more consistent phrasing for both first-time human operators and LLM agents.
- Runtime diagnostics and text-mode command outputs use stable, direct wording with actionable remediation where appropriate.
- Implemented versus placeholder status remains accurate across all rewritten surfaces.
- Existing machine-readable contracts remain stable unless a task explicitly proves a wording-only JSON text change is safe and necessary.
- Tests are updated to lock the revised wording and help output where this repo already treats wording as a contract.
- Mandatory context sync is completed as a final verification step only if the completed change qualifies as an important context update.

## Constraints and non-goals

- In scope: wording, examples, labels, command descriptions, help prose, placeholder phrasing, remediation guidance, and text-mode user communication in the Rust CLI.
- In scope: identifying all current wording surfaces before rewrite work starts, then slicing edits into atomic commit-sized tasks.
- In scope: preserving dual-audience usability for humans and LLM agents.
- Out of scope: new flags, new commands, schema/introspection features, MCP surface changes, output-format expansion, auth flow changes, setup flow changes, or any behavior change unrelated to wording.
- Out of scope: broad context-writing work beyond mandatory final context-sync verification.
- Constraint: keep stdout/stderr routing, exit-code taxonomy, and JSON field contracts stable.
- Constraint: preserve existing placeholder honesty; wording may become clearer but must not overstate implementation status.

## Task stack

- [ ] T01: `Inventory wording surfaces and refresh top-level help contract` (status:todo)
  - Task ID: T01
  - Goal: Confirm the full current wording-surface inventory in code, then rewrite the top-level `sce` help/catalog copy so it sets a clearer communication contract for both humans and agents.
  - Boundaries (in/out of scope): In - `cli/src/command_surface.rs`, any directly coupled top-level help tests, and surfacing any newly discovered wording locations in task notes before edits proceed. Out - command-local help strings, runtime diagnostics, service-rendered status text, or behavior changes.
  - Done when: The top-level help text reflects the approved inventory framing, uses clearer command purposes/examples, keeps implemented-vs-placeholder truth intact, and tests lock the revised output.
  - Verification notes (commands or checks): Run the narrowest CLI tests that cover `command_surface::help_text` and top-level default-help rendering.

- [ ] T02: `Rewrite command-local help and about text` (status:todo)
  - Task ID: T02
  - Goal: Update Clap `about` strings and custom command-local help additions so each command explains itself in simpler, more direct language for humans and agents.
  - Boundaries (in/out of scope): In - `cli/src/cli_schema.rs`, command/subcommand descriptions, examples appended to help text, and tests covering command-local help rendering. Out - app-level diagnostics and service-rendered runtime status text.
  - Done when: Implemented commands and key subcommands have clearer `about` text, command-local help examples are concise and accurate, and help-output tests pass with the new wording.
  - Verification notes (commands or checks): Run targeted tests covering `render_help_for_path`, `auth_help_text`, and command-local help assertions in `cli/src/cli_schema.rs` and `cli/src/app.rs`.

- [ ] T03: `Refresh diagnostics and text-mode runtime messaging` (status:todo)
  - Task ID: T03
  - Goal: Make stderr diagnostics and text-mode command responses easier to understand while preserving the stable error/output contracts that automation depends on.
  - Boundaries (in/out of scope): In - wording in `cli/src/app.rs` and relevant `cli/src/services/*.rs` text renderers, including placeholder messaging and `Try:` guidance where already supported. Out - JSON field names, exit codes, command routing, and non-wording behavior changes.
  - Done when: Parse/validation/runtime/dependency diagnostics are clearer, text-mode success/placeholder/status messages are consistent across commands, and updated tests cover the revised wording where those surfaces are already asserted.
  - Verification notes (commands or checks): Run the narrowest relevant CLI tests for app error rendering plus service-specific text-rendering tests for touched modules.

- [ ] T04: `Validate wording consistency and complete mandatory context sync` (status:todo)
  - Task ID: T04
  - Goal: Verify the wording refresh as a coherent whole, clean up any remaining inconsistencies, and complete final validation plus mandatory context-sync checks.
  - Boundaries (in/out of scope): In - full relevant validation for touched CLI wording surfaces, final consistency pass across help and runtime text, and context-sync verification/update only if the completed change alters durable current-state CLI contracts. Out - new wording scope not required by failed validation.
  - Done when: Revised help/runtime wording is consistent, targeted validation passes, and required context sync is either completed or explicitly verified as unnecessary.
  - Verification notes (commands or checks): Run targeted wording/help tests first, then the repo's lightweight post-task verification baseline and final context-sync verification appropriate to the completed implementation.

## Open questions

- None. Scope is now: identify all current wording surfaces, optimize for both humans and LLM agents, and limit non-code artifacts to mandatory context sync only.

## Readiness

- ready_for_implementation: yes
- recommended_next_task: T01
- blockers: none
- ambiguity: none
- missing_acceptance_criteria: none

## Handoff

- plan_name: `cli-help-about-clarity-refresh`
- plan_path: `context/plans/cli-help-about-clarity-refresh.md`
- next command: `/next-task cli-help-about-clarity-refresh T01`
