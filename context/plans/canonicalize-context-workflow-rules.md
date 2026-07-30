# Plan: canonicalize-context-workflow-rules

## Change summary

Move the durable rules currently supplied in root-level `context-rules.md` into a focused, discoverable document under `context/`. Reconcile its obsolete bootstrap and singular synchronization descriptions with the canonical Pkl workflows, index the resulting document, and remove the stale root copy so there is one current source of durable guidance.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [x] AC1: The bootstrap, maintenance, and synchronization rules from `context-rules.md` have a canonical current-state-oriented home under `context/`, remain at most 250 lines, and are discoverable from `context/context-map.md` through relative links.
  - Validate: Inspect the new context document and `context/context-map.md`; confirm the document covers bootstrap, ongoing invariants, task/plan synchronization, and feature-existence documentation, and run `wc -l <new-context-path>` to confirm the line limit.
- [x] AC2: The durable rules identify `sce setup --bootstrap-context` as the bootstrap boundary and the separately gated `sce-task-context-sync` and `sce-plan-context-sync` skills as the synchronization lifecycle, consistent with the canonical Pkl workflows.
  - Validate: Compare the resulting document against `config/pkl/base/workflow-change-to-plan.pkl`, `config/pkl/base/workflow-next-task.pkl`, `config/pkl/base/workflow-validate.pkl`, and `config/pkl/base/workflow-context-sync.pkl`; confirm no obsolete `sce-bootstrap-context` or singular `sce-context-sync` ownership remains.
- [x] AC3: No conflicting root-level `context-rules.md` remains after its durable content is migrated.
  - Validate: Confirm `context-rules.md` is absent and search repository documentation for stale references to that path or the obsolete workflow names.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- The new canonical rules document under `context/sce/` and `context/context-map.md` are the durable context changed by this plan.
- `context/overview.md`, `context/architecture.md`, `context/glossary.md`, and `context/patterns.md` must remain consistent with the canonical Pkl ownership model; update them only if the mandatory synchronization pass finds a material mismatch.

## Constraints and non-goals

- **In scope:** Migrate and reconcile `context-rules.md` into one focused document under `context/sce/`, update `context/context-map.md`, repair directly affected context links, and remove the superseded root file.
- **Out of scope:** Changes to Pkl workflow behavior, generated skill packages, commands, application code, tests, or unrelated durable context.
- **Constraints:** Treat `config/pkl/base/workflow-change-to-plan.pkl`, `workflow-next-task.pkl`, `workflow-validate.pkl`, and `workflow-context-sync.pkl` as code truth; keep context current-state oriented, focused, at most 250 lines per file, relatively linked, and indexed; do not delete `context-rules.md` if it has uncommitted changes not attributable to this change.
- **Non-goal:** Preserve obsolete skill names or turn the migrated document into a historical account of workflow evolution.

## Assumptions

- Place the canonical document in `context/sce/` because these are cross-workflow SCE rules and the existing workflow lifecycle documentation lives there.
- Migration means removing the superseded root file rather than retaining a duplicate that could drift independently.
- The exact focused filename and map section are reversible local choices and should follow neighboring `context/sce/` naming and indexing conventions.

## Task stack

- [x] T01: `Migrate context workflow rules into durable context` (status:done)
  - Task ID: T01
  - Goal: Establish one canonical, Pkl-aligned context maintenance rules document under `context/sce/` and make it discoverable from the context index.
  - Boundaries (in/out of scope): In — rewrite the durable substance of `context-rules.md` into a focused `context/sce/` document, reconcile bootstrap and synchronization ownership with current Pkl, update `context/context-map.md` and directly affected relative links, and remove the superseded root file when safe. Out — changing workflow implementation, generated assets, tests, or unrelated context.
  - Dependencies: none
  - Done when: The migrated document accurately describes current bootstrap, maintenance, task-sync, and plan-sync rules; it meets context hygiene constraints; the context map links it; stale references and obsolete workflow names are absent from the migrated guidance; and the root source has been safely removed.
  - Verification notes (commands or checks): Inspect the document against the four canonical workflow Pkl modules; run `wc -l <new-context-path>`; search documentation for `context-rules.md`, `sce-bootstrap-context`, and singular `sce-context-sync`; inspect all changed relative links; run `git diff --check`.
  - Completed: 2026-07-27
  - Files changed: `context/sce/context-workflow-rules.md`, `context/context-map.md`, `context/sce/shared-context-plan-workflow.md`, `context/decisions/2026-03-03-plan-code-agent-separation.md`; removed the untracked superseded source `context-rules.md` after migrating its durable content.
  - Evidence: Compared the migrated rules with `config/pkl/base/workflow-change-to-plan.pkl`, `workflow-next-task.pkl`, `workflow-validate.pkl`, and `workflow-context-sync.pkl`; `wc -l context/sce/context-workflow-rules.md` reported 139 lines; a documentation search outside historical plan artifacts found no `context-rules.md`, `sce-bootstrap-context`, or singular `sce-context-sync`; all added relative links resolve; `test ! -e context-rules.md` and `git diff --check` passed.
  - Notes: Updated the directly affected plan-workflow bootstrap continuation and decision record to current canonical ownership. No workflow implementation or generated assets changed.

## Open questions

None. The loaded brief identifies both the missing durable owner and the canonical Pkl sources needed to reconcile the stale rules.

## Validation Report

**Status:** validated  
**Date:** 2026-07-27

### Commands run

- `nix flake check` -> exit 0 (all flake checks passed)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed for 73 files)
- `wc -l context/sce/context-workflow-rules.md && test ! -e context-rules.md && git diff --check` -> exit 0 (canonical rules are 139 lines, the root copy is absent, and the diff is clean)
- `nix run nixpkgs#bun -- -e '<relative-link check>'` -> exit 0 (all changed-document relative links resolve)
- `nix run nixpkgs#ripgrep -- -n --glob '*.md' --glob '!context/plans/**' 'context-rules\\.md|sce-bootstrap-context|`sce-context-sync`' .` -> exit 0 (no stale current-documentation references found)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: The bootstrap, maintenance, and synchronization rules have a canonical current-state-oriented home under `context/`, remain at most 250 lines, and are discoverable from the context map -> inspected `context/sce/context-workflow-rules.md` and `context/context-map.md`; all required subjects are covered, the relative link resolves, and the document is 139 lines.
- [x] AC2: The durable rules name the current bootstrap boundary and separate task/plan synchronization lifecycle -> compared the document with all four canonical Pkl workflow modules; ownership and success gates match `sce setup --bootstrap-context`, `sce-task-context-sync`, and `sce-plan-context-sync`.
- [x] AC3: No conflicting root-level rules file or stale current-documentation reference remains -> the root file absence check and the documentation search both passed; obsolete references remain only in historical plan artifacts.

### Failed checks and follow-ups

- None.

### Residual risks

- Historical active-plan artifacts still mention superseded names as recorded execution history; the canonical current-state context excludes them.
