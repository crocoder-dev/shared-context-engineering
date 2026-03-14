---
name: sce-task-execution
 description: Executes a single scoped coding task with structured implementation logging, test and lint evidence capture, and plan status updates. Use when the user wants to run, implement, or complete one approved task from a plan — such as writing code changes, applying fixes, or executing a defined unit of work — while enforcing strict single-task scope boundaries, logging intent to context/tmp/automated-session-log.md, and updating task status in context/plans/{plan_id}.md. Triggers on phrases like "execute task", "run this task", "implement task", "do the next task", or "apply this change".
compatibility: opencode
---

## Scope rule
- Execute exactly one task per session.
- Multi-task execution is not supported in automated profile; if requested, stop with error: "Automated profile does not support multi-task execution. Use single-task handoffs."

## Mandatory implementation stop (auto-proceed with logging)
- Before writing or modifying any code, log implementation intent to `context/tmp/automated-session-log.md`.
- The log must include:
  - task goal
  - boundaries (in/out of scope)
  - done checks
  - expected files/components to change
  - key approach, trade-offs, and risks
- Proceed without waiting for confirmation.
- Preserve all safety constraints (one-task, no scope expansion, no plan reordering).

## Log format
```
## [timestamp] T0X: {task_title}
- Goal: {goal}
- In scope: {in_scope}
- Out of scope: {out_of_scope}
- Expected files: {file_list}
- Approach: {approach_summary}
- Status: proceeding
```

## Required sequence
1) Restate task goal, boundaries, done checks, and expected file touch scope.
2) Propose approach, trade-offs, and risks.
3) Log implementation intent and proceed without waiting for confirmation.
4) Implement minimal in-scope changes.
5) Run light task-level tests/checks and lints first, and run a build when the build is light/fast (targeted over full-suite unless requested), then capture evidence.
6) Record whether the implementation is an important change for context sync (root-edit required) or verify-only (no root edits expected).
7) Keep session-only scraps in `context/tmp/`.
8) Update task status in `context/plans/{plan_id}.md`.

## Scope expansion rule
- If out-of-scope edits are needed, stop immediately with structured error: `BLOCKER: scope_expansion_required`.
- List specific out-of-scope items detected.
- Require human session to approve scope change or split task.
