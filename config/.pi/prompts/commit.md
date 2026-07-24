---
description: "Use `sce-atomic-commit` to propose atomic commit message(s) from staged changes"
argument-hint: "[oneshot|skip]"
---

## Purpose
<!-- sce-execution-profile: shared-context-code -->
- Perform controlled repository and operational work from explicit user intent or an approved SCE workflow.
- Keep implementation evidence and durable context aligned with code truth.
- Produce repository-style atomic commit messaging from staged changes.
- In regular mode, return proposals only; in `oneshot`/`skip` mode, produce one message and execute one commit.

## Inputs
- The active workflow, requested scope, repository state, applicable acceptance criteria, and human decisions.
- Relevant code, configuration, context, and verification commands.
- `$ARGUMENTS`: optional commit context; the first token selects bypass mode when it is `oneshot` or `skip` (case-insensitive).
- The staged diff from `git diff --cached`.

## Preconditions
- Before acting, read `.pi/skills/sce-atomic-commit/SKILL.md` completely and follow it as the entry procedure.
1. Establish the active workflow's authority, boundaries, and observable completion criteria before writes.
2. Resolve blockers or ambiguity required by that workflow before irreversible or scope-expanding action.
3. Inspect existing worktree state and preserve unrelated changes.
1. Determine regular or bypass mode from the first argument token.
2. In regular mode, ask the user to stage all intended files and confirm staging.
3. In bypass mode, skip the staging prompt but require a non-empty staged diff.

## Workflow
1. Establish current truth from relevant repository and context sources.
2. Follow the invoked workflow and its required skills for implementation, handover, commit, or validation work.
3. Make the smallest coherent in-scope change and collect proportionate evidence.
4. Reconcile durable context when behavior, policy, architecture, or canonical terminology changes.
5. Return the workflow-specific result and remaining risks or handoff.
1. Load `sce-atomic-commit`.
2. In regular mode, classify staged scope, apply the skill's context guidance, and return one or more proposals plus split guidance when needed; do not commit.
3. In bypass mode, skip context-guidance gating and split analysis, require exactly one message, and treat plan/task citations as best-effort.
4. In bypass mode, run `git commit -m "<message>"` once.
5. Report proposals in regular mode or the new commit hash in bypass mode, then stop.

## Guardrails
- Do not expand scope, change dependencies, or overwrite unrelated work without explicit approval.
- Respect capability approvals before process execution, repository writes, or version-control actions when required.
- Keep stdout/stderr, generated-source ownership, and repository conventions intact.
- Treat the human as owner of architecture, risk, and final decisions.
- Treat code as source of truth when code and `context/` disagree; repair context instead of rationalizing drift.
- Keep temporary session material under `context/tmp/` and durable context current-state oriented.
- Delete a context file only when it exists and has no uncommitted changes.
- Analyze only intentionally staged changes.
- Keep message grammar and atomicity decisions skill-owned.
- Never invent plan slugs, task IDs, issue references, or change intent.
- In bypass mode, do not amend, retry, create fallback commits, or propose splits after a failed commit.

## Outputs
- The repository, context, evidence, or handoff artifacts required by the active workflow.
- A concise account of verification and any unresolved risk.
- Regular mode: commit-message proposal(s) and file split guidance when justified.
- Bypass mode: exactly one commit message and either the successful commit hash or the exact commit failure.

## Completion criteria
- The active workflow's acceptance and evidence requirements are satisfied.
- Repository and context state are consistent, and no unapproved scope expansion remains.
- Regular mode ends after faithful proposals are returned.
- Bypass mode ends after exactly one `git commit` attempt is reported.

## Failure handling
- Stop for a human decision before scope expansion, destructive action, or unresolved architecture and risk choices.
- Report failed checks with their command and relevant evidence; never claim success without proof.
- Preserve partial in-scope evidence and identify the workflow phase that failed.
- Stop with `No staged changes. Stage changes before commit.` when the staged diff is empty.
- In regular mode, stop for clarification when staged plan changes require citations that cannot be inferred faithfully.
- In bypass mode, omit ambiguous plan citations and report a failed commit without retrying.

## Related units
- `shared-context-code` — execution profile composed into this workflow.
- `sce-atomic-commit` — skill required by this workflow.
- `sce-atomic-commit` — sole owner of staged-diff analysis and message construction.
- `Shared Context Code` — default agent for this command.
