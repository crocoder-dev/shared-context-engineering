---
description: "Analyze and report drift between context and code"
allowed-tools: Task, Read, Glob, Grep, Edit, Write, Question, Skill, Bash
---

Load and follow the `sce-drift-analyzer` skill.

Behavior:
- Collect structured signals from `context/` and code.
- Analyze mismatches between documented and implemented state.
- Save findings to `context/tmp/drift-analysis-YYYY-MM-DD.md`.
- Ask user whether to apply fixes or keep report-only output.
