---
name: sce-bootstrap-context
description: |
  Creates the SCE (Shared Context Engineering) baseline `context/` directory structure — a set of markdown files and sub-folders used as shared project memory (overview, architecture, patterns, glossary, decisions, plans, handovers, and a temporary scratch space). Use when the `context/` folder is missing from the repository, when a user asks to initialise the project context, set up context, create baseline documentation structure, or when shared configuration files for project memory are absent.
compatibility: opencode
---

## When to use
- Use only when `context/` is missing.
- Ask for human approval before creating files.

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

Use the following commands to create the directory structure:
```bash
mkdir -p context/plans context/handovers context/decisions context/tmp
touch context/overview.md context/architecture.md context/patterns.md context/glossary.md context/context-map.md
```

`context/tmp/.gitignore` content:
```
*
!.gitignore
```

## Validation
After running the commands, verify all expected paths exist before proceeding:
```bash
ls context/overview.md context/architecture.md context/patterns.md context/glossary.md context/context-map.md context/plans context/handovers context/decisions context/tmp context/tmp/.gitignore
```
If any path is missing, re-create it before moving on.

## No-code bootstrap rule
- If the repository has no application code, keep `overview.md`, `architecture.md`, `patterns.md`, and `glossary.md` empty or placeholder-only.
- Do not invent implementation details.

Example placeholder content for empty files in a no-code repo:
```markdown
# Overview

> This section has not been populated yet. Add a high-level description of the project here.
```

## After bootstrapping
- Add baseline links in `context/context-map.md`.
- Tell the user that `context/` should be committed as shared memory.
