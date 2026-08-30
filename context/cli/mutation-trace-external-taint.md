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

The module carries `#![allow(dead_code)]` (the `services/capabilities.rs`
precedent) for the parts not yet reached; `new`/`exists`/`persist`/`clear` are
armed by `coordinate()` (below).

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
  `ExternalTaintOperation` is `Inspect | Persist | Clear`. An `Inspect` or
  `Persist` failure is returned **before** any checkout-identity, DB, snapshot,
  or protocol work (fail closed — an unarmed fence must abort the boundary). A
  `Clear` failure is returned **after** a durable boundary, with the marker
  deliberately left in place for a later conservative re-recovery.
- `AgentTraceDbUnavailable(source)` — the caller-supplied `open_db` provider
  returned `Err` after the marker was armed. The marker is intentionally left in
  place; the lower-level `coordinate_boundary` pipeline is never entered.

### Inherited taint, not yet consumed

`coordinate()` computes `inherited_external_taint` from `marker.exists()` and
threads it to `coordinate_boundary` as `_inherited_external_taint`, unused so
far. Mapping an inherited marker into `protocol::database_failure` recovery
against the single captured snapshot — and re-injecting it across a losing
recovery CAS — is the next task in the same plan. `WorktreeProjection::
into_protocol_state()` still returns an empty `external_taint`; the filesystem
overlay is applied only by runtime code.

Inline `coordinator.rs` tests drive the public `coordinate()` against real
repositories: a successful call clears the marker; a snapshot failure, a
non-snapshot failure (revision-exhausted recovery), and a DB provider returning
`Err` each leave the marker present; and both marker-I/O failure operations —
`Inspect` (a deterministic `ENOTDIR`, no permission changes) and `Persist` —
fail the call closed before the DB provider is ever invoked.

## On-disk layout addition

```text
<worktree-git-dir>/sce/
└── mutation-cursor-tainted    (runtime::external_taint, empty; existence = armed)
```

See also: [`mutation-trace-runtime-coordinator.md`](mutation-trace-runtime-coordinator.md),
[`mutation-trace-protocol.md`](mutation-trace-protocol.md),
[`checkout-identity.md`](checkout-identity.md).
