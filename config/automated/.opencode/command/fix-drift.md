---
description: "Run `sce-drift-fixer` to repair context files from current code truth"
agent: "Shared Context Drift"
---

Load and follow the `sce-drift-fixer` skill.

Audit the `context/` and ensure it correctly describes the system as implemented

- treat code as authoritative
- summarize each discrepancy clearly
- propose exact context updates
- auto-apply updates to `context/` files without confirmation
- if code changes would be required, emit report-only with blocker: "Drift requires code changes. Manual intervention required."
- log all applied fixes to `context/tmp/automated-drift-fixes.md`

Make updates directly in `context/` and keep files concise, current-state oriented, and linked from `context/context-map.md` when relevant.
