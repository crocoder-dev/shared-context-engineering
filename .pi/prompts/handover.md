---
description: "Run `sce-handover-writer` to capture the current task for handoff"
argument-hint: "[task context]"
---

## Purpose
- Create a durable handover for the current task by delegating to `sce-handover-writer`.

## Inputs
- `$ARGUMENTS`: optional plan name, task ID, scope note, or handover context.
- Current repository, plan, and task state available to the agent.

## Preconditions
- Before acting, read `.pi/skills/sce-handover-writer/SKILL.md` completely and follow it as the entry procedure.
1. Establish the active workflow's authority, boundaries, and observable completion criteria before writes.
2. Resolve blockers or ambiguity required by that workflow before irreversible or scope-expanding action.
3. Inspect existing worktree state and preserve unrelated changes.
1. The current plan and task can be identified when available.

## Workflow
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
- Distinguish observed facts from inferences, and label assumptions and unresolved questions as such.
- Do not implement or change task scope while producing a handover.

## Outputs
- One complete handover file and its exact path under `context/handovers/`.

## Completion criteria
- The handover records current task state, decisions and rationale, blockers/open questions, and one next recommended step.

## Failure handling
- Stop for a human decision before scope expansion, destructive action, or unresolved architecture and risk choices.
- Report failed checks with their command and relevant evidence; never claim success without proof.
- Preserve partial in-scope evidence and identify the workflow phase that failed.
- When no reliable task state can be established, stop with the missing inputs rather than inventing a handover.
- Report write failures directly.

## Related units
- `shared-context-code` — execution profile bound to this workflow.
- `sce-handover-writer` — entry skill for this workflow.
