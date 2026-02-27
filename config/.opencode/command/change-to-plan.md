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
- Return the full ordered task list and handoff to Shared Context Code.
