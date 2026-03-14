---
description: "Run `sce-plan-review` -> `sce-task-execution` -> `sce-context-sync` for one approved SCE task"
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
- Apply readiness confirmation gate from `sce-plan-review`:
  - auto-pass only when both plan + task ID are provided and review reports no blockers/ambiguity/missing acceptance criteria
  - otherwise stop with structured error listing unresolved items
- Run `sce-task-execution`; keep mandatory implementation stop (auto-proceed with logging), scoped implementation, checks/lints/build, and plan status updates skill-owned.
- Run `sce-context-sync` as the required done gate.
- Wait for user feedback; if in-scope fixes are requested, apply fixes, rerun light checks (and a light/fast build when applicable), then run `sce-context-sync` again.
- If this is the final plan task, run `sce-validation`.
- If more tasks remain, prompt a new session with `/next-task {plan_name} T0X`.
