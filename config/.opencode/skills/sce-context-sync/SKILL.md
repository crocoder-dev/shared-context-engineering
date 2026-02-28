---
name: sce-context-sync
description: Use when user wants to Synchronize context files to match current code behavior after task execution.
compatibility: opencode
---

## Principle
- Context is durable AI memory and must reflect current-state truth.
- If context and code diverge, code is source of truth.

## What to update when relevant
- `context/overview.md` for system-level behavior changes
- `context/architecture.md` for boundary or flow changes
- Domain files under `context/{domain}/` for detailed behavior changes
- `context/patterns.md` for newly established implementation patterns
- `context/glossary.md` for new domain terms
- `context/context-map.md` for new or moved context files

## Quality constraints
- Keep one topic per file.
- Prefer concise current-state documentation over narrative changelogs.
- Link related context files with relative paths.
- Include concrete code examples when needed to clarify non-trivial behavior.
- Use Mermaid when a diagram is needed.
- Keep files under 250 lines; split and link when needed.
- Ensure major code areas have matching context coverage.
