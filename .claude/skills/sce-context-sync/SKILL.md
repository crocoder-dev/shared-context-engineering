---
name: sce-context-sync
description: |
  Use when user wants to update project documentation to reflect code changes, sync docs with code, refresh project context, or keep AI memory files accurate after completing an implementation task. Scans modified code, classifies the change significance, then updates or verifies Markdown context files under `context/` (overview, architecture, glossary, patterns, context-map, and domain files) so that durable AI memory stays aligned with current code truth.
compatibility: claude
---

## Purpose
- Reconcile durable SCE context with implemented code so future sessions read current-state truth.

## Inputs
- The completed task, modified files, resulting behavior, plan state, and verification evidence.
- Existing root and domain context files.

## Preconditions
1. Read the affected code and treat it as source of truth.
2. Classify the change as root-impacting or verify-only before editing root context.

## Workflow
1. Classify significance: root edits required for cross-cutting behavior, repository policy, architecture boundaries, or canonical terminology; otherwise verify-only.
2. Verify `context/overview.md`, `context/architecture.md`, and `context/glossary.md` against code truth in every sync pass.
3. Update relevant root files only for root-impacting changes.
4. Create or update focused `context/{domain}/` files for feature-specific behavior.
5. Ensure every newly implemented feature has a durable canonical description discoverable from context.
6. Add or refresh links in `context/context-map.md`.
7. Add glossary entries for new canonical domain language.
8. Verify file length, one-topic focus, relative links, and diagrams where needed.

## Guardrails
- Do not write changelog-style completion narratives into core context.
- Do not edit root files merely to prove a sync occurred.
- Keep one topic per file and each context file at or below 250 lines.
- Split oversized detail into focused domain files and link them.
- Treat completed plans as disposable; preserve durable outcomes elsewhere.

## Outputs
- A significance classification.
- Updated or verified root context.
- Updated domain context and context-map links when needed.
- A concise sync report listing changed and verified files.

## Completion criteria
- Code and context express the same current behavior and terminology.
- Every new feature is discoverable through `context/context-map.md`.
- No context quality constraint is violated.

## Failure handling
- Report unresolved code/context contradictions and the authoritative code evidence.
- Stop before deleting a context file with uncommitted changes.
- Report broken links, oversized files, or missing feature coverage as sync blockers.

## Related units
- `sce-task-execution` — supplies implemented change and significance hint.
- `sce-validation` — confirms final context alignment.
- `/next-task` — treats this skill as a mandatory done gate.

## Reference
Classify root-context impact with this rule:

| Root edits required | Verify-only |
| --- | --- |
| Cross-cutting behavior, repository-wide policy, architecture boundaries, or canonical terminology changes | Localized feature or bug fix with no root-level behavior, architecture, or terminology impact |

Use `context/{domain}/` for feature-specific detail. Keep every context file at or below 250 lines, use one topic per file, use relative links, and add discoverability links to `context/context-map.md`.
