# Plan: plan-decision-brief

## Change summary

This plan adds a hidden `sce plan decision-brief --repo <owner/name> --head <sha> <path>...` command. It prints a short Markdown Decision Brief for a GitHub pull request. Each marked design decision shows its review question, its authored proposed decision, and one link to the decision line at the `--head` commit. The brief helps a senior reviewer judge the marked architectural choices in two to three minutes, without the PR diff.

The change extends the plan service from PR #342. It reuses the `markdown-rs` extractor, the `validate` rules, the review-pair selection, the three-question limit, the URL builder, and the Markdown escaping in `pr_description.rs`. The extractor gains the decision statement. A format enum selects between the existing intent-review layout and the new Decision Brief layout. Unlike `sce plan pr-description`, the new command reads every plan from the Git object at `--head`, so each link matches the parsed content. `sce plan pr-description` keeps its working-tree reads and its byte-identical output. The plan also adds review-question authoring guidance to the canonical `/change-to-plan` template rules.

## Design decisions

- D1: The extractor owns the definition statement as `PlanEntry.statement`. It is the source slice after `ID:` in the definition's first paragraph, with whitespace runs folded to one space and trimmed.
  - Boundary: `cli/src/services/plan/extract.rs` produces `statement` in the same `definition` path that parses the ID. Renderers read `statement` and never parse Markdown or scan source lines.
  - Rationale: `parse_id_token` already returns the text after `ID:`. Keeping it removes a second parse and keeps the rules for authoritative sections, fenced examples, lines, and order unchanged.
  - Rejected: A regex or line scan for `- Dn:` in the renderer — this duplicates the extractor and ignores fenced examples. An `Option<String>` only on decisions — every kind has the same opening paragraph, and one shape is simpler.
  - Trade-offs: Every entry carries a string that only the Decision Brief uses. Folding whitespace changes the display of authored line breaks, but it keeps every word.
- D2: `pr_description.rs` keeps one selection pass and one argument grammar for both commands. A `BriefFormat::{IntentReview, DecisionBrief}` enum selects only the layout. A marked decision with an empty statement fails only in the `DecisionBrief` layout.
  - Boundary: `pr_description.rs` owns selection, plan ordering, the limit, URLs, escaping, and both layouts. `command.rs` owns I/O and passes the format. `validate.rs` does not change.
  - Rationale: One selection pass makes both commands agree on which decisions are marked. A static enum follows the repository's static dispatch rule. The empty-statement check stays out of `validate`, so it cannot invalidate unrelated plans.
  - Rejected: A second renderer module — it duplicates selection and escaping. A renderer trait or `dyn` dispatch — the repository uses enums for static dispatch.
  - Trade-offs: One `render` function carries two layouts. The `pr-description` call sites change to pass `BriefFormat::IntentReview`.
- D3: `decision-brief` reads each plan only from the `--head` commit through Git plumbing at the repository root. It runs `git rev-parse --verify --quiet <sha>^{commit}`, then `git ls-tree -z <sha> -- <path>`, and accepts only a regular blob with mode `100644` or `100755`. It then runs `git cat-file blob <oid>` and decodes the bytes as UTF-8. `pr-description` keeps its working-tree reads.
  - Boundary: `cli/src/services/plan/command.rs` owns the Git process calls, with arguments passed directly and no shell. Extraction, validation, and diagnostics reuse the shared load path after the source text is read. `--repo` is a caller-supplied GitHub repository identifier, and `--head` must identify a commit that is available in the local repository. The generated source links refer to the supplied repository and commit. The command guarantees local source integrity: the parsed plan content and the source line numbers match the local `--head` commit. It does not guarantee remote link availability: it does not check that the commit was pushed to GitHub or exists in the `--repo` repository.
  - Rationale: Links are built from `--head`, so the parsed text must come from `--head`. A tree entry check rejects symlinks, and Git resolves paths only inside the commit tree, so no local filesystem path is followed.
  - Rejected: `git show <sha>:<path>` — it returns a symlink blob's target text as content without an error. A working-tree read with a cleanliness check — it fails on unrelated edits and still reads the wrong source. The GitHub API — the command stays offline.
  - Trade-offs: The commit must exist locally, and uncommitted plan edits are invisible to the brief. Each plan costs Git process starts. `pr-description` keeps the stale-link risk that its own D5 accepted. Because the command stays offline, published links work only when the `--head` commit is available in the named `--repo` repository. A wrong `--repo` or an unpushed commit yields a well-formed brief whose links do not resolve on GitHub. The command adds no GitHub API call, network check, or remote-discovery heuristic to close this gap.
