---
name: next-task-planner
description: Use when user wants to Review an existing context/plans file and prepare the next task
compatibility: opencode
metadata:
  owner: Shared Context
---

## What I do
- Continue execution from an existing plan in `context/plans/`.
- Read the selected plan using the `Shared Context` agent.
- Identify the next task from the first unchecked checkbox.
- Ask the user focused questions for anything not clear enough to execute safely.

## How to run this
- Use this skill when the user asks to continue a plan or next tasks.
- If `context/` is missing, ask once: "`context/` is missing. Bootstrap SCE baseline now?"
  - If yes, create baseline and continue.
  - If no, stop and explain SCE workflows require `context/`.
- Read `context/context-map.md`, `context/overview.md`, and `context/glossary.md` before broad exploration.
- Resolve plan target:
  - If plan path argument exists, use it.
  - If multiple plans exist and no explicit path is provided, ask user to choose.
- Invoke `Shared Context` and collect:
  - completed tasks
  - next task
  - blockers, ambiguity, and missing acceptance criteria
- Prompt user to resolve unclear points before implementation.
- Confirm scope explicitly for this session: one task by default unless user requests multi-task execution.

## Rules
- Do not auto-mark tasks complete in review.
- Keep continuation state in the plan markdown itself.
- Keep implementation blocked until decision alignment on unclear points.
- If plan context is stale or partial, continue with code truth and flag context updates.
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
- Confirmed next task with clarified acceptance criteria.
- Explicit user-aligned decisions needed to proceed to implementation.
