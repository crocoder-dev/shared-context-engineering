---
description: "Write a session handover document, or load one for continuation"
argument-hint: "[context/handovers/<file>.md]"
agent: "Shared Context Code"
entry-skill: "sce-handover"
skills:
  - "sce-handover"
---

Invoke the `sce-handover` skill exactly once with `$ARGUMENTS`.
The skill owns the complete workflow, including all waits and same-session resume
behavior. Do not invoke any phase skill or sequence workflow steps in this command.
