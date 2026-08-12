# Decision: Persist Workflow Synchronization Lifecycle in Plans

Date: 2026-08-12
Status: Accepted
Plan: `context/plans/workflow-skill-boundary-cleanup.md`
Task: `T03`

## Context

Task-level and plan-level context synchronization currently exists only as
in-session workflow state. A later session can therefore start another task or
finish validation without durable evidence that an earlier synchronization was
completed. The next-task and validate workflows share this lifecycle boundary
across all generated targets, so the state needs one durable, target-neutral
owner.

## Decision

Persist task-level and plan-level context synchronization lifecycle state in the
plan Markdown. Each lifecycle uses `pending`, `synced`, or `blocked`; a blocked
record carries its blocker, required action, and retry condition. A completed
task must have `synced` task-level state before another implementation task can
start or the plan can finish. The plan-level state becomes `pending` before
final plan synchronization and becomes `synced` or `blocked` afterward.

## Rationale

The plan already persists task status and completion evidence, is available in a
fresh session, and is the canonical input to both workflows. Extending that
format avoids a second persistence mechanism while making unresolved
synchronization debt explicit and actionable. Keeping the lifecycle contract in
the canonical Pkl plan template and shared workflow sources preserves parity for
Pi, Claude, and OpenCode.

## Alternatives considered

- **Keep lifecycle state in chat or transient phase results** — A fresh session
  cannot reliably observe or gate on it.
- **Create a separate lifecycle database or sidecar file** — This introduces a
  second persistence owner and complicates plan portability and cleanup.
- **Infer `synced` from historical evidence** — Existing reports may be absent or
  ambiguous, so inference would allow unresolved debt to pass silently.

## Compatibility and risks

- Plans authored before this decision may lack lifecycle fields; the workflows
  treat missing state on completed tasks as unresolved debt rather than silently
  declaring it synchronized.
- Existing task and plan status fields remain unchanged; lifecycle state is an
  additional persisted contract.
- A synchronization blocker now stops continuation while preserving the
  completed implementation, with the plan recording how to retry.

## Guardrails

- Keep the state in the Markdown plan format; do not add a database or alternate
  session store.
- Allow only `pending`, `synced`, and `blocked` lifecycle values.
- Require blocker, required-action, and retry-condition details for `blocked`.
- Write `pending` before invoking synchronization and write the terminal state
  after the synchronization result is known.
- Do not infer `synced` from conversation history.

## Consequences

- `/next-task` can refuse new implementation work when an earlier completed task
  has unresolved synchronization debt.
- `/validate` can refuse to treat a plan as finishable while task-level debt
  remains.
- The plan file becomes the durable handoff for synchronization retry state across
  sessions.

## Follow-up

None.

## References

- Plan: [`workflow-skill-boundary-cleanup`](../plans/workflow-skill-boundary-cleanup.md)
- Task: `T03`
- Current-state context: [`Shared Context Code Workflows`](../sce/shared-context-code-workflow.md)
- Evidence: [`workflow-change-to-plan.pkl`](../../config/pkl/base/workflow-change-to-plan.pkl)
- Evidence: [`workflow-next-task.pkl`](../../config/pkl/base/workflow-next-task.pkl)
- Evidence: [`workflow-validate.pkl`](../../config/pkl/base/workflow-validate.pkl)
- Evidence: [`workflow-content.pkl`](../../config/pkl/base/workflow-content.pkl)
