---
description: Create a structured SCE handover of the current task
allowed-tools: Task, Read, Glob, Grep, Edit, Write
---

Run the `handover-writer` skill.

Use the `shared-context` agent to create a new handover file in `context/handovers/` that captures:

- Current task state
- Decisions made and rationale
- Open questions or blockers
- Next recommended step

Use a timestamped filename (for example: `context/handovers/{task-name}-{plan-name}-{current-date}-handover.md`).
If key details are missing, infer what you can from the current repo state and clearly label assumptions.
