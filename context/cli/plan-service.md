# Plan Service

`cli/src/services/plan/` owns deterministic, read-only structural analysis of one SCE plan Markdown file, the `sce plan pr-description` intent review brief, and the `sce plan init` scaffold generator, its only writing operation. The analysis is the Rust counterpart of the test-only Pkl checker `config/pkl/renderers/plan-structure.pkl`.

## Current scope

- `extract.rs` is the only plan-service code that touches `markdown::mdast::Node`. It parses the source with `markdown-rs` (`markdown` crate, `to_mdast(source, &ParseOptions::gfm())`) and returns an owned `Plan` model. It never scans lines to find structure.
- `validate.rs` holds the pure `validate(&Plan) -> PlanReport` rules. It has no I/O and never touches `mdast` types.
- `command.rs` owns the `plan` command. `PlanCommand` holds one `PlanRequest::{Check(PlanCheckRequest), Init(PlanInitRequest), PrDescription(PlanPrDescriptionRequest)}`, mapped from `PlanSubcommand` in `services/parse/command_runtime.rs` and dispatched by a static `match`. `execute_with_stderr(stderr)`, called from `RuntimeCommand::execute_with_stderr`, passes `std::env::current_dir` and the command stderr writer to `execute_with`, which tests call with an explicit start directory and a captured stderr buffer instead of changing process state (see [Command contract](#command-contract), [Init command contract](#init-command-contract), and [PR description command contract](#pr-description-command-contract)). `check` and `pr-description` share one load path: read, `extract`, `validate`, and the same diagnostic rendering.
- `init.rs` owns `PlanName` validation and `create(&PlanName, start)`, the repository-root scaffold publication. Its `resolve_repository_root(start, command)` is shared with `pr-description` and names the calling command in its remediation text.
- `pr_description.rs` is a pure module with no I/O. It owns the argument grammar (`parse_arguments`), the collection of marked decisions, the question limit, Markdown escaping (`escape_markdown`), and `render(&PrDescriptionArguments, &[PlanInput])`.
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
  - Owned fields appear only on their owning kind: `Boundary`/`Rationale`/`Rejected`/`Trade-offs`/`Review`/`Review question` → D, `Validate`/`Decisions`/`Scenarios` → AC, `Given`/`When`/`Then`/`Check` → S, `Covers` → T.
  - Every definition carries its kind's required fields.
- Review pair rules, per decision:
  - `Review:` and `Review question:` are optional, but each requires the other.
  - Every `Review:` value must be `required`.
  - Every `Review question:` value must be nonempty and must not contain a newline. The source-sliced value keeps lazy continuation lines, so a wrapped question fails.
  - A repeated `Review:` or `Review question:` key is reported once.
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
  - A missing review-pair half points at its decision. A review value, empty, or single-line defect points at its field. A repeated review key points at its second occurrence.

## Command contract

`sce plan check <path>` checks exactly the file at `<path>`, relative or absolute. It is synchronous, uses no database or repository root, and never runs `Check:`, `Validate:`, or `Verify:` content.

- Valid structured plan: stdout `structured plan: reference integrity holds`, exit `0`.
- Structurally invalid plan: `CliError::validation` (exit `3`, `SCE-ERR-VALIDATION`). The stderr message is a `Plan structure check failed for '{path}':` header, one `{path}:{line}: {message}` line per diagnostic (`{path}: {message}` when the diagnostic has no line) in `validate` order, and a closing `Try:` line. `{path}` is printed as given.
- Missing or unreadable file: `CliError::runtime` (exit `4`, `SCE-ERR-RUNTIME`) with `Failed to read plan file '{path}'` context, and no diagnostics.

## Init command contract

`sce plan init <name>` creates `context/plans/<name>.md` at the Git repository root from the embedded scaffold, with its single `{plan-name}` token replaced by `<name>`. It is synchronous, offline, uses no database, and invokes only `git`.

- Name: one required positional `String`. `PlanName::parse` runs before any filesystem access, including reading the current directory: the value must match `[a-z0-9]+(-[a-z0-9]+)*`, be at most 64 bytes (`PLAN_NAME_MAX_BYTES`), and not end in `.md`. Clap errors keep the existing `classify_clap_error` mapping: hyphen-prefixed values, unknown options, and extra positionals are parse errors (exit `2`); a missing `<NAME>` is exit `3`. A value after `--` reaches name validation.
- Root: `git rev-parse --show-toplevel` run with the start directory as its working directory. A root that is not UTF-8 or contains a newline is refused before any write.
- Directories: `context` and `context/plans` must be real directories (`symlink_metadata`, so symlinks are refused) and are created when absent. Symlinked directories are refused even when a repository links `context/` elsewhere on purpose, because following the link would allow writes outside the repository root. Directories created by a failed run are not rolled back.
- Publication: the content is written to a hidden `.<name>.md.<pid>.tmp` file opened with `create_new` inside `context/plans/`, then published with `std::fs::hard_link`. A successful hard link is the commit point: from then on the run succeeds. The temporary file is removed after publication and on every handled error after its creation. `init::create` delegates to `init::create_with`, which takes the link and remove operations as generic closures so tests inject failures deterministically. The destination either does not exist or holds the complete scaffold; an existing entry, including a dangling symlink, is never replaced. There is no non-atomic fallback, no `fsync`, and no crash-recovery cleanup, so a killed run can leave an orphaned temporary file.
- Success: exit `0` and exactly three stdout lines: `Created plan: context/plans/<name>.md`, `Next: complete the plan and run`, and `sce plan check '<root>/context/plans/<name>.md'`. The path is always wrapped in POSIX single quotes, each `'` written as `'\''`, so the printed command works from any directory.
- Cleanup warning: when the hard link succeeds but removing the temporary file fails, the run still exits `0` with the same three stdout lines, and writes one stderr line before them: `Warning: Created plan context/plans/<name>.md, but failed to remove temporary file '<temporary-path>': <error>. Try: remove '<temporary-path>' manually; the plan was created and does not need to be initialized again.` The published destination is never removed.
- Failures: an invalid name or an existing destination (`Plan already exists: context/plans/<name>.md`) is `CliError::validation` (exit `3`). A missing repository, a failed or missing `git`, an unprintable root, a non-directory `context` path, and any other filesystem failure are `CliError::runtime` (exit `4`). When writing or publishing fails, the temporary file is removed and the original error and class are returned. When that removal also fails, the class is kept and the rendered message is the original error followed by a line `Cleanup also failed: could not remove temporary file '<temporary-path>': <error>. Try: remove it manually.`

## PR description command contract

`sce plan pr-description --repo <owner/name> --head <sha> <path>...` prints an intent review brief for one or more plans. It is synchronous and offline. It uses no database, no network, no AI, and no heuristics, and it never writes files.

- Arguments are checked before any file access. `--repo` is `<owner>/<name>`, with owner `[A-Za-z0-9-]+` and name `[A-Za-z0-9._-]+` that is not `.` or `..`. `--head` is exactly 40 lowercase hexadecimal characters. Each path is `context/plans/<file>.md`, with `<file>` in `[A-Za-z0-9._-]+` and not `.` or `..`. A path given twice is rejected. Because of this grammar, URLs need no percent-encoding.
- Plans are read from the working tree at `{root}/{path}`, where the root comes from `git rev-parse --show-toplevel`. The command never reads Git objects and never checks that the working tree matches `--head`. The caller must pass the SHA that contains the plan files as read.
- Every plan must pass the same `extract` and `validate` checks as `sce plan check`. A failing plan reports the same diagnostic lines, labeled with its relative path.
- A marked decision carries `Review: required` and its `Review question:`. More than three marked decisions across all plans is an error, never a truncation. The error states the count and lists each one as `<label> <decision-id>`.
- Output: the `<!-- sce:intent-review:start -->` marker, `## Intent review`, and a `**Plans:**` line with one link per plan in sorted path order, joined by `, `. Then `**Decisions requiring review**` and numbered `{n}. {question} — [{label} {D-id}]({url}#L{line})` items, separated by blank lines, in sorted plan order and source decision order. With no marked decision, the line `No decisions explicitly flagged for review.` replaces the list. A blank line and the `<!-- sce:intent-review:end -->` marker close the brief, and stdout ends with one newline.
- A plan label is the file name without `.md`. A link is `https://github.com/{repo}/blob/{sha}/{path}`, and a decision link adds `#L{line}` for the decision entry line.
- Escaping: the renderer puts one backslash before each of `` \ ` * _ [ ] < > ! | ~ & `` in questions and labels. URLs are never escaped.
- Failures: invalid arguments, a duplicate path, an invalid plan, and more than three marked decisions are `CliError::validation` (exit `3`). A missing or unreadable plan, a failed `git`, or a start directory outside a repository is `CliError::runtime` (exit `4`). Stdout is empty on every failure.

`plan` is a visible top-level command and appears in `sce --help` (with `sce plan check <path>`, `sce plan init <name>`, and `sce plan pr-description --repo <owner/name> --head <sha> <path>...` lines under `Plan Usage:`) and in generated Bash/Zsh/Fish completions.

## Terms

- `plan-structure expectations manifest`: `config/pkl/renderers/fixtures/plan-structure/expectations.tsv`, the single source of fixture verdicts for both the Pkl checker (driven by `config/pkl/check-generated.sh`) and the Rust validator. Each row is tab-separated: fixture file, `pass` or `fail`, then the complete expected defect messages for `fail` rows, compared as an exact order-insensitive multiset. Every `*.md` fixture in the directory must be listed exactly once.
- `sce plan init`: The CLI command that publishes the plan init scaffold as `context/plans/<name>.md` at the repository root through the [init command contract](#init-command-contract). It never overwrites an existing plan.
- `sce plan check`: The read-only CLI command that runs `extract` then `validate` on one plan file and reports through the exit `0`/`3`/`4` contract above.
- `sce plan pr-description`: The offline CLI command that validates plans and prints their intent review brief through the [PR description command contract](#pr-description-command-contract).
- `intent review brief`: The marker-delimited Markdown block that `sce plan pr-description` prints for a pull request. It lists the input plans and their review questions with links to exact decision lines.
- `review pair`: The optional `Review: required` and one-line `Review question:` fields on a design decision. They mark a decision that needs senior engineering judgment.
- `review question`: The `Review question:` value of a marked decision. The intent review brief shows it verbatim, with Markdown punctuation escaped.
- `plan diagnostic`: One `PlanDiagnostic` from `validate`, with Pkl defect wording and an optional 1-based source line.
- `plan init scaffold`: The generated `config/plan-init-scaffold.md` plan skeleton rendered by `config/pkl/base/plan-init-scaffold.pkl` and embedded as `SCAFFOLD`. Untouched, it fails `sce plan check` with exactly its `Scenarios: TODO` and `Covers: TODO` reference defects (see the [plan structure contract](../sce/plan-structure-contract.md)).

## Divergences from the Pkl line scanner

The extractor follows CommonMark/GFM nesting. The Pkl checker requires fields at exactly `entry indent + 2` and accepts definitions at any indentation. Indentation-only nesting differences, `[X]` checkboxes, and multi-line field values can therefore extract differently. Agreement is pinned by the shared fixtures under `config/pkl/renderers/fixtures/plan-structure/`. The full divergence list and the shared rules live in the [plan structure contract](../sce/plan-structure-contract.md).

## Tests

`cli/src/services/plan/extract.rs` unit tests (`plan::extract`) read the shared plan-structure fixtures through `CARGO_MANIFEST_DIR/../config/pkl/renderers/fixtures/plan-structure`. They cover fenced examples, misplaced definitions, unstructured plans with fenced structured examples, source-sliced field values with lines, and the ID grammar.

Module-level tests in `cli/src/services/plan/mod.rs` make the shared contract mechanical. `plan::fixture_expectations_match_manifest` runs every manifest row through `extract` and `validate` and requires that every fixture is listed. `plan::diagnostic_lines_match_source_positions` asserts exact diagnostic lines, and checks that a missing heading has no line. The `plan::command_*` tests drive `sce plan check` through the parser and stdout/stderr renderer for valid plans (by relative and absolute path), an invalid plan, an unstructured plan, a missing file, and a plan whose commands would create a marker file. `plan::plan_check_appears_in_help`, `plan::completion_scripts_include_plan_check`, `plan::plan_init_appears_in_help`, `plan::completion_scripts_include_plan_init`, `plan::plan_pr_description_appears_in_help`, and `plan::completion_scripts_include_plan_pr_description` cover help and completion. `plan::scaffold_untouched_fails_check` asserts the embedded scaffold has one `{plan-name}` token, defines no decision or scenario entries, and validates to exactly the three untouched-scaffold diagnostics.

The `plan::init_*` tests run `sce plan init` in temporary `git init` repositories through the parser, `execute_with`, and the renderer: creation at the root with exact stdout, file content, and `git status` (`init_creates_scaffold_at_repository_root`); a nested start directory under a root containing a space and an apostrophe, whose third stdout line is executed by `sh` and dispatched back to `plan check` (`init_from_nested_directory_targets_repository_root`); a regular file and a dangling symlink left untouched with no temporary file (`init_never_overwrites_existing_entry`); application-validation, Clap-parse, and missing-argument rejections with an unchanged tree (`init_rejects_unsafe_names_without_writing`); and exit `4` outside a repository (`init_outside_repository_is_runtime_failure`). Three tests call `init::create_with` with injected operations: a cleanup failure after a real hard link exits `0` with the exact stdout, the exact warning line, the complete destination, and the temporary file left in place (`init_cleanup_failure_after_publication_warns_and_succeeds`); an injected link failure exits `4` with the original publish error and an empty `context/plans/` (`init_publication_failure_removes_temporary_and_keeps_error`); and a real conflict on an existing file with an injected cleanup failure exits `3` with `Plan already exists` plus the `Cleanup also failed` line, leaving the existing bytes unchanged (`init_publication_and_cleanup_failure_keeps_original_error_and_destination`). `plan::scaffold_authored_passes_check` shows that filling only `Scenarios:` and `Covers:` makes `check` pass while `TODO` placeholders remain.

The `plan::pr_description_*` tests cover the brief. Three tests call `pr_description::render` on in-memory plans: an exact golden brief for two plans given in reverse order (`pr_description_golden_multiple_plans`), the no-marked-decision output (`pr_description_no_flagged_decisions`), and escaping that round-trips to the original question (`pr_description_escapes_markdown`). Four tests run the command in temporary `git init` repositories through the parser, `execute_with`, and the renderer. They cover byte-identical output for both path orders, invalid argument rejection before file access with exit `3` and empty stdout, four marked decisions with exit `3`, and the invalid-plan and missing-plan cases with exit `3` and `4`.
