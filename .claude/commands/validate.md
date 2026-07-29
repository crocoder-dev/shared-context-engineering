---
description: "Validate one completed SCE plan and synchronize its durable context"
argument-hint: "<plan-name>"
allowed-tools: Task, Read, Glob, Grep, Edit, Write, Question, Skill, Bash
---

Invoke the `sce-validate` skill exactly once with `$ARGUMENTS`.
The skill owns the complete workflow, including all waits and same-session resume
behavior. Do not invoke any phase skill or sequence workflow steps in this command.
