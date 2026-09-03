# Mutation-trace runtime coordinator (`mutation_trace::runtime`)

The imperative-shell layer that connects the verified, pure
mutation-cursor protocol kernel ([`protocol.rs`](mutation-trace-protocol.md))
and its persistence layer ([`store.rs`](mutation-trace-store.md)) to a real
Git worktree, built by the `mutation-cursor-runtime-coordinator` plan
(`context/plans/mutation-cursor-runtime-coordinator.md`).

`cli/src/services/mutation_trace/runtime/` is a private submodule
(`pub(crate) mod runtime;` in `mutation_trace/mod.rs`), registered under the
same `#[allow(dead_code)]` precedent as the rest of `mutation_trace`.
Every submodule — `coordinator`, `protected_worktree`, `ref_reconciliation`,
and the rest — is declared privately in `runtime/mod.rs`, so `coordinate()` and
`reconcile_worktree` are reachable only from within `runtime` itself (its own
tests) for now; a `pub(crate)` re-export is deferred until a harness adapter
needs it. Nothing under `runtime/` is wired into any hook, command, or
`diff_traces` insertion yet.

`runtime` depends on `protocol`/`store`/`types` and on `services::checkout`,
never the reverse — this is a structural module boundary, not merely a
documented convention.

## Current code surface

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
- `cli/src/services/mutation_trace/runtime/git_snapshot.rs` — the isolated Git
  snapshot and ref-pinning service (`GitSnapshotService`:
  `new`/`capture_tree`/`pin_tree`/`diff_trees`, plus the callerless
  worktree-scoped `list_pins` pin inventory —
  `Result<Vec<PinnedRef>, PinInventoryError>` — and conditional-atomic
  `delete_pins` batch deletion). It writes tree/blob objects into the
  repository's normal, shared object database and protects durable trees with
  create-only, **direct** `refs/sce/mutation-cursor/<worktree-id>/<tree-sha>`
  refs; a symbolic ref inside that namespace is malformed and rejected, and
  `delete_pins` uses no-dereference semantics so an inventory→delete ref-type
  race cannot escape the inventoried namespace. Full contract in
  [`mutation-trace-snapshot-service.md`](mutation-trace-snapshot-service.md).
- `cli/src/services/mutation_trace/runtime/ref_reconciliation.rs` — the
  conservative per-worktree snapshot-ref maintenance pass — `reconcile_worktree`
  / `pub(super) reconcile_worktree_inner` return `Result<ReconciliationOutcome,
  ReconcileError>` (`ReconciliationOutcome` = `Reconciled(ReconciliationReport)`
  | `SkippedNoCheckoutIdentity`). Under the worktree's `WorktreeLock` it deletes
  only pins whose tree is a durable root of **no** worktree, fails closed if any
  local root lacks a pin, and writes no `mutation_trace_*` row or taint marker
  (only the namespace of a checkout id a current worktree still derives — a
  namespace no current worktree owns, via a deleted worktree or checkout-id
  metadata loss/recreation, is future repository-scoped work). Full contract in
  [`mutation-trace-ref-reconciliation.md`](mutation-trace-ref-reconciliation.md).
- `cli/src/services/mutation_trace/runtime/protected_worktree.rs` — the shared
  safety prefix every runtime entrypoint runs behind (`ProtectedWorktree`:
  resolve `git_dir` → `WorktreeLock` → external-taint fence → `WorktreeId`, plus
  an explicit `complete()` as the only thing that clears the marker). Full
  contract in [`mutation-trace-protected-worktree.md`](mutation-trace-protected-worktree.md).
