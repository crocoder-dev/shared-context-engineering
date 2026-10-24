# Plan: plan-pr-description

## Change summary

This plan adds a deterministic reviewer brief for GitHub pull requests. It extends the structured plan format from PR #342 with two optional, paired design-decision fields: `Review: required` and `Review question: {one question}`. Both structural checkers, `cli/src/services/plan/validate.rs` and `config/pkl/renderers/plan-structure.pkl`, validate the pair with identical diagnostics. The canonical plan template and authoring rules tell the planning agent to mark only decisions that need senior engineering judgment.

A new `sce plan pr-description --repo {owner/name} --head {sha} {plan-path}...` command validates each plan with the existing extractor and validator. It then prints a compact Markdown brief to stdout. The brief lists the input plans and the explicitly marked review questions, with links to the exact decision lines at the supplied commit. The brief uses no AI, heuristics, risk scoring, summarization, network access, or file writes. The plan dogfoods the format on `context/plans/sce-plan-init.md` D4 and D6. This change extends existing behavior and keeps every current plan valid.

## Design decisions

- D1: `Review:` and `Review question:` are optional fields owned by design decisions, and both checkers validate them as one pair.
  - Boundary: `validate.rs` and `plan-structure.pkl` own the pair rules. `extract.rs` stays unchanged because its field grammar already accepts `Review question`.
  - Rationale: A field owned by the decision kind reuses the existing misplaced-field rule, the line-anchored diagnostics, and the shared fixture manifest.
  - Rejected: A separate review section that lists decision IDs — it splits one decision across two places and needs new reference rules.
  - Trade-offs: Each new defect message is a change to both checkers, and the Pkl line scanner needs explicit continuation-line detection for the single-line rule.
- D2: The brief renderer is a pure function in a new `cli/src/services/plan/pr_description.rs` module, reached through a `PlanRequest::PrDescription` variant.
  - Boundary: `pr_description.rs` owns argument grammar, collection of marked decisions, the question limit, escaping, and rendering. `command.rs` owns file reads, repository-root resolution, and error classes.
  - Rationale: A pure renderer over extracted `Plan` values makes golden and determinism tests independent of the filesystem.
  - Rejected: Rendering inside `command.rs` — it mixes I/O with output formatting and makes golden tests depend on temporary repositories.
  - Trade-offs: The command adds a third `PlanRequest` variant and one more help and completion surface.
- D3: The command accepts only strict input grammar, so every URL is built without percent-encoding.
  - Boundary: `--repo` matches `{owner}/{name}` with owner `[A-Za-z0-9-]+` and name `[A-Za-z0-9._-]+`, excluding `.` and `..`. `--head` is exactly 40 lowercase hexadecimal characters. Each plan path is `context/plans/{file}.md`, where `{file}` uses only `[A-Za-z0-9._-]` and is not `.` or `..`.
  - Rationale: Strict grammar rejects absolute paths, `..` traversal, nested paths, and characters that need URL encoding before any file access.
  - Rejected: Percent-encoding arbitrary paths — it adds an encoding contract and accepts paths that SCE never creates.
  - Trade-offs: Abbreviated or uppercase SHAs and plans in subdirectories of `context/plans/` are rejected.
- D4: The renderer escapes a fixed set of Markdown punctuation in every interpolated text with a backslash.
  - Boundary: The escaped set is `` \ ` * _ [ ] < > ! | ~ & `` and applies to review questions and plan link labels. URLs are never escaped because D3 restricts their characters.
  - Rationale: CommonMark renders a backslash-escaped ASCII punctuation character as that literal character, so the rendered question keeps its exact wording.
  - Rejected: Wrapping questions in code spans — it changes the presentation and fails on questions that contain backticks.
  - Trade-offs: Inline code and emphasis in a question render as literal punctuation, not as formatting.
- D5: The command reads each plan from the working tree at the Git repository root, not from the Git object at `--head`.
  - Boundary: `command.rs` resolves the root with `git rev-parse --show-toplevel`, as `sce plan init` does, and reads `{root}/{plan-path}`. The command never reads Git objects and never checks that the working tree matches `--head`.
  - Rationale: A preview works before the plan edits are committed, and the command stays independent of local object availability.
  - Rejected: `git show {sha}:{path}` — it fails for an uncommitted preview and for a SHA that is not available locally.
  - Trade-offs: When the working tree differs from `--head`, a decision link can point at a wrong line. The caller must pass the SHA that contains the plan files as read.
  - Review: required
  - Review question: Is reading plans from the working tree instead of the `--head` commit acceptable for stable links?
- D6: More than three marked questions across all input plans is a validation error, never a truncation.
  - Boundary: The limit applies after every input plan passes the structural check. The error names the count and every marked decision as `{plan-label} {decision-id}`.
  - Rationale: The brief targets a two-to-three-minute review, and a silent truncation hides a decision that the author marked.
  - Rejected: Rendering the first three questions with a warning — the output then omits marked decisions without a failing exit code.
  - Trade-offs: A pull request with four or more marked decisions must remove marks or split the plan set before it can generate a brief.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: The Rust and Pkl checkers accept decisions with and without a complete review pair, and both report identical diagnostics for a missing pair, an invalid `Review:` value, an empty question, a multi-line question, a duplicate review field, and a misplaced review field.
  - Validate: `nix run .#pkl-check-generated` and `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::` pass with the new review-field fixtures listed in `expectations.tsv`.
  - Decisions: D1
  - Scenarios: S1, S2, S3
