# Plan: sce-plan-check

## Change summary

Add `sce plan check <path>`, a deterministic, read-only Rust CLI command that
validates the structure and reference integrity of one selected SCE plan. It
works in the distributed `sce` binary and needs no Pkl, Nix, LLM, or network
access. Today, structural checking exists only as the test-only Pkl checker
`config/pkl/renderers/plan-structure.pkl` (run through
`config/pkl/renderers/plan-structure-check.pkl` and the fixtures under
`config/pkl/renderers/fixtures/plan-structure/` by `config/pkl/check-generated.sh`).
The `/change-to-plan` and `/validate` workflows apply the `Reference rules` by
hand. Users of the shipped binary have no deterministic checker.

The Rust checker parses Markdown with `markdown-rs` (`markdown` crate) into a
typed GFM `mdast` tree, extracts an SCE plan model with source positions, and
runs pure validation over that model. It keeps the Pkl checker's behavior: both
implementations run against the same fixtures, driven by one shared expectations
manifest. The canonical `/change-to-plan` and `/validate` workflow sources then
call `sce plan check` for structured plans and keep their current legacy-plan
behavior. This extends existing behavior. The Pkl checker stays in place as the
generation-time reference implementation.

## Design decisions

- D1: `markdown-rs` (`markdown` crate, `to_mdast` with `ParseOptions::gfm()`) is the only Markdown parser; plan structure is extracted from the typed `mdast` tree, never from line scanning.
  - Boundary: `cli/src/services/plan/extract.rs` is the only code that touches `markdown::mdast::Node`. Sections are the runs of root children between `Heading` nodes, identified by depth and plain text. A definition is a `ListItem` of a root-level `List` in its authoritative section whose first paragraph starts with an ID token and `:`; GFM `checked` gives the checkbox. A definition's fields are the `ListItem`s of its directly nested `List`. Field values are sliced from the original source using node positions, so Markdown is never rewritten. `Code` nodes, both fenced and indented, are never visited for structure. Simple string parsing applies only to ID tokens and reference values after structural extraction.
  - Rationale: CommonMark/GFM nesting, fences, and checkboxes are what the plan rules actually depend on. An AST gives them correctly, along with line positions for diagnostics.
  - Rejected: Porting the Pkl regex/line scanner to Rust — this is the custom Markdown parser the request forbids. `comrak` / `pulldown-cmark` — the request names `markdown-rs` explicitly, and a second parser adds no value.
  - Trade-offs: The AST follows CommonMark list-nesting rules, while the Pkl checker requires exactly `entry indent + 2` spaces. Inputs that only one of them treats as nested can diverge. The fixtures pin the shared behavior, and the contract document (D3) records the known divergences.
- D2: Parsing, plan representation, semantic validation, and CLI integration are separate layers in a new `cli/src/services/plan/` service with static dispatch.
  - Boundary: `extract.rs` turns source into `Plan`: headings with lines, plus `PlanEntry { id, kind, checked, line, fields: Vec<PlanField { key, value, line }> }` grouped by kind. `validate.rs` holds a pure `fn validate(plan: &Plan) -> PlanReport { structured, diagnostics: Vec<PlanDiagnostic { line: Option<usize>, message }> }` with no I/O. `command.rs` reads the file, calls extract then validate, and renders the result. `cli_schema.rs`, `services/parse/command_runtime.rs`, and `services/command_registry.rs` add one `Plan`/`PlanCommand` variant each. No `dyn`, no boxed futures, no async, no persistent state.
  - Rationale: Pure validation over a small owned model can be tested without the CLI. Keeping the AST out of `validate.rs` stops parser types from leaking into the rules.
  - Rejected: Validating directly over `mdast` nodes — this mixes traversal with rules and makes the rules parser-shaped. A trait-based checker abstraction — there is only one implementation.
  - Trade-offs: One small intermediate model to maintain. It is kept minimal: one entry type with a kind enum, not four structs per kind.
