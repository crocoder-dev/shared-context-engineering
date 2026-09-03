# Mutation-trace runtime coordinator (`mutation_trace::runtime`)

The imperative-shell layer that connects the verified, pure
mutation-cursor protocol kernel ([`protocol.rs`](mutation-trace-protocol.md))
and its persistence layer ([`store.rs`](mutation-trace-store.md)) to a real
Git worktree, built by the `mutation-cursor-runtime-coordinator` plan
(`context/plans/mutation-cursor-runtime-coordinator.md`).

`cli/src/services/mutation_trace/runtime/` is a private submodule
(`pub(crate) mod runtime;` in `mutation_trace/mod.rs`), registered under the
same `#[allow(dead_code)]` precedent as the rest of `mutation_trace`.
Every submodule is declared privately in `runtime/mod.rs`, which re-exports
`coordinate` and `abandon_scope` at `pub(crate)` while `reconcile_worktree`
stays `runtime`-internal. The exported seam is documented in
[`mutation-scope-runtime.md`](mutation-scope-runtime.md), and nothing under
`runtime/` is wired into any hook, command, or `diff_traces` insertion yet.

`runtime` depends on `protocol`/`store`/`types` only, and has no dependency
on any checkout-identity service — that service was removed from SCE
entirely (see [`checkout-identity.md`](checkout-identity.md)) and never
existed alongside this coordinator on `main`. Worktree identity is derived
directly from Git's own topology inside `runtime/git_snapshot.rs`.

## Current code surface

The per-worktree runtime lock, the isolated Git snapshot service (which also
derives `WorktreeId` from Git topology), the coordinator's internal
protocol-integration pipeline, and the public, lock-wrapped `coordinate()`
entrypoint that owns the external-taint fence and DB provider around that
pipeline all exist, with cross-module integration tests in `runtime/tests.rs`
exercising the public API end to end. Only harness/command wiring remains.

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
- `cli/src/services/mutation_trace/runtime/git_snapshot.rs` —
  `GitSnapshotService::new(repository_root: &Path) -> Result<GitSnapshotService>`
  resolves `git_dir` once via `git rev-parse --absolute-git-dir`, so
  `git_dir` is always an absolute path — even when the caller's
  `repository_root` is relative, which matters because every Git subprocess
  this service spawns runs with `cwd = repository_root` and
  `GIT_DIR = git_dir`; a relative `git_dir` would otherwise be resolved by
  the child process against its own already-`repository_root`-joined `cwd`,
  double-joining the path. `capture_tree(&self) -> Result<TreeId>` snapshots
  the current worktree (staged, unstaged, untracked, and deleted state,
  respecting `.gitignore`) into the repository's normal, shared Git object
  database, never touching the real index or working tree: it reserves a
  unique `<git-dir>/sce/tmp/index-<uuid>` path via an RAII guard (never
  pre-creating the file), probes `HEAD` via a dedicated `head_exists`
  helper that inspects the Git exit status directly — status `0` means
  `HEAD` resolves, status `1` is `--verify --quiet`'s documented "does not
  resolve" signal (a genuinely unborn `HEAD`), and every other status
  propagates as an error rather than being treated as empty, since HEAD
  absence is a normal Git state but a HEAD-probe failure is a snapshot
  failure — then runs `git read-tree HEAD` or, on a genuinely unborn `HEAD`,
  the explicit `git read-tree --empty` (never a bare/absent index file),
  then `git add -A -- .`, then `git write-tree`, all with only
  `GIT_DIR`/`GIT_INDEX_FILE` set — no `GIT_OBJECT_DIRECTORY`/
  `GIT_ALTERNATE_OBJECT_DIRECTORIES` override anywhere. `TreeId` is an opaque
  string; nothing assumes a fixed length, so a SHA-256 repository needs no
  special handling. `pin_tree(&self, worktree_id, tree) -> Result<()>` makes
  a tree durable by creating
  `refs/sce/mutation-cursor/<worktree_id>/<tree-sha>` via `git update-ref` —
  create-only and idempotent for the same `(worktree_id, tree)` pair — which
  is what makes a pinned tree survive `git gc --prune=now`/`git prune
  --expire=now`, unlike an unpinned, unreachable tree in the same repository.
  `diff_trees(&self, before, after) -> Result<String>` runs `git diff
  --binary --full-index --no-ext-diff --no-textconv` between two tree SHAs,
  returning the raw diff text `patch.rs::parse_patch` already knows how to
  parse. It also exposes worktree-scoped `list_pins` inventory with distinct
  Git and malformed-ref errors, and `delete_pins` conditional-atomic batch
  deletion through one `git update-ref --no-deref --stdin` transaction. Pin
  refs are required to remain direct refs; symbolic refs are malformed and
  rejected, and a ref moved after inventory aborts the whole batch before any
  deletion. The full snapshot/ref-reconciliation
  contract is also documented in
  [`mutation-trace-snapshot-service.md`](mutation-trace-snapshot-service.md).
  `coordinator.rs` is its only caller, via the `SnapshotCapture` trait
  below. This file also exposes `resolve_git_dir(repository_root) ->
  Result<PathBuf>` (`pub(super)`, the same `--absolute-git-dir` resolution
  `GitSnapshotService::new` uses internally, now reusable by `coordinator.rs`
  for lock-path resolution — one canonical Git-dir implementation, not a
  duplicate) and `resolve_worktree_id(repository_root) ->
  Result<WorktreeId>` (`pub(super)`): it additionally resolves `git rev-parse
  --path-format=absolute --git-common-dir` and compares it against
  `resolve_git_dir`'s result. When they match, the checkout is the main
  worktree and the identity is `WorktreeId("main")`. When they differ, the
  checkout is a linked worktree — Git's own `--absolute-git-dir` for a linked
  worktree is `<git-common-dir>/worktrees/<name>`, a name Git assigns and
  keeps stable for the life of that worktree — so the identity is
  `WorktreeId("worktrees/<name>")`, with `<name>` restricted to
  ASCII-alphanumeric/`-`/`_`/`.` (any other character replaced with `_`) so
  it can never produce an invalid or surprising
  `refs/sce/mutation-cursor/<worktree-id>/...` ref. This derivation reads
  only Git's own state — no file is created or read under `<git-dir>/sce/`,
  and no identity is generated or persisted anywhere.
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
  resolve `git_dir` → `WorktreeLock` → external-taint fence → Git-derived
  `WorktreeId`, plus an explicit `complete()` as the only thing that clears the
  marker). Full contract in [`mutation-trace-protected-worktree.md`](mutation-trace-protected-worktree.md).
