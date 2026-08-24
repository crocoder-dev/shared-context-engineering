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

If required handoff information is absent, stale, or contradictory, still show the
gate using what is known, clearly identify the handoff problem, and do not edit
files. With the `approve` flag supplied, do not treat pre-approval as permission
to repair or reinterpret the handoff: after showing the gate, set internal status
`blocked` deterministically. Without the flag, wait for the user's response and
then set internal status `blocked`; do not retry the handoff in the same phase.

A successful `complete` handoff must explicitly contain all of these fields:

- The resolved `plan` object, including its path and completion counts.
- The selected `task` identity, including its ID and title.
- `changes.files_changed`, the implementation's baseline-relative changed-file list.
- `changes.summary`, a concise implementation summary.
- `verification`, with every reported outcome marked `passed` and its evidence.
- `done_checks`, pairing every done check with evidence.
- `plan_update`, proving the selected task was marked complete and evidence recorded.
- `context_impact`, including classification, affected areas, and reason.

Do not omit, invent, or reconstruct any of these fields when handing off to context
synchronization.

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

Before editing, capture a Git baseline. Record the current `HEAD` commit, the
staged and unstaged patch/content state, and every untracked path/content state
using equivalent `git status`, `git diff`, and `git diff --cached` views. If the
baseline cannot be captured reliably, stop before editing and set internal status
`blocked`.

After implementation, capture the same views again. Compute
`changes.files_changed` by comparing the post-edit snapshot with the pre-edit
baseline, not by listing the whole working tree or by diffing only against
`HEAD`. Include each path whose state or content changed during this task once;
exclude paths unchanged from the baseline, including unrelated pre-existing
staged, unstaged, and untracked changes. A path already dirty at baseline is
included only when this task changed its state or content.

Then:

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
- Record directly on the completed task: `Completed` (the date), the
  baseline-relative `Files changed` list, a concise factual `Result`, the
  actual outcome of every planned `Verify` check, and `Context impact`.
- Set that task's `Context synchronization` field to `pending` in the plan file
  before returning `complete`; this write must happen after the execution
  facts above and before the synchronization phase is invoked.
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

Before determining terminal status for a `complete` result, verify that the
handoff contains the resolved plan, task identity, baseline-relative changed
files, implementation summary, verification evidence, done-check evidence, plan
update, and context-impact classification listed above. The mandatory five-root-
file context pass remains required for every completed task, regardless of the
reported context-impact classification, because it is cheap, deterministic, and
load-bearing for context accuracy; `context_impact` must not be used to waive it.

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
