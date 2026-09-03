# Mutation-trace Git snapshot service (`runtime::git_snapshot`)

`cli/src/services/mutation_trace/runtime/git_snapshot.rs` — the isolated Git
snapshot and ref-pinning service the
[runtime coordinator](mutation-trace-runtime-coordinator.md) uses to make a
worktree's state durable in the repository's own object database. It writes
tree/blob objects into the repository's normal, shared Git object database and
protects the durable ones with refs under
`refs/sce/mutation-cursor/<worktree-id>/<tree-sha>`, rather than maintaining a
private object store.

## Construction

`GitSnapshotService::new(repository_root: &Path) -> Result<GitSnapshotService>`
resolves `git_dir` once via `git rev-parse --absolute-git-dir`, so `git_dir` is
always an absolute path — even when the caller's `repository_root` is relative,
which matters because every Git subprocess this service spawns runs with
`cwd = repository_root` and `GIT_DIR = git_dir`; a relative `git_dir` would
otherwise be resolved by the child process against its own
already-`repository_root`-joined `cwd`, double-joining the path.

## Capture and pin

`capture_tree(&self) -> Result<TreeId>` snapshots the current worktree (staged,
unstaged, untracked, and deleted state, respecting `.gitignore`) into the
repository's normal, shared Git object database, never touching the real index
or working tree: it reserves a unique `<git-dir>/sce/tmp/index-<uuid>` path via
an RAII guard (never pre-creating the file), probes `HEAD` via a dedicated
`head_exists` helper that inspects the Git exit status directly — status `0`
means `HEAD` resolves, status `1` is `--verify --quiet`'s documented "does not
resolve" signal (a genuinely unborn `HEAD`), and every other status propagates
as an error rather than being treated as empty, since HEAD absence is a normal
Git state but a HEAD-probe failure is a snapshot failure — then runs
`git read-tree HEAD` or, on a genuinely unborn `HEAD`, the explicit
`git read-tree --empty` (never a bare/absent index file), then `git add -A -- .`,
then `git write-tree`, all with only `GIT_DIR`/`GIT_INDEX_FILE` set — no
`GIT_OBJECT_DIRECTORY`/`GIT_ALTERNATE_OBJECT_DIRECTORIES` override anywhere.
`TreeId` is an opaque string; nothing assumes a fixed length, so a SHA-256
repository needs no special handling.

`pin_tree(&self, worktree_id, tree) -> Result<()>` makes a tree durable by
creating `refs/sce/mutation-cursor/<worktree_id>/<tree-sha>` via
`git update-ref` — create-only and idempotent for the same
`(worktree_id, tree)` pair — which is what makes a pinned tree survive
`git gc --prune=now`/`git prune --expire=now`, unlike an unpinned, unreachable
tree in the same repository.

`diff_trees(&self, before, after) -> Result<String>` runs
`git diff --binary --full-index --no-ext-diff --no-textconv` between two tree
SHAs, returning the raw diff text `patch.rs::parse_patch` already knows how to
parse.

## Pin inventory and conditional deletion

**Mutation-cursor pins are direct refs.** Every valid pin is a direct ref
`refs/sce/mutation-cursor/<worktree-id>/<tree-sha>` pointing straight at a tree
object. A **symbolic ref anywhere inside the SCE mutation-cursor namespace is
malformed and rejected** — never followed or normalized — because a symref
under worktree `A`'s prefix could resolve through worktree `B`'s ref, so a pass
reasoning about `A`'s pins would silently be acting on `B`'s.

`list_pins(&self, worktree_id) -> Result<Vec<PinnedRef>, PinInventoryError>`
inventories the SCE snapshot pins owned by one worktree — `git for-each-ref`
constrained to the single prefix `refs/sce/mutation-cursor/<worktree_id>/`,
with a NUL-separated `%(refname)%00%(objectname)%00%(objecttype)%00%(symref)`
format. Each ref must satisfy all of: under the exact target prefix; no extra
path segment; `%(symref)` empty (a direct ref); target object type `tree`;
final path component equal to the target SHA. A `git for-each-ref`
execution/exit failure is `PinInventoryError::Git`; any malformed ref inside
the namespace (symbolic ref, non-tree target, name/target SHA mismatch,
unparseable line, extra path segment) is
`PinInventoryError::MalformedRef { ref_name, reason }`, matchable separately.

