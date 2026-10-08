# Decision: PR2 scope includes cancellation-protected blocking boundaries and a focused test set

Date: 2026-10-08
Status: Accepted
Plan: `context/plans/cli-async-turso-persistence-pr2.md`
Task: `T05`
Supersedes: [Directly awaited Turso persistence](2026-10-07-directly-awaited-turso-persistence.md) (scope and verification posture only)

## Context

The 2026-10-07 decision described PR2 as Turso conversion plus its connected
callers, and recorded that ordinary Rust suites were disabled. The implemented
PR2 also moved Git snapshot subprocesses, protected-worktree marker I/O,
lock acquisition, Git ref mutation and guarded-shell supervision onto
`spawn_blocking` boundaries, and the PR #301 review required fixes for
transaction exclusivity, credential rotation, terminal sync failure, adapter
boundary leases and guarded-shell descriptor inheritance.

## Decision

- Native awaited Turso persistence is unchanged: no DB-owned runtime, executor
  bridge, or Turso future inside a blocking worker.
- PR2 includes the Git/process/filesystem boundaries above. Each retained
  blocking worker owns its protected resource (lock, lease, marker, child) until
  it finishes after caller cancellation; `spawn_blocking` that provides this is
  retained, not removed mechanically.
- Transactions require exclusive `&mut` access with checked transaction
  construction.
- Correctness evidence is the Quint MBT suite, surviving targeted tests and a
  small set of focused regression tests (transaction, cancellation, contention,
  credential rotation, terminal sync, boundary lease, descriptor inheritance).
  The deleted ordinary suites are not restored.
- PR3 is cosmetic/maintainability cleanup only; PR4 is OpenTelemetry.

## Alternatives considered

- **Move the blocking boundaries back to async workers** — cancellation could
  release locks or leases while blocking work is still running.
- **Restore the deleted suites** — large cost; the focused set targets the
  distinct new regressions.

## Compatibility and risks

- Runtime shutdown waits for retained workers, each bounded by a lock timeout,
  file I/O or a child process.
- A killed process can leave a temporary Git index file behind.
- A crash between remote refresh-token rotation and the local save can still
  lose the rotated token.
- Ordinary behavioral coverage remains lower than before the migration.

## References

- [Architecture: application execution runtime](../architecture.md#application-execution-runtime)
- [Shared Turso adapter](../sce/shared-turso-db.md)
- [Application-owned async command runtime](2026-10-07-application-owned-async-command-runtime.md)
