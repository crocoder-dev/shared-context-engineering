# Implementation gate

Always show this gate at the start of `sce-task-execution`, before editing any
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
- Do not emit terminal YAML while waiting for the user's answer. When control
  must return to the invoking workflow before the user answers, return the
  `awaiting_confirmation` result and do not restate the gate inside it.
- If the handoff is stale or incomplete, show the known task information and
  identify the problem under **Risks or trade-offs**.
