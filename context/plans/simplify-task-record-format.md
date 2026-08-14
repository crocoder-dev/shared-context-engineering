# Plan: simplify-task-record-format

## Change summary

The SCE plan task record currently represents the same implementation and
verification facts through several overlapping fields: `Goal`, `Boundaries
(in/out of scope)`, `Done when`, `Verification notes`, `Implementation
evidence`, `Verification evidence`, `Files changed`, `Evidence`, and a
separately serialized `Context synchronization handoff` (itself repeating
plan path, task ID, task title, changed files, implementation summary,
verification, done checks, and context impact). This plan compacts the
task-record format so a completed task is the single authoritative record of
implementation intent, completion conditions, verification, actual execution
result, changed files, context impact, and context synchronization state,
with no second "handoff" representation. `/change-to-plan` continues to
author task intent (title, `Scope`, `Dependencies`, `Done when`, planned
`Verify` checks, `Context synchronization: pending`); `/next-task` writes
execution facts (`Completed`, `Files changed`, `Result`, `Verify` outcomes,
`Context impact`, `Context synchronization: pending`) directly onto that same
task instead of building a second record; task context synchronization reads
that completed task record directly — identified only by plan path and task
ID — for both immediate synchronization and later sync-debt recovery, and
writes back only `Context synchronization: synced` or `blocked` plus
blocker-only-when-blocked metadata. This extends the existing
`config/pkl/base/workflow-{change-to-plan,next-task,context-sync}.pkl`
sources and the `generation-contract-check.pkl` contract; it does not migrate
historical plans or add backward compatibility for the old format.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: A newly generated plan task uses the compact schema — title,
      `Scope`, `Dependencies`, `Done when`, `Verify`,
      `Context synchronization` — and never `Goal`,
      `Boundaries (in/out of scope)`, or `Verification notes`.
  - Validate: generate an ephemeral payload and inspect
    `sce-change-to-plan/references/plan-template.md`'s new-task example for
    the compact fields and the absence of the removed fields.
- [x] AC2: After `/next-task` completes implementation, execution facts are
      recorded once on the task itself (`Completed`, `Files changed`,
      `Result`, `Verify` outcomes, `Context impact`,
      `Context synchronization`), with no `Implementation evidence` or
      `Verification evidence` section.
  - Validate: inspect generated `sce-next-task/references/task-execution.md`
    and the plan-template completed-task example for the compact completion
    shape and the absence of `Implementation evidence` /
    `Verification evidence`.
- [x] AC3: No `Context synchronization handoff` appears in newly generated
      plans or workflow instructions.
  - Validate: `rg -n "Context synchronization handoff" <ephemeral generated
    payload root>` returns no matches.
- [x] AC4: Task context synchronization performs both immediate
      synchronization and later sync-debt recovery by reading the completed
      task record identified by plan path and task ID, without a duplicated
      handoff structure.
  - Validate: inspect generated
    `sce-next-task/references/{context-sync,plan-review}.md` for direct
    completed-task-record reading on both the immediate-sync and
    sync-debt-recovery paths.
- [x] AC5: A blocked context sync adds only synchronization-specific blocker
      metadata (`Blocker`, `Required action`, `Retry condition`) and leaves
      the task's existing execution facts authoritative.
  - Validate: inspect the generated `context-sync.md` blocked write-back
    instructions and the plan-template completion-record blocked example.
- [x] AC6: Generated Pi, Claude, and OpenCode workflow packages all express
      the same compact model.
  - Validate: generate the ephemeral payload and compare the three targets'
    `sce-change-to-plan` / `sce-next-task` reference documents for parity
    apart from supported target-specific frontmatter.
- [x] AC7: Generation-contract checks and fixtures prevent the old
      duplicated-field model from being reintroduced.
  - Validate: `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl`;
    `nix run .#pkl-check-generated`.
- [x] AC8: Durable `context/` documentation describes the completed task
      record as the durable synchronization/recovery source, with no
      current-state claim that a separate synchronization handoff must be
      persisted.
  - Validate: `rg -n "Context synchronization handoff" context/overview.md
    context/sce/*.md context/glossary.md` returns no current-state matches.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix run .#pkl-check-generated`
