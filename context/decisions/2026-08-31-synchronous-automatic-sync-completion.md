# Decision: Wait for automatic sync completion at the post-commit launcher boundary

Date: 2026-08-31
Status: Accepted
Plan: `context/plans/auto-sync-failure-guidance.md`
Task: `T06`

## Context

The post-commit hook launches the existing one-shot `sce sync --format json`
command after local Agent Trace persistence. The launcher must preserve the
repository-root working directory, the internal automatic-invocation marker,
null stdin/stdout, inherited stderr, and the hook's fail-open boundary. The
selected completion policy also needs deterministic child termination and stream
closure, while avoiding a daemon, retry queue, timeout policy, or persistent
failure state.

T06 implements and verifies the selected policy with focused launcher tests for
completion, non-zero child exit, wait failure, command preservation, and startup
failure. The plan's prior completion-policy evidence records that waiting was
selected despite its measured commit-latency cost.

## Decision

The automatic post-commit launcher waits for the spawned `sce sync --format
json` child to reach terminal completion before returning.

## Rationale

Waiting gives the post-commit boundary a deterministic completion point and
ensures the inherited diagnostic stream is closed before the launcher returns.
The child remains responsible for rendering synchronization failures, so the
parent does not duplicate non-zero child diagnostics. Startup and wait errors
are still rendered through the existing typed launcher diagnostic and remain
fail-open to the successful hook result.

## Alternatives considered

- **Detached, non-waiting launch** — rejected because it leaves child completion
  and stream closure nondeterministic at the post-commit boundary.
- **Waiting with a new timeout or retry mechanism** — rejected because it would
  add policy and persistent or repeated execution behavior outside this change.

## Compatibility and risks

- Automatic post-commit execution can add the child and network runtime to commit
  latency; the hook still returns success for non-zero child exits and launcher
  wait failures.
- The command arguments, working directory, marker, stdio routing, manual sync
  behavior, and child-rendered diagnostic ownership remain compatible.

## Guardrails

- Launch exactly one `sync --format json` child through the current executable.
- Keep stdin and stdout null, stderr inherited, and the repository root as cwd.
- Do not add timeouts, retries, daemons, queues, schedulers, or persistent state.
- Ignore child exit status after successful wait; report only launcher startup or
  wait errors through the typed fail-open diagnostic path.

## Consequences

- Automatic synchronization has a synchronous completion boundary at the
  post-commit launcher while remaining fail-open to the commit.
- Successful automatic JSON sync remains stdout-silent and child failures remain
  visible through inherited stderr.
- Commits may take longer when automatic synchronization is enabled.

## Follow-up

- Document the synchronous completion semantics and latency trade-off in the
  current-state context contracts.

## References

- Plan: [`auto-sync-failure-guidance`](../plans/auto-sync-failure-guidance.md)
- Task: `T06`
- Current-state context: [`Automatic Agent Trace synchronization`](../cli/agent-trace-auto-sync.md)
- Evidence: [`automatic sync launcher`](../../cli/src/services/sync/auto_sync.rs)
- Related decision: None.