- [x] AC2: Every structured plan that passed `sce plan check` before this change still passes it without edits.
  - Validate: `sce plan check` built from this branch passes on `context/plans/plan-decisions-and-verification-scenarios.md` and `context/plans/sce-plan-check.md`, and every existing `pass` fixture keeps its `pass` row.
  - Decisions: D1
  - Scenarios: S4
- [x] AC3: The canonical plan template and authoring rules document the optional review pair, and tell the planning agent to mark only decisions that need senior engineering judgment.
  - Validate: inspection of the `Design decision rules`, the decision template entry, and the `Reference rules` field list in `config/pkl/base/workflow-change-to-plan.pkl`, plus a passing `nix run .#pkl-check-generated`.
  - Decisions: D1
  - Scenarios: none (inspection)
- [x] AC4: `sce plan pr-description` prints the brief between the `<!-- sce:intent-review:start -->` and `<!-- sce:intent-review:end -->` markers, with sorted plan links, marked questions in plan order, escaped question text, exact decision-line links, and byte-identical output on repeated runs.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::pr_description`
  - Decisions: D2, D4
  - Scenarios: S5, S6, S7, S8
- [x] AC5: `sce plan pr-description` rejects invalid repository, SHA, and plan-path arguments, duplicate plan paths, structurally invalid plans, and more than three marked questions, and it writes nothing to stdout in each case.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::pr_description`
  - Decisions: D3, D5, D6
  - Scenarios: S9, S10, S11
- [x] AC6: The command runs offline and adds no dependency, trait, `dyn` dispatch, async runtime, database access, GitHub API call, network call, or file write.
  - Validate: inspection of the branch diff for `cli/Cargo.toml`, `cli/Cargo.lock`, and `cli/src/services/plan/`.
  - Decisions: D2, D5
  - Scenarios: none (inspection)
- [x] AC7: The PR #342 preview over its three plans contains exactly the two questions marked on `sce-plan-init` D4 and D6, links to the D4 and D6 entry lines, and links to all three plans.
  - Validate: `sce plan pr-description --repo crocoder-dev/shared-context-engineering --head "$(git rev-parse HEAD)" context/plans/plan-decisions-and-verification-scenarios.md context/plans/sce-plan-check.md context/plans/sce-plan-init.md`, built from this branch, prints the brief that S12 describes.
  - Decisions: D5
  - Scenarios: S12
- [x] AC8: `sce --help`, `sce plan --help`, and the generated Bash, Zsh, and Fish completions list `pr-description`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::plan_pr_description_appears_in_help`
  - Decisions: D2
  - Scenarios: S13

### Verification scenarios

- S1: Decisions with and without the review pair are valid
  - Given: A structured plan in which one decision carries `Review: required` and a one-line `Review question:`, and another decision carries neither field.
  - When: Both checkers check the plan.
  - Then: Both report `structured plan: reference integrity holds`.
  - Check: fixture `review-fields-valid.md` with a `pass` row in `expectations.tsv`, created by T01, run by `plan::fixture_expectations_match_manifest` and `nix run .#pkl-check-generated`.
- S2: Missing, malformed, and duplicate review fields fail with identical messages
  - Given: Decisions that carry only `Review:`, only `Review question:`, `Review: optional`, an empty `Review question:`, a question with a continuation line, and a repeated `Review:` or `Review question:`.
  - When: Both checkers check the fixtures.
  - Then: Both report exactly the messages in the fixture rows: `` `D1` has `Review:` without `Review question:` ``, `` `D1` has `Review question:` without `Review:` ``, `` `Review:` on `D1` must be `required` ``, `` `Review question:` on `D1` is empty ``, `` `Review question:` on `D1` must be a single line ``, and `` `Review:` appears more than once on `D1` ``.
  - Check: fixtures `review-fields-missing-pair.md`, `review-fields-malformed.md`, and `review-fields-duplicate.md` with `fail` rows, created by T01, run by `plan::fixture_expectations_match_manifest` and `nix run .#pkl-check-generated`.
