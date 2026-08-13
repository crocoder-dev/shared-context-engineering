# Decision: Split Commit Message Guidance from Atomic Commit Procedure

Date: 2026-08-13
Status: Accepted
Plan: `context/plans/remove-validate-context-sync.md`
Task: `T02`

## Context

The generated `/commit` package is produced for OpenCode, Claude, and Pi from one
canonical Pkl module. Its staged baseline moves commit-message style guidance to a
package-local `references/commit-message-style.md` document and removes the
obsolete YAML result-contract section from `references/atomic-commit.md`. The
commit phase still uses internal statuses and preserves regular/bypass routing;
only reference ownership and the obsolete serialized contract documentation
change.

## Decision

The generated `sce-commit` package will emit a separate
`references/commit-message-style.md` for message wording, while
`references/atomic-commit.md` owns staged-diff procedure, result branching, and
commit boundaries without a generated YAML result-contract section or
`commit-contract.yaml` artifact.

## Rationale

A separate style reference gives message-writing rules one explicit owner and
keeps the operational phase document focused on staged-diff analysis. Removing
the obsolete YAML section matches the single-skill model, where result statuses
remain internal to `/commit` rather than being transported between generated
packages.

## Alternatives considered

- **Keep style guidance inline in `atomic-commit.md`** — This preserves the old
  ownership split and makes the staged reference baseline non-canonical.
- **Retain the YAML result-contract section or generate `commit-contract.yaml`** —
  This preserves a removed serialized contract that the current single-skill
  workflow no longer needs.

## Compatibility and risks

- All three targets gain the same package-local style reference and retain the
  same regular/bypass behavior and internal result statuses.
- The generated reference inventory changes, so exact path and target-parity
  checks must reject the obsolete contract artifact and verify the new style file.

## Guardrails

- `config/pkl/base/workflow-commit.pkl` remains the sole authoring source.
- Commit routing, staged truth, message result fields, and human-visible output
  layouts remain unchanged.
- Target differences remain limited to supported frontmatter.

## Consequences

- `references/commit-message-style.md` is a durable generated package reference
  for all three targets.
- `references/atomic-commit.md` no longer carries the obsolete YAML result-contract
  section, and no `commit-contract.yaml` replacement is generated.

## Follow-up

None.

## References

- Plan: [`remove-validate-context-sync`](../plans/remove-validate-context-sync.md)
- Task: `T02`
- Current-state context: [`Atomic commit workflow`](../sce/atomic-commit-workflow.md)
- Evidence: [`Canonical commit workflow`](../../config/pkl/base/workflow-commit.pkl)
- Related decision: [`Package-local phase references`](2026-08-03-package-local-phase-reference-workflow-packages.md)
