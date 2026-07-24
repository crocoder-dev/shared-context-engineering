---
description: "Use `sce-atomic-commit` to propose atomic commit message(s) from staged changes"
agent: "Shared Context Code"
subtask: false
entry-skill: "sce-atomic-commit"
skills:
  - "sce-atomic-commit"
permission:
  default: block
  read: allow
  edit: allow
  glob: allow
  grep: allow
  list: allow
  bash: allow
  question: allow
  codesearch: allow
  lsp: allow
  skill:
    "*": block
    "sce-atomic-commit": allow
---

## Purpose
- Produce one repository-style commit message from staged changes and execute exactly one commit.

## Inputs
- `$ARGUMENTS`: optional commit context.
- Non-empty staged diff.

## Preconditions
1. Skip staging confirmation.
2. Require `git diff --cached` to be non-empty.

## Workflow
1. Load `sce-atomic-commit`.
2. Classify staged scope and apply the skill's context-guidance rules.
3. Produce exactly one message for the staged diff; do not branch into split guidance.
4. Run `git commit -m "<message>"` once.
5. Report the commit hash or exact failure and stop.

## Guardrails
- Use only staged changes.
- Do not invent change intent or plan/task citations.
- Do not retry, amend, or create fallback commits after failure.

## Outputs
- One commit message and either a successful commit hash or exact commit failure.

## Completion criteria
- Exactly one commit attempt is reported.

## Failure handling
- Stop with `No staged changes. Stage changes before commit.` when empty.
- Stop for ambiguous required plan citations rather than guessing.

## Related units
- `sce-atomic-commit` — one-message construction owner.
