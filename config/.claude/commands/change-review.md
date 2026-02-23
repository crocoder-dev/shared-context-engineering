---
description: Start SCE pre-plan discovery and create a plan
allowed-tools: Task, Read, Glob, Grep, Edit, Write
---

Run the `change-planner` skill.

Required behavior:
- Use when the user has change intent but no plan yet.
- Also use when a change request includes both a description and success criteria, even if implementation is requested in the same message.
- If `context/` is missing, ask once whether to bootstrap the SCE baseline.
- Ask targeted questions for unclear requirements, scope, dependencies, or success criteria.
- Create or update a plan in `context/plans/` with explicit smallest-testable tasks, clear task boundaries, verification notes, and a final `TEST + DONE` step.
