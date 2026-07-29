# Plan: shorten-generated-workflow-docs

## Change summary

The four generated workflow packages (`sce-change-to-plan`, `sce-next-task`,
`sce-validate`, `sce-commit`) carry roughly 295 lines of duplicated and
vestigial text per target. Three distinct causes: `SKILL.md` inlines the same
fenced user-visible layouts that `references/output.md` already owns; the
validate package's `references/output.md` states its blocked and failed layouts
twice, once as workflow-level layouts and once as the phase's return-value
contract; and inlining the phase skills left behind bare frontmatter fragments,
`Return a result matching: <no-op reference>` sentences, and references to
`validation-report.md` / `validation-result.md`, files that no longer exist in
any package.

This removes all three without dropping a single instruction. Every layout
`SKILL.md` currently inlines has a named `references/output.md` section, so the
step bodies cite that section instead of restating it — which is what
`context/patterns.md` already requires ("put every and only human-visible gate,
report, and terminal response layout in `output.md`"). The change is confined to
canonical Pkl under `config/pkl/`; generated targets stay ephemeral and are
verified through ephemeral generation.

## Acceptance criteria

- [ ] AC1: No composite `SKILL.md` contains a fenced user-visible layout block
      that duplicates a `references/output.md` section; each step that produces
      user-visible output names the `references/output.md` section it renders.
  - Validate: generate with `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"`,
    then for all 12 generated workflow `SKILL.md` files confirm no fenced block
    reproduces an `output.md` layout, and that every branch producing output
    names its section by heading.
- [ ] AC2: Each workflow's `references/output.md` states every layout exactly
      once, and references no file outside its own package.
  - Validate: same generated payload; confirm `sce-validate/references/output.md`
    has one `blocked` layout, one `failed` handoff layout, and one completion
    layout, and that no package document mentions `validation-report.md` or
    `validation-result.md`.
- [ ] AC3: No composite `SKILL.md` contains embedded document frontmatter
      fragments or reference sentences that resolve to no document.
  - Validate: same generated payload; confirm no workflow `SKILL.md` contains a
    bare `name:` / `description: >` / `argument-hint:` line outside its own
    frontmatter block, and no `Return a result matching:` or
    `must return a result matching its ... contract` sentence remains.
- [ ] AC4: The four Claude workflow packages total at most 3,520 lines, down
      from 3,803, with the same steps, gates, branches, waits, and prohibitions
      as before.
  - Validate: `wc -l` over the generated `.claude/skills/sce-*/SKILL.md` plus
    `references/output.md` in the temp generation root; read the diff of each
    package against a pre-change generation root to confirm only duplicated or
    dead text was removed.
- [ ] AC5: The generation contract is unchanged — same 46 artifact paths,
      metadata, inventory, and forbidden-path assertions.
  - Validate: `nix run .#pkl-check-generated`

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/patterns.md` (Pkl renderer layering: composite embedding omits
  document frontmatter; `SKILL.md` cites `output.md` sections rather than
  inlining layouts)
- `context/architecture.md` (`workflow-content.pkl` and
  `workflow-composite.pkl` composition description)
- `context/sce/shared-context-plan-workflow.md`,
  `context/sce/shared-context-code-workflow.md`,
  `context/sce/atomic-commit-workflow.md` (only if a phase boundary description
  changes; the intent is that none does)

## Constraints and non-goals

- **In scope:** `config/pkl/base/workflow-content.pkl`,
  `config/pkl/base/workflow-{change-to-plan,next-task,validate,commit}.pkl`,
  `config/pkl/base/workflow-context-sync.pkl`, and
  `config/pkl/renderers/workflow-composite.pkl`.
- **Out of scope:** the committed root `.claude/`, `.opencode/`, and `.pi/`
  installed trees; the Rust CLI; `workflow-catalog.pkl` identity metadata;
  target metadata renderers; OpenCode routing agents; the no-improvisation
  preamble; and the two context-sync phase bodies' internal duplication (source
  duplication in `workflow-context-sync.pkl` that does not shorten any rendered
  file).
- **Constraints:** edit canonical Pkl only and verify through ephemeral
  generation. The byte-identity regression guard in `context/patterns.md` does
  not apply here — generated output must change — so each task reviews its
  generated diff against a pre-task generation root instead. Every step, gate,
  branch, wait, prohibition, and continuation must survive: this plan removes
  duplicate and dead text only, never a rule.
- **Non-goal:** rewriting or condensing instructions that appear once. Items 4-8
  of the source analysis (duplicated ownership lists, `## Completion` sections
  restating their own steps, the two prohibition lists, the plan template's
  repeated task rules, per-phase boilerplate) are deliberately excluded from
  this plan.