- `cli/src/services/mutation_trace/runtime/scope_runtime.rs` — the second
  protected entrypoint, `abandon_scope(repository_root, scope, open_db)`, which
  retires an unobserved scope without capturing or pinning a Git snapshot. It
  classifies durable scope state before applying `protocol::abandon`, retries
  CAS conflicts from fresh state, and clears the shared fence only after a
  settled outcome. Full contract in
  [`mutation-trace-scope-abandonment.md`](mutation-trace-scope-abandonment.md).
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
  `repository_root → git_dir / git_common_dir → WorktreeLock → Git-derived
  WorktreeId`; the DB is not on that chain. (`coordinate()` is a one-line
  delegation to the `pub(super) coordinate_inner(.., on_lock_contention,
  after_load, after_recovery)` test seam — reachable from `runtime::tests`,
  invisible outside `runtime`; production passes a no-op for all three.
  `after_load: impl FnMut(u32)` fires each CAS attempt after `load_worktree` and
  before the real `store.commit` CAS; the reconciliation pin→CAS lock-race
  regression uses it to pause a real `coordinate()` between `pin` and CAS. No
  production behavior change.) A `WorktreeLock` acquisition failure surfaces
  as `CoordinateError::LockAcquisition`; pre-commit marker-I/O and DB-provider
  failures have their own fail-closed variants, and a
  post-commit `marker.clear()` failure surfaces as
  `CoordinateError::MarkerClearAfterCommit { source, committed }` — the
  boundary did commit, so the durable `CoordinateOutcome` (with any
  `MutationEvent`) rides along in `committed` rather than being lost, and the
  marker stays armed. See
  [`mutation-trace-external-taint.md`](mutation-trace-external-taint.md) for the
  fence ordering, the safety invariant, and the `CoordinateError` variants.
  The pipeline does, per invocation: capture and pin

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

The runtime lock guards every runtime entrypoint's critical section
(external-taint marker arming/clearing, snapshot capture where applicable,
worktree/scope materialization, recovery, and the CAS retry loop):
`coordinate()` and `abandon_scope()` acquire it before their protected work and
hold it until the call returns. The separate `ref_reconciliation::reconcile_worktree`
pass acquires this same lock before inventorying pins, reading durable roots,
or deleting refs, with its own bounded timeout. `<git-dir>/sce/mutation-cursor.lock`
remains worktree-specific because `git_dir` itself is worktree-specific for linked
worktrees (`resolve_git_dir` resolves each worktree's own
`--absolute-git-dir`), so each worktree has an independent critical section.

