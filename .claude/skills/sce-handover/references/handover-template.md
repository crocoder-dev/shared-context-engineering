The Markdown document writer mode creates under
`context/handovers/{name}.md`. This is the persisted file's content, distinct
from the terminal response defined in `references/output.md`.

### Layout

```markdown
# Handover: {plan name or short session topic}

Date: {YYYY-MM-DD}
Plan: `{context/plans/plan-name.md}` (omit when no plan applies)
Task: `{task-id}` (omit when no single task applies)

## Current Task State

{What is being worked on, what is complete, what is in progress. Cite files,
commands, or plan/task references where they ground the statement.}

## Decisions Made

- {Decision and its rationale, or `None made this session.`}

## Open Questions / Blockers

- {Unresolved question or blocker, or `None identified.`}

## Next Recommended Step

{The single most useful next action for the following session, concrete
enough to act on directly.}

## Assumptions

- {Any detail above that was inferred rather than directly evidenced, or
  `None.`}
```

### Rules

- Include `Plan` and `Task` only when the session was working one identifiable
  plan task; omit them rather than guessing.
- Every one of the four required sections must appear, in this order, even
  when its content is `None identified.` or an equivalent.
- Keep `Assumptions` scoped to details actually labeled as inferred elsewhere
  in the document; do not duplicate confirmed facts here.
- Describe durable state useful to a future session, not a transcript of this
  one.
