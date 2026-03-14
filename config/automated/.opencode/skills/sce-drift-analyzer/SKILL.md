---
name: sce-drift-analyzer
description: "Compares documentation in `context/` files against the actual codebase to detect and fix mismatches. Use when the user notices docs are out of date, code comments are stale, spec no longer matches implementation, or wants to sync documentation with code — e.g. \"my docs are outdated\", \"check if context files match the code\", \"find undocumented capabilities\", \"docs out of date\", \"documentation sync\". Scans for missing documentation, outdated claims, changed paths, and completed tasks with no supporting code. Produces a structured drift report and auto-applies context-only fixes without confirmation."
compatibility: opencode
---

## What I do
- Collect context and code signals with JavaScript collectors.
- Detect mismatches between documented state (`context/`) and implemented state (source code).
- Produce a clear drift report with actionable fixes.
- Auto-apply drift fixes to `context/` files without confirmation.

## How to run this
- If `context/` is missing, stop with error: "Automated profile requires existing context/. Run manual bootstrap first."
- Collect data using the drift collectors module located at `lib/drift-collectors.js` (relative to the project root):

```javascript
const collectors = require("../../lib/drift-collectors.js");
const data = await collectors.collectAll(process.cwd(), {
  sources: ["context", "code"],
});
```

- Analyze for these drift classes using the collected data. For each class, a pseudocode description is followed by a concrete implementation approach:

```
// PSEUDOCODE - translate to concrete file/AST operations for the target project

// Missing documentation:
//   Find all exported functions, classes, and modules in source code.
//   Cross-reference each against mentions in context/ files.
//   Flag any capability with no matching mention.
missingDocs = code.exports.filter(cap => !context.mentions(cap.name))
```

Concrete example - grep-based check for a missing export mention:
```bash
# List all named exports from source files
grep -rE "^export (function|class|const) (\w+)" src/ --include="*.ts" -h \
  | sed -E 's/^export (function|class|const) ([A-Za-z_]+).*/\2/' \
  | sort -u > /tmp/exported_names.txt

# For each name, check whether it appears anywhere in context/
while IFS= read -r name; do
  grep -rl "$name" context/ > /dev/null 2>&1 || echo "MISSING: $name"
done < /tmp/exported_names.txt
```

```
// Outdated context:
//   For each factual claim in context/ (e.g. "AuthService exposes refreshToken()"),
//   verify the referenced identifier still exists at the stated path/signature.
//   Flag claims where the identifier is absent or the signature has changed.
outdatedContext = context.claims.filter(claim =>
  !sourceFileContains(claim.referencedPath, claim.referencedIdentifier)
)

// Structure drift:
//   Extract file paths and module boundaries named in context/ files.
//   Check each against the current directory tree.
//   Flag any path that no longer exists on disk.
structureDrift = context.paths.filter(p => !existsOnDisk(p))

// Completion drift:
//   Extract tasks marked as done/completed in context/ files.
//   For each, search source code for an implementation matching the task description.
//   Flag tasks with no corresponding implementation found.
completionDrift = context.completedTasks.filter(task =>
  !grepSource(task.keywords)
)
```

- Write findings to `context/tmp/drift-analysis-YYYY-MM-DD.md` using this format:

```markdown
## Drift Finding: <DriftClass>
- **File:** `context/architecture.md` (line 12)
- **Claim:** "AuthService exposes a `refreshToken()` method"
- **Reality:** No `refreshToken` found in `src/auth/AuthService.ts`
- **Severity:** High
- **Fix:** Remove or update the claim to match current implementation.
```

- Auto-apply context-only fixes to `context/` files.
- After applying fixes, re-run collectors to verify drift was resolved:

```javascript
const verification = await collectors.collectAll(process.cwd(), {
  sources: ["context", "code"],
});
const remaining = verification.drift.length;
if (remaining > 0) {
  console.warn(`${remaining} drift item(s) could not be auto-resolved.`);
}
```

- If code changes would be required, emit report-only with blocker: "Drift requires code changes. Manual intervention required."
- Log all applied fixes to `context/tmp/automated-drift-fixes.md`.

## Rules
- Treat code as source of truth when context and code disagree.
- Keep findings concrete with file-level evidence.
- Keep recommendations scoped and directly actionable.
- Auto-apply context-only fixes without confirmation.

## Expected output
- Drift report in `context/tmp/drift-analysis-YYYY-MM-DD.md` with one section per finding.
- Prioritized action list with exact context files to update.
- Applied fixes logged to `context/tmp/automated-drift-fixes.md`.
- Verification summary confirming how many drift items were resolved.
