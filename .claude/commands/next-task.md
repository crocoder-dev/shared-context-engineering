---
description: "Run `sce-plan-review` -> `sce-task-execution` -> `sce-context-sync` for one approved SCE task"
allowed-tools: Task, Read, Glob, Grep, Edit, Write, Question, Skill, Bash
---

## Purpose
- Review, authorize, execute, verify, and context-sync one SCE plan task.
- Route final tasks through full validation and non-final tasks to a clean next-session handoff.

## Inputs
- `$ARGUMENTS`: plan name or path (required) and task ID `T0X` (optional).
- User decisions or confirmation when the readiness gate cannot auto-pass.

## Preconditions
1. Resolve an existing plan and task through `sce-plan-review`.
2. Require no blockers, ambiguity, or missing acceptance criteria.
3. Auto-pass readiness only when both plan and task ID are explicit and review is clean; otherwise obtain explicit user confirmation.

## Workflow
1. Load `sce-plan-review` and return its readiness verdict.
2. Resolve open points and obtain readiness authorization when required.
3. Load `sce-task-execution` and preserve its mandatory pre-implementation stop.
4. After implementation, load `sce-context-sync` as a done gate.
5. Wait for feedback; apply only in-scope fixes, rerun light checks, and sync context again.
6. If the task is final, load `sce-validation`; otherwise return `/next-task {plan_name} T0X` for the next unchecked task.

## Guardrails
- Keep this command as orchestration; detailed review, implementation, sync, and validation rules remain skill-owned.
- Execute one task by default.
- Do not write code before readiness authorization and the task-execution gate pass.
- Stop before scope expansion.

## Outputs
- A readiness verdict.
- Implemented changes with verification evidence and updated task status.
- Context-sync results.
- Either a final validation result or the exact next-session command.

## Completion criteria
- The selected task is complete with evidence and synchronized context.
- Final tasks include a validation report; non-final tasks include the next task handoff.

## Failure handling
- Stop on unresolved readiness issues and list the decision needed.
- Stop on scope expansion, failed checks that cannot be fixed in scope, or context-sync blockers.
- Preserve partial evidence and report the exact phase that failed.

## Related units
- `sce-plan-review` — task selection and readiness.
- `sce-task-execution` — implementation and task-level evidence.
- `sce-context-sync` — durable context reconciliation.
- `sce-validation` — final full validation and cleanup.
