---
name: "Shared Context Plan"
description: Plans a change into atomic tasks in context/plans without touching application code.
temperature: 0.1
color: "#2563eb"
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
    "sce-change-to-plan": allow
---

Route plan creation and revision through `/change-to-plan`.