## Assumptions

- `references/output.md` is the authority and `SKILL.md` is the citation side,
  per `context/patterns.md`'s two-file rule, rather than the reverse.
- Refreshing the committed root `.claude/`, `.opencode/`, and `.pi/` trees is
  handled by the repository's existing separate regeneration step, as in commits
  `bc09266` and `4133819`, and is not part of any task here.
- Dropping the embedded `description:` / `argument-hint:` command fragment loses
  no information, because each workflow's `## Input` section states its
  arguments more precisely than the hint does.

## Task stack

- [x] T01: `Stop embedding document frontmatter in composite rendering` (status:done)
  - Task ID: T01
  - Goal: Composite-mode rendering of an embedded command or phase document emits
    only its body, so no generated `SKILL.md` carries bare `name:`,
    `description: >`, or `argument-hint:` fragments as prose.
  - Boundaries (in/out of scope): In — the composite branch of
    `StructuredWorkflowDocument.render` and the now-unused
    `WorkflowFrontmatter.renderEmbedded` in
    `config/pkl/base/workflow-content.pkl`, plus any spacing adjustment needed in
    `renderStructuredPhase` / `renderCanonicalWorkflow` in
    `config/pkl/renderers/workflow-composite.pkl`. Out — package-mode rendering,
    the skill entrypoint's own frontmatter, phase body text, and
    `phaseNameBySlug`.
  - Dependencies: none
  - Done when: all 12 generated workflow `SKILL.md` files contain no bare
    frontmatter line outside their own leading `---` block; each
    `## Internal phase:` heading is followed directly by that phase's `#` title;
    `## Canonical workflow` is followed directly by the command body; package-mode
    skill frontmatter is unchanged; `nix run .#pkl-check-generated` and
    `nix flake check` pass.
  - Verification notes (commands or checks):
    `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"`; grep the 12
    generated `SKILL.md` files for `^(name|argument-hint|description): ` outside
    frontmatter; diff against a pre-task generation root to confirm only the
    fragments were removed; `nix run .#pkl-check-generated`; `nix flake check`.
  - Evidence: `config/pkl/base/workflow-content.pkl` — the composite branch of
    `StructuredWorkflowDocument.render` now returns `body.render.apply(mode)`
    alone, and `WorkflowFrontmatter.renderEmbedded` was deleted along with its
    stale doc comment. `config/pkl/renderers/workflow-composite.pkl` needed no
    change: the phase and command bodies already begin at their own heading, so
    `renderStructuredPhase` and `renderCanonicalWorkflow` produce correct
    spacing unchanged.
  - Verification: generated a pre-change and a post-change root with
    `nix run .#pkl-generate`. `diff -r` shows 12 differing files (the four
    workflow `SKILL.md` files across `.claude/`, `.opencode/`, `.pi/`), 318
    lines removed and 0 lines added. An awk scan over the 12 files finds no
    `name:` / `description:` / `argument-hint:` line outside the leading `---`
    block; every `## Internal phase:` heading is followed by a blank line then
    that phase's `#` title, and every `## Canonical workflow` heading by the
    `SCE ...` command line. No package-mode document changed.
    `nix run .#pkl-check-generated` passed (46 files, inventory sha256
    `0e040dd7…`); `nix flake check` passed (all checks).
  - Deviation: the committed root `.claude/`, `.opencode/`, and `.pi/` trees
    were refreshed during this task rather than by the plan's assumed separate
    regeneration step — their 12 `SKILL.md` files now show the same 318-line
    deletion and were verified byte-identical to the ephemeral generation root.
    They were not hand-edited, and no canonical Pkl outside the task's declared
    scope was touched.

