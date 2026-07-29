---
name: sce-next-task
description: >
  Review, approve, implement, verify, and synchronize one SCE plan task
---

# SCE Next Task

## Purpose

Own this workflow from input parsing through its terminal user-visible response.
Execute the phases below directly and in order. Phase statuses are internal state,
not inter-skill handoffs. Do not invoke another SCE skill, sibling package, or
workflow command.

## User-visible output

Use `references/output.md` for every gate and terminal response. Render no raw
internal state. The reference contains only human-visible Markdown layouts.

## Canonical workflow


description: "Run the **Plan review phase** -> the **Task execution phase** -> the **Task context synchronization phase** for one SCE plan task"
argument-hint: "<plan-name> [T0X] [approved]"


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

Every `{plan-path}` and `{candidate-path}` emitted anywhere in this workflow is the path resolved by the **Plan review phase** (`plan.path`, or an entry of `candidates`), so every emitted command is directly runnable.

## Workflow

### 1. Review the task

Run the **Plan review phase** with the parsed `plan-name-or-path` and, when present, the parsed `task-id`.

Do not pass the `auto-approve` token to the **Plan review phase**.

The skill must return a result matching its readiness contract.

Branch on `status`:

`blocked` -> Do not run implementation. Present the result as prose. Do not print the raw result. Stop.

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

`ready` -> Pass the complete readiness result to the **Task execution phase**.

Do not reconstruct, summarize, or reinterpret the reviewed task before passing it.

### 2. Execute the task

Run the **Task execution phase** with the complete `ready` result from the **Plan review phase**.

Branch on `auto-approve`:

`approved` -> Also pass the `approve` flag. the **Task execution phase** then shows its implementation gate as a summary and proceeds without asking.

else -> Do not pass the `approve` flag. the **Task execution phase** shows its implementation gate and waits for the user's decision.

the **Task execution phase** exclusively owns:

- Presenting the implementation summary.
- Requesting implementation confirmation.
- Implementing the task.
- Running task-level verification.
- Updating the task status and evidence.

Do not present an additional implementation confirmation.

Branch on the execution result.

`declined` -> Present "You have declined to proceed with this task". Do not run context synchronization. Stop.

`blocked` -> Present:

- The blocker.
- Work completed before the blocker.
- The decision or action required.

Do not run context synchronization. Stop.

`incomplete` -> Present:

- Work completed.
- Verification evidence.
- Remaining work.
- The reason the task is incomplete.

Do not run context synchronization. Do not select another task. Stop.

`complete` -> continue to the next step.

### 3. Synchronize context

Run the **Task context synchronization phase** with the complete `complete` result returned by the **Task execution phase**.

Pass that result verbatim. It is the authoritative handoff, and the **Task context synchronization phase** owns reading the plan, task, changed files, verification evidence, and reported context impact out of it.

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

`synced` | `no_context_change` -> Print out the report the **Task context synchronization phase** returned. Continue to the next step.

### 4. Determine the continuation

Use `plan.completed_tasks` and `plan.total_tasks` from the execution result to determine which continuation applies.

Do not execute another task. Return exactly one continuation.

If incomplete tasks remain, read the plan and name the first unchecked task in plan order. Do not evaluate its dependencies; the **Plan review phase** checks them when the emitted command runs and returns `blocked` if they are unmet.

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
- Do not duplicate the internal instructions of embedded phases.
- Do not ask for implementation confirmation outside "Task execution phase".
- Do not run full-plan validation.
- Do not mark the plan complete.
- Do not execute the continuation returned at the end.
- Do not infer success when an embedded phase returns a non-success status.
- Preserve completed work and evidence when a later phase fails.

## Embedded phase behavior

## Internal phase: Plan review phase


name: Plan review phase
description: >
  Internal SCE workflow skill that resolves one task from an existing plan and
  determines whether it is ready for implementation. Returns ready, blocked, or
  plan_complete with a structured payload. Use from /next-task. Do not implement
  changes, request implementation approval, update the plan, synchronize
  context, or run final validation.


# SCE Plan Review

## Purpose

Resolve exactly one task from an SCE plan (located in `context/plans/`) and
determine whether it can enter the implementation phase without inventing
material requirements.

This skill owns:

- Resolving one plan.
- Selecting at most one task.
- Inspecting the context needed to judge readiness.
- Determining readiness.
- Recording one structured readiness result.

Return a result matching:

the internal readiness state described by this workflow

## Input

The invoking workflow provides:

- A plan name or path.
- An optional task ID.

## Workflow

### 1. Resolve the plan

Resolve the supplied plan name or path to exactly one existing plan.

When no plan can be found, set internal status `blocked`.

