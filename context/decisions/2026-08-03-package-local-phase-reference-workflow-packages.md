# Decision: Use Package-Local Phase References in Phase-Based Workflow Packages

Date: 2026-08-03
Status: Accepted
Plan: `context/plans/canonicalize-workflow-phase-references.md`
Task: `T01`
Supersedes in part: `context/decisions/2026-07-29-cross-target-workflow-skill-packages.md`

## Context

The single-skill workflow package model prevents unreliable inter-skill phase-result transport, but placing every phase instruction and persisted-document format directly in `SKILL.md` forces the agent to load the complete lifecycle before it reaches later phases. The staged Claude package refactor demonstrated a package-relative shape that preserves one invocation and internal phase state while allowing each phase document to be read only when its workflow step is reached.

OpenCode, Claude, and Pi must continue to receive the same workflow behavior and package-relative inventory, with target differences limited to supported frontmatter. Human-visible gates and terminal layouts must remain owned only by `references/output.md`.

## Decision

Every phase-based generated workflow package uses one `SKILL.md` for control flow and package-local Markdown references for phase instructions and persisted-document formats. `SKILL.md` parses input, orders phases, branches on internal state, owns waits and same-session resume, and requires the applicable reference to be read before that phase runs. OpenCode, Claude, and Pi render the same reference inventory and document bodies, apart from supported target frontmatter.

Phase-free workflows retain their existing two-file package shape. The standalone decision package remains unchanged.

## Rationale

This keeps the single-skill execution boundary that solved phase-result transport failures while reducing initial instruction loading and making phase ownership explicit. Package-relative references preserve self-containment: no command invokes a phase skill, no phase state crosses package boundaries, and every referenced instruction ships beside its workflow entrypoint.

## Alternatives considered

- **Keep all phase instructions inline in `SKILL.md`** — This preserves single-file loading but requires every run to load phases it may never reach and obscures phase-specific ownership.
- **Restore generated phase-skill packages** — This would recreate the unreliable inter-skill transport boundary the single-skill model removed.
- **Apply the reference split only to Claude** — This would reintroduce target-specific workflow composition and inventory drift.

## Compatibility and risks

- Existing installed workflow package inventories change for the four phase-based workflows; setup's remove-and-replace installation policy handles stale files.
- A missing, stale, or unresolved reference could omit load-bearing behavior. Exact metadata and generation contracts therefore check package inventories, resolvable phase references, output-layout deduplication, and forbidden sibling-package references.
- Workflow gates, statuses, waits, branches, routing, and user-visible layouts remain behaviorally compatible.

## Guardrails

- Commands and prompts invoke exactly one workflow skill and never sequence phase skills.
- `SKILL.md` remains the sole owner of phase ordering, branching, waits, and same-session resume.
- A phase reference is read before its phase takes action; references are package-local.
- `references/output.md` remains the sole owner of human-visible gates and terminal layouts.
- Phase-free workflows and the decision package do not gain empty phase references.
- Target-specific differences remain limited to supported frontmatter.

## Consequences

- The exact two-file clause for phase-based packages in the superseded decision no longer applies; those packages contain `SKILL.md`, `references/output.md`, and their named phase or persisted-document references.
- Canonical Pkl owns the workflow body and complete package-relative document inventory for all three targets.
- Phase behavior remains internal to one skill invocation and no serialized phase-result contract is reintroduced.
- The generated artifact contract increases from 71 to 101 paths.

## Follow-up

- None.

## References

- Plan: [`canonicalize-workflow-phase-references`](../plans/canonicalize-workflow-phase-references.md)
- Task: `T01`
- Current-state context: [`Architecture`](../architecture.md)
- Current-state context: [`Patterns`](../patterns.md)
- Evidence: [`Canonical workflow content`](../../config/pkl/base/workflow-content.pkl)
- Evidence: [`Generation contract`](../../config/pkl/renderers/generation-contract-check.pkl)
- Related decision: [`Render Every Target's Workflows as Single-Skill Packages`](2026-07-29-cross-target-workflow-skill-packages.md)
