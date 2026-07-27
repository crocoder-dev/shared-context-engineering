# Validation Result

Return only one completed Markdown report using the applicable variant below.
Do not include unused sections, placeholders, YAML, or a fenced code block.

The `Status` value must be exactly one of:

- `validated`
- `failed`
- `blocked`

The plan-file `## Validation Report` section is written separately using
`validation-report.md`. This file is the skill's return value to the invoking
workflow.

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

{Include only non-blocking information the invoking workflow should retain.
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
  `blocked`; plan context sync is not invoked for non-success states.
- Do not include context synchronization results in this report. The invoking
  workflow runs `sce-plan-context-sync` only after `validated`.
- Do not select or describe an unrelated next implementation task when status is
  `validated`.
- Omit empty optional sections rather than writing placeholders.
