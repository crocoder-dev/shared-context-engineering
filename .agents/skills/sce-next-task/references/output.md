# Next-task output layouts

Use only the applicable layout. Values come from internal workflow state.

## Review blocked

Present the selected task, then each issue's problem, impact, and required
decision. If plan resolution is ambiguous, list candidate paths and
`/next-task {candidate-path}`. State whether another task remains executable.

## Plan already complete

```markdown
-------------------------------------

# Implementation tasks are complete.

Run the final validation:

`/validate {plan-path}`
```

## Declined

```markdown
You have declined to proceed with this task
```

## Execution blocked or incomplete

For `blocked`, present the blocker, work completed before it, and the required
decision or action. For `incomplete`, present completed work, verification
evidence, remaining work, and the reason it remains incomplete.

## Context synchronization blocked

State that task `{completed-task-id}` was implemented, verified, and recorded;
report the contradiction or synchronization failure, preserved edits, required
action, and retry condition. State that durable context is out of date and must
be synchronized before continuing.

## More tasks remain

```markdown
-------------------------------------

# Task {completed-task-id} completed.

{completed-tasks} of {total-tasks} tasks complete.

Next up:

{next-task-id} — {next-task-title}

`/next-task {plan-path} {next-task-id}`
```

## All tasks complete

```markdown
-------------------------------------

# Task {completed-task-id} completed.

All tasks are complete.

Run the final validation:

`/validate {plan-path}`
```

# Implementation gate

Always show this gate at the start of the **Task execution phase**, before editing any
file.

The gate is user-facing prose. It is never serialized into a YAML result. This
file is the only authority for the gate's content and order.

## Format

# `{task.id} - {task.title} - {plan.name}`

## Goal

{task.goal}

## In scope

- {task.in_scope}

## Out of scope

- {task.out_of_scope}

## Done when

- {task.done_checks}

## Expected changes

- List confirmed files or areas expected to change.
- Label uncertain entries as likely rather than confirmed.

## Approach

Describe the smallest coherent implementation approach in 2–5 steps.

## Assumptions

- Include material assumptions returned by plan review.
- Omit this section when there are no assumptions.

## Risks or trade-offs

- Include only risks relevant to approving this task.
- Omit this section when there are no meaningful risks.

## Verification

- {task.verification}

When the `approve` flag is absent, end with exactly:

`Continue with implementation now? (yes/no)`

When the `approve` flag is supplied, omit the question and end after
**Verification**.

## Rules

- Show the gate exactly once for an unchanged task.
- Do not modify files before approval.
- Do not add requirements absent from the reviewed task.
- Do not present multiple competing approaches unless a material decision is
  required.
- Do not emit YAML while waiting for the user's answer. Stop after the gate and
  wait.
- If the handoff is stale or incomplete, show the known task information and
  identify the problem under **Risks or trade-offs**.
