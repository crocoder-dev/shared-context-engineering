---
name: sce-task-execution
description: >
  Internal SCE workflow skill that always presents one reviewed task to the
  user before editing, executes it only after approval, verifies the
  task, records evidence in the plan, and returns one Markdown result: declined,
  blocked, incomplete, or complete. Accepts a ready result from
  sce-plan-review. Do not select or execute another task,
  synchronize durable context, run final plan validation, create commits, or
  expand task scope.
compatibility: claude
---

# SCE Task Execution

## Purpose

Execute exactly one reviewed SCE plan task (located in `context/plans/`).

This skill owns:

- Showing the implementation gate at the start of every invocation.
- Receiving the user's approval or rejection, or accepting approval
  pre-supplied by the invoking workflow.
- Implementing one approved task.
- Running task-level verification.
- Updating that task and its evidence in the plan.
- Returning one terminal Markdown result.

Use the gate defined in:

`references/implementation-gate.md`

Return a final result matching:

the **Result contract** section in this file

## Input

The invoking workflow provides:

- The complete `ready` result from `sce-plan-review`.
- An optional `approve` flag.

The `approve` flag means the user pre-approved this task when invoking the
workflow. It suppresses the approval question and the wait. It never suppresses
the gate. Only the invoking workflow may set it, and only from an explicit
user-supplied approval token. Never infer it.

The readiness result must identify:

- One resolved plan.
- Exactly one incomplete task.
- Relevant files and context.
- Review assumptions.

The readiness result is a compact handoff carrying only what review
discovered. Read the task's goal, scope boundaries, done checks,
dependencies, and verification from that task's entry in the plan file.
The handoff does not repeat them, and their absence from it is not a
handoff problem.

If required handoff information is absent or stale, still show the gate using
what is known, clearly identify the handoff problem, and do not edit files.
After the user responds, return `blocked`.

## Workflow

### 1. Validate the handoff without editing

Confirm that:

- The readiness status is `ready`.
- Exactly one task is present.
- The plan file exists.
- The selected task is still incomplete.
- The task entry in the plan still matches the reviewed task ID and title.
- Declared dependencies remain complete.

Do not reconstruct missing material requirements.

### 2. Always show the implementation gate

At the start of the skill, before any file modification, present the task using
`references/implementation-gate.md`. Read that task's entry in the plan file for
the gate's goal, scope, done-check, and verification fields.

The gate must be shown even when:

- The task appears straightforward.
- The invoking workflow believes approval was already implied.
- The handoff is stale or incomplete.
- The user is likely to approve.

When the `approve` flag is absent, end the gate with exactly one approval
question:

`Continue with implementation now? (yes/no)`

Stop and wait for the user's answer. Do not return Markdown, and make no file
modifications, until the user has answered.

When the `approve` flag is supplied, show the gate as a summary, omit the
approval question, do not wait, and continue at *Prepare the implementation*.

### 3. Handle the user's decision

Skip this step when the `approve` flag was supplied.

When the user rejects or cancels, do not modify files and return `declined`.

When the user does not clearly approve, do not modify files. Ask the same
approval question once more only when the response is genuinely ambiguous.
Otherwise return `blocked`.

When the user approves, continue with implementation.

Treat constraints supplied with approval as part of the approved task boundary.
If those constraints materially contradict the reviewed task, return `blocked`
before editing.

### 4. Prepare the implementation

Before editing:

- Read the relevant files supplied by plan review.
- Inspect nearby code and tests when needed.
- Identify the smallest coherent change satisfying the task.
- Follow surrounding naming, structure, error handling, and test style.
- Preserve unrelated behavior.

Do not create a second plan.

Do not broaden the reviewed task.

### 5. Implement one task

Make the minimum coherent changes required to satisfy the task goal and done
checks.

Use judgment for ordinary, reversible local implementation choices.

Stop when implementation requires:

- Material scope expansion.
- A new external dependency not authorized by the task.
- A public-interface decision not established by the plan.
- A destructive or difficult-to-reverse operation.
- An unresolved security, privacy, or data decision.
- Contradicting the reviewed task or repository architecture.

When stopped, preserve completed in-scope work unless retaining it would leave
the repository unsafe or invalid.

### 6. Verify the task

Run the narrowest authoritative checks that demonstrate the done checks.

Start with the verification declared by that task's entry in the plan. Add
nearby or directly relevant checks only when needed.

Verification may include:

- Targeted tests.
- Type checking for affected code.
- Linting affected files.
- Formatting checks.
- A focused build or compile step.
- Direct behavioral inspection when no automated check exists.

Do not run final plan validation unless the task itself explicitly requires it.

When a check fails:

- Determine whether the task caused the failure.
- Fix it when the correction remains in scope.
- Rerun the relevant check.
- Return `incomplete` when a done check remains unsatisfied, or `blocked` when
  completing it requires an unapproved decision or scope expansion.

