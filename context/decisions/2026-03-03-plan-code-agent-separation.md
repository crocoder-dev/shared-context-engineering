# Decision: Keep Shared Context Plan and Shared Context Code Separate

Date: 2026-03-03
Plan: `context/plans/sce-plan-code-convergence-and-sync-policy.md`
Task: `T05`

## Decision

- Do not merge Shared Context Plan and Shared Context Code.
- Keep role separation as the stable architecture: Plan owns planning/clarification/handoff; Code owns single-task implementation/validation/context-sync execution.

## Why this path

- The overlap inventory (`context/sce/plan-code-overlap-map.md`) shows mostly intentional structural overlap, not ownership confusion.
- Workflow clarity is stronger when planning and implementation gates stay in separate agents (`/change-to-plan` vs `/next-task`).
- Merge now would increase behavior coupling and regression risk across confirmation gates, scope controls, and context-sync sequencing.
- Current maintainability pressure is duplication reduction, which is solvable through canonical shared snippets without collapsing role boundaries.

## Compatibility and risk analysis

- Command compatibility remains stable by preserving existing entrypoints and responsibilities.
- No migration breakage is introduced for `/change-to-plan` and `/next-task` flows.
- Main risk avoided: blended agent instructions that blur stop conditions and approval boundaries.
- Residual risk: duplicated text drifting over time; mitigated via explicit dedup ownership.

## Dedup strategy while separate

- Keep one canonical shared baseline block for cross-agent principles (`human owns decisions`, `context as durable memory`, `code truth wins`, and `context/` authority rules).
- Keep role-specific mission, hard boundaries, and procedures local to each agent and phase-owning skills.
- Keep `/next-task` concise and orchestration-focused, delegating detailed contracts to `sce-plan-review`, `sce-task-execution`, and `sce-context-sync`.

## Consequences for follow-up tasks

- `T06` (conditional merge implementation) is not applicable under this decision.
- Continue with `T07` validation/cleanup to verify final consistency and generated parity.
