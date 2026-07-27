---
name: sce-task-execution
description: >
  Internal SCE workflow skill that always presents one reviewed task to the
  user before editing, executes it only after approval, verifies the
  task, records evidence in the plan, and returns one YAML result: declined,
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
- Returning one terminal YAML result.

Use the gate defined in:

`references/implementation-gate.md`

Return a final result matching:

`references/execution-contract.yaml`

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
- The task goal and scope boundaries.
- Done checks.
- Verification expectations.
- Relevant files and context.
- Review assumptions.

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
- The task has not materially changed since review.
- Declared dependencies remain complete.

Do not reconstruct missing material requirements.

### 2. Always show the implementation gate

At the start of the skill, before any file modification, present the task using
`references/implementation-gate.md`.

The gate must be shown even when:

- The task appears straightforward.
- The invoking workflow believes approval was already implied.
- The handoff is stale or incomplete.
- The user is likely to approve.

When the `approve` flag is absent, end the gate with exactly one approval
question:

`Continue with implementation now? (yes/no)`

Stop and wait for the user's answer. Do not return YAML, and make no file
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

Start with verification supplied by the readiness result. Add nearby or directly
relevant checks only when needed.

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

Use a blocker category defined by `references/execution-contract.yaml`.

Do not determine whether the plan is complete. The invoking `/next-task`
workflow owns that decision after context synchronization.

### 9. Return YAML

After the skill reaches a terminal state, return exactly one YAML document
matching `references/execution-contract.yaml`.

Return only the YAML document. Do not add explanatory prose before or after it.

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
- One valid terminal YAML result matching `references/execution-contract.yaml`
  was returned.
