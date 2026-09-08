# Plan-file Validation Report

The Markdown section `sce-validation` appends to the plan file when returning
`validated` or `failed`. Write it at the end of `context/plans/{plan_name}.md`
under exactly one `## Validation Report` heading.

This is plan-file content. The result returned to the workflow is defined
separately in `references/validation.md`.

This reference owns only the persisted report's structure and presentation.
Validation execution owns command selection and execution, evidence
interpretation, acceptance-criterion state, outcome classification, and the
non-repairing boundary. Consume those results here; do not redefine or rerun
validation policy in this report reference.

Do not author this section while planning. Only `/validate` through `sce-validation`
writes it.

## Layout

```markdown
## Validation Report

**Status:** {validated | failed}  
**Date:** {YYYY-MM-DD}

### Commands run

- `{command}` -> exit {code} ({concise outcome summary})
- `{command}` -> exit {code} ({concise outcome summary})

### Success-criteria verification

- [x] AC1: {criterion statement} -> {evidence}
- [ ] AC2: {criterion statement} -> {evidence of failure or not checked}

### Failed checks and follow-ups

- {check}: {problem}; evidence: {command output or inspection}; required: {decision or next action}
- None.

### Residual risks

- {risk}
- None identified.

### Retry

{Only when Status is failed:}

After repairs, rerun:

`/validate {plan path}`
```

## Rules

- Use the `validated` or `failed` status produced by validation execution; this
  report does not redefine status-selection criteria.
- List every command result supplied by validation execution under **Commands
  run**. Preserve its exit code and concise outcome; do not invent either.
- Under **Success-criteria verification**, render the acceptance-criterion
  checkbox state and evidence already established by validation execution. Do
  not independently re-evaluate or reclassify criteria here.
- Under **Failed checks and follow-ups**, render every failure and follow-up
  supplied by validation execution. Write `None.` when status is `validated`.
- When status is `failed`, always include **Retry** with the exact
  `/validate {plan path}` command. Omit **Retry** when status is `validated`.
- Keep evidence concise and factual. Do not narrate the whole implementation
  history or add execution-policy claims absent from the validation result.
- When a previous `## Validation Report` already exists, replace it with the new
  one rather than stacking duplicates.