- `cli/src/services/mutation_trace/runtime/coordinator.rs` — the composition
  point that drives `protocol.rs`/`store.rs`/`git_snapshot.rs` together. Its
  `SnapshotCapture` trait (`capture(&self) -> Result<TreeId>`, `pin(&self,
  worktree_id, tree) -> Result<()>`) is the one dependency-injection seam the
  pipeline introduces for determinism; `GitSnapshotService` implements it
  directly, and the module's own tests use a fake, call-counting
  implementation instead of real concurrent Git processes.
  `RuntimeBoundary` is a hook/flush boundary in already-canonical runtime
  identities (`Start`/`Advance`/`Close` carry `{ scope, event, actor_kind }`;
  `Flush` carries nothing — its worktree is always the invocation's own
  already-resolved one, never caller-supplied) and documents the
  `(ScopeId, EventId)` replay-identity contract a future harness adapter must
  uphold. The public `coordinate(repository_root, boundary, open_db) ->
  Result<CoordinateOutcome, CoordinateError>` entrypoint owns the whole
  protected operation. It does **not** receive an already-open DB handle:
  `open_db: impl FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>` is a
  caller-supplied provider it invokes itself, so DB acquisition falls inside
  the external-taint fence. The critical section is the `ProtectedWorktree`
  prefix above — no caller-supplied `WorktreeId` or `Boundary` is ever
  accepted — then `open_db()`, `GitSnapshotService`, the internal
  generic-over-`SnapshotCapture` pipeline, and `ProtectedWorktree::complete()`
  only on a successful outcome; `ProtectedWorktreeError` maps onto exactly the
  `CoordinateError` variants that step already produced. Identity flows
  `repository_root → git_dir → WorktreeLock → checkout ID → WorktreeId`; the
  DB is not on that chain. (`coordinate()` is a one-line delegation to the
  `pub(super) coordinate_inner(.., on_lock_contention, after_load, after_recovery)`
  test seam — reachable from `runtime::tests`, invisible outside `runtime`;
  production passes a no-op for all three. `after_load: impl FnMut(u32)` fires
  each CAS attempt after `load_worktree` and before the real `store.commit` CAS;
  the reconciliation pin→CAS lock-race regression uses it to pause a real
  `coordinate()` between `pin` and CAS. No production behavior change.) A
  `WorktreeLock` acquisition failure surfaces as
  `CoordinateError::LockAcquisition`; pre-commit
  marker-I/O and DB-provider failures have their own fail-closed variants, and a
  post-commit `marker.clear()` failure surfaces as
  `CoordinateError::MarkerClearAfterCommit { source, committed }` — the boundary
  did commit, so the durable `CoordinateOutcome` (with any `MutationEvent`) rides
  along in `committed` rather than being lost, and the marker stays armed. See
  [`mutation-trace-external-taint.md`](mutation-trace-external-taint.md) for the
  fence ordering, the safety invariant, and the `CoordinateError` variants it
  adds. The pipeline does, per invocation: capture and pin
  exactly one Git snapshot; on failure, run a bounded taint-retry loop instead
  (below) and return without touching the rest of the pipeline; on success,
  idempotently materialize the worktree row and, for hook boundaries, the
  scope row; then loop (bounded, `MAX_CAS_RETRY_ATTEMPTS = 5`, no backoff):
  load durable state fresh, recover first if the worktree is tainted, needs
  rebaseline, or inherited an external-taint marker (overlaid as
  `database_failure`; its CAS commit reuses the one captured tree), then
  `prepare`/`commit` the triggering boundary against that state (a second CAS
  commit) — reloading and recomputing from scratch on `Conflict`, without ever
  re-capturing or re-pinning. A settled no-op result (a stale, rejected, or
  replayed attempt) is a successful return, not an error.

  A capture or pin failure is handled by its own bounded taint-retry loop: a
  fresh `load_worktree` on every iteration, always evaluated after the
  failure, never before it — so a worktree another caller materializes
  concurrently while this invocation's own capture is still in flight is
  still found and correctly tainted. No durable worktree row on that fresh
  read means no taint to record (`persisted_taint: false`, no write); an
  already-tainted no-op reads back the current flag instead of assuming
  success; otherwise the loop commits the taint transition and retries on
  `Conflict`, reporting `persisted_taint: false` only once every bounded
  attempt has been exhausted.

The runtime lock guards the whole critical section — fence arming/clearing,
snapshot capture, worktree/scope materialization, recovery, and the CAS retry
loop — and is held on every `coordinate()` call, unlike the
checkout-identity-creation lock.
`ref_reconciliation::reconcile_worktree` acquires the **same** lock file
(bounded by its own `RECONCILIATION_LOCK_TIMEOUT`) before it inventories pins,
reads durable roots, or deletes anything.

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
├── mutation-cursor.lock        (runtime::worktree_lock)
├── mutation-cursor-tainted     (runtime::external_taint, empty; existence = fence armed)
└── tmp/
    └── index-<uuid>            (runtime::git_snapshot, ephemeral per capture)

<repository's normal, shared object database>       (runtime::git_snapshot writes here directly)
<repository's normal, shared refs namespace>
└── refs/sce/mutation-cursor/<worktree-id>/<tree-sha>   (runtime::git_snapshot, create-only per invocation; orphan/unreferenced pins reclaimed by runtime::ref_reconciliation only for a checkout id a current worktree still derives, every pin for a current or historical durable mutation-cursor root retained; a namespace no current worktree owns — deleted worktree or checkout-id metadata loss/recreation — is unreachable, future repository-scoped work)
```

## Testing boundary

`WorktreeLock`'s inline `#[cfg(test)] mod tests` in `worktree_lock.rs` covers
contention (a second acquirer blocks until the first releases), independence
across distinct worktree paths, timing out with a distinct matchable error
while the lock is still held, and a leftover lock file with no active OS lock
never blocking a fresh acquirer — each test uses a unique
`std::env::temp_dir()` path, following the same filesystem-touching
inline-unit-test precedent as `cli/src/services/checkout/mod.rs` and
`cli/src/services/mutation_trace/store.rs` (see `context/patterns.md`).

