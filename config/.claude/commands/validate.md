---
description: "Run final validation and cleanup for an SCE plan"
allowed-tools: Task, Read, Glob, Grep, Edit, Write, Question, Skill, Bash
---

Load and follow the `sce-validation` skill.

Input:
`$ARGUMENTS`

Behavior:
- Run full validation checks.
- Capture evidence.
- Report pass/fail and any residual risks.
