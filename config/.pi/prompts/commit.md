---
description: "Use `sce-atomic-commit` to propose atomic commit message(s) from staged changes"
argument-hint: "[oneshot|skip]"
---

## Purpose
- Produce repository-style atomic commit messaging from staged changes.
- In regular mode, return proposals only; in `oneshot`/`skip` mode, produce one message and execute one commit.

## Inputs
- `$ARGUMENTS`: optional commit context; the first token selects bypass mode when it is `oneshot` or `skip` (case-insensitive).
- The staged diff from `git diff --cached`.

## Preconditions
1. Determine regular or bypass mode from the first argument token.
2. In regular mode, ask the user to stage all intended files and confirm staging.
3. In bypass mode, skip the staging prompt but require a non-empty staged diff.

## Workflow
1. Load `sce-atomic-commit`.
2. In regular mode, classify staged scope, apply the skill's context guidance, and return one or more proposals plus split guidance when needed; do not commit.
3. In bypass mode, skip context-guidance gating and split analysis, require exactly one message, and treat plan/task citations as best-effort.
4. In bypass mode, run `git commit -m "<message>"` once.
5. Report proposals in regular mode or the new commit hash in bypass mode, then stop.

## Guardrails
- Analyze only intentionally staged changes.
- Keep message grammar and atomicity decisions skill-owned.
- Never invent plan slugs, task IDs, issue references, or change intent.
- In bypass mode, do not amend, retry, create fallback commits, or propose splits after a failed commit.

## Outputs
- Regular mode: commit-message proposal(s) and file split guidance when justified.
- Bypass mode: exactly one commit message and either the successful commit hash or the exact commit failure.

## Completion criteria
- Regular mode ends after faithful proposals are returned.
- Bypass mode ends after exactly one `git commit` attempt is reported.

## Failure handling
- Stop with `No staged changes. Stage changes before commit.` when the staged diff is empty.
- In regular mode, stop for clarification when staged plan changes require citations that cannot be inferred faithfully.
- In bypass mode, omit ambiguous plan citations and report a failed commit without retrying.

## Related units
- `sce-atomic-commit` — sole owner of staged-diff analysis and message construction.
- `Shared Context Code` — default agent for this command.