- D3: One shared fixture contract prevents Pkl/Rust drift. Both checkers run the same `config/pkl/renderers/fixtures/plan-structure/*.md` fixtures from a single manifest, `config/pkl/renderers/fixtures/plan-structure/expectations.tsv` (fixture, `structured`/`legacy`/`fail`, expected diagnostic substrings). Rust diagnostic messages reuse the Pkl defect wording verbatim.
  - Boundary: `config/pkl/check-generated.sh` reads the manifest instead of hard-coding the `expect_plan_structure_pass`/`expect_plan_structure_failure` calls. A Rust test in `cli/src/services/plan/` reads the same manifest and fixtures at test runtime through `CARGO_MANIFEST_DIR/../config/pkl/renderers/fixtures/plan-structure`. That tree already reaches the Crane sandbox through `cliBuildInputFileset` (`./config/pkl`), so no flake change is needed. The behavioral contract is documented in `context/sce/plan-structure-contract.md`.
  - Rationale: A single manifest makes "same fixtures, same verdicts, same defect text" mechanically true rather than a convention.
  - Rejected: Separate Rust fixture copies or a Rust-only expectations table — two equivalent sets drift. Making the Rust CLI or its tests invoke Pkl — the distributed binary must not need Pkl.
  - Trade-offs: Pkl defect wording becomes a cross-implementation contract, and rewording requires touching both checkers.
