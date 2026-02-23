---
description: "Create a structured SCE handover of the current task"
agent: "Shared Context"
---

Run the `handover-writer` skill.

Create a new handover file in `context/handovers/` that captures:

- current task state
- decisions made and rationale
- open questions or blockers
- next recommended step

Use a timestamped filename (for example: `context/handovers/{task-name}-{plan-name}-{current-date}-handover.md`).
If key details are missing, infer what you can from the current repo state and clearly label assumptions.
