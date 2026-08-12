# Decision: Use Explicit Baseline-Relative Task Handoffs

Date: 2026-08-12
Status: Accepted
Plan: `context/plans/workflow-skill-boundary-cleanup.md`
Task: `T06`

## Context

The task-execution and task-context-synchronization phases are separate internal
steps of one generated workflow, but the synchronization phase must receive an
authoritative account of what implementation changed. Whole-working-tree Git
status or a diff against `HEAD` can include unrelated pre-existing staged,
unstaged, or untracked work. Missing or contradictory handoff fields can also
cause synchronization to reconstruct facts from conversation history.

## Decision

Use an explicit, baseline-relative execution handoff as the sole interface from
task execution to task context synchronization. Before editing, execution
captures the Git `HEAD`, staged and unstaged patch/content state, and untracked
path/content state; after implementation it compares the same snapshot shape and
reports only task-caused paths in `changes.files_changed`, alongside the resolved
plan and task identity, implementation summary, verification and done-check
evidence, plan update, and context impact. The synchronization phase consumes
that list as authoritative. The mandatory five-root-file context pass remains
unconditional for every completed task.

## Rationale

An explicit contract makes the cross-phase handoff inspectable and prevents the
context phase from silently broadening its evidence to unrelated working-tree
changes. Baseline comparison preserves useful attribution when a repository was
already dirty before implementation, while required fields make successful
handoffs complete without conversational reconstruction.

## Alternatives considered

- **Diff the final working tree against `HEAD`** — This attributes unrelated
  pre-existing changes to the task.
- **Let synchronization rediscover changed files** — This duplicates execution
  responsibility and can produce different results from the implementation
  evidence.
- **Make the root-file pass conditional on context impact** — This would permit
  cross-cutting context drift to escape the deterministic accuracy check.

## Compatibility and risks

- The generated execution contract now requires richer complete handoffs and
  blocks stale, invalid, or contradictory handoffs, including under auto-approval.
- A path modified both before and during implementation is reported once when
  its state or content changes during the task; the handoff does not claim to
  separate unrelated hunks within that path.

## Guardrails

- Capture the baseline before any file modification and compare the same state
  shape after implementation.
- Do not use whole-working-tree status or a `HEAD`-only diff as `files_changed`.
- Do not omit, invent, or reconstruct required complete-handoff fields.
- Keep the five-root-file context pass mandatory regardless of reported impact.
- Keep the handoff contract in the execution phase reference and its existing
  result contract; do not create a standalone execution reference.

## Consequences

- Task context synchronization receives deterministic, task-attributed file
  evidence and can block safely when the handoff is incomplete or contradictory.
- Pre-existing unrelated work remains outside the changed-file list, reducing
  accidental context edits and misleading synchronization reports.
- Every successful task still pays the small deterministic cost of verifying the
  five root context files.

## Follow-up

None.

## References

- Plan: [`workflow-skill-boundary-cleanup`](../plans/workflow-skill-boundary-cleanup.md)
- Task: `T06`
- Current-state context: [`Shared Context Code Workflows`](../sce/shared-context-code-workflow.md)
- Current-state context: [`Architecture`](../architecture.md)
- Evidence: [`workflow-next-task.pkl`](../../config/pkl/base/workflow-next-task.pkl)
- Evidence: [`workflow-context-sync.pkl`](../../config/pkl/base/workflow-context-sync.pkl)
- Related decision: [`Persist Workflow Synchronization Lifecycle in Plans`](2026-08-12-persist-workflow-sync-lifecycle-in-plans.md)
