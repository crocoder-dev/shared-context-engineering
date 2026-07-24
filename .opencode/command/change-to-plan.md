---
description: "Use `sce-plan-authoring` to turn a change request into a scoped SCE plan"
agent: "Shared Context Plan"
subtask: false
entry-skill: "sce-plan-authoring"
skills:
  - "sce-plan-authoring"
permission:
  default: ask
  read: allow
  edit: allow
  glob: allow
  grep: allow
  list: allow
  bash: deny
  question: allow
  codesearch: allow
  lsp: allow
  skill:
    "*": ask
    "sce-plan-authoring": allow
---

## Purpose
- Turn `$ARGUMENTS` into a scoped SCE implementation plan by delegating to `sce-plan-authoring`.
- Provide a planning handoff without beginning implementation.

## Inputs
- `$ARGUMENTS`: a change request and optional existing plan identifier.
- Any success criteria, constraints, non-goals, dependency choices, and acceptance signals included by the user.

## Preconditions
1. `$ARGUMENTS` supplies a change request that `sce-plan-authoring` can resolve into a plan.

## Workflow
1. Load `sce-plan-authoring`.
2. Pass `$ARGUMENTS` without inventing requirements; when critical requirements are missing, surface the skill's focused clarification questions and stop before writing.
3. Let the skill resolve new-versus-existing plan, plan shape, and atomic task slicing.
4. When ready, write or update `context/plans/{plan_name}.md`.
5. Return the planning handoff and stop.

## Guardrails
- Keep this command thin; do not duplicate the skill's planning rules.
- Do not modify application code or imply implementation approval.
- Do not bypass the clarification gate.

## Outputs
- The plan path and complete ordered task list when planning succeeds.
- One canonical `/next-task {plan_name} T01` command for a new implementation session.
- Focused clarification questions instead of a plan when planning is blocked.

## Completion criteria
- `sce-plan-authoring` reports a valid plan and the plan file exists at the reported path.

## Failure handling
- Report plan-write or validation failures directly; do not claim a plan was saved when it was not.

## Related units
- `shared-context-plan` — execution profile bound to this workflow.
- `sce-plan-authoring` — entry skill for this workflow.
- `/next-task` — canonical next entrypoint after plan approval.
