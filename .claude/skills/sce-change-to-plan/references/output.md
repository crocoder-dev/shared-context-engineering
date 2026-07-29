# Change-to-plan output layouts

Use only the applicable layout. Values come from internal workflow state.

## Missing context bootstrap gate

```markdown
-------------------------------------

# This repository has no durable context.

Bootstrap it, then continue in this session:

`sce setup --bootstrap-context`
```

## Clarification gate

```markdown
-------------------------------------

# Clarification needed.

No plan was written.

Answer each question below.  

## {question-id} · {category}

{question}

Why this blocks planning: {why_blocking}
```

## Blocked

Present each issue's problem, impact, and required decision. For ambiguity, list
candidate plan paths and explain that naming one candidate resolves it.

## Ready continuation

```markdown
-------------------------------------

# Plan {plan-name} is ready.

{total-tasks} {task|tasks} planned.

This plan is a draft. State a correction and it will be updated.

Next up:

{next-task-id} — {next-task-title}

`/next-task {plan-path} {next-task-id}`
```

For revisions, replace `is ready` with `revised`.

# SCE Plan Summary

The user-facing summary shown after a plan is written. The invoking workflow
renders it from the `plan_ready` result, immediately before the continuation
block.

This is chat output, not a file. Nothing here is written to the plan.

## Layout

```
# Plan: {plan.name}

Path: {plan.path}

## Summary:
{plan summary}

## Tasks:
1. {task.id} — {task.title}
2. {task.id} — {task.title}

## Assumptions:
- {assumption}

## Open questions:
- {open question}
```

## Field mapping

Every value comes from the `plan_ready` result. Render nothing the result does
not carry.

- `Plan:` — `plan.name`. Append ` (updated)` when `plan.action` is `updated`.
  Render nothing extra when it is `created`.
- `Path:` — `plan.path`, exactly as returned, so it stays runnable.
- `Summary:` — `summary`, as prose. This is the only place the reader learns
  what the plan actually does, so never omit it and never replace it with a
  restatement of the task titles.
- `Tasks:` — one numbered line per entry in `tasks`, in plan order. Append
  ` (done)` to any task whose `status` is `done`.
- `Assumptions:` — one line per entry in `assumptions`.
- `Open questions:` — one line per entry in `open_questions`.

## Empty sections

Never drop a section heading. An absent section reads as an oversight; an
explicit `None.` confirms nothing is pending.

When `assumptions` is empty:

```
## Assumptions:
- None.
```

When `open_questions` is absent:

```
## Open questions:
- None.
```

## Rules

- Render the sections in the order above.
- Keep task titles as authored. Do not reword, expand, or re-scope them.
- Do not restate goals, boundaries, done checks, or verification notes. The plan
  file owns task detail; this summary orients the reader.
- Do not print the raw result, and do not wrap the summary in a code fence.
- Do not add commentary, recommendations, or a next step. The continuation block
  that follows owns the handoff.

## Example

```
# Plan: red-sce-banner

Path: context/plans/red-sce-banner.md

## Summary:
Renders the ASCII-art SCE banner at the top of `sce` help in red instead of the current gradient. Colour-disabled output is unchanged, and no other help surface is affected.

## Tasks:
1. T01 — Render the SCE banner in red

## Assumptions:
- "SCE letters" refers to the ASCII-art banner in top-level help.
- Red is uniform terminal red when colors are enabled; plain ASCII remains unchanged otherwise.

## Open questions:
- None.
```
