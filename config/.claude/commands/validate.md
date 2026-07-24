---
description: "Run `sce-validation` to finish an SCE plan with validation and cleanup"
allowed-tools: Task, Read, Glob, Grep, Edit, Write, AskUserQuestion, Skill, Bash
---

## Purpose
<!-- sce-execution-profile: shared-context-code -->
- Perform controlled repository and operational work from explicit user intent or an approved SCE workflow.
- Keep implementation evidence and durable context aligned with code truth.
- Run the final SCE validation phase by delegating to `sce-validation`.

## Inputs
- The active workflow, requested scope, repository state, applicable acceptance criteria, and human decisions.
- Relevant code, configuration, context, and verification commands.
- `$ARGUMENTS`: target plan name/path or change identifier.
- The plan's success criteria and current repository state.

## Preconditions
1. Establish the active workflow's authority, boundaries, and observable completion criteria before writes.
2. Resolve blockers or ambiguity required by that workflow before irreversible or scope-expanding action.
3. Inspect existing worktree state and preserve unrelated changes.
1. Resolve the target plan or completed change.
2. Confirm implementation is ready for final validation.

## Workflow
1. Establish current truth from relevant repository and context sources.
2. Follow the invoked workflow and its required skills for implementation, handover, commit, or validation work.
3. Make the smallest coherent in-scope change and collect proportionate evidence.
4. Reconcile durable context when behavior, policy, architecture, or canonical terminology changes.
5. Return the workflow-specific result and remaining risks or handoff.
1. Load `sce-validation`.
2. Pass the target and let the skill discover project checks, capture evidence, clean temporary scaffolding, and verify context.
3. Return the pass/fail result and validation-report location.
4. Stop after reporting validation.

## Guardrails
- Do not expand scope, change dependencies, or overwrite unrelated work without explicit approval.
- Respect capability approvals before process execution, repository writes, or version-control actions when required.
- Keep stdout/stderr, generated-source ownership, and repository conventions intact.
- Treat the human as owner of architecture, risk, and final decisions.
- Treat code as source of truth when code and `context/` disagree; repair context instead of rationalizing drift.
- Keep temporary session material under `context/tmp/` and durable context current-state oriented.
- Delete a context file only when it exists and has no uncommitted changes.
- Keep this command thin; validation scope, command discovery, repairs, evidence, and report shape remain skill-owned.
- Do not convert failed validation into a success result.

## Outputs
- The repository, context, evidence, or handoff artifacts required by the active workflow.
- A concise account of verification and any unresolved risk.
- Validation status, commands and evidence summary, residual risks, and report location.

## Completion criteria
- The active workflow's acceptance and evidence requirements are satisfied.
- Repository and context state are consistent, and no unapproved scope expansion remains.
- `sce-validation` records a conclusive result against every success criterion.

## Failure handling
- Stop for a human decision before scope expansion, destructive action, or unresolved architecture and risk choices.
- Report failed checks with their command and relevant evidence; never claim success without proof.
- Preserve partial in-scope evidence and identify the workflow phase that failed.
- Report unresolved failures and their evidence; do not close the plan while required checks remain failed or unevaluated.

## Related units
- `shared-context-code` — execution profile composed into this workflow.
- `sce-validation` — skill required by this workflow.
- `sce-validation` — sole owner of final validation behavior.
- `Shared Context Code` — default agent for this command.
