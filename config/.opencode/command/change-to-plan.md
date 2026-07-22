---
description: "Use `sce-plan-authoring` to turn a change request into a scoped SCE plan"
agent: "Shared Context Plan"
entry-skill: "sce-plan-authoring"
skills:
  - "sce-plan-authoring"
---

## Purpose
- Turn `$ARGUMENTS` into a scoped SCE implementation plan by delegating to `sce-plan-authoring`.
- Provide a planning handoff without beginning implementation.

## Inputs
- `$ARGUMENTS`: a change request and optional existing plan identifier.
- Any success criteria, constraints, non-goals, dependency choices, and acceptance signals included by the user.

## Preconditions
1. Treat missing critical planning details as blocking.
2. Preserve the approval and clarification behavior owned by `sce-plan-authoring`.

## Workflow
1. Load `sce-plan-authoring`.
2. Pass `$ARGUMENTS` without inventing requirements.
3. Let the skill resolve new-versus-existing plan, clarification needs, plan shape, and atomic task slicing.
4. When ready, write or update `context/plans/{plan_name}.md`.
5. Return the exact path, ordered task list, and `/next-task {plan_name} T01`.
6. Stop after the planning handoff.

## Guardrails
- Keep this command thin; do not duplicate the skill's planning rules.
- Do not modify application code or imply implementation approval.
- Do not bypass the clarification gate.

## Outputs
- A plan path and complete ordered task list when planning succeeds.
- Focused clarification questions when planning is blocked.
- One canonical next command for a new implementation session.

## Completion criteria
- `sce-plan-authoring` reports a valid plan and the plan file exists at the reported path.
- The response includes the full task order and stops before implementation.

## Failure handling
- Stop and surface the skill's focused questions when critical information is missing.
- Report path or write failures directly; do not claim a plan was saved when it was not.

## Related units
- `sce-plan-authoring` — sole owner of detailed planning behavior.
- `Shared Context Plan` — default agent for this command.
- `/next-task` — canonical next entrypoint after plan approval.
