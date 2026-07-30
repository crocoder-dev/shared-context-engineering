---
description: "Turn one change request into a scoped SCE plan in one self-contained workflow"
argument-hint: "<describe changes you want to introduce>"
---

Invoke the `sce-change-to-plan` skill exactly once with `$ARGUMENTS`.
The skill owns the complete workflow, including all waits and same-session resume
behavior. Do not invoke any phase skill or sequence workflow steps in this command.
