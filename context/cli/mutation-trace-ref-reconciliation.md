# Mutation-cursor snapshot-ref reconciliation (`runtime::ref_reconciliation`)

`cli/src/services/mutation_trace/runtime/ref_reconciliation.rs` is a
conservative, per-worktree maintenance pass that removes orphaned / unreferenced
SCE-owned snapshot pins **within one still-identifiable worktree's ref
namespace** while retaining every tree any current or historical durable
mutation-cursor state in the repository still references. Built by the
`mutation-cursor-ref-reconciliation` plan
(`context/plans/mutation-cursor-ref-reconciliation.md`). It is deliberately
**not** a guarantee that SCE snapshot refs never accumulate: a retired linked
worktree's namespace is beyond its reach (see
[Scope: an active, still-identifiable worktree namespace only](#scope-an-active-still-identifiable-worktree-namespace-only)).

`GitSnapshotService::pin_tree` is **create-only per invocation** — a crash,
failed transition, or other interrupted `coordinate()` path can leave a
`refs/sce/mutation-cursor/<worktree-id>/<tree-sha>` pin with no corresponding
durable root. Reconciliation is the reclamation step for exactly that state. It
is **not** a bound on storage growth: every retained `mutation_trace_events`
row keeps its `before_tree` / `after_tree` as durable roots, so a normal
successful history `A → B → C → D` keeps all four pins. Truly bounding
historical snapshot storage needs a separate future retention/compaction
lifecycle that this plan does not design.

The design is deliberately asymmetric: **keeping an unnecessary ref costs disk;
deleting a required ref destroys durable evidence.** False retention is
acceptable; false deletion is not.

`mod ref_reconciliation;` is private in `runtime/mod.rs` (like `mod
coordinator;`). `reconcile_worktree` is `pub` only within `runtime` — no
`pub(crate)` re-export — and nothing wires it into a hook, command, or the
`diff_traces` path yet; deciding *when* it runs is deferred to the
harness-wiring PR.

## Entry point and identity

```rust
pub fn reconcile_worktree(
    repository_root: &Path,
    open_db: impl FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
) -> Result<ReconciliationOutcome, ReconcileError>
```

A one-line delegation to `pub(super) fn reconcile_worktree_inner(..,
on_lock_contention: impl FnOnce())` — the deterministic test seam mirroring
`coordinate` / `coordinate_inner`, reachable from `runtime` and
`runtime::tests` (where the T04/T05 tests live) but invisible outside
`runtime`.

Like `coordinate()`, it never accepts a `WorktreeId`, `TreeId`, or ref name,
and never opens the DB itself: worktree identity is derived
`repository_root → checkout::resolve_git_dir → checkout::read_checkout_id`, and
`open_db` is a caller-supplied provider. `read_checkout_id → Ok(None)` (no
current checkout identity to derive an owned ref prefix from) returns
`Ok(ReconciliationOutcome::SkippedNoCheckoutIdentity)` — an observable skip,
**not** a zero-count report: no namespace inventoried, `open_db` never called,
no durable-root comparison, no ref deleted, lock released on return, no identity
created. It makes **no** claim that the repository holds no SCE mutation-cursor
refs for a prior checkout identity — those may be the stranded retired-worktree
namespaces below. `Err` is `ReconcileError::CheckoutIdentity` (a corrupt id, not an absent one).

## Scope: an active, still-identifiable worktree namespace only

Reconciliation reclaims orphan / unreferenced pins **only for a namespace whose
worktree is still present and identifiable**. The owned ref prefix is derived
from the *current* checkout identity of the worktree at `repository_root`
(`resolve_git_dir → read_checkout_id`), so a pass can only ever inventory
`refs/sce/mutation-cursor/<that-id>/`. High-frequency harness traffic against an
active worktree — where interrupted `coordinate()` runs leave orphan pins — is
exactly the case this pass reclaims under the `WorktreeLock`.

A per-worktree `<git-dir>/sce/checkout-id` disappears when its linked worktree
is removed (`git worktree remove`), but the
`refs/sce/mutation-cursor/<checkout-id>/*` refs it created live in the shared
repository ref namespace and survive. No surviving worktree can derive that
retired id, so no per-worktree pass can ever inventory or reconcile that
namespace — those refs are stranded.

```text
<worktree-git-dir>/sce/checkout-id                  per-worktree, gone with the worktree
refs/sce/mutation-cursor/<checkout-id>/<tree-sha>   repository-shared, survives

    git worktree remove W  →  checkout id gone  →  refs/.../<retired-id>/*
    unreachable by every per-worktree pass  →  stranded
```

**Harness gate.** A persistent / current-worktree harness lifecycle can rely on
this pass and is storage-cleanup complete for this module's scope. A harness
that **automatically creates and destroys ephemeral linked worktrees** must
**not** be treated as storage-cleanup complete until the repository-scoped
operation below exists — each removed worktree strands its whole namespace.
This is a limitation of scope, not a bug: it is orthogonal to the "reconciliation
≠ historical retention policy" boundary above (historical event trees stay
pinned because they remain durable roots; retired namespaces stay pinned because
no pass can reach them).