`delete_pins(&self, pins: &[PinnedRef]) -> Result<()>` removes exactly the
supplied pins in one atomic **`git update-ref --no-deref --stdin`**
transaction, each `delete` conditioned on the inventoried tree SHA. Two
independent safety properties:

- **Atomic** — every command commits together at end of input; if any fails
  (including a failed old-value check) the whole transaction aborts and no ref
  is changed.
- **No dereference** — `--no-deref` makes every `delete` act on the exact ref
  name given, never on a ref reached by resolving a symbolic ref. A
  direct-ref → symbolic-ref race between inventory and deletion therefore
  cannot cause `delete_pins` to touch the symref's target (for example a ref
  owned by another worktree).

Before issuing the transaction, `delete_pins` re-inventories the exact supplied
ref names in one `git for-each-ref` (order-independent) and fails closed unless
every one is still present, still a direct ref to a tree, and still pointing at
the recorded SHA — preferring a clean `Err` over acting on unexpected namespace
state. An empty slice is a successful no-op.

These are two independent defenses: the preflight catches the common
inventory→delete change (deleted, retargeted, or turned into a symref) before
any `git update-ref` runs; the per-`delete` expected-old-value check inside the
atomic transaction catches a change that lands *after* preflight has passed and
commits nothing. A private test-only `delete_pins_inner(pins, after_preflight)`
seam (production passes a no-op hook) fires between the two so the atomicity
property can be tested with the transaction actually issued.

`REF_NAMESPACE` (`refs/sce/mutation-cursor`) and the private `pin_ref_prefix`
helper are the single source of truth for the pin path; `pin_tree`,
`list_pins`, and `delete_pins` all derive their ref names from it.

## Callers

`coordinator.rs` is the only caller of `capture`/`pin`/`diff_trees` so far, via
its `SnapshotCapture` trait; `list_pins` / `delete_pins` have no caller yet —
the deferred per-worktree ref-reconciliation maintenance pass is their first
consumer.

## Testing boundary

`GitSnapshotService`'s inline `#[cfg(test)] mod tests` uses the
filesystem-touching inline-unit-test precedent (see `context/patterns.md`),
extended to real per-test `git init` repositories: index/working-tree
preservation across staged/unstaged/untracked/deleted state, `.gitignore`
exclusion, unborn-`HEAD` capture with and without files, an unexpected
`HEAD`-probe failure (a corrupted/missing `.git/HEAD`) propagating as an error
rather than a false empty-baseline capture, a relative `repository_root` still
resolving `git_dir` absolute, survival after the temp index file is gone,
`git gc --prune=now`/`git prune --expire=now` survival for a pinned tree versus
reclamation of a distinct unpinned tree in the same repository, `pin_tree`
idempotency, `diff_trees` output shape, worktree-scoped `list_pins` inventory
(prefix isolation, a symbolic ref inside the namespace rejected as
`MalformedRef` with `B/T` left intact, other malformed-ref rejection matchable
separately from a `for-each-ref` execution failure), and `delete_pins`
conditional atomic batch deletion (exact removal, empty-slice no-op). Its two
delete defenses are proven separately:

- **Preflight revalidation** — an inventoried direct ref turned into a symbolic
  ref before the call makes `delete_pins` return `Err` and spawn no
  `git update-ref` at all, leaving both the symref and its cross-worktree
  target untouched.
- **Expected-old-value atomic Git transaction** — proven through the private
  test-only `delete_pins_inner(pins, after_preflight)` seam (production
  `delete_pins` passes a no-op hook). The test pins two valid trees, passes the
  explicit `[valid, mismatched]` batch through preflight, then uses
  `after_preflight` to retarget the second ref so
  `git update-ref --no-deref --stdin` is genuinely issued; git's per-`delete`
  old-value check aborts the batch and the first (valid) ref is left intact. A
  sequential conditional-delete implementation in input order would commit the
  first delete and fail this.

See also: [`mutation-trace-runtime-coordinator.md`](mutation-trace-runtime-coordinator.md),
[`mutation-trace-protocol.md`](mutation-trace-protocol.md),
[`mutation-trace-store.md`](mutation-trace-store.md).
