---
name: sce-bootstrap-context
description: |
  Use when user wants to Bootstrap SCE baseline context directory when missing.
compatibility: opencode
---

## When to use
- Use only when `context/` is missing.
- Automated profile does not support auto-bootstrap; stop with error requiring manual bootstrap.

## Required baseline
Create these paths:
- `context/overview.md`
- `context/architecture.md`
- `context/patterns.md`
- `context/glossary.md`
- `context/context-map.md`
- `context/plans/`
- `context/handovers/`
- `context/decisions/`
- `context/tmp/`
- `context/tmp/.gitignore`

`context/tmp/.gitignore` content:
```
*
!.gitignore
```

## No-code bootstrap rule
- If the repository has no application code, keep `overview.md`, `architecture.md`, `patterns.md`, and `glossary.md` empty or placeholder-only.
- Do not invent implementation details.

## After bootstrapping
- Add baseline links in `context/context-map.md`.
- Tell the user that `context/` should be committed as shared memory.
