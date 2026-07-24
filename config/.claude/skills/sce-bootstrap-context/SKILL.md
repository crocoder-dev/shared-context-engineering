---
name: sce-bootstrap-context
description: |
  Creates the SCE (Shared Context Engineering) baseline `context/` directory structure — a set of markdown files and sub-folders used as shared project memory (overview, architecture, patterns, glossary, decisions, plans, handovers, and a temporary scratch space). Use when the `context/` folder is missing from the repository, when a user asks to initialise the project context, set up context, create baseline documentation structure, or when shared configuration files for project memory are absent.
compatibility: claude
---

## Purpose
- Create the baseline SCE `context/` directory and files when they are absent.

## Inputs
- Repository root.
- Explicit human approval to bootstrap.
- Whether the repository currently contains application code.

## Preconditions
1. Confirm that `context/` is missing.
2. Obtain explicit human approval before creating any path.

## Workflow
1. Create `context/plans/`, `context/handovers/`, `context/decisions/`, and `context/tmp/`.
2. Create `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, and `context/context-map.md`.
3. Write `context/tmp/.gitignore` with `*` followed by `!.gitignore`.
4. When the repository has no application code, keep root context files empty or placeholder-only.
5. Add baseline discoverability links to `context/context-map.md`.
6. Verify every required path exists.
7. Tell the user that `context/` should be committed as shared project memory.

## Guardrails
- Do not overwrite an existing `context/` tree.
- Do not invent architecture, behavior, patterns, or terminology for a no-code repository.
- Limit writes to the approved baseline paths.

## Outputs
- A verified baseline `context/` tree.
- A concise report listing created paths and any placeholders used.

## Completion criteria
- Every required file and directory exists.
- `context/tmp/.gitignore` preserves only itself.
- `context/context-map.md` exposes the baseline files.

## Failure handling
- Stop when approval is not granted.
- Report any path that could not be created or verified; do not continue into planning with a partial baseline.

## Related units
- `Shared Context Plan` — invokes this skill when planning starts without `context/`.
- `sce-plan-authoring` — begins only after a valid baseline exists.

## Reference
Required paths:

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
