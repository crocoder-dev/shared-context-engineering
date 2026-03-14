---
name: sce-drift-fixer
description: "Use when user wants to audit and repair code-context drift in context/ using SCE rules."
compatibility: opencode
---

## What I do
- Audit `context/` and verify it matches the implemented system.
- Treat code as the source of truth when context and code disagree.
- Summarize drift items with clear evidence.
- Auto-apply updates to `context/` files without confirmation.
- Use existing drift analysis reports from `context/tmp/` as the primary input for fixes.

## How to run this
- If `context/` is missing, stop with error: "Automated profile requires existing context/. Run manual bootstrap first."
- Search `context/tmp/` for `drift-analysis-*.md`.
- If one or more reports exist, use the latest report as the fix input.
- If no report exists, run `sce-drift-analyzer` to generate one before continuing.
- Auto-apply all fixes to `context/` files without confirmation.
- If code changes would be required, emit report-only with blocker: "Drift requires code changes. Manual intervention required."
- Log all applied fixes to `context/tmp/automated-drift-fixes.md`.
- Keep context files concise, current-state oriented, and linked from `context/context-map.md` when relevant.

## Expected output
- A clear list of drift findings sourced from `context/tmp/drift-analysis-*.md`.
- Concrete file-level edits in `context/` that resolve selected drift items.
- Applied fixes logged to `context/tmp/automated-drift-fixes.md`.
- Verification summary:
  - items resolved
  - context files updated
