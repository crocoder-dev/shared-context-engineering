# Task execution phase

Run this phase for step 2 of the workflow. It is the only phase that writes
application code, and the only one that asks the user for anything.

Input: the complete `ready` result from the plan review phase, plus the `approve`
flag when the user pre-approved this invocation.

This phase exclusively owns:

- Presenting the implementation summary.
- Requesting implementation confirmation.
- Implementing the task.
- Running task-level verification.
- Updating the task status and evidence.

Do not present an additional implementation confirmation anywhere else.

The `approve` flag means the user pre-approved this task when invoking the
workflow. It suppresses the approval question and the wait. It never suppresses
the gate. Only the workflow entrypoint may set it, and only from an explicit
user-supplied approval token. Never infer it.

If required handoff information is absent or stale, still show the gate using
what is known, clearly identify the handoff problem, and do not edit files. After
the user responds, set internal status `blocked`.

## 2.1 Validate the handoff without editing

Confirm that:

- The readiness status is `ready`.
- Exactly one task is present.
- The plan file exists.
- The selected task is still incomplete.
- The task has not materially changed since review.
- Declared dependencies remain complete.

Do not reconstruct missing material requirements.

## 2.2 Always show the implementation gate

At the start of the phase, before any file modification, present the task using
`references/output.md`.

The gate must be shown even when:

- The task appears straightforward.
- The workflow believes approval was already implied.
- The handoff is stale or incomplete.
- The user is likely to approve.

When the `approve` flag is absent, end the gate with exactly one approval
question:

`Continue with implementation now? (yes/no)`

Stop and wait for the user's answer. Do not return internal state, and make no
file modifications, until the user has answered.

When the `approve` flag is supplied, show the gate as a summary, omit the
approval question, do not wait, and continue at step 2.4.

## 2.3 Handle the user's decision

Skip this step when the `approve` flag was supplied.

When the user rejects or cancels, do not modify files and set internal status
`declined`.

When the user does not clearly approve, do not modify files. Ask the same
approval question once more only when the response is genuinely ambiguous.
Otherwise set internal status `blocked`.

When the user approves, continue with implementation.

Treat constraints supplied with approval as part of the approved task boundary.
If those constraints materially contradict the reviewed task, set internal status
`blocked` before editing.

## 2.4 Prepare the implementation

Before editing:

- Read the relevant files supplied by plan review.
- Inspect nearby code and tests when needed.
- Identify the smallest coherent change satisfying the task.
- Follow surrounding naming, structure, error handling, and test style.
- Preserve unrelated behavior.

Do not create a second plan.

Do not broaden the reviewed task.

## 2.5 Implement one task

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

## 2.6 Verify the task

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
- Set internal status `incomplete` when a done check remains unsatisfied, or
  `blocked` when completing it requires an unapproved decision or scope
  expansion.

Never report a check as passed unless it ran successfully.

## 2.7 Update the plan

Only after successful implementation and task-level verification:

- Mark only the selected task complete.
- Record concise implementation evidence.
- Record verification commands and outcomes.
- Record material deviations or approved assumptions.
- Preserve the plan's existing structure and terminology.

Do not mark the task complete when returning `declined`, `blocked`, or
`incomplete`.

## 2.8 Determine the terminal status

Set internal status `complete` when the task was implemented, verified, and
marked complete in the plan with evidence.

Set internal status `incomplete` when in-scope work was completed but one or more
done checks remain unsatisfied.

Set internal status `declined` when the user rejected implementation.

Set internal status `blocked` for every other non-successful outcome, including:

- Missing approval.
- Stale or invalid handoff.
- Material blocker.
- A verification failure that cannot be resolved in scope.

Do not determine whether the plan is complete. The `/next-task` workflow owns
that decision after context synchronization.

## 2.9 Return internal state

After the phase reaches a terminal state, set exactly one internal state.

Record only the internal state. Do not add explanatory prose before or after it.

A `complete` result is the authoritative handoff into step 3, which reads the
plan, completed task, changed files, implementation summary, verification
evidence, done-check evidence, and context-impact classification out of it. Step
3 is forbidden from reconstructing any of that, so it has to be present here.

## Task execution boundaries

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
