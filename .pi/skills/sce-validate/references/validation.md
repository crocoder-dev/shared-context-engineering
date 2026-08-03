# Validation phase

Run this phase for step 1 of the workflow. It resolves one plan, confirms the
implementation is finished, runs the plan's checks, and records what it found.

Input: the plan name or path, unmodified.

This phase exclusively owns:

- Resolving one plan.
- Confirming every implementation task is complete.
- Running full validation and acceptance-criteria checks.
- Removing temporary scaffolding.
- Writing the Validation Report into the plan.
- Returning one Markdown validation result.

Do not duplicate any of it elsewhere in the workflow.

## 1.1 Resolve the plan

Resolve the supplied plan name or path to exactly one existing plan under
`context/plans/`.

When no plan can be found, set internal status `blocked`.

When multiple plans match and none can be selected safely, set internal status
`blocked` with the matching candidates.

Read the selected plan before exploring the repository.

## 1.2 Confirm implementation is finished

Set internal status `blocked` with incomplete tasks listed when any
implementation task remains incomplete.

Final validation measures finished work. Do not run the full suite against a
partial stack, and do not complete remaining tasks here.

## 1.3 Read the validation contract from the plan

From the plan, collect:

- Every acceptance criterion and its `Validate:` check.
- The `Full validation` command list.
- The `Context sync` requirements, for the context-impact handoff only.

Set internal status `blocked` when the plan has no usable acceptance criteria, or
when no validation commands can be determined from the plan or repository
conventions.

Prefer the plan's authored checks. Fall back to repository-primary test, lint,
and format commands only when `Full validation` is absent, and record that
fallback under notes on a `validated` or `failed` result.

## 1.4 Remove temporary scaffolding

Before or while running checks, remove temporary scaffolding introduced during
the change when it is clearly throwaway:

- Debug-only patches or flags left enabled.
- Temporary files or intermediate artifacts not part of the delivered design.
- Local scaffolding the plan or task notes mark as temporary.

Do not delete durable product code, tests, configuration, or context files.

Record every removed path. When nothing temporary remains, report `None.`

## 1.5 Run full validation and acceptance checks

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

Do not run task-by-task implementation work for incomplete tasks. That belongs to
`/next-task`.

## 1.6 Update the plan

For `validated` and `failed` outcomes:

- Mark each acceptance criterion checkbox to match the evidence.
- Append or replace the plan's `## Validation Report` section using
  `references/validation-report.md`. Read that file before writing the section.
- When status is `failed`, the plan-file report must include the retry command
  `/validate {plan path}`.

Do not reopen completed tasks, rewrite task evidence, or change the task stack.

For `blocked`, leave the plan file unchanged.

## 1.7 Determine context impact for the handoff

On `validated` only, classify the durable context impact of the finished plan so
the **Plan context synchronization phase** can start from the plan's own
requirements:

- Start from the plan's `Context sync` section.
- Inspect what the completed implementation actually changed when needed.
- Report required context paths and affected areas.
- Use `none`, `local`, `domain`, or `root` with the same meanings as task-level
  context sync.

Do not edit context files here.

On `failed` or `blocked`, omit context impact; context sync will not run.

## 1.8 Return the internal state

Set exactly one internal state:

- `validated` when every acceptance criterion is met, required full validation
  passed, and the Validation Report was written.
- `failed` when evidence was captured but required checks or criteria remain
  unsatisfied. Shape it as a session handoff per `references/output.md`, ending
  recommended work with `/validate {plan path}`.
- `blocked` when validation cannot proceed safely.

Record only the Markdown report. Do not add explanatory prose before or after it.
Do not return internal state.

A `validated` result is the authoritative handoff into step 2, which reads the
plan path, required context paths, validation evidence, and reported context
impact out of it. It must report:

```markdown
**Status:** validated
**Plan:** {plan path}
```

and must carry the resolved plan path, validation commands and outcomes,
acceptance-criteria evidence, scaffolding removals, and the reported context
impact with required context paths and affected areas. Step 2 is forbidden from
reconstructing any of that, so it has to be present here.

## Validation boundaries

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
- Return a internal state.
- Run plan context synchronization. The workflow owns that step.
