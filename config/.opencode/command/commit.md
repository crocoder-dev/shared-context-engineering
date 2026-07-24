---
description: "Use `sce-atomic-commit` to propose atomic commit message(s) from staged changes"
agent: "Shared Context Code"
subtask: false
entry-skill: "sce-atomic-commit"
skills:
  - "sce-atomic-commit"
permission:
  default: ask
  read: allow
  edit: allow
  glob: allow
  grep: allow
  list: allow
  bash: ask
  question: allow
  codesearch: allow
  lsp: allow
  skill:
    "*": ask
    "sce-atomic-commit": allow
---

## Purpose
- Produce repository-style atomic commit messaging from staged changes.
- In regular mode, return proposals only; in `oneshot`/`skip` mode, produce one message and execute one commit.

## Inputs
- `$ARGUMENTS`: optional commit context; the first token selects bypass mode when it is `oneshot` or `skip` (case-insensitive).
- The staged diff from `git diff --cached`.

## Preconditions
1. Intended changes are staged before invocation; `git diff --cached` is the authoritative change source.

## Workflow
1. Determine regular or bypass mode from the first argument token (`oneshot`/`skip`, case-insensitive).
2. Load `sce-atomic-commit`.
3. Regular mode: confirm staging, classify staged scope, apply the skill's context guidance, and return one or more proposals plus split guidance when needed; do not commit.
4. Bypass mode: require a non-empty staged diff, produce exactly one message, treat plan/task citations as best-effort, and run `git commit -m "<message>"` once.
5. Return the mode-specific result and stop.

## Guardrails
- Analyze only intentionally staged changes.
- Do not invent plan slugs, task IDs, issue references, or change intent absent from the diff or supplied context.
- Do not amend, retry, or make additional commit attempts.

## Outputs
- Regular mode: commit-message proposal(s) and file split guidance when justified.
- Bypass mode: exactly one commit message and either the successful commit hash or the exact commit failure.

## Completion criteria
- The invocation ends with mode-appropriate output: regular-mode proposals, or exactly one reported bypass-mode commit attempt.

## Failure handling
- Stop with `No staged changes. Stage changes before commit.` when the staged diff is empty.
- In regular mode, stop for clarification when staged plan changes require citations that cannot be inferred faithfully.
- In bypass mode, omit ambiguous plan citations and report a failed commit without retrying.

## Related units
- `shared-context-code` — execution profile bound to this workflow.
- `sce-atomic-commit` — entry skill for this workflow.