- D4: Outcome mapping reuses existing CLI classification and the Pkl result wording.
  - Boundary: A valid plan prints `structured plan: reference integrity holds` or `legacy plan: structured reference rules do not apply` to stdout and exits `0`. A structurally invalid plan returns `CliError::validation` (exit `3`, `SCE-ERR-VALIDATION`), whose stderr message lists one diagnostic per line as `{path}:{line}: {message}`, or `{path}: {message}` for plan-level defects such as a missing heading. `{path}` is the path as given. Diagnostics are ordered by line, then message, for deterministic output. Missing or unreadable files go through the existing runtime-failure path (exit `4`) with `with_context` naming the path.
  - Rationale: Exit `3` already means validation failure in `context/sce/cli-exit-code-contract.md`. Reusing the Pkl strings keeps D3's contract simple.
  - Rejected: A new exit code for plan defects — it breaks the class-based contract. Printing diagnostics to stdout — stdout is reserved for command payloads.
  - Trade-offs: The diagnostics share the `CliError` stderr envelope (including the class's `Try:` guidance) rather than a bespoke report format.
- D5: The workflows trust the checker only when it actually ran. A result counts only on exit `0` with one of the two D4 classification lines on stdout, or exit `3` with diagnostics. Any other outcome means the checker is unavailable, and the workflow falls back to its existing manual `Reference rules` pass and says so.
  - Boundary: Canonical `config/pkl/base/workflow-change-to-plan.pkl` (step 2.9/2.10) and `config/pkl/base/workflow-validate.pkl` (step 4 "Check plan references") only. `/next-task` is untouched.
  - Rationale: An older `sce` binary resolves the unknown `plan` command to help with exit `0` (per the exit-code contract), so exit status alone would report a false pass.
  - Rejected: Requiring the new binary and failing without it — this would break workflows for users on an older `sce`.
  - Trade-offs: The workflow prose carries one extra fallback rule.

## Acceptance criteria

- [ ] AC1: `sce plan check <path>` validates exactly the selected file, read-only. It accepts relative and absolute paths. It exits `0` and prints the structured or legacy classification line for a valid plan. It exits `3` and prints `{path}:{line}: {message}` diagnostics on stderr for a structurally invalid plan. A missing or unreadable file fails through the existing runtime-failure handling. It never executes `Check:`, `Validate:`, or `Verify:` content.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::` plus `nix run .#sce -- plan check config/pkl/renderers/fixtures/plan-structure/valid-structured-plan.md; echo $?` (expect `0`) and the same against `duplicate-id.md` (expect `3`).
  - Decisions: D1, D2, D4
  - Scenarios: S1, S2, S3, S4
- [ ] AC2: The Rust validator and the Pkl checker give the same verdict and defect text for every fixture under `config/pkl/renderers/fixtures/plan-structure/`, both driven by the single `expectations.tsv` manifest. The fixtures cover valid structured, inspection-only, fenced-example, legacy, and legacy-with-fenced-structured-example plans. They also cover duplicate IDs, missing and empty `Covers:`, dangling references, wrong reference kinds, uncovered AC, unreferenced scenario, missing required fields, misplaced definitions and fields, wrong heading placement, and keywords only inside fenced examples.
  - Validate: `nix run .#pkl-check-generated` and `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::fixture`
  - Decisions: D1, D3
  - Scenarios: S5, S6
- [ ] AC3: Every definition-, field-, and heading-anchored diagnostic carries the 1-based source line of the offending node, taken from `mdast` positions.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::diagnostic_lines`
  - Decisions: D1, D4
  - Scenarios: S7
- [ ] AC4: `sce plan` appears in top-level help, `sce plan check --help` documents the required `<path>` argument, and generated bash/zsh/fish completion scripts include `plan check`.
  - Validate: `nix run .#sce -- --help`, `nix run .#sce -- plan check --help`, and `nix run .#sce -- completion --shell bash | grep -n "plan"`
  - Decisions: D2
  - Scenarios: S8
- [ ] AC5: The generated `/change-to-plan` workflow runs `sce plan check {plan-path}` after writing a structured plan and corrects reported defects before returning `plan_ready`. The generated `/validate` workflow runs `sce plan check {plan-path}` before scenario verification for structured plans and records each reported defect as a failed check. Both fall back to the manual `Reference rules` pass when the checker is unavailable, keep legacy-plan behavior unchanged, and leave `/next-task` unchanged.
  - Validate: `nix run .#pkl-check-generated`
  - Decisions: D5
  - Scenarios: S9
- [ ] AC6: The Pkl/Rust behavioral contract is documented: the shared rules, the shared manifest as the source of truth, the result and diagnostic wording, and the known AST-versus-line-scanner divergences.
  - Validate: inspection of `context/sce/plan-structure-contract.md`, and of `context/sce/shared-context-plan-workflow.md` linking to it.
  - Scenarios: none (inspection)
- [ ] AC7: The only new runtime dependency is `markdown`. No repository plan under `context/plans/` is read by generation or Nix checks, and `flake.nix` gains no `context/plans/` source dependency.
  - Validate: inspection of `git diff main -- cli/Cargo.toml flake.nix config/pkl/check-generated.sh`
  - Scenarios: none (inspection)

### Verification scenarios

- S1: Valid structured and legacy plans pass
  - Given: `valid-structured-plan.md`, `inspection-only-plan.md`, and `legacy-plan.md` fixtures.
  - When: `sce plan check` runs on each one, once by relative path and once by absolute path.
  - Then: Exit `0`. Stdout is exactly `structured plan: reference integrity holds` for the structured fixtures and `legacy plan: structured reference rules do not apply` for the legacy fixture. Stderr is empty.
  - Check: Rust command test `plan::command_valid_plans_exit_success`, created by T03, plus the AC1 `nix run .#sce` command.
- S2: Structurally invalid plan fails with validation class
  - Given: The `duplicate-id.md` fixture.
  - When: `sce plan check` runs on it.
  - Then: Exit `3`. Stderr contains `SCE-ERR-VALIDATION` and one `{path}:{line}:` diagnostic carrying the Pkl defect text for the duplicated `S1` ID ("is defined more than once"). Stdout is empty.
  - Check: Rust command test `plan::command_invalid_plan_exits_validation`, created by T03, plus the AC1 `nix run .#sce` command.
- S3: Missing file uses existing runtime error handling
  - Given: A path that does not exist.
  - When: `sce plan check` runs on it.
  - Then: Exit `4`. Stderr names the path and carries `SCE-ERR-RUNTIME`. No diagnostics are printed.
  - Check: Rust command test `plan::command_missing_file_is_runtime_failure`, created by T03.
- S4: The checker never executes plan commands
  - Given: A structured fixture whose `Check:`, `Validate:`, and `Verify:` values name a command that would create a marker file.
  - When: `sce plan check` runs on it.
  - Then: Exit `0`, and the marker file does not exist afterwards.
  - Check: Rust command test `plan::command_does_not_execute_plan_commands`, created by T03, plus named inspection that `cli/src/services/plan/` contains no `std::process` use.
- S5: Rust matches every manifest expectation
  - Given: `expectations.tsv` and every fixture it lists.
  - When: The Rust fixture test extracts and validates each fixture.
  - Then: Each fixture classified `structured` or `legacy` has no diagnostics and that classification. Each `fail` fixture produces a diagnostic containing every expected substring. Every `.md` fixture in the directory is listed in the manifest.
  - Check: Rust test `plan::fixture_expectations_match_manifest`, created by T02.
- S6: Pkl matches every manifest expectation
  - Given: The same `expectations.tsv`.
  - When: `config/pkl/check-generated.sh` iterates it through `plan-structure-check.pkl`.
  - Then: Every pass fixture prints its classification, every fail fixture fails with each expected substring, and the run aborts on any mismatch or unlisted fixture.
  - Check: `nix run .#pkl-check-generated`
- S7: Diagnostics point at the offending line
  - Given: The `misplaced-field.md`, `duplicate-id.md`, and `task-empty-covers.md` fixtures.
  - When: The Rust validator runs on each one.
  - Then: Each diagnostic's `line` equals the 1-based line of the offending field or definition in the fixture source. A plan-level missing-heading defect has no line.
  - Check: Rust test `plan::diagnostic_lines_match_source_positions`, created by T02.
- S8: Help and completion expose the command
  - Given: The built `sce` binary.
  - When: Top-level help, `plan check --help`, and `completion --shell {bash,zsh,fish}` run.
  - Then: Top-level help lists `plan`. Command help shows the required `<PATH>` argument. Each completion script contains `plan` and `check`.
  - Check: Existing help/completion tests updated by T03, plus the AC4 commands.
- S9: Generated workflows invoke the checker with fallback
  - Given: The canonical workflow Pkl sources after T04.
  - When: Generation and the contract checks run.
  - Then: Every target's generated `sce-change-to-plan/references/plan-authoring.md` and `sce-validate/references/validation.md` contain the `sce plan check` invocation, the D5 trust rule, the manual fallback, and the legacy skip. No generated `sce-next-task` document mentions `sce plan check`.
  - Check: `nix run .#pkl-check-generated`, with the extended `plan-authoring-structured-references` and `validate-scenario-evidence` assertions from T04.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/sce/plan-structure-contract.md` (new, the Pkl/Rust behavioral contract)
- `context/sce/shared-context-plan-workflow.md` (responsibility split: `sce plan check` for one selected plan, checker fallback)
- `context/sce/config-generation.md` (`check-generated.sh` drives plan-structure fixtures from `expectations.tsv`)
- `context/cli/cli-command-surface.md` and `context/overview.md` (new `plan` command and `services/plan` domain)
- `context/glossary.md` (`sce plan check`, plan-structure expectations manifest)
- `context/context-map.md` (link the new contract file)

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/Cargo.toml`/`Cargo.lock` (`markdown`), new `cli/src/services/plan/` (`mod.rs`, `extract.rs`, `validate.rs`, `command.rs`), `cli/src/services/mod.rs`, `cli/src/cli_schema.rs`, `cli/src/services/parse/command_runtime.rs`, `cli/src/services/command_registry.rs`, the help/completion tests they affect, `config/pkl/renderers/fixtures/plan-structure/` (manifest plus new fixtures), `config/pkl/check-generated.sh`, `config/pkl/base/workflow-change-to-plan.pkl`, `config/pkl/base/workflow-validate.pkl`, `config/pkl/renderers/generation-contract-check.pkl`, and `context/sce/plan-structure-contract.md`.
- **Out of scope:** `sce plan init`. Repository-wide plan scanning. Validating plans inside `nix flake check` or generation. Any new Nix source dependency on `context/plans/`. Executing planned tests or validation commands. Editing historical plans. YAML/JSON sidecar schemas. `/next-task` gates and task-completion semantics. Git hooks. Unrelated CLI services.
- **Constraints:** No `comrak`, `pulldown-cmark`, or custom Markdown parser, and no regex/line scanning for structure. The distributed binary never invokes Pkl. Static dispatch, with no `dyn`, boxed futures, or background tasks. Rust import grouping and `anyhow` context conventions apply. Stable diagnostic wording is shared with Pkl. All commands run through Nix.
- **Non-goal:** A general Markdown linter or auto-fixer. The checker reports defects and never rewrites the plan.

## Assumptions

- `markdown = "1"` is used as the request specifies. `Cargo.lock` pins the resolved version.
- The command lives under a new top-level `plan` command group shown in top-level help. It is visible rather than hidden because workflows and users invoke it directly.
- The success output reuses `plan-structure-check.pkl`'s single-plan strings exactly. There is no `--format json` in this change.
- The heading, ordering, required-field, ownership, reference, and coverage rules are exactly those in `config/pkl/renderers/plan-structure.pkl`'s `check` path. Template-skeleton mode (`templateDefects`) stays Pkl-only because it validates generated templates, not plans.
- New fixtures are added where the current set lacks a case the request names: a wrong reference kind (for example `Covers: S1`), and one fixture for the S4 no-execution check if an existing one cannot serve.
- Rust fixture tests read `config/pkl` at test runtime. That tree already reaches the Crane sandbox through `cliBuildInputFileset`, and crates.io packaging never compiles tests, so neither needs a flake change.

## Task stack

- [x] T01: `Extract SCE plan structure from the markdown-rs AST` (status:done)
  - Task ID: T01
  - Covers: AC2, AC7
  - Scope: In — add `markdown = "1"` to `cli/Cargo.toml`. Add `cli/src/services/plan/{mod.rs,extract.rs}` with the D1/D2 `Plan`/`PlanEntry`/`PlanField` model, extracted from `to_mdast(source, &ParseOptions::gfm())`. Covered: sections by heading depth and text; definitions only in authoritative sections; GFM checkbox; nested field items; source-sliced values; 1-based lines; `Code` nodes ignored. Add focused extraction unit tests on fenced-example and legacy-with-fenced fixtures. Out — validation rules, CLI wiring, manifest.
  - Dependencies: none
  - Done when: Extraction of `fenced-example-plan.md` yields only the out-of-fence definitions with correct lines. Extraction of `misplaced-definition.md` yields no `S` entry. No regex or line scanner identifies structure, and no other Markdown crate is added.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::extract`
  - Completed: 2026-10-24
  - Files changed: `cli/Cargo.toml`, `cli/Cargo.lock`, `cli/src/services/mod.rs`, `cli/src/services/plan/mod.rs`, `cli/src/services/plan/extract.rs`
  - Result: Added `markdown = "1"` (locked `markdown 1.0.0`, plus its transitive `unicode-id 0.3.7`). `services::plan::extract::extract(&str) -> Result<Plan>` parses with `to_mdast(.., &ParseOptions::gfm())`. It walks the root children, tracking the current heading (`PlanHeading { depth, text, line }`). It takes definitions only from `ListItem`s of root-level `List`s in authoritative sections whose first paragraph starts with an ID token and `:`. It gets fields from directly nested `List` items with a `Key:` prefix, slices values from source by node offsets, and records 1-based lines from `mdast` positions. `PlanEntry { id, kind: PlanEntryKind, checked, line, fields }` is stored in source order, with `Plan::entries_of(kind)`. Approved assumptions: `Plan` also records `outside_section_ids` (definition-shaped items outside authoritative positions) and `fenced_ids` (definition-shaped items found by re-parsing `Code` values with `to_mdast`, never line scanning) for T02's undefined-reference hints. `pub mod plan` carries `#[cfg_attr(not(test), allow(dead_code, ...))]` until T03 wires the command. Remove it there.
  - Verify outcome: `plan::extract` passed (6 tests: fenced-example entries/lines/fenced IDs, fenced headings skipped, source-sliced fields with lines and GFM checkbox, misplaced-definition yields no `S` entry, legacy-with-fenced example ignored, ID grammar). Additionally `clippy --all-targets` (pedantic, deny warnings) and `cargo fmt` are clean. No regex, line scanner, `comrak`, or `pulldown-cmark` appears under `cli/src/services/plan/`.
  - Context impact: localized — new internal `cli/src/services/plan/` domain (extraction model only, not yet reachable from the CLI) and a new `markdown` runtime dependency. No user-visible behavior change.
  - Context synchronization: synced

- [x] T02: `Validate plan structure in pure Rust against the shared fixture manifest` (status:done)
  - Task ID: T02
  - Covers: AC2, AC3
  - Scope: In — `cli/src/services/plan/validate.rs` with a pure `validate(&Plan) -> PlanReport` that ports the `plan-structure.pkl` `check` rules and defect wording: structured detection, required, duplicate, and ordered headings; `### Verification scenarios` parent; duplicate IDs; field ownership; required fields; ID-list grammar; reference kind and resolution with the outside-section/fenced hints; AC coverage; scenario referencing. Diagnostics are sorted deterministically. Add `config/pkl/renderers/fixtures/plan-structure/expectations.tsv` and the new wrong-reference-kind fixture. Switch `config/pkl/check-generated.sh` from hard-coded calls to iterating the manifest, and fail when a fixture is unlisted. Add `plan::fixture_expectations_match_manifest` and `plan::diagnostic_lines_match_source_positions`. Out — CLI wiring, workflow prose, template-skeleton mode.
  - Dependencies: T01
  - Done when: Both the Rust fixture test and `pkl-check-generated` pass from the same manifest, and every fixture's verdict and defect text agree. The diagnostic-line test asserts exact lines.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::`; `nix run .#pkl-check-generated`
  - Completed: 2026-10-24
  - Files changed: `cli/src/services/plan/validate.rs`, `cli/src/services/plan/mod.rs`, `cli/src/services/plan/extract.rs`, `config/pkl/renderers/fixtures/plan-structure/expectations.tsv`, `config/pkl/renderers/fixtures/plan-structure/wrong-reference-kind.md`, `config/pkl/check-generated.sh`
  - Result: `services::plan::validate::validate(&Plan) -> PlanReport { structured, diagnostics: Vec<PlanDiagnostic { line: Option<usize>, message }> }` ports the `plan-structure.pkl` `check` rules with verbatim defect wording: structured detection; required, duplicate, and ordered headings; the `### Verification scenarios` parent rule; duplicate IDs; field ownership; required fields; ID-list grammar; reference kind and resolution with the outside-section and fenced hints from `Plan.outside_section_ids` and `Plan.fenced_ids`; AC coverage; and scenario referencing. Diagnostics are sorted by `(line, message)`, with line-less diagnostics first. `PlanEntryKind::of_id` in `extract.rs` became `pub` so it can be reused. `expectations.tsv` (tab-separated: fixture, `structured`/`legacy`/`fail`, then expected substrings; `#` comment rows) lists all 17 fixtures, including the new `wrong-reference-kind.md` (`Covers: S1`). `check-generated.sh` iterates the manifest on fd 3. It rejects missing fixtures, unknown verdicts, pass rows that list substrings, fail rows without substrings, and any `*.md` fixture not listed exactly once. Approved assumptions: plan-level missing-heading defects carry no line. Duplicate-heading and duplicate-ID defects anchor at the first repeated occurrence. Misordered-heading defects anchor at the later heading. The parent-rule defect anchors at the `### Verification scenarios` heading. Deviation: `plan::fixture_expectations_match_manifest` and `plan::diagnostic_lines_match_source_positions` are module-level `#[test]` functions in `plan/mod.rs`, not in a `tests` submodule, so the plan's `plan::fixture` and `plan::diagnostic_lines` Cargo filters match them.
  - Verify outcome: `plan::` passed 8 tests (6 `extract`, plus `fixture_expectations_match_manifest` and `diagnostic_lines_match_source_positions`). `nix run .#pkl-check-generated` passed (145 files); the new files were marked intent-to-add with `git add -N` so the flake source includes them. A direct `nix develop -c ./config/pkl/check-generated.sh` also passed. A temporary unlisted fixture made it fail with `Plan structure fixture must be listed exactly once …`, and the fixture was removed afterwards. `clippy --all-targets -D warnings` is clean, and `cargo fmt` was applied.
  - Context impact: localized — the internal `cli/src/services/plan/` domain gains a validation layer that is not yet reachable from the CLI. The plan-structure fixture check in `check-generated.sh` is now driven by `expectations.tsv`, which changes the documented `config-generation.md` fixture-run description. No user-visible CLI behavior change.
  - Context synchronization: synced

- [ ] T03: `Wire sce plan check into the CLI schema and static dispatcher` (status:todo)
  - Task ID: T03
  - Covers: AC1, AC4
  - Scope: In — `cli_schema.rs`: `PLAN_*` constants, a `Commands::Plan { subcommand: PlanSubcommand::Check { path: PathBuf } }` variant, and a `TOP_LEVEL_COMMANDS` entry. Conversion in `services/parse/command_runtime.rs`, a `RuntimeCommand::Plan` variant in `services/command_registry.rs`, and `services/plan/command.rs`, which reads the file with path context (runtime failure) and maps defects to `CliError::validation` with D4 rendering. Update the affected help/completion tests, and add the S1–S4 command tests. Out — workflow prose, any `plan init`.
  - Dependencies: T02
  - Done when: S1–S4 tests pass. `sce plan check` on a valid fixture exits `0` and on an invalid one exits `3` with `{path}:{line}:` diagnostics. Help and completion list `plan check`.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml completion`; `nix run .#sce -- plan check config/pkl/renderers/fixtures/plan-structure/duplicate-id.md; echo $?`
  - Context synchronization: pending

- [ ] T04: `Invoke sce plan check from the change-to-plan and validate workflows` (status:todo)
  - Task ID: T04
  - Covers: AC5
  - Scope: In — `config/pkl/base/workflow-change-to-plan.pkl` steps 2.9–2.10: after writing a structured plan, run `sce plan check {plan-path}`, fix the reported defects, and re-run before `plan_ready`. `config/pkl/base/workflow-validate.pkl` step 4: run the checker before scenario verification and record each diagnostic as a failed check. Both get the D5 trust rule, the manual `Reference rules` fallback, and the legacy skip. Extend the existing `plan-authoring-structured-references` and `validate-scenario-evidence` assertions in `generation-contract-check.pkl`, and the negative fixtures they already use, so the invocation, fallback, and `/next-task` exclusion are required. Out — `/next-task` sources, Git hooks, new assertion families.
  - Dependencies: T03
  - Done when: Generated plan-authoring and validation references on every target carry the invocation, trust rule, fallback, and legacy skip, and `pkl-check-generated` passes.
  - Verify: `nix run .#pkl-check-generated`
  - Context synchronization: pending

- [ ] T05: `Document the Pkl/Rust plan-structure behavioral contract` (status:todo)
  - Task ID: T05
  - Covers: AC6
  - Scope: In — new `context/sce/plan-structure-contract.md` covering: the shared rule set; `expectations.tsv` as the single source of truth and how to add a fixture; the D4 result and diagnostic wording; the rule that changing defect text requires both checkers; the known AST-versus-line-scanner divergences (list-nesting indentation tolerance, `[X]` checkboxes, definitions nested inside other list items, multi-line reference values); and that Pkl stays generation-only while the CLI never invokes Pkl. Link it from `context/sce/shared-context-plan-workflow.md` and `context/context-map.md`. Out — code changes.
  - Dependencies: T04
  - Done when: The contract file exists with those sections and is linked from both files.
  - Verify: inspection of `context/sce/plan-structure-contract.md` and its links.
  - Context synchronization: pending

## Open questions

- Should a known AST-versus-line-scanner divergence (D1 trade-off) be fixed in the Pkl checker so the two agree beyond the fixtures, or only documented? The plan documents divergences and pins agreement through fixtures. Tightening `plan-structure.pkl` would be a follow-up.
