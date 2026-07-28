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

### Goal

{task.goal}

### In scope

- {task.in_scope}

### Out of scope

- {task.out_of_scope}

### Done checks

- {task.done_checks}

### Dependencies

- {dependency.id} — complete

### Verification

- {task.verification}

## Relevant files

- {relevant_file}

## Relevant context

- {relevant_context}

## Assumptions

- {assumption}
```

Every section is required. `Name` is the plan basename without its extension.
Repeat list items as needed.

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
