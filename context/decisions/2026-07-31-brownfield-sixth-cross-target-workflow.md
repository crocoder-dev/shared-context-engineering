# Decision: Register `/brownfield` as the sixth cross-target SCE workflow and a bounded second writer of durable context

Date: 2026-07-31
Status: Accepted
Plan: `context/plans/brownfield-workflow.md`
Task: `T01, T02, T03`
Supersedes: `context/decisions/2026-07-30-restore-handover-cross-target-workflow.md`

## Context

A repository that already has code but no durable `context/` cannot reach the
existing SCE lifecycle. Task synchronization requires a completed task handoff
and plan synchronization requires a validated plan, so both paths presuppose the
memory a cold-start repository is missing. The only cross-target surface capable
of producing that memory from the repository's own evidence did not exist.

Two constraints shaped the answer. The workflow-oriented catalog and composite
renderer are designed to absorb a new workflow as one typed record plus one base
module, with routing, permissions, and generation contracts derived rather than
hand-written. And durable `context/` had exactly two authorized writers — the
task and plan synchronization phases — a boundary that any new context-writing
surface would widen.

## Decision

Register `brownfield` as the sixth entry in
`config/pkl/base/workflow-catalog.pkl`, compose it through
`config/pkl/renderers/workflow-composite.pkl`, and generate it for OpenCode,
Claude, and Pi as one thin command or prompt routed to exactly one phase-free
`sce-brownfield` package — and authorize that skill to write durable `context/`
outside the synchronization lifecycle, bounded to additive-by-default
reconstruction whose sole rewrite path is an explicit leading `rebuild` token.

This supersedes only the exact five-workflow inventory recorded by the
`/handover` restoration. That decision's catalog-derived routing model,
phase-free workflow shape, two-file package contract, and single
`shared-context-code` routing role remain authoritative and are reused here.

## Rationale

`/brownfield` fits the established architecture without exception: no new
routing agent, no phase package, no sibling-skill invocation, and no Rust CLI
change. Registering it through the catalog keeps the inventory, permissions, and
generation contracts uniformly derived instead of adding a bespoke surface.

The writer-authority extension is the part that could not be avoided. Cold-start
reconstruction has no execution or validation handoff to attach to, so it cannot
run inside either synchronization role. Granting the authority explicitly, with
its own narrower boundary, is safer than leaving a second writer's limits
implied. Additive-by-default means the failure mode of a mistaken run is a
skipped file rather than lost memory, and `rebuild` — the one mode that can
overwrite curated context — is recognized only as an explicit literal token and
never inferred from repository state or conversation.

## Alternatives considered

- **Leave `/brownfield` as a project-root Pi-only baseline** — Rejected; two of
  the three targets would lack the workflow, and the Pi tree would drift from
  canonical generation.
- **Add a third OpenCode routing role for it** — Rejected; context
  reconstruction is operational work with no planning/execution split, so it
  reuses `shared-context-code` as `/handover` does.
- **Route its writes through the existing synchronization phases** — Rejected;
  those phases require an authoritative execution or validation handoff that a
  cold-start repository cannot produce.
- **Make rewrite the default and additive the opt-in** — Rejected; rewrite
  authority can destroy curated memory, so the destructive mode must be the one
  the user asks for by name.

## Compatibility and risks

- Additive to the generated inventory: `nix run .#pkl-check-generated` now
  expects 70 paths (was 61), stated as a literal `expectedArtifactPathCount` so
  an unintended inventory change fails rather than redefining the expectation.
- No existing generated artifact changes except the `Shared Context Code`
  OpenCode agent, which gains one catalog-derived `"sce-brownfield": allow`
  permission line.
- Two writers of durable context with independently authored structure rules can
  drift. The rules stay duplicated across `workflow-context-sync.pkl` and
  `workflow-brownfield.pkl` rather than factored into one module; factoring them
  was deliberately out of scope.
- No new external dependency, no CLI or data-model change, and no
  distribution-relevant surface. The workflow performs no network access.

## Guardrails

- `sce-brownfield` stays phase-free and invokes no sibling skill, workflow
  command, or `sce-decision`.
- It never creates the `context/` root; `sce setup --bootstrap-context` remains
  the sole owner of that, and the root check precedes all investigation.
- Evidence is strictly local — the repository plus argument-supplied local paths.
  No URL fetch, web search, registry query, or external documentation lookup.
- Writes stay additive unless the literal leading `rebuild` token was supplied,
  no context file is deleted in either mode, and `context/plans/`,
  `context/handovers/`, `context/decisions/`, and `context/tmp/` are never
  touched.
- No confidence score, commit hash, timestamp, or date is written under
  `context/`.
- `/brownfield` remains a cold-start and gap-fill tool. Recurring context
  maintenance and drift repair stay owned by the synchronization phases.

## Consequences

- The catalog, coverage, and generation contracts enforce a six-workflow,
  seven-skill-package-per-target inventory instead of five and six.
- `generation-contract-check.pkl` now asserts required brownfield behavior
  directly: the bootstrap gate, documentation-discovery sweep, no-network rule,
  sub-`50` blocking threshold, always-disclosed contradiction contract, and
  additive-vs-`rebuild` write rule must survive in every generated `SKILL.md`.
- `context/sce/context-workflow-rules.md` now names two authorized writers of
  durable context with distinct boundaries, rather than one.
- Repository-root `.pi`, `.claude`, and `.opencode` dogfood mirrors all expose
  `/brownfield` consistently with the other five workflows.

## Follow-up

- None.

## References

- Plan: [`brownfield-workflow`](../plans/brownfield-workflow.md)
- Task: `T01, T02, T03`
- Current-state context: [`Brownfield workflow`](../sce/brownfield-workflow.md)
- Current-state context: [`Context Workflow Rules`](../sce/context-workflow-rules.md)
- Current-state context: [`Architecture`](../architecture.md)
- Evidence: [`workflow-brownfield.pkl`](../../config/pkl/base/workflow-brownfield.pkl)
- Evidence: [`workflow-catalog.pkl`](../../config/pkl/base/workflow-catalog.pkl)
- Evidence: [`generation-contract-check.pkl`](../../config/pkl/renderers/generation-contract-check.pkl)
- Related decision: [`Restore /handover as the fifth canonical cross-target SCE workflow`](2026-07-30-restore-handover-cross-target-workflow.md)
- Related decision: [`Allow Decision Writing During Successful Context Synchronization`](2026-07-30-synchronization-scoped-decision-writing.md)
