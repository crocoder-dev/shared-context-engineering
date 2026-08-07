# Decision: Allow Non-SCE Helper Skills Inside SCE Workflow Steps

Date: 2026-08-07
Status: Accepted
Plan: `context/plans/update-sce-skill-orchestration.md`
Task: `T01`
Supersedes: `2026-07-30-synchronization-scoped-decision-writing.md` (only its absolute sibling-invocation prohibition)

## Context

The cross-target single-skill model keeps workflow control flow, gates, waits,
writes, validation, stops, and terminal output inside each SCE workflow skill.
The canonical wording nevertheless prohibited invoking any sibling skill without
scoping that rule to SCE workflow packages, which also made unrelated helper
capabilities appear forbidden. The generated workflows need one consistent
ownership boundary across OpenCode, Claude, and Pi.

## Decision

SCE workflow skills remain the exclusive owners of SCE workflow control flow and
must not chain arbitrary SCE skills, packages, or workflow commands. Relevant
non-SCE skills may be used as helper capabilities during the active step; when a
helper returns, control returns to that step, and helper use must preserve phase
order, gates, waits, writes, validation, stops, and terminal user-visible output.

## Rationale

This preserves the single-skill state-transport boundary while allowing unrelated
skills to contribute capabilities without becoming workflow owners or handoffs.
A shared target-neutral rule keeps the distinction identical across all targets.

## Alternatives considered

- **Keep the unscoped prohibition** — Prevents useful unrelated helper composition
  and makes the intended SCE ownership boundary ambiguous.
- **Allow arbitrary SCE workflow chaining** — Reintroduces the inter-skill control-
  flow and state-transport risk that the single-skill model removed.
- **Let each target define its own helper policy** — Creates cross-target drift in
  canonical workflow behavior.

## Compatibility and risks

- Generated workflow prose changes on all three targets, but workflow phase order,
  gates, waits, writes, validation, stops, and output contracts remain unchanged.
- A helper could be mistaken for a workflow handoff; the shared rule explicitly
  requires return to the active step and preserves every control-flow invariant.

## Guardrails

- Scope workflow-control prohibitions to SCE skills, packages, and commands.
- Keep helper composition target-neutral and shared by the composite preamble.
- Permit no arbitrary SCE workflow chaining; retain the synchronization-only
  `sce-decision` exception.
- Do not change generated target trees or runtime permission behavior in this task.

## Consequences

- Non-SCE helper skills are composable within an active SCE step without weakening
  workflow ownership.
- All generated SCE workflow skills state the same helper-return and invariant rule.
- The prior absolute sibling-invocation wording is no longer current; its
  synchronization-only `sce-decision` exception and single-skill ownership remain.

## Follow-up

- T02 and T03 continue the planned OpenCode permission and generated-contract work.

## References

- Plan: [`update-sce-skill-orchestration`](../plans/update-sce-skill-orchestration.md)
- Task: `T01`
- Current-state context: [`Architecture`](../architecture.md)
- Current-state context: [`Patterns`](../patterns.md)
- Current-state context: [`Shared Context Code Workflows`](../sce/shared-context-code-workflow.md)
- Evidence: [`workflow composite renderer`](../../config/pkl/renderers/workflow-composite.pkl)
- Evidence: [`workflow content primitives`](../../config/pkl/base/workflow-content.pkl)
- Related decision: [`Allow Decision Writing During Successful Context Synchronization`](2026-07-30-synchronization-scoped-decision-writing.md)
