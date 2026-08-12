# Decision: Make Final Validation Observational

Date: 2026-08-12
Status: Accepted
Plan: `context/plans/workflow-skill-boundary-cleanup.md`
Task: `T05`

## Context

Final validation is a repository-wide workflow boundary shared by OpenCode,
Claude, and Pi. A validation run must measure the finished implementation, but
its prior contract also allowed it to remove temporary scaffolding before
recording evidence. That makes validation destructive and can hide unfinished
or debug-only artifacts that should remain visible to the implementation
workflow.

## Decision

Final validation is observational: it never deletes or repairs application,
test, configuration, context, debug-only, temporary, or local-scaffolding
artifacts. Any leftover debug or temporary artifact is recorded as failed
validation evidence under the failure follow-ups, and repair occurs only in a
later implementation session.

## Rationale

Recording leftovers preserves complete evidence of the delivered state and
keeps validation's proof boundary separate from implementation repair. The
rule is target-neutral and fits the existing failed-validation handoff, which
already stops without modifying product or test code.

## Alternatives considered

- **Delete temporary artifacts during validation** — This can hide incomplete
  work and makes the proof run mutate what it is meant to measure.
- **Repair artifacts automatically during validation** — This expands final
  validation into an implementation phase and makes its evidence dependent on
  unreviewed edits.

## Compatibility and risks

- Plans that expect a cleanup field in successful Validation Reports must use
  the failure-evidence section for leftovers instead.
- Existing validation runs may expose artifacts that an older run would have
  removed; this is intentional and gives the next implementation session an
  actionable repair target.

## Guardrails

- Keep the no-repair rule for application, test, and configuration code.
- Record leftover debug or temporary artifacts as failed checks with their path
  and evidence; do not delete them.
- Keep cleanup and repair outside `/validate` and within a normal implementation
  workflow.

## Consequences

- `/validate` proves the existing finished state without changing it.
- A successful Validation Report has no scaffolding-removal field.
- Generated targets share one deterministic observational validation contract.

## Follow-up

None.

## References

- Plan: [`workflow-skill-boundary-cleanup`](../plans/workflow-skill-boundary-cleanup.md)
- Task: `T05`
- Current-state context: [`Architecture`](../architecture.md)
- Current-state context: [`Shared Context Code Workflows`](../sce/shared-context-code-workflow.md)
- Evidence: [`workflow-validate.pkl`](../../config/pkl/base/workflow-validate.pkl)
- Evidence: [`workflow-context-sync.pkl`](../../config/pkl/base/workflow-context-sync.pkl)
- Related decision: [`Persist Workflow Synchronization Lifecycle in Plans`](2026-08-12-persist-workflow-sync-lifecycle-in-plans.md)
