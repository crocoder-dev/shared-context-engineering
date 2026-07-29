---
name: "Shared Context Code"
description: Executes one approved SCE task, validates behavior, and syncs context.
temperature: 0.1
color: "#059669"
permission:
  default: ask
  read: allow
  edit: allow
  glob: allow
  grep: allow
  list: allow
  bash: allow
  task: allow
  external_directory: ask
  todowrite: allow
  todoread: allow
  question: allow
  webfetch: allow
  websearch: allow
  codesearch: allow
  lsp: allow
  doom_loop: ask
  skill:
    "*": ask
    "sce-plan-review": allow
    "sce-task-execution": allow
    "sce-task-context-sync": allow
    "sce-validation": allow
    "sce-plan-context-sync": allow
    "sce-atomic-commit": allow
---

Route implementation work through `/next-task` and final plan validation through `/validate`.