On-disk layout so far:

```text
<worktree-git-dir>/sce/
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
inline-unit-test precedent already used in
`cli/src/services/mutation_trace/store.rs` (see `context/patterns.md`).

`ProtectedWorktree`'s inline tests use RAII `tempfile::TempDir` fixtures over
real `git init` repositories; coverage in
[`mutation-trace-protected-worktree.md`](mutation-trace-protected-worktree.md#testing-boundary).

`scope_runtime`'s inline tests use the same RAII fixtures and a real temporary
Agent Trace DB; coverage in
[`mutation-trace-scope-abandonment.md`](mutation-trace-scope-abandonment.md#testing-boundary).

`GitSnapshotService`'s inline `#[cfg(test)] mod tests` in `git_snapshot.rs`
uses the same precedent, extended to real per-test `git init` repositories:
index/working-tree preservation across staged/unstaged/untracked/deleted
state, `.gitignore` exclusion, unborn-`HEAD` capture with and without files,
an unexpected `HEAD`-probe failure (a corrupted/missing `.git/HEAD`)
propagating as an error rather than a false empty-baseline capture, a
relative `repository_root` still resolving `git_dir` absolute, survival
after the temp index file is gone, `git gc --prune=now`/`git prune
--expire=now` survival for a pinned tree versus reclamation of a distinct
unpinned tree in the same repository, `pin_tree` idempotency, `diff_trees` output shape, worktree-scoped pin
inventory (including malformed target/name/path and Git-failure cases), and
SHA-conditioned batch deletion (including empty batches and whole-transaction
abort when a ref moved after inventory). It also covers
`resolve_worktree_id`: a normal repository always resolves to
`WorktreeId("main")`, stable across repeated calls; two linked worktrees of
one repository each resolve to a distinct `WorktreeId("worktrees/<name>")`,
distinct from `"main"` and from each other, also stable across repeated calls;
and resolving a worktree id never creates any file or directory under
`<git-dir>/sce/`.

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

`runtime/tests.rs` is `runtime`'s own `#[cfg(test)] mod tests`, holding
cross-module integration tests that drive the public `coordinate()` and
`abandon_scope()` APIs against real Git repositories (`git init`, `git worktree
add`) and real temp-file `RepositoryAgentTraceDb`s, following the same
unique-temp-path precedent: two linked worktrees of one repository (different
`git_dir` → different lock paths → different `WorktreeId`s) are proven
independently locked by holding one worktree's `WorktreeLock` across a
synchronous `coordinate()` call for the other and observing that call return
`Ok` before the held guard is dropped — a shared lock could not be acquired
while the guard is alive, and no wall-clock timing is used. Each call is handed
a provider closure that opens the one shared repository-scoped DB path
(`coordinate()` never resolves the DB), and both distinct worktree rows then
coexist in it. A full failure/recovery cycle and the cross-runtime abandonment
regressions also run through the public entrypoints. The same module covers the
public `reconcile_worktree` integration suite and the
`coordinate_inner` / `reconcile_worktree_inner` / `abandon_scope_inner`
lock-race seams, including orphan reclamation, durable-root retention,
missing-pin fail-closed behavior, malformed refs, linked-worktree scoping, and
the real coordinator and abandonment CAS races.

## Status

The shared `ProtectedWorktree` prefix, per-worktree runtime lock, isolated Git
snapshot service (including Git-topology-derived `WorktreeId` resolution),
protocol-integration pipeline, public `coordinate()` entrypoint, and
`abandon_scope()` entrypoint are implemented. Both entrypoints are
`pub(crate)` re-exported from `runtime/mod.rs` and described in
[`mutation-scope-runtime.md`](mutation-scope-runtime.md); `runtime/tests.rs`
covers their public behavior and cross-runtime regressions. An inherited marker
is overlaid onto `database_failure` recovery on the next invocation. Harness and
command wiring remain future work tracked by the
`mutation-cursor-external-taint`, `mutation-cursor-runtime-coordinator`, and
`mutation-scope-runtime-integration` plans.

See also: [`mutation-trace-protocol.md`](mutation-trace-protocol.md),
[`mutation-trace-store.md`](mutation-trace-store.md),
[`mutation-trace-external-taint.md`](mutation-trace-external-taint.md)
(the `<git-dir>/sce/mutation-cursor-tainted` write-ahead fence),
[`mutation-trace-protected-worktree.md`](mutation-trace-protected-worktree.md)
(the shared prefix that arms it),
[`mutation-trace-scope-abandonment.md`](mutation-trace-scope-abandonment.md),
[`checkout-identity.md`](checkout-identity.md).
