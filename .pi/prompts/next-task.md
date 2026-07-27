---
description: "Run `sce-plan-review` -> `sce-task-execution` -> `sce-task-context-sync` for one SCE plan task"
argument-hint: "<plan-name> [T0X] [approved]"
---

SCE NEXT TASK `$ARGUMENTS`

## Input

Parse `$ARGUMENTS` into three positional parts before invoking any skill:

<plan-name-or-path> [task-id] [auto-approve]

- `plan-name-or-path` is required.
- `task-id` is optional. It is present only when the token matches a task ID (`T01`, `T02`, ...).
- `auto-approve` is optional. It is present only when the token is exactly `approved`.

Resolve `auto-approve` even when `task-id` is absent.

A token matching neither a task ID nor `approved` is an error. Report the unrecognized token and the expected arguments, and stop. Do not guess its meaning.

Pass each part only to the phase that owns it. Do not forward the raw `$ARGUMENTS` string to a skill.

Every `{plan-path}` and `{candidate-path}` emitted anywhere in this workflow is the path resolved by `sce-plan-review` (`plan.path`, or an entry of `candidates`), so every emitted command is directly runnable.

## Workflow

### 1. Review the task

Invoke `sce-plan-review` with the parsed `plan-name-or-path` and, when present, the parsed `task-id`.

Do not pass the `auto-approve` token to `sce-plan-review`.

The skill must return a result matching its readiness contract.

Branch on `status`:

`blocked` -> Do not invoke implementation. Present the result as prose. Do not print the raw result. Stop.

When `candidates` is present, the plan could not be resolved. Present:

- The problem reported by the review.
- The candidate plan paths.
- `/next-task {candidate-path}` for the intended plan.

Otherwise one plan and task were resolved. Present:

- The task ID and title.
- Each issue in `issues`: its problem, its impact, and the decision it requires.
- When `executable_tasks_remaining` is true: other tasks remain executable, and `/next-task {plan-path} {task-id}` selects one.
- When `executable_tasks_remaining` is false: no task in the plan can proceed until the plan is updated.

`plan_complete` -> Return:

```

-------------------------------------

# Implementation tasks are complete.

Run the final validation:

`/validate {plan-path}`
```

Stop.

`ready` -> Pass the complete readiness result to `sce-task-execution`.

Do not reconstruct, summarize, or reinterpret the reviewed task before passing it.

### 2. Execute the task

Invoke `sce-task-execution` with the complete `ready` result from `sce-plan-review`.

Branch on `auto-approve`:

`approved` -> Also pass the `approve` flag. `sce-task-execution` then shows its implementation gate as a summary and proceeds without asking.

else -> Do not pass the `approve` flag. `sce-task-execution` shows its implementation gate and waits for the user's decision.

`sce-task-execution` exclusively owns:

- Presenting the implementation summary.
- Requesting implementation confirmation.
- Implementing the task.
- Running task-level verification.
- Updating the task status and evidence.

Do not present an additional implementation confirmation.

Branch on the execution result.

`declined` -> Present "You have declined to proceed with this task". Do not invoke context synchronization. Stop.

`blocked` -> Present:

- The blocker.
- Work completed before the blocker.
- The decision or action required.

Do not invoke context synchronization. Stop.

`incomplete` -> Present:

- Work completed.
- Verification evidence.
- Remaining work.
- The reason the task is incomplete.

Do not invoke context synchronization. Do not select another task. Stop.

`complete` -> continue to the next step.

### 3. Synchronize context

Invoke `sce-task-context-sync` with the complete `complete` result returned by `sce-task-execution`.

Pass that result verbatim. It is the authoritative handoff, and `sce-task-context-sync` owns reading the plan, task, changed files, verification evidence, and reported context impact out of it.

Do not restate, summarize, or reconstruct any part of the execution result.

Branch on the synchronization result.

`blocked` -> The task itself succeeded and is already marked complete in the plan. Present:

- That task {completed-task-id} was implemented, verified, and recorded in the plan.
- The context contradiction or synchronization failure.
- Any context edits the report says were preserved.
- The action required to resolve the problem.
- The retry condition stated by the report.

State that durable context is now out of date, and that synchronization must be resolved before continuing the plan. Nothing records the skipped synchronization, so it is lost once this session ends.

Do not select another task. Stop.

`synced` | `no_context_change` -> Print out the report `sce-task-context-sync` returned. Continue to the next step.

### 4. Determine the continuation

Use `plan.completed_tasks` and `plan.total_tasks` from the execution result to determine which continuation applies.

Do not execute another task. Return exactly one continuation.

If incomplete tasks remain, read the plan and name the first unchecked task in plan order. Do not evaluate its dependencies; `sce-plan-review` checks them when the emitted command runs and returns `blocked` if they are unmet.

Return:

```

-------------------------------------

# Task {completed-task-id} completed.

{completed-tasks} of {total-tasks} tasks complete.

Next up:

{next-task-id} — {next-task-title}

`/next-task {plan-path} {next-task-id}`
```

If all tasks are completed return:

```

-------------------------------------

# Task {completed-task-id} completed.

All tasks are complete.

Run the final validation:

`/validate {plan-path}`
```

Stop.

## Rules

- Execute at most one plan task per invocation.
- Review at most one task.
- Do not duplicate the internal instructions of invoked skills.
- Do not ask for implementation confirmation outside "sce-task-execution".
- Do not run full-plan validation.
- Do not mark the plan complete.
- Do not execute the continuation returned at the end.
- Do not infer success when an invoked skill returns a non-success status.
- Preserve completed work and evidence when a later phase fails.
