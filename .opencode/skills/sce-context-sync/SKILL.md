---
name: sce-context-sync
description: Use when user wants to Synchronize context files to match current code behavior after task execution.
compatibility: opencode
---

## Principle
- Context is durable AI memory and must reflect current-state truth.
- If context and code diverge, code is source of truth.

## Mandatory sync pass (important-change gated)
For every completed implementation task, run a sync pass over these shared files:
- `context/overview.md`
- `context/architecture.md`
- `context/glossary.md`
- `context/patterns.md`
- `context/context-map.md`

Do not default to editing root context files on every task. First classify whether the task is an important change; then edit or verify accordingly.

## Root context significance gating
- Treat root context edits as required when a task introduces or changes cross-cutting behavior, repository-wide policy/contracts, architecture boundaries, or canonical terminology.
- Treat root context edits as verify-only when a task is localized to a single feature/domain and no root-level behavior, architecture, or terminology changed.
- When verify-only applies, keep root files unchanged and capture details in focused domain/workflow files instead.
- When root updates are not needed, still verify `context/overview.md`, `context/architecture.md`, and `context/glossary.md` against code truth before declaring done.

## Domain file creation policy
- Use domain files under `context/{domain}/` for detailed feature behavior.
- If a feature does not cleanly fit an existing domain file, create a new domain file instead of deferring documentation.
- If the feature appears to be part of a larger future domain, still document the implemented slice now in a focused file and link it to related context.
- Prefer creating a small, precise domain file over overloading `overview.md` with detail.
- If updates for the current feature/domain become too detailed or large for shared files, migrate that detail into `context/{domain}/` files and keep only concise pointers in shared files.
- Whenever detail is migrated, add discoverability links in `context/context-map.md` and cross-link relevant context files (`overview.md`, `architecture.md`, `glossary.md`, `patterns.md`) as needed.

## Feature existence rule (required)
- Every newly implemented feature must be discoverable from context.
- Ensure at least one durable canonical description exists in either:
  - a domain file under `context/{domain}/`, or
  - `context/overview.md` (for cross-cutting/system features).
- Ensure discoverability links are present from `context/context-map.md`.
- Add glossary entries for any new domain language.

## Final-task requirement
- In the final plan task (validation/cleanup), confirm feature existence documentation is present and linked.
- If a feature was implemented but not represented in context, add the missing context entry before declaring the task done.

## Quality constraints
- Keep one topic per file.
- Prefer concise current-state documentation over narrative changelogs.
- Link related context files with relative paths.
- Include concrete code examples when needed to clarify non-trivial behavior.
- Every context file you create or update must stay at or below 250 lines; if it would exceed 250, split into focused files and link them.
- Add a Mermaid diagram when structure, boundaries, or flows are complex.
- Ensure major code areas have matching context coverage.
