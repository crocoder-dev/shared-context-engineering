---
description: "Analyze and report drift between context and code"
agent: "Shared Context"
---

Run the `drift-analyzer` skill.

Behavior:
- Collect structured signals from `context/` and code.
- Analyze mismatches between documented and implemented state.
- Save findings to `context/tmp/drift-analysis-YYYY-MM-DD.md`.
- Ask user whether to apply fixes or keep report-only output.
