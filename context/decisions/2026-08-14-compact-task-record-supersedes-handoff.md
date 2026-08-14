# Decision: Compact Task Record Supersedes Persisted Synchronization Handoff

Date: 2026-08-14
Status: Accepted
Plan: `context/plans/simplify-task-record-format.md`
Task: `T01, T02, T03, T04, T05`
Supersedes: `context/decisions/2026-08-12-persist-workflow-sync-lifecycle-in-plans.md`

## Context

The prior decision established that task-level and plan-level context
synchronization lifecycle state (`pending`/`synced`/`blocked`) must persist in
the plan Markdown, durable across sessions. In practice, the completed-task
record also grew a second, duplicated representation of the same facts: a
serialized `Context synchronization handoff` completion-record field
(explicit `Plan path`, `Task ID`, `Task title`, changed files, implementation
summary, verification, done checks, and context impact) alongside separate
`Implementation evidence` and `Verification evidence` sections, all restating
information the completed task already carried through `Goal`,
`Boundaries (in/out of scope)`, `Verification notes`, `Files changed`, and
`Evidence`. Immediate synchronization and later sync-debt recovery both read
from this duplicated handoff rather than from the completed task record
itself.

## Decision

The completed task record — `Completed`, `Files changed`, `Result`, `Verify`
outcomes, `Context impact`, and `Context synchronization`, identified only by
plan path and task ID — is the sole durable input for both immediate task
context synchronization and later cross-session sync-debt recovery. No
separate `Context synchronization handoff` structure is constructed,
persisted, or read. A blocked synchronization adds only
synchronization-specific blocker metadata (`Blocker`, `Required action`,
`Retry condition`) beside that same task record; it does not duplicate the
task's execution facts. The `pending`/`synced`/`blocked` lifecycle-state
invariant decided in the superseded ADR is not re-decided and remains in
force unchanged.

## Rationale

The completed task record already contains everything a synchronization
retry needs once execution facts are written directly onto it in a compact
shape. A second serialized handoff duplicates that data, can drift from it,
and adds an evidence structure with no independent source of truth. Reading
the task record directly keeps the plan the single durable source for both
completion evidence and synchronization state, matching the same
plan-as-durable-store rationale the superseded decision already established.

## Alternatives considered

- **Keep the separate persisted handoff field** — Preserves duplication
  between the completed task's own evidence and the handoff's restated copy,
  with no benefit once the task record itself carries complete facts.
- **Drop the completed task record and rely solely on a handoff structure** —
  Loses the task record's authoritative status field
  (`(status:done)`/`Completed`) as a single place the plan-level view and the
  synchronization view actually agree.

## Compatibility and risks

- Historical plans authored before this decision may still carry the
  duplicated-field format; they are not migrated by this decision and are
  read only by the surviving legacy-migration branch when their completed
  tasks carry unresolved synchronization debt.
- Generation-contract checks and fixtures assert the compact fields are
  present and the removed fields/sections are absent from newly generated
  workflow instructions and plan-template examples.

## Guardrails

- Do not persist or read a `Context synchronization handoff` field, or any
  differently named field that recreates the same duplication (for example, a
  verbose "Implementation summary" alongside `Result`).
- Do not weaken the `pending`/`synced`/`blocked` lifecycle-state invariant or
  its blocker/required-action/retry-condition requirement for `blocked`.
- Keep `Result` a short factual outcome, not a prose diff.

## Consequences

- A completed task is the single authoritative record of implementation
  intent, completion conditions, verification, actual execution result,
  changed files, context impact, and context synchronization state.
- Sync-debt recovery identifies a task only by plan path and task ID and
  reads that same record, with no separate cross-session artifact to keep in
  sync.
- Legacy plans predating this format are detected by the absence of the
  compact execution fields and blocked with migration guidance rather than a
  reconstructed retry.

## Follow-up

None.

## References

- Plan: [`simplify-task-record-format`](../plans/simplify-task-record-format.md)
- Task: `T01`, `T02`, `T03`, `T04`, `T05`
- Current-state context: [`Shared Context Code Workflows`](../sce/shared-context-code-workflow.md)
- Evidence: [`workflow-change-to-plan.pkl`](../../config/pkl/base/workflow-change-to-plan.pkl)
- Evidence: [`workflow-next-task.pkl`](../../config/pkl/base/workflow-next-task.pkl)
- Evidence: [`workflow-context-sync.pkl`](../../config/pkl/base/workflow-context-sync.pkl)
- Evidence: [`generation-contract-check.pkl`](../../config/pkl/renderers/generation-contract-check.pkl)
- Related decision: [`Persist Workflow Synchronization Lifecycle in Plans`](2026-08-12-persist-workflow-sync-lifecycle-in-plans.md)
