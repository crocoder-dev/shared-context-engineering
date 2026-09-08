---
name: sce-next-task
description: >
  Review, approve, implement, verify, and synchronize one SCE plan task
---

# SCE Next Task

## Execution contract

Own this workflow from input through its terminal user-visible response.
Follow its steps, gates, and stops in order; do not add, skip, reorder, or merge them.
Keep internal phase results private and continue immediately until a defined wait or stop.
Resume user waits in this same skill and session.
Render user-visible output only from the named workflow layouts or phase reports.
Do not expose raw internal state or add text around a rendered layout or report.
Non-SCE helpers may assist, but must return to the active step without changing
phase order, gates, waits, writes, validation, stops, or terminal output.
Do not invoke another SCE skill, package, or workflow command except `sce-decision`,
and only from the successful context-synchronization decision gate.

## Phase references

Each numbered step below dispatches to a phase whose steps, gates, and boundaries
live in a reference file. This document holds the control flow — which phase runs,
what it receives, and how its result branches — and each reference holds the phase
itself.

| Step | Read before running the phase |
|---|---|
| 1 | `references/plan-review.md` |
| 2 | `references/task-execution.md` |
| 3 | `references/context-sync.md` |

Read a step's reference before taking any action for that step, not after. The
references carry gates that must fire before their phase's first side effect, so a
phase begun from this summary alone will already have skipped them. Read only the
reference for the step you have reached: a run that stops at step 1 never needs the
other two, which is why they are separate files.

## Input

Parse `invocation input` into three positional parts before invoking any phase:

<plan-name-or-path> [task-id] [auto-approve]

- `plan-name-or-path` is required.
- `task-id` is optional. It is present only when the token matches a task ID (`T01`, `T02`, ...).
- `auto-approve` is optional. It is present only when the token is exactly `approved`.

Resolve `auto-approve` even when `task-id` is absent.

A token matching neither a task ID nor `approved` is an error. Report the unrecognized token and the expected arguments, and stop. Do not guess its meaning.

Pass each part only to the phase that owns it. Do not forward the raw `invocation input` string to a phase.

Every `{plan-path}` and `{candidate-path}` emitted anywhere in this workflow is the path resolved in step 1 (`plan.path`, or an entry of `candidates`), so every emitted command is directly runnable.

For example: `$sce-next-task my-plan T03 approved`.

## Workflow

### 1. Review the task

Read `references/plan-review.md`, then run the **Plan review phase** with the
parsed `plan-name-or-path` and, when present, the parsed `task-id`.

Do not pass the `auto-approve` token to the **Plan review phase**.

Branch on `status`:

`blocked` -> Do not run implementation. Render the **Review blocked** layout from `references/output.md`. When `candidates` is present the plan could not be resolved, and each entry is a candidate path for `/next-task {candidate-path}`. `executable_tasks_remaining` true means another task remains executable and `/next-task {plan-path} {task-id}` selects one; false means no task in the plan can proceed until the plan is updated. Do not print the raw result. Stop.

`sync_debt` -> Read `references/context-sync.md`, then run the **Task context synchronization phase** with the resolved plan path, debt task ID/title, and completed task record returned by the **Plan review phase**, plus its persisted `Context synchronization blocker` when present. Pass that completed task record verbatim. Do not reconstruct missing task data from conversation history.

Write the debt task's lifecycle to the plan: `synced`, clearing its blocker, required action, and retry condition, for `synced` or `no_context_change`; a refreshed `blocked` state with the report's blocker, required action, and retry condition for `blocked`. If that lifecycle write fails, treat the outcome as `blocked`.

Branch on the outcome:

`blocked` -> Render the **Context synchronization blocked** layout from `references/output.md`, distinct from the **Review blocked** layout above. The plan's task lifecycle record contains the blocker, required action, and retry condition. Do not select or start a new task. Stop.

`synced` | `no_context_change` -> Re-invoke the **Plan review phase** with the same `plan-name-or-path` and, when present, `task-id` to resume normal task selection.

`plan_complete` -> Render the **Implementation complete** layout from `references/output.md`. Stop.

`ready` -> Pass the complete readiness result to the **Task execution phase**.

Do not reconstruct, summarize, or reinterpret the reviewed task before passing it.

The review inspects every completed task's `Context synchronization` field in
the plan, in plan order, regardless of its position relative to the task being
selected or resumed, before allowing a new implementation task to start. A
missing field, or any value other than `synced`, is unresolved synchronization
debt. Never infer `synced` from conversation history. When the debt-carrying
task has no durable completed-task record, the **Plan review phase** returns
`blocked` directly with a legacy-migration required action; otherwise it returns
`sync_debt` with the resolved plan path, debt task ID/title, completed task
record, and persisted blocker when present, for the branch above to route.

### 2. Execute the task

Read `references/task-execution.md`, then run the **Task execution phase** with
the complete `ready` result from the **Plan review phase**.

Pass the `approve` flag only when `auto-approve` is `approved`; otherwise omit it.
The **Task execution phase** owns the implementation gate, approval question, wait,
user-decision handling, and no-edit-before-approval boundary. Do not duplicate that
procedure here or present an additional implementation confirmation.

Branch on the execution result.

`declined` -> Render the **Declined** layout from `references/output.md`. Do not run context synchronization. Stop.

`blocked` -> Render the **Execution blocked or incomplete** layout from `references/output.md`. Do not run context synchronization. Stop.

`incomplete` -> Render the same **Execution blocked or incomplete** layout. Do not run context synchronization. Do not select another task. Stop.

`complete` -> continue to the next step.

### 3. Synchronize context

Read `references/context-sync.md`, then run the **Task context synchronization
phase** with the task execution result whose `status` is `complete`.

Pass that result unchanged as the authoritative live handoff to the
**Task context synchronization phase**. Do not restate, summarize, or reconstruct it.

This phase verifies the five root context files on every invocation, whatever the
change's reported impact, so it is never correct to skip it as unnecessary.

Before branching on the synchronization result, write the completed task's
lifecycle to the plan file: `synced` for `synced` or `no_context_change`, and
`blocked` with the report's blocker, required action, and retry condition for
`blocked`. If that lifecycle write fails, treat synchronization as `blocked`.

Branch on the synchronization result.

`blocked` -> The task itself succeeded and is already marked complete in the plan. Render the **Context synchronization blocked** layout from `references/output.md`. The plan's task lifecycle record contains the blocker, required action, and retry condition.

Do not select another task. Stop.

`synced` | `no_context_change` -> Render the Markdown report returned by the **Task context synchronization phase** unchanged. Continue to the next step.

### 4. Determine the continuation

Use `plan.completed_tasks` and `plan.total_tasks` from the execution result to determine which continuation applies.

Do not execute another task. Return exactly one continuation.

If incomplete tasks remain, read the plan and name the first unchecked task in plan order. Do not evaluate its dependencies; the **Plan review phase** checks them when the emitted command runs and returns `blocked` if they are unmet.

Render the **More tasks remain** layout from `references/output.md`.

If all tasks are completed, render the **All tasks complete** layout instead.

Stop.

## Rules

- Execute at most one plan task per invocation.
- Review at most one task.
- Read each phase's reference before running that phase.
- Do not duplicate the internal instructions of embedded phases.
- The only permitted sibling-skill invocation is `sce-decision`, and only the
  successful context-synchronization decision gate may invoke it.
- Do not ask for implementation confirmation outside "Task execution phase".
- Do not run full-plan validation.
- Do not mark the plan complete.
- Do not execute the continuation returned at the end.
- Do not infer success when an embedded phase returns a non-success status.
- Preserve completed work and evidence when a later phase fails.