- D4: The command is hidden with Clap `hide = true` and has no `Plan Usage:` line. Completion rendering rebuilds the `plan` command without its hidden subcommands, because `clap_complete` 4.6.7 emits hidden subcommands.
  - Boundary: `cli/src/cli_schema.rs` marks `PlanSubcommand::DecisionBrief` hidden. `cli/src/services/completion/mod.rs` removes hidden subcommands under `plan` only. Hidden `hooks` subcommands keep their current completion output.
  - Rationale: `hide = true` already removes the command from Clap help. `clap_complete::aot::generator::utils::subcommands` iterates every subcommand, so the completion command needs explicit pruning.
  - Rejected: Post-processing the generated script text — it depends on each shell's output shape. Pruning every hidden subcommand in the tree — it changes the completion output of the hidden `hooks` adapters, which this change does not own.
  - Trade-offs: The rebuilt `plan` command must copy the settings that completion uses. Hiding is discoverability only, so any user who knows the name can run the command.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: `pr_description::render` with `BriefFormat::DecisionBrief` prints the start marker, `## Decision Brief`, `**Review requested:** N decisions`, the `**Plans:**` link line, one `### N. {question}` subsection per marked decision with its `**Proposed:** {statement}` line and `[Review {label} {Dn} — rationale, alternatives and trade-offs]({url}#L{line})` link, and the end marker. Elements are separated by one blank line, and the output ends with exactly one newline.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::decision_brief_`
  - Decisions: D1, D2
  - Scenarios: S1, S2, S3
- [ ] AC2: Both commands select the same decisions. A decision is selected only when it carries `Review: required` and `Review question:`. Plans are sorted by path, decisions keep source order, and numbering starts at 1. Four or more marked decisions fail with exit `3`, name every marked decision, and print nothing on stdout. A marked decision with an empty statement fails the Decision Brief with exit `3`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::decision_brief_`
  - Decisions: D2
  - Scenarios: S4, S5, S6
- [ ] AC3: `sce plan decision-brief` parses and links the plan content at the `--head` commit, never the working tree. A missing commit, a missing blob, a non-regular tree entry, and non-UTF-8 content fail with exit `4`. Invalid arguments, duplicate paths, and structurally invalid plans fail with exit `3`. Stdout is empty on every failure.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::decision_brief_`
  - Decisions: D3
  - Scenarios: S7, S8
- [ ] AC4: `sce plan decision-brief` and `sce plan decision-brief --help` run directly. The command is absent from `sce --help`, from `sce plan --help`, from the custom `Plan Usage:` lines, and from the Bash, Zsh, and Fish completion scripts. The existing hidden `hooks` adapters keep their completion output.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::decision_brief_hidden`
  - Decisions: D4
  - Scenarios: S9, S10
- [ ] AC5: `sce plan pr-description` keeps its working-tree reads and its exact output, identical to the baseline implementation at `7cb9552f99e8d815850bdca8c89d0c68e8586004`. Every `plan::pr_description_` test that exists at that baseline passes with its original golden expectations unchanged. New Decision Brief tests may be added to `cli/src/services/plan/mod.rs`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::pr_description_`, and `git diff 7cb9552f99e8d815850bdca8c89d0c68e8586004 -- cli/src/services/plan/mod.rs` shows no removed or changed line inside an existing `pr_description_` test, including its expected strings. Added `decision_brief_` tests are allowed.
  - Decisions: D2
  - Scenarios: S11
- [ ] AC6: Every extracted entry carries a `statement` equal to its whitespace-folded opening paragraph after `ID:`. Entry order, lines, authoritative sections, and fenced-example handling are unchanged.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::extract`
  - Decisions: D1
  - Scenarios: S12
