---
description: "Create a structured SCE handover of the current task"
allowed-tools: Task, Read, Glob, Grep, Edit, Write, Question, Skill
---

Load and follow the `sce-handover-writer` skill.

Input:
`$ARGUMENTS`

Create a new handover file in `context/handovers/` that captures:

- current task state
- decisions made and rationale
- open questions or blockers
- next recommended step

Default naming should align with task execution handovers: `context/handovers/{plan_name}-{task_id}-{timestamp}.md`.
If key details are missing, infer what you can from the current repo state and clearly label assumptions.
