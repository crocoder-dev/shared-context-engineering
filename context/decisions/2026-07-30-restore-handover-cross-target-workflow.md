# Decision: Restore `/handover` as the fifth canonical cross-target SCE workflow

Date: 2026-07-30
Status: Accepted
Plan: `context/plans/handover-workflow.md`
Task: T01, T02, T03
Supersedes: `context/decisions/2026-07-27-workflow-oriented-pkl-generation.md`

## Context

`context/decisions/2026-07-27-workflow-oriented-pkl-generation.md` removed the
generated `handover` command/skill surface along with other obsolete Markdown
outputs when it established the current workflow-oriented Pkl generation
architecture. The user later requested a dual-mode `/handover` workflow —
writer mode captures a session/repository transition document, loader mode
reads one back for continuation — restored on top of that same
thin-command/self-contained-skill/cross-target Pkl architecture rather than as
a one-off surface.

## Decision

Register `handover` as the fifth entry in the typed
`config/pkl/base/workflow-catalog.pkl`, model it as a phase-free canonical
workflow in `config/pkl/base/workflow-handover.pkl`, wire it into the shared
`config/pkl/renderers/workflow-composite.pkl` composition, and generate it for
OpenCode, Claude, and Pi as one thin command routed to exactly one
`sce-handover` skill package — the same contract every other canonical
workflow already satisfies.

## Rationale

The workflow-oriented catalog/composite architecture is designed to scale to
new workflows by adding one typed record and one base module, without
per-target special-casing. `/handover` fits that shape exactly: it needs no
new OpenCode agent, no Rust CLI change, and no sibling-skill invocation, since
writer and loader mode never delegate or wait mid-run. Restoring it through
the catalog keeps the five-workflow inventory, permissions, and generation
contracts uniformly derived rather than reintroducing a bespoke surface
outside the established model.

## Alternatives considered

- **Leave `/handover` as a project-root-only Pi baseline** — rejected; the
  user asked for the same command across OpenCode, Claude, and Pi, and a
  Pi-only surface would leave two of the three targets without it.
- **Add a new OpenCode routing role for handover** — rejected; handover is an
  operational task-continuity workflow with no planning/execution split, so it
  reuses the existing `shared-context-code` role rather than introducing a
  third routing role.

## Compatibility and risks

- Additive to the generated artifact inventory: `nix run .#pkl-check-generated`
  now expects 61 paths (was 52); no existing workflow's generated content
  changes except the `Shared Context Code` OpenCode agent, which gains one
  `"sce-handover": allow` permission line.
- No new external dependency, no CLI/data-model change, and no security- or
  distribution-relevant surface.

## Guardrails

- `sce-handover` remains phase-free and invokes no sibling skill, workflow
  command, or `sce-decision`.
- Loader mode stays strictly read-only; writer mode never overwrites an
  existing handover file.
- No OpenCode routing role beyond `shared-context-code` is introduced for this
  workflow.

## Consequences

- The workflow catalog and generation-contract checks now enforce a
  five-workflow, six-skill-package-per-target inventory instead of four/five.
- Repository-root `.pi`, `.claude`, and `.opencode` dogfood mirrors all expose
  `/handover` consistently with the other four workflows.
- Future workflow additions have one more concrete precedent for a phase-free,
  no-agent, no-decision-invoking workflow shape.

## Follow-up

None.

## References

- Plan: [`handover-workflow`](../plans/handover-workflow.md)
- Task: `T01`, `T02`, `T03`
- Current-state context: [`context/sce/handover-workflow.md`](../sce/handover-workflow.md), [`context/architecture.md`](../architecture.md)
- Evidence: `config/pkl/base/workflow-catalog.pkl`, `config/pkl/renderers/workflow-composite.pkl`, `config/pkl/renderers/generation-contract-check.pkl`
- Related decision: [`workflow-oriented Pkl generation`](2026-07-27-workflow-oriented-pkl-generation.md)
