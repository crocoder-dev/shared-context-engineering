# Decision: Render Every Target's Workflows as Single-Skill Packages

Date: 2026-07-29
Status: Accepted
Plan: `context/plans/pi-opencode-workflow-single-skill-packages.md`
Supersedes in part: `2026-07-27-workflow-oriented-pkl-generation.md`, `2026-07-29-claude-workflow-skill-packages.md`

## Decision

- Render OpenCode, Claude, and Pi identically: four thin workflow entrypoints (Pi: prompts; OpenCode and Claude: commands) plus four workflow-level skill packages — `sce-change-to-plan`, `sce-next-task`, `sce-validate`, and `sce-commit`.
- Emit exactly two files per package for every target: `SKILL.md` and `references/output.md`. The latter is the sole package reference and owns all human-visible gates, reports, and terminal Markdown layouts.
- Route each entrypoint to exactly its one corresponding workflow skill. No entrypoint sequences phase skills, and no workflow skill invokes a sibling SCE package.
- Keep the four canonical workflow definitions in `config/pkl/base/workflow-*.pkl` and their phase modules as the single behavioral source. They remain authoring inputs to composition; they are no longer a generated output surface for any target.
- Own composition in the target-neutral `config/pkl/renderers/workflow-composite.pkl`, parameterized only by the extra skill and command frontmatter each target supports. Claude passes `compatibility: claude` plus `allowed-tools:`, OpenCode passes `compatibility: opencode`, and Pi passes none.
- Keep phase statuses and phase-to-phase data as internal workflow state on every target. No target transports a YAML or Markdown phase-result contract between packages.
- Keep OpenCode's two thin routing agents and narrow their `skill:` permission allowlists to the four workflow slugs: `sce-change-to-plan` for Plan; `sce-next-task`, `sce-validate`, and `sce-commit` for Code.

## Rationale

The transport failure that motivated the Claude collapse was observed on Claude: a phase result returned through one skill can become the apparent deliverable and end the turn, so a command sequencing sibling skills is an unreliable carrier of workflow state. Pi and OpenCode were not observed failing that way. They are collapsed for cross-target uniformity and a single composition path — one renderer producing the same eight workflow documents for all three targets, rather than one composite renderer for Claude and a separate decomposed rendering path for the other two.

The trade-off is explicit: three targets now share one composition path and identical generated behavior, at the cost of OpenCode's explicit per-phase contracts and independently loadable phase packages. Those contracts survive as canonical Pkl authoring structure and as the internal phase boundaries composed into each `SKILL.md`; they simply stop being an installed surface.

Keeping the collapse in the renderer rather than the canonical base preserves the phase modules as the reviewable behavioral source and keeps the four workflows' gates, waits, writes, verification, synchronization, and continuation rules stated exactly once.

## Consequences

- No target emits `sce-context-load`, `sce-plan-authoring`, `sce-plan-review`, `sce-task-execution`, `sce-task-context-sync`, `sce-validation`, `sce-plan-context-sync`, or `sce-atomic-commit` as a package. Those names now denote canonical authoring modules and internal phases only.
- Per-target generated workflow inventory is four entrypoints plus eight skill Markdown files across four packages. OpenCode adds its two routing agents; Pi retains its extension; Claude retains settings and hooks.
- `config/pkl/renderers/workflow-composite.pkl` owns `workflowSkillSlugByCommand` for all targets; `claude-metadata.pkl` re-exports it.
- `config/pkl/renderers/metadata-coverage-check.pkl` asserts the workflow-package inventory and single-skill command route for all three targets through the target-neutral `expectedWorkflowSkillByCommand`, `expectedWorkflowSkillDocumentPaths`, and `assertCommandRoute(target, slug, document)`.
- OpenCode command frontmatter still carries `entry-skill` and `skills`, now naming one skill in both.
- Claude's generated output is unchanged by this decision; its byte-identity was verified across the change.
- Stale installed phase-skill directories in existing checkouts are handled by the existing setup remove-and-replace policy for the whole target directory (`context/sce/setup-no-backup-policy-seam.md`). No migration code exists.

## Superseded scope

This decision supersedes the Pi/OpenCode phase-package inventory and phase-result transport claims in `2026-07-27-workflow-oriented-pkl-generation.md` — specifically that all three targets receive the same eight self-contained phase packages, and that Pi and OpenCode preserve canonical YAML phase-result contracts. That decision remains authoritative for the four-workflow model, the `.pi/` behavioral baseline, the shared context-sync skeleton, routing-only agents, removed automated profiles, and retained non-Markdown outputs.

It also supersedes the clauses in `2026-07-29-claude-workflow-skill-packages.md` that scope the collapse to Claude alone: that phase packages remain the rendered form for Pi and OpenCode, and that Pi/OpenCode command behavior, eight-skill inventories, and phase contracts are preserved byte-for-byte. Everything that decision states about the composed package shape, the two-file package rule, and internal phase statuses now applies to all three targets.

`2026-03-03-plan-code-agent-separation.md` remains in force. OpenCode keeps its two routing agents; only their skill allowlists changed.
