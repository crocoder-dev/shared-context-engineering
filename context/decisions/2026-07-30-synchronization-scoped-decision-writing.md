# Decision: Allow Decision Writing During Successful Context Synchronization

Date: 2026-07-30
Status: Accepted
Plan: `context/plans/sce-decision-skill.md`
Task: `T01, T02, T03`
Supersedes: `context/decisions/2026-07-29-cross-target-workflow-skill-packages.md`

## Context

The cross-target workflow package model previously required each workflow skill to be fully self-contained and prohibited every sibling SCE package invocation. Durable task and plan synchronization can establish system-wide constraints whose rationale must remain discoverable after active plans are removed, but current-state context alone does not preserve immutable architecture history. OpenCode, Claude, and Pi therefore need one consistent, bounded mechanism for recording a qualifying decision without adding a user-facing workflow or reopening general inter-skill orchestration.

## Decision

Emit a standalone internal `sce-decision` package for OpenCode, Claude, and Pi, outside the four-command workflow catalog and without a command or prompt. Successful task and plan context synchronization may invoke it once for each qualifying system-wide decision, after impact discovery and before current-state context edits; no other workflow point or sibling skill invocation is allowed. The skill writes one dated ADR per decision, defaults new records to `Accepted` unless another allowed status is requested, and never edits an accepted ADR. A correction, reversal, or replacement creates a new dated ADR that references and supersedes the accepted record.

This supersedes only the prior decision's exact four-package inventory and absolute prohibition on sibling workflow-skill invocation. Its four command-routed workflow packages, two-file workflow-package shape, internal phase state, thin entrypoints, and shared composition model remain authoritative.

## Rationale

Keeping decision writing outside the command catalog preserves the existing user-facing workflow surface while giving both synchronization lifecycles one deterministic ADR contract. Restricting invocation to successful synchronization keeps execution and validation failure branches unable to write architecture history, and placing the gate before current-state edits makes a written or reused ADR available for links in the same synchronization pass. Immutable accepted records preserve historical rationale while superseding records express later changes explicitly.

## Alternatives considered

- **Keep all decision rationale only in current-state context** — This loses architecture history when current state changes and cannot explain why a costly system-wide constraint was adopted.
- **Add a user-facing `/decision` command or prompt** — This broadens the command surface and permits decision writing outside the synchronization evidence that establishes the choice.
- **Allow arbitrary sibling-skill orchestration** — This reintroduces the cross-package state-transport risk that the single-skill workflow model removed.
- **Permit accepted ADRs to be edited** — This obscures historical decisions and makes later corrections or reversals non-auditable.

## Compatibility and risks

- Generated inventory grows from 46 to 52 paths because each target gains `sce-decision/SKILL.md` and `sce-decision/references/adr-template.md`; exact generation checks enforce the new inventory.
- The sibling exception could expand accidentally; generation guards, target permissions, and workflow rules restrict references and invocation to successful `sce-next-task` and `sce-validate` synchronization.
- Task and plan synchronization can observe the same decision; matching ADR reuse prevents duplicate records.

## Guardrails

- Keep `sce-decision` outside `workflow-catalog.pkl` and do not generate a command or prompt for it.
- Invoke it only from the decision gate in successful task or plan synchronization, once per qualifying decision.
- Qualify only system-wide, durable, costly-to-reverse constraints; routine local, temporary, and easily reversible choices do not create ADRs.
- Allow only `Proposed`, `Accepted`, `Rejected`, `Deprecated`, or `Superseded`, with `Accepted` as the default.
- Never edit an accepted ADR; create a new dated superseding record for corrections, reversals, or replacements.
- Reuse an existing ADR that records the same decision instead of creating a duplicate.

## Consequences

- Every generated target has four command-routed workflow packages plus one standalone internal decision package.
- Commands and Pi prompts still route to exactly one workflow skill, and ordinary workflow phases remain internal.
- A decision-writing blocker stops context synchronization before current-state edits without undoing successful task execution or plan validation.
- Written or reused ADR paths become synchronization evidence and can be linked from current-state context immediately.

## Follow-up

- None.

## References

- Plan: [`sce-decision-skill`](../plans/sce-decision-skill.md)
- Task: `T01, T02, T03`
- Current-state context: [`Architecture`](../architecture.md)
- Current-state context: [`Shared Context Code Workflows`](../sce/shared-context-code-workflow.md)
- Current-state context: [`Context Workflow Rules`](../sce/context-workflow-rules.md)
- Evidence: [`decision-skill.pkl`](../../config/pkl/base/decision-skill.pkl)
- Evidence: [`workflow-context-sync.pkl`](../../config/pkl/base/workflow-context-sync.pkl)
- Related decision: [`Render Every Target's Workflows as Single-Skill Packages`](2026-07-29-cross-target-workflow-skill-packages.md)
