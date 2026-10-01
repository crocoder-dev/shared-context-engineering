# Decision: Keep Claude managed-background shell admission fail-closed until lifecycle completion is proven

Date: 2026-10-01
Status: Accepted
Plan: `context/plans/claude-background-shell-mutation-scope.md`
Task: `T01`

## Context

Claude Code `2.1.284` did not produce a model turn or any managed-background
tool lifecycle after the T01 capture session reached `SessionStart`; a no-hook
control behaved the same way. The current-version probe therefore could not
establish a terminal event, a stable completion correlation, or a boundary
proving that the process can no longer mutate. Earlier `2.1.258` evidence
showed only that `PostToolUse` acknowledges a background task quickly and
includes `backgroundTaskId`; it did not establish completion. Acknowledgement
alone is not a safe mutation boundary.

## Decision

Claude-managed background shell execution remains denied until SCE can prove
completion through a reliable lifecycle contract or an SCE-owned process
supervisor.

## Rationale

Keeping the existing denial preserves fail-closed mutation attribution when
SCE cannot prove that a detached process has stopped. Admission based only on
the acknowledgement event could leave a repository-mutating process outside a
live suppressor and create false-positive or falsely scoped mutation evidence.

## Alternatives considered

- **Admit on `PostToolUse` acknowledgement** — rejected because prior evidence
  shows the acknowledgement can precede background completion.
- **Implement the hook-driven lifecycle immediately** — rejected because the
  installed version did not establish the required G1-G5 contract.
- **Use the existing external-mutation guard unchanged** — rejected because
  that supervisor is the human-mutation path and does not yet establish the
  Claude-managed AI execution contract.

## Compatibility and risks

- Background Bash remains unavailable to Claude until a later plan revision or
  probe establishes safe completion; foreground behavior is unchanged.
- The conservative boundary may produce a false negative for legitimate
  background execution, which is preferred to unproven positive attribution.

## Guardrails

- No `run_in_background = true` admission change may land without evidence of
  process termination ordering and a proven no-more-mutation boundary.
- Prior-version fixtures remain evidence only and must not be treated as the
  current Claude lifecycle contract.

## Consequences

- The hook-driven background lifecycle plan stops before protocol admission.
- A future implementation must revise the plan toward SCE-owned supervision or
  first capture the missing current-version lifecycle evidence.

## Follow-up

Revise the plan toward SCE-owned process supervision before admitting managed
background shells.

## References

- Plan: [`claude-background-shell-mutation-scope.md`](../plans/claude-background-shell-mutation-scope.md)
- Task: `T01`
- Current-state context: [`claude-mutation-scope-background-execution.md`](../cli/claude-mutation-scope-background-execution.md)
- Evidence: [`probe18-t01-current-version.evidence.json`](../../cli/src/services/hooks/claude_mutation_scope/fixtures/probe18-t01-current-version.evidence.json)
- Related decision: [`External-mutation guard as a kernel-enforced process supervisor`](2026-09-17-external-mutation-guard-process-supervisor.md)
