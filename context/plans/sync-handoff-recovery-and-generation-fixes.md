# Plan: sync-handoff-recovery-and-generation-fixes

## Change summary

Fixes four review findings against PR #205 in the canonical Pkl workflow
sources under `config/pkl/base/**` and `config/pkl/renderers/**`, which are the
sole source of truth for the generated `.opencode`, `.claude`, and `.pi`
workflow packages. This extends existing behavior; it does not replace the
synchronization lifecycle or the optional-workflow install-time architecture.

1. Task-level context-sync `blocked` state currently has no durable retry
   record, so a session boundary after a block deadlocks: `/next-task` refuses
   a new task while sync debt exists, but context-sync forbids reconstructing
   the original execution handoff from chat history, and nothing durable
   preserves it. This plan defines one authoritative "Context synchronization
   handoff" record owned by the task completion entry, has `/next-task` write
   it at completion and read it back for retry, and has context-sync retry
   from either the live or the persisted handoff.
2. The generated `/commit` bypass path tells the agent to write a commit
   message to a temporary file for `git commit -F` while a separate rule bans
   modifying files at all — a literal self-contradiction. This plan scopes the
   mutation rule to repository/worktree files and states the out-of-worktree,
   no-interpolation, single-invocation, and cleanup requirements for the one
   permitted temp file.
3. The layout-reference semantic checker in `generation-contract-check.pkl`
   parses `Render the **X** layout from \`references/foo.md\`` instructions but
   always validates the `## X` heading against a hardcoded `references/output.md`
   instead of the captured `foo.md`, so a broken instruction pointing at the
   wrong file can pass. This plan makes the assertion resolve the captured path.
4. The plan template renders `Context synchronization: pending | synced | blocked`
   as if it were a literal field value instead of the documented value domain,
   and the completion-record shape only allows `synced | blocked` even though
   `pending` is a valid state for a completed task awaiting sync. This plan
   makes new tasks start at the single concrete value `pending` and makes the
   completion schema accept `pending | synced | blocked`.

## Acceptance criteria

- [x] AC1: A completed task whose context sync is `blocked` carries a durable
  "Context synchronization handoff" (changed files, implementation summary,
  verification, done checks, context impact) and a "Context synchronization
  blocker" (blocker, required action, retry condition) in the plan file,
  sufficient for a later session to retry using only the plan.
  - Validate: generated `sce-change-to-plan/references/plan-template.md`
    completion-record example shows both subsections with those fields;
    `sce-next-task/references/task-execution.md` writes them at completion.
- [x] AC2: `/next-task` never starts a new implementation task while an
  earlier completed task's `Context synchronization` field is `pending` or
  `blocked`. It first loads the persisted handoff from the plan and
  retries/repairs synchronization for that task; on success it persists
  `synced` and clears blocker fields before continuing normal selection; on a
  renewed block it persists the updated blocker/required-action/retry
  condition and stops. A legacy plan with sync debt but no durable handoff
  structure fails explicitly with migration guidance instead of attempting a
  reconstructed retry.
  - Validate: `nix develop -c pkl eval config/pkl/renderers/fixtures/next-task-sync-debt-recovery-check.pkl`
    (new negative fixture) throws; `nix run .#pkl-check-generated` passes.
- [x] AC3: Task-context-sync accepts either the live execution handoff
  (same-session) or the persisted plan-recorded handoff (cross-session retry)
  as authoritative input, never reconstructs a missing one from conversation
  history, and its blocked report always writes the handoff and blocker
  subsections from AC1.
  - Validate: generated `sce-next-task/references/context-sync.md` documents
    both input sources and the blocked-report shape; `nix develop -c pkl eval
    config/pkl/base/workflow-context-sync.pkl`.
- [x] AC4: Bypass-mode `/commit` instructions permit exactly the
  out-of-worktree commit-message temp file (no shell interpolation, exactly
  one `git commit -F <temp-file>`, cleanup including failure paths, explicit
  post-success hash retrieval) and no longer state an unscoped "do not modify
  files" rule beside that instruction.
  - Validate: generated `sce-commit/SKILL.md` and
    `sce-commit/references/atomic-commit.md` contain the reconciled wording in
    every rendered occurrence (package and composite).
- [x] AC5: The layout-reference checker validates the `## X` heading against
  the exact file an instruction cites, not a hardcoded `references/output.md`,
  while still keeping the general package-local-reference-existence
  assertion.
  - Validate: `nix develop -c pkl eval
    config/pkl/renderers/fixtures/wrong-file-layout-reference-check.pkl`
    (new negative fixture) throws; `nix develop -c pkl eval
    config/pkl/renderers/fixtures/correct-file-layout-reference-check.pkl`
    (new positive fixture) passes; `nix run .#pkl-check-generated` passes.
