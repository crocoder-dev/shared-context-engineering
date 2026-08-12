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
    "*": allow
    "sce-*": deny
    "sce-next-task": allow
    "sce-validate": allow
    "sce-commit": allow
    "sce-handover": allow
    "sce-brownfield": allow
    "sce-decision": allow
---

Route implementation work through `/next-task` and final plan validation through `/validate`.
