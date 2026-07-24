---
description: "Run `sce-handover-writer` to capture the current task for handoff"
argument-hint: "[task context]"
---

## Purpose
<!-- sce-execution-profile: shared-context-code -->
- Perform controlled repository and operational work from explicit user intent or an approved SCE workflow.
- Keep implementation evidence and durable context aligned with code truth.
- Create a durable handover for the current task by delegating to `sce-handover-writer`.

## Inputs
- The active workflow, requested scope, repository state, applicable acceptance criteria, and human decisions.
- Relevant code, configuration, context, and verification commands.
- `$ARGUMENTS`: optional plan name, task ID, scope note, or handover context.
- Current repository, plan, and task state available to the agent.

## Preconditions
- Before acting, read `.pi/skills/sce-handover-writer/SKILL.md` completely and follow it as the entry procedure.
1. Establish the active workflow's authority, boundaries, and observable completion criteria before writes.
2. Resolve blockers or ambiguity required by that workflow before irreversible or scope-expanding action.
3. Inspect existing worktree state and preserve unrelated changes.
1. Identify the current plan/task when possible.
2. Distinguish observed facts from inferred details.

## Workflow
1. Establish current truth from relevant repository and context sources.
2. Follow the invoked workflow and its required skills for implementation, handover, commit, or validation work.
3. Make the smallest coherent in-scope change and collect proportionate evidence.
4. Reconcile durable context when behavior, policy, architecture, or canonical terminology changes.
5. Return the workflow-specific result and remaining risks or handoff.
1. Load `sce-handover-writer`.
2. Pass `$ARGUMENTS` and the current task state.
3. Let the skill choose task-aligned naming and write the handover under `context/handovers/`.
4. Return the exact handover path and stop.

## Guardrails
- Do not expand scope, change dependencies, or overwrite unrelated work without explicit approval.
- Respect capability approvals before process execution, repository writes, or version-control actions when required.
- Keep stdout/stderr, generated-source ownership, and repository conventions intact.
- Treat the human as owner of architecture, risk, and final decisions.
- Treat code as source of truth when code and `context/` disagree; repair context instead of rationalizing drift.
- Keep temporary session material under `context/tmp/` and durable context current-state oriented.
- Delete a context file only when it exists and has no uncommitted changes.
- Keep this command thin; the skill owns structure, naming, and completeness checks.
- Label unsupported inferences as assumptions.
- Do not implement or change task scope while producing a handover.

## Outputs
- The repository, context, evidence, or handoff artifacts required by the active workflow.
- A concise account of verification and any unresolved risk.
- One complete handover file and its exact path.

## Completion criteria
- The active workflow's acceptance and evidence requirements are satisfied.
- Repository and context state are consistent, and no unapproved scope expansion remains.
- The handover records current task state, decisions and rationale, blockers/open questions, and one next recommended step.

## Failure handling
- Stop for a human decision before scope expansion, destructive action, or unresolved architecture and risk choices.
- Report failed checks with their command and relevant evidence; never claim success without proof.
- Preserve partial in-scope evidence and identify the workflow phase that failed.
- When no reliable task state can be established, stop with the missing inputs rather than inventing a handover.
- Report write failures directly.

## Related units
- `shared-context-code` — execution profile composed into this workflow.
- `sce-handover-writer` — skill required by this workflow.
- `sce-handover-writer` — sole owner of handover content and file shape.
- `Shared Context Code` — default agent for this command.
