---
name: sce-plan-review
description: Use when user wants to review an existing plan and prepare the next task safely.
compatibility: opencode
---

## What I do
- Continue execution from an existing plan in `context/plans/`.
- Read the selected plan and identify the next task from the first unchecked checkbox.
- Stop with structured error for anything not clear enough to execute safely.

## How to run this
- Use this skill when the user asks to continue a plan or pick the next task.
- If `context/` is missing, stop with error: "Automated profile requires existing context/. Run manual bootstrap first."
- Read `context/context-map.md`, `context/overview.md`, and `context/glossary.md` before broad exploration.
- Resolve plan target:
  - If plan path argument exists, use it.
  - If no plan path specified and multiple plans exist, stop with error listing available plans and requiring explicit plan path.
  - If no plan path specified and single plan exists, auto-select the single plan.
- Collect:
  - completed tasks
  - next task
  - blockers, ambiguity, and missing acceptance criteria
- If any blockers, ambiguity, or missing acceptance criteria exist, stop with structured error listing all unresolved items with category labels.
- Confirm scope explicitly for this session: one task only (multi-task execution not supported in automated profile).

## Rules
- Do not auto-mark tasks complete during review.
- Keep continuation state in the plan markdown itself.
- Treat `context/plans/` as active execution artifacts; completed plans are disposable and not a durable context source.
- If durable history is needed, record it in current-state context files and/or `context/decisions/` instead of completed plan files.
- Keep implementation blocked until all issues are resolved.
- If plan context is stale or partial, continue with code truth and flag context updates.

## Expected output
- Confirmed next task with clarified acceptance criteria.
- Explicit readiness verdict: `ready_for_implementation: yes|no`.
- If not ready, explicit issue categories: blockers, ambiguity, missing acceptance criteria.
- Auto-proceed to implementation when readiness conditions are met.
