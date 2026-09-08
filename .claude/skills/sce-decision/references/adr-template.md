# Decision: {concise decision title}

Date: {YYYY-MM-DD}
Status: {Proposed|Accepted|Rejected|Deprecated|Superseded}
Plan: `{context/plans/plan-name.md}`
Task: `{task-id or comma-separated task IDs}`
Supersedes: `{context/decisions/YYYY-MM-DD-prior-decision.md}`

Omit `Task` or `Supersedes` only when it does not apply. Do not omit `Plan`.

## Context

{Forces, constraints, and evidence that require this decision.}

## Decision

{Exactly one durable system-wide choice.}

## Rationale

{Why this choice best satisfies the constraints.}

## Alternatives considered

- **{Alternative}** — {Why it was not selected.}

## Compatibility and risks

- {Compatibility effect, migration concern, or material risk and mitigation.}

## Guardrails

- {Durable limit that keeps the decision narrow.}

## Consequences

- {Positive or negative resulting constraint.}

## Follow-up

- {Established follow-up work or condition, or `None.`}

## References

- Plan: [`{plan name}`]({relative path})
- Task: `{task ID}`
- Current-state context: [`{context file}`]({relative path})
- Evidence: [`{file or report}`]({relative path})
- Related decision: [`{decision title}`]({relative path})
