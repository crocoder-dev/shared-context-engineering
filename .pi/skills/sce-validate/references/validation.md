# SCE Validation

## Purpose

Prove that one finished SCE plan meets its acceptance criteria and repository
validation bar, then record the evidence on the plan and return one Markdown
result.

This phase owns:

- Resolving one plan.
- Confirming every implementation task is complete.
- Running the plan's full validation commands and each acceptance criterion
  check.
- Writing the Validation Report into the plan.
- Marking acceptance criteria against the evidence.
- Returning one Markdown validation result.

Return a result matching:

the **Validation Result** section below in this file

Write plan-file evidence matching:

`references/validation-report.md`

## Input

A plan name or path.

## Workflow

### 1. Resolve the plan

Resolve the supplied plan name or path to exactly one existing plan under
`context/plans/`.

When no plan can be found, return `blocked`.

When multiple plans match and none can be selected safely, return `blocked`
with the matching candidates.

Read the selected plan before exploring the repository.

### 2. Confirm implementation is finished

Return `blocked` with incomplete tasks listed when any implementation task
remains incomplete.

Final validation measures finished work. Do not run the full suite against a
partial stack, and do not complete remaining tasks here.

### 3. Read the validation contract from the plan

From the plan, collect:

- Every acceptance criterion and its `Validate:` check.
- The `Full validation` command list.

Return `blocked` when the plan has no usable acceptance criteria, or when no
validation commands can be determined from the plan or repository conventions.

Prefer the plan's authored checks. Fall back to repository-primary test, lint,
and format commands only when `Full validation` is absent, and record that
fallback under notes on a `validated` or `failed` result.

### 4. Run full validation and acceptance checks

Run the plan's `Full validation` commands.

Then verify each acceptance criterion using its `Validate:` line. Prefer a
runnable command. Use a named inspection only when the criterion authorizes it,
and say exactly what was inspected.

Treat leftover debug-only flags, temporary files, intermediate artifacts, or
local scaffolding as a failed validation check. Record the path and evidence
under **Failed checks and follow-ups**; never delete or repair it during
validation.

When a check fails, record the failure and continue gathering evidence. Do not
modify tests, application code, or configuration to make a check pass. Final
validation measures the finished work; repair belongs to a later work session,
not this skill.

Never report a check as passed unless it ran successfully or the authorized
inspection confirmed the criterion.

Do not run task-by-task implementation work for incomplete tasks. That belongs
to `/next-task`.

### 5. Update the plan

For `validated` and `failed` outcomes:

- Mark each acceptance criterion checkbox to match the evidence.
- Append or replace the plan's `## Validation Report` section using
  `references/validation-report.md`.
- When status is `failed`, the plan-file report must include the retry command
  `/validate {plan path}`.

Do not reopen completed tasks, rewrite task evidence, or change the task stack.

For `blocked`, leave the plan file unchanged.

### 6. Return the Markdown result

Return exactly one Markdown result:

- `validated` when every acceptance criterion is met, required full validation
  passed, and the Validation Report was written.
- `failed` when evidence was captured but required checks or criteria remain
  unsatisfied. Shape it as a session handoff per
  the **Validation Result** section below in this file, ending recommended work with
  `/validate {plan path}`.
- `blocked` when validation cannot proceed safely.

Return only the Markdown report. Do not add explanatory prose before or after
it. Do not return YAML.

## Validation boundaries

Do not:

- Validate more than one plan.
- Complete remaining implementation tasks.
- Modify tests, application code, or configuration to make a failing check pass.
- Apply lint or format auto-fixes that change product or test files as part of
  making validation green.
- Synchronize durable context under `context/`.
- Create the context root.
- Mark the plan archived or delete the plan.
- Create a Git commit or push changes.
- Invent acceptance criteria the plan does not state.
- Claim verification that was not performed.
- Return a YAML result.

## Completion

The phase is complete after:

- One plan was resolved, or resolution failed and was reported.
- Implementation completeness was checked.
- Validation ran to a terminal state, or a blocker prevented it.
- One valid Markdown result matching the **Validation Result** section below in this file was
  returned.



# Validation Result

Return only one completed Markdown report using the applicable variant below.
Do not include unused sections, placeholders, YAML, or a fenced code block.

The `Status` value must be exactly one of:

- `validated`
- `failed`
- `blocked`

The plan-file `## Validation Report` section is written separately using
`references/validation-report.md`. This file is the skill's return value to the
invoking workflow.

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

## Residual risks

- {risk}
- None identified.

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
- Validation does not synchronize durable context.
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
returns `validated`.

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
- Do not include durable context synchronization results in this report.
- Do not select or describe an unrelated next implementation task when status is
  `validated`.
- Omit empty optional sections rather than writing placeholders.