When multiple plans match and none can be selected safely, set internal status `blocked` with
the matching candidates.

Read the selected plan before exploring the repository.

### 2. Resolve one task

When a task ID is supplied, select that task.

Otherwise, select the first incomplete task in plan order whose declared
dependencies are complete.

Set internal status `plan_complete` when no incomplete tasks remain.

Set internal status `blocked` when incomplete tasks remain but none can currently be
executed.

Review at most one task per invocation.

### 3. Inspect relevant context

Start with the task and the files it directly references.

Inspect only what is needed to understand:

- Existing behavior.
- Applicable repository conventions.
- Architectural boundaries.
- Relevant tests.
- Available verification commands.
- Decisions or specifications connected to the task.

Load root context only when the task affects repository-wide behavior,
architecture, shared terminology, or cross-domain interfaces.

Do not explore the entire repository by default.

### 4. Determine readiness

A task is `ready` when:

- Its goal is clear.
- Its scope is sufficiently bounded.
- Its dependencies are complete.
- Its done checks are observable.
- A credible verification method exists.
- No unresolved decision would materially change the implementation.

Use repository conventions for ordinary local choices.

Do not block on:

- Naming inferable from surrounding code.
- Established formatting or style.
- Reversible local implementation details.
- Details that do not change observable behavior or scope.

Record these choices under `assumptions`.

Set internal status `blocked` when a missing decision materially affects:

- User-visible behavior.
- Public interfaces.
- Architecture or ownership boundaries.
- Data shape or persistence.
- Security or privacy.
- External dependencies.
- Destructive or difficult-to-reverse behavior.
- The evidence needed to prove completion.

### 5. Return the result

Set exactly one internal state:

- `ready`
- `blocked`
- `plan_complete`

Record only the internal state. Do not add explanatory prose before or after
it.

## Boundaries

Do not:

- Modify application code.
- Modify tests.
- Update the plan.
- Mark the task complete.
- Request implementation confirmation.
- Run task execution.
- Synchronize context.
- Run final validation.
- Review more than one task.

## Completion

The skill is complete after:

- One plan was resolved.
- At most one task was resolved.
- One valid readiness result matching the internal readiness state described by this workflow was
  returned.

## Internal phase: Task execution phase


name: Task execution phase
description: >
  Internal SCE workflow skill that always presents one reviewed task to the
  user before editing, executes it only after approval, verifies the
  task, records evidence in the plan, and returns one internal state: declined,
  blocked, incomplete, or complete. Accepts a ready result from
  Plan review phase. Do not select or execute another task,
  synchronize durable context, run final plan validation, create commits, or
  expand task scope.


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
- Recording one terminal internal state.

Use the gate defined in:

`references/output.md`

Return a final result matching:

the internal execution state described by this workflow

## Input

The invoking workflow provides:

- The complete `ready` result from the **Plan review phase**.
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
After the user responds, set internal status `blocked`.

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
`references/output.md`.

The gate must be shown even when:

- The task appears straightforward.
- The invoking workflow believes approval was already implied.
- The handoff is stale or incomplete.
- The user is likely to approve.

When the `approve` flag is absent, end the gate with exactly one approval
question:

`Continue with implementation now? (yes/no)`

Stop and wait for the user's answer. Do not return internal state, and make no file
modifications, until the user has answered.

When the `approve` flag is supplied, show the gate as a summary, omit the
approval question, do not wait, and continue at *Prepare the implementation*.

### 3. Handle the user's decision

Skip this step when the `approve` flag was supplied.

When the user rejects or cancels, do not modify files and set internal status `declined`.

When the user does not clearly approve, do not modify files. Ask the same
approval question once more only when the response is genuinely ambiguous.
Otherwise set internal status `blocked`.

When the user approves, continue with implementation.

Treat constraints supplied with approval as part of the approved task boundary.
If those constraints materially contradict the reviewed task, set internal status `blocked`
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
- Set internal status `incomplete` when a done check remains unsatisfied, or `blocked` when
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

Set internal status `complete` when the task was implemented, verified, and marked complete
in the plan with evidence.

Set internal status `incomplete` when in-scope work was completed but one or more done checks
remain unsatisfied.

Set internal status `declined` when the user rejected implementation.

Set internal status `blocked` for every other non-successful outcome, including:

- Missing approval.
- Stale or invalid handoff.
- Material blocker.
- A verification failure that cannot be resolved in scope.

Use a blocker category defined by the internal execution state described by this workflow.

Do not determine whether the plan is complete. The invoking `/next-task`
workflow owns that decision after context synchronization.

### 9. Return internal state

After the skill reaches a terminal state, set exactly one internal state
matching the internal execution state described by this workflow.

