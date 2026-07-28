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

## Plan update

- Task marked complete: true
- Evidence recorded: true

## Context impact

- Classification: {none|local|domain|root}
- Affected areas: {comma-separated areas, or none}
- Reason: {reason}
```

Every shown section is required. Repeat verification and done-check blocks as
needed. This layout is the authoritative handoff to context synchronization.
