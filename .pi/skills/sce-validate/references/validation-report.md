# Plan-file Validation Report

The Markdown section `sce-validation` appends to the plan file when returning
`validated` or `failed`. Write it at the end of `context/plans/{plan_name}.md`
under exactly one `## Validation Report` heading.

This is plan-file content. The result returned to the workflow is defined
separately in `references/validation-result.md`.

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

### Scaffolding removed

- `{path}` — {why it was temporary}
- None.

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

- Use **Status:** `validated` only when every acceptance criterion is met and
  every required full-validation command passed.
- Use **Status:** `failed` when evidence was captured but required checks or
  criteria remain unsatisfied.
- List every command that ran under **Commands run**, including ones that
  failed. Do not invent exit codes or outcomes.
- Prefer the plan's `Full validation` commands and each criterion's `Validate:`
  line over rediscovering project defaults. Fall back to repository conventions
  only when the plan omits them.
- Mark each acceptance criterion checkbox in the plan's `## Acceptance criteria`
  section to match the evidence. Do not mark a criterion met unless the check
  ran successfully or the inspection named by `Validate:` confirms it.
- Under **Scaffolding removed**, list only temporary debug code, intermediate
  artifacts, or throwaway files introduced during the change. Write `None.` when
  nothing temporary remained.
- Under **Failed checks and follow-ups**, record the failing check and its
  evidence only. Do not describe code or test edits made during validation;
  validation does not modify tests or product code to clear failures. Write
  `None.` when status is `validated`.
- When status is `failed`, always include **Retry** with the exact
  `/validate {plan path}` command. Omit **Retry** when status is `validated`.
- Keep evidence concise and factual. Do not narrate the whole implementation
  history.
- Do not claim context synchronization completed. Plan context sync is a later
  workflow step and runs only after `validated`.
- Do not rewrite task evidence or reopen completed tasks.
- When a previous `## Validation Report` already exists, replace it with the new
  one rather than stacking duplicates.
