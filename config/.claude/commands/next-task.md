---
description: "Review a plan and execute one SCE task from an approved plan"
allowed-tools: Task, Read, Glob, Grep, Edit, Write, Question, Skill, Bash
---

Use the `shared-context-code` agent, then load and follow `sce-plan-review`, `sce-task-execution`, and `sce-context-sync` in order.

Input:
`$ARGUMENTS`

Expected arguments:
- plan name or plan path (required)
- task ID (`T0X`) (optional)

Behavior:
- Run `sce-plan-review` first to resolve plan target, task selection, bootstrap gating, and clarification questions.
- Ask the user to confirm the task is ready for implementation.
- After user confirmation, run `sce-task-execution` for approval, scoped implementation, light task-level checks/lints, a build when it is light/fast, and plan status updates.
- Run `sce-context-sync` after implementation to align context files with current code truth.
- Wait for user feedback; if feedback requires in-scope fixes, apply fixes, rerun light checks/lints, run a build when it is light/fast, and run `sce-context-sync` again.
- If this is the final task in the plan, run `sce-validation`.