- [ ] AC7: The canonical `Design decision rules` tell plan authors to ask one specific accept-or-reject question about an architectural choice and its consequence, to mark only decisions that need senior judgment, and to not repeat a concern. The structural review-pair rules do not change.
  - Validate: inspection of `config/pkl/base/workflow-change-to-plan.pkl` under `## Design decision rules`, and `nix run .#pkl-check-generated`.
  - Scenarios: none (inspection)
- [ ] AC8: The dogfood brief for all five PR #342 plans, `plan-decisions-and-verification-scenarios.md`, `sce-plan-check.md`, `sce-plan-init.md`, `plan-pr-description.md`, and `plan-decision-brief.md`, at `git rev-parse HEAD` links all five plans and shows `**Review requested:** 3 decisions` with exactly three subsections in sorted plan order and source decision order: `plan-pr-description` D5 (line 31), `sce-plan-init` D4 (line 50), and `sce-plan-init` D6 (line 72). `plan-decision-brief.md` contributes no marked decision. Each decision link line is the line of that `- Dn:` definition in `git show <sha>:<path>`. `HEAD` must be a commit that contains the removal of the D3 and D4 review markers from `plan-decision-brief.md`; at `7ca0a187` the brief would have five marked decisions and fail the limit.
  - Validate: `nix run .#sce -- plan decision-brief --repo crocoder-dev/shared-context-engineering --head "$(git rev-parse HEAD)" context/plans/plan-decisions-and-verification-scenarios.md context/plans/sce-plan-check.md context/plans/sce-plan-init.md context/plans/plan-pr-description.md context/plans/plan-decision-brief.md`
  - Decisions: D3
  - Scenarios: S13
- [ ] AC9: `context/cli/plan-service.md` documents the hidden command, its reviewer audience, its exact output, its relation to `pr-description`, the three-question limit, commit-backed reads, its offline read-only behavior, that hiding is not access control, and that review intent is not verified implementation evidence. It also states the D3 link limitation: `--repo` is caller-supplied and `--head` must be a local commit; the command guarantees that parsed content and line numbers match the local commit, but it does not verify that the commit was pushed to GitHub or exists in `--repo`, so published links work only when that commit is available in the named repository.
  - Validate: inspection of the `Decision brief command contract` section in `context/cli/plan-service.md`.
  - Scenarios: none (inspection)

### Verification scenarios

- S1: Golden Decision Brief for two marked decisions
  - Given: One in-memory structured plan with two marked decisions, one of which has a wrapped opening paragraph.
  - When: `render` runs with `BriefFormat::DecisionBrief`.
  - Then: The output equals the expected string byte for byte, including both questions, both folded statements, both decision IDs, both `#L{line}` anchors, the blank lines, the markers, and one trailing newline.
  - Check: `plan::decision_brief_golden_two_decisions`, created by T02.
- S2: No marked decisions
  - Given: Valid plans with no `Review: required` decision.
  - When: `render` runs with `BriefFormat::DecisionBrief`.
  - Then: The output has `**Review requested:** 0 decisions`, the plans line, and `No decisions explicitly flagged for review.`, and it has no `###` subsection and no approval or safety claim.
  - Check: `plan::decision_brief_no_marked_decisions`, created by T02.
