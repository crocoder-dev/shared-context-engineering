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
- Run `sce-plan-review` first to resolve plan target/task and readiness.
- Confirmation gate before execution:
  - if plan + task ID are provided and plan review reports no blockers/ambiguity/missing acceptance criteria, auto-pass readiness
  - otherwise, resolve open points and ask the user to confirm the task is ready before continuing
- After readiness passes, run `sce-task-execution` and enforce its mandatory implementation stop before any edits.
- After user confirms that implementation stop, continue `sce-task-execution` for scoped implementation, checks/lints/build (as applicable), and plan task status updates.
- Run `sce-context-sync` as a required done gate; keep `context/` aligned with code truth, including required shared-file verification and feature discoverability links.
- Wait for user feedback; if in-scope fixes are requested, apply fixes, rerun light checks (and a light/fast build when applicable), then run `sce-context-sync` again.
- If this is the final plan task, run `sce-validation`.
- If more tasks remain, prompt a new session with `/next-task {plan_name} T0X`.
