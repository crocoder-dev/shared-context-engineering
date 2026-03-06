---
name: sce-handover-writer
description: Use when user wants to create a structured SCE handover for the current task.
compatibility: opencode
---

## What I do
- Create a new handover file in `context/handovers/`.
- Capture:
  - current task state
  - decisions made and rationale
  - open questions or blockers
  - next recommended step

## How to run this
- Prefer task-aligned naming: `context/handovers/{plan_name}-{task_id}.md`.
- If key details are missing, infer from repo state and clearly label assumptions.

## Expected output
- A complete handover document in `context/handovers/` using task-aligned naming when possible.
