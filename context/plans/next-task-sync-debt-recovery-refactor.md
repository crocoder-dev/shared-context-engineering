# Plan: next-task-sync-debt-recovery-refactor

## Change summary

Fixes five remaining review findings against PR #205 in the canonical Pkl
workflow sources under `config/pkl/base/**` and `config/pkl/renderers/**`, the
sole source of truth for the generated `.opencode`, `.claude`, and `.pi`
`sce-next-task` package. This extends the synchronization-lifecycle and
handoff-persistence behavior the already-validated
`sync-handoff-recovery-and-generation-fixes` plan established; it does not
replace the `pending`/`synced`/`blocked` model or introduce a second
persistence mechanism.

1. The persisted "Context synchronization handoff" carries changed files,
   implementation summary, verification, done checks, and context impact, but
   task-context-sync's own validation still requires a `plan` object with a
   `path` and exactly one identified task — fields the persisted handoff does
   not itself carry. A cross-session retry currently depends on plan-review
   supplying that identity out of band rather than the handoff being
   self-contained. This plan adds the resolved plan path and task ID/title to
   the persisted handoff and requires them at validation.
2. `sce-plan-review` currently detects earlier-completed-task synchronization
   debt and then invokes the Task context synchronization phase itself,
   without ever citing `references/context-sync.md` — the file that owns that
   phase's steps and boundaries, per the "read a step's reference before
   running it" rule stated at the top of every generated `SKILL.md`. This
   plan turns plan review back into a read-only detect-and-report phase and
   adds an explicit top-level recovery branch in `/next-task` that loads
   `references/context-sync.md` before invoking the phase it owns.
3. The debt scan is currently scoped to tasks *earlier* than the one being
   selected, which does not match the accepted lifecycle invariant recorded in
   `2026-08-12-persist-workflow-sync-lifecycle-in-plans.md`: no new
   implementation may start while *any* completed task has unresolved
   synchronization debt, regardless of its position relative to the task being
   selected. This plan widens the scan to every completed task in plan order.
