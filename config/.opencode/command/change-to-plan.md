---
description: "Create or update an SCE plan from a change request"
agent: "Shared Context Plan"
---

Load and follow the `sce-plan-authoring` skill.

Input change request:
`$ARGUMENTS`

Behavior:
- If `context/` is missing, request bootstrap approval and use `sce-bootstrap-context`.
- Ask targeted clarifying questions only when needed.
- Write/update `context/plans/{plan_name}.md`.
- Confirm plan creation with `{plan_name}` and exact path.
- Return the full ordered task list.
- Prompt user to start a new session to implement `T01` and provide `/next-task {plan_name} T01`.
