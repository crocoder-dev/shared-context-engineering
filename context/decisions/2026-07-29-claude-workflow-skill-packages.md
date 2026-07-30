# Decision: Collapse Claude Workflows into Single-Skill Packages

Date: 2026-07-29
Status: Accepted (Claude-only scope superseded 2026-07-29)
Plan: `context/plans/claude-workflow-single-skill-packages.md`
Supersedes in part: `2026-07-27-workflow-oriented-pkl-generation.md`

## Decision

- Keep the four canonical workflow definitions and their phase modules as the behavioral source. (This decision rendered them as eight packages for Pi and OpenCode; `2026-07-29-cross-target-workflow-skill-packages.md` collapsed those targets too, so the phase modules are now authoring inputs only.)
- Render Claude as exactly four thin commands and four workflow-level skills: `sce-change-to-plan`, `sce-next-task`, `sce-validate`, and `sce-commit`.
- Route each Claude command to exactly one corresponding workflow skill. Claude commands do not sequence phase skills.
- Compose each Claude workflow skill from the canonical command, phase behavior, and context-sync policy in `config/pkl/renderers/claude-workflow-results.pkl` (since promoted to the target-neutral `config/pkl/renderers/workflow-composite.pkl`).
- Keep phase statuses and phase-to-phase data as internal workflow state. A Claude workflow skill never invokes another SCE skill or sibling package.
- Emit exactly two files per Claude package: `SKILL.md` and `references/output.md`. The latter is the sole package reference and owns all human-visible gates, reports, and terminal Markdown layouts.
- Preserve Pi and OpenCode generated command behavior, eight-skill inventories, and phase contracts byte-for-byte. (Superseded: those targets were collapsed the same way on 2026-07-29.)

## Rationale

Claude executes a skill as the durable unit of control flow. Returning a phase result through one skill and asking a command to resume another phase creates an unreliable turn boundary: the intermediate result can become the apparent deliverable and terminate the workflow. Putting the complete lifecycle in one skill removes that transport seam while retaining the canonical gates, waits, writes, verification, synchronization, and continuation rules.

The collapse belongs in the renderer rather than the canonical base, so the phase modules remain the reviewable behavioral source. At the time of this decision that also let Pi and OpenCode keep the decomposed workflow and its explicit phase contracts; they were subsequently collapsed for cross-target uniformity.

## Consequences

- Claude no longer emits `sce-context-load`, `sce-plan-authoring`, `sce-plan-review`, `sce-task-execution`, `sce-task-context-sync`, `sce-validation`, `sce-plan-context-sync`, or `sce-atomic-commit` packages.
- Claude has no inter-skill YAML or Markdown phase-result contracts. Internal statuses still mirror canonical variants so behavior remains aligned.
- `config/pkl/renderers/claude-metadata.pkl` owns the exact command-to-workflow-skill slug mapping.
- `config/pkl/renderers/metadata-coverage-check.pkl` rejects stale phase packages, extra references, missing two-file packages, and incorrect command routes.
- Claude's generated workflow inventory is eight Markdown files total across four packages, in addition to four command files and retained settings/hooks.

## Superseded scope

This decision supersedes only the Claude inventory and Claude phase-result transport claims in `2026-07-27-workflow-oriented-pkl-generation.md`. That decision remains authoritative for canonical workflow ownership, the shared context-sync skeleton, thin OpenCode agents, removed automated profiles, and retained non-Markdown outputs.

`2026-07-29-cross-target-workflow-skill-packages.md` supersedes this decision's Claude-only scoping: the clauses preserving Pi/OpenCode phase packages, eight-skill inventories, and phase-result contracts no longer hold. Everything stated here about the composed package shape, the two-file package rule, single-skill command routing, and internal phase statuses now applies to all three targets.
