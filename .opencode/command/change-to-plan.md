---
description: "Create or update an SCE plan from a change request"
agent: "Shared Context Plan"
---

<!-- GENERATED FILE: DO NOT EDIT DIRECTLY. Update canonical sources under config/pkl/ and regenerate. -->

Load and follow the `sce-plan-authoring` skill.

Input change request:
`$ARGUMENTS`

Behavior:
- If `context/` is missing, request bootstrap approval and use `sce-bootstrap-context`.
- Enforce the skill's clarification gate: ask 1-3 targeted questions and pause if any critical detail is unclear.
- Do not create a plan until dependency choices, domain ambiguities, and architecture concerns are explicitly resolved.
- Write/update `context/plans/{plan_name}.md`.
- Confirm plan creation with `{plan_name}` and exact path.
- Return the full ordered task list.
- Prompt user to start a new session to implement `T01` and provide `/next-task {plan_name} T01`.