- S3: Misplaced review fields and diagnostic lines
  - Given: An acceptance criterion and a task that carry `Review:` and `Review question:`, and the defect fixtures from S2.
  - When: The Rust validator checks them.
  - Then: Each misplaced field reports `` `{key}:` on `{id}` is misplaced; it belongs to a design decision `` at the field line. Pair defects point at the decision line. Value, empty, single-line, and duplicate defects point at the offending field line.
  - Check: fixture `review-fields-misplaced.md` with a `fail` row, plus extended assertions in `plan::diagnostic_lines_match_source_positions`, created by T01.
- S4: Existing structured plans stay valid
  - Given: The existing `pass` fixtures and the unchanged PR #342 plans `plan-decisions-and-verification-scenarios.md` and `sce-plan-check.md`.
  - When: Both checkers run after T01.
  - Then: Every existing `pass` row still passes, and `sce plan check` exits `0` on both plans.
  - Check: `nix run .#pkl-check-generated`, `plan::fixture_expectations_match_manifest`, and `nix run .#sce -- plan check` on both plans.
- S5: Golden brief for multiple plans
  - Given: Two valid in-memory plans supplied in reverse lexicographic path order, with marked decisions on the second and first decision of one plan and one marked decision in the other.
  - When: The renderer runs with a fixed repository and SHA.
  - Then: The output equals the golden string byte for byte: start marker, `## Intent review`, a `**Plans:**` line with links in sorted path order, `**Decisions requiring review**`, numbered questions in sorted plan order and source decision order, each with a `blob/{sha}/{path}#L{line}` link, then the end marker and one trailing newline.
  - Check: `plan::pr_description_golden_multiple_plans`, created by T03.
- S6: No marked decisions
  - Given: Valid plans with no `Review: required` decision.
  - When: The renderer runs.
  - Then: The output contains the markers, the heading, the `**Plans:**` line, and the line `No decisions explicitly flagged for review.`, and it contains no `**Decisions requiring review**` line.
  - Check: `plan::pr_description_no_flagged_decisions`, created by T03.
- S7: Markdown escaping keeps the question text
  - Given: A review question that contains `` ` ``, `*`, `_`, `[`, `]`, `<`, `>`, `|`, `~`, `&`, and `\`.
  - When: The renderer runs.
  - Then: Each character of the D4 set is preceded by one backslash, every other character is copied verbatim, and removing those backslashes gives the original question.
  - Check: `plan::pr_description_escapes_markdown`, created by T03.
- S8: Repeated generation is byte-identical
  - Given: The same repository, SHA, and plan files in a temporary Git repository.
  - When: `sce plan pr-description` runs twice through the parser and `execute_with`, once with the paths in reverse order.
  - Then: Both stdout strings are byte-identical.
  - Check: `plan::pr_description_repeated_runs_are_identical`, created by T03.
- S9: Invalid arguments are rejected before file access
  - Given: Repository values `owner`, `owner/`, `/name`, `a/b/c`, and `owner/..`, SHA values with 39 characters, 41 characters, uppercase hexadecimal, and a non-hexadecimal character, plan paths that are absolute, outside `context/plans/`, nested, without `.md`, or with `..`, and one plan path given twice.
  - When: `sce plan pr-description` runs with each input.
  - Then: Each run exits `3` (`SCE-ERR-VALIDATION`), names the rejected argument, and writes nothing to stdout.
  - Check: `plan::pr_description_rejects_invalid_arguments`, created by T03.
- S10: More than three marked questions is an error
  - Given: Two valid plans that mark four decisions in total.
  - When: `sce plan pr-description` runs with both plans.
  - Then: The run exits `3`, the error states the count `4`, lists every marked decision as `{plan-label} {decision-id}`, and stdout is empty.
  - Check: `plan::pr_description_rejects_more_than_three_questions`, created by T03.
- S11: Invalid or missing plans fail
  - Given: One structurally invalid plan, and separately one valid path that names no file.
  - When: `sce plan pr-description` runs with each input.
  - Then: The invalid plan exits `3` with the same diagnostic lines that `sce plan check` prints for it. The missing file exits `4` (`SCE-ERR-RUNTIME`). Stdout is empty in both cases.
  - Check: `plan::pr_description_rejects_invalid_plan`, created by T03.
- S12: PR #342 dogfood preview
  - Given: `sce-plan-init.md` D4 and D6 carry the review pair with the questions from AC7, and the other two PR #342 plans carry no review fields.
  - When: The AC7 command runs at the repository root.
  - Then: Stdout has three plan links, exactly two numbered questions, `Are hard-link-only publication and its filesystem limitations acceptable?` linked to the D4 entry line and `Is workflow-only completeness enforcement acceptable?` linked to the D6 entry line.
  - Check: the AC7 command, with each `#L{line}` compared against `grep -n -E '^- D(4|6):' context/plans/sce-plan-init.md`.
