# Plan Service

`cli/src/services/plan/` owns deterministic, read-only structural analysis of one SCE plan Markdown file. It is the Rust counterpart of the test-only Pkl checker `config/pkl/renderers/plan-structure.pkl`.

## Current scope

- `extract.rs` is the only plan-service code that touches `markdown::mdast::Node`. It parses the source with `markdown-rs` (`markdown` crate, `to_mdast(source, &ParseOptions::gfm())`) and returns an owned `Plan` model. It never scans lines to find structure.
- `validate.rs` holds the pure `validate(&Plan) -> PlanReport` rules. It has no I/O and never touches `mdast` types.
- `command.rs` owns the `sce plan check <path>` command (see [Command contract](#command-contract)).
- `mod.rs` embeds the generated plan init scaffold as the crate-visible `SCAFFOLD` constant (`include_str!` of `OUT_DIR/pkl-generated/config/plan-init-scaffold.md`) with its `SCAFFOLD_PLAN_NAME_TOKEN` (`{plan-name}`). `cli/build.rs` requires that staged path, so a payload or package fallback without the scaffold fails the build. The binary never renders the scaffold itself.

## Plan model

- `Plan.headings`: one `PlanHeading { depth, text, line }` per root-level heading, where `text` is the heading's plain text.
- `Plan.entries`: one `PlanEntry { id, kind, checked, line, fields }` per definition, in source order. Use `Plan::entries_of(kind)` to select one kind. `PlanEntryKind` is one of `Decision`, `AcceptanceCriterion`, `Scenario`, or `Task`.
- `PlanField { key, value, line }`: one per directly nested list item whose first paragraph starts with `Key:`. The key is `[A-Z][A-Za-z -]*`, followed by whitespace or the end of the text. `value` is the trimmed source slice after the colon, so inline code and punctuation are kept verbatim.
- `Plan.outside_definitions` and `Plan.fenced_ids` hold definition-shaped list items that are not definitions. The first holds `OutsideDefinition { id, line, section }` for items found outside an authoritative position. `section` is set only for a top-level list item under an authoritative section of another kind, which marks a definition in the wrong section rather than nested or unrelated prose. The second lists items found inside `Code` blocks, which are re-parsed with `to_mdast` rather than scanned by line. They support undefined-reference hints and never create entries.
- Lines are 1-based and come from `mdast` positions.

## Extraction rules

- A section is the run of root children after a heading, up to the next heading of any depth. The authoritative sections are `## Design decisions` (D), `## Acceptance criteria` (AC), `### Verification scenarios` (S), and `## Task stack` (T).
- A definition is a `ListItem` of a root-level `List` in the section that matches its kind. Its first paragraph must start with an ID token (`D<n>`, `AC<n>`, `S<n>` with `n` ≥ 1 and no leading zero, or `T` plus two or more digits), then `:`, then whitespace or the end of the text. The GFM `checked` value gives `checked`.
- A definition-shaped item of the wrong kind, in a non-authoritative section, or nested inside another list item is not an entry.
- `Code` nodes, both fenced and indented, never contribute headings, entries, or fields.

## Validation rules

`validate` mirrors the `check` path of `config/pkl/renderers/plan-structure.pkl`. Template-skeleton mode stays Pkl-only. Defect messages are copied verbatim from the Pkl checker, so changing their wording means changing both checkers.

- Every plan is checked as a structured plan. There is no legacy classification: a plan in the pre-structured format fails with its missing `Scenarios:`, `Covers:`, and coverage diagnostics.
- Heading rules, with headings compared as `#`×depth plus text:
  - `## Acceptance criteria` and `## Task stack` are required.
  - `## Design decisions`, `## Acceptance criteria`, `### Verification scenarios`, and `## Task stack` may each appear only once, in that order.
  - The nearest preceding depth-1 or depth-2 heading of `### Verification scenarios` must be `## Acceptance criteria`.
- Definition rules:
  - IDs are unique.
  - Owned fields appear only on their owning kind: `Boundary`/`Rationale`/`Rejected`/`Trade-offs` → D, `Validate`/`Decisions`/`Scenarios` → AC, `Given`/`When`/`Then`/`Check` → S, `Covers` → T.
  - Every definition carries its kind's required fields.
- Reference rules:
  - `Decisions:`, `Scenarios:`, and `Covers:` on their owning kind must be an exact `ID, ID` list. `Scenarios: none (inspection)` is exempt.
  - Each listed ID must name the right kind and resolve to a definition.
  - An unresolved ID gets an `outside_definitions` or `fenced_ids` hint.
  - An `OutsideDefinition` with a `section` is reported at its own line as defined under the wrong section.
  - Every AC must be covered by some task's `Covers:`, and every S must be referenced by some AC's `Scenarios:`.
- `PlanReport.diagnostics` holds `PlanDiagnostic { line, message }` values, sorted by `(line, message)`. Line-less diagnostics come first, and duplicates are kept.
- Where a diagnostic's line comes from:
  - Missing required headings have no line.
  - Duplicate headings and duplicate IDs point at the first repeated occurrence.
  - A misordered heading points at the later heading.
  - The parent-rule defect points at `### Verification scenarios`.
  - Misplaced and reference defects point at their field.
  - Missing-field, uncovered-AC, and unreferenced-S defects point at their definition.

## Command contract

`sce plan check <path>` checks exactly the file at `<path>`, relative or absolute. It is synchronous, uses no database or repository root, and never runs `Check:`, `Validate:`, or `Verify:` content.

- Valid structured plan: stdout `structured plan: reference integrity holds`, exit `0`.
- Structurally invalid plan: `CliError::validation` (exit `3`, `SCE-ERR-VALIDATION`). The stderr message is a `Plan structure check failed for '{path}':` header, one `{path}:{line}: {message}` line per diagnostic (`{path}: {message}` when the diagnostic has no line) in `validate` order, and a closing `Try:` line. `{path}` is printed as given.
- Missing or unreadable file: `CliError::runtime` (exit `4`, `SCE-ERR-RUNTIME`) with `Failed to read plan file '{path}'` context, and no diagnostics.

`plan` is a visible top-level command and appears in `sce --help` (including a `Plan Usage:` line) and in generated Bash/Zsh/Fish completions.

## Terms

- `plan-structure expectations manifest`: `config/pkl/renderers/fixtures/plan-structure/expectations.tsv`, the single source of fixture verdicts for both the Pkl checker (driven by `config/pkl/check-generated.sh`) and the Rust validator. Each row is tab-separated: fixture file, `pass` or `fail`, then the complete expected defect messages for `fail` rows, compared as an exact order-insensitive multiset. Every `*.md` fixture in the directory must be listed exactly once.
- `sce plan check`: The read-only CLI command that runs `extract` then `validate` on one plan file and reports through the exit `0`/`3`/`4` contract above.
- `plan diagnostic`: One `PlanDiagnostic` from `validate`, with Pkl defect wording and an optional 1-based source line.
- `plan init scaffold`: The generated `config/plan-init-scaffold.md` plan skeleton rendered by `config/pkl/base/plan-init-scaffold.pkl` and embedded as `SCAFFOLD`. Untouched, it fails `sce plan check` with exactly its `Scenarios: TODO` and `Covers: TODO` reference defects (see the [plan structure contract](../sce/plan-structure-contract.md)).

## Divergences from the Pkl line scanner

The extractor follows CommonMark/GFM nesting. The Pkl checker requires fields at exactly `entry indent + 2` and accepts definitions at any indentation. Indentation-only nesting differences, `[X]` checkboxes, and multi-line field values can therefore extract differently. Agreement is pinned by the shared fixtures under `config/pkl/renderers/fixtures/plan-structure/`. The full divergence list and the shared rules live in the [plan structure contract](../sce/plan-structure-contract.md).

## Tests

`cli/src/services/plan/extract.rs` unit tests (`plan::extract`) read the shared plan-structure fixtures through `CARGO_MANIFEST_DIR/../config/pkl/renderers/fixtures/plan-structure`. They cover fenced examples, misplaced definitions, unstructured plans with fenced structured examples, source-sliced field values with lines, and the ID grammar.

Module-level tests in `cli/src/services/plan/mod.rs` make the shared contract mechanical. `plan::fixture_expectations_match_manifest` runs every manifest row through `extract` and `validate` and requires that every fixture is listed. `plan::diagnostic_lines_match_source_positions` asserts exact diagnostic lines, and checks that a missing heading has no line. The `plan::command_*` tests drive `sce plan check` through the parser and stdout/stderr renderer for valid plans (by relative and absolute path), an invalid plan, an unstructured plan, a missing file, and a plan whose commands would create a marker file. `plan::plan_check_appears_in_help` and `plan::completion_scripts_include_plan_check` cover help and completion. `plan::scaffold_untouched_fails_check` asserts the embedded scaffold has one `{plan-name}` token, defines no decision or scenario entries, and validates to exactly the three untouched-scaffold diagnostics.
