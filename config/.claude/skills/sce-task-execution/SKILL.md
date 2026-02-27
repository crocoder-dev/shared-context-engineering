---
name: sce-task-execution
description: Use when user wants to Execute one approved task with explicit scope, evidence, and status updates.
compatibility: claude
---

## Scope rule
- Execute exactly one task per session by default.
- If multi-task execution is requested, confirm explicit human approval.

## Required sequence
1) Restate task goal, boundaries, done checks, and expected file touch scope.
2) Propose approach, trade-offs, and risks.
3) Ask for explicit permission before implementation.
4) Implement minimal in-scope changes.
5) Run light task-level tests/checks and lints first, and run a build when the build is light/fast (targeted over full-suite unless requested), then capture evidence.
6) Keep session-only scraps in `context/tmp/`.
7) Update task status in `context/plans/{plan_id}.md`.

## Scope expansion rule
- If out-of-scope edits are needed, stop and ask for approval.
