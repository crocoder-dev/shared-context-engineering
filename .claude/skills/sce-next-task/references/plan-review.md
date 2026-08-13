# Plan review phase

Run this phase for step 1 of the workflow. It resolves one plan, selects one
task, and decides whether that task can be implemented right now. It reads; it
never writes.

Inputs: the parsed `plan-name-or-path`, and `task-id` when present. The
`auto-approve` token is not passed here and has no meaning in this phase.

## 1.1 Resolve the plan

Resolve the supplied plan name or path to exactly one existing plan.

When no plan can be found, set internal status `blocked`.

When multiple plans match and none can be selected safely, set internal status
`blocked` with the matching candidates.

Read the selected plan before exploring the repository.

## 1.2 Resolve one task

When a task ID is supplied, select that task.

Otherwise, select the first incomplete task in plan order whose declared
dependencies are complete.

Set internal status `plan_complete` when no incomplete tasks remain.

Set internal status `blocked` when incomplete tasks remain but none can currently
be executed.

Review at most one task per invocation.

## 1.3 Inspect relevant context

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

## 1.4 Determine readiness

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

## 1.5 Return the result

Set exactly one internal state:

- `ready`
- `blocked`
- `plan_complete`

Record only the internal state. Do not add explanatory prose before or after it.

A `ready` result must identify:

- One resolved plan.
- Exactly one incomplete task.
- The task goal and scope boundaries.
- Done checks.
- Verification expectations.
- Relevant files and context.
- Review assumptions.

Step 2 consumes this result verbatim, so anything the execution phase needs has
to be present here.

## Plan review boundaries

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
