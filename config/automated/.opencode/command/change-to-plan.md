---
description: "Create or update an SCE plan from a change request"
agent: "Shared Context Plan"
---

Load and follow the `sce-plan-authoring` skill.

Input change request:
`$ARGUMENTS`

Behavior:
- Keep this command as thin orchestration; delegate clarification/ambiguity handling and plan-shape contracts to `sce-plan-authoring`.
- If any critical detail is unclear, stop with structured error listing all unresolved items with category labels.
- Do not invent assumptions silently.
- Ensure plan output follows one-task/one-atomic-commit slicing through `sce-plan-authoring` task-shape rules.
- Write/update `context/plans/{plan_name}.md`.
- Confirm plan creation with `{plan_name}` and exact path.
- Return the full ordered task list.
- Prompt user to start a new session to implement `T01` and provide `/next-task {plan_name} T01`.
