---
name: sce-task-execution
description: Use when user wants to Execute one approved task with explicit scope, evidence, and status updates.
compatibility: opencode
---

## Scope rule
- Execute exactly one task per session by default.
- If multi-task execution is requested, confirm explicit human approval.

## Mandatory implementation stop
- Before writing or modifying any code, pause and prompt the user.
- The prompt must explain:
  - task goal
  - boundaries (in/out of scope)
  - done checks
  - expected files/components to change
  - key approach, trade-offs, and risks
- Then ask explicitly whether to continue.
- Do not edit files, generate code, or apply patches until the user confirms.

## Required sequence
1) Restate task goal, boundaries, done checks, and expected file touch scope.
2) Propose approach, trade-offs, and risks.
3) Stop and ask: "Continue with implementation now?" (yes/no).
4) Implement minimal in-scope changes.
5) Run light task-level tests/checks and lints first, and run a build when the build is light/fast (targeted over full-suite unless requested), then capture evidence.
6) Record whether the implementation is an important change for context sync (root-edit required) or verify-only (no root edits expected).
7) Keep session-only scraps in `context/tmp/`.
8) Update task status in `context/plans/{plan_id}.md`.

## Scope expansion rule
- If out-of-scope edits are needed, stop and ask for approval.
