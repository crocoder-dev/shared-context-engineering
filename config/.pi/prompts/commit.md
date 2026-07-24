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
- Before acting, read `.pi/skills/sce-atomic-commit/SKILL.md` completely and follow it as the entry procedure.
1. Establish the active workflow's authority, boundaries, and observable completion criteria before writes.
2. Resolve blockers or ambiguity required by that workflow before irreversible or scope-expanding action.
3. Inspect existing worktree state and preserve unrelated changes.
1. Intended changes are staged before invocation; `git diff --cached` is the authoritative change source.

## Workflow
1. Determine regular or bypass mode from the first argument token (`oneshot`/`skip`, case-insensitive).
2. Load `sce-atomic-commit`.
3. Regular mode: confirm staging, classify staged scope, apply the skill's context guidance, and return one or more proposals plus split guidance when needed; do not commit.
4. Bypass mode: require a non-empty staged diff, produce exactly one message, treat plan/task citations as best-effort, and run `git commit -m "<message>"` once.
5. Return the mode-specific result and stop.

## Guardrails
- Do not expand scope, change dependencies, or overwrite unrelated work without explicit approval.
- Respect capability approvals before process execution, repository writes, or version-control actions when required.
- Keep stdout/stderr, generated-source ownership, and repository conventions intact.
- Treat the human as owner of architecture, risk, and final decisions.
- Treat code as source of truth when code and `context/` disagree; repair context instead of rationalizing drift.
- Keep temporary session material under `context/tmp/` and durable context current-state oriented.
- Delete a context file only when it exists and has no uncommitted changes.
- Analyze only intentionally staged changes.
- Do not invent plan slugs, task IDs, issue references, or change intent absent from the diff or supplied context.
- Do not amend, retry, or make additional commit attempts.

## Outputs
- Regular mode: commit-message proposal(s) and file split guidance when justified.
- Bypass mode: exactly one commit message and either the successful commit hash or the exact commit failure.

## Completion criteria
- The invocation ends with mode-appropriate output: regular-mode proposals, or exactly one reported bypass-mode commit attempt.

## Failure handling
- Stop for a human decision before scope expansion, destructive action, or unresolved architecture and risk choices.
- Report failed checks with their command and relevant evidence; never claim success without proof.
- Preserve partial in-scope evidence and identify the workflow phase that failed.
- Stop with `No staged changes. Stage changes before commit.` when the staged diff is empty.
- In regular mode, stop for clarification when staged plan changes require citations that cannot be inferred faithfully.
- In bypass mode, omit ambiguous plan citations and report a failed commit without retrying.

## Related units
- `shared-context-code` — execution profile bound to this workflow.
- `sce-atomic-commit` — entry skill for this workflow.
