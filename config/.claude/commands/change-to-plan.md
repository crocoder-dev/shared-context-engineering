---
description: "Use `sce-plan-authoring` to turn a change request into a scoped SCE plan"
allowed-tools: Task, Read, Glob, Grep, Edit, Write, AskUserQuestion, Skill, Bash
---

## Purpose
<!-- sce-execution-profile: shared-context-plan -->
- Establish planning policy for repository changes while keeping architecture, risk, and approval decisions human-owned.
- Produce implementation-ready context artifacts without crossing into implementation.
- Turn `$ARGUMENTS` into a scoped SCE implementation plan by delegating to `sce-plan-authoring`.
- Provide a planning handoff without beginning implementation.

## Inputs
- Change intent, repository and context truth, constraints, risks, and human decisions.
- The planning workflow and skills selected for the invocation.
- `$ARGUMENTS`: a change request and optional existing plan identifier.
- Any success criteria, constraints, non-goals, dependency choices, and acceptance signals included by the user.

## Preconditions
1. Establish whether baseline SCE context exists before planning writes begin.
2. Read the context map and relevant current-state context before broad exploration.
3. Keep planning blocked while critical scope, dependency, architecture, or acceptance decisions are unresolved.
1. Treat missing critical planning details as blocking.
2. Preserve the approval and clarification behavior owned by `sce-plan-authoring`.

## Workflow
1. Establish current truth from the minimum relevant code and context.
2. Use the invoked workflow and its entry skill to perform the requested planning action.
3. Preserve an explicit boundary between planning artifacts and implementation authorization.
4. End with a reviewable planning result or focused unresolved decisions.
1. Load `sce-plan-authoring`.
2. Pass `$ARGUMENTS` without inventing requirements.
3. Let the skill resolve new-versus-existing plan, clarification needs, plan shape, and atomic task slicing.
4. When ready, write or update `context/plans/{plan_name}.md`.
5. Return the exact path, ordered task list, and `/next-task {plan_name} T01`.
6. Stop after the planning handoff.

## Guardrails
- Do not modify application code or treat a planning result as approval to implement.
- Run process commands only when the active workflow and approved capability policy permit them.
- Write only planning and context artifacts required by the active workflow.
- Treat code as source of truth when code and `context/` disagree; repair focused context drift.
- Keep durable context current-state oriented and optimized for future AI sessions.
- Delete a context file only when it exists and has no uncommitted changes.
- Treat completed plans as disposable execution artifacts; promote durable outcomes into current-state context or `context/decisions/`.
- Keep this command thin; do not duplicate the skill's planning rules.
- Do not modify application code or imply implementation approval.
- Do not bypass the clarification gate.

## Outputs
- Planning or context artifacts requested by the active workflow.
- A bounded handoff that distinguishes completed planning from decisions still required.
- A plan path and complete ordered task list when planning succeeds.
- Focused clarification questions when planning is blocked.
- One canonical next command for a new implementation session.

## Completion criteria
- The active planning workflow's observable criteria are satisfied.
- Resulting artifacts are bounded, reviewable, and do not imply implementation approval.
- `sce-plan-authoring` reports a valid plan and the plan file exists at the reported path.
- The response includes the full task order and stops before implementation.

## Failure handling
- Stop when required context bootstrap authorization or a critical human decision is absent.
- Surface focused unresolved decisions instead of inventing requirements or writing partial authoritative plans.
- When context is stale, proceed from code truth only within the active planning scope and identify the repair.
- Stop and surface the skill's focused questions when critical information is missing.
- Report path or write failures directly; do not claim a plan was saved when it was not.

## Related units
- `shared-context-plan` — execution profile composed into this workflow.
- `sce-plan-authoring` — skill required by this workflow.
- `sce-plan-authoring` — sole owner of detailed planning behavior.
- `Shared Context Plan` — default agent for this command.
- `/next-task` — canonical next entrypoint after plan approval.
