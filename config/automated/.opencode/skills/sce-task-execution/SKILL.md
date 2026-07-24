---
name: sce-task-execution
description: |
  Executes a single scoped coding task with structured implementation logging, test and lint evidence capture, and plan status updates. Use when the user wants to run, implement, or complete one approved task from a plan — such as writing code changes, applying fixes, or executing a defined unit of work — while enforcing strict single-task scope boundaries, logging intent to context/tmp/automated-session-log.md, and updating task status in context/plans/{plan_id}.md. Triggers on phrases like "execute task", "run this task", "implement task", "do the next task", or "apply this change".
compatibility: opencode
---

## Purpose
- Implement exactly one approved task non-interactively while logging intent, enforcing scope, capturing evidence, and updating plan status.

## Inputs
- A reviewed task with `ready_for_implementation: yes`, explicit boundaries, done checks, verification notes, and repository state.

## Preconditions
1. Require exactly one task; automated multi-task execution is unsupported.
2. Require a clean readiness verdict.
3. Prepare an implementation-intent record before modifying code.

## Workflow
1. Restate goal, in/out boundaries, done checks, expected files/components, approach, trade-offs, and risks.
2. Append that intent to `context/tmp/automated-session-log.md` with timestamp and task ID.
3. Proceed without waiting for confirmation.
4. Implement the minimal in-scope change.
5. Run targeted checks and lints plus a light/fast build when applicable.
6. Capture commands, exit codes, and key evidence.
7. Classify context impact and update plan task status.

## Guardrails
- Do not execute multiple tasks.
- Do not expand scope, reorder the plan, or add unrelated refactors.
- Stop immediately with `BLOCKER: scope_expansion_required` when out-of-scope work is necessary.
- Keep session-only scraps under `context/tmp/`.

## Outputs
- Intent log entry, minimal implementation, evidence, context-impact classification, and updated plan status.

## Completion criteria
- Done checks pass with evidence and the task remains within declared boundaries.

## Failure handling
- Return structured blocker details and required human action for scope expansion or non-trivial failed checks.
- Leave the task incomplete until failures are resolved and reverified.

## Related units
- `sce-plan-review` — readiness owner.
- `sce-context-sync` — mandatory post-implementation gate.
- `sce-validation` — final-plan validation.

## Reference
Log shape:

```markdown
## {timestamp} T0X: {task_title}
- Goal: ...
- In scope: ...
- Out of scope: ...
- Expected files: ...
- Approach: ...
- Trade-offs: ...
- Risks: ...
- Status: proceeding
```
