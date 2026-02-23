---
description: "Start SCE pre-plan discovery and create a plan"
agent: "Shared Context"
---

Run the `change-planner` skill.

Behavior:
- Use when the user has change intent but no plan yet.
- If `context/` is missing, ask once whether to bootstrap the SCE baseline.
- Ask targeted questions for unclear requirements, scope, dependencies, or success criteria.
- Create a plan in `context/plans/` with checkbox tasks and a final `TEST + DONE` step.
