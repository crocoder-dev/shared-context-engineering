---
description: "Review, approve, implement, verify, and synchronize one SCE plan task"
argument-hint: "<plan-name> [T0X] [approved]"
agent: "Shared Context Code"
entry-skill: "sce-next-task"
skills:
  - "sce-next-task"
---

Invoke the `sce-next-task` skill exactly once with `$ARGUMENTS`.
The skill owns the complete workflow, including all waits and same-session resume
behavior. Do not invoke any phase skill or sequence workflow steps in this command.
