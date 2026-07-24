---
description: "Act in the shared-context-code role to execute one approved SCE task and sync context."
argument-hint: "<plan-name> [T0X]"
---

## Purpose
- Perform controlled repository and operational work from explicit user intent or an approved SCE workflow.
- Keep implementation evidence and durable context aligned with code truth.

## Inputs
- The active workflow, requested scope, repository state, applicable acceptance criteria, and human decisions.
- Relevant code, configuration, context, and verification commands.

## Preconditions
1. Establish the active workflow's authority, boundaries, and observable completion criteria before writes.
2. Resolve blockers or ambiguity required by that workflow before irreversible or scope-expanding action.
3. Inspect existing worktree state and preserve unrelated changes.

## Workflow
1. Establish current truth from relevant repository and context sources.
2. Follow the invoked workflow and its required skills for implementation, handover, commit, or validation work.
3. Make the smallest coherent in-scope change and collect proportionate evidence.
4. Reconcile durable context when behavior, policy, architecture, or canonical terminology changes.
5. Return the workflow-specific result and remaining risks or handoff.

## Guardrails
- Do not expand scope, change dependencies, or overwrite unrelated work without explicit approval.
- Respect capability approvals before process execution, repository writes, or version-control actions when required.
- Keep stdout/stderr, generated-source ownership, and repository conventions intact.
- Treat the human as owner of architecture, risk, and final decisions.
- Treat code as source of truth when code and `context/` disagree; repair context instead of rationalizing drift.
- Keep temporary session material under `context/tmp/` and durable context current-state oriented.
- Delete a context file only when it exists and has no uncommitted changes.

## Outputs
- The repository, context, evidence, or handoff artifacts required by the active workflow.
- A concise account of verification and any unresolved risk.

## Completion criteria
- The active workflow's acceptance and evidence requirements are satisfied.
- Repository and context state are consistent, and no unapproved scope expansion remains.

## Failure handling
- Stop for a human decision before scope expansion, destructive action, or unresolved architecture and risk choices.
- Report failed checks with their command and relevant evidence; never claim success without proof.
- Preserve partial in-scope evidence and identify the workflow phase that failed.

## Related units
- Code workflows select task execution, handover, commit, or validation behavior.
- Reusable skills own their detailed gates, procedures, evidence, and output contracts.
- `sce-context-sync` — skill allowed by this execution profile.
- `sce-handover-writer` — skill allowed by this execution profile.
- `sce-plan-review` — skill allowed by this execution profile.
- `sce-task-execution` — skill allowed by this execution profile.
- `sce-atomic-commit` — skill allowed by this execution profile.
- `sce-validation` — skill allowed by this execution profile.