- S3: Markdown-sensitive text stays literal
  - Given: A marked decision whose question and statement contain `` ` * _ [ ] < > | \ `` characters.
  - When: `render` runs with `BriefFormat::DecisionBrief`.
  - Then: Each listed character has one backslash before it, and removing those backslashes gives the authored words.
  - Check: `plan::decision_brief_escapes_markdown`, created by T02.
- S4: Ordering and determinism
  - Given: Two plans given in reverse lexicographic order, each with marked decisions.
  - When: The brief is rendered twice with that order and once with the sorted order.
  - Then: The three outputs are byte-identical, plans are linked in sorted order, and questions follow sorted plan order and source decision order.
  - Check: `plan::decision_brief_orders_plans_deterministically`, created by T02.
- S5: Three-question limit
  - Given: Plans with exactly three marked decisions, and plans with four marked decisions.
  - When: The command runs on each set.
  - Then: Three succeed with three subsections. Four fail with exit `3`, the message states `4` and names all four `<label> <Dn>` pairs, and stdout is empty.
  - Check: `plan::decision_brief_question_limit`, created by T03.
- S6: Marked decision without a usable statement
  - Given: A marked decision whose definition line is `- D1:` with no text.
  - When: The Decision Brief renders, and `pr-description` renders the same plan.
  - Then: The Decision Brief fails with exit `3` and names the plan and decision. `pr-description` succeeds, and `sce plan check` still passes the plan.
  - Check: `plan::decision_brief_rejects_empty_statement`, created by T02.
- S7: Commit content wins over the working tree
  - Given: A temporary Git repository with a committed structured plan, whose working-tree copy later gains inserted lines and a changed question.
  - When: The command runs with the original commit SHA before and after the edit.
  - Then: Both outputs are byte-identical and show the committed question, statement, and line anchor.
  - Check: `plan::decision_brief_reads_committed_plan`, created by T03.
- S8: Invalid inputs fail without output
  - Given: An invalid `--repo`, a short and an uppercase `--head`, an unknown commit, a path absent at the commit, a path outside `context/plans/`, a duplicate path, a committed invalid plan, a committed symlink at a plan path, and committed non-UTF-8 plan bytes.
  - When: The command runs for each input.
  - Then: Argument, duplicate, and invalid-plan cases exit `3`. Missing-commit, missing-blob, symlink, and non-UTF-8 cases exit `4`. Stdout is empty in every case.
  - Check: `plan::decision_brief_rejects_invalid_git_inputs`, created by T03.
- S9: Direct invocation and help
  - Given: The built CLI parser.
  - When: `sce plan decision-brief --repo ... --head ... <path>` and `sce plan decision-brief --help` are parsed and run.
  - Then: The first prints a brief. The second prints help that names `--repo <OWNER/NAME>`, `--head <SHA>`, and `<PATHS>...`.
  - Check: `plan::decision_brief_hidden_but_directly_invocable`, created by T03.
- S10: Hidden from help and completions
  - Given: The root help text, `sce plan --help`, and the Bash, Zsh, and Fish completion scripts.
  - When: Each is rendered.
  - Then: None contains `decision-brief`. Each still contains `pr-description`, and the completion scripts still contain `claude-mutation-scope`.
  - Check: `plan::decision_brief_hidden_from_help_and_completions`, created by T03 for help and extended by T04 for completions.
- S11: Legacy intent review is unchanged
  - Given: The `plan::pr_description_` tests and their golden expected strings at baseline `7cb9552f99e8d815850bdca8c89d0c68e8586004`.
  - When: The tests run after the change.
  - Then: Every baseline test passes, no baseline test or expected string changed, and `sce plan pr-description` keeps the baseline output and working-tree reads. New `decision_brief_` tests in `mod.rs` are allowed.
  - Check: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::pr_description_`, and `git diff 7cb9552f99e8d815850bdca8c89d0c68e8586004 -- cli/src/services/plan/mod.rs`
- S12: Statement extraction
  - Given: A plan with a one-line decision, a wrapped decision paragraph, an empty `- D2:` definition, and a fenced `- D9:` example.
  - When: `extract` runs.
  - Then: The statements are the trimmed one-line text, the wrapped text joined by single spaces, and an empty string. No entry exists for `D9`, and entry order and lines are unchanged.
  - Check: `plan::extract::tests::definition_statement_is_folded_source_text`, created by T01.
