---
name: sce-drift-analyzer
description: Use when user wants to analyze drift between context and code using structured collectors.
compatibility: opencode
---

## What I do
- Collect context and code signals with pure JavaScript collectors.
- Analyze semantic drift between documented state and implemented state.
- Produce a clear drift report with actionable fixes.
- Ask the user what to do next before making edits.

## How to run this
- If `context/` is missing, ask once whether to bootstrap SCE baseline.
  - If yes, create baseline and continue.
  - If no, stop and explain drift analysis requires `context/`.
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
- Ask user: "Apply these fixes?" with options:
  - Yes, apply all
  - Selectively
  - No, document only

## Rules
- Treat code as source of truth when context and code disagree.
- Keep findings concrete with file-level evidence.
- Keep recommendations scoped and directly actionable.
- Do not apply edits until user confirms.

## Expected output
- Drift report in `context/tmp/`.
- Prioritized action list with exact context files to update.
