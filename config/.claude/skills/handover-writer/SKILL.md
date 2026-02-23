---
name: handover-writer
description: Use when user wants to Create a structured Shared Context Engineering handover file for the current task
compatibility: claude
metadata:
  owner: shared-context
---

## What I do
- Create a new handover file in `context/handovers/`.
- Capture:
  - current task state
  - decisions made and rationale
  - open questions or blockers
  - next recommended step

## How to run this
- Ask for user permission before running this skill unless permission has already been granted.
- Use the `shared-context` agent to generate the handover.
- Use a timestamped filename, for example: `context/handovers/{task-name}-{plan-name}-{current-date}-handover.md`.
- If key details are missing, infer from repo state and clearly label assumptions.
- Do not document behavior, structure, or examples sourced from directories whose names start with `.` (dot-directories).

## Expected output
- A complete, timestamped handover document in `context/handovers/`.
