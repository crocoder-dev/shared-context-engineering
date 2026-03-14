---
description: "Use `sce-plan-authoring` to turn a change request into a scoped SCE plan"
allowed-tools: Task, Read, Glob, Grep, Edit, Write, Question, Skill
---

Load and follow the `sce-plan-authoring` skill.

Input change request:
`$ARGUMENTS`

Behavior:
- Keep this command as thin orchestration; detailed clarification handling, plan-shape rules, and task-writing behavior stay owned by `sce-plan-authoring`.
- Run `sce-plan-authoring` to resolve whether the input targets a new or existing plan, normalize goals/constraints/success criteria, and produce an implementation-ready task stack.
- Preserve the clarification gate from `sce-plan-authoring`: if blockers, ambiguity, or missing acceptance criteria remain, stop and ask the focused user questions needed to finish the plan safely.
- Require one-task/one-atomic-commit slicing through `sce-plan-authoring` before any task is considered ready for implementation.
- When the plan is ready, write or update `context/plans/{plan_name}.md`, confirm the resolved `{plan_name}` and exact path, and return the ordered task list.
- Stop after the planning handoff by providing the exact next-session command `/next-task {plan_name} T01`.