- S13: Dogfood links resolve
  - Given: The five PR #342 plans, including `plan-decision-brief.md`, at `git rev-parse HEAD`, where `HEAD` contains the D3 and D4 review-marker removal.
  - When: The command runs on them.
  - Then: The plans line links all five plans. The brief reports `3 decisions` with subsections for `plan-pr-description` D5, `sce-plan-init` D4, and `sce-plan-init` D6, in that order. Each `#L{n}` anchor points at the `- Dn:` line in `git show <sha>:<path>`.
  - Check: the AC8 command, and `git show <sha>:<path> | sed -n '<n>p'` for each link.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::`
- `nix run .#pkl-check-generated`
- `nix flake check`
- `nix build .#default --no-link`

### Context sync

- `context/cli/plan-service.md`: the plan model `statement` field, the `BriefFormat` layouts, and the Decision Brief command contract.
- `context/cli/cli-command-surface.md`: the hidden `plan decision-brief` subcommand and the completion pruning under `plan`.
- `context/sce/shared-context-plan-workflow.md`: the review-question authoring guidance.
- `context/glossary.md` and `context/context-map.md`: the `Decision Brief` term and the updated plan-service annotation.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/plan/{extract,pr_description,command,mod}.rs`, `cli/src/cli_schema.rs`, `cli/src/services/parse/command_runtime.rs`, `cli/src/services/completion/mod.rs`, `config/pkl/base/workflow-change-to-plan.pkl`, and the context files under `Context sync`.
- **Out of scope:** Changes to `validate.rs` rules, the Pkl plan-structure checker, `sce plan check`, `sce plan init`, the `sce plan pr-description` output and its working-tree reads, and the hidden `hooks` completion output. Historical plans keep their content.
- **Constraints:** No new crate dependency, trait, `dyn`, async code, database, network call, GitHub API call, AI call, or file write. Git runs as a direct process at the repository root. The command is synchronous and offline. Every new test is in `cli/src/services/plan/` and runs under the `plan::` filter.
- **Non-goal:** A drill-down view of acceptance criteria, scenarios, and tasks, risk ranking, automatic PR description updates, and a general mechanism that hides commands from completions across the whole CLI.

## Assumptions

- The singular count line is `**Review requested:** 1 decision`. Every other count uses `decisions`, as the request shows for 0 and 2.
- Statements and questions use the existing escaping contract, so inline code such as `` `sce plan init` `` renders as literal backticks. The request's section 10 requires this, although its layout example shows unescaped backticks.
- The four-or-more error reuses the existing `pr-description` message, because selection and the limit are shared.
- A non-regular tree entry at a plan path, such as a symlink or a submodule, is a runtime failure with exit `4`, like a missing blob. Non-UTF-8 committed content is also exit `4`, like an unreadable working-tree plan.
- `nix run .#sce` is the CLI that runs the dogfood command, because the host `sce` on `PATH` predates the `plan` command.

## Task stack