4. When recovery's own call to task-context-sync returns `blocked`, the
   workflow currently reports it through the generic **Review blocked**
   layout (plan review's own blocked branch) instead of the existing
   **Context synchronization blocked** layout, discarding the sync report's
   blocker/required-action/retry-condition shape. This plan routes a
   recovery-time block through the sync-specific layout.
5. None of the above has a semantic generation-contract check or fixture, so a
   future edit can silently regress any of them. This plan adds one check per
   behavior, following the existing `assertX` + fixture + `check-generated.sh`
   registration pattern already used for the prior plan's checks.

## Acceptance criteria

- [x] AC1: A persisted "Context synchronization handoff" carries `Plan path`,
  `Task ID`, and `Task title` fields alongside the existing changed
  files/summary/verification/done-checks/context-impact fields, and
  task-context-sync's handoff validation requires those three fields to be
  present in the handoff itself — not supplied out of band by the caller —
  before treating a persisted retry handoff as valid.
  - Validate: generated `sce-change-to-plan/references/plan-template.md`
    completion-record example shows `Plan path`, `Task ID`, and `Task title`
    inside the `Context synchronization handoff` line; generated
    `sce-next-task/references/task-execution.md` instructs writing them;
    generated `sce-next-task/references/context-sync.md` step "Validate the
    handoff" lists them as required fields for a persisted handoff.
- [x] AC2: Sync-debt recovery is an explicit top-level `/next-task` workflow
  branch, not behavior hidden inside plan review. The branch is reached
  before normal task selection when debt is detected, explicitly states
  reading `references/context-sync.md` before invoking the Task context
  synchronization phase, writes `synced` or refreshed `blocked` lifecycle
  state to the plan, and then either resumes plan review (to select a task
  normally) or stops. `references/plan-review.md` no longer instructs
  invoking the Task context synchronization phase itself; it only detects and
  reports debt.
  - Validate: generated `sce-next-task/SKILL.md` contains a recovery branch
    that cites `references/context-sync.md` before any instruction to run the
    Task context synchronization phase; generated
    `sce-next-task/references/plan-review.md` no longer contains an
    instruction to run that phase; new semantic check (name chosen by the
    implementing task) plus its negative fixture pass.
- [x] AC3: The synchronization-debt scan inspects every completed task in
  plan order before allowing a new implementation task to start, not only
  tasks earlier than the one being selected or resumed.
  - Validate: generated `sce-next-task/references/plan-review.md` states the
    scan covers every completed task regardless of position, with no
    "earlier completed task" position-relative scoping language remaining;
    new semantic check plus negative fixture (using the old
    position-scoped wording) pass.
- [x] AC4: When the recovery branch's own call to task-context-sync returns
  `blocked`, `/next-task` renders the existing **Context synchronization
  blocked** layout — preserving the sync report's blocker, required action,
  retry condition, and preserved context edits — not the generic **Review
  blocked** layout.
  - Validate: generated `sce-next-task/SKILL.md` routes the recovery branch's
    `blocked` outcome to the **Context synchronization blocked** layout
    citation, distinct from plan review's own `blocked` branch; new semantic
    check plus negative fixture pass.
- [x] AC5: One semantic generation-contract check and one negative fixture
  exists for each of AC1's validation requirement, AC2's reference-loading
  requirement, AC3's all-completed-tasks scope, and AC4's blocked-output
  routing, registered in `config/pkl/check-generated.sh` alongside the
  existing checks.
  - Validate: `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl`
    passes with the new checks present; each new negative fixture throws when
    evaluated directly; `nix run .#pkl-check-generated` passes.
- [x] AC6: Every claim in this plan's Context sync files still matches the
  regenerated behavior across all three targets, and no target has fallen out
  of parity.
  - Validate: a temporary full regeneration (`nix run .#pkl-generate -- "$(mktemp -d)"`)
    shows byte-identical `sce-next-task` package content under `.claude`,
    `.opencode`, and `.pi` apart from supported per-target frontmatter.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/sce/shared-context-code-workflow.md` (`/next-task` phase-ownership
  section: recovery is a top-level branch, not plan-review behavior; the scan
  covers all completed tasks, not only earlier ones)
- `context/architecture.md` (generation-contract-check semantic-check count
  and description)
- `context/glossary.md` (`baseline-relative task handoff` entry: add plan
  path and task identity to what the handoff carries)
- `context/decisions/2026-08-12-persist-workflow-sync-lifecycle-in-plans.md`
  (reference only; this plan implements that decision's already-recorded
  "any completed task" invariant more precisely and does not change the
  decision itself)

## Context synchronization lifecycle

- **Plan context synchronization:** synced

## Constraints and non-goals

- **In scope:** `config/pkl/base/workflow-next-task.pkl`,
  `config/pkl/base/workflow-context-sync.pkl`,
  `config/pkl/base/workflow-change-to-plan.pkl`,
  `config/pkl/renderers/generation-contract-check.pkl`,
  `config/pkl/renderers/fixtures/*.pkl`, and the fixture registration list in
  `config/pkl/check-generated.sh`. Ephemeral regeneration of `.opencode`,
  `.claude`, and `.pi` outputs through the normal generation pipeline.
  Durable context updates limited to the files named under Context sync.
- **Out of scope:** `sce-validate`'s plan-role synchronization (`/validate`
  never runs sync-debt recovery); `sce-handover`, `sce-brownfield`, and
  `sce-decision` packages; any CLI Rust code under `cli/`; the bypass-commit
  temp-file rule and the layout-reference checker fix (already resolved by
  `sync-handoff-recovery-and-generation-fixes`); optional-workflow
  install-time semantics.
- **Constraints:** `config/pkl/**` remains the sole source of truth; never
  hand-edit generated `.pi/**`, `.claude/**`, or `.opencode/**` output; keep
  the plan/completion-record Markdown format consistent with
  `references/plan-template.md`; keep package-vs-composite rendering parity;
  keep the `pending | synced | blocked` lifecycle values and the
  Markdown-only persistence mechanism unchanged.
- **Non-goal:** Redesigning the synchronization lifecycle beyond making the
  persisted handoff self-contained and making recovery an explicit,
  reference-citing workflow branch; changing plan-role (`/validate`)
  synchronization; adding new lifecycle values beyond
  `pending`/`synced`/`blocked`.

## Task stack

- [x] T01: `Persist plan path and task identity in the synchronization handoff` (status:done)
  - Task ID: T01
  - Goal: In `config/pkl/base/workflow-change-to-plan.pkl`'s plan template,
    add `Plan path`, `Task ID`, and `Task title` fields to the
    `Context synchronization handoff` completion-record line, and in
    `config/pkl/base/workflow-next-task.pkl`'s task-execution phase (2.7
    "Update the plan"), instruct writing those three fields into the handoff
    subsection alongside the existing changed-files/summary/verification/
    done-checks/context-impact fields.
  - Boundaries (in/out of scope): In — the plan-template completion-record
    example (package and composite) and the task-execution write instruction
    (package and composite). Out — context-sync's read/validation side
    (T02); plan-review's read side (T03).
  - Dependencies: none
  - Done when: `nix develop -c pkl eval config/pkl/base/workflow-change-to-plan.pkl`
    and `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl`
    succeed; a temporary generation shows the completion-record example with
    `Plan path`, `Task ID`, and `Task title` inside the handoff line, and
    `sce-next-task/references/task-execution.md` instructs writing them.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/base/workflow-change-to-plan.pkl`; `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl`; targeted generation and grep for the three field names in both files.
  - Context synchronization: synced
  - Context synchronization handoff: Plan path: context/plans/next-task-sync-debt-recovery-refactor.md; Task ID: T01; Task title: Persist plan path and task identity in the synchronization handoff; Changed files: config/pkl/base/workflow-change-to-plan.pkl, config/pkl/base/workflow-next-task.pkl; Implementation summary: Added `Plan path`, `Task ID`, and `Task title` fields to the `Context synchronization handoff` completion-record line in both the composite and package renderings of `workflow-change-to-plan.pkl`'s plan template, and updated the "Update the plan" (2.7) write instruction in both the composite and package renderings of `workflow-next-task.pkl` to state the handoff subsection includes the resolved plan path and task ID/title alongside the existing fields; Verification: `nix develop -c pkl eval config/pkl/base/workflow-change-to-plan.pkl` passed, `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl` passed, temporary `nix run .#pkl-generate` confirmed all three targets (`.claude`, `.opencode`, `.pi`) render the new fields in `sce-change-to-plan/references/plan-template.md` and `sce-next-task/references/task-execution.md`; Done checks: both pkl evals succeed (done), generated plan template and task-execution.md show the three fields (done); Context impact: none — this task only changes generation-source Pkl content whose downstream durable-context implications are covered by T06's closing regeneration/reconciliation pass
  - Completed: 2026-08-13
  - Files changed: config/pkl/base/workflow-change-to-plan.pkl, config/pkl/base/workflow-next-task.pkl
  - Evidence: `nix develop -c pkl eval config/pkl/base/workflow-change-to-plan.pkl` (exit 0); `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl` (exit 0); `nix run .#pkl-generate -- <tmpdir>` (exit 0) followed by grep confirming `Plan path: {path}; Task ID: {id}; Task title: {title}` in `sce-change-to-plan/references/plan-template.md` and "the resolved plan path, the task ID and title" in `sce-next-task/references/task-execution.md` across `.claude`, `.opencode`, and `.pi`
  - Notes: none

- [x] T02: `Require persisted plan/task identity at context-sync validation and in the blocked-report shape` (status:done)
  - Task ID: T02
  - Goal: In the task role of `config/pkl/base/workflow-context-sync.pkl`,
    require `Plan path`, `Task ID`, and `Task title` as present-in-the-handoff
    fields at "Validate the handoff" (3.1) for a persisted retry handoff — so
    validation succeeds using only the handoff text itself, without depending
    on an out-of-band plan/task value supplied by the caller — and update the
    blocked-report rendering to name those fields consistently with T01's
    plan-template shape.
  - Boundaries (in/out of scope): In — task-role "Validate the handoff" step
    and blocked-report rendering in `workflow-context-sync.pkl` (package and
    composite). Out — plan-role (`/validate`) synchronization, unaffected.
  - Dependencies: T01
  - Done when: `nix develop -c pkl eval config/pkl/base/workflow-context-sync.pkl`
    succeeds; generated `sce-next-task/references/context-sync.md` lists
    `Plan path`, `Task ID`, and `Task title` as required fields of a
    persisted handoff at validation, and its blocked-report section names
    them.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/base/workflow-context-sync.pkl`; targeted generation and grep of `sce-next-task/references/context-sync.md` for the new required-field wording.
  - Context synchronization: synced
  - Context synchronization handoff: Plan path: context/plans/next-task-sync-debt-recovery-refactor.md; Task ID: T02; Task title: Require persisted plan/task identity at context-sync validation and in the blocked-report shape; Changed files: config/pkl/base/workflow-context-sync.pkl; Implementation summary: In `config/pkl/base/workflow-context-sync.pkl`'s task role, updated step 1 "Validate the handoff" (both the mode-based composite/package render function and the standalone `taskReference` string that generates `sce-next-task/references/context-sync.md`) to state that a persisted retry handoff satisfies the plan-identity and task-identity requirements via its own `Plan path`, `Task ID`, and `Task title` fields rather than an out-of-band value from the caller. Updated step 8's blocked-report description in both renderings to name `Plan path`, `Task ID`, and `Task title` alongside the existing changed-files/summary/verification/done-checks/context-impact fields. Added `Plan path`, `Task ID`, and `Task title` bullets to `taskReport.blockedHandoffSection`, which renders the actual blocked-report template in `references/sync-report.md`, matching T01's plan-template field order; Verification: `nix develop -c pkl eval config/pkl/base/workflow-context-sync.pkl` passed; temporary `nix run .#pkl-generate` confirmed `sce-next-task/references/context-sync.md` and `references/sync-report.md` carry the new wording/fields identically across `.claude`, `.opencode`, and `.pi`; `nix run .#pkl-check-generated` passed; Done checks: pkl eval succeeds (done), generated context-sync.md lists the three required fields at validation and names them in the blocked-report section (done); Context impact: none — this task only changes generation-source Pkl content whose downstream durable-context implications are covered by T06's closing regeneration/reconciliation pass
  - Completed: 2026-08-13
  - Files changed: config/pkl/base/workflow-context-sync.pkl
  - Evidence: `nix develop -c pkl eval config/pkl/base/workflow-context-sync.pkl` (exit 0); `nix run .#pkl-generate -- <tmpdir>` (exit 0) followed by grep confirming "carries its own `Plan path` field" and "carries its own `Task ID` and `Task title` fields" in `sce-next-task/references/context-sync.md`, and `Plan path: {plan path}` / `Task ID: {task id}` / `Task title: {task title}` bullets in `references/sync-report.md`, byte-identical across `.claude`, `.opencode`, and `.pi`; `nix run .#pkl-check-generated` (exit 0, "Ephemeral Pkl generation passed: 107 files")
  - Notes: none

- [x] T03: `Turn plan review back into a read-only debt detector covering every completed task` (status:done)
  - Task ID: T03
  - Goal: In the plan-review phase of `config/pkl/base/workflow-next-task.pkl`,
    change the synchronization-debt scan (1.2 "Resolve one task") to inspect
    every completed task's `Context synchronization` field in plan order —
    not only tasks earlier than the one being selected — and stop invoking
    the Task context synchronization phase directly. When the first task
    carrying debt has no durable `Context synchronization handoff`
    subsection, still return `blocked` immediately with the legacy-migration
    required action (this needs no recovery invocation). Otherwise, return a
    new status (for example `sync_debt`) naming the debt task, its persisted
    handoff, and its persisted blocker when present, without running or
    citing the Task context synchronization phase. Restore plan review's
    "reads; never writes" framing now that it no longer performs recovery
    writes itself.
  - Boundaries (in/out of scope): In — plan-review phase steps in
    `workflow-next-task.pkl` (package and composite): the debt-scan scope and
    the return-status change. Out — the new top-level recovery branch that
    consumes the `sync_debt` status (T04); context-sync's own behavior (T02).
  - Dependencies: T01, T02
  - Done when: `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl`
    succeeds; generated `sce-next-task/references/plan-review.md` states the
    scan covers every completed task regardless of position, no longer
    instructs running the Task context synchronization phase, and still
    states the legacy-no-handoff blocked case with its migration required
    action.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl`; targeted generation and grep of `sce-next-task/references/plan-review.md` for "every completed task" and the absence of Task-context-synchronization-phase invocation wording.
  - Context synchronization: synced
  - Context synchronization handoff: Plan path: context/plans/next-task-sync-debt-recovery-refactor.md; Task ID: T03; Task title: Turn plan review back into a read-only debt detector covering every completed task; Changed files: config/pkl/base/workflow-next-task.pkl; Implementation summary: In `config/pkl/base/workflow-next-task.pkl`'s `nextTaskPlanReviewReference` literal (the sole source of the generated `sce-next-task/references/plan-review.md`, confirmed byte-identical across `.claude`, `.opencode`, and `.pi`), widened the synchronization-debt scan in "1.2 Resolve one task" from "earlier completed task" to "every completed task ... regardless of its position relative to the task being selected or resumed"; replaced the direct "Run the **Task context synchronization phase**" invocation with a new `sync_debt` internal status that names the debt task's ID/title, its persisted `Context synchronization handoff`, and its persisted `Context synchronization blocker` when present, without running or citing the Task context synchronization phase; added `sync_debt` to "1.5 Return the result"'s internal-state list with its own required-fields bullet; and restored plan review's read-only "Plan review boundaries" framing by removing the "except to persist a synchronization-debt recovery outcome" and "except to retry ... unresolved synchronization debt" exceptions, and changed the phase's opening description from "It reads, and writes only to persist a synchronization-debt recovery outcome for an earlier completed task, per 1.2." to "It reads; it never writes." Left the parallel mode-based `renderReviewSkillBody`/`REVIEW_SKILL`/`planReviewPackage` scaffold in the same file untouched: confirmed dead code, since its `workflow.skills` mapping is never read by any of the three target renderers or by `generation-contract-check.pkl`, and its distinctive intro text does not appear anywhere in currently generated output; Verification: `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl` passed; `nix run .#pkl-generate -- <tmpdir>` confirmed the generated `sce-next-task/references/plan-review.md` is byte-identical across `.claude`, `.opencode`, and `.pi` (md5sum 7f331ce36149070e7afb1608c5b7ced7), states the scan covers "every completed task" with no "earlier completed task" wording remaining, and no longer contains "Run the **Task context synchronization phase**"; `nix run .#pkl-check-generated` passed ("Ephemeral Pkl generation passed: 107 files"), including the pre-existing `next-task-sync-debt-recovery-check.pkl` fixture; Done checks: pkl eval succeeds (done), generated plan-review.md states the scan covers every completed task regardless of position (done), no longer instructs running the Task context synchronization phase (done), still states the legacy-no-handoff blocked case with its migration required action (done, unchanged); Context impact: none — this task only changes generation-source Pkl content whose downstream durable-context implications (shared-context-code-workflow.md's phase-ownership description) are deferred to T06's closing regeneration/reconciliation pass, once T04's top-level recovery branch completes the described behavior
  - Completed: 2026-08-13
  - Files changed: config/pkl/base/workflow-next-task.pkl
  - Evidence: `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl` (exit 0); `nix run .#pkl-generate -- <tmpdir>` (exit 0) followed by grep confirming "every completed task" wording and the absence of "earlier completed task" / direct sync-phase-invocation wording in `sce-next-task/references/plan-review.md` across `.claude`, `.opencode`, and `.pi` (byte-identical, md5sum 7f331ce36149070e7afb1608c5b7ced7); `nix run .#pkl-check-generated` (exit 0, "Ephemeral Pkl generation passed: 107 files")
  - Notes: none

- [x] T04: `Add an explicit sync-debt-recovery branch to /next-task that loads context-sync.md and routes its own blocked result to the sync-specific layout` (status:done)
  - Task ID: T04
  - Goal: In the composed `SKILL.md` workflow section of
    `config/pkl/base/workflow-next-task.pkl`, add a top-level branch reached
    when plan review returns the `sync_debt` status from T03, positioned
    before normal task selection resumes. The branch must explicitly state
    reading `references/context-sync.md` before invoking the Task context
    synchronization phase with the persisted handoff, write `synced`
    (clearing blocker fields) or a refreshed `blocked` lifecycle state to the
    plan depending on the result, and then either re-invoke plan review to
    resume normal task selection (on success) or render the existing
    **Context synchronization blocked** layout and stop (on a renewed
    block) — not the **Review blocked** layout plan review's own `blocked`
    branch uses.
  - Boundaries (in/out of scope): In — the top-level `Workflow` section of
    the composed `SKILL.md` in `workflow-next-task.pkl` (package and
    composite): the new branch, its reference-loading statement, its
    lifecycle-state write, and its output routing. Out — plan review's own
    detection logic (T03, already complete); context-sync's own steps (T02,
    already complete).
  - Dependencies: T03
  - Done when: `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl`
    succeeds; generated `sce-next-task/SKILL.md` contains the recovery branch
    citing `references/context-sync.md` before invoking the Task context
    synchronization phase, and its `blocked` outcome renders the
    **Context synchronization blocked** layout distinctly from plan review's
    `blocked` branch.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl`; targeted generation and inspection of `sce-next-task/SKILL.md`'s Workflow section for the new branch, its reference citation, and its output routing.
  - Context synchronization: synced
  - Context synchronization handoff: Plan path: context/plans/next-task-sync-debt-recovery-refactor.md; Task ID: T04; Task title: Add an explicit sync-debt-recovery branch to /next-task that loads context-sync.md and routes its own blocked result to the sync-specific layout; Changed files: config/pkl/base/workflow-content.pkl; Implementation summary: The composed composite `SKILL.md` body for `/next-task` is not stored in `workflow-next-task.pkl` itself — it is the `nextTaskSkillBody` literal in `config/pkl/base/workflow-content.pkl`, consumed directly as `compositeSkillBody` by `config/pkl/renderers/workflow-composite.pkl`'s `renderSkill`, bypassing the dead package-mode scaffold in `workflow-next-task.pkl` that T03 already confirmed is unread by every target renderer. Edited `nextTaskSkillBody`'s step-1 "Branch on `status`" list to add a new `sync_debt` branch, positioned between the existing `blocked` and `plan_complete` branches: it states reading `references/context-sync.md` before running the **Task context synchronization phase** with the debt task's persisted `Context synchronization handoff` (and persisted `Context synchronization blocker` when present) named by the **Plan review phase**; it writes the debt task's lifecycle to the plan (`synced` clearing blocker/required-action/retry-condition, or a refreshed `blocked` state) before branching on the outcome; its `blocked` outcome renders the existing **Context synchronization blocked** layout from `references/output.md`, explicitly distinguished from the **Review blocked** layout used by plan review's own `blocked` branch, and stops without selecting another task; its `synced`/`no_context_change` outcome re-invokes the **Plan review phase** to resume normal task selection. Also updated the immediately adjacent stale summary paragraph (previously "every earlier completed task ... blocks implementation until the plan records the blocker") to match T03's already-implemented behavior: the scan covers every completed task regardless of position, a legacy handoff-less debt task still returns `blocked` directly, and any other debt task returns `sync_debt` resolved by the new branch — this paragraph directly restated the same mechanism the new branch depends on, so leaving the pre-T03 wording in place would have made the section self-contradictory. Left the dead package-mode scaffold in `workflow-next-task.pkl` (confirmed unread by any renderer, per T03) untouched, consistent with T03's precedent; Verification: `nix develop -c pkl eval config/pkl/base/workflow-content.pkl` passed, `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl` passed, temporary `nix run .#pkl-generate -- <tmpdir>` confirmed generated `sce-next-task/SKILL.md` across `.claude`, `.opencode`, and `.pi` carries byte-identical bodies (frontmatter differing only in the per-target `compatibility` line) containing the `sync_debt` branch that cites `references/context-sync.md` before invoking the Task context synchronization phase and routes its `blocked` outcome to the **Context synchronization blocked** layout distinctly from **Review blocked**, `nix run .#pkl-check-generated` passed ("Ephemeral Pkl generation passed: 107 files"), `nix flake check` passed ("all checks passed!"); Done checks: both pkl evals succeed (done), generated `sce-next-task/SKILL.md` contains the recovery branch citing `references/context-sync.md` before invoking the Task context synchronization phase (done), its `blocked` outcome renders the **Context synchronization blocked** layout distinctly from plan review's `blocked` branch (done); Context impact: none — this task only changes generation-source Pkl content whose downstream durable-context implications (shared-context-code-workflow.md's phase-ownership description) are deferred to T06's closing regeneration/reconciliation pass
  - Completed: 2026-08-13
  - Files changed: config/pkl/base/workflow-content.pkl
  - Evidence: `nix develop -c pkl eval config/pkl/base/workflow-content.pkl` (exit 0); `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl` (exit 0); `nix run .#pkl-generate -- <tmpdir>` (exit 0) followed by diff confirming byte-identical `sce-next-task/SKILL.md` bodies across `.claude`, `.opencode`, and `.pi`, each containing the new `sync_debt` branch, its `references/context-sync.md` citation, and its distinct **Context synchronization blocked** routing; `nix run .#pkl-check-generated` (exit 0, "Ephemeral Pkl generation passed: 107 files"); `nix flake check` (exit 0, "all checks passed!")
  - Notes: The plan's Goal/Boundaries text for this task named `config/pkl/base/workflow-next-task.pkl` as the file carrying "the composed `SKILL.md` workflow section"; the actual source is `nextTaskSkillBody` in `config/pkl/base/workflow-content.pkl` (confirmed via `config/pkl/renderers/workflow-composite.pkl`'s `renderSkill`, which uses `structuredSource.compositeSkillBody` directly whenever it is non-null, which it is for `next-task`). Treated as an ordinary local implementation-detail correction rather than a blocking scope question, since the required generated-output change was unambiguous from the acceptance criteria and `config/pkl/**` remains the sole source of truth either way.

- [x] T05: `Add semantic generation-contract checks and fixtures for the handoff, reference-loading, all-tasks-scope, and blocked-routing behaviors` (status:done)
  - Task ID: T05
  - Goal: In `config/pkl/renderers/generation-contract-check.pkl`, add one
    semantic check per behavior added by T01-T04: (a) a persisted handoff
    example and context-sync's validation wording both name `Plan path`,
    `Task ID`, and `Task title`; (b) the generated `sce-next-task/SKILL.md`
    recovery branch cites `references/context-sync.md` before any instruction
    to run the Task context synchronization phase; (c)
    `sce-next-task/references/plan-review.md` states the debt scan covers
    every completed task, with no surviving "earlier completed task"
    position-relative wording; (d) the recovery branch's `blocked` outcome
    resolves to the **Context synchronization blocked** layout citation, not
    **Review blocked**. Add one negative fixture per check (following the
    existing `assertX` + fixture-stub + `check-generated.sh` pattern already
    used for `plan-review-sync-debt-recovery` and
    `wrong-file-layout-reference-check`), and register each in
    `config/pkl/check-generated.sh`.
  - Boundaries (in/out of scope): In — the four new `assertX` functions,
    their fixtures under `config/pkl/renderers/fixtures/`, and their
    `check-generated.sh` registration. Out — any existing check or fixture
    not touched by this plan.
  - Dependencies: T04
  - Done when: `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl`
    succeeds with all four new checks present; each new negative fixture
    throws when evaluated directly with a diagnostic naming the missing
    behavior; the real generated output passes all four; `nix run .#pkl-check-generated`
    passes.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl`; `nix develop -c pkl eval config/pkl/renderers/fixtures/<each-new-fixture>.pkl` (expect throw); `nix run .#pkl-check-generated`.
  - Context synchronization: synced
  - Context synchronization handoff: Plan path: context/plans/next-task-sync-debt-recovery-refactor.md; Task ID: T05; Task title: Add semantic generation-contract checks and fixtures for the handoff, reference-loading, all-tasks-scope, and blocked-routing behaviors; Changed files: config/pkl/renderers/generation-contract-check.pkl, config/pkl/check-generated.sh, config/pkl/renderers/fixtures/handoff-identity-fields-check.pkl, config/pkl/renderers/fixtures/sync-debt-recovery-branch-check.pkl, config/pkl/renderers/fixtures/plan-review-all-tasks-scope-check.pkl, config/pkl/renderers/fixtures/sync-debt-blocked-routing-check.pkl; Implementation summary: In `config/pkl/renderers/generation-contract-check.pkl`, added four new `hidden assertX` functions immediately after the existing `assertPlanReviewSyncDebtRecovery`: `assertHandoffIdentityFields` checks that `sce-change-to-plan/references/plan-template.md` contains the literal `Plan path: {path}; Task ID: {id}; Task title: {title}` handoff-line fragment and that `sce-next-task/references/context-sync.md` contains both `carries its own \`Plan path\` field` and `carries its own \`Task ID\` and \`Task title\` fields` (AC1); `assertSyncDebtRecoveryBranch` isolates the \`sync_debt\` branch paragraph in `sce-next-task/SKILL.md` and asserts the index of `references/context-sync.md` precedes the index of `Task context synchronization phase` within it (AC2); `assertPlanReviewAllTasksScope` checks `plan-review.md` contains `every completed task` and does not contain `earlier completed task` (AC3); `assertSyncDebtBlockedRouting` isolates the `sync_debt` branch's `Branch on the outcome:` section and, reusing the existing `layoutReferencesIn` regex helper, asserts its first layout citation names `Context synchronization blocked` rather than `Review blocked` (AC4). Registered all four in the `contractChecks` mapping. Added one negative fixture per check under `config/pkl/renderers/fixtures/`, each importing `generation-contract-check.pkl` and calling the corresponding `assertX` directly on a minimal `Mapping` engineered to fail that specific behavior, following the existing `next-task-sync-debt-recovery-check.pkl`/`wrong-file-layout-reference-check.pkl` pattern; registered each in `config/pkl/check-generated.sh` via `expect_pkl_fixture_failure` with the exact thrown diagnostic (AC5). Staged the four new fixture files with `git add` (no commit) because `nix flake check`'s `pkl-generated-check` derivation builds from the git-tracked source tree and could not otherwise see the new untracked files; Verification: `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl` passed with all four new checks present and passing against the real generated content; each of the four new fixtures evaluated directly (`nix develop -c pkl eval config/pkl/renderers/fixtures/<fixture>.pkl`) threw with its exact expected diagnostic; `nix run .#pkl-check-generated` passed ("Ephemeral Pkl generation passed: 107 files"); `nix flake check` passed ("all checks passed!"); Done checks: pkl eval succeeds with all four new checks present (done), each new negative fixture throws with a diagnostic naming the missing behavior (done), the real generated output passes all four (done), `nix run .#pkl-check-generated` passes (done); Context impact: none — this task only adds generation-source Pkl semantic checks and fixtures whose downstream durable-context implications (shared-context-code-workflow.md's phase-ownership description, architecture.md's generation-contract-check semantic-check count) are deferred to T06's closing regeneration/reconciliation pass, consistent with T01-T04's precedent
  - Completed: 2026-08-13
  - Files changed: config/pkl/renderers/generation-contract-check.pkl, config/pkl/check-generated.sh, config/pkl/renderers/fixtures/handoff-identity-fields-check.pkl, config/pkl/renderers/fixtures/sync-debt-recovery-branch-check.pkl, config/pkl/renderers/fixtures/plan-review-all-tasks-scope-check.pkl, config/pkl/renderers/fixtures/sync-debt-blocked-routing-check.pkl
  - Evidence: `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl` (exit 0, all four new checks — `handoff-identity-fields`, `sync-debt-recovery-branch`, `plan-review-all-tasks-scope`, `sync-debt-blocked-routing` — present and passing); four fixture evaluations each exiting non-zero with their exact registered diagnostic; `nix run .#pkl-check-generated` (exit 0, "Ephemeral Pkl generation passed: 107 files"); `nix flake check` (exit 0, "all checks passed!")
  - Notes: none

- [x] T06: `Regenerate all targets, verify cross-target parity, and reconcile durable context` (status:done)
  - Task ID: T06
  - Goal: Run a full temporary regeneration and confirm `sce-next-task`
    package content is identical across `.claude`, `.opencode`, and `.pi`
    apart from supported per-target frontmatter, with no target retaining
    stale "earlier completed task" wording, a plan-review-invoked sync call,
    or a recovery block routed to the generic Review blocked layout. Update
    `context/sce/shared-context-code-workflow.md`,
    `context/architecture.md`, and `context/glossary.md` to describe the
    resulting `/next-task` phase-ownership and handoff-identity behavior.
  - Boundaries (in/out of scope): In — the closing regeneration/verification
    pass and the named durable-context files. Out — any other context file;
    any ADR (this plan implements the existing lifecycle decision more
    precisely and does not change it).
  - Dependencies: T01, T02, T03, T04, T05
  - Done when: `nix run .#pkl-check-generated` and `nix flake check` both
    pass; manual inspection of a temporary full generation confirms cross
    target parity and the absence of stale wording named above; the three
    Context sync files accurately describe the new behavior.
  - Verification notes (commands or checks): `nix run .#pkl-generate -- "$(mktemp -d)"`; `nix run .#pkl-check-generated`; `nix flake check`; grep across `.claude`/`.opencode`/`.pi` `sce-next-task` output for stale wording.
  - Context synchronization: synced
  - Context synchronization handoff: Plan path: context/plans/next-task-sync-debt-recovery-refactor.md; Task ID: T06; Task title: Regenerate all targets, verify cross-target parity, and reconcile durable context; Changed files: context/architecture.md, context/glossary.md, context/sce/shared-context-code-workflow.md; Implementation summary: Ran a full temporary regeneration (`nix run .#pkl-generate -- "$(mktemp -d)"`) and confirmed the generated `sce-next-task` package (`SKILL.md` plus `references/{plan-review,task-execution,context-sync,sync-report,output}.md`) is byte-identical across `.claude`, `.opencode`, and `.pi` apart from the supported per-target `compatibility` frontmatter line; grepped the generated output and confirmed no "earlier completed task" wording survives, `plan-review.md`'s "1.2 Resolve one task" states the scan covers "every completed task" and explicitly states it does not run or cite the Task context synchronization phase, and the generated `SKILL.md`'s `sync_debt` branch cites `references/context-sync.md` before running the Task context synchronization phase and routes its `blocked` outcome to the **Context synchronization blocked** layout, distinct from **Review blocked**. Updated `context/sce/shared-context-code-workflow.md`'s `/next-task` phase-ownership section (`sce-plan-review` entry) to describe plan review as a read-only debt detector covering every completed task regardless of position, returning `sync_debt` rather than retrying synchronization itself, and added a new bullet describing the top-level sync-debt-recovery branch (reads `references/context-sync.md`, runs task context synchronization, writes `synced`/`blocked` lifecycle state, and routes a renewed block to **Context synchronization blocked**). Updated `context/architecture.md`'s generation-contract-check description: raised the itemized semantic-check count from eleven to fifteen and named the four new checks (`handoff-identity-fields`, `sync-debt-recovery-branch`, `plan-review-all-tasks-scope`, `sync-debt-blocked-routing`) with what each asserts. Updated `context/glossary.md`'s `baseline-relative task handoff` entry to describe the persisted `Context synchronization handoff` completion-record field's `Plan path`/`Task ID`/`Task title` fields and their self-contained cross-session-retry purpose; Verification: `nix run .#pkl-generate -- "$(mktemp -d)"` (exit 0) followed by diff confirming byte-identical `sce-next-task` bodies across all three targets; `nix run .#pkl-check-generated` passed ("Ephemeral Pkl generation passed: 107 files"); `nix flake check` passed ("all checks passed!"); grep confirmed absence of "earlier completed task" and confirmed the `sync_debt` branch's reference-before-invocation ordering and blocked-layout routing; Done checks: `nix run .#pkl-check-generated` passes (done), `nix flake check` passes (done), manual inspection of the temporary full generation confirms cross-target parity and the absence of stale wording (done), the three Context sync files accurately describe the new behavior (done); Context impact: current-state — this task's own edits are the durable-context reconciliation the plan's Context sync section named (`shared-context-code-workflow.md`, `architecture.md`, `glossary.md`); no ADR applies, since this plan implements the already-recorded `2026-08-12-persist-workflow-sync-lifecycle-in-plans.md` "any completed task" invariant more precisely rather than changing the decision itself
  - Completed: 2026-08-13
  - Files changed: context/architecture.md, context/glossary.md, context/sce/shared-context-code-workflow.md, context/patterns.md
  - Evidence: `nix run .#pkl-generate -- "$(mktemp -d)"` (exit 0) plus diff confirming byte-identical `sce-next-task` package bodies across `.claude`/`.opencode`/`.pi` (frontmatter `compatibility` line excepted); `nix run .#pkl-check-generated` (exit 0, "Ephemeral Pkl generation passed: 107 files, inventory sha256 9b1906b124f0a3c3c380ccad557229b3b45c273f133b0af4c0f10bbbdfe0a10e"); `nix flake check` (exit 0, "all checks passed!"); grep of generated output confirming no "earlier completed task" wording, plan-review.md's "every completed task" scope wording and "Do not run or cite the Task context synchronization phase" statement, and SKILL.md's `sync_debt` branch citing `references/context-sync.md` before "Task context synchronization phase" and routing `blocked` to "Context synchronization blocked" distinct from "Review blocked"
  - Notes: The context-synchronization phase's mandatory five-root-file pass (`overview.md`, `architecture.md`, `glossary.md`, `patterns.md`, `context-map.md`) found `patterns.md`'s `generation-contract-check.pkl` bullet still stated "eleven semantic violations" with a list omitting the four checks T05 added; corrected it to "fifteen" and named all four alongside the existing ones. `overview.md` and `context-map.md` were verified accurate at their existing generality and needed no edit.

## Open questions

None. The request names the exact persisted fields, the exact routing defect,
the exact scope defect, and the exact fixture-based testing approach this
repository already uses for its other semantic generation checks, so no
scope, acceptance-criteria, or ordering decision remains open. The precise
name chosen for plan review's new non-`blocked` status (`sync_debt` above is
illustrative) is an ordinary local implementation choice left to T03/T04,
consistent with existing status-naming conventions (`ready`, `blocked`,
`plan_complete`) in the same file.

## Validation Report

**Status:** validated  
**Date:** 2026-08-13

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (Ephemeral Pkl generation passed: 107 files, inventory sha256 9b1906b124f0a3c3c380ccad557229b3b45c273f133b0af4c0f10bbbdfe0a10e)
- `nix flake check` -> exit 0 (all checks passed!)
- `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl` -> exit 0 (all 28 contract checks, including the four new ones, present and passing)
- `nix develop -c pkl eval config/pkl/renderers/fixtures/handoff-identity-fields-check.pkl` -> exit 1 (throws expected diagnostic: "persisted handoff must carry Plan path, Task ID, and Task title, and context-sync validation must require them from the handoff itself")
- `nix develop -c pkl eval config/pkl/renderers/fixtures/sync-debt-recovery-branch-check.pkl` -> exit 1 (throws expected diagnostic: "sce-next-task SKILL.md sync-debt recovery branch must cite references/context-sync.md before invoking the Task context synchronization phase")
- `nix develop -c pkl eval config/pkl/renderers/fixtures/plan-review-all-tasks-scope-check.pkl` -> exit 1 (throws expected diagnostic: "sce-next-task plan-review reference must state the synchronization-debt scan covers every completed task, with no earlier-completed-task position-relative wording remaining")
- `nix develop -c pkl eval config/pkl/renderers/fixtures/sync-debt-blocked-routing-check.pkl` -> exit 1 (throws expected diagnostic: "sce-next-task SKILL.md sync-debt recovery blocked outcome must route to the Context synchronization blocked layout, not Review blocked")
- `nix run .#pkl-generate -- "$(mktemp -d)"` -> exit 0 (temporary full regeneration for cross-target inspection)

### Success-criteria verification

- [x] AC1: Persisted handoff carries `Plan path`/`Task ID`/`Task title` and context-sync validation requires them from the handoff itself -> `sce-change-to-plan/references/plan-template.md` line 171 shows `Context synchronization handoff: Plan path: {path}; Task ID: {id}; Task title: {title}; ...`; `sce-next-task/references/context-sync.md` "3.1 Validate the handoff" states a persisted retry handoff "carries its own `Plan path` field" and "carries its own `Task ID` and `Task title` fields", met by the handoff text itself
- [x] AC2: Sync-debt recovery is an explicit top-level `/next-task` branch citing `references/context-sync.md` before invoking the phase; plan review is read-only -> generated `SKILL.md` line 88 reads "Read `references/context-sync.md`, then run the **Task context synchronization phase**..."; generated `plan-review.md` states "Do not run or cite the Task context synchronization phase. Stop."
- [x] AC3: Debt scan covers every completed task regardless of position -> generated `plan-review.md` states "inspect every completed task's" and "Only after every completed task is `synced` does task selection proceed"; no "earlier completed task" wording remains
- [x] AC4: Recovery branch's own `blocked` outcome routes to **Context synchronization blocked**, not **Review blocked** -> generated `SKILL.md` line 92: "`blocked` -> Render the **Context synchronization blocked** layout ..., distinct from the **Review blocked** layout above."
- [x] AC5: One semantic check + negative fixture per behavior, registered in `check-generated.sh` -> `generation-contract-check.pkl` contains `handoff-identity-fields`, `sync-debt-recovery-branch`, `plan-review-all-tasks-scope`, `sync-debt-blocked-routing`, all passing against real generated output; each corresponding fixture throws its expected diagnostic when evaluated directly; `nix run .#pkl-check-generated` passes
- [x] AC6: `sce-next-task` package content is byte-identical across `.claude`, `.opencode`, and `.pi` apart from per-target frontmatter -> temporary regeneration diffed: `SKILL.md` differs only in the `compatibility:` frontmatter line; `references/{plan-review,task-execution,context-sync,sync-report,output}.md` are byte-identical across all three targets

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.

