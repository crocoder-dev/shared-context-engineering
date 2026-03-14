---
name: sce-drift-fixer
 description: Use when the user wants to fix stale or outdated context documentation that no longer matches the actual codebase — e.g. "update docs", "sync context files", "fix outdated documentation", "refresh context", or "context is out of date". Audits files in `context/`, identifies discrepancies between documentation and implemented code (treating code as the source of truth), then updates context files to remove outdated references, correct stale descriptions, and sync documentation with recent code changes.
compatibility: opencode
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
- If no report exists, explicitly tell the user no drift analysis report was found, then run `sce-drift-analyzer` by invoking the `sce-drift-analyzer` skill before continuing.
- Ask whether to apply all fixes or apply selectively.
- If any finding is ambiguous or lacks enough evidence, prompt the user before editing.
- Keep context files concise, current-state oriented, and linked from `context/context-map.md` when relevant.

## Example drift finding and fix

**Finding from `context/tmp/drift-analysis-2024-11-01.md`:**
> `context/architecture.md` line 12 states the auth module uses JWT tokens, but `src/auth/handler.ts` was refactored to use session cookies in commit `a3f92c1`.

**Before (`context/architecture.md`):**
```markdown
## Auth
The auth module issues JWT tokens on login and validates them on each request.
```

**After (`context/architecture.md`):**
```markdown
## Auth
The auth module uses session cookies on login and validates the session on each request.
```

## Expected output
- A clear list of drift findings sourced from `context/tmp/drift-analysis-*.md`.
- Explicit clarification questions for any uncertain drift items.
- Concrete file-level edits in `context/` that resolve selected drift items.
- Verification summary:
  - items resolved
  - context files updated
