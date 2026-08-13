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

## Architecture decisions

- `{written or reused ADR path}` — {decision and status}
- None qualified.

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

## Architecture decisions

- `{reused ADR path}` — {decision and status}
- None qualified.

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

## Context synchronization handoff

- Changed files: {list each changed file from the handoff except paths
  under `context/`; state `None.` when no files remain}
- Implementation summary: {summary}
- Verification: {commands and outcomes}
- Done checks: {status of each done check}
- Context impact: {durable context this change affects, or none}

## Context synchronization blocker

- Blocker: {specific synchronization blocker}
- Required action: {decision or correction required}
- Retry condition: {concrete condition under which context synchronization
  should run again}

## Context changes

- {List safe context edits preserved, or state `No context files were changed.`}

## Architecture decisions

- `{ADR path written or reused before the blocker}` — {decision and status}
- None written or reused before the blocker.

## Report rules

- Name exact context files when they were changed or reviewed.
- Under **Architecture decisions**, list every ADR path written or reused during
  the decision gate. In a successful report, state `None qualified.` when the
  gate skipped invocation. In a blocked report, state
  `None written or reused before the blocker.` when applicable.
- Under **Updated files** (synced and no-context-change reports) or **Changed
  files** (blocked reports), list every changed file from the execution
  handoff except paths under `context/`.
- Report the missing context root as `blocked`, with `sce setup
  --bootstrap-context` as the required action and the existence of `context/` as
  the retry condition.
- In a blocked report, write the `Context synchronization handoff` and
  `Context synchronization blocker` subsections using the same field names
  the plan's completion record uses, so plan review can persist them
  verbatim.
- Omit **Feature existence** only when the task implemented no feature.
- Describe durable truth, not implementation-session chronology.
- Keep evidence concise and factual.
- Do not claim final validation passed.
- Do not determine whether the plan is complete.
- Do not recommend a next implementation task.
