---
description: "Run `sce-plan-review` -> `sce-task-execution` -> `sce-context-sync` for one approved SCE task"
agent: "Shared Context Code"
subtask: false
entry-skill: "sce-plan-review"
skills:
  - "sce-plan-review"
  - "sce-task-execution"
  - "sce-context-sync"
  - "sce-validation"
permission:
  default: ask
  read: allow
  edit: allow
  glob: allow
  grep: allow
  list: allow
  bash: allow
  question: allow
  codesearch: allow
  lsp: allow
  skill:
    "*": ask
    "sce-plan-review": allow
    "sce-task-execution": allow
    "sce-context-sync": allow
    "sce-validation": allow
---

## Purpose
- Review, authorize, execute, verify, and context-sync one SCE plan task.

## Inputs
- `$ARGUMENTS`: plan name or path (required) and task ID `T0X` (optional).
- User decisions at readiness, authorization, and implementation gates.

## Preconditions
1. An existing plan and task can be resolved through `sce-plan-review`.

## Workflow
1. Load `sce-plan-review`, resolve the selected task, and produce its structured readiness verdict.
2. If `ready_for_implementation: no`, report the issues and focused questions, then stop.
3. If readiness requires authorization and authorization is absent, report the verdict, request authorization, then stop.
4. If readiness is auto-authorized or explicitly authorized, immediately load `sce-task-execution`; present the task goal, boundaries, done checks, expected changes, approach, trade-offs, and risks; then ask `Continue with implementation now? (yes/no)` and wait.
5. If implementation is not confirmed, modify no files and return `current_task_incomplete`.
6. If implementation is confirmed, execute only the selected task, run its required checks, record evidence, update its plan status, and load `sce-context-sync` as the done gate.
7. Apply only in-scope feedback, rerun affected lightweight checks, and synchronize context again before continuing.
8. After successful execution and context synchronization, re-read the updated plan and resolve exactly one continuation:
   - `current_task_incomplete` if the selected task remains incomplete;
   - `next_task` for the first plan-ordered incomplete task whose dependencies are satisfied;
   - `blocked` if incomplete tasks remain but none are executable;
   - provisional `plan_complete` if no incomplete tasks remain.
9. Before returning `plan_complete`, load `sce-validation`; if final validation fails, return `blocked` with the evidence.
10. Render the state-appropriate output. For `next_task`, make the exact `/next-task {plan_path} {task_id}` command the final response content.

## Guardrails
- Execute only the confirmed current task; never execute the resolved next task.
- Modify no files before the implementation confirmation gate passes.
- Stop before expanding beyond the accepted task scope.

## Outputs
- `not_ready`: readiness verdict, blockers or ambiguities, and focused questions.
- `authorization_required`: readiness verdict and an explicit authorization request.
- `implementation_gate`: authorized readiness verdict, task-execution gate, and `Continue with implementation now? (yes/no)` as the final line.
- After confirmed execution: changes, verification evidence, updated task status, context-sync result, and exactly one continuation:
  - `next_task`: a final `### Next task: {task_id} — {task_title}` section with the plan path and exact `/next-task {plan_path} {task_id}` command as the last content;
  - `plan_complete`: final-validation evidence and no next-task command;
  - `blocked`: exact blocker and no invented command;
  - `current_task_incomplete`: remaining work and no next-task command.

## Completion criteria
- The invocation ends at the correct workflow boundary with exactly one state-appropriate result.

## Failure handling
- If execution or context synchronization cannot complete within the accepted scope, return `current_task_incomplete` and do not resolve a next task.
- If final validation fails, return `blocked`.

## Related units
- `shared-context-code` — execution profile bound to this workflow.
- `sce-plan-review` — entry skill for this workflow.
- `sce-task-execution` — required skill for this workflow.
- `sce-context-sync` — required skill for this workflow.
- `sce-validation` — required skill for this workflow.
