# Handover document format

The Markdown document writer mode creates under
`context/handovers/{name}.md`. This is the persisted file's content, distinct
from the terminal response defined in `references/output.md`.

## Template

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

## Completeness contract

- The first four `##` sections shown in **Template** are required and must appear
  in that order.
- Each required section's content, up to the next `##` heading or the end of the
  file, must contain non-whitespace content. An empty list marker, unreplaced
  `{...}` placeholder, or other template scaffolding alone is invalid.
- Explicit `None identified.` statements or section-appropriate equivalents are
  real content and are valid.
- Writer mode must satisfy this contract before reporting success; loader mode
  validates the same contract before presenting a handover.

## Rules

- Include `Plan` when one plan is known. Include `Task` only when one task is
  known. Omit either field rather than guessing its value.
- Keep `Assumptions` scoped to details actually labeled as inferred elsewhere
  in the document; do not duplicate confirmed facts here.
- Describe durable state useful to a future session, not a transcript of this
  one.
