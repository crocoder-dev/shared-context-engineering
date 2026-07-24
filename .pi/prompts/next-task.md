---
description: "Run `sce-plan-review` -> `sce-task-execution` -> `sce-context-sync` for one approved SCE task"
argument-hint: "<plan-name> [T0X]"
---

## Purpose
<!-- sce-execution-profile: shared-context-code -->
- Perform controlled repository and operational work from explicit user intent or an approved SCE workflow.
- Keep implementation evidence and durable context aligned with code truth.
- Review, authorize, execute, verify, and context-sync one SCE plan task.
- Route final tasks through full validation and non-final tasks to a clean next-session handoff.

## Inputs
- The active workflow, requested scope, repository state, applicable acceptance criteria, and human decisions.
- Relevant code, configuration, context, and verification commands.
- `$ARGUMENTS`: plan name or path (required) and task ID `T0X` (optional).
- User decisions or confirmation when the readiness gate cannot auto-pass.

## Preconditions
- Before acting, read `.pi/skills/sce-plan-review/SKILL.md` completely and follow it as the entry procedure.
1. Establish the active workflow's authority, boundaries, and observable completion criteria before writes.
2. Resolve blockers or ambiguity required by that workflow before irreversible or scope-expanding action.
3. Inspect existing worktree state and preserve unrelated changes.
1. Resolve an existing plan and task through `sce-plan-review`.
2. Require no blockers, ambiguity, or missing acceptance criteria.
3. Auto-pass readiness only when both plan and task ID are explicit and review is clean; otherwise obtain explicit user confirmation.

## Workflow
1. Establish current truth from relevant repository and context sources.
2. Follow the invoked workflow and its required skills for implementation, handover, commit, or validation work.
3. Make the smallest coherent in-scope change and collect proportionate evidence.
4. Reconcile durable context when behavior, policy, architecture, or canonical terminology changes.
5. Return the workflow-specific result and remaining risks or handoff.
1. Load `sce-plan-review` and return its readiness verdict.
2. Resolve open points and obtain readiness authorization when required.
3. Load `sce-task-execution` and preserve its mandatory pre-implementation stop.
4. After implementation, load `sce-context-sync` as a done gate.
5. Wait for feedback; apply only in-scope fixes, rerun light checks, and sync context again.
6. If the task is final, load `sce-validation`; otherwise return `/next-task {plan_name} T0X` for the next unchecked task.

## Guardrails
- Do not expand scope, change dependencies, or overwrite unrelated work without explicit approval.
- Respect capability approvals before process execution, repository writes, or version-control actions when required.
- Keep stdout/stderr, generated-source ownership, and repository conventions intact.
- Treat the human as owner of architecture, risk, and final decisions.
- Treat code as source of truth when code and `context/` disagree; repair context instead of rationalizing drift.
- Keep temporary session material under `context/tmp/` and durable context current-state oriented.
- Delete a context file only when it exists and has no uncommitted changes.
- Keep this command as orchestration; detailed review, implementation, sync, and validation rules remain skill-owned.
- Execute one task by default.
- Do not write code before readiness authorization and the task-execution gate pass.
- Stop before scope expansion.

## Outputs
- The repository, context, evidence, or handoff artifacts required by the active workflow.
- A concise account of verification and any unresolved risk.
- A readiness verdict.
- Implemented changes with verification evidence and updated task status.
- Context-sync results.
- Either a final validation result or the exact next-session command.

## Completion criteria
- The active workflow's acceptance and evidence requirements are satisfied.
- Repository and context state are consistent, and no unapproved scope expansion remains.
- The selected task is complete with evidence and synchronized context.
- Final tasks include a validation report; non-final tasks include the next task handoff.

## Failure handling
- Stop for a human decision before scope expansion, destructive action, or unresolved architecture and risk choices.
- Report failed checks with their command and relevant evidence; never claim success without proof.
- Preserve partial in-scope evidence and identify the workflow phase that failed.
- Stop on unresolved readiness issues and list the decision needed.
- Stop on scope expansion, failed checks that cannot be fixed in scope, or context-sync blockers.
- Preserve partial evidence and report the exact phase that failed.

## Related units
- `shared-context-code` — execution profile composed into this workflow.
- `sce-plan-review` — skill required by this workflow.
- `sce-task-execution` — skill required by this workflow.
- `sce-context-sync` — skill required by this workflow.
- `sce-validation` — skill required by this workflow.
- `sce-plan-review` — task selection and readiness.
- `sce-task-execution` — implementation and task-level evidence.
- `sce-context-sync` — durable context reconciliation.
- `sce-validation` — final full validation and cleanup.
