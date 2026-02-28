---
name: sce-drift-fixer
description: Use when user wants to audit and repair code-context drift in context/ using SCE rules.
compatibility: claude
---

## What I do
- Audit `context/` and verify it matches the implemented system.
- Treat code as the source of truth when context and code disagree.
- Summarize drift items with clear evidence.
- Apply updates once the user confirms, or immediately when already authorized.
- Use existing drift analysis reports from `context/tmp/` as the primary input for fixes.

## How to run this
- If `context/` is missing, ask once whether to bootstrap SCE baseline.
  - If yes, create baseline and continue.
  - If no, stop and explain SCE workflows require `context/`.
- Search `context/tmp/` for `drift-analysis-*.md`.
- If one or more reports exist, use the latest report as the fix input.
- If no report exists, explicitly tell the user no drift analysis report was found, then run `sce-drift-analyzer` to generate one before continuing.
- Ask whether to apply all fixes or apply selectively.
- If any finding is ambiguous or lacks enough evidence, prompt the user before editing.
- Keep context files concise, current-state oriented, and linked from `context/context-map.md` when relevant.
- Do not document behavior, structure, or examples sourced from directories whose names start with `.` (dot-directories).

## Expected output
- A clear list of drift findings sourced from `context/tmp/drift-analysis-*.md`.
- Explicit clarification questions for any uncertain drift items.
- Concrete file-level edits in `context/` that resolve selected drift items.
- Verification summary:
  - items resolved
  - context files updated
