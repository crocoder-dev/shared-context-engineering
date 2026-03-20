---
name: sce-plan-review
description: |
  Use when the user wants to continue, resume, or pick up the next step from a markdown plan file stored in `context/plans/`. Reads the selected plan file, identifies the first unchecked checkbox as the next task, validates that acceptance criteria are clear and blockers are resolved, then issues an explicit readiness verdict before proceeding to implementation. Use when the user says "continue the plan", "what's the next task", "resume work", "pick up where we left off", or references a specific plan file.
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
Emit a readiness verdict using this structure:

```
next_task: "Task title or description from plan"
acceptance_criteria:
  - Criterion A
  - Criterion B
ready_for_implementation: yes | no
```

If `ready_for_implementation: no`, include an issues block:

```
issues:
  blockers:
    - "Dependency on X is unresolved"
  ambiguity:
    - "It is unclear whether Y should be replaced or extended"
  missing_acceptance_criteria:
    - "No definition of done for the migration step"
```

- Auto-proceed to implementation when `ready_for_implementation: yes`.

## Structured error examples

**Multiple plans found (no path specified):**
```
ERROR: Multiple plans found. Specify an explicit plan path.
Available plans:
  - context/plans/migrate-auth.md
  - context/plans/refactor-api.md
```

**Blockers or ambiguity detected:**
```
ERROR: Next task cannot proceed. Unresolved items:
  [blocker] Auth service interface not yet defined - task depends on it.
  [ambiguity] "Update schema" - unclear whether additive or destructive migration.
  [missing_acceptance_criteria] No rollback criteria specified for the deployment step.
Resolve all items above before re-running plan review.
```
