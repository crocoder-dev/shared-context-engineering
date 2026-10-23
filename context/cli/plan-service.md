# Plan Service

`cli/src/services/plan/` owns deterministic, read-only structural analysis of one SCE plan Markdown file. It is the Rust counterpart of the test-only Pkl checker `config/pkl/renderers/plan-structure.pkl`.

## Current scope

- `extract.rs` is the only plan-service code that touches `markdown::mdast::Node`. It parses the source with `markdown-rs` (`markdown` crate, `to_mdast(source, &ParseOptions::gfm())`) and returns an owned `Plan` model. It never scans lines to find structure.
- `validate.rs` holds the pure `validate(&Plan) -> PlanReport` rules. It has no I/O and never touches `mdast` types.
- The `sce plan check` CLI command is not implemented yet (plan `context/plans/sce-plan-check.md`, T03). The module is not reachable from the CLI.

## Plan model

- `Plan.headings`: one `PlanHeading { depth, text, line }` per root-level heading, where `text` is the heading's plain text.
- `Plan.entries`: one `PlanEntry { id, kind, checked, line, fields }` per definition, in source order. Use `Plan::entries_of(kind)` to select one kind. `PlanEntryKind` is one of `Decision`, `AcceptanceCriterion`, `Scenario`, or `Task`.
- `PlanField { key, value, line }`: one per directly nested list item whose first paragraph starts with `Key:`. The key is `[A-Z][A-Za-z -]*`, followed by whitespace or the end of the text. `value` is the trimmed source slice after the colon, so inline code and punctuation are kept verbatim.
- `Plan.outside_section_ids` and `Plan.fenced_ids` hold the IDs of definition-shaped list items that are not definitions. The first lists items found outside an authoritative position. The second lists items found inside `Code` blocks, which are re-parsed with `to_mdast` rather than scanned by line. They support undefined-reference hints and never create entries.
- Lines are 1-based and come from `mdast` positions.

## Extraction rules

- A section is the run of root children after a heading, up to the next heading of any depth. The authoritative sections are `## Design decisions` (D), `## Acceptance criteria` (AC), `### Verification scenarios` (S), and `## Task stack` (T).
- A definition is a `ListItem` of a root-level `List` in the section that matches its kind. Its first paragraph must start with an ID token (`D<n>`, `AC<n>`, `S<n>` with `n` ≥ 1 and no leading zero, or `T` plus two or more digits), then `:`, then whitespace or the end of the text. The GFM `checked` value gives `checked`.
- A definition-shaped item of the wrong kind, in a non-authoritative section, or nested inside another list item is not an entry.
- `Code` nodes, both fenced and indented, never contribute headings, entries, or fields.

## Validation rules

`validate` mirrors the `check` path of `config/pkl/renderers/plan-structure.pkl`. Template-skeleton mode stays Pkl-only. Defect messages are copied verbatim from the Pkl checker, so changing their wording means changing both checkers.

- A plan is structured when it has a `### Verification scenarios` heading or any task carries `Covers:`. A legacy plan gets `structured: false` and no diagnostics.
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
  - An unresolved ID gets an `outside_section_ids` or `fenced_ids` hint.
  - Every AC must be covered by some task's `Covers:`, and every S must be referenced by some AC's `Scenarios:`.
- `PlanReport.diagnostics` holds `PlanDiagnostic { line, message }` values, sorted by `(line, message)`. Line-less diagnostics come first, and duplicates are kept.
- Where a diagnostic's line comes from:
  - Missing required headings have no line.
  - Duplicate headings and duplicate IDs point at the first repeated occurrence.
  - A misordered heading points at the later heading.
  - The parent-rule defect points at `### Verification scenarios`.
  - Misplaced and reference defects point at their field.
  - Missing-field, uncovered-AC, and unreferenced-S defects point at their definition.

## Terms

- `plan-structure expectations manifest`: `config/pkl/renderers/fixtures/plan-structure/expectations.tsv`, the single source of fixture verdicts for both the Pkl checker (driven by `config/pkl/check-generated.sh`) and the Rust validator. Each row is tab-separated: fixture file, `structured`, `legacy`, or `fail`, then the expected defect substrings for `fail` rows. Every `*.md` fixture in the directory must be listed exactly once.
- `plan diagnostic`: One `PlanDiagnostic` from `validate`, with Pkl defect wording and an optional 1-based source line.

## Divergences from the Pkl line scanner

The extractor follows CommonMark/GFM nesting. The Pkl checker requires fields at exactly `entry indent + 2` and accepts definitions at any indentation. Indentation-only nesting differences, `[X]` checkboxes, and multi-line field values can therefore extract differently. Agreement is pinned by the shared fixtures under `config/pkl/renderers/fixtures/plan-structure/`.

## Tests

`cli/src/services/plan/extract.rs` unit tests (`plan::extract`) read the shared plan-structure fixtures through `CARGO_MANIFEST_DIR/../config/pkl/renderers/fixtures/plan-structure`. They cover fenced examples, misplaced definitions, legacy plans with fenced structured examples, source-sliced field values with lines, and the ID grammar.

Module-level tests in `cli/src/services/plan/mod.rs` make the shared contract mechanical. `plan::fixture_expectations_match_manifest` runs every manifest row through `extract` and `validate` and requires that every fixture is listed. `plan::diagnostic_lines_match_source_positions` asserts exact diagnostic lines, and checks that a missing heading has no line.
