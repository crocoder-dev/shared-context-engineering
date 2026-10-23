# Plan Service

`cli/src/services/plan/` owns deterministic, read-only structural analysis of one SCE plan Markdown file. It is the Rust counterpart of the test-only Pkl checker `config/pkl/renderers/plan-structure.pkl`.

## Current scope

- `extract.rs` is the only plan-service code that touches `markdown::mdast::Node`. It parses the source with `markdown-rs` (`markdown` crate, `to_mdast(source, &ParseOptions::gfm())`) and returns an owned `Plan` model. It never scans lines to find structure.
- Validation rules, the shared fixture manifest, and the `sce plan check` CLI command are not implemented yet (plan `context/plans/sce-plan-check.md`, T02–T03). The module is not reachable from the CLI.

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

## Divergences from the Pkl line scanner

The extractor follows CommonMark/GFM nesting. The Pkl checker requires fields at exactly `entry indent + 2` and accepts definitions at any indentation. Indentation-only nesting differences, `[X]` checkboxes, and multi-line field values can therefore extract differently. Agreement is pinned by the shared fixtures under `config/pkl/renderers/fixtures/plan-structure/`.

## Tests

`cli/src/services/plan/extract.rs` unit tests (`plan::extract`) read the shared plan-structure fixtures through `CARGO_MANIFEST_DIR/../config/pkl/renderers/fixtures/plan-structure`. They cover fenced examples, misplaced definitions, legacy plans with fenced structured examples, source-sliced field values with lines, and the ID grammar.
