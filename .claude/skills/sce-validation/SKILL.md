---
name: sce-validation
description: >
  Internal SCE workflow skill that runs final plan validation after all
  implementation tasks are complete: full validation commands, acceptance
  criteria checks, temporary scaffolding cleanup, a Validation Report written
  into the plan, and one Markdown result (validated, failed, or blocked).
  Failing checks are reported only; do not modify tests or product code to make
  validation pass. A failed result is a session handoff that ends by retrying
  /validate. Use from /validate. Do not synchronize durable context, implement
  remaining plan tasks, create commits, or select another task.
compatibility: claude
---

# SCE Validation

## Purpose

Prove that one finished SCE plan meets its acceptance criteria and repository
validation bar, then record the evidence on the plan and return one Markdown
result.

This skill owns:

- Resolving one plan.
- Confirming every implementation task is complete.
- Running the plan's full validation commands and each acceptance criterion
  check.
- Removing temporary scaffolding introduced by the change.
- Writing the Validation Report into the plan.
- Marking acceptance criteria against the evidence.
- Returning one Markdown validation result.

Return a result matching:

`references/validation-result.md`

Write plan-file evidence matching:

`references/validation-report.md`

Context synchronization is not this skill's job. The invoking `/validate`
workflow runs `sce-plan-context-sync` only after a `validated` result.

## Input

The invoking workflow provides:

- A plan name or path.

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
- The `Context sync` requirements, for the context-impact handoff only.

Return `blocked` when the plan has no usable acceptance criteria, or when no
validation commands can be determined from the plan or repository conventions.

Prefer the plan's authored checks. Fall back to repository-primary test, lint,
and format commands only when `Full validation` is absent, and record that
fallback under notes on a `validated` or `failed` result.

### 4. Remove temporary scaffolding

Before or while running checks, remove temporary scaffolding introduced during
the change when it is clearly throwaway:

- Debug-only patches or flags left enabled.
- Temporary files or intermediate artifacts not part of the delivered design.
- Local scaffolding the plan or task notes mark as temporary.

Do not delete durable product code, tests, configuration, or context files.

Record every removed path. When nothing temporary remains, report `None.`

### 5. Run full validation and acceptance checks

Run the plan's `Full validation` commands.

Then verify each acceptance criterion using its `Validate:` line. Prefer a
runnable command. Use a named inspection only when the criterion authorizes it,
and say exactly what was inspected.

When a check fails, record the failure and continue gathering evidence. Do not
modify tests, application code, or configuration to make a check pass. Final
validation measures the finished work; repair belongs to a later work session,
not this skill.

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

### 7. Determine context impact for the handoff

On `validated` only, classify the durable context impact of the finished plan
so `sce-plan-context-sync` can start from the plan's own requirements:

- Start from the plan's `Context sync` section.
- Inspect what the completed implementation actually changed when needed.
- Report required context paths and affected areas.
- Use `none`, `local`, `domain`, or `root` with the same meanings as task-level
  context sync.

Do not edit context files here.

On `failed` or `blocked`, omit context impact; context sync will not run.

### 8. Return the Markdown result

Return exactly one Markdown result:

- `validated` when every acceptance criterion is met, required full validation
  passed, and the Validation Report was written.
- `failed` when evidence was captured but required checks or criteria remain
  unsatisfied. Shape it as a session handoff per
  `references/validation-result.md`, ending recommended work with
  `/validate {plan path}`.
- `blocked` when validation cannot proceed safely.

Return only the Markdown report. Do not add explanatory prose before or after
it. Do not return YAML.

## Boundaries

Do not:

- Validate more than one plan.
- Complete remaining implementation tasks.
- Modify tests, application code, or configuration to make a failing check pass.
- Apply lint or format auto-fixes that change product or test files as part of
  making validation green.
- Synchronize durable context under `context/` outside the plan file.
- Create the context root.
- Mark the plan archived or delete the plan.
- Create a Git commit or push changes.
- Invent acceptance criteria the plan does not state.
- Claim verification that was not performed.
- Return a YAML result.
- Invoke plan context sync. The workflow owns that step.

## Completion

The skill is complete after:

- One plan was resolved, or resolution failed and was reported.
- Implementation completeness was checked.
- Validation ran to a terminal state, or a blocker prevented it.
- One valid Markdown result matching `references/validation-result.md` was
  returned.
