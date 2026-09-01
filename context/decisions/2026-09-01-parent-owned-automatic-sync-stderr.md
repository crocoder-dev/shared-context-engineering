# Decision: Parent-owned captured stderr for automatic sync

Date: 2026-09-01
Status: Accepted
Plan: `context/plans/auto-sync-captured-stderr.md`
Task: `T01`
Supersedes: `context/decisions/2026-08-31-synchronous-automatic-sync-completion.md`

## Context

The synchronous post-commit launcher must keep automatic `sce sync --format json`
stdout silent, preserve visible typed failure diagnostics, and avoid leaving the
child attached to the caller's stderr descriptor. The previously accepted
completion boundary requires the launcher to wait for terminal child completion,
but inherited stderr conflicts with that descriptor-ownership requirement.

## Decision

The automatic launcher pipes the child stderr, drains it through terminal child
completion, and forwards the captured bytes through the parent's stderr after the
wait succeeds.

## Rationale

`wait_with_output` drains the pipe while waiting, avoiding a full-pipe deadlock
while retaining synchronous completion. Parent forwarding preserves the child's
operator-visible diagnostics without descriptor inheritance, and ignoring the
child exit status preserves the existing single-diagnostic fail-open behavior.

## Alternatives considered

- **Inherit stderr** — rejected because the child retains the post-commit caller's
  stderr descriptor.
- **Suppress captured stderr** — rejected because automatic failure visibility is
  an established operator-facing contract.
- **Stream stderr concurrently to the parent** — rejected as unnecessary added
  coordination when capture plus `wait_with_output` provides bounded synchronous
  draining and preserves output bytes.

## Compatibility and risks

- Successful automatic JSON sync remains stdout-silent and child diagnostics
  remain visible on stderr, but they are forwarded after completion rather than
  appearing live while the child runs.
- A wait failure still uses the one typed launcher diagnostic and remains
  fail-open; a non-zero child exit does not create a duplicate parent diagnostic.

## Guardrails

- Keep the one-shot `sync --format json` command, repository-root working
  directory, null stdin/stdout, internal automatic marker, and synchronous wait.
- Do not add a timeout, retry, queue, daemon, or persistent synchronization state.
- Do not change manual sync stream routing or the post-commit trigger boundary.

## Consequences

- The parent owns the automatic child's stderr descriptor and forwards captured
  diagnostics through its own stderr path.
- Automatic diagnostics are buffered until terminal completion, while pipe
  draining remains deadlock-safe.

## Follow-up

- Update current-state automatic-sync and CLI stream contracts to remove inherited
  stderr wording and describe parent forwarding.

## References

- Plan: [`auto-sync-captured-stderr`](../plans/auto-sync-captured-stderr.md)
- Task: `T01`
- Current-state context: [`Automatic Agent Trace synchronization`](../cli/agent-trace-auto-sync.md)
- Evidence: [`automatic sync launcher`](../../cli/src/services/sync/auto_sync.rs)
- Related decision: [`Wait for automatic sync completion at the post-commit launcher boundary`](2026-08-31-synchronous-automatic-sync-completion.md)
