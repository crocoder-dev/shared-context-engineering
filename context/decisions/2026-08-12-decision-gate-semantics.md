# Decision: Make synchronization decision-gate outcomes and ADR history explicit

Date: 2026-08-12
Status: Accepted
Plan: `context/plans/workflow-skill-boundary-cleanup.md`
Task: `T10`
Supersedes: `context/decisions/2026-07-30-synchronization-scoped-decision-writing.md`

## Context

Successful task and plan synchronization use the standalone `sce-decision`
skill to preserve qualifying system-wide decisions. The existing contract
classified a nonqualifying request as `blocked`, which incorrectly stopped
otherwise successful synchronization. Its ADR history rules also protected
accepted records without stating the equivalent active statuses that may be
reused or the immutable treatment of rejected, deprecated, and superseded
records.

## Decision

The synchronization decision gate will return `not_qualified` or `skipped` for
nonqualifying decisions and continue synchronization; `blocked` is reserved for
missing, contradictory, or unsafe decision-writing input or history. ADRs are
immutable regardless of status: only an equivalent `Proposed` or `Accepted`
(active) ADR may be reused, while rejected, deprecated, and superseded records
are never reused. Any changed, corrected, reversed, or replaced decision
creates a new dated ADR. `Deprecated` and `Superseded` remain distinct
creation-time-only statuses.

## Rationale

Separating nonqualification from failure keeps the decision gate from turning a
normal context synchronization into a blocker. Active-only reuse preserves one
canonical current record without reviving decisions that were rejected or
replaced. Immutable records and dated replacements keep the decision history
auditable, while retaining both terminal status names preserves their distinct
historical meaning.

## Alternatives considered

- **Keep nonqualifying decisions as `blocked`** — Rejected; ordinary changes
  would incorrectly prevent durable context synchronization from completing.
- **Reuse any equivalent ADR regardless of status** — Rejected; rejected,
  deprecated, and superseded records do not represent active guidance.
- **Edit an existing ADR when a decision changes** — Rejected; this would erase
  the historical rationale and make reversals non-auditable.
- **Collapse `Deprecated` and `Superseded`** — Rejected; the statuses retain
  useful distinct historical meaning, so both remain creation-time-only.

## Compatibility and risks

- Existing accepted ADRs remain valid and unchanged; existing rejected,
  deprecated, and superseded ADRs become explicitly non-reusable.
- Generated synchronization consumers continue to stop on genuine `blocked`
  decision handoffs while continuing after `not_qualified` or `skipped`.
- Callers must distinguish a nonqualifying result from a decision-writing
  failure when interpreting the internal handoff.

## Guardrails

- Invoke `sce-decision` only from successful task or plan context synchronization.
- Write at most one ADR per structured decision request and never mutate an
  existing ADR or its status.
- Use only `Proposed`, `Accepted`, `Rejected`, `Deprecated`, or `Superseded`
  for a newly written ADR, defaulting to `Accepted`.
- Use a new dated ADR for every correction, reversal, or changed decision.

## Consequences

- Synchronization can complete normally when no architecture decision qualifies.
- Decision history records active reuse and all later changes without rewriting
  prior records.
- The standalone decision package exposes a stable distinction between
  nonqualification, deliberate skipping, and unsafe decision writing.

## Follow-up

None.

## References

- Plan: [`workflow-skill-boundary-cleanup`](../plans/workflow-skill-boundary-cleanup.md)
- Task: `T10`
- Current-state context: [`Shared Context Code Workflows`](../sce/shared-context-code-workflow.md)
- Current-state context: [`Architecture`](../architecture.md)
- Current-state context: [`Patterns`](../patterns.md)
- Evidence: [`decision-skill.pkl`](../../config/pkl/base/decision-skill.pkl)
- Evidence: [`workflow-context-sync.pkl`](../../config/pkl/base/workflow-context-sync.pkl)
- Related decision: [`Allow Decision Writing During Successful Context Synchronization`](2026-07-30-synchronization-scoped-decision-writing.md)