- `nix develop -c pkl eval config/pkl/base/workflow-change-to-plan.pkl`
- `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl`
- `nix develop -c pkl eval config/pkl/base/workflow-context-sync.pkl`
- `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl`

### Context sync

- `context/overview.md`
- `context/sce/shared-context-plan-workflow.md`
- `context/sce/shared-context-code-workflow.md`
- `context/glossary.md` (the `baseline-relative task handoff` entry)
- A new ADR under `context/decisions/` superseding the handoff-shape portion
  of `context/decisions/2026-08-12-persist-workflow-sync-lifecycle-in-plans.md`

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `config/pkl/base/workflow-change-to-plan.pkl`,
  `config/pkl/base/workflow-next-task.pkl`,
  `config/pkl/base/workflow-context-sync.pkl`,
  `config/pkl/renderers/generation-contract-check.pkl` and its fixtures, the
  ephemeral generated Pi/Claude/OpenCode payloads (verification only, never
  hand-edited), and current-state `context/` documentation describing this
  lifecycle (including one new/superseding ADR).
- **Out of scope:** `/validate`'s validation-only boundary, plan-level
  context synchronization, `sce-decision` ownership, task approval
  semantics, baseline-relative changed-file computation, and migration of
  historical `context/plans/` files.
- **Constraints:** No backward compatibility for the old verbose task-record
  format. `config/pkl/**` remains the canonical source; generated targets are
  produced and inspected only through ephemeral generation
  (`nix run .#pkl-generate` / `nix run .#pkl-check-generated`), never
  hand-edited.