- S13: Help and completions list the command
  - Given: The built CLI.
  - When: `sce --help`, `sce plan --help`, and `sce completion` for Bash, Zsh, and Fish run.
  - Then: The help output contains a `sce plan pr-description` usage line, and each completion script contains `pr-description`.
  - Check: `plan::plan_pr_description_appears_in_help` and `plan::completion_scripts_include_plan_pr_description`, created by T03.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::`
- `nix flake check`
- `nix build .#default --no-link`

### Context sync

- `context/cli/plan-service.md`: the review pair rules, the `pr_description.rs` module, the `PrDescription` request, and the `sce plan pr-description` command contract.
- `context/sce/plan-structure-contract.md`: the review pair in the shared rule set, the new defect messages, and the new fixtures.
- `context/sce/shared-context-plan-workflow.md`: the optional review pair in the plan format and the marking rule for planning agents.
- `context/cli/cli-command-surface.md` and `context/glossary.md`: the new command and the terms `review question` and `intent review brief`.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/plan/` (`validate.rs`, `command.rs`, `mod.rs`, new `pr_description.rs`), `cli/src/cli_schema.rs`, `cli/src/services/parse/command_runtime.rs`, help text, `config/pkl/renderers/plan-structure.pkl`, `config/pkl/renderers/fixtures/plan-structure/`, `config/pkl/base/workflow-change-to-plan.pkl`, `context/plans/sce-plan-init.md` D4 and D6, and the durable context files listed under `Context sync`.
- **Out of scope:** creating or updating a pull request, pushing commits, editing any GitHub pull request description, replacing an existing description section, and changes to `/next-task` or `/validate`.
- **Constraints:** Reuse the existing `markdown-rs` extractor and validator. Add no dependency, trait, `dyn` dispatch, async runtime, database, GitHub API, or network call. Keep exact Rust and Pkl diagnostic parity through `expectations.tsv`. Preserve completed tasks, validation evidence, and historical records in `sce-plan-init.md`.
- **Non-goal:** Automatic selection, ranking, summarization, or risk scoring of decisions. Acceptance-criteria tables, task logs, validation reports, and CI status in the brief.

## Assumptions

- The `Review question:` value is not required to end with `?`. The rule requires only a nonempty single-line value, as the change request states.
- In the no-marked-decisions case, the brief omits the `**Decisions requiring review**` line and prints `No decisions explicitly flagged for review.` in its place.
- A plan link label is the plan file name without `.md`, for example `sce-plan-init`. Multiple plan links are joined with `, ` on the `**Plans:**` line.
- Numbered questions are separated by one blank line, as in the requested example, and the output ends with the end marker and one newline.
- Argument and limit errors use `CliError::validation` (exit `3`), and an unreadable plan file uses `CliError::runtime` (exit `4`), as `sce plan check` does.
- The plan template's decision example shows the review pair as optional lines, and the generated plan init scaffold does not include them, because an unmarked decision is the default.

## Task stack

- [x] T01: `Validate optional review fields in both plan checkers` (status:done)
  - Task ID: T01
  - Covers: AC1, AC2
  - Scope: In — `Review` and `Review question` as decision-owned fields in `validate.rs` and `plan-structure.pkl`, the pair, value, empty, single-line, and duplicate rules with the S2 messages, the five `review-fields-*.md` fixtures and their `expectations.tsv` rows, and extended line assertions in `plan::diagnostic_lines_match_source_positions`. Out — template text, the new command, and edits to existing plans.
  - Dependencies: none
  - Done when: Both checkers agree on every manifest row, the new fixtures cover S1, S2, and S3, and every existing `pass` fixture still passes.
  - Verify: `nix run .#pkl-check-generated`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::fixture`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::diagnostic_lines_match_source_positions`.
  - Completed: 2026-10-24
  - Files changed: `cli/src/services/plan/validate.rs`, `cli/src/services/plan/mod.rs`, `config/pkl/renderers/plan-structure.pkl`, `config/pkl/renderers/fixtures/plan-structure/expectations.tsv`, `config/pkl/renderers/fixtures/plan-structure/review-fields-valid.md`, `config/pkl/renderers/fixtures/plan-structure/review-fields-missing-pair.md`, `config/pkl/renderers/fixtures/plan-structure/review-fields-malformed.md`, `config/pkl/renderers/fixtures/plan-structure/review-fields-duplicate.md`, `config/pkl/renderers/fixtures/plan-structure/review-fields-misplaced.md`
  - Result: Both checkers treat `Review` and `Review question` as decision-owned fields, so the existing misplaced rule covers acceptance criteria and tasks. A new per-decision review rule set reports the duplicate key once at its second occurrence, a missing pair half at the decision line, and a non-`required` value, an empty question, and a multi-line question at the field line. The value, empty, and single-line rules run on every occurrence. Rust detects multi-line values through the newline that the extracted lead paragraph keeps. Pkl records each field's lazy continuation lines (until a blank, fenced, heading, list-item, or block-quote line), joins them with the field value, and applies the same three predicates. `extract.rs` is unchanged. Five new fixtures have manifest rows: `review-fields-valid.md` passes (it includes a nested list under a question, which is not a continuation). `review-fields-malformed.md` also covers a multi-line `Review:` value.
  - Verify outcomes: `nix run .#pkl-check-generated` passed (146 files; every manifest row matched for Pkl). `plan::fixture` passed (`fixture_expectations_match_manifest`). `plan::diagnostic_lines_match_source_positions` passed. Extra checks: all 28 `plan::` tests passed, `run-cli-cargo.sh clippy --all-targets -- -D warnings` was clean, and `nix run .#sce -- plan check` exited `0` on `plan-decisions-and-verification-scenarios.md`, `sce-plan-check.md`, and `plan-pr-description.md` (S4).
  - Deviations: The S2 messages are written with `D1`. The missing-pair and duplicate fixtures need two decisions, so their second messages name `D2`.
  - Context impact: durable — the plan structure contract gains the review pair rules, six new defect messages, and five fixtures (`context/sce/plan-structure-contract.md`, `context/cli/plan-service.md`).
  - Context synchronization: synced

