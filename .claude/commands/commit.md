---
description: "Analyze staged changes and run the regular or explicit bypass commit workflow"
argument-hint: "[oneshot|skip] [commit context]"
allowed-tools: Task, Read, Glob, Grep, Edit, Write, Question, Skill, Bash
---

Invoke the `sce-commit` skill exactly once with `$ARGUMENTS`.
The skill owns the complete workflow, including all waits and same-session resume
behavior. Do not invoke any phase skill or sequence workflow steps in this command.
