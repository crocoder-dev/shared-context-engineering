# Mutation-cursor external-taint boundary (`runtime::external_taint`)

The worktree-local durability boundary for the mutation cursor: a filesystem
marker that a later hook invocation reads as the external signal that the
previous invocation never proved a trustworthy durable completion. This is the
concrete runtime refinement of the abstract `ProtocolState.external_taint` /
`databaseFailure` / `recover` semantics in
[`mutation-trace-protocol.md`](mutation-trace-protocol.md) — it changes no
protocol semantics, adds no database state, and needs no migration.

Built by the `mutation-cursor-external-taint` plan
(`context/plans/mutation-cursor-external-taint.md`), which extends the
[`mutation-trace-runtime-coordinator.md`](mutation-trace-runtime-coordinator.md)
work.

## `ExternalTaintMarker` primitive

`cli/src/services/mutation_trace/runtime/external_taint.rs` —
`ExternalTaintMarker::new(git_dir: &Path)` builds a handle rooted at
`<git-dir>/sce/mutation-cursor-tainted`, the worktree-specific Git directory as
resolved by `checkout::resolve_git_dir`. Two linked worktrees resolve to two
different Git directories and therefore two independent markers.

The marker file is empty; **its existence is its entire state**:

- `exists() -> Result<bool>` — `symlink_metadata`; a `NotFound` is `Ok(false)`,
  any other inspection failure is `Err`.
- `persist() -> Result<()>` — `create_dir_all` the `sce/` directory,
  `create`-open the marker without truncation (`write(true).create(true).truncate(false)`,
  since the contents carry no meaning), `sync_data()` it, then a best-effort
  `#[cfg(unix)]` parent-directory `sync_all` whose error is not propagated.
  Idempotent: an existing marker is re-opened and re-synced, never rewritten.
- `clear() -> Result<()>` — `remove_file`, treating `NotFound` as success, then
  the same best-effort parent-directory sync. Idempotent.

Durability mirrors `checkout::persist_checkout_id_inner`: it protects against
process error, non-graceful process exit, `SIGKILL`, and normal runtime
restart — not host power loss or a filesystem-level crash (the parent-directory
sync is best-effort). The marker is never removed via `Drop`; only an explicit
`clear()` removes it. It is never authoritative for normal cursor state.

Every method — `new`/`exists`/`persist`/`clear` — is now reached by the
`coordinate()` fence (below), so the module carries no `allow(dead_code)`.

Inline `#[cfg(test)] mod tests` follows the unique-`std::env::temp_dir()`-path
precedent (see [`../patterns.md`](../patterns.md)): marker path is worktree
scoped, the marker survives reconstruction of the handle until an explicit
`clear()`, and `persist`/`clear` are idempotent.

## The write-ahead fence around `coordinate()`

The public entrypoint in
[`mutation-trace-runtime-coordinator.md`](mutation-trace-runtime-coordinator.md)
owns the whole protected operation and no longer receives an already-open DB
handle:

```text
coordinate(repository_root, boundary, open_db)
  open_db: impl FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>
```

Order inside the held `WorktreeLock`:

```text
resolve git_dir → acquire WorktreeLock
  → ExternalTaintMarker::new(git_dir)
  → inherited_external_taint = marker.exists()?
  → marker.persist()?                         ← fence armed here, write-ahead
  → get_or_create_checkout_id → WorktreeId
  → open_db()                                 ← DB acquired INSIDE the fence
  → GitSnapshotService::new
  → coordinate_boundary(&db, .., inherited_external_taint)
  → marker.clear()?                           ← only on a successful outcome
```

**Safety invariant:** no failure after the marker is armed — including a
failure to open the Agent Trace DB, or to resolve checkout identity — can
disappear without leaving the worktree-local signal for the next invocation.
Arming *before* `open_db()` is the whole point: if the DB open fails, the marker
is already on disk, so a later invocation that opens the DB successfully still
sees the inherited signal instead of trusting a lost interval.

The marker is cleared only by `coordinate()`'s success path (`marker.clear()`
after an `Ok` outcome). Every error path — snapshot failure, DB provider `Err`,
checkout-identity failure, DB read/write failure, CAS exhaustion, scope-identity
conflict, unexpected error — returns with the marker present. No `Drop` clears
it.

### `CoordinateError` variants

- `ExternalTaintMarker { operation: ExternalTaintOperation, source }` where
  `ExternalTaintOperation` is `Inspect | Persist`. Both operations run
  **before** any checkout-identity, DB, snapshot, or protocol work, so **no
  mutation boundary has committed** and there is no `CoordinateOutcome` to
  surface. The boundary is aborted fail closed — an unarmed fence must not let
  the boundary proceed.
