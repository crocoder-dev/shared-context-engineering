# Decision: Claude background Bash runs natively and untracked

Date: 2026-10-02
Status: Accepted
Plan: `context/plans/claude-background-shell-mutation-scope.md`
Task: `T02`
Supersedes: `context/decisions/2026-10-01-claude-background-admission-requires-proven-lifecycle.md`

## Context

The 2026-10-01 decision kept Claude-managed background shell execution denied
until SCE could prove completion through a reliable lifecycle contract or an
SCE-owned process supervisor. The T01 probe on Claude Code `2.1.284` could not
establish G1-G7: no terminal event, no stable completion correlation, and no
boundary proving that the background process can no longer mutate. That
technical conclusion still holds, so attribution-safe background tracking
cannot be built from the observed Claude lifecycle.

Holding the denial until the lifecycle is proven leaves background Bash
unusable under Claude for an unbounded time. The product direction is to
restore that capability and accept that SCE does not attribute what it
produces.

## Decision

SCE permits Claude-managed `Bash(run_in_background=true)` natively, without
mutation-scope tracking, as an accepted unsupported mutation-attribution
boundary. Native background Bash creates no SCE mutation scope and no Claude
adapter attempt, calls no generic mutation-scope `start`, and receives no
lifecycle tracking. Claude lifecycle completion remains unproven, and SCE
deliberately makes no mutation-attribution guarantee for mutations produced by
that background process.

## Rationale

Lifecycle completion could not be proven, so the only ways to keep attribution
correct are to deny background Bash or to build supervision SCE does not have.
Permitting it untracked restores a native Claude capability at the cost of a
bounded, explicitly documented attribution gap. Naming the gap as outside
SCE's guarantees is more honest than a tracking design that would claim
correctness on an unproven terminal signal.

## Alternatives considered

- **Keep the fail-closed denial until lifecycle completion is proven** — the
  superseded 2026-10-01 decision. Rejected because the lifecycle could not be
  proven, so background Bash would stay unavailable under Claude indefinitely.
- **Hook-driven attribution-safe lifecycle tracking** (suppressor, safety
  latch or taint, adapter state v2, background correlation) — rejected because
  G1-G7 are unproven and the design cannot rest on the observed lifecycle.
- **SCE-owned process supervision** — rejected for this change as out of scope
  and disproportionate to the capability being restored.

## Compatibility and risks

- This is a compatibility tradeoff, not an attribution-safe or fail-closed
  feature. Mutations from a native background Bash process may overlap with
  and contaminate the attribution of later or concurrent tracked scopes.
- Foreground mutation-capable Claude tools keep their existing lifecycle,
  attribution, failure, read-only, and delegation behavior.
- Background `PowerShell` remains denied.
- Decision state and implementation state are distinct. This record
  establishes the policy; the adapter continues to deny
  `Bash(run_in_background=true)` until T03 of the plan implements it.

## Guardrails

- The change introduces no suppressor, safety latch, worktree taint, unknown
  epoch, attribution filter, adapter-state version, background correlation,
  new terminal hook, capability gate, process supervision, mutation-protocol
  or Quint change, migration, doctor state, or recovery/reset command.
- The background Bash early return sits before repository resolution, the
  recovery barrier, model-state resolution, and normal mutation-scope
  establishment.
- No test or protocol machinery attempts to distinguish an untracked
  background process from a tracked foreground scope.
- The self-detaching-descendant boundary (D20) and the `probe14-*` /
  `probe17-*` evidence are unchanged. The T01 evidence fixture stays
  unmodified as historical evidence.

## Consequences

- Background Bash becomes usable under Claude once T03 lands, and SCE records
  nothing about it.
- The following sequence is an accepted consequence outside SCE's guarantees,
  not a bug:

  ```text
  background A starts            -> untracked
  foreground B starts            -> tracked
  A mutates the repository while B is live
  B closes
  SCE may observe A's mutation at B's boundary and may attribute it to B
  ```

- Attribution evidence for a tracked scope that overlapped a native background
  Bash process carries no guarantee that every observed mutation belongs to
  that scope.
- The 2026-10-01 record stays unmodified as history; this record is the
  effective background-admission decision.

## Follow-up

- T03 of the plan implements the admission and moves current-state context to
  the final contract.

## References

- Plan: [`claude-background-shell-mutation-scope.md`](../plans/claude-background-shell-mutation-scope.md)
- Task: `T02`
- Current-state context: [`claude-mutation-scope-background-execution.md`](../cli/claude-mutation-scope-background-execution.md)
- Current-state context: [`claude-mutation-scope-integration.md`](../cli/claude-mutation-scope-integration.md)
- Evidence: [`probe18-t01-current-version.evidence.json`](../../cli/src/services/hooks/claude_mutation_scope/fixtures/probe18-t01-current-version.evidence.json)
- Evidence: [`NOTES.md`](../../cli/src/services/hooks/claude_mutation_scope/fixtures/NOTES.md)
- Related decision: [`Keep Claude managed-background shell admission fail-closed until lifecycle completion is proven`](2026-10-01-claude-background-admission-requires-proven-lifecycle.md)
- Related decision: [`Make synchronization decision-gate outcomes and ADR history explicit`](2026-08-12-decision-gate-semantics.md)
- Related decision: [`External-mutation guard as a kernel-enforced process supervisor`](2026-09-17-external-mutation-guard-process-supervisor.md)
