---
name: sce-drift-analyzer
description: Use when user wants to analyze drift between context and code using structured collectors.
compatibility: opencode
---

## What I do
- Collect context and code signals with pure JavaScript collectors.
- Analyze semantic drift between documented state and implemented state.
- Produce a clear drift report with actionable fixes.
- Auto-apply drift fixes to `context/` files without confirmation.

## How to run this
- If `context/` is missing, stop with error: "Automated profile requires existing context/. Run manual bootstrap first."
- Collect data:

```javascript
const collectors = require("../../lib/drift-collectors.js");
const data = await collectors.collectAll(process.cwd(), {
  sources: ["context", "code"],
});
```

- Analyze for these drift classes:
  - missing documentation (code capability not represented in `context/`)
  - outdated context (context claim no longer matches code)
  - structure drift (paths and boundaries changed)
  - completion drift (checked tasks with no supporting implementation)
- Write findings to `context/tmp/drift-analysis-YYYY-MM-DD.md`.
- Auto-apply drift fixes to `context/` files without confirmation.
- If code changes would be required, emit report-only with blocker: "Drift requires code changes. Manual intervention required."
- Log all applied fixes to `context/tmp/automated-drift-fixes.md`.

## Rules
- Treat code as source of truth when context and code disagree.
- Keep findings concrete with file-level evidence.
- Keep recommendations scoped and directly actionable.
- Auto-apply context-only fixes without confirmation.

## Expected output
- Drift report in `context/tmp/`.
- Prioritized action list with exact context files to update.
- Applied fixes logged to `context/tmp/automated-drift-fixes.md`.