### Future work: repository-scoped retired-worktree reconciliation

Not implemented here — this module adds **no** repository-global namespace scan
and **no** repository-global ref deletion. Recorded shape:

```text
enumerate refs/sce/mutation-cursor/<id>/*   (git for-each-ref on the namespace)
        ↓
active checkout ids                         (git worktree list → each worktree's checkout-id)
        ↓
retired ids                                 (namespace present, no active worktree owns it)
        ↓
for each retired namespace, each pinned tree T:
    T ∉ durable_roots(repository)  →  delete refs/sce/mutation-cursor/<id>/T
    T ∈ durable_roots(repository)  →  retain
```

It must inherit this module's guarantees: the repository-wide durability
invariant (`delete <retired-W>/T` only if `T ∉ durable_roots(repository)`, read
through `load_all_tree_roots()`) because a retired worktree may still own
historical `mutation_trace_events` rows other tooling needs; the
false-retention-over-false-deletion bias; and a hard prohibition on the shortcut
"namespace has no active worktree → delete the whole namespace". It is a
separate PR — it needs the repository-global namespace scan and cross-worktree
active-id inventory this module deliberately omits — gated behind the same
harness-wiring work as invocation timing. This module's per-worktree pass
remains the correct mechanism for high-frequency harness traffic against
**active** worktrees.

## Two invariants

Conflating them — deciding deletion from the target worktree's roots alone — is
the cross-worktree safety bug this design exists to avoid. Linked worktrees
share one Git object database, so an `A`-owned ref can be the last SCE ref
protecting a tree that only worktree `B` durably requires.

```mermaid
flowchart TD
    inv["list_pins(W) — actual pins under refs/sce/mutation-cursor/&lt;W&gt;/"]
    local["load_tree_roots(W)\n(this worktree's cursor + event trees)"]
    repo["load_all_tree_roots()\n(every worktree's cursor + event trees)"]
    inv --> lc
    local --> lc{"local consistency:\ndurable_roots(W) ⊆ pinned_trees(W)?"}
    lc -- "no" --> fail["ReconcileError::MissingRequiredPins\n— fail closed, delete nothing"]
    lc -- "yes" --> ds
    inv --> ds{"deletion safety:\npin.tree ∉ durable_roots(repository)?"}
    repo --> ds
    ds -- "stale" --> del["delete_pins(stale) — one atomic\ngit update-ref --no-deref --stdin"]
    ds -- "retained" --> keep["keep the pin"]
```

- **Local consistency** (`load_tree_roots(W)`) is strictly per-worktree. A
  missing pin in some *other* worktree never makes `W`'s pass fail — that would
  let one worktree's degradation block maintenance everywhere.
- **Deletion safety** (`load_all_tree_roots()`) is repository-wide. `W/T` is
  deleted only when `T` is in **no** worktree's durable root set. If `B`
  durably needs `T` and `A` also has a `T` pin, `A` retains it — `A`'s
  otherwise-stale ref then supplies accidental backup reachability for `B`'s
  degraded state (`reconcile_a_retains_its_pin_when_another_worktree_durably_requires_the_same_tree`).

A DB `TreeId` is a *logical* durability requirement, not itself a Git
reachability edge: it obliges reconciliation to keep at least one SCE ref
protecting that tree, and that retained ref is what supplies physical Git
reachability. Each root-set query is [one SQL statement over one DB
snapshot](mutation-trace-store.md), which is what keeps a concurrent atomic
`cursor T → X` + `event T → X` commit on another worktree from tearing the
repository-wide read — no repository-global lock is needed.

## Locking

Reconciliation holds the **same** `<git-dir>/sce/mutation-cursor.lock`
`WorktreeLock` that `coordinate()` holds across `pin → CAS → return`, acquired
via `worktree_lock::acquire_inner` and bounded by the module-owned
`const RECONCILIATION_LOCK_TIMEOUT: Duration = Duration::from_secs(10)` (its
value matches the coordinator's private `WORKTREE_LOCK_TIMEOUT` by intent, not
a shared constant). Mutual exclusion on that one file makes the pin → DB-CAS
race structurally impossible: the reconciler's inventory → diff → delete runs
wholly before `coordinate()` takes the lock (tree not pinned yet) or wholly
after it releases it (tree committed → a durable root → retained; or never
committed → a true orphan → safe to delete). The lock is per-worktree; only the
retention *read* is repository-wide.

