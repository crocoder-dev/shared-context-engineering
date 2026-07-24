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
- Route final tasks through full validation and non-final tasks to a clean next-session handoff.

## Inputs
- `$ARGUMENTS`: plan name or path (required) and task ID `T0X` (optional).
- User decisions or confirmation when the readiness gate cannot auto-pass.

## Preconditions
1. Resolve an existing plan and task through `sce-plan-review`.
2. Require no blockers, ambiguity, or missing acceptance criteria.
3. Auto-pass readiness only when both plan and task ID are explicit and review is clean; otherwise obtain explicit user confirmation.
4. Treat authorization as permission to present the implementation gate, never as permission to implement before that gate is confirmed.

## Workflow
1. Load `sce-plan-review` and return its structured readiness verdict.
2. When `ready_for_implementation: no`, report the issues and focused questions, then stop.
3. When readiness requires authorization and authorization is absent, report the verdict, request authorization, then stop.
4. When readiness is auto-authorized or explicitly authorized, immediately load `sce-task-execution`, present its scope, approach, trade-offs, and risks, and end the same response with `Continue with implementation now? (yes/no)`.
5. When implementation confirmation is absent or negative, modify no files and return the structured outcome `current_task_incomplete`.
6. When implementation is confirmed, execute one task, run its checks, update its plan status, and load `sce-context-sync` as a done gate.
7. Wait for feedback; apply only in-scope fixes, rerun light checks, and synchronize context again.
8. After successful execution and context synchronization, re-read the updated plan from disk.
9. Resolve exactly one continuation outcome: `current_task_incomplete` when the selected task remains incomplete; otherwise `next_task` for the first plan-ordered incomplete task whose dependencies are satisfied; otherwise `blocked` when incomplete tasks remain but none are executable; otherwise a provisional `plan_complete`.
10. Before returning `plan_complete`, load `sce-validation` and return completion only after final validation passes; report a failed final validation as `blocked` with its exact evidence.
11. For `next_task`, render a final `### Next task: {task_id} — {task_title}` section containing the actual plan path, task ID, title, and exact `/next-task {plan_path} {task_id}` invocation, with nothing after the command.

## Guardrails
- Keep this command as orchestration; detailed review, implementation, sync, and validation rules remain skill-owned.
- Execute one task by default and never execute the resolved next task automatically.
- Do not write code before readiness authorization and the exact task-execution confirmation gate passes.
- Select continuation by plan order and satisfied dependencies, never by task-ID arithmetic.
- Emit exactly one structured continuation outcome: `next_task`, `plan_complete`, `blocked`, or `current_task_incomplete`.
- Do not append a generic review tail or invent a next-task command for complete, blocked, or incomplete outcomes.
- Stop before scope expansion.

## Outputs
- A readiness verdict and, only when authorized, the task-execution confirmation gate.
- Implemented changes with verification evidence, updated task status, and context-sync results after confirmation.
- Exactly one structured continuation outcome with `outcome` and `plan`; `next_task` additionally includes the actual `task_id`, `title`, and `command`, while blocked or incomplete results include the exact blocker or remaining work.

## Completion criteria
- The selected task is complete with evidence and synchronized context, or is explicitly reported as `current_task_incomplete` without premature writes.
- The updated plan has been re-read and exactly one deterministic continuation outcome has been returned.
- A `next_task` command is the final response content; all other outcomes contain no invented command or generic tail.

## Failure handling
- Stop on `ready_for_implementation: no` with issues and focused questions.
- Stop when readiness authorization is required but absent, or when implementation confirmation is absent or negative; preserve the selected task as `current_task_incomplete` and modify no files.
- Stop on scope expansion, failed checks that cannot be fixed in scope, or context-sync blockers.
- Preserve partial evidence, report the exact phase that failed, and do not emit a next-task command while the current task remains incomplete.

## Related units
- `sce-plan-review` — task selection and readiness.
- `sce-task-execution` — implementation and task-level evidence.
- `sce-context-sync` — durable context reconciliation.
- `sce-validation` — final full validation and cleanup.
