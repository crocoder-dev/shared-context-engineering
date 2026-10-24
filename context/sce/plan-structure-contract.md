# Plan Structure Contract (Pkl and Rust checkers)

Two implementations check the structure and reference integrity of an SCE plan:

- **Pkl checker** — `config/pkl/renderers/plan-structure.pkl` (`check` path), run through `config/pkl/renderers/plan-structure-check.pkl`. It is the generation-time reference implementation. It runs only inside generation checks (`nix run .#pkl-check-generated`, also part of `nix flake check`), and only against fixtures.
- **Rust checker** — `cli/src/services/plan/` (`extract` then `validate`), shipped as `sce plan check <path>` (see [plan service](../cli/plan-service.md)). It is the deterministic checker for one selected plan. The distributed `sce` binary and its tests never invoke Pkl, Nix, an LLM, or the network.

This contract defines what the two must agree on, how that agreement is enforced, and where they are known to differ.

## Shared rule set

Both checkers apply the same rules to the plan source outside fenced code blocks:

- **Classification.** A plan is structured when it has a `### Verification scenarios` heading or any task under `## Task stack` carries `Covers:`. Every other plan is legacy and gets no defects.
- **Headings.**
  - `## Acceptance criteria` and `## Task stack` are required.
  - `## Design decisions`, `## Acceptance criteria`, `### Verification scenarios`, and `## Task stack` appear at most once each, in that order.
  - `### Verification scenarios` sits inside `## Acceptance criteria`.
- **Definitions.**
  - `D`, `AC`, `S`, and `T` entries count only in their authoritative section: `## Design decisions`, `## Acceptance criteria`, `### Verification scenarios`, and `## Task stack`, each ending at the next heading.
  - IDs are unique.
  - A top-level list item that defines an ID under another authoritative section is reported at its own line as defined under the wrong section. Nested items, prose outside authoritative sections, and fenced examples are not.
  - Owned fields appear only on their owning kind. The `D` fields are `Boundary`, `Rationale`, `Rejected`, and `Trade-offs`. The `AC` fields are `Validate`, `Decisions`, and `Scenarios`. The `S` fields are `Given`, `When`, `Then`, and `Check`. The `T` field is `Covers`.
  - Each definition carries its kind's required fields.
- **References.**
  - `Decisions:`, `Scenarios:`, and `Covers:` hold an exact `ID, ID` list. `Scenarios: none (inspection)` is exempt.
  - Each listed ID names the right kind and resolves to a definition.
  - An undefined ID that appears only outside its section, or only inside a fenced block, gets a hint saying so.
- **Coverage.** Every `AC` is covered by some task's `Covers:`, and every `S` is referenced by some `AC`'s `Scenarios:`.

Template-skeleton mode (`templateDefects` in `plan-structure.pkl`) validates generated plan templates, not plans. It stays Pkl-only and is outside this contract.

## Shared fixtures and manifest

The single source of truth is `config/pkl/renderers/fixtures/plan-structure/expectations.tsv`. It covers the `*.md` fixtures in the same directory.

- Each row is tab-separated: the fixture file name, then its verdict (`structured`, `legacy`, or `fail`). `fail` rows then list the complete expected defect messages, one per column. Comparison is exact on the sorted multiset of messages: missing, unexpected, or miscounted duplicate defects fail. Source lines are not part of the manifest; Rust tests assert them separately. Rows starting with `#` are comments.
- `config/pkl/check-generated.sh` iterates the manifest. `structured` and `legacy` rows must print their classification line through `plan-structure-check.pkl`, and `fail` rows must make it fail. Every row is then compared against the structured report from `config/pkl/renderers/fixtures/plan-structure-report.pkl`: classification on the first line, then one defect per line. The run aborts on any of these: a missing fixture, an unknown verdict, a pass row with diagnostics, a fail row without diagnostics, a classification or diagnostic-set mismatch, or a `*.md` fixture not listed exactly once.
- The Rust test `plan::fixture_expectations_match_manifest` reads the same manifest and fixtures at test runtime through `CARGO_MANIFEST_DIR/../config/pkl/renderers/fixtures/plan-structure`. It requires the same verdicts and the same exact sorted message multiset, and fails on any unlisted fixture.
- To add a fixture:
  1. Add the `.md` file to the directory.
  2. Add its manifest row.
  3. Run both `nix run .#pkl-check-generated` and `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::fixture`.

  A fixture passes only when both checkers agree with its row. Never keep a Rust-only or Pkl-only expectation table.
- Generation and Nix checks read only these fixtures. They never read `context/plans/`.

## Result and diagnostic wording

- Valid structured plan: `structured plan: reference integrity holds`.
- Valid legacy plan: `legacy plan: structured reference rules do not apply`.
- Defect messages are the Pkl defect strings, used verbatim by the Rust validator. Examples are ``heading `## Task stack` is missing``, `` `S1` is defined more than once``, and `` `Covers:` on `T01` names `S1`, which is not an acceptance criterion ``.
- `sce plan check` exit and output behavior:
  - It exits `0` and prints the classification line on stdout.
  - For a structurally invalid plan, it exits `3` (`SCE-ERR-VALIDATION`). Stderr lists one diagnostic per line as `{path}:{line}: {message}`, or `{path}: {message}` for a plan-level defect with no line. Diagnostics are ordered by line, then message.
  - It exits `4` (`SCE-ERR-RUNTIME`) for a missing or unreadable file.
- The Pkl checker throws one joined `plan failed the structural reference-integrity check: …` message. Its result strings match the Rust stdout lines exactly.
- Changing a classification line or a defect message is a change to both checkers. Update `plan-structure.pkl`, `cli/src/services/plan/validate.rs` or `command.rs`, any affected manifest substrings, and the workflow trust rule in [plan workflow](shared-context-plan-workflow.md). The trust rule matches the two classification lines.

## Known divergences

The Rust extractor follows the CommonMark/GFM `mdast` tree from `markdown-rs`. The Pkl checker scans lines with regexes. They can disagree on inputs outside the fixture set:

- **List-nesting indentation.** Pkl takes a field only at exactly `entry indent + 2` spaces. Rust takes any item of a list nested directly under the definition, so other indentation that CommonMark still nests is accepted.
- **Checkbox case.** Pkl matches only `[ ]` and `[x]`. With `[X]`, the line is not a definition for Pkl, while GFM accepts `[X]` as checked.
- **Nested definitions.** Pkl accepts a definition line at any indentation inside its section, including one nested in another list item. Rust takes definitions only from items of root-level lists.
- **Multi-line values.** Pkl reads a field value from its first line only. Rust slices the field item's whole first paragraph from source, so a wrapped `Covers:`, `Decisions:`, or `Scenarios:` list keeps its continuation lines. A wrapped list then fails the ID-list grammar in Rust, but may pass in Pkl.
- **List markers.** Pkl accepts only `-` bullets. Rust accepts any GFM list (`-`, `*`, `+`, or ordered).
- **Indented code blocks.** Pkl recognizes only backtick and tilde fences. Rust also ignores indented (four-space) code blocks.

The fixtures pin the shared behavior. Do not author plans that depend on these divergences. Aligning them, for example by tightening `plan-structure.pkl`, is a possible follow-up, not part of this contract.
