---
name: sce-plan-review
description: |
  Use when the user wants to continue, resume, or pick up the next step from a markdown plan file stored in `context/plans/`. Reads the selected plan file, identifies the first unchecked checkbox as the next task, validates that acceptance criteria are clear and blockers are resolved, then issues an explicit readiness verdict before proceeding to implementation. Use when the user says "continue the plan", "what's the next task", "resume work", "pick up where we left off", or references a specific plan file.
compatibility: opencode
---

## Purpose
- Select the next task from an active plan and produce a deterministic readiness verdict.

## Inputs
- Plan name/path and optional task ID, current plan state, relevant context, and code truth.

## Preconditions
1. Require an existing `context/` tree.
2. Use an explicit plan path when supplied.
3. Auto-select only when exactly one plan exists; stop with an available-plan list when multiple plans exist without an explicit target.

## Workflow
1. Read context map, overview, and glossary before broad exploration.
2. Open the plan and select the explicit task or first unchecked task.
3. Extract task goal, boundaries, acceptance, verification, and dependencies.
4. Compare with current code/context truth.
5. Categorize every blocker, ambiguity, and missing criterion.
6. Emit the stable readiness shape.
7. Auto-proceed only when the verdict is `yes`; otherwise stop with a structured error.

## Guardrails
- Do not mark tasks complete during review.
- Execute one task only.
- Do not ask interactive questions in the automated profile.
- Prefer code truth and flag stale context.

## Outputs
- Structured readiness verdict or categorized blocking error.

## Completion criteria
- A unique task is selected and all acceptance and verification details are executable.

## Failure handling
- List available plans when target selection is ambiguous.
- List all unresolved items with categories and required human action.

## Related units
- `sce-task-execution` — auto-starts only on a clean verdict.
- `/next-task` — automated orchestrator.

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