## Algorithm and error contract

Every step runs under the lock, and every fallible step maps to one dedicated
`ReconcileError` variant — there is no `Other` catch-all:

| Step | Error on failure |
| --- | --- |
| `resolve_git_dir` | `GitDir` |
| `acquire_inner(RECONCILIATION_LOCK_TIMEOUT)` | `Lock(WorktreeLockError)` |
| `read_checkout_id` → `Err` (corrupt id) | `CheckoutIdentity` |
| `open_db()` provider | `AgentTraceDbUnavailable` |
| `GitSnapshotService::new` | `SnapshotService` |
| `list_pins` → `PinInventoryError::Git` | `PinInventory` |
| `list_pins` → `PinInventoryError::MalformedRef` | `MalformedPin { ref_name, reason }`, delete nothing |
| `load_tree_roots` / `load_all_tree_roots` | `DurableRoots` |
| a target-worktree root has no pin | `MissingRequiredPins { missing }`, delete nothing |
| `delete_pins` transaction | `DeleteTransaction`, delete nothing (atomic-or-nothing) |

`AgentTraceDbUnavailable` here is a **maintenance** error only: reconciliation
never arms `ExternalTaintMarker`, never calls `protocol::*`, never writes any
`mutation_trace_*` row, and a failure never becomes
`CoordinateError::AgentTraceDbUnavailable` — no mutation boundary is being
coordinated (contrast [`mutation-trace-external-taint.md`](mutation-trace-external-taint.md)).

## Outcome and report

Both entrypoints return `Result<ReconciliationOutcome, ReconcileError>` —
`Reconciled(ReconciliationReport)` | `SkippedNoCheckoutIdentity`, the skip an
`Ok` (never `Err`) and distinct from a zero-work `Reconciled(.. { deleted: 0 })`.

`ReconciliationReport { local_required: usize, retained: usize, deleted: usize }`
— `local_required = load_tree_roots(W).len()`, `deleted` = stale pins removed,
`retained = actual.len() − deleted`. `retained == local_required` is **not**
an invariant (a pin another worktree needs counts toward `retained` only); the
only `Ok`-path relation is `local_required ≤ retained`.

## Model boundary

Ref reconciliation is imperative durability maintenance **below** the verified
`spec/mutation_cursor.qnt` protocol — it never advances the cursor, chooses
attribution, changes scope state, or creates a `MutationEvent`, so no Quint or
refinement-matrix change is warranted. It deletes only SCE-owned refs, never
Git objects; Git reclaims the now-unreachable objects itself on its own GC
schedule (this pass runs no `git gc` / `git prune` / `git reflog expire`).

## Testing boundary

`ref_reconciliation.rs`'s inline `#[cfg(test)] mod tests` uses a RAII
`Fixture` (`tempfile::TempDir`, real `git init` repo, checkout id via
`get_or_create_checkout_id`, a schema `RepositoryAgentTraceDb` at a path
**beside** the worktree so it never perturbs a captured tree) with raw-SQL row
seeders, following the filesystem-touching inline-test precedent
(`context/patterns.md`). Coverage: orphan pin deleted (with and without a
worktree row); current-cursor pin retained without a referencing event;
historical `before`/`after` pins retained after the cursor advances; a pin
another worktree durably requires retained (also the `retained > local_required`
count case); `MissingRequiredPins` fail-closed; a malformed namespace ref
(symbolic ref) fail-closed; idempotence; refs deleted without object
reclamation (`git cat-file -t` still resolves the orphan tree); and the
no-checkout-identity skip returning `SkippedNoCheckoutIdentity` rather than a
zero-count `Reconciled` report (`no_checkout_identity_returns_a_distinct_skipped_outcome`
and `a_missing_checkout_identity_skip_touches_no_db_and_no_ref`, the latter with
an `open_db` provider that panics if invoked). The deterministic pin→CAS lock-race
regression and the cross-module linked-worktree scenarios live in
`runtime/tests.rs` (plan tasks T04/T05).

See also:
[`mutation-trace-runtime-coordinator.md`](mutation-trace-runtime-coordinator.md),
[`mutation-trace-snapshot-service.md`](mutation-trace-snapshot-service.md)
(`list_pins` / `delete_pins`),
[`mutation-trace-store.md`](mutation-trace-store.md)
(`load_tree_roots` / `load_all_tree_roots`),
[`mutation-trace-protocol.md`](mutation-trace-protocol.md).