- [x] T01: `Extract the definition statement into PlanEntry` (status:done)
  - Task ID: T01
  - Covers: AC6
  - Scope: In — `statement: String` on `PlanEntry`, set in `extract::definition` from the `parse_id_token` remainder with whitespace folded, the S12 unit test, and updates to existing tests and callers that build or compare `PlanEntry`. Out — renderer changes and validation rules.
  - Dependencies: none
  - Done when: Every entry carries its folded statement, the S12 test passes, and every existing `plan::extract` and fixture-manifest test passes unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::`
  - Completed: 2026-10-24
  - Files changed: `cli/src/services/plan/extract.rs`
  - Result: `PlanEntry` gains `statement: String`, which `extract::definition` sets from the `parse_id_token` remainder by folding whitespace with `split_whitespace().join(" ")`. Added `definition_statement_is_folded_source_text`, which covers a one-line statement, a wrapped statement with a tab, an empty `- D2:`, and a fenced `- D9:`. No other code builds `PlanEntry`, and no existing test changed.
  - Verify outcome: `plan::` passed: 38 passed, 0 failed, including the new test, every `plan::extract` test, and `fixture_expectations_match_manifest`.
  - Context impact: localized. The plan model gains a `statement` field that later tasks consume. `context/cli/plan-service.md` documents this field in T06.
  - Context synchronization: synced

- [ ] T02: `Render the Decision Brief layout through BriefFormat` (status:todo)
  - Task ID: T02
  - Covers: AC1, AC2, AC5
  - Scope: In — `BriefFormat::{IntentReview, DecisionBrief}` in `pr_description.rs`, one shared selection pass that carries the statement, the Decision Brief layout, the empty-statement validation error, the `pr-description` call site passing `IntentReview`, and the S1, S2, S3, S4, and S6 tests. Out — Git reads, CLI wiring, and help or completion changes.
  - Dependencies: T01
  - Done when: The new render tests pass, and every `plan::pr_description_` test from baseline `7cb9552f99e8d815850bdca8c89d0c68e8586004` passes with its original golden expectations unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::`, and `git diff 7cb9552f99e8d815850bdca8c89d0c68e8586004 -- cli/src/services/plan/mod.rs` shows no changed or removed line in an existing `pr_description_` test.
  - Context synchronization: pending

- [ ] T03: `Add the hidden commit-backed plan decision-brief command` (status:todo)
  - Task ID: T03
  - Covers: AC2, AC3, AC4, AC8
  - Scope: In — the hidden `PlanSubcommand::DecisionBrief` Clap variant, its `command_runtime.rs` mapping, a `PlanRequest::DecisionBrief` variant, the D3 Git reads, reuse of the shared extract, validate, and diagnostic path, and the S5, S7, S8, S9, and S10 help tests. Out — completion pruning and documentation.
  - Dependencies: T02
  - Done when: The command prints the brief from committed content, every S8 case fails with the stated exit code and empty stdout, the command is absent from `sce --help` and `sce plan --help`, and the AC8 dogfood command over all five plans prints the expected three-decision brief.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::`, and the AC8 dogfood command.
  - Context synchronization: pending

- [ ] T04: `Omit hidden plan subcommands from completion scripts` (status:todo)
  - Task ID: T04
  - Covers: AC4
  - Scope: In — `render_completion` rebuilds the `plan` subcommand without its hidden subcommands, and the S10 completion assertions for Bash, Zsh, and Fish. Out — hidden subcommands of other commands.
  - Dependencies: T03
  - Done when: No completion script contains `decision-brief`, each still contains `pr-description` and `claude-mutation-scope`, and the existing completion tests pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml completion`, and `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::`
  - Context synchronization: pending

- [ ] T05: `Add review-question authoring guidance to the plan template rules` (status:todo)
  - Task ID: T05
  - Covers: AC7
  - Scope: In — the `## Design decision rules` text in `config/pkl/base/workflow-change-to-plan.pkl`, with the request's preferred and rejected question examples. Out — new plan fields, structural rule changes, and edits to existing plans.
  - Dependencies: none
  - Done when: The rules state the AC7 guidance, and the generated-output parity check passes.
  - Verify: `nix run .#pkl-check-generated`
  - Context synchronization: pending

- [ ] T06: `Document the Decision Brief command contract` (status:todo)
  - Task ID: T06
  - Covers: AC9
  - Scope: In — a `Decision brief command contract` section with the D3 local-integrity versus remote-link-availability limitation, the `statement` field, `BriefFormat`, and the new tests in `context/cli/plan-service.md`. Out — context files that this change does not affect.
  - Dependencies: T03, T04
  - Done when: The section records every AC9 item, and the plan model and Tests sections name the new field and tests.
  - Verify: inspection of `context/cli/plan-service.md`.
  - Context synchronization: pending

## Open questions

- The escaping contract turns inline code in proposed statements into literal backticks, for example `` \`sce plan init\` ``. This plan follows the request's section 10 escaping rule, not its layout example. Inline code can stay intact if statements keep their authored Markdown, but authored brackets could then create links.
