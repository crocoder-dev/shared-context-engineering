---
description: "Review a plan and prepare the next task"
agent: "Shared Context"
---

Run the `next-task-planner` skill.

Behavior:
- Use when the user wants to continue an existing plan.
- Read the specified plan file under `context/plans/`.
- If no plan path is provided and multiple plans exist, ask the user to choose.
- Set next task to the first unchecked checkbox (`- [ ]`).
- Ask focused questions for any unclear detail before implementation.