`ProtectedWorktree`'s inline tests use RAII `tempfile::TempDir` fixtures over
real `git init` repositories; coverage in
[`mutation-trace-protected-worktree.md`](mutation-trace-protected-worktree.md#testing-boundary).

`GitSnapshotService`'s inline `#[cfg(test)] mod tests` in `git_snapshot.rs` uses
the same precedent, extended to real per-test `git init` repositories; coverage
in [`mutation-trace-snapshot-service.md`](mutation-trace-snapshot-service.md).

`coordinator.rs`'s inline `#[cfg(test)] mod tests` exercises the internal
pipeline against a real temp-file `RepositoryAgentTraceDb`, using a fake,
call-counting `SnapshotCapture` (or, for CAS-conflict scenarios, real OS
threads racing separate DB handles against one on-disk database): first
observation establishes a baseline with no evidence; an edit observed between
`Start` and `Advance` commits exactly one `AiExclusive` event; replaying an
identical `(scope, event)` boundary is a no-op, not a duplicate; `Close`
attributes to the scope it is about to close; two live scopes yield
`AiContended` regardless of matching or differing `ActorKind`; a CAS conflict
reloads and recomputes without a second capture or pin; `needs_rebaseline`
recovery preserves live scopes while taint recovery abandons them; and the
taint-retry loop taints an existing worktree, survives a losing CAS before
committing on retry, reports `persisted_taint: false` once exhausted, makes
no write when no worktree row exists yet, and still finds and taints a
worktree another caller materializes concurrently during this invocation's
own failing capture. Further tests drive the public `coordinate()` against
real repositories: the critical-section serialization (a worker's
`coordinate_inner` observes the real `TryLockError::WouldBlock` branch while a
first `WorktreeLock` is held, then acquires and returns `Ok` once it drops); and
the external-taint fence — a successful call clears the marker, while a snapshot
failure, a non-snapshot failure, a DB-provider `Err`, and an un-armable marker
each leave it present (the last failing closed before the DB provider runs). A
further test drives the private `after_recovery` seam to inject a failure at the
recovery-committed / boundary-not-yet-prepared transition, proving the recovery
durable, the boundary unprocessed with no `MutationEvent`, the marker still
present, and a later `coordinate()` re-recovering off it; another proves an
attributable `Advance` that commits then fails its trailing `marker.clear()`
surfaces `MarkerClearAfterCommit` with the matching committed outcome. The
`after_load` seam is exercised by the reconciliation pin→CAS lock-race regression
([`mutation-trace-ref-reconciliation.md`](mutation-trace-ref-reconciliation.md)), pausing a real `coordinate()` between `pin` and CAS.

`runtime/tests.rs` is `runtime`'s own `#[cfg(test)] mod tests` of cross-module
integration tests against real Git repositories (`git init`, `git worktree
add`) and real temp-file `RepositoryAgentTraceDb`s — the public `coordinate()`,
the public `reconcile_worktree` integration suite (detailed in
[`mutation-trace-ref-reconciliation.md`](mutation-trace-ref-reconciliation.md#testing-boundary)), and the `pub(super)` `coordinate_inner` / `reconcile_worktree_inner` lock-race seams. Two linked worktrees (different `git_dir` →
different lock paths → different `WorktreeId`s) are proven independently locked
by holding one worktree's `WorktreeLock` across a synchronous `coordinate()`
call for the other and seeing it return `Ok` only after the guard drops; each
call's provider closure opens the one shared repository-scoped DB path and both
worktree rows coexist in it. A first-ever `agent_trace_storage` resolution and a
`coordinate()` call on one checkout converge on one checkout identity; and a
full baseline → snapshot-failing taint → recovery cycle runs through the public
entrypoint.

## Status

The `ProtectedWorktree` prefix, lock, snapshot service, protocol-integration
pipeline, and the public `coordinate()` entrypoint (prefix → DB provider →
pipeline → `complete()` on success) are all implemented, with `runtime/tests.rs`
covering the public API end to end; an inherited external-taint marker is
overlaid onto `database_failure` recovery on the next invocation. A `pub(crate)`
re-export of `coordinate()`, a second `abandon_scope()` entrypoint sharing the
same prefix, and harness/command wiring remain future work
(`mutation-cursor-external-taint`, `mutation-cursor-runtime-coordinator`,
`mutation-scope-runtime-integration`).

See also: [`mutation-trace-ref-reconciliation.md`](mutation-trace-ref-reconciliation.md)
(the per-worktree snapshot-ref maintenance pass under the same `WorktreeLock`),
[`mutation-trace-snapshot-service.md`](mutation-trace-snapshot-service.md)
(the `GitSnapshotService` capture/pin/diff/inventory/delete contract),
[`mutation-trace-protocol.md`](mutation-trace-protocol.md),
[`mutation-trace-store.md`](mutation-trace-store.md),
[`mutation-trace-external-taint.md`](mutation-trace-external-taint.md)
(the `<git-dir>/sce/mutation-cursor-tainted` write-ahead fence),
[`mutation-trace-protected-worktree.md`](mutation-trace-protected-worktree.md)
(the shared prefix that arms it), [`checkout-identity.md`](checkout-identity.md).
