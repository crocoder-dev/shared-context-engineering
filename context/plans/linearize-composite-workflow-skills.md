# Plan: linearize-composite-workflow-skills

## Change summary

Each generated workflow `SKILL.md` currently reads as two documents stapled
together: a `## Canonical workflow` section whose steps say "Run the **Context
load phase**", followed by a `## Embedded phase behavior` appendix that defines
those phases under headings that contradict themselves — `## Internal phase:
Context load phase` immediately followed by `# SCE Context Load`, an `h1` nested
inside an `h2`. The result is that `sce-next-task/SKILL.md` carries four `##
Purpose` sections, four `## Input` sections, four `## Workflow` sections and
three `#` titles below its own, every phase reference points forward past a
hundred-plus lines to a section named differently from the reference, and the
phase bodies still address a standalone skill that no longer exists ("This skill
owns:", "The invoking workflow provides:", "Bootstrapping is the invoking
workflow's decision, not this skill's.").

This makes each composed skill one document that reads top to bottom. Every
phase body moves inside the numbered workflow step that runs it, with its steps
renumbered as `#### {step}.{n}` beneath that step; the `## Canonical workflow`
wrapper and the `## Embedded phase behavior` appendix both disappear, so the
workflow's own `## Input`, `## Workflow`, and `## Rules` become the skill's only
sections at that level. Parts that carry nothing once inlined are dropped rather
than relocated: each phase's `# SCE X` title, its `## Purpose` ownership list
(the step already states what the phase owns), its `## Input` provider list (the
step already states what it is run with), and its `## Completion` section (which
restates the phase's own steps). Substantive `## Input` prose is folded into the
step, and each phase's `## Boundaries` and `## Tone` sections survive as
subsections of the step. Persisted-document formats — the plan template and the
plan-file validation report — stay as trailing appendices, correctly leveled and
placed after `## Rules`.

The change is confined to canonical Pkl under `config/pkl/`. Generated targets
stay ephemeral and are verified through ephemeral generation and a reviewed diff
against a pre-task generation root.

AC4's own validation greps the 12 `SKILL.md` files *and* their `references/output.md`
siblings, and four stale sentences already live inside `output.md`-defining string
constants of three already-in-scope `.pkl` files — `workflow-change-to-plan.pkl`
("The invoking workflow renders it..." in the Plan Summary layout description),
`workflow-validate.pkl` and `workflow-context-sync.pkl` (a shared Notes-section
placeholder saying "...the invoking workflow should retain", and one line saying
"This skill is not run for `failed` or `blocked`..."). These are the same class of
stale standalone-skill phrasing this plan already rewrites in `SKILL.md`, just
inside an `output.md` template string instead. Fixing the four sentences is a
one-line reword apiece — it changes no layout, field, or rule `output.md` states —
so this plan now scopes that narrow correction in alongside AC4 rather than leaving
AC4 unsatisfiable against its own stated non-goal.

## Acceptance criteria

- [x] AC1: Every generated workflow `SKILL.md` states each phase's instructions
      inside the numbered workflow step that runs it. No `## Canonical
      workflow`, `## Embedded phase behavior`, or `## Internal phase:` heading
      remains, and no phase title heading remains.
  - Validate: generate with `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"`,
    then confirm none of the 12 generated workflow `SKILL.md` files matches
    `^## (Canonical workflow|Embedded phase behavior|Internal phase:)` or
    `^# SCE (Context Load|Plan Authoring|Plan Review|Task Execution|Task Context Sync|Validation|Plan Context Sync|Atomic Commit)`.
- [x] AC2: Every generated workflow `SKILL.md` has exactly one `#` heading, no
      heading-level skip, and no heading text repeated at the same level — one
      `## Purpose`, one `## Input`, one `## Workflow`, one `## Rules` per file.
  - Validate: same generated payload; an awk pass over the 12 files reporting,
    per file, the count of `^# `, any heading whose level exceeds its
    predecessor's by more than one, and any duplicated `level + text` pair.
- [x] AC3: No phase reference points forward. Where a phase is named rather than
      inlined (a re-run site such as context load after bootstrap or plan
      authoring on revision), the step that holds its instructions appears
      earlier in the same file.
  - Validate: same generated payload; for every `**{Phase} phase**` mention in
    each `SKILL.md`, confirm the inlined step heading carrying that phase's
    instructions occurs at a lower line number.
- [x] AC4: No generated workflow document attributes phase behavior to a
      separate skill or to an invoking workflow, and no sentence begins with a
      lowercase phase reference.
  - Validate: same generated payload; grep the 12 `SKILL.md` files and their
    `references/output.md` siblings for `invoking workflow`, `This skill`,
    `this skill's`, `The skill is complete after`, `Internal SCE workflow
    skill`, and `^the \*\*` — the only permitted `skill` mentions are the
    entrypoint's own frontmatter and the preamble's "Do not invoke another SCE
    skill" prohibition. This includes the four sentences named above the
    acceptance-criteria list, which T06 rewords.
- [x] AC5: Every step, gate, branch, wait, prohibition, verification instruction,
      and continuation that exists before the change still exists after it. Only
      titles, ownership restatements, provider lists, and completion recaps are
      removed.
  - Validate: for each of the four workflows, diff its generated `SKILL.md`
    against a pre-change generation root and confirm every removed line is a
    heading, a dropped `## Purpose` / `## Input` / `## Completion` line, or text
    restated at the step that now holds the phase.
- [x] AC6: The generation contract is unchanged — same 46 artifact paths,
      metadata, inventory, and forbidden-path assertions.
  - Validate: `nix run .#pkl-check-generated`

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/patterns.md` (Pkl renderer layering: composite embedding inlines each
  phase body at its step through typed heading scaling, emits no phase appendix,
  and states every heading once per document)
- `context/architecture.md` (`workflow-content.pkl` / `workflow-composite.pkl`
  composition description)
- `context/sce/shared-context-plan-workflow.md`,
  `context/sce/shared-context-code-workflow.md`,
  `context/sce/atomic-commit-workflow.md` (only where they describe the composed
  document's shape; no phase boundary changes)

## Constraints and non-goals

- **In scope:** `config/pkl/base/workflow-content.pkl`,
  `config/pkl/base/workflow-{change-to-plan,next-task,validate,commit}.pkl`,
  `config/pkl/base/workflow-context-sync.pkl`, and
  `config/pkl/renderers/workflow-composite.pkl`.
- **Out of scope:** the committed root `.claude/`, `.opencode/`, and `.pi/`
  installed trees; `references/output.md` bodies and the layouts they own,
  **except** the four stale standalone-skill sentences named above the
  acceptance-criteria list, which T06 rewords without changing any layout,
  field, or rule; the Rust CLI; `workflow-catalog.pkl` identity metadata; target
  metadata renderers; OpenCode routing agents; and the shared no-improvisation
  preamble's content.
- **Constraints:** edit canonical Pkl only and verify through ephemeral
  generation. Heading levels and inline-versus-name placement must be expressed
  as typed mode-aware values, never by parsing or rewriting Markdown markers —
  `context/patterns.md` forbids post-processing. Package-mode rendering must keep
  compiling and stay a coherent standalone document. The byte-identity guard does
  not apply: generated output must change, so each task reviews its generated
  diff against a pre-task generation root. Every rule, gate, wait, and
  prohibition must survive; this plan moves and deletes restated text, never an
  instruction. T06's four rewords must not change any `output.md` layout,
  placeholder, field name, or report rule — only the stale "invoking workflow" /
  "This skill" phrasing itself.
- **Non-goal:** removing the unconsumed package-mode surface (see Open
  questions), condensing instructions that appear exactly once, and rewriting
  `references/output.md` beyond T06's four named sentences.

## Assumptions

- Phase prose that reads correctly in both modes is changed once rather than
  split into a mode-aware pair: a phase package document saying "This phase
  owns" instead of "This skill owns" is correct in package mode too. Only
  heading level and inline-versus-name placement need both spellings.
- Inlined phase steps are renumbered `{parent step}.{n}` (`#### 2.3`), which
  reads unambiguously beneath `### 2. Author the plan` and keeps the workflow's
  own step numbers stable, so existing "as in step 2" cross-references stay
  valid.
- The plan template and plan-file validation report remain appendices rather than
  being inlined, because each is referenced from more than one step and is a
  persisted-file format rather than an instruction sequence.

## Task stack

- [x] T01: `Inline phase bodies structurally in composite rendering` (status:done)
  - Task ID: T01
  - Goal: `workflow-content.pkl` provides typed primitives for mode-aware
    heading level/numbering and for a step-level phase placement, and
    `workflow-composite.pkl` renders the phase appendix only for phases a module
    still lists, drops the `## Canonical workflow` wrapper so the workflow's own
    sections sit directly under the skill title, and states the composite
    control-flow rules with the rest of the preamble instead of after the
    appendices.
  - Boundaries (in/out of scope): In — `config/pkl/base/workflow-content.pkl`
    (heading-scale and step-heading helpers, an optional/empty `phases` listing
    in `StructuredCompositeSource`, a placement primitive for inlining a phase
    body at a step) and `config/pkl/renderers/workflow-composite.pkl`
    (`renderSkill` section order, conditional appendix, `phaseNameBySlug`
    retained only while phases remain listed). Out — phase body text, command
    body text, `references/output.md`, package-mode document shape, and the four
    workflow modules' contents.
  - Dependencies: none
  - Done when: all 12 generated workflow `SKILL.md` files have their workflow
    `## Input` / `## Workflow` / `## Rules` sections directly under the skill's
    `#` title with no `## Canonical workflow` wrapper and no leading `SCE
    {WORKFLOW} $ARGUMENTS` line; the composite control-flow paragraph appears in
    the preamble before `## Input`; the phase appendix still renders unchanged
    for all four workflows because no module has migrated yet; a module that
    lists no phases would render no `## Embedded phase behavior` heading;
    `nix run .#pkl-check-generated` and `nix flake check` pass.
  - Verification notes (commands or checks):
    `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"` into pre- and
    post-task roots; `diff -r` to confirm the only changes are the dropped
    wrapper/argument line and the relocated control-flow paragraph;
    `nix run .#pkl-check-generated`; `nix flake check`.
  - Implementation: `workflow-content.pkl` gained `headingMarker`, a
    `PhaseHeadings` class (`section`, `stepHeading`, `stepSubsection`,
    `stepReference`) that shifts one heading level and renumbers a phase step as
    `{step}.{n}` in composite mode, the separator-owning `packageOnlyBlock` /
    `compositeOnlyBlock` helpers, `commandBanner` and `inlinePhaseBody` built on
    them, and a default-empty `phases` on `StructuredCompositeSource`.
    `workflow-composite.pkl` rebuilt `renderSkill` as a joined section list:
    preamble (now carrying `## Composite control flow`), the workflow body with
    no `## Canonical workflow` wrapper, then the phase appendix and the internal
    documents each emitted only when their listing is non-empty. Each of the four
    workflow modules replaced its literal `SCE {WORKFLOW} $ARGUMENTS` banner line
    with the rendered `commandBanner`.
  - Verification: `nix run .#pkl-generate` into pre- and post-task roots;
    `diff -r` reported exactly the 12 workflow `SKILL.md` files changed, and only
    by the dropped wrapper heading, the dropped banner line, the relocated
    control-flow section, and the blank lines the empty `internalDocuments`
    listing used to leave behind. A grep over the post-task root found no
    `^## Canonical workflow` and no `^SCE ... $ARGUMENTS` line, with
    `## Composite control flow` ahead of the first `## Input` in all 12 files.
    Temporarily emptying the commit module's `phases` confirmed the document then
    renders no `## Embedded phase behavior` heading and ends at `## Rules`; that
    change was reverted. `nix run .#pkl-check-generated` passed (46 files,
    inventory sha256
    `ede8bd67a00030abe80962e2065b47858e7e4d346dd48c637664f7c431173ebe`).
    `nix flake check` passed.
  - Deviation: dropping the leading banner required a one-line change in each of
    the four workflow modules, which this task's boundary lists as out of scope.
    The line is emitted from inside each module's `renderCommandBody` string, so
    the renderer cannot drop it without parsing Markdown, which
    `context/patterns.md` forbids. Nothing else in those modules changed.
  - Note: `PhaseHeadings` and `inlinePhaseBody` are defined but not yet consumed;
    T02–T05 are their callers.

## Validation Report

**Status:** validated  
**Date:** 2026-07-30

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (46 files, inventory sha256 `877ea6283a1d801986f8d34add00b55a79c405c127cb66c293bedcda4bbe58fb`, matching T06's recorded value — generation contract stable)
- `nix flake check` -> exit 0 (all checks passed)
- `nix run .#pkl-generate -- {tmpdir}` (fresh ephemeral generation) -> exit 0 (46 files generated)
- AC1 grep for `^## (Canonical workflow|Embedded phase behavior|Internal phase:)` and `^# SCE (Context Load|...|Atomic Commit)` over the 12 `SKILL.md` files -> no matches
- AC2 awk pass over the 12 `SKILL.md` files (outside fenced blocks) -> each file: one `#` heading, one each of `## Purpose`/`## Input`/`## Workflow`/`## Rules`, no heading-level skip, no duplicated level+text heading
- AC3 inspection of every `**{Phase} phase**` mention across the four workflows' generated `SKILL.md` files against the line number of the step that inlines that phase's instructions -> every named-only mention either follows its inlining step or immediately precedes it as the handoff line into that step (the same short anticipatory-mention pattern independently accepted in T02–T05), no mention points forward past its inlining step
- AC4 grep over the 12 `SKILL.md` files *and their `references/output.md` siblings* for `invoking workflow`, `This skill`, `this skill's`, `The skill is complete after`, `Internal SCE workflow skill`, `^the \*\*` -> no matches anywhere (T06's four rewords closed the gap left open by the prior `/validate` run)
- AC5 diff of each of the four workflows' `.claude` `SKILL.md` against the pre-plan baseline (commit `935745f`, the last commit before this plan's uncommitted work), plus a targeted re-check of every removed line containing a gate/prohibition keyword (`do not`, `wait for`, `stop.`, `must not`, `never`, `gate`) that had no exact match on the added side -> every such line is a rewording of a rule that still exists under new phrasing (e.g. "This skill must not be run for `declined`, `blocked`, or `incomplete`" -> "This phase must not be run for `declined`, `blocked`, or `incomplete`"; "Never commit on the regular path." preserved verbatim), not a dropped rule
- AC6: covered by the `pkl-check-generated` run above; inventory sha256 matches T06's recorded value

### Scaffolding removed

None.

### Success-criteria verification

- [x] AC1: no forbidden heading in any generated workflow `SKILL.md` -> grep found zero matches across all 12 files
- [x] AC2: exactly one `#`, one each of the four `##` sections, no level skip, no duplicate heading -> awk pass confirmed for all 12 files
- [x] AC3: no phase reference points forward -> every `**{Phase} phase**` mention resolves to an inlining step heading at or before it, matching the pattern already verified per-task in T02–T05
- [x] AC4: no generated workflow document attributes phase behavior to a separate skill or an invoking workflow -> grep over the 12 `SKILL.md` files and their `references/output.md` siblings found zero matches, confirming T06's rewords closed the prior run's gap
- [x] AC5: every step, gate, branch, wait, prohibition, and verification instruction present before the change is still present -> diff against the pre-plan baseline (`935745f`) shows every removed gate/prohibition line restated under new phrasing at the step that now holds the phase, with no rule dropped
- [x] AC6: generation contract unchanged -> `nix run .#pkl-check-generated` passed with 46 files and the same inventory sha256 recorded after T06

### Failed checks and follow-ups

None.

### Residual risks

- None identified.

- [x] T02: `Inline the change-to-plan phases at their steps` (status:done)
  - Task ID: T02
  - Goal: `sce-change-to-plan/SKILL.md` reads top to bottom: step 1 carries the
    context-load instructions as `#### 1.1`–`#### 1.5` plus its boundaries, step
    2 carries the plan-authoring instructions as `#### 2.1`–`#### 2.8` plus its
    tone and boundaries, and the plan template is a correctly leveled appendix
    after `## Rules`.
  - Boundaries (in/out of scope): In — `config/pkl/base/workflow-change-to-plan.pkl`:
    the two phase bodies, their placement in `renderCommandBody` steps 1 and 2,
    the plan-template appendix leveling, the dropped titles / `## Purpose` /
    `## Input` provider lists / `## Completion` sections, the folding of
    substantive Input prose into the step, and the lowercase sentence starts
    (`the **Plan authoring phase** exclusively owns:`, `it. the **Plan authoring
    phase** owns resolving it`). Out — the other three workflow modules,
    `CHANGE_OUTPUT`, and the workflow's steps 3 and 4.
  - Dependencies: T01
  - Done when: the generated `sce-change-to-plan/SKILL.md` has one `#` heading,
    one each of `## Purpose` / `## Input` / `## Workflow` / `## Rules`, no
    heading-level skip, no `## Internal phase:` or `# SCE Context Load` /
    `# SCE Plan Authoring` heading, and no `invoking workflow` / `This skill`
    sentence; the step-2 re-run references in steps 3 and 4 point back at step 2;
    every gate, branch, wait, and prohibition present before the change is still
    present; `nix run .#pkl-check-generated` and `nix flake check` pass.
  - Verification notes (commands or checks):
    `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"`; read the
    generated `sce-change-to-plan/SKILL.md` end to end; run the AC2 awk pass and
    the AC1/AC3/AC4 greps against it; diff against the pre-task root and account
    for every removed line; `nix run .#pkl-check-generated`; `nix flake check`.
  - Implementation: `workflow-change-to-plan.pkl` now places the context-load
    and plan-authoring bodies directly under workflow steps 1 and 2 through
    `inlinePhaseBody`, renders their numbered instructions as `1.1`–`1.5` and
    `2.1`–`2.8`, gives their surviving tone and boundary sections unique scaled
    headings, and leaves the composite phase listing empty. Package-only titles,
    ownership/provider restatements, and completion recaps no longer appear in
    the composite document. The plan-template appendix scales its outer sections
    beneath the renderer-owned appendix heading. Input text now points to step 2,
    so phase names are introduced only after their instruction-bearing workflow
    heading.
  - Verification: generated pre- and post-task roots with `nix run
    .#pkl-generate`; `diff -qr` reported only the three target-specific
    `sce-change-to-plan/SKILL.md` files changed. The generated Pi skill was read
    end to end. Greps found no canonical-workflow, embedded-phase,
    internal-phase, standalone phase-title, separate-skill, invoking-workflow,
    or lowercase phase-reference text. An awk pass outside fenced examples found
    one H1, no heading-level skip, and no duplicate level-plus-text heading.
    The reviewed generated diff contains only structural movement, typed heading
    scaling, and the planned removal or rewording of ownership/provider/title/
    completion restatements. `nix run .#pkl-check-generated` passed (46 files,
    inventory sha256
    `3f623e766aca009298ca3114cb41fcc1ba9094ff1a0a03eef770c4a09aa1881e`).
    `nix flake check` passed.
  - Deviation: the first use of T01's `PhaseHeadings` exposed that Pkl requires
    its module-level `headingMarker` closure to be `const`; added that modifier
    in `workflow-content.pkl` so the already-authored typed helper can execute.

- [x] T03: `Inline the next-task phases at their steps` (status:done)
  - Task ID: T03
  - Goal: `sce-next-task/SKILL.md` carries plan review under step 1, task
    execution under step 2, and task context synchronization under step 3, each
    renumbered beneath its step, with the four duplicated `## Purpose` /
    `## Input` / `## Workflow` section sets reduced to one of each.
  - Boundaries (in/out of scope): In — `config/pkl/base/workflow-next-task.pkl`
    (the review and execution phase bodies and their placement at steps 1 and 2)
    and the task role in `config/pkl/base/workflow-context-sync.pkl` (its body
    and placement at step 3, with the role carrying its own heading scale so the
    plan role is untouched). Out — the plan role's rendering, the other three
    workflow modules, `NEXT_OUTPUT`, and the sync report bodies.
  - Dependencies: T01
  - Done when: the generated `sce-next-task/SKILL.md` satisfies the AC1–AC4
    checks; intra-phase step cross-references (for example the execution phase's
    reference to its own terminal-status step) name the new dotted numbers;
    every gate, wait, prohibition, and verification instruction survives; the
    generated `sce-validate/SKILL.md` is unchanged by this task;
    `nix run .#pkl-check-generated` and `nix flake check` pass.
  - Verification notes (commands or checks):
    `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"`; read the
    generated `sce-next-task/SKILL.md` end to end; AC1–AC4 checks against it;
    confirm `diff -r` reports no change to the `sce-validate` package; diff
    against the pre-task root and account for every removed line;
    `nix run .#pkl-check-generated`; `nix flake check`.
  - Implementation: `workflow-next-task.pkl` now inlines plan review under step
    1 and task execution under step 2 through typed `PhaseHeadings` and
    `inlinePhaseBody` rendering, with dotted phase-step references and no phase
    appendix. The task role in `workflow-context-sync.pkl` owns an independent
    step-3 heading scale, inlines its substantive input, workflow, and boundaries,
    and keeps its package-only introduction and completion recap out of the
    composite document. The plan context-sync role retains its existing rendering.
  - Verification: generated pre- and post-task roots with `nix run
    .#pkl-generate`; `diff -qr` reported only the three target-specific
    `sce-next-task/SKILL.md` files changed and no `sce-validate` artifact changed.
    The generated Pi skill was read end to end. Focused greps found no canonical,
    embedded-phase, internal-phase, standalone phase-title, separate-skill,
    invoking-workflow, or lowercase phase-reference text. An awk pass outside
    fenced examples found one H1, one each of the workflow's `Purpose`, `Input`,
    `Workflow`, and `Rules` sections, no heading-level skip, and no duplicate
    level-plus-text heading. The generated diff preserved every gate, wait,
    branch, prohibition, verification instruction, and continuation while moving
    the three phase bodies to their owning steps. `nix run
    .#pkl-check-generated` passed (46 files, inventory sha256
    `3cc6ef68a31940fccc72310b83019d72fe56644e15fe6d7f6bb0e8df564594ae`).
    `nix flake check` passed.

- [x] T04: `Inline the validate phases at their steps` (status:done)
  - Task ID: T04
  - Goal: `sce-validate/SKILL.md` carries the validation phase under step 1 and
    plan context synchronization under step 2, with the plan-file validation
    report as a correctly leveled appendix after `## Rules`.
  - Boundaries (in/out of scope): In — `config/pkl/base/workflow-validate.pkl`
    (the validation phase body, its placement at step 1, and the
    validation-report appendix leveling), the plan role in
    `config/pkl/base/workflow-context-sync.pkl` (its body and placement at step
    2), and this module's lowercase sentence starts. Out — the task role's
    rendering, the other three workflow modules, `VALIDATE_OUTPUT`, and the
    validation-result output document.
  - Dependencies: T01, T03
  - Done when: the generated `sce-validate/SKILL.md` satisfies the AC1–AC4
    checks; the plan-file validation report keeps every rule and placeholder it
    states today; the generated `sce-next-task/SKILL.md` is unchanged by this
    task; `nix run .#pkl-check-generated` and `nix flake check` pass.
  - Verification notes (commands or checks):
    `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"`; read the
    generated `sce-validate/SKILL.md` end to end; AC1–AC4 checks against it;
    confirm `diff -r` reports no change to the `sce-next-task` package; diff
    against the pre-task root and account for every removed line;
    `nix run .#pkl-check-generated`; `nix flake check`.
  - Completed: 2026-07-30
  - Files changed: `config/pkl/base/workflow-validate.pkl`,
    `config/pkl/base/workflow-context-sync.pkl`.
  - Implementation: `workflow-validate.pkl` now inlines the validation body under
    step 1 through typed `PhaseHeadings` and `inlinePhaseBody` rendering, leaves
    its composite phase listing empty, and scales the plan-file validation report
    beneath the renderer-owned appendix heading. The plan role in
    `workflow-context-sync.pkl` now owns independent step-2 heading scaling,
    preserves its substantive final-pass and handoff prose inside the workflow
    step, and keeps its package-only title, ownership/provider restatements, and
    completion recap out of the composite document.
  - Verification: generated pre- and post-task roots with `nix run
    .#pkl-generate`; `diff -qr` reported only the three target-specific
    `sce-validate/SKILL.md` files changed, while all generated `sce-next-task`
    packages and `sce-validate/references/output.md` files remained byte-identical.
    The generated Pi skill was read end to end. Focused greps found no canonical,
    embedded-phase, internal-phase, standalone phase-title, separate-skill,
    invoking-workflow, or lowercase phase-reference text. An awk pass outside
    fenced examples found one H1, one each of the workflow's `Purpose`, `Input`,
    `Workflow`, and `Rules` sections, no heading-level skip, and no duplicate
    level-plus-text heading. The generated diff preserved the validation-report
    placeholders and rules while structurally moving both phase bodies. `nix run
    .#pkl-check-generated` passed (46 files, inventory sha256
    `ce8ed0571eb21812d0fa2296844fbcbd2018c2567d980ee5401fc5ffda2dd0a8`).
    `nix flake check` passed.

- [x] T05: `Inline the commit phase at its step` (status:done)
  - Task ID: T05
  - Goal: `sce-commit/SKILL.md` carries the atomic-commit phase inside the
    regular-mode step that runs it, its `#### 1.`/`#### 2.` bypass and regular
    sub-steps stop competing with the phase's own numbering, and the appendix is
    gone.
  - Boundaries (in/out of scope): In — `config/pkl/base/workflow-commit.pkl`:
    the atomic-commit phase body, its placement at the proposing step, the
    existing `####` mode sub-step numbering, and `the **Atomic commit phase**
    exclusively owns:`. Out — the other three workflow modules, `COMMIT_OUTPUT`,
    and the commit-message-style output document.
  - Dependencies: T01
  - Done when: the generated `sce-commit/SKILL.md` satisfies the AC1–AC4 checks
    with a single unambiguous numbering scheme across bypass mode, regular mode,
    and the inlined phase; the staged-truth rules, plan-citation rules, and every
    prohibition survive; `nix run .#pkl-check-generated` and `nix flake check`
    pass.
  - Verification notes (commands or checks):
    `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"`; read the
    generated `sce-commit/SKILL.md` end to end; AC1–AC4 checks against it; diff
    against the pre-task root and account for every removed line;
    `nix run .#pkl-check-generated`; `nix flake check`.
  - Completed: 2026-07-30
  - Files changed: `config/pkl/base/workflow-commit.pkl`.
  - Implementation: `workflow-commit.pkl` now reorders the command body so the
    Regular path (the default, no-token path) comes first and the Bypass path
    second. The atomic-commit phase is inlined at the Regular path's step 2
    ("Propose commits") through a `PhaseHeadings { step = 2; compositeShift = 2
    }` instance (an extra shift level because that step already sits one heading
    level deeper, behind the mode-path grouping heading), rendering its own steps
    as `2.1`–`2.9` and its boundaries as a sibling heading of the mode sub-steps.
    The Bypass path's step 2, which invokes the same phase, now points back to
    the Regular path's inlined description instead of restating it, since it is
    a plain name-only reference that must resolve to content already stated
    earlier in the document. The phase's package-only title, `## Purpose`
    ownership list (already stated by the command's own "exclusively owns"
    bullets), `## Input` provider list, and `## Completion` section were dropped
    from composite rendering; its substantive Input prose (mode inference,
    commit-context truth, staged-diff-only acceptance) was folded in unconditionally
    since it isn't restated elsewhere. `the **Atomic commit phase** exclusively
    owns:` became `The **Atomic commit phase** exclusively owns:` to remove the
    lowercase phase-reference sentence start. `structuredComposite.phases` is now
    empty.
  - Verification: generated pre- and post-task roots with `nix run
    .#pkl-generate`; `diff -qr` reported only the three target-specific
    `sce-commit/SKILL.md` files changed, with `references/output.md` and
    `COMMIT_CONTRACT`/`COMMIT_MESSAGE_STYLE`-derived files untouched. The
    generated Claude-target `sce-commit/SKILL.md` was read end to end. AC1 and
    AC4 greps found no `## Canonical workflow` / `## Embedded phase behavior` /
    `## Internal phase:` heading, no `# SCE Atomic Commit` title, and no
    `invoking workflow`, `This skill`, `this skill's`, `The skill is complete
    after`, `Internal SCE workflow skill`, or `^the \*\*` text. An AC2 awk pass
    outside fenced examples found exactly one `#` heading, one each of `##
    Purpose` / `## Input` / `## Workflow` / `## Rules`, no heading-level skip,
    and no duplicated level-plus-text heading. AC3 was checked by locating every
    `**Atomic commit phase**` mention and the phase's first inlined heading
    (`##### 2.1`): the only mentions preceding that heading are within the same
    Regular-path step 2 paragraph that inlines it (matching the precedent already
    accepted in the generated `sce-next-task` and `sce-validate` packages), and
    every mention in the (now second) Bypass path and in `## Rules` follows it.
    `diff -u` against the pre-task file confirmed every removed line is a
    heading, a dropped `## Purpose` / `## Input` / `## Completion` line, or the
    one lowercase-start rewording, with every gate, branch, wait, and
    prohibition preserved verbatim. `nix run .#pkl-check-generated` passed (46
    files, inventory sha256
    `7af4cc1bf1ee1313f5b94d46fb09ddd11db7a55e0f01cb2eec3581c8a220359f`). `nix
    flake check` passed.

- [x] T06: `Reword the four stale invoking-workflow sentences in output.md templates` (status:done)
  - Task ID: T06
  - Goal: the four sentences named above the acceptance-criteria list no longer
    read "the invoking workflow" or "This skill is not run for", in each of the
    three already-in-scope `.pkl` files that define them, without changing any
    `output.md` layout, placeholder, field name, or report rule.
  - Boundaries (in/out of scope): In — the exact string literal at
    `config/pkl/base/workflow-change-to-plan.pkl:1349` ("The user-facing summary
    shown after a plan is written. The invoking workflow renders it..."); the
    exact string literal at `config/pkl/base/workflow-validate.pkl:588` and the
    identical shared literal at `config/pkl/base/workflow-context-sync.pkl:746`
    ("{Include only non-blocking information the invoking workflow should
    retain. ...}"); and the exact string literal at
    `config/pkl/base/workflow-context-sync.pkl:984` ("...This skill is not run
    for `failed` or `blocked` validation results."). Out — every other sentence
    in these four files' `output.md`-defining constants, every `SKILL.md`-facing
    string in these files, `workflow-commit.pkl`'s package-only
    `COMMIT_CONTRACT` instance of "The invoking workflow executes the commit"
    (already confirmed absent from every generated composite document), and any
    layout structure, heading, placeholder token, or field name in the four
    sentences being reworded.
  - Dependencies: none
  - Done when: a fresh `nix run .#pkl-generate -- {tmpdir}` produces 12
    `SKILL.md` files and their `references/output.md` siblings with zero matches
    for `invoking workflow`, `This skill`, `this skill's`, `The skill is
    complete after`, `Internal SCE workflow skill`, and `^the \*\*` (AC4's full
    grep, now including the `references/output.md` siblings); `diff -qr`
    against a pre-task generation root shows changes confined to the four
    `references/output.md` files that carried the stale sentences (each in all
    three targets), with every placeholder token, field name, and layout line
    otherwise identical; `nix run .#pkl-check-generated` and `nix flake check`
    pass with the same 46-file, same-metadata contract.
  - Verification notes (commands or checks):
    `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"` into pre- and
    post-task roots; the full AC4 grep (including `references/output.md`
    siblings) against the post-task root; `diff -qr` between the two roots;
    `nix run .#pkl-check-generated`; `nix flake check`.
  - Completed: 2026-07-30
  - Files changed: `config/pkl/base/workflow-change-to-plan.pkl`,
    `config/pkl/base/workflow-validate.pkl`,
    `config/pkl/base/workflow-context-sync.pkl`.
  - Implementation: reworded the exact four string literals named in scope.
    `workflow-change-to-plan.pkl:1349`'s Plan Summary description now reads "It
    is rendered from the `plan_ready` result, immediately before the
    continuation block." instead of naming an invoking workflow.
    `workflow-validate.pkl:588` and the identical shared literal at
    `workflow-context-sync.pkl:746` (the Notes-section placeholder used by both
    the plan and task context-sync report roles) now read "{Include only
    non-blocking information worth retaining. Omit this section when
    unnecessary.}". `workflow-context-sync.pkl:984`'s plan-report input-status
    sentence now reads "This report is not produced for `failed` or `blocked`
    validation results." instead of "This skill is not run for...". No
    placeholder token, field name, heading, or layout line changed in any of
    the four sentences.
  - Verification: generated a pre-task root with `nix run .#pkl-generate`,
    applied the four rewords, then generated a post-task root. The full AC4
    grep (`invoking workflow`, `This skill`, `this skill's`, `The skill is
    complete after`, `Internal SCE workflow skill`, `^the \*\*`) over the
    post-task root's 12 `SKILL.md` files and their `references/output.md`
    siblings found zero matches. `diff -qr` between the two roots reported
    exactly 9 changed files — the `sce-change-to-plan`, `sce-next-task`, and
    `sce-validate` `references/output.md` files across all three targets — and
    a line-level `diff -u` of each confirmed every changed line is one of the
    four reworded sentences, with every placeholder token, field name, and
    layout line otherwise identical. `nix run .#pkl-check-generated` passed (46
    files; inventory sha256 changed to
    `877ea6283a1d801986f8d34add00b55a79c405c127cb66c293bedcda4bbe58fb` since the
    generated content itself changed, but the same 46 artifact paths and
    metadata contract held). `nix flake check` passed (all checks passed).

## Open questions

- Package mode renders nothing. `workflow-composite.pkl` is the only consumer of
  the four base modules, and it reads `structuredComposite` plus
  `workflow.command.document` for its `.path` alone — so `COMMAND`,
  `CONTEXT_SKILL`, `AUTHORING_SKILL`, every `references/*-contract.yaml`, and the
  `SkillPackage` / `WorkflowPackage` values are unconsumed by `generate.pkl` and
  every renderer. That is roughly half of `config/pkl/base/`, and it is the
  direct cause of the defects you reported: the dual-spelling machinery is what
  produced `# SCE Context Load` under `## Internal phase: Context load phase` and
  the lowercase mid-sentence substitutions. This plan works around it (heading
  level and placement stay mode-aware; prose is changed once) rather than
  deleting it, because `context/patterns.md` deliberately retains those packages
  as authoring inputs and removing them is a decision-record change, not a
  refactor. If you would rather collapse to a single spelling, that is a separate
  plan and it would shrink these five tasks to roughly two.
- Nothing guards the result. `generation-contract-check.pkl` asserts paths,
  metadata, and stale slugs — not "one `#` per document, no heading-level skip,
  no repeated heading". The AC2 awk pass is a one-off; the natural guard is a
  predicate in the contract check, which the previous plan also identified for
  layout duplication and also left out of scope. Worth one follow-up plan
  covering both.