Never report a check as passed unless it ran successfully.

### 7. Update the plan

Only after successful implementation and task-level verification:

- Mark only the selected task complete.
- Record concise implementation evidence.
- Record verification commands and outcomes.
- Record material deviations or approved assumptions.
- Preserve the plan's existing structure and terminology.

Do not mark the task complete when returning `declined`, `blocked`, or
`incomplete`.

### 8. Determine the terminal status

Return `complete` when the task was implemented, verified, and marked complete
in the plan with evidence.

Return `incomplete` when in-scope work was completed but one or more done checks
remain unsatisfied.

Return `declined` when the user rejected implementation.

Return `blocked` for every other non-successful outcome, including:

- Missing approval.
- Stale or invalid handoff.
- Material blocker.
- A verification failure that cannot be resolved in scope.

Use a blocker category defined by the **Result contract** section in this file.

Do not determine whether the plan is complete. The invoking `/next-task`
workflow owns that decision after context synchronization.

### 9. Return Markdown

After the skill reaches a terminal state, return exactly one Markdown document
matching the **Result contract** section in this file.

Return only the Markdown document. Do not add explanatory prose before or after it.

## Boundaries

Do not:

- Edit before approval, whether explicit or pre-supplied.
- Execute more than one task.
- Select or execute the next task.
- Skip the implementation gate.
- Ask for multiple approval gates for the same unchanged task.
- Expand scope without authorization.
- Synchronize durable context.
- Run final plan validation.
- Determine whether the plan is complete.
- Create a Git commit.
- Push changes.
- Modify unrelated files.
- Claim verification that was not performed.

## Completion

The skill is complete after:

- The implementation gate was shown.
- The user approved or rejected the task, or approval was pre-supplied.
- At most one task was executed.
- One valid terminal Markdown result matching the **Result contract** section in this file
  was returned.

## Result contract

# SCE Task Execution Result Contract

Return exactly one Markdown document using one layout below after the
implementation gate reaches a terminal state. `Status` is the branch value
consumed by `/next-task`. Use every required heading and label exactly as
written, omit optional sections that do not apply, and do not add prose
outside the selected layout. Empty required lists must contain `- None.`.

Report task counts as they stand. Never serialize the implementation gate,
select the next task, or include synchronization or final-validation results.

## Shared Plan and Task layout

Every status includes these sections:

```markdown
## Plan

- Path: {plan.path}
- Completed tasks: {plan.completed_tasks}
- Total tasks: {plan.total_tasks}

## Task

- ID: {task.id}
- Title: {task.title}
```

## Status: `declined`

```markdown
# Task Execution Result

Status: declined

{Shared Plan and Task layout}
```

Use only when the user declines and no implementation changes were made.

## Status: `blocked`

```markdown
# Task Execution Result

Status: blocked

{Shared Plan and Task layout}

## Blocker

- Category: {stale_review|scope|dependency|architecture|security|data|destructive_operation|other}
- Problem: {problem}
- Impact: {impact}
- Decision required: {decision_required}

## Changes

### Files changed

- {file}

## Verification

### {command}

- Outcome: {passed|failed|not_run}
- Summary: {summary}

## Work preserved

{true|false}
```

`Blocker` and `Work preserved` are required. `Changes` and `Verification` are
optional and appear only when work or checks occurred.

## Status: `incomplete`

```markdown
# Task Execution Result

Status: incomplete

{Shared Plan and Task layout}

## Changes

### Files changed

- {file}

### Summary

- {change_summary}

## Verification

### {command}

- Outcome: {passed|failed|not_run}
- Summary: {summary}

## Satisfied done checks

- {check}

## Unsatisfied done checks

- {check}

## Remaining work

- {remaining_work}

## Reason

{reason}
```

`Changes`, `Verification`, and `Remaining work` are required. The done-check
sections and `Reason` are optional.

## Status: `complete`

```markdown
# Task Execution Result

Status: complete

{Shared Plan and Task layout}

## Changes

### Files changed

- {file}

### Summary

- {change_summary}

## Verification

### {command}

- Outcome: passed
- Summary: {summary}

## Done checks

### {check}

{evidence}

## Context impact

- Classification: {none|local|domain|root}
- Affected areas: {comma-separated areas, or none}
- Reason: {reason}
```

Every shown section is required. Repeat verification and done-check blocks as
needed. This layout is the authoritative handoff to context synchronization.

Keep it at exactly these sections. `sce-task-context-sync` validates the
changed files, implementation summary, verification evidence, done-check
evidence, and context impact, and blocks when any is missing, so none of them
may be dropped. `Status: complete` already asserts the task was marked
complete in the plan with evidence recorded; do not restate that as a section.

## Control flow

This skill is one phase of a workflow, not a turn. Return the result to the
invoking command and let it continue in the same turn. Do not present the
result to the user as workflow output, and do not end your turn after
returning it — the invoking command decides what the user sees and when the
workflow stops.