Record only the internal state. Do not add explanatory prose before or after it.

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
- One valid terminal internal state matching the internal execution state described by this workflow
  was returned.

## Internal phase: Task context synchronization phase


name: Task context synchronization phase
description: >
  Internal SCE workflow skill that accepts a successful status: complete result
  from Task execution phase, reconciles the completed implementation with durable
  repository context, and returns a Markdown synchronization report. Run only
  after one task has been implemented and verified successfully. Do not implement
  application code, change plan state, determine whether the plan is complete,
  run final validation, or select another task.


# SCE Task Context Sync

## Purpose

Reconcile one completed task with the repository's durable context and return a
Markdown report.

This skill owns:

- Validating the execution handoff.
- Confirming the context root exists.
- Discovering the context affected by one completed task.
- Deciding whether durable context changed.
- Editing and verifying the affected context files.
- Recording one Markdown synchronization report.

Use the report format in:

`references/output.md`

## Input

The invoking workflow provides:

- The complete result returned by the **Task execution phase**.

The execution result must have:

```text
status: complete
```

Treat the execution result as the authoritative handoff for:

- The resolved plan and completed task.
- Files changed by implementation.
- Implementation summary.
- Verification evidence.
- Done-check evidence.
- Reported context impact.

This skill must not be run for `declined`, `blocked`, or `incomplete`
execution results.

Do not reconstruct a missing execution result from conversation history.

## Workflow

### 1. Validate the execution handoff

Confirm that:

- `status` is exactly `complete`.
- A `plan` object with a `path` is present.
- Exactly one completed task is identified.
- Changed files and an implementation summary are present.
- Verification evidence is present.
- Done-check evidence is present.
- A context-impact classification is present.

If the handoff is missing required information or is internally contradictory,
do not modify context. Return a `blocked` Markdown report.

### 2. Confirm the context root

When `context/` does not exist, there is no durable memory to synchronize.
Do not create it, and do not write context files outside it.

Return a `blocked` report whose required action is:

`sce setup --bootstrap-context`

State that the task itself is complete and recorded in the plan, and that
synchronization should run again once the context root exists.

Bootstrapping is the user's action, not this skill's.

### 3. Discover applicable context

Start with the execution result:

- `context_impact.classification`
- `context_impact.affected_areas`
- Changed files.
- Implementation summary.
- Done-check evidence.

Then inspect existing repository context in this order when present:

1. `context/context-map.md`
2. Context files for the affected domain or subsystem
3. `context/overview.md`
4. `context/architecture.md`
5. `context/glossary.md`
6. `context/patterns.md`
7. Operational, product, or decision records directly related to the change

Use the context map and existing links to locate authoritative files.

Do not scan or rewrite the entire `context/` tree by default.

Do not create a new context file when an existing authoritative file can be
updated coherently.

#### The mandatory root pass

Every invocation verifies these five files against code truth, whatever the
reported classification is:

- `context/overview.md`
- `context/architecture.md`
- `context/glossary.md`
- `context/patterns.md`
- `context/context-map.md`

Verifying is not editing. A classification that warrants no root edit still
requires reading each of these and confirming it is not contradicted by the
completed implementation. A file that is absent is a gap; record it in the
report rather than creating it to satisfy the pass.

Report each of the five as verified or edited. Never declare synchronization
done while one of them is unchecked.

Do not create a new context file when an existing authoritative file can be
updated coherently.

### 4. Determine whether durable context changed

Use the reported context impact as a strong hint, then verify it against the
implementation and existing context.

Durable context includes non-obvious repository knowledge such as:

- User-visible or externally observable behavior.
- Architecture, boundaries, ownership, and dependency direction.
- Public interfaces, data contracts, and persistence behavior.
- Operational procedures and important failure modes.
- Security or privacy behavior.
- Shared terminology.
- Intentional limitations and meaningful design decisions.

Do not document:

- Details already obvious from the implementation.
- Temporary debugging information.
- A file-by-file narration of the change.
- Test output that belongs only in task evidence.
- Speculation or future work not established by the completed implementation.
- Generic engineering practices.

Interpret impact classifications as follows. Each governs which files are
*edited*; none of them waives the mandatory root pass.

- `none`: Make no edits beyond any correction the root pass turns up.
- `local`: Update the nearest existing authoritative context only when the new
  behavior is not reliably discoverable from code.
- `domain`: Update affected domain context and the context map when its links or
  summaries changed.
- `root`: Update the relevant root context and any affected domain context.

