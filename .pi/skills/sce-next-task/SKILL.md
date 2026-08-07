---
name: sce-next-task
description: >
  Review, approve, implement, verify, and synchronize one SCE plan task
---

# SCE Next Task

## Purpose

Own this workflow from input parsing through its terminal user-visible response.
Execute the phases below directly and in order. Phase statuses are internal state,
not inter-SCE workflow handoffs. Do not invoke another SCE skill, sibling SCE
package, or SCE workflow command except `sce-decision`, and invoke `sce-decision`
only from the successful context-synchronization decision gate. Follow the canonical workflow's steps, gates,
and stops exactly as written: never invent, skip, reorder, or merge a step.

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

## User-visible output

Use `references/output.md` for every gate and terminal response. Render no raw
internal state. The reference contains only human-visible Markdown layouts.
User-visible output is limited to those layouts: never invent a layout, and never
wrap one in an added preamble, commentary, summary, or extra section.

## Composite control flow

Keep phase results as internal state and continue immediately whenever the
canonical workflow says to continue. Stop only at a user wait or terminal branch.
Approval, clarification, revision, failed-validation repair, and bootstrap waits
resume this same skill in the same session. Never expose an internal phase result
as the workflow's final response.

Relevant non-SCE skills may be used as helper capabilities during the active step.
They are not workflow handoffs: when a helper returns, control returns to the active
step. Helper use must preserve the canonical phase order, gates, waits, writes,
validation, stops, and terminal user-visible output.

## Input

Parse `$ARGUMENTS` into three positional parts before invoking any phase:

<plan-name-or-path> [task-id] [auto-approve]

- `plan-name-or-path` is required.
- `task-id` is optional. It is present only when the token matches a task ID (`T01`, `T02`, ...).
- `auto-approve` is optional. It is present only when the token is exactly `approved`.

Resolve `auto-approve` even when `task-id` is absent.

A token matching neither a task ID nor `approved` is an error. Report the unrecognized token and the expected arguments, and stop. Do not guess its meaning.

Pass each part only to the phase that owns it. Do not forward the raw `$ARGUMENTS` string to a phase.

Every `{plan-path}` and `{candidate-path}` emitted anywhere in this workflow is the path resolved in step 1 (`plan.path`, or an entry of `candidates`), so every emitted command is directly runnable.

## Workflow

### 1. Review the task

Read `references/plan-review.md`, then run the **Plan review phase** with the
parsed `plan-name-or-path` and, when present, the parsed `task-id`.

Do not pass the `auto-approve` token to the **Plan review phase**.

Branch on `status`:

`blocked` -> Do not run implementation. Render the **Review blocked** layout from `references/output.md`. When `candidates` is present the plan could not be resolved, and each entry is a candidate path for `/next-task {candidate-path}`. `executable_tasks_remaining` true means another task remains executable and `/next-task {plan-path} {task-id}` selects one; false means no task in the plan can proceed until the plan is updated. Do not print the raw result. Stop.

`plan_complete` -> Render the **Plan already complete** layout from `references/output.md`. Stop.

`ready` -> Pass the complete readiness result to the **Task execution phase**.

Do not reconstruct, summarize, or reinterpret the reviewed task before passing it.

### 2. Execute the task

Read `references/task-execution.md`, then run the **Task execution phase** with
the complete `ready` result from the **Plan review phase**.

This phase always shows an implementation gate before it modifies any file, and it
is the only phase permitted to ask the user for confirmation. Both properties are
load-bearing, so reach them through the reference rather than acting from this
summary.

Branch on `auto-approve`:

`approved` -> Also pass the `approve` flag. The **Task execution phase** then shows its implementation gate as a summary and proceeds without asking.

else -> Do not pass the `approve` flag. The **Task execution phase** shows its implementation gate and waits for the user's decision.

Do not present an additional implementation confirmation.

Branch on the execution result.

`declined` -> Render the **Declined** layout from `references/output.md`. Do not run context synchronization. Stop.

`blocked` -> Render the **Execution blocked or incomplete** layout from `references/output.md`. Do not run context synchronization. Stop.

`incomplete` -> Render the same **Execution blocked or incomplete** layout. Do not run context synchronization. Do not select another task. Stop.

`complete` -> continue to the next step.

### 3. Synchronize context

Read `references/context-sync.md`, then run the **Task context synchronization
phase** with the complete `complete` result returned by the **Task execution
phase**.

Pass that result verbatim. It is the authoritative handoff, and the **Task context synchronization phase** owns reading the plan, task, changed files, verification evidence, and reported context impact out of it.

Do not restate, summarize, or reconstruct any part of the execution result.

This phase verifies the five root context files on every invocation, whatever the
change's reported impact, so it is never correct to skip it as unnecessary.

Branch on the synchronization result.

`blocked` -> The task itself succeeded and is already marked complete in the plan. Render the **Context synchronization blocked** layout from `references/output.md`. Nothing records the skipped synchronization, so it is lost once this session ends.

Do not select another task. Stop.

`synced` | `no_context_change` -> Print out the report the **Task context synchronization phase** returned. Continue to the next step.

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
