---
description: "Run `sce-plan-review` -> `sce-task-execution` -> `sce-context-sync` for one approved SCE task"
agent: "Shared Context Code"
entry-skill: "sce-plan-review"
skills:
  - "sce-plan-review"
  - "sce-task-execution"
  - "sce-context-sync"
  - "sce-validation"
---

Load and follow `sce-plan-review`, then `sce-task-execution`, then `sce-context-sync`.

Input:
`$ARGUMENTS`

Expected arguments:
- plan name or plan path (required)
- task ID (`T0X`) (optional)

Behavior:
- Keep this command as thin orchestration; skill-owned review, implementation, validation, and context-sync details stay in the referenced skills.
- Run `sce-plan-review` first to resolve the plan target, choose the task, and report readiness.
- Apply the readiness confirmation gate from `sce-plan-review` before implementation:
  - auto-pass only when both plan + task ID are provided and review reports no blockers, ambiguity, or missing acceptance criteria
  - otherwise resolve the open points and ask the user to confirm the task is ready before continuing
- Run `sce-task-execution` next; keep the mandatory implementation stop, scoped edits, light checks/lints/build, and plan status updates skill-owned.
- After implementation, run `sce-context-sync` as the required done gate and wait for user feedback.
- If feedback requires in-scope fixes, apply the fixes, rerun light checks (and a light/fast build when applicable), then run `sce-context-sync` again.
- If this was the final plan task, run `sce-validation`; otherwise stop after prompting a new session with `/next-task {plan_name} T0X`.
