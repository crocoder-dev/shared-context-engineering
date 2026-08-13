# Decision: Make `/validate` validation-only

Date: 2026-08-13
Status: Accepted
Plan: `context/plans/remove-validate-context-sync.md`
Task: `T01`
Supersedes: `context/decisions/2026-08-12-persist-workflow-sync-lifecycle-in-plans.md`

## Context

The canonical `/validate` workflow previously ran a plan-level context
synchronization phase after successful validation and persisted plan-level sync
lifecycle state around that handoff. The requested workflow boundary removes
that phase while retaining task-level synchronization in `/next-task`, final
validation, and the plan's Validation Report. The change is cross-target and
changes the ownership and completion contract of a core workflow.

## Decision

`/validate` performs final validation, writes the Validation Report, and returns
`validated`, `failed`, or `blocked`; it does not invoke plan-level context
synchronization or persist a plan-context-sync lifecycle handoff. Task-level
context synchronization remains owned by `/next-task`.

## Rationale

Validation and durable context reconciliation have distinct responsibilities.
Keeping `/validate` focused on observational validation removes the unwanted
plan-level synchronization lifecycle while preserving the task-level context
synchronization boundary and deterministic validation evidence.

## Alternatives considered

- **Retain automatic plan synchronization after validation** — This preserves
  the removed lifecycle and contradicts the requested validation-only boundary.
- **Replace plan synchronization with another automatic mechanism** — This
  expands scope and creates a new contract not established by the change.

## Compatibility and risks

- Plans and current-state documentation that describe `/validate` as invoking
  plan synchronization must be migrated to the validation-only contract.
- Durable context is no longer reconciled automatically as part of `/validate`;
  future replacement requires a separate decision and implementation.

## Guardrails

- Preserve all validation acceptance-criteria checks, full-validation commands,
  Validation Report writing, and `validated`/`failed`/`blocked` statuses.
- Do not remove the shared task-context-sync implementation or `/next-task`
  synchronization behavior.
- Do not generate a package-local plan context-sync reference for `/validate`.

## Consequences

- `/validate` has one validation phase and reports its result and Validation
  Report path without claiming durable context synchronization.
- `/next-task` remains the owner of task-level context synchronization after
  successful implementation.

## Follow-up

- Update current-state workflow and generation documentation to describe the
  validation-only `/validate` boundary.

## References

- Plan: [`remove-validate-context-sync`](../plans/remove-validate-context-sync.md)
- Task: `T01`
- Current-state context: [`Shared Context Code Workflows`](../sce/shared-context-code-workflow.md)
- Evidence: [`workflow-validate.pkl`](../../config/pkl/base/workflow-validate.pkl)
- Evidence: [`workflow-content.pkl`](../../config/pkl/base/workflow-content.pkl)
- Related decision: [`Persist Workflow Synchronization Lifecycle in Plans`](2026-08-12-persist-workflow-sync-lifecycle-in-plans.md)
