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

The per-worktree runtime lock, the isolated Git snapshot service, and the
coordinator's internal protocol-integration pipeline exist so far; only the
public, lock-wrapped `coordinate()` entrypoint that drives the lock and
checkout identity around that pipeline is still future work.

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
  parse. `coordinator.rs` is its only caller, via the `SnapshotCapture` trait
  below.
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
  uphold. The internal, generic-over-`SnapshotCapture` pipeline — not yet
  reachable outside `runtime`, since the public `coordinate()` entrypoint is
  still future work (see Status) — does, per invocation: capture and pin
  exactly one Git snapshot; on failure, run a bounded taint-retry loop instead
  (below) and return without touching the rest of the pipeline; on success,
  idempotently materialize the worktree row and, for hook boundaries, the
  scope row; then loop (bounded, `MAX_CAS_RETRY_ATTEMPTS = 5`, no backoff):
  load durable state fresh, recover first if the worktree is tainted or needs
  rebaseline (its own CAS commit, reusing the one captured tree as the
  rebaseline target), then `prepare`/`commit` the triggering boundary against
  that state (a second CAS commit) — reloading and recomputing from scratch
  on `Conflict`, without ever re-capturing or re-pinning. A settled no-op
  result (a stale, rejected, or replayed attempt) is a successful return, not
  an error.

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

The runtime lock will guard the coordinator's own critical section (snapshot
capture, worktree/scope materialization, recovery, and the CAS retry loop)
once the public `coordinate()` entrypoint wires it in. It will be held on
every `coordinate()` call, unlike the checkout-identity-creation lock.

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
uses the same precedent, extended to real per-test `git init` repositories:
index/working-tree preservation across staged/unstaged/untracked/deleted
state, `.gitignore` exclusion, unborn-`HEAD` capture with and without files,
an unexpected `HEAD`-probe failure (a corrupted/missing `.git/HEAD`)
propagating as an error rather than a false empty-baseline capture, a
relative `repository_root` still resolving `git_dir` absolute, survival
after the temp index file is gone, `git gc --prune=now`/`git prune
--expire=now` survival for a pinned tree versus reclamation of a distinct
unpinned tree in the same repository, `pin_tree` idempotency, and
`diff_trees` output shape.

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
own failing capture.

## Status

The per-worktree runtime lock, the isolated Git snapshot service, and the
coordinator's internal protocol-integration pipeline (above) are implemented.
The public, lock-wrapped `coordinate()` entrypoint (resolving `git_dir`,
acquiring `WorktreeLock`, resolving checkout identity, deriving `WorktreeId`)
and cross-module integration tests remain future work tracked by the
`mutation-cursor-runtime-coordinator` plan's task stack. This file will grow
to cover them as each lands.

See also: [`mutation-trace-protocol.md`](mutation-trace-protocol.md),
[`mutation-trace-store.md`](mutation-trace-store.md),
[`checkout-identity.md`](checkout-identity.md).