- [x] T02: `State each validate output layout exactly once` (status:done)
  - Task ID: T02
  - Goal: `sce-validate/references/output.md` carries one blocked layout, one
    failed-handoff layout, and one completion layout, with no reference to the
    removed `validation-report.md` or `validation-result.md` files.
  - Boundaries (in/out of scope): In — `VALIDATE_OUTPUT` in
    `config/pkl/renderers/workflow-composite.pkl` and the validation-result
    document rendered by `renderValidationResult` in
    `config/pkl/base/workflow-validate.pkl`, including its dead cross-file
    references. Out — the plan-context-sync report, the plan-file validation
    report format, the validation phase body, and the other three workflows'
    output layouts.
  - Dependencies: none
  - Composition note: the fuller variants (which carry the `validated` variant,
    context impact, and report rules) are the ones to keep; the condensed
    duplicates in `VALIDATE_OUTPUT` are the ones to drop, preserving
    `VALIDATE_OUTPUT`'s unique `Context synchronization blocked` and `Completion`
    sections.
  - Done when: generated `sce-validate/references/output.md` contains exactly one
    `Status: blocked` layout and one `Status: failed` handoff layout; every
    `validated`, `failed`, and `blocked` field and report rule present before the
    change is still present; the file no longer mentions `validation-report.md`
    or `validation-result.md`; the file is at least 100 lines shorter;
    `nix run .#pkl-check-generated` and `nix flake check` pass.
  - Verification notes (commands or checks):
    `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"`; `wc -l` and read
    the generated `sce-validate/references/output.md`; diff its section inventory
    against the pre-task version to prove no unique section or rule was lost;
    `nix run .#pkl-check-generated`; `nix flake check`.
  - Evidence: `config/pkl/renderers/workflow-composite.pkl` — `VALIDATE_OUTPUT`
    lost its condensed `## Validation blocked` and `## Validation failed handoff`
    sections; a two-line pointer now states that both layouts live once under
    **Validation Result**, and its unique `## Context synchronization blocked` and
    `## Completion` sections are unchanged.
    `config/pkl/base/workflow-validate.pkl` — the paragraph separating the
    plan-file report from the returned result became one mode-aware
    `reportVersusResult` reference, so composite mode names the embedded
    **Plan-file validation report** section and `references/output.md` instead of
    the removed `validation-report.md` / `validation-result.md` files. Package-mode
    text keeps the literal filenames it still owns.
  - Verification: `nix run .#pkl-generate` into pre- and post-change roots.
    `diff -r` shows six differing files (the `sce-validate` `SKILL.md` and
    `references/output.md` across `.claude/`, `.opencode/`, `.pi/`).
    `sce-validate/references/output.md` went 429 -> 356 lines; it now holds
    exactly one `Validation blocked` layout and one `Validation failed — handoff`
    layout (the only other `**Status:** blocked` line belongs to the separate Plan
    Context Sync Report), plus the single `validated` variant with its
    **Context impact** block and all ten report rules. A sorted line diff confirms
    every field, placeholder, and rule dropped from `VALIDATE_OUTPUT` still appears
    in the retained fuller variants; `Omit inapplicable optional sections` is
    covered by the variants' per-section omit notes and the `Omit empty optional
    sections` rule. No generated workflow document mentions `validation-report.md`
    or `validation-result.md`. `nix run .#pkl-check-generated` passed (46 files,
    inventory sha256 `2dc33467…`); `nix flake check` passed (all checks).
  - Deviation: the done check "at least 100 lines shorter" was met only to 73
    lines (429 -> 356). The available duplication was the two condensed layouts
    (76 lines) minus the two-line pointer that replaced them, plus the reworded
    reference paragraph; cutting to 100 would have required removing text that
    appears exactly once, which the plan's constraints forbid. Approved at the
    implementation gate with the reduced figure. As in T01, the committed root
    `.claude/`, `.opencode/`, and `.pi/` trees were refreshed here rather than by a
    separate regeneration step, and verified byte-identical to the ephemeral
    generation root. Beyond the stated boundary, the same class of dead
    cross-file reference inside the plan-file validation report document was also
    made mode-aware, because AC2 requires no package document to mention either
    removed filename and no other task covers that sentence.

