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

# Context Sync Report

Return only one completed Markdown report using the applicable variant below.
Do not include unused sections, placeholders, YAML, or a fenced code block.

The `Status` value must be exactly one of:

- `synced`
- `no_context_change`
- `blocked`

The input execution status is always `complete` and does not need to be repeated
as a separate workflow state.

## Synced variant

# Context Sync Report

**Status:** synced  
**Plan:** `{plan path}`  
**Task:** `{task id} — {task title}`

## Updated files

- {List each changed file from the execution handoff except paths under
  `context/`; state `None.` when no files remain.}

## Updated context

- `{context file}` — {concise description of the durable truth updated}

## Feature existence

- `{feature}` — `{context file that canonically describes it}`

## Verification

- {How the edited context was checked against implementation and execution evidence.}
- {File hygiene: line counts, relative links, diagrams where structure is complex.}
- {Documentation, link, or formatting checks that were run, when applicable.}

## Notes

{Include only non-blocking information worth retaining.
Omit this section when unnecessary.}

---

## No-context-change variant

# Context Sync Report

**Status:** no_context_change  
**Plan:** `{plan path}`  
**Task:** `{task id} — {task title}`

## Updated files

- {List each changed file from the execution handoff except paths under
  `context/`; state `None.` when no files remain.}

## Synchronization result

{Explain why the completed implementation did not introduce durable,
non-obvious repository knowledge requiring an update.}

## Context reviewed

- `{context file or area}` — {what was checked and why it remains accurate}

## Feature existence

- `{feature}` — `{context file that canonically describes it}`, already present.

## Verification

- {How existing context was compared with implementation and execution evidence.}

---

## Blocked variant

# Context Sync Report

**Status:** blocked  
**Plan:** `{plan path}`  
**Task:** `{task id} — {task title}`

## Updated files

- {List each changed file from the execution handoff except paths under
  `context/`; state `None.` when no files remain.}

## Blocker

**Problem:** {specific synchronization blocker}  
**Impact:** {why context cannot be made authoritative safely}  
**Required action:** {decision or correction required}

## Context changes

- {List safe context edits preserved, or state `No context files were changed.`}

## Retry condition

{State the concrete condition under which context synchronization should run
again.}

## Report rules

- Name exact context files when they were changed or reviewed.
- Under **Updated files**, list every changed file from the execution handoff
  except paths under `context/`.
- Report the missing context root as `blocked`, with `sce setup
  --bootstrap-context` as the required action and the existence of `context/` as
  the retry condition.
- Omit **Feature existence** only when the task implemented no feature.
- Describe durable truth, not implementation-session chronology.
- Keep evidence concise and factual.
- Do not claim final validation passed.
- Do not determine whether the plan is complete.
- Do not recommend a next implementation task.
