---
description: "Review a plan and execute one SCE task from an approved plan"
agent: "Shared Context Code"
---

Load and follow `sce-plan-review`, then `sce-task-execution`, then `sce-context-sync`.

Input:
`$ARGUMENTS`

Expected arguments:
- plan name or plan path (required)
- task ID (`T0X`) (optional)

Behavior:
- Run `sce-plan-review` first to resolve plan target, task selection, bootstrap gating, and clarification questions.
- If both plan and task ID are provided and `sce-plan-review` reports no blockers, ambiguity, or missing acceptance criteria, treat plan-review confirmation as satisfied and continue to `sce-task-execution`.
- If blockers, ambiguity, or missing acceptance criteria are present, resolve open points and ask the user to confirm the task is ready before `sce-task-execution`.
- If either plan or task ID is missing, ask the user to confirm the task is ready before `sce-task-execution`.
- After the confirmation gate passes (auto-pass or explicit user confirmation), start `sce-task-execution` with a mandatory implementation stop before any code edits:
  - explain task goal, boundaries (in/out of scope), done checks, expected files/components, and key approach/trade-offs/risks
  - ask explicitly: "Continue with implementation now?" (yes/no)
  - do not edit files, generate code, or apply patches until user confirms
- After this implementation-stop confirmation passes, continue `sce-task-execution` for scoped implementation, light task-level checks/lints, a build when it is light/fast, and plan status updates.
- Run `sce-context-sync` after implementation as a required done gate to align context files with current code truth.
- Treat context sync as mandatory, not optional: explicitly verify `context/overview.md`, `context/architecture.md`, and `context/glossary.md`, and update them when relevant.
- Ensure new feature existence is documented in durable context (domain file or `context/overview.md`) and linked from `context/context-map.md`.
- Wait for user feedback; if feedback requires in-scope fixes, apply fixes, rerun light checks/lints, run a build when it is light/fast, and run `sce-context-sync` again.
- If this is the final task in the plan, run `sce-validation`.
- When you are finished, if there are more tasks in the plan prompt user to start a new session to implement next task `T0X` and provide `/next-task {plan_name} T0X`.
