# SCE Plan Authoring Result Contract

Return exactly one Markdown document using one layout below. `Status` is the
branch value consumed by the invoking command. Use every required heading and
label exactly as written, omit optional sections that do not apply, and do
not add prose outside the selected layout. Empty required lists must contain
`- None.`.

Report plan names without extensions and paths exactly as written so emitted
commands are runnable. Only `plan_ready` writes a plan. Do not include
implementation, synchronization, or final-validation results.

## Status: `plan_ready`

Use after creating or updating a plan with at least one incomplete task.

```markdown
# Plan Authoring Result

Status: plan_ready

## Plan

- Path: {plan.path}
- Name: {plan.name}
- Action: {created|updated}
- Completed tasks: {plan.completed_tasks}
- Total tasks: {plan.total_tasks}

## Summary

{summary}

## Tasks

- {task.id} — {task.title} — {todo|done}

## Next task

- ID: {next_task.id}
- Title: {next_task.title}

## Assumptions

- {assumption}

## Open questions

- {open_question}
```

`Plan`, `Summary`, `Tasks`, `Next task`, and `Assumptions` are required.
List tasks in plan order, including completed tasks. `Next task` is the first
unchecked task. Include `Open questions` only for genuine non-blocking
questions. Summary describes resulting behavior rather than repeating tasks.

## Status: `needs_clarification`

Use when one to three critical questions block writing the plan.

```markdown
# Plan Authoring Result

Status: needs_clarification

## Plan target

- Name: {plan_target.name}
- Action: {created|updated}
- Path: {existing plan_target.path; omit this label when no plan exists}

## Questions

### {question.id}

- Category: {scope|success_criteria|constraints|dependency|domain|architecture|sequencing}
- Question: {question}
- Why blocking: {why_blocking}
```

`Questions` is required. `Plan target` is optional and appears only when the
request resolved to one target before authoring stopped. Never report a path
for a plan that does not exist.

## Status: `blocked`

Use when the target cannot be resolved or the request cannot be safely
planned. Nothing is written.

```markdown
# Plan Authoring Result

Status: blocked

## Candidates

- {candidate_path}

## Issues

### {issue.id}

- Category: {ambiguous_plan_target|missing_request|conflicting_request|no_actionable_work|other}
- Problem: {problem}
- Impact: {impact}
- Decision required: {decision_required}
```

`Issues` is required. Include `Candidates` only for an ambiguous existing-plan
match. Use `needs_clarification` when an answer would make the request
plannable; use `no_actionable_work` when no incomplete task would result.
