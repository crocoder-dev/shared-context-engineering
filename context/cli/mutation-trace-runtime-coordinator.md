# Mutation-trace runtime coordinator (`mutation_trace::runtime`)

The imperative-shell layer that connects the verified, pure
mutation-cursor protocol kernel ([`protocol.rs`](mutation-trace-protocol.md))
and its persistence layer ([`store.rs`](mutation-trace-store.md)) to a real
Git worktree, built by the `mutation-cursor-runtime-coordinator` plan
(`context/plans/mutation-cursor-runtime-coordinator.md`).

`cli/src/services/mutation_trace/runtime/` is a private submodule
(`pub(crate) mod runtime;` in `mutation_trace/mod.rs`), registered under the
same `#[allow(dead_code)]` precedent as the rest of `mutation_trace`.
`coordinator::coordinate()` is the public entrypoint, but `runtime/mod.rs`
still declares `mod coordinator;` privately, so `coordinate()` is reachable
only from within `runtime` itself (its own tests) for now; a `pub(crate)`
re-export is deferred until a harness adapter needs it. Nothing under
`runtime/` is wired into any hook, command, or `diff_traces` insertion yet.

`runtime` depends on `protocol`/`store`/`types` and on `services::checkout`,
never the reverse — this is a structural module boundary, not merely a
documented convention.

## Current code surface

The per-worktree runtime lock, the isolated Git snapshot service, the
coordinator's protocol-integration pipeline, and the public `coordinate()`
entrypoint (lock, external-taint fence, checkout identity, and DB provider
around that pipeline) all exist, with `runtime/tests.rs` exercising the public
API end to end. Only harness/command wiring remains.

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
  the external-taint fence. The critical section: resolve `git_dir` via
  `checkout::resolve_git_dir`, acquire the `WorktreeLock` (bounded 10s, held
  for the whole call), arm the `ExternalTaintMarker` write-ahead, resolve
  checkout identity via `checkout::get_or_create_checkout_id` and wrap it as
  `WorktreeId` — no caller-supplied `WorktreeId` or `Boundary` is ever
  accepted — invoke `open_db()`, construct `GitSnapshotService`, delegate to
  the internal generic-over-`SnapshotCapture` pipeline, and clear the marker
  only on a successful outcome. Identity flows
  `repository_root → git_dir → WorktreeLock → checkout ID → WorktreeId`; the
  DB is not on that chain. (`coordinate()` is a one-line delegation to a
  private `coordinate_inner(.., open_db, on_lock_contention: impl FnOnce(),
  after_recovery: impl FnMut(u32) -> Result<()>)` test seam; production passes a
  no-op contention closure and `|_| Ok(())`.) A `WorktreeLock`
  acquisition failure surfaces as `CoordinateError::LockAcquisition`; pre-commit
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

The runtime lock guards the coordinator's own critical section (external-taint
marker arming/clearing, snapshot capture, worktree/scope materialization,
recovery, and the CAS retry loop): `coordinate()` acquires it before arming the
marker and resolving checkout identity, and holds it until the call returns. It
is held on every `coordinate()` call, unlike the checkout-identity-creation
lock.

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
└── refs/sce/mutation-cursor/<worktree-id>/<tree-sha>   (runtime::git_snapshot, one ref per pinned tree, create-only)
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

`GitSnapshotService`'s inline `#[cfg(test)] mod tests` in `git_snapshot.rs`
uses the same precedent, extended to real per-test `git init` repositories;
its coverage is documented in
[`mutation-trace-snapshot-service.md`](mutation-trace-snapshot-service.md).

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
`coordinate_inner(.., open_db, on_lock_contention)` observes the real
`TryLockError::WouldBlock` branch while a first `WorktreeLock` is held, then
acquires and returns `Ok` once it drops); and the external-taint fence — a
successful call clears the marker, while a snapshot failure, a non-snapshot
failure, a DB-provider `Err`, and an un-armable marker each leave it present
(the last failing closed before the DB provider runs). A further test drives the
private `after_recovery` seam to inject a failure at the exact
recovery-committed / boundary-not-yet-prepared transition and proves the
recovery is durable, the boundary unprocessed with no `MutationEvent`, the
on-disk marker still present, and a later `coordinate()` re-recovering
conservatively off it; `runtime/tests.rs` separately proves an attributable
`Advance` that commits durably then fails its trailing `marker.clear()` surfaces
`MarkerClearAfterCommit` carrying the matching committed outcome (including its
`MutationEvent`).

`runtime/tests.rs` is `runtime`'s own `#[cfg(test)] mod tests`, holding
cross-module integration tests that drive only the public `coordinate()` API
against real Git repositories (`git init`, `git worktree add`) and real
temp-file `RepositoryAgentTraceDb`s, following the same unique-temp-path
precedent: two linked worktrees of one repository (different `git_dir` →
different lock paths → different `WorktreeId`s) are proven independently
locked by holding one worktree's `WorktreeLock` across a synchronous
`coordinate()` call for the other and observing that call return `Ok` before
the held guard is dropped. Each call is handed a provider closure that opens
the one shared repository-scoped DB path (`coordinate()` never resolves the
DB), and both distinct worktree rows then coexist in it. A first-ever
`agent_trace_storage` resolution and a `coordinate()` call on the same
checkout converge on one checkout identity; and a full failure/recovery
cycle — baseline call, a snapshot-failing call that durably taints the
worktree, then a recovery call that clears the taint before processing its
boundary — runs entirely through the public entrypoint.

## Status

The lock, snapshot service, protocol-integration pipeline, and the public
`coordinate()` entrypoint (resolve `git_dir` → `WorktreeLock` → arm the
external-taint marker → checkout identity → caller-supplied DB provider →
pipeline → clear the marker on success) are all implemented, with
`runtime/tests.rs` covering the public API end to end; an inherited external-taint
marker is now overlaid onto `database_failure` recovery on the next invocation. A
`pub(crate)` re-export of `coordinate()` beyond `runtime` and harness/command
wiring remain future work tracked by the `mutation-cursor-external-taint` and
`mutation-cursor-runtime-coordinator` plans.

See also: [`mutation-trace-snapshot-service.md`](mutation-trace-snapshot-service.md)
(the `GitSnapshotService` capture/pin/diff/inventory/delete contract),
[`mutation-trace-protocol.md`](mutation-trace-protocol.md),
[`mutation-trace-store.md`](mutation-trace-store.md),
[`mutation-trace-external-taint.md`](mutation-trace-external-taint.md)
(the `<git-dir>/sce/mutation-cursor-tainted` write-ahead fence armed by
`coordinate()`), [`checkout-identity.md`](checkout-identity.md).