- [x] T02: `Document the review pair in the plan template and authoring rules` (status:done)
  - Task ID: T02
  - Covers: AC3
  - Scope: In — the decision template entry, `Design decision rules`, and `Reference rules` field list in `config/pkl/base/workflow-change-to-plan.pkl`, with a rule to mark only decisions that need senior engineering judgment and to add no placeholder value. Out — the plan init scaffold content, `/next-task`, and `/validate`.
  - Dependencies: T01
  - Done when: The generated `plan-template.md` shows the optional pair and the marking rule, and the skeleton and generation contract checks still pass.
  - Verify: `nix run .#pkl-check-generated`.
  - Completed: 2026-10-24
  - Files changed: `config/pkl/base/workflow-change-to-plan.pkl`
  - Result: The structured plan template (`changeToPlanPlanTemplate`) has two changes. First, its D1 entry shows the optional lines `- Review: required` and a single-line `- Review question:` placeholder, which says to remove both lines when no review is needed. Second, the `Design decision rules` gain three bullets: mark only a decision that needs senior engineering judgment; the pair is all or nothing, with `Review:` exactly `required` and a nonempty one-line question; and an unmarked decision carries no review field and no placeholder value such as `Review: none`. The `Reference rules` field list now names `Review:` and `Review question:` as fields that belong to a design decision. The compact package template and the plan init scaffold are unchanged.
  - Verify outcomes: `nix run .#pkl-check-generated` passed (146 files). This run includes the plan-template structure, structured schema, and plan-init scaffold contract checks. An ephemeral `pkl-generate` into the scratchpad confirmed that the generated `sce-change-to-plan/references/plan-template.md` contains the review pair and the marking rules.
  - Context impact: durable — the plan workflow's authoring rules now include the optional review pair and its marking rule (`context/sce/shared-context-plan-workflow.md`).
  - Context synchronization: synced