A change is `root` when it introduces cross-cutting behavior, repository-wide
policy or contracts, an architecture or ownership boundary, or a change to
canonical terminology. A change confined to one feature or domain, with no
repository-wide behavior, architecture, or terminology impact, is `domain` or
`local`: capture its detail in domain files and leave the root files unedited.

If the reported classification is inconsistent with the actual change, use the
verified classification and explain the difference in the report.

### 5. Synchronize context

Make the smallest coherent documentation change that preserves repository truth.

When editing context:

- Describe the resulting behavior, not the implementation session.
- Preserve repository terminology and document structure.
- Remove or correct statements contradicted by the completed implementation.
- Update cross-references when files are added, moved, renamed, or superseded.
- Keep one authoritative statement for each durable fact.
- Avoid copying the execution result verbatim into context files.
- Do not change application code, tests, or plan state.

Create a new context file only when:

- The knowledge is durable and non-obvious.
- No existing file owns it coherently.
- The new file has a clear place in the context map.

#### Feature existence

Every feature the completed task implemented must have at least one durable
canonical description discoverable from `context/`, in a domain file under
`context/{domain}/` or in `context/overview.md` for a cross-cutting feature.

When the task implemented a feature no context file describes, add that
description. A feature that fits no existing domain file gets a new focused
file; do not defer it to a later task. Prefer a small, precise domain file over
overloading `overview.md` with detail.

This is the one case where documentation is warranted by the change itself
rather than by a gap in durable knowledge. It is not license to narrate the
diff: describe what the feature is and how it behaves, not what was edited.

#### Glossary

Add a `context/glossary.md` entry for any domain language the task introduced.
New terminology is durable knowledge whatever the classification is: a `domain`
change that names a new concept still earns its glossary entry.

#### File hygiene

Every context file this skill writes must satisfy:

- One topic per file.
- At most 250 lines. When an edit would push a file past 250 lines, split it
  into focused files and link them rather than letting it grow.
- Relative paths in every link to another context file.
- A Mermaid diagram where structure, boundaries, or flows are complex enough
  that prose alone would not carry them.
- Concrete code examples only where they clarify non-trivial behavior.

When detail outgrows a shared file, migrate it into `context/{domain}/`, leave a
concise pointer behind, and link the new file from `context/context-map.md`.

### 6. Verify synchronization

After edits, verify:

- Every changed context file accurately reflects the completed implementation.
- No edited statement contradicts the code, plan, or execution evidence.
- Every file in the mandatory root pass was read and confirmed against code
  truth, whether or not it was edited.
- Each feature implemented by the task has a durable canonical description
  reachable from `context/`.
- Every changed file is at or below 250 lines, covers one topic, and links other
  context files by relative path.
- Diagrams are present where structure, boundaries, or flows are complex.
- Links and referenced paths resolve when practical to check.
- New context files are reachable from the context map or another authoritative
  index.
- Root context remains concise and delegates details to domain files.
- Unrelated context was not changed.

Use focused documentation, link, or formatting checks when available.

Do not run full application or plan validation.

If synchronization cannot be completed without inventing facts or resolving a
material contradiction, preserve safe edits when appropriate and return a
`blocked` report.

### 7. Return the Markdown report

Set exactly one report status:

- `synced`
- `no_context_change`
- `blocked`

`synced` means context files were updated and verified. `no_context_change`
means existing context was checked and no edit was warranted. `blocked` means
context could not be synchronized safely.

Record only the Markdown report. Do not add explanatory prose before or after
it.

Do not determine whether the plan is complete. The invoking `/next-task`
workflow owns that decision after context synchronization.

## Boundaries

Do not:

- Accept an execution result whose status is not `complete`.
- Implement or modify application code.
- Modify tests.
- Change task completion status or plan evidence.
- Determine whether the plan is complete.
- Select or execute another task.
- Run full-plan validation.
- Mark the plan validated, closed, or archived.
- Create a Git commit or push changes.
- Create the context root. `sce setup --bootstrap-context` owns that.
- Narrate changed files as documentation. Feature existence is the only reason
  to document a change that introduced no other durable knowledge.
- Delete a context file that has uncommitted changes.
- Return an execution-style internal state.

## Completion

The skill is complete after:

- The context root was confirmed, or a `blocked` report named
  `sce setup --bootstrap-context` as the required action.
- The mandatory root pass was run.
- Applicable durable context was synchronized and verified, no context change
  was warranted, or a synchronization blocker was reported.
- One Markdown report matching `references/output.md` was returned.



## Composite control flow

Keep phase results as internal state and continue immediately whenever the
canonical workflow says to continue. Stop only at a user wait or terminal branch.
Approval, clarification, revision, failed-validation repair, and bootstrap waits
resume this same skill in the same session. Never expose an internal phase result
as the workflow's final response.