- **Non-goal:** Do not introduce a differently named field that recreates the
  removed duplication (for example, keeping both a verbose "Implementation
  summary" and `Result`). `Result` stays a short factual outcome, not a prose
  diff.

## Assumptions

- Both the composite reference document (`changeToPlanPlanTemplate`, which
  feeds the real generated `sce-change-to-plan/references/plan-template.md`)
  and the legacy package-mode renderer (`renderPlanTemplate` /
  `PLAN_TEMPLATE`, which feeds the currently-unrendered canonical phase
  module `planAuthoringPackage`) are updated together for consistency, even
  though only the composite path is emitted to real targets today. This
  follows the existing "canonical phase module" convention recorded in
  `context/glossary.md`.
- The new ADR supersedes only the handoff-shape aspect of
  `2026-08-12-persist-workflow-sync-lifecycle-in-plans.md`; that decision's
  `pending`/`synced`/`blocked` lifecycle-state invariant is not re-decided
  and remains in force.

## Task stack

- [x] T01: `Compact the change-to-plan task schema and plan template` (status:done)
  - Task ID: T01
  - Goal: Replace the new-task authoring shape and completed-task shape in
    `config/pkl/base/workflow-change-to-plan.pkl` with the compact schema —
    title/`Scope`/`Dependencies`/`Done when`/`Verify`/`Context
    synchronization` for new tasks; `Completed`/`Files changed`/`Result`/
    `Context impact`/`Verify` outcomes/`Context synchronization`, plus a
    blocker subsection only when blocked, for completed tasks — removing
    `Goal`, `Boundaries (in/out of scope)`, `Verification notes`,
    `Implementation evidence`, `Verification evidence`, and
    `Context synchronization handoff` from every rendering of the plan
    template.
  - Boundaries (in/out of scope): In — `renderPlanTemplate` / `PLAN_TEMPLATE`
    (package-mode template render), `changeToPlanPlanTemplate` (composite
    reference document), the filled-in task example, the completion-record
    example, and any authoring-skill prose (`changeToPlanPlanAuthoring`,
    `AUTHORING_SKILL` / `renderAuthoringSkillBody`, `AUTHORING_CONTRACT`)
    that names the removed fields or describes task authoring. Out —
    `workflow-next-task.pkl`, `workflow-context-sync.pkl`,
    `generation-contract-check.pkl`, and durable `context/` docs (covered by
    later tasks); no change to the clarification gate, acceptance-criteria
    section, or task-slicing rules unrelated to the per-task field schema.
  - Dependencies: none
  - Done when: Both the composite (`changeToPlanPlanTemplate`) and legacy
    package-mode (`renderPlanTemplate`) renderings of the plan template
    define the compact new-task shape and the compact completed-task shape
    exactly as specified, and reference neither `Goal:`,
    `Boundaries (in/out of scope):`, `Verification notes`,
    `Implementation evidence`, `Verification evidence`, nor
    `Context synchronization handoff`.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/base/workflow-change-to-plan.pkl`;
    generate an ephemeral payload (`nix run .#pkl-generate -- "$(mktemp -d)"`)
    and inspect the generated `sce-change-to-plan/references/plan-template.md`
    for the new task/completed-task examples.
  - Context synchronization: synced
  - Context synchronization handoff: Plan path: context/plans/simplify-task-record-format.md; Task ID: T01; Task title: Compact the change-to-plan task schema and plan template; Changed files: config/pkl/base/workflow-change-to-plan.pkl; Implementation summary: Replaced the new-task and completed-task field shapes in both the composite (`changeToPlanPlanTemplate`) and legacy package-mode (`renderPlanTemplate`/`PLAN_TEMPLATE`) plan-template renders: dropped `Goal`, renamed `Boundaries (in/out of scope)` to `Scope`, and renamed `Verification notes (commands or checks)` to `Verify` for new tasks (template block and filled-in example, both renders); replaced the completed-task template's `Context synchronization handoff`/`Evidence`/`Notes` fields with `Result` and `Context impact`, keeping `Completed`, `Files changed`, `Context synchronization`, and the blocker-only-when-blocked subsection; fixed two stray prose references to the old `Verification notes` field name in "Acceptance criteria rules" (both renders) to say `Verify`. Authoring-skill prose (`changeToPlanPlanAuthoring`, `AUTHORING_SKILL`/`renderAuthoringSkillBody`, `AUTHORING_CONTRACT`) named no removed field directly (it references the template by pointer), so no change was needed there.; Verification: `nix develop -c pkl eval config/pkl/base/workflow-change-to-plan.pkl` succeeded; generated an ephemeral payload via `nix run .#pkl-generate -- <tmpdir>` and inspected `sce-change-to-plan/references/plan-template.md` for Claude, Pi, and OpenCode targets — all three render the compact schema and are byte-identical (`diff` empty); `rg` confirmed no removed field names (`Goal:`, `Boundaries (in/out of scope)`, `Verification notes`, `Implementation evidence`, `Verification evidence`, `Context synchronization handoff`) remain anywhere in the source or generated plan-template. `nix run .#pkl-check-generated` still fails on the pre-existing `handoff-identity-fields` check in `generation-contract-check.pkl`, which is out of scope for T01 and owned by T04.; Done checks: Both renderings define the compact new-task shape and compact completed-task shape exactly as specified, and reference none of the removed field names — satisfied, confirmed by grep and generated-output inspection.; Context impact: none — this task changes only ephemeral-generation source (`config/pkl/**`); durable `context/` doc updates are T05's responsibility.
  - Completed: 2026-08-13
  - Files changed: config/pkl/base/workflow-change-to-plan.pkl
  - Evidence: `nix develop -c pkl eval config/pkl/base/workflow-change-to-plan.pkl` (passed); ephemeral generation via `nix run .#pkl-generate -- <tmpdir>` followed by `rg`/`diff` inspection of the generated `plan-template.md` across all three targets (compact schema present, targets identical, no removed field names); `nix run .#pkl-check-generated` (fails only on the out-of-scope `handoff-identity-fields` check, expected until T04).
  - Notes: None.

- [x] T02: `Persist next-task execution facts directly on the completed task` (status:done)
  - Task ID: T02
  - Goal: Change `/next-task` in `config/pkl/base/workflow-next-task.pkl` so
    successful task execution writes `Completed`, baseline-relative
    `Files changed`, a concise `Result`, actual `Verify` outcomes, and
    `Context impact` directly onto the completed task, sets
    `Context synchronization: pending`, and invokes task context
    synchronization using that same task record — with no separate
    `Implementation evidence`, `Verification evidence`, or
    `Context synchronization handoff` construction — and change sync-debt
    recovery so it identifies the debt task from the plan and carries that
    task's own record forward instead of loading a persisted handoff.
  - Boundaries (in/out of scope): In — the task-execution completion-writing
    steps, the sync-debt recovery branch's task-record loading, and any
    prose in `workflow-next-task.pkl` describing the handoff/evidence
    fields. Out — the implementation gate, the `approved` flag, the
    verification-running steps themselves (only what gets recorded changes),
    plan-review's synced/blocked debt-scan trigger logic, and
    `workflow-context-sync.pkl` (covered by T03).
  - Dependencies: T01
  - Done when: `/next-task`'s completion-writing instructions in
    `workflow-next-task.pkl` name only `Completed`, `Files changed`,
    `Result`, updated `Verify` entries, `Context impact`, and
    `Context synchronization: pending`; no instruction constructs or
    persists `Implementation evidence`, `Verification evidence`, or a
    `Context synchronization handoff` block; sync-debt recovery locates the
    completed task by plan path and task ID and uses that record directly.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl`;
    generate an ephemeral payload and inspect the generated `sce-next-task`
    references (`task-execution.md`, `plan-review.md`) for the new
    completion-writing and sync-debt-recovery wording.
  - Completed: 2026-08-13
  - Files changed: config/pkl/base/workflow-next-task.pkl
  - Result: Replaced the "Update the plan" completion-writing steps (both the
    composite `nextTaskTaskExecutionReference` and the legacy package-mode
    `renderExecutionSkillBody`) so a completed task now records `Completed`,
    baseline-relative `Files changed`, a concise `Result`, actual `Verify`
    outcomes, and `Context impact` directly, dropping the `Context
    synchronization handoff` subsection and the separate implementation-
    evidence/verification-evidence bullets. Replaced the sync-debt recovery
    branch's task-record loading (both the composite
    `nextTaskPlanReviewReference` and the legacy package-mode
    `renderReviewSkillBody`, plus the composite `sync_debt` result-contract
    prose) so it detects unrecoverable debt from a missing completed-task
    record (no `Files changed`/`Result`/`Verify`/`Context impact`) instead of
    a missing handoff subsection, and otherwise reads that task's own
    completed record directly from the plan by plan path and task ID rather
    than loading a persisted handoff. Left plan-review's debt-scan trigger
    loop and `workflow-content.pkl`'s SKILL.md-level `sync_debt` branch prose
    unchanged, per the task's declared boundaries.
  - Context impact: none — this task changes only ephemeral-generation source
    (`config/pkl/**`); durable `context/` doc updates are T05's responsibility.
  - Verify:
    - `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl` — passed.
    - Generated an ephemeral payload via `nix run .#pkl-generate -- <tmpdir>`
      and inspected `sce-next-task/references/task-execution.md` and
      `plan-review.md` for Claude, Pi, and OpenCode targets — all three show
      the new completion-writing and sync-debt-recovery wording and are
      byte-identical (`diff` empty). `rg` confirmed none of the removed field
      names (`Context synchronization handoff`, `Implementation evidence`,
      `Verification evidence`) remain in either file, on any target. One
      unrelated, pre-existing `Verification evidence` string remains at
      `workflow-next-task.pkl:153`, inside user-facing presentation prose for
      the `incomplete` execution-result branch (not a plan-record field);
      out of scope for this task.
    - `nix run .#pkl-check-generated` now clears the `plan-review-sync-debt-
      recovery` check (which required literal "do not attempt a
      reconstructed retry" / "migrate the plan" wording — reflowed to keep
      both phrases on one line after the rewrite) and progresses to the next,
      still-failing `handoff-identity-fields` check — the same pre-existing
      failure T01 already recorded as out of scope, owned by T04.
  - Context synchronization: synced

- [x] T03: `Read the completed task record directly in task context synchronization` (status:done)
  - Task ID: T03
  - Goal: Change task context synchronization in
    `config/pkl/base/workflow-context-sync.pkl` so it resolves plan path and
    task ID, reads the matching completed task from the plan as its
    authoritative input, and on completion writes only
    `Context synchronization: synced` or `Context synchronization: blocked`
    plus a `Context synchronization blocker` (`Blocker` / `Required action` /
    `Retry condition`) — removing every instruction, validation rule, and
    prose reference describing a separate persisted
    `Context synchronization handoff`, including the rule that a valid retry
    depends on a persisted handoff and the duplicated Plan
    path/Task ID/Task title/Changed files/Implementation summary/
    Verification/Done checks/Context impact validation inside it.
  - Boundaries (in/out of scope): In — the task-context-sync phase's input
    resolution, handoff-shaped validation rules, and successful/blocked
    write-back instructions in `workflow-context-sync.pkl`. Out — the
    plan-level context-sync phase (already out of `/validate`'s boundary),
    the `sce-decision` gate invocation contract, and the root-context
    five-file pass.
  - Dependencies: T02
  - Done when: `workflow-context-sync.pkl`'s task-context-sync phase names
    the completed task record (`Files changed`, `Result`, `Verify`,
    `Done when`, `Context impact`, `Context synchronization`) as its input
    and validation source, contains no `Context synchronization handoff`
    validation rule or reference, and its successful/blocked write-back
    instructions match the target completion-record shape
    (blocker-only-when-blocked).
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/base/workflow-context-sync.pkl`;
    generate an ephemeral payload and inspect the generated task
    context-sync references in `sce-next-task` (`context-sync.md`,
    `sync-report.md`) for direct completed-task-record reading.
  - Completed: 2026-08-13
  - Files changed: config/pkl/base/workflow-context-sync.pkl
  - Result: Rewrote the task-context-sync phase's input, §3.1 handoff
    validation, and §3.8 blocked write-back — in both the composite
    (`taskRoleData`) and legacy package-mode (`taskReference`) renders — to
    resolve plan path and task ID and read the matching completed task
    record directly from the plan (`Files changed`, `Result`, `Verify`,
    `Done when`, `Context impact`, `Context synchronization`) for
    cross-session retry, alongside the unchanged live same-session
    `status: complete` execution-result path; dropped the "persisted vs.
    live handoff" dual-path framing and the requirement that a retry carry
    its own `Plan path`/`Task ID`/`Task title` fields, since plan review now
    supplies those identifiers directly (matching T02's updated contract).
    Removed the `blockedHandoffSection` field from the shared `SyncReportRole`
    class and its two implementations (`taskReport`'s eight-field
    `Context synchronization handoff` block, and `planReport`'s already-empty
    stub), simplifying `blockedReport`'s template to always render plan/task
    identity plus the `Context synchronization blocker` section, with no
    conditional handoff block. Updated `taskReport.rules` so blocked reports
    no longer restate the changed-files list (already on the plan's completed
    task record) and blocked write-back cites only the blocker subsection.
    Left `planRoleData`/`planReport` and the `sce-decision` gate untouched, per
    the task's declared boundaries.
  - Context impact: none — this task changes only ephemeral-generation source
    (`config/pkl/**`); durable `context/` doc updates are T05's responsibility.
  - Verify:
    - `nix develop -c pkl eval config/pkl/base/workflow-context-sync.pkl` — passed.
    - Generated an ephemeral payload via `nix run .#pkl-generate -- <tmpdir>`
      and inspected `sce-next-task/references/context-sync.md` and
      `sync-report.md` for Claude, Pi, and OpenCode targets — all three show
      the completed-task-record input/validation/write-back wording and are
      byte-identical (`diff` empty). `rg -n "Context synchronization
      handoff|persisted handoff|live or persisted" <tmpdir>` returned no
      matches anywhere in the generated tree.
    - `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl`
      and `nix run .#pkl-check-generated` both still fail on the same
      pre-existing `handoff-identity-fields` throw T01 and T02 already
      recorded as out of scope, owned by T04; confirmed unchanged by stashing
      this task's edit and reproducing the identical failure beforehand, so
      this task neither introduced nor worsened it.
  - Context synchronization: synced

- [x] T04: `Update generation-contract checks and fixtures for the compact task record` (status:done)
  - Task ID: T04
  - Goal: Update `config/pkl/renderers/generation-contract-check.pkl` —
    removing or rewriting the `handoff-identity-fields` check and any
    assertion expecting `Goal`, `Boundaries (in/out of scope)`,
    `Verification notes`, `Implementation evidence`, `Verification
    evidence`, or `Context synchronization handoff` — and add semantic
    checks plus focused negative fixtures asserting: the compact fields
    (`Scope`, `Done when`, `Verify`, `Result`, `Files changed`,
    `Context impact`, `Context synchronization`) are present where expected;
    the removed fields/sections are absent from generated task-schema and
    workflow instructions; `/next-task` persists execution facts directly;
    sync-debt recovery reads the completed task record; and task context
    synchronization validates the completed task record rather than a
    handoff.
  - Boundaries (in/out of scope): In — `generation-contract-check.pkl`
    checks/assertions, their registration, and any new negative fixture
    files the existing `assertX + fixture + check-generated.sh` pattern
    requires. Out — non-generation-contract renderer files, and any
    target-specific frontmatter/permission checks unrelated to the
    task-record schema.
  - Dependencies: T03
  - Done when: `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl`
    succeeds; `nix run .#pkl-check-generated` passes with the new/updated
    checks active; the old duplicated-field checks (including
    `handoff-identity-fields`) are removed or rewritten to match the compact
    model, with no check asserting the old field names as required content.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl`;
    `nix run .#pkl-check-generated`; confirm each new negative fixture fails
    the check it targets before the corresponding contract fix and passes
    after.
  - Completed: 2026-08-14
  - Files changed: config/pkl/renderers/generation-contract-check.pkl;
    config/pkl/check-generated.sh;
    config/pkl/renderers/fixtures/handoff-identity-fields-check.pkl (deleted);
    config/pkl/renderers/fixtures/compact-plan-template-schema-check.pkl (new);
    config/pkl/renderers/fixtures/next-task-compact-completion-writing-check.pkl (new);
    config/pkl/renderers/fixtures/plan-review-reads-completed-record-check.pkl (new);
    config/pkl/renderers/fixtures/context-sync-validates-task-record-check.pkl (new)
  - Result: Replaced the single `handoff-identity-fields` check with four
    focused checks matching the compact model: `compact-plan-template-schema`
    (asserts the plan template's new-task and completion examples contain the
    compact-schema tokens and none of the six removed legacy field names),
    `next-task-compact-completion-writing` (asserts `task-execution.md`
    records execution facts directly on the task with no separate handoff/
    evidence construction), `plan-review-reads-completed-record` (asserts
    `plan-review.md`'s sync-debt recovery reads the completed task record by
    plan path and task ID rather than a persisted handoff), and
    `context-sync-validates-task-record` (asserts `context-sync.md` validates
    the completed task record, not a persisted handoff). Added one negative
    fixture per check, each importing the contract and asserting the check
    throws its exact diagnostic against a document containing the disallowed
    old-format content, and registered all four via `expect_pkl_fixture_failure`
    in `check-generated.sh` in place of the removed
    `handoff-identity-fields-check.pkl` registration. Deleted the obsolete
    `handoff-identity-fields-check.pkl` fixture.
  - Context impact: none — this task changes only ephemeral-generation source
    (`config/pkl/**`); durable `context/` doc updates are T05's responsibility.
  - Verify:
    - `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl` —
      passed; all 32 registered checks, including the four new ones, evaluate
      successfully.
    - `nix run .#pkl-check-generated` — passed (exit 0); ephemeral generation
      produced 107 files and every `expect_pkl_fixture_failure` assertion,
      including the four new fixtures, matched its expected diagnostic.
    - `rg` (via `nix run nixpkgs#ripgrep`) confirmed no remaining reference to
      `handoff-identity-fields`/`assertHandoffIdentityFields` anywhere in
      `config/pkl/`, and confirmed the six legacy field-name tokens
      (`Goal:`, `Boundaries (in/out of scope)`, `Verification notes`,
      `Implementation evidence`, `Verification evidence`,
      `Context synchronization handoff`) appear in
      `generation-contract-check.pkl` only inside the forbidden-token listing
      and `!text.contains(...)` absence assertions, never as required content.
  - Context synchronization: synced

- [x] T05: `Describe the compact task-record model in durable context` (status:done)
  - Task ID: T05
  - Goal: Update current-state `context/` documentation (at minimum
    `context/overview.md`, `context/sce/shared-context-plan-workflow.md`,
    `context/sce/shared-context-code-workflow.md`, and the
    `context/glossary.md` `baseline-relative task handoff` entry) to
    describe the completed task record as the sole durable
    synchronization/recovery input, with no remaining current-state claim
    that a separate `Context synchronization handoff` must be persisted, and
    write a new ADR under `context/decisions/` superseding the
    handoff-shape portion of
    `2026-08-12-persist-workflow-sync-lifecycle-in-plans.md` while
    preserving its `pending`/`synced`/`blocked` lifecycle-state decision.
  - Boundaries (in/out of scope): In — the current-state prose named above
    plus one new dated ADR. Out — historical `context/plans/` files,
    `context/handovers/`, other ADRs not describing the handoff shape, and
    any code change (this task is documentation-only).
  - Dependencies: T04
  - Done when: The listed current-state docs describe completed-task-record
    recovery with no remaining reference to a persisted
    `Context synchronization handoff` as the recovery mechanism; the new
    ADR exists, states what it supersedes, and preserves the
    lifecycle-state invariant; `nix flake check` and
    `nix run .#pkl-check-generated` both pass.
  - Verification notes (commands or checks): `rg -n "Context synchronization handoff" context/`
    (expect no current-state hits outside historical plan/decision files);
    `nix run .#pkl-check-generated`; `nix flake check`.
  - Completed: 2026-08-14
  - Files changed: context/overview.md; context/sce/shared-context-code-workflow.md;
    context/glossary.md; context/decisions/2026-08-14-compact-task-record-supersedes-handoff.md
  - Result: Updated `context/overview.md`'s `/next-task` synchronization paragraph
    to state that execution facts are recorded directly on the completed task and
    that sync-debt retry reads that same record by plan path and task ID, with no
    separate persisted handoff. Rewrote the `baseline-relative task handoff` entry
    in `context/glossary.md` so its persisted form is described as the completed
    task record itself (`Completed`/`Files changed`/`Result`/`Verify`/`Context
    impact`), dropping the old `Context synchronization handoff` field reference.
    Updated `context/sce/shared-context-code-workflow.md`'s purpose paragraph and
    the `sce-plan-review` phase bullets (legacy-detection condition, `sync_debt`
    naming, and sync-debt-recovery wording) to match the exact generated wording
    in `sce-next-task/references/{plan-review,context-sync}.md` — reading the
    completed task record directly rather than a persisted handoff.
    `context/sce/shared-context-plan-workflow.md` was inspected and needed no
    change: it never named the handoff field. Added
    `context/decisions/2026-08-14-compact-task-record-supersedes-handoff.md`,
    which supersedes only the handoff-shape portion of
    `2026-08-12-persist-workflow-sync-lifecycle-in-plans.md` (status left
    `Accepted`, matching the same partial-supersession precedent set by
    `2026-08-13-validate-validation-only.md`) while explicitly leaving that
    decision's `pending`/`synced`/`blocked` lifecycle-state invariant in force.
    Also staged (via `git add`, no content change) T04's four new fixture files
    under `config/pkl/renderers/fixtures/`, which were left untracked and caused
    `nix flake check`'s git-sourced `pkl-generated` derivation to fail on a
    missing-module error; this was required for this task's own mandated
    `nix flake check` verification to pass and made no code or doc change.
  - Context impact: none — this task itself performs the current-state
    documentation update; no further context work follows from it.
  - Verify:
    - `rg -n "Context synchronization handoff" context/` (via `nix run
      nixpkgs#ripgrep`) — the only current-state hit outside historical
      plan/decision files is `context/architecture.md`'s description of the
      generation-contract check that asserts the field's absence from generated
      output (a forbidden-token listing, not a current-state persistence claim),
      which is expected and unchanged.
    - `nix run .#pkl-check-generated` — passed (exit 0), 107 files, matching the
      inventory hash already established by T04.
    - `nix flake check` — passed (`all checks passed!`) after staging T04's
      untracked fixture files described above; before staging, the
      `pkl-generated` check failed on a missing-module error unrelated to this
      task's own edits.
  - Context synchronization: synced

## Open questions

None. The change request fully specifies the target schema, the file-by-file
scope, the explicit non-goals, and the acceptance criteria.

## Validation Report

**Status:** validated  
**Date:** 2026-08-14

### Commands run

- `nix flake check` -> exit 0 (all checks passed)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral generation passed: 107 files, inventory sha256 a1adb1667e2675dcdfba5353518f802d791859ea237be6135e05c66ef3157f42)
- `nix develop -c pkl eval config/pkl/base/workflow-change-to-plan.pkl` -> exit 0
- `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl` -> exit 0
- `nix develop -c pkl eval config/pkl/base/workflow-context-sync.pkl` -> exit 0
- `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl` -> exit 0
- `nix run .#pkl-generate -- <tmpdir>` -> exit 0 (ephemeral payload generated for acceptance-criteria inspection)

### Success-criteria verification

- [x] AC1: New-task shape uses the compact schema -> generated `sce-change-to-plan/references/plan-template.md` (Claude target) shows the template and filled-in example using `Task ID`/`Scope`/`Dependencies`/`Done when`/`Verify`/`Context synchronization`; `grep` for `Goal:`, `Boundaries (in/out of scope)`, and `Verification notes` returned no matches in the file.
- [x] AC2: Completed-task execution facts recorded once, no separate evidence sections -> generated `plan-template.md`'s "Completion records" section and `sce-next-task/references/task-execution.md` §2.7 record `Completed`, `Files changed`, `Result`, `Verify` outcomes, and `Context impact` directly on the task; `grep` for `Implementation evidence` and `Context synchronization handoff` returned no matches in either file (one unrelated lowercase "verification evidence" phrase remains in `task-execution.md` §2.8 describing the internal handoff-state contract, not a persisted plan-file field).
- [x] AC3: No `Context synchronization handoff` in newly generated plans/instructions -> `rg -n "Context synchronization handoff" <ephemeral generated payload root>` returned no matches (exit 1).
- [x] AC4: Task context synchronization reads the completed task record directly for both immediate sync and sync-debt recovery -> generated `sce-next-task/references/context-sync.md` (§3.1, §3.8) and `plan-review.md` (§1.2, §1.5) both read/write the completed task record by plan path and task ID, with no separate handoff structure.
- [x] AC5: Blocked context sync adds only synchronization-specific blocker metadata -> generated `context-sync.md` §3.8 writes plan/task identity plus a `Context synchronization blocker` section (`Blocker`/`Required action`/`Retry condition`) using the same field names as the plan's completion record; `plan-template.md`'s completion-record example shows the identical blocked shape.
- [x] AC6: Pi, Claude, and OpenCode packages express the same compact model -> `diff -q` across all three targets for `plan-template.md`, `task-execution.md`, `context-sync.md`, `plan-review.md`, and `sync-report.md` reported no differences.
- [x] AC7: Generation-contract checks and fixtures prevent regressions -> `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl` passed; `nix run .#pkl-check-generated` passed with all fixture-based checks (including the four new ones from T04) active.
- [x] AC8: Durable `context/` documentation has no current-state handoff claim -> `rg -n "Context synchronization handoff" context/overview.md context/sce/*.md context/glossary.md` returned no matches (exit 1).

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
