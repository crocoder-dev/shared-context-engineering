---
name: sce-plan-review
description: >
  Internal SCE workflow skill that resolves one task from an existing plan and
  determines whether it is ready for implementation. Returns ready, blocked, or
  plan_complete with a structured payload. Use from /next-task. Do not implement
  changes, request implementation approval, update the plan, synchronize
  context, or run final validation.
---

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
- Returning one structured readiness result.

Return a result matching:

`references/readiness-contract.yaml`

## Input

The invoking workflow provides:

- A plan name or path.
- An optional task ID.

## Workflow

### 1. Resolve the plan

Resolve the supplied plan name or path to exactly one existing plan.

When no plan can be found, return `blocked`.

When multiple plans match and none can be selected safely, return `blocked` with
the matching candidates.

Read the selected plan before exploring the repository.

### 2. Resolve one task

When a task ID is supplied, select that task.

Otherwise, select the first incomplete task in plan order whose declared
dependencies are complete.

Return `plan_complete` when no incomplete tasks remain.

Return `blocked` when incomplete tasks remain but none can currently be
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

Return `blocked` when a missing decision materially affects:

- User-visible behavior.
- Public interfaces.
- Architecture or ownership boundaries.
- Data shape or persistence.
- Security or privacy.
- External dependencies.
- Destructive or difficult-to-reverse behavior.
- The evidence needed to prove completion.

### 5. Return the result

Return exactly one structured result:

- `ready`
- `blocked`
- `plan_complete`

Return only the structured result. Do not add explanatory prose before or after
it.

## Boundaries

Do not:

- Modify application code.
- Modify tests.
- Update the plan.
- Mark the task complete.
- Request implementation confirmation.
- Invoke task execution.
- Synchronize context.
- Run final validation.
- Review more than one task.

## Completion

The skill is complete after:

- One plan was resolved.
- At most one task was resolved.
- One valid readiness result matching `references/readiness-contract.yaml` was
  returned.
