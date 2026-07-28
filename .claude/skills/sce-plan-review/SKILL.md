---
name: sce-plan-review
description: >
  Internal SCE workflow skill that resolves one task from an existing plan and
  determines whether it is ready for implementation. Returns ready, blocked, or
  plan_complete with a structured payload. Use from /next-task. Do not implement
  changes, request implementation approval, update the plan, synchronize
  context, or run final validation.
compatibility: claude
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

the **Result contract** section in this file

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
- One valid readiness result matching the **Result contract** section in this file was
  returned.

## Result contract

# SCE Plan Review Result Contract

Return exactly one Markdown document using one layout below. `Status` is the
branch value consumed by `/next-task`. Use every required heading and label
exactly as written, omit optional sections that do not apply, and do not add
prose outside the selected layout. Empty required lists must contain
`- None.`.

Report task counts as they stand and the plan path exactly as resolved. Do
not request implementation confirmation or include implementation,
synchronization, or final-validation results.

## Status: `ready`

```markdown
# Plan Review Result

Status: ready

## Plan

- Path: {plan.path}
- Name: {plan.name}
- Completed tasks: {plan.completed_tasks}
- Total tasks: {plan.total_tasks}

## Task

- ID: {task.id}
- Title: {task.title}

## Relevant files

- {relevant_file}

## Relevant context

- {relevant_context}

## Assumptions

- {assumption}
```

Every section is required. `Name` is the plan basename without its extension.
Repeat list items as needed.

This layout is a compact handoff, not a task presentation. It carries only
what review discovered and the next phase cannot recover on its own. The
task's goal, scope boundaries, done checks, dependencies, and verification
stay in the plan file, where `sce-task-execution` reads them. Do not restate
them here.

## Status: `blocked`

```markdown
# Plan Review Result

Status: blocked

## Plan

- Path: {plan.path}
- Completed tasks: {plan.completed_tasks}
- Total tasks: {plan.total_tasks}

## Task

- ID: {task.id}
- Title: {task.title}

## Candidates

- {candidate_path}

## Issues

### {issue.id}

- Category: {missing_decision|ambiguity|missing_acceptance_criteria|dependency|scope}
- Problem: {problem}
- Impact: {impact}
- Decision required: {decision_required}

## Executable tasks remaining

{true|false}
```

`Issues` is required. Include `Plan` whenever exactly one plan resolved,
`Task` when one was selected, and `Candidates` only when plan resolution
failed or was ambiguous. Include `Executable tasks remaining` when a plan
resolved.

## Status: `plan_complete`

```markdown
# Plan Review Result

Status: plan_complete

## Plan

- Path: {plan.path}
- Completed tasks: {plan.completed_tasks}
- Total tasks: {plan.total_tasks}
```

## Control flow

This skill is one phase of a workflow, not a turn. Return the result to the
invoking command and let it continue in the same turn. Do not present the
result to the user as workflow output, and do not end your turn after
returning it — the invoking command decides what the user sees and when the
workflow stops.