- [x] T03: `Add the sce plan pr-description command` (status:done)
  - Task ID: T03
  - Covers: AC4, AC5, AC6, AC8
  - Scope: In — `PlanSubcommand::PrDescription` in `cli_schema.rs`, its mapping in `command_runtime.rs`, `PlanRequest::PrDescription` in `command.rs`, the pure renderer in `pr_description.rs`, help text, and the S5 to S11 and S13 tests. Out — any GitHub API, network access, file writes, and edits to plan files.
  - Dependencies: T01
  - Done when: The command renders the golden brief, rejects every S9, S10, and S11 input with the stated exit code and empty stdout, appears in help and completions, and adds no dependency.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::pr_description`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::plan_pr_description_appears_in_help`; `git diff --stat main -- cli/Cargo.toml cli/Cargo.lock` shows no change from this task.
  - Completed: 2026-10-24
  - Files changed: `cli/src/services/plan/pr_description.rs` (new), `cli/src/services/plan/command.rs`, `cli/src/services/plan/init.rs`, `cli/src/services/plan/mod.rs`, `cli/src/cli_schema.rs`, `cli/src/services/parse/command_runtime.rs`, `cli/src/command_surface.rs`
  - Result: `sce plan pr-description --repo {owner/name} --head {sha} {plan-path}...` is wired through `PlanSubcommand::PrDescription`, `PlanRequest::PrDescription`, and the new pure `pr_description.rs` module. That module owns the D3 argument grammar (duplicate paths are rejected before any file access), the D4 backslash escaping, the three-question limit (`Found {n} decisions marked 'Review: required' ...: {label} {id}, ...`), and rendering. `command.rs` parses arguments first, then resolves the root with `git rev-parse --show-toplevel` (`resolve_repository_root` is now shared with `sce plan init` and takes the command name for its remediation text). It then reads `{root}/{path}` and reuses the `sce plan check` load, extract, validate, and diagnostic path through a new `load_valid_plan` helper. Brief layout: start marker, `## Intent review`, blank line, the `**Plans:**` line with sorted escaped labels, blank line, then either `**Decisions requiring review**` followed by numbered `{n}. {escaped question} — [{label} {D-id}]({blob-url}#L{line})` items separated by blank lines, or `No decisions explicitly flagged for review.`. A blank line and the end marker close the brief, and stdout ends with one newline. The top-level and plan help, the `Plan Usage:` section, and the clap arguments list the command. Per user request, the clap `///` argument doc comments in `PlanSubcommand` (including the existing `check` and `init` ones) became `help = "..."` attributes, so the help text is unchanged and no code comments remain.
  - Verify outcomes: `plan::pr_description` passed (7 tests: golden, no flagged, escaping, repeated runs, invalid arguments, more than three questions, invalid plan). `plan::plan_pr_description_appears_in_help` passed. `git diff --stat main -- cli/Cargo.toml cli/Cargo.lock` is non-empty, but only because of the earlier branch commit `1c82b1f7` (the `markdown` crate from PR #342). `git diff --stat HEAD -- cli/Cargo.toml cli/Cargo.lock` is empty, so this task adds no dependency. Extra checks: all 37 `plan::` tests passed (including `completion_scripts_include_plan_pr_description`), `run-cli-cargo.sh clippy --all-targets -- -D warnings` was clean, and a real-binary run over `plan-pr-description.md` and `sce-plan-check.md` printed the brief with the D5 question linked to `#L31`, which matches `grep -n '^- D5:'`.
  - Deviations: Help text lives partly in `cli/src/command_surface.rs` (the `Plan Usage:` section), which the Scope list does not name; it was edited as part of "help text". The plan's Verify grep on `main` cannot be empty on this branch, so the evidence above uses `HEAD`. `PLAN_CLAP_ABOUT` and `PLAN_TOP_LEVEL_PURPOSE` now mention the intent-review brief.
  - Context impact: durable — new `sce plan pr-description` command contract, `pr_description.rs` module, `PlanRequest::PrDescription`, shared `resolve_repository_root`, and help surface (`context/cli/plan-service.md`, `context/cli/cli-command-surface.md`, `context/glossary.md`).
  - Context synchronization: synced

- [x] T04: `Mark PR #342 review decisions and record the preview` (status:done)
  - Task ID: T04
  - Covers: AC7
  - Scope: In — the review pair on D4 and D6 in `context/plans/sce-plan-init.md` with the exact AC7 questions, and the generated three-plan preview recorded in this task's completion evidence. Out — other decisions, completed tasks, validation reports, historical records, and the real pull request description.
  - Dependencies: T03
  - Done when: `sce plan check` passes on `sce-plan-init.md`, and the AC7 command prints exactly the two questions with links to the D4 and D6 entry lines and to the three plans.
  - Verify: `nix run .#sce -- plan check context/plans/sce-plan-init.md`; the AC7 command through `nix run .#sce --`; `git diff context/plans/sce-plan-init.md` shows only the four added field lines.
  - Completed: 2026-10-24
  - Files changed: `context/plans/sce-plan-init.md`, `context/plans/plan-pr-description.md`
  - Result: `sce-plan-init.md` D4 and D6 each gain `Review: required` and a `Review question:` line after their `Trade-offs` field, with the exact AC7 questions. No other decision, task, validation report, or historical record changed. The AC7 preview at `--head cc8a30f565ca580d65d32e83897131ea0d09a199` printed the start marker, `## Intent review`, a `**Plans:**` line linking `plan-decisions-and-verification-scenarios`, `sce-plan-check`, and `sce-plan-init` in sorted order, `**Decisions requiring review**`, then `1. Are hard-link-only publication and its filesystem limitations acceptable? — [sce-plan-init D4](https://github.com/crocoder-dev/shared-context-engineering/blob/cc8a30f565ca580d65d32e83897131ea0d09a199/context/plans/sce-plan-init.md#L50)` and `2. Is workflow-only completeness enforcement acceptable? — [sce-plan-init D6](https://github.com/crocoder-dev/shared-context-engineering/blob/cc8a30f565ca580d65d32e83897131ea0d09a199/context/plans/sce-plan-init.md#L72)`, and the end marker.
  - Verify outcomes: `nix run .#sce -- plan check context/plans/sce-plan-init.md` exited `0` with `structured plan: reference integrity holds`. The AC7 command through `nix run .#sce --` exited `0` with three plan links and exactly two numbered questions. `#L50` and `#L72` match `grep -n -E '^- D(4|6):' context/plans/sce-plan-init.md`. `git diff --numstat context/plans/sce-plan-init.md` shows `4 0`, and the diff contains only the four field lines.
  - Deviations: The preview used the uncommitted working tree, as D5 allows. Both added pairs come after their decision lines, so the D4 and D6 anchors are the same at `HEAD`.
  - Context impact: none — this task changes only plan content. The command and format contracts were synchronized in T01 to T03.
  - Context synchronization: synced

## Open questions

None. The change request specifies the format, rules, and limits. The one material trade-off, reading plans from the working tree, is marked for review on D5.

## Validation Report

**Status:** validated  
**Date:** 2026-10-24

### Commands run

- `nix run .#sce -- plan check context/plans/plan-pr-description.md` -> exit 0 (`structured plan: reference integrity holds`)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed: 146 files; every manifest row matched)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::` -> exit 0 (37 passed, 0 failed)
- `nix flake check` -> exit 0 (all 6 checks passed: cli tests, clippy, fmt, generated-input, pkl-generated, codex-hook-command)
- `nix build .#default --no-link` -> exit 0
- `nix run .#sce -- plan check` on `plan-decisions-and-verification-scenarios.md`, `sce-plan-check.md`, and `sce-plan-init.md` -> exit 0 each (`structured plan: reference integrity holds`)
- `nix run .#sce -- plan pr-description --repo crocoder-dev/shared-context-engineering --head "$(git rev-parse HEAD)" context/plans/plan-decisions-and-verification-scenarios.md context/plans/sce-plan-check.md context/plans/sce-plan-init.md` -> exit 0 (three plan links, two numbered questions linked to `#L50` and `#L72`)
- `grep -n -E '^- D(4|6):' context/plans/sce-plan-init.md` -> exit 0 (D4 at line 50, D6 at line 72)
- `nix run .#sce -- --help`, `nix run .#sce -- plan --help`, `nix run .#sce -- completion --shell {bash,zsh,fish}` -> exit 0 (each output contains `pr-description`)
- `git diff 61f88b3b -- cli/Cargo.toml cli/Cargo.lock config/pkl/renderers/fixtures/plan-structure/expectations.tsv cli/src/services/plan/` -> exit 0 (inspected; baseline is the commit before this plan)

