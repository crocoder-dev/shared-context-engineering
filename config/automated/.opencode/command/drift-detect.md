---
description: "Run `sce-drift-analyzer` to report context-code drift before edits"
agent: "Shared Context Drift"
---

Load and follow the `sce-drift-analyzer` skill.

Behavior:
- Collect structured signals from `context/` and code.
- Analyze mismatches between documented and implemented state.
- Save findings to `context/tmp/drift-analysis-YYYY-MM-DD.md`.
- Auto-apply drift fixes to `context/` files without confirmation.
- If code changes would be required, emit report-only with blocker: "Drift requires code changes. Manual intervention required."
- Log all applied fixes to `context/tmp/automated-drift-fixes.md`.
