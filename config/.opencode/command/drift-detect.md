---
description: "Run `sce-drift-analyzer` to report context-code drift before edits"
agent: "Shared Context Drift"
entry-skill: "sce-drift-analyzer"
skills:
  - "sce-drift-analyzer"
---

Load and follow the `sce-drift-analyzer` skill.

Behavior:
- Keep this command as thin orchestration; drift detection logic, evidence gathering, and report structure stay owned by `sce-drift-analyzer`.
- Run `sce-drift-analyzer` to compare `context/` against code truth, summarize mismatches, and write the drift report to `context/tmp/drift-analysis-YYYY-MM-DD.md`.
- Stop after the analyzer reports findings; do not apply fixes from this command.
- If the user wants repairs after reviewing the report, direct the next step to `/fix-drift` so update behavior stays owned by `sce-drift-fixer`.
