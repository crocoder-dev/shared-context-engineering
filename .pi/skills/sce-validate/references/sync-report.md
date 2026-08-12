# Plan Context Sync Report

Return only one completed Markdown report using the applicable variant below.
Do not include unused sections, placeholders, YAML, or a fenced code block.

The `Status` value must be exactly one of:

- `synced`
- `no_context_change`
- `blocked`

The input validation status is always `validated` and does not need to be
repeated as a separate workflow state. This report is not produced for
`failed` or `blocked` validation results.

## Synced variant

# Plan Context Sync Report

**Status:** synced  
**Plan:** `{plan path}`

## Context impact

**Classification:** `{local | domain | root}`  
**Affected areas:** `{comma-separated areas}`

{Explain which durable behavior, architecture, terminology, operation, or
constraint required plan-level synchronization after validation.}

## Plan context requirements

- `{required context path or statement from the plan}` — {met by edit | already accurate}

## Updated context

- `{context file}` — {concise description of the durable truth updated}

## Architecture decisions

- `{written or reused ADR path}` — {decision and status}
- None qualified.

## Root pass

- `context/overview.md` — {verified | edited | absent}
- `context/architecture.md` — {verified | edited | absent}
- `context/glossary.md` — {verified | edited | absent}
- `context/patterns.md` — {verified | edited | absent}
- `context/context-map.md` — {verified | edited | absent}

## Feature existence

- `{feature}` — `{context file that canonically describes it}`

## Verification

- {How the edited context was checked against the finished implementation and validation evidence.}
- {File hygiene: line counts, relative links, diagrams where structure is complex.}
- {Documentation, link, or formatting checks that were run, when applicable.}

## Notes

{Include only non-blocking information worth retaining.
Omit this section when unnecessary.}

---

## No-context-change variant

# Plan Context Sync Report

**Status:** no_context_change  
**Plan:** `{plan path}`

## Context impact

**Classification:** none

{Explain why the finished plan introduced no durable, non-obvious repository
knowledge requiring an update, or why existing context already matched.}

## Plan context requirements

- `{required context path or statement from the plan}` — already accurate
- None listed by the plan.

## Context reviewed

- `{context file or area}` — {what was checked and why it remains accurate}

## Architecture decisions

- `{reused ADR path}` — {decision and status}
- None qualified.

## Root pass

- `context/overview.md` — {verified | absent}
- `context/architecture.md` — {verified | absent}
- `context/glossary.md` — {verified | absent}
- `context/patterns.md` — {verified | absent}
- `context/context-map.md` — {verified | absent}

## Feature existence

- `{feature}` — `{context file that canonically describes it}`, already present.

## Verification

- {How existing context was compared with the finished implementation and validation evidence.}

---

## Blocked variant

# Plan Context Sync Report

**Status:** blocked  
**Plan:** `{plan path}`

## Blocker

**Problem:** {specific synchronization blocker}  
**Impact:** {why context cannot be made authoritative safely}  
**Required action:** {decision or correction required}

## Context changes

- {List safe context edits preserved, or state `No context files were changed.`}

## Architecture decisions

- `{ADR path written or reused before the blocker}` — {decision and status}
- None written or reused before the blocker.

## Retry condition

{State the concrete condition under which plan context synchronization should
run again.}

## Report rules

- Name exact context files when they were changed or reviewed.
- Under **Architecture decisions**, list every ADR path written or reused during
  the decision gate. In a successful report, state `None qualified.` when the
  gate skipped invocation. In a blocked report, state
  `None written or reused before the blocker.` when applicable.
- Report every file in the root pass, including any that is absent.
- Report the missing context root as `blocked`, with `sce setup
  --bootstrap-context` as the required action and the existence of `context/` as
  the retry condition.
- Cover every path or statement listed in the plan's `Context sync` section
  under **Plan context requirements**.
- Omit **Feature existence** only when the plan implemented no feature.
- Describe durable truth, not validation-session chronology.
- Keep evidence concise and factual.
- Do not claim implementation tasks remain open.
- Do not reopen validation checks.
- Do not recommend a next implementation task unless context cannot be repaired
  without one, and then only as the required action.
