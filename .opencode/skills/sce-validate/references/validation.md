# Validation phase

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
- Returning one Markdown validation report.

Return a result matching:

the **Validation report** section below in this file

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

Read plan structure only outside fenced code blocks; headings, definitions, and
fields inside a fenced example are not plan structure. A definition counts only
in its own section: `D` entries under `## Design decisions`, `AC` entries under
`## Acceptance criteria`, `S` entries under `### Verification scenarios`, and
`T` entries under `## Task stack`.

Also collect:

- Every design decision.
- Every acceptance criterion's `Decisions:` and `Scenarios:` fields.
- Every verification scenario with its `Then:` and `Check:`.
- Every task's `Covers:` field.

Return `blocked` when the plan has no usable acceptance criteria, or when no
validation commands can be determined from the plan or repository conventions.

Prefer the plan's authored checks. Fall back to repository-primary test, lint,
and format commands only when `Full validation` is absent, and record that
fallback under notes on a `validated` or `failed` result.

### 4. Check plan references

Run `sce plan check {plan-path}` before scenario
verification. Record each reported diagnostic as a failed check under
**Failed checks and follow-ups**, quoting its line and message.

Trust the checker only when it actually ran: a result counts only on exit `0`
with `structured plan: reference integrity holds` on stdout, or on exit `3`
with diagnostics. Any other outcome means the checker is unavailable; record
that fallback under notes and apply the manual pass below instead.

For the manual pass, apply every entry of the plan template's
`Reference rules` to the plan. Each of these is a relationship-integrity
defect:

- An ID defined more than once.
- A dangling reference: a `Decisions:`, `Scenarios:`, or `Covers:` ID with no
  definition in the plan.
- An acceptance criterion without `Scenarios:`.
- An acceptance criterion that no task's `Covers:` names.
- A task without `Covers:` or with an empty `Covers:`.
- A scenario that no acceptance criterion's `Scenarios:` references.

Record each defect as a failed check under **Failed checks and follow-ups**,
naming the IDs involved. A relationship-integrity defect makes the result
`failed`; never tolerate one silently. Continue gathering the remaining
evidence.

### 5. Run full validation and acceptance checks

Run the plan's `Full validation` commands.

Then verify each acceptance criterion using its `Validate:` line. Prefer a
runnable command. Use a named inspection only when the criterion authorizes it,
and say exactly what was inspected.

Run or perform every scenario's `Check:` yourself in
this run, and compare the observed result with the scenario's `Then:`. Give
each scenario exactly one status:

- `Verified` when this run's evidence proves its `Then:` in full.
- `Partially verified` when the check ran but proves `Then:` only in part, for
  example at a narrower level than `Then:` requires.
- `Not verified` when the check failed, could not be run or located, or was
  not performed.

Only evidence gathered by this run counts. Task completion, a task's recorded
`Verify` outcomes, a planned `Check:` line, and the plan's own claims are never
evidence. Record every `Partially verified` or `Not verified` scenario as a
failed check with the gap and a required action.

An acceptance criterion is met only when its `Validate:`
check passes and every scenario its `Scenarios:` names is `Verified`. An
acceptance criterion marked `none (inspection)` is met by its `Validate:` check
alone.

Treat leftover debug-only flags, temporary files, intermediate artifacts, or
local scaffolding as a failed validation check. Record the path and evidence
under **Failed checks and follow-ups**; never delete or repair it during
validation.

When a check fails, record the failure and continue gathering evidence. Do not
modify tests, application code, or configuration to make a check pass. Final
validation measures the finished work; repair belongs to a normal work session outside this phase.

Never report a check as passed unless it ran successfully or the authorized
inspection confirmed the criterion.

Do not run task-by-task implementation work for incomplete tasks. That belongs
to `/next-task`.

### 6. Update the plan

For `validated` and `failed` outcomes:

- Mark each acceptance criterion checkbox to match the evidence.
- Append or replace the plan's `## Validation Report` section using
  `references/validation-report.md`.
- When status is `failed`, the plan-file report must include the retry command
  `/validate {plan path}`.

Do not reopen completed tasks, rewrite task evidence, or change the task stack.

For `blocked`, leave the plan file unchanged.

### 7. Return the Markdown report

Return exactly one Markdown report:

- `validated` when every acceptance criterion is met, required full validation
  passed, and the Validation Report was written.
- `failed` when evidence was captured but required checks or criteria remain
  unsatisfied. Shape it as a session handoff per
  the **Validation report** section below in this file, ending recommended work with
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
- One valid Markdown report matching the **Validation report** section below in this file was
  returned.



# Validation report

Return only one completed Markdown report using the applicable variant below.
Do not include unused sections, placeholders, YAML, or a fenced code block.

The `Status` value must be exactly one of:

- `validated`
- `failed`
- `blocked`

The plan-file `## Validation Report` section is written separately using
`references/validation-report.md`. This file is the phase's return value to the
invoking workflow.

## Validated report

# Validation Report

**Status:** validated  
**Plan:** `{plan path}`  
**Name:** `{plan name}`  
**Tasks:** `{completed}/{total} complete`  
**Date:** `{YYYY-MM-DD}`

## Commands run

- `{command}` -> {passed} — {concise outcome summary}

## Acceptance criteria

- [x] AC1: {criterion statement} — {evidence}; scenarios: S1
- [x] AC2: {criterion statement} — {evidence}; scenarios: none (inspection)

## Scenario verification

- S1 (AC1): {test | command | inspection} `{exact test, command, or inspected location}` -> {actual result}; status: Verified

## Residual risks

- {risk}
- None identified.

## Notes

{Include only non-blocking information worth retaining.
Omit this section when unnecessary.}

---

## Failed report

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
- `{scenario id}` ({AC ids}): {Partially verified | Not verified} — {gap between the observed result and `Then:`}
  - Evidence: {exact test, command, or inspected location and its actual result}
  - Required action: {concrete repair or decision}

## Acceptance criteria

- [x] AC1: {criterion} — {evidence}; scenarios: S1
- [ ] AC2: {criterion} — {why unmet}; scenarios: S2

## Scenario verification

- S1 (AC1): {test | command | inspection} `{exact test, command, or inspected location}` -> {actual result}; status: Verified
- S2 (AC2): {test | command | inspection} `{exact test, command, or inspected location}` -> {actual result}; status: {Partially verified | Not verified}

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

---

## Blocked report

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
- Use the status and evidence already produced by validation execution; do not
  upgrade, reinterpret, or invent results while formatting this report.
- The failed variant must always end its **Recommended work** with
  `/validate {plan path}` as the final step after repairs.
- The failed variant must be self-contained enough to hand to another session
  without the original chat.
- The validated and failed variants include **Scenario
  verification** with exactly one entry per scenario in plan order: the scenario
  ID, the acceptance-criterion IDs that reference it, the evidence kind (`test`,
  `command`, or `inspection`), the exact test, command, or inspected location,
  the actual result, and the scenario status. Each acceptance-criterion line
  cites its scenario IDs, or `none (inspection)`. In the failed variant, list
  every `Partially verified` or `Not verified` scenario under **What failed**.
- Do not include durable context synchronization results in this report.
- Do not select or describe an unrelated next implementation task when status is
  `validated`.
- Omit empty optional sections rather than writing placeholders.