- [x] AC6: A freshly authored task in a newly written plan starts with
  `Context synchronization: pending` as a concrete value; the
  `pending | synced | blocked` domain is documented separately from that
  value. Completed tasks may legitimately be `pending`, `synced`, or
  `blocked`, and blocker metadata is present only for `blocked`.
  - Validate: generated `sce-change-to-plan/references/plan-template.md`
    (package and composite) task-authoring example and completion-record
    example.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/sce/shared-context-code-workflow.md`
- `context/sce/atomic-commit-workflow.md`
- `context/decisions/2026-08-12-persist-workflow-sync-lifecycle-in-plans.md`
- `context/architecture.md` (generation-contract-check semantic-check count and
  the plan-template lifecycle description)

## Constraints and non-goals

- **In scope:** `config/pkl/base/workflow-change-to-plan.pkl`,
  `config/pkl/base/workflow-next-task.pkl`,
  `config/pkl/base/workflow-context-sync.pkl`,
  `config/pkl/base/workflow-commit.pkl`,
  `config/pkl/renderers/generation-contract-check.pkl`,
  `config/pkl/renderers/fixtures/*.pkl`, and the fixture registration list in
  `config/pkl/check-generated.sh`. Ephemeral regeneration of `.opencode`,
  `.claude`, and `.pi` outputs through the normal generation pipeline.
- **Out of scope:** `sce-handover`, `sce-brownfield`, and `sce-decision`
  packages; any CLI Rust code under `cli/`; optional-workflow install-time
  semantics and the 2026-07-31 install-time-optional-workflows ADR; any
  workflow behavior not named in the four findings.
- **Constraints:** `config/pkl/**` remains the sole source of truth; never
  hand-edit generated `.pi/**`, `.claude/**`, or `.opencode/**` output; keep
  the plan/completion-record Markdown format consistent with the existing
  template conventions in `references/plan-template.md`; keep the
  package-vs-composite rendering parity the existing renderer contract
  requires.
- **Non-goal:** Redesigning the synchronization lifecycle beyond persisting
  and retrying the existing `pending`/`synced`/`blocked` model; introducing a
  second persistence mechanism (database, sidecar file) for lifecycle state,
  which the accepted `persist-workflow-sync-lifecycle-in-plans` decision
  already rules out.

## Task stack

- [x] T01: `Redefine the plan-template lifecycle values and durable handoff shape` (status:done)
  - Task ID: T01
  - Goal: In `config/pkl/base/workflow-change-to-plan.pkl`, make a freshly
    authored task's `Context synchronization` field a concrete `pending`
    value (never the `pending | synced | blocked` union used as a value), keep
    that union only as separate value-domain documentation, widen the
    completion-record schema from `synced | blocked` to
    `pending | synced | blocked`, and add the "Context synchronization
    handoff" (changed files, implementation summary, verification, done
    checks, context impact) and "Context synchronization blocker" (blocker,
    required action, retry condition) subsections to the completion record,
    matching the PR's conceptual example. Apply the fix to every rendered
    occurrence of the plan template (package mode and the composite-skill
    duplicate).
  - Boundaries (in/out of scope): In — `references/plan-template.md`
    rendering (task-authoring example, completion-record example, lifecycle
    value documentation) in `workflow-change-to-plan.pkl` only. Out — the
    phases that read/write these fields at runtime (T02-T04); any other
    workflow package.
  - Dependencies: none
  - Done when: `nix develop -c pkl eval config/pkl/base/workflow-change-to-plan.pkl`
    succeeds; a temporary generation shows `references/plan-template.md`
    (package) and the composite plan-template section both render
    `Context synchronization: pending` for a new task,
    `pending | synced | blocked` for the completion-record field, and the two
    new subsections with the named fields; no template literal still uses the
    union as a concrete field value.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/base/workflow-change-to-plan.pkl`; `nix run .#pkl-generate -- "$(mktemp -d)"` then grep the generated plan-template content for `Context synchronization: pending` and the new subsection headings.
  - Context synchronization: pending
  - Context synchronization handoff: Changed files: `config/pkl/base/workflow-change-to-plan.pkl`; Implementation summary: In both the composite reference-document template (`changeToPlanPlanTemplate`, ~L601-719) and the package-mode `renderPlanTemplate` function (~L2124-2242), changed the task-authoring and filled-in-task examples' `Context synchronization` line from the `pending | synced | blocked` union to the concrete value `pending` (and removed the now-inapplicable `When blocked:` line from those authoring examples, since a freshly authored task starts `pending`); widened the completion-record example's `Context synchronization` line from `synced | blocked` to `pending | synced | blocked`; added `Context synchronization handoff` (changed files, implementation summary, verification, done checks, context impact) and `Context synchronization blocker` (blocker, required action, retry condition; present only when blocked) lines to the completion-record example, in both occurrences.; Verification: `nix develop -c pkl eval config/pkl/base/workflow-change-to-plan.pkl` (pass); `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl` (pass, includes package-vs-composite target-neutral-references parity check); `nix run .#pkl-check-generated` (pass, 107 files); targeted generation + grep of `references/plan-template.md` under `.claude/.opencode/.pi` skills/sce-change-to-plan confirmed `Context synchronization: pending` for authoring examples, `pending | synced | blocked` plus both new handoff/blocker lines for the completion-record example.; Done checks: All satisfied — pkl eval succeeds, generated plan-template renders the concrete `pending` value for new tasks, the widened union for completion records, and both new subsection lines; no template literal retains the union as a task-authoring value.; Context impact: None — this is workflow-generation source only; no `context/` file describes plan-template field shape independently of the generated reference itself.
  - Completed: 2026-08-13
  - Files changed: `config/pkl/base/workflow-change-to-plan.pkl`
  - Evidence: `nix develop -c pkl eval config/pkl/base/workflow-change-to-plan.pkl` — pass; `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl` — pass (24 semantic checks, including target-neutral-references parity); `nix run .#pkl-check-generated` — pass (107 files, ephemeral generation).
  - Notes: Removed the `When blocked:` line from the task-authoring/filled-in-task examples (not just widened the status value) since a freshly authored task is always `pending` and can never legitimately show blocker fields at authoring time; this is a reversible, in-scope local formatting choice consistent with the task's intent.

- [x] T02: `Persist the synchronization handoff at task completion` (status:done)
  - Task ID: T02
  - Goal: In `config/pkl/base/workflow-next-task.pkl`'s task-execution phase,
    write the T01 "Context synchronization handoff" subsection (changed
    files, implementation summary, verification, done checks, context impact)
    into the plan's completion record for the just-completed task before
    invoking task-context-sync, using the same field set context-sync
    consumes rather than duplicating the entire execution result.
  - Boundaries (in/out of scope): In — task-execution phase steps that mark a
    task complete and record its `Context synchronization: pending` state, in
    both package and composite renderings. Out — sync-debt recovery in plan
    review (T03); context-sync's own read/write behavior (T04).
  - Dependencies: T01
  - Done when: `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl`
    succeeds; the generated task-execution reference instructs writing the
    handoff subsection with the field names T01 defined, immediately before
    the `Context synchronization: pending` write.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl`; targeted generation and grep for the new instruction text in `sce-next-task/references/task-execution.md`.
  - Context synchronization: pending
  - Context synchronization handoff: Changed files: `config/pkl/base/workflow-next-task.pkl`; Implementation summary: In step 7 "Update the plan" of the task-execution phase (both the package-mode `renderTaskExecution` rendering and the composite `nextTaskTaskExecution` template), added one instruction bullet — "Write the task's `Context synchronization handoff` subsection into the completion record: changed files, implementation summary, verification, done checks, and context impact, using the same field set task-context-sync consumes rather than duplicating the entire execution result." — placed immediately after "Mark only the selected task complete." and before the existing "Set that task's `Context synchronization` field to `pending`" bullet, which was reworded to state it happens after the new handoff-subsection write.; Verification: `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl` (pass); targeted generation via `nix run .#pkl-generate` into a temp dir, grep of `sce-next-task/references/task-execution.md` under `.claude` and `.pi` confirmed the new bullet renders identically in both, immediately before the `pending` write line (`.opencode` renders this reference under a different path not covered by this task's done-when).; Done checks: All satisfied — pkl eval succeeds; generated task-execution reference instructs writing the handoff subsection with T01's field names immediately before the `Context synchronization: pending` write, in both package and composite renderings.; Context impact: None — this is workflow-generation source only; no `context/` file describes task-execution phase step ordering independently of the generated reference itself.

- [x] T03: `Add /next-task sync-debt recovery before new-task selection` (status:done)
  - Task ID: T03
  - Goal: In the plan-review phase of `config/pkl/base/workflow-next-task.pkl`,
    before selecting or starting a task, inspect completed tasks' `Context
    synchronization` field; when debt (`pending` or `blocked`) exists, load
    the persisted handoff (and blocker/required-action/retry-condition, if
    blocked) from the plan and retry/repair synchronization for that task
    instead of refusing outright. On success, persist `synced`, clear
    obsolete blocker/retry metadata, and continue normal task selection. On a
    renewed block, persist the updated blocker/required-action/retry
    condition and stop. Never start a later implementation task while earlier
    debt remains, and never reconstruct a missing handoff from chat history:
    a legacy plan with debt but no durable handoff structure fails explicitly
    with a migration/recovery message. Add one semantic generation-contract
    check (alongside the existing ten in `generation-contract-check.pkl`)
    asserting the generated plan-review reference states the recovery and
    legacy-migration-failure behavior, with a negative fixture proving its
    absence fails, registered in `config/pkl/check-generated.sh`.
  - Boundaries (in/out of scope): In — plan-review phase recovery steps in
    `workflow-next-task.pkl` (package and composite); the new semantic
    assertion and its fixture. Out — task-context-sync's own retry mechanics
    (T04, though T03 depends on T04's contract existing conceptually — order
    is fine since this task only asserts plan-review's own instructions, not
    context-sync's).
  - Dependencies: T01, T02
  - Done when: `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl`
    and `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl`
    succeed; the new negative fixture throws when the recovery/migration
    wording is missing and the real generated output passes the new
    assertion.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/renderers/fixtures/next-task-sync-debt-recovery-check.pkl` (expect throw); `nix run .#pkl-check-generated`.
  - Context synchronization: pending
  - Context synchronization handoff: Changed files: `config/pkl/base/workflow-next-task.pkl`, `config/pkl/renderers/generation-contract-check.pkl`, `config/pkl/check-generated.sh`, `config/pkl/renderers/fixtures/next-task-sync-debt-recovery-check.pkl`; Implementation summary: In both renderings of the plan-review phase (the static composite `nextTaskPlanReviewReference` string that produces `references/plan-review.md`, and the package-mode `renderReviewSkillBody` function that produces the standalone `sce-plan-review` skill and the composite command's inlined step 1), rewrote step "1.2 Resolve one task" so that, on finding unresolved synchronization debt in an earlier completed task, the phase no longer refuses outright: it first checks for a durable `Context synchronization handoff` subsection (absent means a legacy plan, which fails explicitly with a migration required action instead of a reconstructed retry), otherwise loads the persisted handoff/blocker and runs the Task context synchronization phase with it as authoritative input, then on `synced`/`no_context_change` persists `synced` and clears blocker fields before continuing to check earlier debt or proceed to normal selection, or on a renewed `blocked` persists the refreshed blocker/required-action/retry-condition and stops. Updated the phase's opening framing ("It reads; it never writes" → reads, and writes only to persist a recovery outcome) and the "Plan review boundaries" list (scoped the "Update the plan" and "Synchronize context" prohibitions to exclude the new recovery path) in both renderings, plus added a "Recovering unresolved synchronization debt from earlier completed tasks" bullet to the package-mode skill's purpose list. Added `assertPlanReviewSyncDebtRecovery` to `generation-contract-check.pkl` (registered as `plan-review-sync-debt-recovery`), asserting every generated `sce-next-task/references/plan-review.md` contains both the recovery wording ("do not attempt a reconstructed retry") and the legacy-migration wording ("migrate the plan"). Added the negative fixture `fixtures/next-task-sync-debt-recovery-check.pkl` (a plan-review.md stub missing both phrases, asserting the check throws) and registered its expected diagnostic in `check-generated.sh`.; Verification: `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl` (pass); `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl` (pass, 25 semantic checks including the new one); `nix develop -c pkl eval config/pkl/renderers/fixtures/next-task-sync-debt-recovery-check.pkl` (throws as expected, diagnostic matches); `nix develop -c ./config/pkl/check-generated.sh` (pass, 107 files, all fixtures including the new one); `nix flake check` (all checks passed, after staging the new fixture file with `git add` so the flake's git-tracked source includes it); manual ephemeral regeneration confirmed `references/plan-review.md` renders byte-identical recovery/migration wording under `.claude`, `.opencode`, and `.pi`.; Done checks: All satisfied — both pkl eval commands succeed, the new negative fixture throws with the expected diagnostic when recovery/migration wording is missing, and the real generated `plan-review.md` passes the new assertion under all three targets.; Context impact: None — this is workflow-generation source only; the referenced decision (`2026-08-12-persist-workflow-sync-lifecycle-in-plans`) already describes the intended lifecycle at the level `context/` documents; no `context/` file describes plan-review phase step-level behavior independently of the generated reference itself.
  - Completed: 2026-08-13
  - Files changed: `config/pkl/base/workflow-next-task.pkl`, `config/pkl/renderers/generation-contract-check.pkl`, `config/pkl/check-generated.sh`, `config/pkl/renderers/fixtures/next-task-sync-debt-recovery-check.pkl`
  - Evidence: `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl` — pass; `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl` — pass (25 semantic checks); `nix develop -c pkl eval config/pkl/renderers/fixtures/next-task-sync-debt-recovery-check.pkl` — throws with expected diagnostic; `nix develop -c ./config/pkl/check-generated.sh` — pass (107 files, ephemeral generation); `nix flake check` — all checks passed.
  - Notes: Kept the recovery loop scoped to "the first task carrying debt" per invocation rather than resolving all debt in one pass, since the phase's existing "review at most one task per invocation" discipline and the retry/stop branching in the goal statement both describe single-step progress; a plan with multiple debts is recovered incrementally across successive `/next-task` invocations. The new semantic check and fixture follow the existing `assertAtomicCommitContent`/`atomic-commit-content-check.pkl` pattern (per-path document substring assertion with a minimal negative-fixture stub) rather than the `assertNoStaleSyncDebtText` pattern, since the check is scoped to one specific reference document rather than all generated artifacts.

- [x] T04: `Retry task-context-sync from the persisted handoff` (status:done)
  - Task ID: T04
  - Goal: In the task-role synchronization defined in
    `config/pkl/base/workflow-context-sync.pkl`, accept either the live
    execution handoff (same-session, from T02) or the persisted plan-recorded
    handoff (cross-session retry invoked by T03) as authoritative input,
    while still refusing to reconstruct a missing handoff from conversation
    history. Update the blocked-report writing to populate the handoff and
    blocker subsections from T01 rather than only the current flat fields.
  - Boundaries (in/out of scope): In — task-role sections of
    `workflow-context-sync.pkl` (handoff validation step, blocked-report
    rendering), package and composite. Out — plan-role (`/validate`)
    synchronization, which this finding does not touch.
  - Dependencies: T01, T02
  - Done when: `nix develop -c pkl eval config/pkl/base/workflow-context-sync.pkl`
    succeeds; generated `sce-next-task/references/context-sync.md` documents
    accepting the persisted handoff on retry and writes the new subsections
    when blocked.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/base/workflow-context-sync.pkl`; targeted generation and grep for the retry-input and blocked-report wording.
  - Context synchronization: pending
  - Context synchronization handoff: Changed files: `config/pkl/base/workflow-context-sync.pkl`; Implementation summary: In the task role's `input` function (package) and the composite `taskReference` string's introduction/step-3.1, widened the accepted handoff to either the live `status: complete` execution result (same-session) or the persisted `Context synchronization handoff` (and, on retry of a `blocked` task, `Context synchronization blocker`) subsection loaded from the plan by the plan-review recovery step (cross-session), and generalized the "do not reconstruct" rule to cover both sources; renamed step 1/3.1 from "Validate the execution handoff" to "Validate the handoff" in both renderings and adjusted its bullet list accordingly. Added a note to step 8/3.8 "Return the Markdown report" (both renderings) stating a `blocked` report always writes `Context synchronization handoff` and `Context synchronization blocker` subsections using the plan's completion-record field names. Extended `SyncReportRole` with `blockedHandoffSection`, `blockedBlockerSection`, and `blockedRetrySection` function fields (replacing the previously hardcoded `## Blocker`/`## Retry condition` blocks in the shared `blockedReport` template with role-supplied content, joined via blank-line-safe conditional interpolation); gave `taskReport` new `## Context synchronization handoff` (Changed files, Implementation summary, Verification, Done checks, Context impact) and `## Context synchronization blocker` (Blocker, Required action, Retry condition) subsections matching T01's completion-record field names, removing the redundant standalone "Updated files" list from the blocked identity; gave `planReport` the same `blockedBlockerSection`/`blockedRetrySection` fields reproducing its prior `## Blocker`/`## Retry condition` content unchanged, and `blockedHandoffSection` as empty, so plan-role rendering is unaffected.; Verification: `nix develop -c pkl eval config/pkl/base/workflow-context-sync.pkl` (pass); `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl` (pass, 25 semantic checks, none newly failing); `nix develop -c ./config/pkl/check-generated.sh` (pass, 107 files, ephemeral generation); `nix flake check` (all checks passed); targeted generation + grep of `sce-next-task/references/context-sync.md` confirmed the persisted-handoff and blocked-subsection wording renders; direct inspection of `taskOutputReport`/`planOutputReport` via `pkl eval -x` confirmed the task blocked variant renders both new subsections cleanly (no blank-line runs) and the plan blocked variant is byte-identical to its prior `## Blocker`/`## Retry condition` rendering.; Done checks: All satisfied — pkl eval succeeds; generated `context-sync.md` documents accepting the persisted handoff on retry (`persisted`, `Context synchronization handoff`, `Context synchronization blocker` wording present) and states the blocked report writes the new subsections.; Context impact: None — this is workflow-generation source only; the referenced decision (`2026-08-12-persist-workflow-sync-lifecycle-in-plans`) already describes the intended lifecycle at the level `context/` documents; no `context/` file describes context-sync phase step-level behavior independently of the generated reference itself.
  - Completed: 2026-08-13
  - Files changed: `config/pkl/base/workflow-context-sync.pkl`
  - Evidence: `nix develop -c pkl eval config/pkl/base/workflow-context-sync.pkl` — pass; `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl` — pass (25 semantic checks); `nix develop -c ./config/pkl/check-generated.sh` — pass (107 files, ephemeral generation); `nix flake check` — all checks passed.
  - Notes: Kept plan-role (`/validate`) blocked-report rendering byte-identical to its pre-task form by giving it the same new role fields with content reproducing the old hardcoded text, rather than special-casing the shared `blockedReport` template per role; this keeps the shared template single-sourced while satisfying the task's "out of scope" boundary for plan-role synchronization. Removed the `retryCondition` field from `SyncReportRole` entirely (folded into `blockedRetrySection`) since nothing referenced it once the blocked-report skeleton switched to role-supplied section functions — a reversible, in-scope local cleanup rather than a second field carrying duplicate content.

- [x] T05: `Reconcile the bypass-commit temp-file rule` (status:done)
  - Task ID: T05
  - Goal: In `config/pkl/base/workflow-commit.pkl`, scope the "do not stage,
    unstage, restore, or otherwise modify files" rule to repository/worktree
    files, and explicitly permit exactly the commit-message temp file the
    bypass path already instructs writing. State that the file must be
    created outside the repository working tree, written verbatim without
    shell interpolation, used for exactly one `git commit -F <temp-file>`,
    and cleaned up after the commit attempt including failure paths where
    practical, with the resulting hash retrieved explicitly only after
    success via `git rev-parse --verify HEAD^{commit}`. Apply consistently to
    every rendered occurrence (package command, package skill, composite
    skill).
  - Boundaries (in/out of scope): In — the bypass execution handoff and
    "Atomic commit boundaries"/rule blocks in `workflow-commit.pkl`. Out —
    regular-mode proposal behavior; anything under next-task/context-sync.
  - Dependencies: none
  - Done when: `nix develop -c pkl eval config/pkl/base/workflow-commit.pkl`
    succeeds; generated `sce-commit/SKILL.md` and
    `sce-commit/references/atomic-commit.md` no longer contain an unscoped
    "do not modify files" statement beside the temp-file instruction, and
    every occurrence states the out-of-worktree, no-interpolation, exactly-once,
    cleanup, and explicit-hash requirements.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/base/workflow-commit.pkl`; targeted generation and grep across `sce-commit/SKILL.md` plus `references/atomic-commit.md` for the reconciled wording.
  - Context synchronization: synced
  - Context synchronization handoff: Changed files: `config/pkl/base/workflow-commit.pkl`; Implementation summary: In `renderCommandBody` (package command / composite command shared body), the "Execute exactly one commit" bypass step now instructs creating the temp file outside the repository working tree and adds an explicit "delete the temp file after the commit attempt, including on failure, where practical" step, and the command's "Rules" list scopes the mutation prohibition to "repository or worktree files" with the temp file named as the sole exception because it lives outside the working tree. Made the matching pair of edits in `renderAtomicCommitSkillBody`'s "Bypass execution handoff" section (canonical package-skill/`.opencode`/`.pi` source) and its "Atomic commit boundaries" `Do not:` list. Made the same pair of edits in the static composite `commitSkillBody` string's step 3 "Execute exactly one commit" and its "Rules" list (renders into `.claude`'s `sce-commit/SKILL.md`). All three rendered occurrences now state: out-of-worktree temp-file creation, no shell interpolation (pre-existing wording, unchanged), exactly one `git commit -F <temp-file>` (pre-existing, unchanged), post-success-only explicit hash retrieval via `git rev-parse --verify HEAD^{commit}` (pre-existing, unchanged), and post-attempt cleanup including failure paths where practical (new); the "do not modify files" rule now reads "repository or worktree files" with the out-of-worktree temp file carved out explicitly, so the bypass instruction to write it no longer contradicts the rule beside it.; Verification: `nix develop -c pkl eval config/pkl/base/workflow-commit.pkl` (pass); targeted ephemeral generation via `nix run .#pkl-generate` into a temp dir, grep of `sce-commit/SKILL.md` under `.claude`/`.opencode`/`.pi` and `sce-commit/references/atomic-commit.md` under `.opencode`/`.pi` confirmed the out-of-worktree/cleanup wording and the scoped mutation rule render identically in all three targets; `nix develop -c ./config/pkl/check-generated.sh` (pass, 107 files, ephemeral generation).; Done checks: All satisfied — pkl eval succeeds; every generated occurrence states the out-of-worktree, no-interpolation, exactly-once, cleanup, and explicit-hash requirements, and no occurrence retains an unscoped "do not modify files" statement beside the temp-file instruction.; Context impact: None — this is workflow-generation source only; no `context/` file describes the bypass-commit temp-file rule independently of the generated reference itself.
  - Completed: 2026-08-13
  - Files changed: `config/pkl/base/workflow-commit.pkl`
  - Evidence: `nix develop -c pkl eval config/pkl/base/workflow-commit.pkl` — pass; `nix develop -c ./config/pkl/check-generated.sh` — pass (107 files, ephemeral generation); targeted generation + grep of `sce-commit/SKILL.md` (`.claude`, `.opencode`, `.pi`) and `sce-commit/references/atomic-commit.md` (`.opencode`, `.pi`) confirmed the reconciled wording renders in every occurrence.
  - Notes: The package-mode `commit.md` command document (`structuredCommand.render.apply("package")`) is not among the three currently-generated targets — all three (`.claude`, `.opencode`, `.pi`) render the composite command stub plus the full skill body — but the shared `renderCommandBody` function was still updated since it is the single source for that content wherever it is later used, satisfying "every rendered occurrence" for what the generation pipeline currently produces.

- [x] T06: `Resolve the layout-reference checker against the captured file and verify all targets` (status:done)
  - Task ID: T06
  - Goal: In `config/pkl/renderers/generation-contract-check.pkl`, change
    `assertLayoutReferences` so each `Render the **X** layout from
    \`references/foo.md\`` instruction resolves the exact captured
    `references/foo.md` (package-root-relative) instead of a hardcoded
    `references/output.md`, asserting both that the document exists and that
    it contains a matching `## X` heading, while keeping the existing general
    package-local-reference-existence assertion
    (`assertPackageLocalReferences`) unchanged. Add a negative fixture (an
    instruction citing `references/wrong-file.md`, which lacks `## Completion`,
    while `references/output.md` has it) proving the filename matters, and a
    positive fixture (heading present in the actually-referenced document),
    registered in `config/pkl/check-generated.sh`. Finish by regenerating all
    three targets and running the full verification suite, confirming no
    generated file retains contradictory lifecycle or temporary-file wording
    from T01-T05 across `.opencode`, `.claude`, and `.pi`.
  - Boundaries (in/out of scope): In — `assertLayoutReferences`, its two new
    fixtures, `check-generated.sh` registration, and the closing
    regeneration/verification pass. Out — any other semantic check or
    fixture.
  - Dependencies: T01, T02, T03, T04, T05
  - Done when: `nix develop -c pkl eval
    config/pkl/renderers/fixtures/wrong-file-layout-reference-check.pkl`
    throws with a diagnostic naming the mismatched file; `nix develop -c pkl
    eval config/pkl/renderers/fixtures/correct-file-layout-reference-check.pkl`
    passes; `nix run .#pkl-check-generated` and `nix flake check` both pass;
    manual inspection of a temporary full generation confirms items 1-9 in
    the originating request's Verification section.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/renderers/fixtures/wrong-file-layout-reference-check.pkl` (expect throw); `nix develop -c pkl eval config/pkl/renderers/fixtures/correct-file-layout-reference-check.pkl`; `nix run .#pkl-check-generated`; `nix flake check`.
  - Context synchronization: pending
  - Context synchronization handoff: Changed files: `config/pkl/renderers/generation-contract-check.pkl`, `config/pkl/renderers/fixtures/layout-reference-check.pkl`, `config/pkl/renderers/fixtures/wrong-file-layout-reference-check.pkl` (new), `config/pkl/renderers/fixtures/correct-file-layout-reference-check.pkl` (new), `config/pkl/check-generated.sh`; Implementation summary: Rewrote `assertLayoutReferences` in `generation-contract-check.pkl` to stop hardcoding `references/output.md`: it now iterates every `Render the **X** layout from \`references/foo.md\`` citation in each SKILL.md (excluding `sce-decision`), resolves the exact captured `references/foo.md` path relative to that skill's package root, and collects a violation (the resolved path) whenever that document is missing or lacks a matching `## X` heading, using `documents.toMap().entries.filter/flatMap` instead of the prior nested `every`/hardcoded-path form; the throw now names the first violating file (`` `\(violations[0])` ``) instead of a static "its references/output.md" message, and the success message reads "every citation resolves to a heading in its cited references document". `assertPackageLocalReferences` was left unchanged. Updated the pre-existing `layout-reference-check.pkl` fixture's unreachable local fallback message to match (cosmetic only — the fixture's real diagnostic comes from the production throw). Added `wrong-file-layout-reference-check.pkl`: an sce-next-task SKILL.md instruction citing `references/wrong-file.md` (injected, heading-less) while `references/output.md` in the same package is also given a `## Completion` heading, proving the checker resolves the cited file rather than falling back to `output.md`; asserts the throw names the `wrong-file.md` path. Added `correct-file-layout-reference-check.pkl`: an instruction citing `references/correct-file.md` (injected, containing `## Completion`), asserting `assertLayoutReferences.apply(documents)` returns its success string without throwing. Registered both fixtures in `check-generated.sh`: the wrong-file fixture via `expect_pkl_fixture_failure` with the file-naming diagnostic, the correct-file fixture via a plain `pkl eval ... >/dev/null` (following the existing metadata-coverage/generation-contract eval-only pattern, since it is the suite's first positive fixture); also updated the pre-existing `layout-reference-check.pkl` registration's expected diagnostic to the new file-naming message. Staged both new fixture files with `git add` so `nix flake check`'s git-tracked source filter includes them (same requirement T03 encountered).; Verification: `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl` (pass, 25 semantic checks, `layout-references` message updated); `nix develop -c pkl eval config/pkl/renderers/fixtures/wrong-file-layout-reference-check.pkl` (throws, diagnostic names `config/.opencode/skills/sce-next-task/references/wrong-file.md`); `nix develop -c pkl eval config/pkl/renderers/fixtures/correct-file-layout-reference-check.pkl` (passes, no throw); `nix run .#pkl-check-generated` (pass, 107 files, ephemeral generation); `nix flake check` (all 5 checks passed, after staging the new fixtures); manual ephemeral regeneration into a temp dir + grep across `.claude`/`.opencode`/`.pi` confirmed: no unscoped "do not modify files" wording beside the commit temp-file instruction (the only remaining "do not modify files" hits are the unrelated task-execution approval-decline rule), the commit rule's "repository or worktree files" scoping renders in all three targets, the plan-template's concrete `Context synchronization: pending` and widened `pending | synced | blocked` completion-record union render in all three targets, the plan-review sync-debt-recovery wording ("do not attempt a reconstructed retry") renders in all three targets, and no target contains the stale "Nothing records the skipped synchronization" text.; Done checks: All satisfied — both new fixtures behave as specified (negative throws naming the mismatched file, positive passes), `nix run .#pkl-check-generated` and `nix flake check` both pass, and manual inspection of a temporary full generation found no contradictory lifecycle or temporary-file wording surviving from T01-T05 across any target.; Context impact: None — this is workflow-generation source only; no `context/` file describes the layout-reference checker's resolution behavior independently of the generated reference/check itself.
  - Completed: 2026-08-13
  - Files changed: `config/pkl/renderers/generation-contract-check.pkl`, `config/pkl/renderers/fixtures/layout-reference-check.pkl`, `config/pkl/renderers/fixtures/wrong-file-layout-reference-check.pkl`, `config/pkl/renderers/fixtures/correct-file-layout-reference-check.pkl`, `config/pkl/check-generated.sh`
  - Evidence: `nix develop -c pkl eval config/pkl/renderers/generation-contract-check.pkl` — pass (25 semantic checks); `nix develop -c pkl eval config/pkl/renderers/fixtures/wrong-file-layout-reference-check.pkl` — throws, names `config/.opencode/skills/sce-next-task/references/wrong-file.md`; `nix develop -c pkl eval config/pkl/renderers/fixtures/correct-file-layout-reference-check.pkl` — passes; `nix run .#pkl-check-generated` — pass (107 files, ephemeral generation); `nix flake check` — all checks passed.

## Open questions

None. The request specifies exact field names, per-state semantics, the
checker's exact defect and fix, and the fixture-based testing approach this
repository already uses for semantic generation checks, so no scope,
acceptance-criteria, or ordering decision remains open.

## Validation Report

**Status:** validated  
**Date:** 2026-08-13

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (107 files, ephemeral generation, pass)
- `nix flake check` -> exit 0 (all checks passed)
- `nix develop -c pkl eval config/pkl/renderers/fixtures/next-task-sync-debt-recovery-check.pkl` -> exit 1 (throws as required: "plan-review reference must state sync-debt recovery and legacy-migration-failure behavior")
- `nix develop -c pkl eval config/pkl/base/workflow-context-sync.pkl` -> exit 0 (pass)
- `nix develop -c pkl eval config/pkl/renderers/fixtures/wrong-file-layout-reference-check.pkl` -> exit 1 (throws as required, names `config/.opencode/skills/sce-next-task/references/wrong-file.md`)
- `nix develop -c pkl eval config/pkl/renderers/fixtures/correct-file-layout-reference-check.pkl` -> exit 0 (passes, returns success string)
- `nix develop -c pkl eval config/pkl/base/workflow-change-to-plan.pkl` -> exit 0 (pass)
- `nix run .#pkl-generate -- <tmpdir>` -> exit 0 (targeted regeneration for AC1/AC3/AC4/AC6 inspection)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: durable "Context synchronization handoff" and "Context synchronization blocker" subsections in the completion-record example -> generated `sce-change-to-plan/references/plan-template.md` completion-record example (line ~170-172) shows both subsections with the named fields; `task-execution.md` (T02) writes them at completion, confirmed via `nix develop -c pkl eval config/pkl/base/workflow-next-task.pkl` pass and prior task evidence.
- [x] AC2: `/next-task` sync-debt recovery before new-task selection -> `nix develop -c pkl eval config/pkl/renderers/fixtures/next-task-sync-debt-recovery-check.pkl` throws as required; `nix run .#pkl-check-generated` passes.
- [x] AC3: task-context-sync accepts live or persisted handoff, never reconstructs -> generated `sce-next-task/references/context-sync.md` documents both input sources ("live" / "persisted") and the blocked-report shape (lines 9, 17, 25, 33, 37, 52, 308, 310); `nix develop -c pkl eval config/pkl/base/workflow-context-sync.pkl` passes.
- [x] AC4: bypass-mode `/commit` reconciled wording -> generated `sce-commit/SKILL.md` (`.claude`, `.opencode`, `.pi`) and `sce-commit/references/atomic-commit.md` (`.opencode`, `.pi`) all state "Create the commit-message temp file outside the repository working tree" and the scoped "do not... modify repository or worktree files" rule, in every rendered occurrence.
- [x] AC5: layout-reference checker resolves the cited file, not a hardcoded `references/output.md` -> `nix develop -c pkl eval config/pkl/renderers/fixtures/wrong-file-layout-reference-check.pkl` throws naming the mismatched file; `nix develop -c pkl eval config/pkl/renderers/fixtures/correct-file-layout-reference-check.pkl` passes; `nix run .#pkl-check-generated` passes.
- [x] AC6: freshly authored task starts `Context synchronization: pending` as a concrete value, domain documented separately -> generated `sce-change-to-plan/references/plan-template.md` (package and composite) task-authoring examples (lines 81, 90, 116) render the concrete `pending` value; the `pending | synced | blocked` domain is documented separately (lines 48, 51); completion-record example accepts the full union (line 170).

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