### Success-criteria verification

- [x] AC1: Rust and Pkl checkers accept and reject review pairs with identical diagnostics -> `pkl-check-generated` and `plan::` (including `fixture_expectations_match_manifest` and `diagnostic_lines_match_source_positions`) passed with the five `review-fields-*.md` manifest rows; scenarios: S1, S2, S3
- [x] AC2: Existing structured plans still pass -> `sce plan check` exited `0` on `plan-decisions-and-verification-scenarios.md` and `sce-plan-check.md`; the `expectations.tsv` diff since `61f88b3b` only adds the five review rows, so every existing `pass` row is unchanged; scenarios: S4
- [x] AC3: Template and authoring rules document the review pair -> inspected `config/pkl/base/workflow-change-to-plan.pkl` lines 620-621 (decision template entry), 782-790 (`Design decision rules`, including the senior-engineering-judgment marking rule and no placeholder value), and 833 (`Reference rules` field list); `pkl-check-generated` passed; scenarios: none (inspection)
- [x] AC4: Brief rendering, sorting, escaping, links, and determinism -> `plan::pr_description` tests passed; scenarios: S5, S6, S7, S8
- [x] AC5: Invalid arguments, duplicates, invalid plans, and more than three questions are rejected with empty stdout -> `plan::pr_description` tests passed; scenarios: S9, S10, S11
- [x] AC6: Offline, no new dependency, trait, `dyn`, async, database, network, or file write -> `git diff 61f88b3b -- cli/Cargo.toml cli/Cargo.lock` is empty; the `cli/src/services/plan/` diff adds no `trait`, `dyn`, `async`, network, or database code; `pr_description.rs` has no `std::fs`, `std::process`, or `std::net` use; the only added `fs::write` and `create_dir_all` calls are in `#[cfg(test)]` helpers in `mod.rs`; scenarios: none (inspection)
- [x] AC7: PR #342 preview contains exactly the D4 and D6 questions and three plan links -> AC7 command printed both questions linked to `#L50` and `#L72`, matching the D4 and D6 entry lines, plus links to all three plans; scenarios: S12
- [x] AC8: Help and completions list `pr-description` -> `plan::plan_pr_description_appears_in_help` and `plan::completion_scripts_include_plan_pr_description` passed; the real binary help and Bash, Zsh, and Fish completions contain `pr-description`; scenarios: S13

