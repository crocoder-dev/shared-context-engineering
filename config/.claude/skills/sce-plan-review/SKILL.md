---
name: sce-plan-review
description: |
  Reviews an existing SCE plan file (a Markdown checklist in `context/plans/`) to identify the next unchecked task, surface blockers or ambiguous acceptance criteria, and produce an explicit readiness verdict before implementation begins. Use when the user wants to continue a plan, resume work, pick the next step, or check what remains in an active plan — e.g. "continue the plan", "what's next?", "resume work on the plan", "review my plan and prepare the next task".
compatibility: claude
---

## Purpose
- Review an active SCE plan, identify the next task, and issue an explicit implementation-readiness verdict.

## Inputs
- Plan name/path and optional task ID.
- Current plan checkboxes, task details, relevant context, and code truth.

## Preconditions
1. Ensure `context/` exists; when missing, ask once whether to run `sce-bootstrap-context`, then stop if declined.
2. Read `context/context-map.md`, `context/overview.md`, and `context/glossary.md` before broad exploration.
3. Use an explicit plan path when provided; when multiple plans exist without one, ask the user to choose.

## Workflow
1. Open the selected plan and count completed and remaining tasks.
2. Select the explicit task ID when provided; otherwise select the first unchecked task.
3. Extract goal, boundaries, done checks, verification notes, dependencies, and relevant decisions.
4. Compare plan assumptions with current code and context.
5. Classify issues as blockers, ambiguity, or missing acceptance criteria.
6. Return `ready_for_implementation: yes|no` and the decisions required to proceed.
7. When unresolved issues remain, request explicit user resolution and keep implementation blocked.

## Guardrails
- Do not mark tasks complete during review.
- Do not reorder or rewrite plan structure without approval.
- Confirm one-task scope by default.
- Treat completed plans as disposable, not durable history.
- Prefer code truth when the plan or context is stale and flag the required repair.

## Outputs
- A structured readiness summary with completed count, selected task, acceptance criteria, issue categories, and verdict.

## Completion criteria
- The selected task is unambiguous, bounded, and has observable acceptance and verification.
- The verdict is explicit and no unresolved issue is hidden.

## Failure handling
- Stop and ask for a plan choice when multiple candidates exist.
- Return `ready_for_implementation: no` and focused questions when any blocker, ambiguity, or missing criterion remains.
- Stop when no unchecked task exists and report that the plan is ready for final validation or closure.

## Related units
- `sce-bootstrap-context` — create missing baseline context after approval.
- `sce-task-execution` — runs only after readiness authorization.
- `/next-task` — orchestrates review, execution, and context sync.

## Reference
Return readiness in this stable shape:

```yaml
plan: context/plans/{plan_name}.md
completed_tasks: 2/5
next_task:
  id: T03
  title: Implement login endpoint
acceptance_criteria:
  - POST /auth/login returns a token for valid credentials
  - Invalid credentials return 401
issues:
  blockers: []
  ambiguity: []
  missing_acceptance_criteria: []
ready_for_implementation: yes
```
