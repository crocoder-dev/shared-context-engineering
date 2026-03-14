---
name: sce-drift-analyzer
 description: Compares project documentation against actual code implementation to identify outdated, missing, or mismatched content — then produces a prioritized report with actionable fixes. Use when the user says docs are out of date, wants to sync documentation with code, suspects the spec no longer matches implementation, notices code comments or context files are stale, or asks whether documentation reflects the current codebase. Third-person: analyzes documentation-vs-implementation alignment using JavaScript collectors, writes a structured drift report, and asks the user before applying any changes.
compatibility: opencode
---

## What I do
- Collect context and code signals with pure JavaScript collectors.
- Compare documented state against implemented state to find mismatches.
- Produce a clear drift report with actionable fixes.
- Ask the user what to do next before making edits.

## How to run this
- If `context/` is missing, ask once whether to bootstrap SCE baseline.
  - If yes, create baseline and continue.
  - If no, stop and explain drift analysis requires `context/`.
- Collect data using standard Node.js APIs:

```javascript
const fs = require("fs");
const path = require("path");
const { execSync } = require("child_process");

const root = process.cwd();

// --- Collect context/ claims, paths, topics, and completed tasks ---
function readContextFiles(contextDir) {
  const results = { claims: [], paths: [], topics: [], completedTasks: [] };
  if (!fs.existsSync(contextDir)) return results;

  const walk = (dir) => fs.readdirSync(dir, { withFileTypes: true }).forEach(entry => {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) return walk(full);
    if (!entry.name.endsWith(".md")) return;
    const text = fs.readFileSync(full, "utf8");
    const lines = text.split("
");
    lines.forEach((line, i) => {
      // Collect quoted file/path claims (e.g. "handled by src/foo.ts")
      const pathClaim = line.match(/"([^"]*\.[a-z]{2,4})"/i);
      if (pathClaim) results.claims.push({ file: full, line: i + 1, raw: line.trim(), ref: pathClaim[1] });
      // Collect explicit path mentions (src/..., lib/..., etc.)
      const pathRef = line.match(/\b(src|lib|app|test|tests|dist)\/[\w\/\.\-]+/);
      if (pathRef) results.paths.push({ file: full, line: i + 1, ref: pathRef[0] });
      // Collect completed task markers
      if (/^\s*-\s*\[x\]/i.test(line)) results.completedTasks.push({ file: full, line: i + 1, raw: line.trim() });
      // Collect section headings as documented topics
      const heading = line.match(/^#{1,3}\s+(.+)/);
      if (heading) results.topics.push(heading[1].trim());
    });
  });
  walk(contextDir);
  return results;
}

// --- Collect exported symbols and module paths from source code ---
function readCodeSymbols(srcDirs) {
  const symbols = []; // { name, file }
  const allPaths = new Set();
  srcDirs.forEach(dir => {
    if (!fs.existsSync(dir)) return;
    const walk = (d) => fs.readdirSync(d, { withFileTypes: true }).forEach(entry => {
      const full = path.join(d, entry.name);
      if (entry.isDirectory()) return walk(full);
      const rel = path.relative(root, full);
      allPaths.add(rel);
      if (!/\.(js|ts|mjs|cjs)$/.test(entry.name)) return;
      const text = fs.readFileSync(full, "utf8");
      // Named exports: export function foo, export const foo, export class Foo
      const exportMatches = text.matchAll(/^export\s+(?:async\s+)?(?:function|const|class|let|var)\s+(\w+)/gm);
      for (const m of exportMatches) symbols.push({ name: m[1], file: rel });
      // module.exports.foo = ... or exports.foo = ...
      const cjsMatches = text.matchAll(/(?:module\.exports|exports)\.(\w+)\s*=/g);
      for (const m of cjsMatches) symbols.push({ name: m[1], file: rel });
    });
  });
  return { symbols, allPaths };
}

const context = readContextFiles(path.join(root, "context"));
const code = readCodeSymbols(["src", "lib", "app"].map(d => path.join(root, d)));
```

- Analyze for these drift classes:

  - **Missing documentation** - a function or module exists in code but has no corresponding entry in `context/`
    ```javascript
    const undocumented = code.symbols.filter(
      sym => !context.topics.some(t => t.toLowerCase().includes(sym.name.toLowerCase()))
    );
    ```

  - **Outdated context** - a path or file referenced in `context/` no longer exists on disk
    ```javascript
    const stale = context.claims.filter(
      claim => {
        const abs = path.resolve(root, claim.ref);
        return !fs.existsSync(abs) && !code.allPaths.has(claim.ref);
      }
    );
    ```

  - **Structure drift** - file paths mentioned in `context/` have changed or moved
    ```javascript
    const moved = context.paths.filter(
      p => !fs.existsSync(path.resolve(root, p.ref)) && !code.allPaths.has(p.ref)
    );
    ```

  - **Completion drift** - tasks marked `[x]` in `context/` reference paths or symbols with no evidence in code
    ```javascript
    const phantom = context.completedTasks.filter(task => {
      const ref = task.raw.match(/\b(src|lib|app)\/[\w\/\.\-]+/);
      if (!ref) return false;
      return !fs.existsSync(path.resolve(root, ref[0])) && !code.allPaths.has(ref[0]);
    });
    ```

- Write findings to `context/tmp/drift-analysis-YYYY-MM-DD.md`.
- Ask user: "Apply these fixes?" with options:
  - Yes, apply all
  - Selectively
  - No, document only

## Expected drift finding format

Each finding in the report should follow this structure:

```
[outdated-context] ARCHITECTURE.md line 12 claims "auth handled by middleware/session.js"
  -> File no longer exists. Auth now lives in src/auth/tokenGuard.ts.
  -> Fix: update ARCHITECTURE.md line 12 to reference src/auth/tokenGuard.ts.

[missing-documentation] src/payments/reconciler.ts exports `reconcileDaily()`
  -> No entry found in context/ for this capability.
  -> Fix: add reconciler capability to context/modules.md.
```

## Rules
- Treat code as source of truth when context and code disagree.
- Keep findings concrete with file-level evidence.
- Keep recommendations scoped and directly actionable.
- Do not apply edits until user confirms.

## Expected output
- Drift report in `context/tmp/`.
- Prioritized action list with exact context files to update.
