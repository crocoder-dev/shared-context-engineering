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

## Context impact

**Classification:** `{local | domain | root}`  
**Affected areas:** `{comma-separated areas}`

{Explain which durable behavior, architecture, terminology, operation, or
constraint required synchronization.}

## Updated context

- `{context file}` — {concise description of the durable truth updated}

## Root pass

- `context/overview.md` — {verified | edited | absent}
- `context/architecture.md` — {verified | edited | absent}
- `context/glossary.md` — {verified | edited | absent}
- `context/patterns.md` — {verified | edited | absent}
- `context/context-map.md` — {verified | edited | absent}

## Feature existence

- `{feature}` — `{context file that canonically describes it}`

## Verification

- {How the edited context was checked against implementation and execution evidence.}
- {File hygiene: line counts, relative links, diagrams where structure is complex.}
- {Documentation, link, or formatting checks that were run, when applicable.}

## Notes

{Include only non-blocking information the invoking workflow should retain.
Omit this section when unnecessary.}

---

## No-context-change variant

# Context Sync Report

**Status:** no_context_change  
**Plan:** `{plan path}`  
**Task:** `{task id} — {task title}`

## Context impact

**Classification:** none

{Explain why the completed implementation did not introduce durable,
non-obvious repository knowledge requiring an update.}

## Context reviewed

- `{context file or area}` — {what was checked and why it remains accurate}

## Root pass

- `context/overview.md` — {verified | absent}
- `context/architecture.md` — {verified | absent}
- `context/glossary.md` — {verified | absent}
- `context/patterns.md` — {verified | absent}
- `context/context-map.md` — {verified | absent}

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
- Report every file in the root pass, including any that is absent. A root pass
  with a file missing from the list reads as a file that was never checked.
- Report the missing context root as `blocked`, with `sce setup
  --bootstrap-context` as the required action and the existence of `context/` as
  the retry condition.
- Omit **Feature existence** only when the task implemented no feature.
- Describe durable truth, not implementation-session chronology.
- Keep evidence concise and factual.
- Do not claim final validation passed.
- Do not determine whether the plan is complete.
- Do not recommend a next implementation task.