- `MarkerClearAfterCommit { source, committed: Box<CoordinateOutcome> }` — the
  mutation boundary **committed successfully** to the Agent Trace DB (the
  `CoordinateOutcome`, including any `MutationEvent`, is durable), but the
  trailing `marker.clear()` failed. This is deliberately **not** the same shape
  as an `Inspect`/`Persist` failure: the committed outcome is carried in
  `committed` so the caller never loses access to it (a future harness must read
  it out of the error and still route the `MutationEvent` onward), the `Display`
  text says the boundary committed and only cleanup failed, and the marker stays
  logically armed so the next invocation conservatively recovers.
- `AgentTraceDbUnavailable(source)` — the caller-supplied `open_db` provider
  returned `Err` after the marker was armed. The marker is intentionally left in
  place; the lower-level `coordinate_boundary` pipeline is never entered.

Summary of the pre-commit vs. post-commit distinction:

```text
Inspect / Persist failure   → no mutation boundary committed;
                              no CoordinateOutcome exists;
                              boundary aborted fail closed.

Clear failure               → mutation boundary already committed;
(MarkerClearAfterCommit)      CoordinateOutcome (with any MutationEvent)
                              remains available in `committed`;
                              marker stays armed;
                              next invocation conservatively recovers.
```

### Inherited taint recovery

`coordinate()` reads `inherited_external_taint` from `marker.exists()` before
arming the marker and threads it into `coordinate_boundary`, which seeds an
invocation-local `external_taint_pending` flag. While that flag is set, each
freshly loaded projection is overlaid with `protocol::database_failure` before
the recovery check, so `protocol::recover` performs exactly one conservative
recovery against the single already-captured snapshot — cursor rebaselined to
the observed tree, revision advanced once, live scopes abandoned, no
`MutationEvent` for the fenced interval — and the triggering boundary is then
processed against the recovered state. A worktree with no durable row yet is
baselined against the observed tree first, then recovered the same way.

The overlay is never persisted: `DurableTransition` ignores `external_taint` and
`WorktreeProjection::into_protocol_state()` always returns it empty, so passing
the overlaid state as the CAS baseline is safe. A losing recovery CAS keeps
`external_taint_pending` set, so the next reload re-injects the overlay and
recomputes until recovery lands or the retry budget is spent; once it lands the
flag clears, so a later boundary-CAS retry in the same invocation does not
re-trigger recovery. The filesystem marker is never touched here —
`coordinate()`'s success path still owns clearing it.

A private `coordinate_inner(.., after_recovery)` / `coordinate_boundary_inner(..,
after_load, after_recovery)` seam lets tests inject a failure at the exact
transition *after* the recovery CAS returns `Applied` and *before* the triggering
boundary is prepared (production passes `|_| Ok(())`). It is never exposed
publicly.

Inline `coordinator.rs` tests drive the public `coordinate()` against real
repositories: a successful call clears the marker; a snapshot failure, a
non-snapshot failure (revision-exhausted recovery), and a DB provider returning
`Err` each leave the marker present; and both marker-I/O failure operations —
`Inspect` (a deterministic `ENOTDIR`, no permission changes) and `Persist` —
fail the call closed before the DB provider is ever invoked. Pipeline tests
cover the inherited-taint overlay directly: one conservative recovery sharing
the single snapshot before the boundary, a first-ever inherited marker with no
worktree row baselining without evidence, a losing recovery CAS re-injecting the
overlay until it applies, and the flag clearing so a post-recovery boundary-CAS
retry does not recover again.
`a_failure_after_recovery_before_boundary_commit_leaves_marker_and_forces_later_recovery`
uses the `after_recovery` seam to prove the recovery-committed / boundary-failed
window: the recovery is durable (cursor rebaselined, scope abandoned, revision
advanced), the triggering boundary is unprocessed with no `MutationEvent`, the
on-disk marker survives, and a later `coordinate()` inherits it and recovers
conservatively again without resurrecting the abandoned scope.
`runtime/tests.rs`'s
`a_marker_clear_failure_after_a_durable_boundary_keeps_the_marker_for_a_later_recovery`
proves an attributable `Advance` commits durably, the returned
`MarkerClearAfterCommit` carries the matching `committed` outcome
(`worktree_id` / `revision` / `observed_tree`, and `mutation_event.is_some()`),
and a later invocation still recovers off the still-armed marker.

## On-disk layout addition

```text
<worktree-git-dir>/sce/
└── mutation-cursor-tainted    (runtime::external_taint, empty; existence = armed)
```

See also: [`mutation-trace-runtime-coordinator.md`](mutation-trace-runtime-coordinator.md),
[`mutation-trace-protocol.md`](mutation-trace-protocol.md),
[`checkout-identity.md`](checkout-identity.md).
