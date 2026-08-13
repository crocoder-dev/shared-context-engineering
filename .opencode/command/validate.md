---
description: "Validate one completed SCE plan and record final validation evidence"
argument-hint: "<plan-name>"
agent: "Shared Context Code"
entry-skill: "sce-validate"
skills:
  - "sce-validate"
---

Invoke the `sce-validate` skill exactly once with `$ARGUMENTS`.
The skill owns the complete workflow, including all waits and same-session resume
behavior. Do not invoke any phase skill or sequence workflow steps in this command.