### Scenario verification

- S1 (AC1): test `review-fields-valid.md` `pass` row via `plan::fixture_expectations_match_manifest` and `nix run .#pkl-check-generated` -> both passed; status: Verified
- S2 (AC1): test `review-fields-missing-pair.md`, `review-fields-malformed.md`, `review-fields-duplicate.md` `fail` rows via `plan::fixture_expectations_match_manifest` and `nix run .#pkl-check-generated` -> both checkers matched every manifest message; status: Verified
- S3 (AC1): test `review-fields-misplaced.md` `fail` row and `plan::diagnostic_lines_match_source_positions` -> both passed; status: Verified
- S4 (AC2): command `nix run .#pkl-check-generated`, test `plan::fixture_expectations_match_manifest`, and `nix run .#sce -- plan check` on both PR #342 plans -> all passed, both plans exited `0`; status: Verified
- S5 (AC4): test `plan::pr_description_golden_multiple_plans` -> passed; status: Verified
- S6 (AC4): test `plan::pr_description_no_flagged_decisions` -> passed; status: Verified
- S7 (AC4): test `plan::pr_description_escapes_markdown` -> passed; status: Verified
- S8 (AC4): test `plan::pr_description_repeated_runs_are_identical` -> passed; status: Verified
- S9 (AC5): test `plan::pr_description_rejects_invalid_arguments` -> passed; status: Verified
- S10 (AC5): test `plan::pr_description_rejects_more_than_three_questions` -> passed; status: Verified
- S11 (AC5): test `plan::pr_description_rejects_invalid_plan` -> passed; status: Verified
- S12 (AC7): command AC7 `sce plan pr-description` at `--head cc8a30f565ca580d65d32e83897131ea0d09a199` -> three plan links; `Are hard-link-only publication and its filesystem limitations acceptable?` linked to `#L50` and `Is workflow-only completeness enforcement acceptable?` linked to `#L72`, matching `grep -n -E '^- D(4|6):'`; status: Verified
- S13 (AC8): test `plan::plan_pr_description_appears_in_help` and `plan::completion_scripts_include_plan_pr_description` -> passed; real binary `sce --help` shows the `sce plan pr-description` usage line and each completion script contains `pr-description`; status: Verified

### Failed checks and follow-ups

- None.

### Residual risks

- The AC7 preview read the uncommitted working tree (`sce-plan-init.md` is staged, not committed) at `--head` `cc8a30f5`. The D4 and D6 lines are the same at `HEAD`, but the review fields are not in that commit yet, so the links resolve correctly only after the change is committed and the SHA that contains it is passed (D5 trade-off).
- `nix flake check` covered only `x86_64-linux`; other systems were omitted.
