# Plan review phase

Run this phase for step 1 of the workflow. It resolves one plan, selects one
task, and decides whether that task can be implemented right now. It reads;
it never writes.

Inputs: the parsed `plan-name-or-path`, and `task-id` when present. The
`auto-approve` token is not passed here and has no meaning in this phase.

## 1.1 Resolve the plan

Resolve the supplied plan name or path to exactly one existing plan.

When no plan can be found, set internal status `blocked`.

When multiple plans match and none can be selected safely, set internal status
`blocked` with the matching candidates.

Read the selected plan before exploring the repository.

## 1.2 Resolve one task

Before selecting or starting a task, inspect every completed task's
`Context synchronization` field in the plan, in plan order, regardless of its
position relative to the task being selected or resumed. A missing field, or
any value other than `synced`, is unresolved synchronization debt. Never infer
`synced` from chat history.

For the first task carrying debt:

- When the task has no durable `Context synchronization handoff` subsection (a
  legacy plan predating that structure), do not attempt a reconstructed retry.
  Set internal status `blocked` with a required action to migrate the plan
  (add the handoff subsection, or resolve the debt manually) and a retry
  condition of the plan carrying that structure. Stop.
- Otherwise, set internal status `sync_debt`, naming the debt task (its ID and
  title), its persisted `Context synchronization handoff`, and — when its
  field is `blocked` — its persisted `Context synchronization blocker`. Do not
  run or cite the Task context synchronization phase. Stop. Do not select or
  start a new task.

Only after every completed task is `synced` does task selection proceed.

When a task ID is supplied, select that task only after the same synchronization-
debt check passes.

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
- `sync_debt`

Record only the internal state. Do not add explanatory prose before or after it.

A `ready` result must identify:

- One resolved plan.
- Exactly one incomplete task.
- The task goal and scope boundaries.
- Done checks.
- Verification expectations.
- Relevant files and context.
- Review assumptions.

A `sync_debt` result must identify:

- The debt-carrying task's ID and title.
- Its persisted `Context synchronization handoff`.
- Its persisted `Context synchronization blocker`, when present.

Step 2 consumes a `ready` result verbatim, so anything the execution phase
needs has to be present here.

## Plan review boundaries

Do not:

- Modify application code.
- Modify tests.
- Update the plan.
- Mark a task complete.
- Request implementation confirmation.
- Run task execution.
- Synchronize context.
- Run final validation.
- Review more than one task.
