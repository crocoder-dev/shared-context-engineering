# Validate output layouts

Use only the applicable layout. Values come from internal workflow state.

The `blocked` and `failed` layouts are stated once, under **Validation Result**
below.

## Context synchronization blocked

State that validation passed and its report is recorded, then report the context
failure, preserved edits, required action, and retry condition. State that durable
context remains out of date and synchronization must finish before closure.

## Completion

```markdown
-------------------------------------

# Plan {plan-name} validated.

All implementation tasks were already complete.
Final validation passed.
Durable context is synchronized.

Validation report: {plan-path}
```

# Validation Result

Return only one completed Markdown report using the applicable variant below.
Do not include unused sections, placeholders, YAML, or a fenced code block.

The `Status` value must be exactly one of:

- `validated`
- `failed`
- `blocked`

The plan-file `## Validation Report` section is written separately using the
**Plan-file validation report** section embedded in this file. This layout
carries the validation phase's result into the workflow's own branches.

## Validated variant

# Validation Report

**Status:** validated  
**Plan:** `{plan path}`  
**Name:** `{plan name}`  
**Tasks:** `{completed}/{total} complete`  
**Date:** `{YYYY-MM-DD}`

## Commands run

- `{command}` -> {passed} — {concise outcome summary}

## Acceptance criteria

- [x] AC1: {criterion statement} — {evidence}
- [x] AC2: {criterion statement} — {evidence}

## Scaffolding removed

- `{path}` — {why it was temporary}
- None.

## Residual risks

- {risk}
- None identified.

## Context impact

**Classification:** `{none | local | domain | root}`  
**Affected areas:** `{comma-separated areas}`  
**Required context:**

- `{path or statement from the plan Context sync section}`

{One or two sentences on why this classification fits the finished plan.}

## Notes

{Include only non-blocking information worth retaining.
Omit this section when unnecessary.}

---

## Failed variant

This variant is a session handoff. Another agent or a later session must be
able to act from it alone. Write it as a prompt the user can paste forward, not
as a summary of the validation run.

# Validation failed — handoff

**Status:** failed  
**Plan:** `{plan path}`  
**Name:** `{plan name}`  
**Tasks:** `{completed}/{total} complete`  
**Date:** `{YYYY-MM-DD}`  
**Validation report:** written to `{plan path}`

## Goal for the next session

Repair the unfinished validation so every acceptance criterion and full
validation command passes. Do not modify tests or product code inside a
`/validate` run to force green results; fix the implementation (or the plan) in
a normal work session, then rerun validation.

## What failed

- `{check or AC id}`: {problem}
  - Evidence: {command output, exit summary, or inspection finding}
  - Required action: {concrete repair or decision}

## Acceptance criteria

- [x] AC1: {criterion} — {evidence}
- [ ] AC2: {criterion} — {why unmet}

## Commands run

- `{command}` -> {passed | failed | not_run} — {concise outcome summary}

## Constraints

- All implementation tasks were already complete when validation ran.
- Validation did not modify tests, application code, or configuration to clear
  failures.
- Durable context was not synchronized; plan context sync runs only after
  validation succeeds.
- Prefer the plan at `{plan path}` and its Validation Report as the source of
  recorded evidence.

## Residual risks

- {risk}
- None identified.

## Recommended work

1. {First concrete fix, with files or areas when known}
2. {Second concrete fix, or decision the user must make}
3. Rerun final validation after the fixes land:

`/validate {plan path}`

Do not stop after the repair. The plan is not finished until `/validate`
returns `validated` and plan context sync completes.

---

## Blocked variant

# Validation blocked

**Status:** blocked  
**Plan:** `{plan path when resolved}`  
**Name:** `{plan name when resolved}`

## Issues

- **{issue id}** ({category}): {problem}
  - Impact: {impact}
  - Required: {decision or action}

## Incomplete tasks

- `{task id}` — {title}
- Omit this section when no incomplete tasks apply.

## Candidates

- `{candidate plan path}`
- Omit this section when plan resolution was not ambiguous.

## Next step

{Exactly one continuation, matching the blocker:}

- Incomplete tasks:

`/next-task {plan path}`

- Ambiguous plan:

`/validate {candidate path}`

- Missing plan content or other blocker: state the decision required. Do not
  invent a command.

---

## Report rules

- Name the exact `Plan:` path so every emitted command is runnable.
- Use **Status:** exactly `validated`, `failed`, or `blocked`.
- Never claim a check passed unless it ran successfully or the authorized
  inspection confirmed it.
- Do not modify tests or product code to clear a failure; record it under
  **What failed**.
- The failed variant must always end its **Recommended work** with
  `/validate {plan path}` as the final step after repairs.
- The failed variant must be self-contained enough to hand to another session
  without the original chat.
- Include **Context impact** only on `validated`. Omit it on `failed` and
  `blocked`; plan context sync is not run for non-success states.
- Do not include context synchronization results in this report. The invoking
  workflow runs the **Plan context synchronization phase** only after `validated`.
- Do not select or describe an unrelated next implementation task when status is
  `validated`.
- Omit empty optional sections rather than writing placeholders.

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
