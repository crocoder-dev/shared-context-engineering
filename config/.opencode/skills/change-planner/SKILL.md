---
name: change-planner
description: Use when user wants to Review change intent and create an implementation plan in context/plans
compatibility: opencode
metadata:
  owner: Shared Context
---

## What I do
- Run pre-plan discovery when the user has a change idea but no plan yet.
- Treat any change request that includes both a description and success criteria as a required planning trigger.
- Read existing `context/` knowledge using the `Shared Context` agent.
- Ask focused questions for anything not clear enough to proceed.
- Produce a plan file in `context/plans/` with a task stack and `TEST + DONE`.

## How to run this
- Use this skill when the user asks to plan a new change.
- Also use this skill automatically before implementation when a change request includes both a description and success criteria.
- If `context/` is missing, ask once: "`context/` is missing. Bootstrap SCE baseline now?"
  - If yes, create baseline and continue.
  - If no, stop and explain SCE workflows require `context/`.
- Invoke the `Shared Context` agent to gather current context coverage.
- Read `context/context-map.md`, `context/overview.md`, and `context/glossary.md` before broad exploration.
- Ask the user clarification questions when any of these are unclear:
  - requirements or acceptance criteria
  - scope boundaries
  - dependencies or constraints
  - sequencing risks
- Create a plan in `context/plans/<change-name>.md` with this structure:
  1. `Change`
  2. `Task Stack` (checkboxes)
  3. `Open Questions`
  4. `TEST + DONE`
- Decompose the change into explicit tasks (smallest testable delivery units) with clear boundaries and verification notes.
- Keep the default operating model of one task per session unless the user explicitly requests multi-task execution.

## Rules
- Keep all durable SCE artifacts in `context/`.
- Keep planning and continuation state in `context/plans/` plan files.
- Follow SCE workflow order: read context, clarify in chat mode, plan, then implementation.
- If context appears stale or partial, continue with code truth and call out context repairs.
- Do not document behavior, structure, or examples sourced from directories whose names start with `.` (dot-directories).

## Bootstrap baseline
When bootstrapping, create:
- `context/overview.md`
- `context/architecture.md`
- `context/patterns.md`
- `context/glossary.md`
- `context/context-map.md`
- `context/plans/`
- `context/handovers/`
- `context/decisions/`
- `context/tmp/`
- `context/[domain]/` (for example `context/api/`, `context/auth/`, `context/ui/`)
  - `*.md` one focused topic per file
- `context/tmp/.gitignore` with:
  - `*`
  - `!.gitignore`

## Expected output
- A plan file in `context/plans/` ready for `/plan-review` continuation.
- A short list of resolved and unresolved questions captured in the plan.
