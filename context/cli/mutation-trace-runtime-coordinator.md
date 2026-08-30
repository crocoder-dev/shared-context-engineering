# Mutation-trace runtime coordinator (`mutation_trace::runtime`)

The imperative-shell layer that will connect the verified, pure
mutation-cursor protocol kernel ([`protocol.rs`](mutation-trace-protocol.md))
and its persistence layer ([`store.rs`](mutation-trace-store.md)) to a real
Git worktree, built by the `mutation-cursor-runtime-coordinator` plan
(`context/plans/mutation-cursor-runtime-coordinator.md`).

`cli/src/services/mutation_trace/runtime/` is a private submodule
(`pub(crate) mod runtime;` in `mutation_trace/mod.rs`), registered under the
same `#[allow(dead_code)]` precedent as the rest of `mutation_trace`: only
`coordinator.rs`'s eventual public entrypoints will be reachable from outside
`runtime` once they exist, and nothing under `runtime/` is wired into any
hook, command, or `diff_traces` insertion yet.

`runtime` depends on `protocol`/`store`/`types` and on `services::checkout`,
never the reverse — this is a structural module boundary, not merely a
documented convention.

## Current code surface

Only the per-worktree runtime lock exists so far.

- `cli/src/services/mutation_trace/runtime/worktree_lock.rs` —
  `WorktreeLock::acquire(git_dir: &Path, timeout: Duration) ->
  Result<WorktreeLock, WorktreeLockError>` opens/creates
  `<git_dir>/sce/mutation-cursor.lock` and polls `std::fs::File::try_lock()`
  on a 100ms interval against the caller-supplied bounded `timeout`, rather
  than calling the blocking `File::lock()` directly. A held `WorktreeLock`
  releases the OS lock when dropped (RAII). Timing out returns a distinct,
  matchable `WorktreeLockError::TimedOut { path, timeout }` variant, separate
  from `WorktreeLockError::Io` (file-open or other I/O failure). The lock
  file's mere on-disk existence is never treated as ownership — only a
  successful OS-level `try_lock()` counts, so a leftover lock file with no
  active OS lock held against it never blocks a fresh acquirer.

This lock guards the coordinator's own critical section (snapshot capture,
worktree/scope materialization, recovery, and the CAS retry loop) once the
coordinator exists. It is held on every future `coordinate()` call, unlike
the checkout-identity-creation lock.

## Two distinct locks, two distinct invariants

`<git-dir>/sce/mutation-cursor.lock` (this module) and
`<git-dir>/sce/checkout-id.lock` (see
[`checkout-identity.md`](checkout-identity.md)) are deliberately separate
locks guarding separate invariants, not one lock reused for two purposes:

| | Path | Guards | Held by | Blocking behavior |
| --- | --- | --- | --- | --- |
| Checkout-identity lock | `<git-dir>/sce/checkout-id.lock` | "this checkout has at most one durable identity" | any caller of `get_or_create_checkout_id` | blocks indefinitely, no timeout — the critical section is a handful of filesystem syscalls |
| Mutation-cursor runtime lock | `<git-dir>/sce/mutation-cursor.lock` | the coordinator's entire runtime critical section | only the coordinator, on every invocation | bounded polling with a caller-supplied timeout — a stuck holder must not deadlock every future hook invocation |

On-disk layout so far:

```text
<worktree-git-dir>/sce/
├── checkout-id                 (services::checkout)
├── checkout-id.lock            (services::checkout)
└── mutation-cursor.lock        (runtime::worktree_lock, this module)
```

## Testing boundary

`WorktreeLock`'s inline `#[cfg(test)] mod tests` in `worktree_lock.rs` covers
contention (a second acquirer blocks until the first releases), independence
across distinct worktree paths, timing out with a distinct matchable error
while the lock is still held, and a leftover lock file with no active OS lock
held against it never blocking a fresh acquirer — each test uses a unique
`std::env::temp_dir()` path, following the same filesystem-touching
inline-unit-test precedent already used in `cli/src/services/checkout/mod.rs`
and `cli/src/services/mutation_trace/store.rs` (see `context/patterns.md`).

## Status

Only the per-worktree runtime lock (above) is implemented. The Git snapshot
service, the coordinator's protocol-integration pipeline, the public
lock-wrapped `coordinate()` entrypoint, and cross-module integration tests
remain future work tracked by the `mutation-cursor-runtime-coordinator`
plan's task stack. This file will grow to cover them as each lands.

See also: [`mutation-trace-protocol.md`](mutation-trace-protocol.md),
[`mutation-trace-store.md`](mutation-trace-store.md),
[`checkout-identity.md`](checkout-identity.md).