- [x] T03: `Cite output.md sections instead of inlining layouts in SKILL bodies` (status:done)
  - Task ID: T03
  - Goal: Every workflow step that produces user-visible output names the
    `references/output.md` section it renders, and no `SKILL.md` reproduces that
    section's fenced block or prose restatement.
  - Boundaries (in/out of scope): In — the `COMMAND` bodies in all four
    `config/pkl/base/workflow-{change-to-plan,next-task,validate,commit}.pkl`
    modules: the bootstrap, clarification, and ready-continuation blocks; the
    plan-complete, declined, blocked-or-incomplete, sync-blocked,
    more-tasks-remain, and all-tasks-complete blocks; the validate completion
    block and sync-blocked prose; and the commit staging-gate, no-staged-changes,
    and bypass-result blocks. Out — operational instructions that are not
    layouts (field mapping, singular/plural rules, branch conditions, waits, and
    every prohibition), `references/output.md` bodies, and phase document bodies.
  - Dependencies: T02
  - Done when: each removed block is replaced by a citation naming its
    `output.md` section heading; the set of branches that produce output is
    unchanged; every non-layout instruction that accompanied a removed block
    (question rendering order, `task` vs `tasks`, `is ready` vs `revised`, stop
    and wait points) is preserved; the four Claude `SKILL.md` files are at least
    80 lines shorter in total; `nix run .#pkl-check-generated` and
    `nix flake check` pass.
  - Verification notes (commands or checks):
    `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"`; for each of the 12
    `SKILL.md` files confirm every citation resolves to a real heading in the
    sibling `references/output.md`; diff each against the pre-task root to
    confirm only layout text was removed; `nix run .#pkl-check-generated`;
    `nix flake check`.
  - Evidence: each in-scope layout became a `model.semanticReference` in its
    module, so `packageText` keeps the inline block a command file still owns and
    `compositeText` cites the `output.md` section by heading.
    `workflow-change-to-plan.pkl` — `bootstrapLayout`, `clarificationLayout`,
    `authoringBlockedBranch`, `readyContinuationLayout`.
    `workflow-next-task.pkl` — `reviewBlockedBranch`, `planCompleteBranch`,
    `declinedBranch`, `executionBlockedBranch`, `syncBlockedBranch`,
    `continuationLayouts`. `workflow-validate.pkl` — `syncBlockedBranch`,
    `completionLayout`. `workflow-commit.pkl` — `noStagedChangesLayout`,
    `stagingGateLayout`, `bypassResultLayouts`, `bypassBlockedBranch`,
    `regularBlockedBranch`, `proposalBranch`. Field mapping (`candidates`,
    `executable_tasks_remaining`, `issues`), branch conditions, waits, and every
    prohibition stayed at the branch, as did the two sentences `output.md` does
    not state ("Nothing records the skipped synchronization, so it is lost once
    this session ends" in next-task and validate).
  - Verification: `nix run .#pkl-generate` into pre- and post-change roots.
    `diff -r` shows exactly 12 differing files (the four workflow `SKILL.md`
    files across `.claude/`, `.opencode/`, `.pi/`); no `references/output.md`
    changed. The four Claude `SKILL.md` files went 2,750 -> 2,602 lines, 148
    shorter against the 80-line check; the four Claude packages total 3,476.
    Every `Render the **X** layout` citation in all 12 files resolves to a real
    `^## X` heading in its sibling `references/output.md`. The remaining fenced
    blocks in the canonical-workflow sections are the plan template, the task
    evidence example, the validation-report layout, and the two handoff
    status snippets — none is an `output.md` layout. A scan for `Present`,
    `Return:`, `stop with`, and `prompt the user` over the canonical-workflow
    sections finds no output-producing branch left uncited.
    `nix run .#pkl-check-generated` passed (46 files, inventory sha256
    `4cc61a02…`); `nix flake check` passed (all checks).
  - Deviation: commit's `blocked` (both paths) and `proposal` branches are not in
    the task's enumerated block list, but the task goal requires every branch that
    produces output to name its section, so they gained citations. Their prose was
    replaced by an equivalent citation, not deleted wholesale. As in T01 and T02,
    the committed root `.claude/`, `.opencode/`, and `.pi/` trees were refreshed
    here rather than by a separate regeneration step, and verified byte-identical
    to the ephemeral generation root.

- [x] T04: `Remove reference sentences that resolve to no document` (status:done)
  - Task ID: T04
  - Goal: No workflow document contains a `Return a result matching:`,
    `must return a result matching its ... contract`, or
    `Use the document format defined in:` sentence whose target is a
    self-reference or a document that does not exist.
  - Boundaries (in/out of scope): In — those sentences and their semantic
    references in `config/pkl/base/workflow-{change-to-plan,next-task,validate,commit}.pkl`
    and `config/pkl/base/workflow-context-sync.pkl`, plus any
    `workflow.semanticReference` binding left unused by their removal. Out —
    references that resolve to a real sibling section or package file (for
    example the `Plan template` and `Plan-file validation report` internal
    documents, and `references/output.md`), the `### N. Return the result` steps
    that enumerate the internal states, and the `## Completion` sections.
  - Dependencies: T03
  - Done when: no generated workflow document contains a reference sentence
    naming `the internal ... state described by this workflow` or a
    non-existent file; every phase still names its terminal internal states in
    its own return step; unused semantic-reference bindings are deleted rather
    than left dangling; the four Claude packages total at most 3,520 lines;
    `nix run .#pkl-check-generated` and `nix flake check` pass.
  - Verification notes (commands or checks):
    `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"`; grep the 12
    `SKILL.md` files for `Return a result matching`,
    `result matching its`, `described by this workflow`, and
    `Use the document format defined in`; `wc -l` the four Claude packages;
    `nix run .#pkl-check-generated`; `nix flake check`.
  - Evidence: every dangling contract sentence became a package-only
    `model.semanticReference` whose composite text is empty or the surviving
    remainder, and each block-shaped reference owns its surrounding blank lines so
    composite leaves no stray blank line. The five self-referential bindings
    (`contextBrief`, `authoringContract`, `readinessContract`, `executionContract`,
    `commitContract`) were deleted, as was `yamlResult` in `workflow-commit.pkl`,
    left unused by the completion-bullet rewrite.
    `workflow-change-to-plan.pkl` — `contextBriefHandoffSentence`,
    `contextBriefBlock`, `contextBriefClause`, `authoringHandoffSentence`,
    `authoringContractBlock`, `authoringContractClause`.
    `workflow-next-task.pkl` — `readinessHandoffSentence`,
    `readinessContractBlock`, `readinessCompletionBullet`,
    `executionContractBlock`, `blockerCategorySentence`,
    `executionContractClause`, `executionCompletionBullet`.
    `workflow-commit.pkl` — `commitContractHandoff` (keeping the surviving
    "Branch on `status`:" instruction), `commitContractBlock`,
    `commitContractClause`, `commitCompletionBullet`. `workflow-validate.pkl` —
    `validationResultHandoff` (keeping "Branch on the report's `Status:`.").
    `workflow-context-sync.pkl` needed no change: its only such sentence is `Use
    the report format in: references/output.md`, which resolves.
  - Verification: `nix run .#pkl-generate` into pre- and post-change roots.
    `diff -r` shows exactly 12 differing files (the four workflow `SKILL.md`
    files across `.claude/`, `.opencode/`, `.pi/`); no `references/output.md`
    changed. Every hunk is a removal of a sentence or block that named a
    non-existent target — nothing else moved. A grep over the 12 files for
    `Return a result matching`, `result matching its`,
    `described by this workflow`, and `Use the document format defined in` leaves
    only the two out-of-scope resolving references: change-to-plan's "Use the
    document format defined in" pointing at the embedded **Plan template**
    section, and validate's "Return a result matching" pointing at
    `references/output.md`. No generated
    document mentions any `references/*-contract.yaml`, `context-brief.yaml`,
    `validation-report.md`, or `validation-result.md`. The four Claude packages
    total 3,441 lines (2,567 in `SKILL.md`) against the 3,520 ceiling.
    `nix run .#pkl-check-generated` passed (46 files, inventory sha256
    `3626c61b…`); `nix flake check` passed (all checks).
  - Deviation: the done check "every phase still names its terminal internal
    states in its own return step" holds for five of the six phases. The task
    execution phase's `### 9. Return internal state` now reads "set exactly one
    internal state." with no list; its four states are named one step earlier in
    `### 8. Determine the terminal status`. That step already carried the
    enumeration before this change — the removed clause pointed at
    `references/execution-contract.yaml`, which named nothing in composite mode —
    so adding a second list would duplicate text that appears once, which the
    plan's constraints forbid. For the same reason `Use a blocker category
    defined by ...` was dropped rather than rewritten: step 8 already enumerates
    the blocked reasons in prose. Two `## Completion` bullets and one return-step
    sentence were reworded, which the boundary lists as out of scope; the done
    check's ban on `the internal ... state described by this workflow` reaches
    into them, so the dangling `matching ...` clause was dropped while the
    sections' structure and state enumerations stayed intact. As in T01–T03, the
    committed root `.claude/`, `.opencode/`, and `.pi/` trees were refreshed here
    rather than by a separate regeneration step, and verified byte-identical to
    the ephemeral generation root. `invokeLower` in `workflow-next-task.pkl` is
    unused but was left alone: T03 stranded it, not this task.

## Open questions

- Item 1 (T03) trades a literal block at the branch point for a pointer to
  `references/output.md`. That is what the two-file rule in
  `context/patterns.md` mandates, and the preamble already says to use
  `output.md` for every gate — but an agent that sees the literal layout
  inline probably reproduces it more faithfully than one that must go read the
  sibling file mid-branch. I think the dedup is still right, because two copies
  that can drift is the worse failure, and the composite preamble plus the
  no-improvisation rule already point at `output.md`. If you would rather keep
  the layouts inline and instead shrink `references/output.md` to a rules-only
  file, say so and T03 inverts.
- Nothing prevents item 1 from regressing: `generation-contract-check.pkl`
  asserts paths, metadata, and stale slugs, not "no `SKILL.md` reproduces an
  `output.md` layout". A contract check for that is the natural guard and would
  be roughly one predicate over the generated documents, but it is scope beyond
  items 1-3, so no task authors it. Worth a follow-up plan.
- Unrelated drift the context brief turned up, not scheduled here:
  `context/patterns.md` still says to use the project-root `.pi/` workflows as
  the behavioral baseline for canonical workflow packages, but `.pi/prompts/*.md`
  are now 8-line thin prompts and `.pi/skills/` holds generated installed
  copies, so no baseline lives there any more.
