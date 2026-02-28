---
name: sce-plan-review
description: Use when user wants to review an existing plan and prepare the next task safely.
compatibility: claude
---

## What I do
- Continue execution from an existing plan in `context/plans/`.
- Read the selected plan and identify the next task from the first unchecked checkbox.
- Ask focused questions for anything not clear enough to execute safely.

## How to run this
- Use this skill when the user asks to continue a plan or pick the next task.
- If `context/` is missing, ask once: "`context/` is missing. Bootstrap SCE baseline now?"
  - If yes, create baseline with `sce-bootstrap-context` and continue.
  - If no, stop and explain SCE workflows require `context/`.
- Read `context/context-map.md`, `context/overview.md`, and `context/glossary.md` before broad exploration.
- Resolve plan target:
  - If plan path argument exists, use it.
  - If multiple plans exist and no explicit path is provided, ask user to choose.
- Collect:
  - completed tasks
  - next task
  - blockers, ambiguity, and missing acceptance criteria
- Prompt user to resolve unclear points before implementation.
- Confirm scope explicitly for this session: one task by default unless user requests multi-task execution.

## Rules
- Do not auto-mark tasks complete during review.
- Keep continuation state in the plan markdown itself.
- Keep implementation blocked until decision alignment on unclear points.
- If plan context is stale or partial, continue with code truth and flag context updates.

## Expected output
- Confirmed next task with clarified acceptance criteria.
- Explicit readiness verdict: `ready_for_implementation: yes|no`.
- If not ready, explicit issue categories: blockers, ambiguity, missing acceptance criteria.
- Explicit user-aligned decisions needed to proceed to implementation.
- Explicit user confirmation request that the task is ready for implementation when unresolved issues remain.
