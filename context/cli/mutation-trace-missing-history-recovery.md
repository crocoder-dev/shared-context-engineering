# Missing-pin repair and missing durable history (`ref_reconciliation::repair_missing_pins`)

The repair step for the target worktree `W` in [snapshot-ref reconciliation](mutation-trace-ref-reconciliation.md),
run by `sce doctor --fix` before the local-consistency check.

## Repair steps

The repository-scoped DB is keyed by repository identity, not by clone, and the
primary worktree is always `main`. A re-clone of the same remote therefore
inherits the old clone's `main` cursor and events while its pins and snapshot
objects stayed in the deleted `.git`. Before the local-consistency check,
`repair_missing_pins` handles `W`'s unpinned durable roots:

1. Classify each unpinned root with `git cat-file --batch-check`
   (`GitSnapshotService::absent_trees`). A non-tree object fails the pass with
   `ObjectInventory`.
2. If any root is absent from the object store and it is `W`'s own
   `cursor_tree`, fail with `UnrecoverableCursor` and change nothing. The message
   tells the operator to let one agent tool run in this checkout and rerun
   `sce doctor --fix` (see [case 2](#missing-durable-history-heuristic)).
3. Otherwise delete every `W` event whose `before_tree` or `after_tree` is absent,
   with its `mutation_trace_event_active_scopes` rows, in one transaction guarded
   by a CAS on `W`'s current revision (`MutationTraceStore::prune_mutation_events`).
   A lost CAS is `PruneConflict`. Such events can never be replayed by
   attribution, which already treats unreadable trees as a reconstruction
   barrier. The verified maintenance connection is read-only and Turso shares
   one database handle per path in a process, so the read-only connection is
   dropped and a writable one is opened on the same verified path only when a
   prune is needed.
4. Re-pin every remaining unpinned root (all present) in one atomic
   `git update-ref --no-deref --stdin` `create` transaction
   (`GitSnapshotService::pin_trees`).

Order makes the repair idempotent: a crash after the prune and before the re-pin
leaves only present unpinned roots, which the next pass re-pins.

## Missing durable history (heuristic)

Detection is a heuristic over the object store, built by the
`missing-history-recovery` plan. A durable tree absent from the current Git
object store is history this checkout cannot have produced or kept. It detects
**missing history, not clone identity**: no checkout ID, clone ID, marker, or
identity column exists, and `resolve_worktree_id` still returns `main` for the
primary worktree of every clone.
An existing tree is **not evidence of an unchanged clone**; a tree can exist
because it was pushed, fetched, or recreated from identical content. Concurrent
independent clones sharing one DB are out of scope.

Three cases for the target worktree `W`:

1. **Missing non-cursor trees.** Every `W` event whose `before_tree` or
   `after_tree` is absent is pruned with its active-scope rows, and the surviving
   unpinned roots are re-pinned. The pass completes and reports `pruned_events`
   and `repinned`.
2. **Missing cursor tree.** The pass fails closed with `unrecoverable_cursor` and
   changes nothing. The state is transient: the next committed agent hook
   boundary replaces `cursor_tree` with a freshly captured, pinned tree and the
   old cursor becomes an event `before_tree`, which case 1 then prunes on the
   next `sce doctor --fix`. No separate cursor-recovery transition exists.
3. **Existing cursor and event trees.** No destructive action and no claim that
   the clone is unchanged; reconciliation continues under the limitation above.

Repair runs only from the explicit `sce doctor --fix` pass. No agent hook
boundary prunes events, rewrites the cursor, or abandons scopes because of
absent trees, and plain `sce doctor` does not detect absent trees; it keeps the
existing reconciliation recommendation.

Invariants:

- **I1:** After a completed pass, every durable root of `W` (cursor and event
  trees) is present in the object store and pinned under `W`'s namespace.
- **I2:** An event is pruned only if one of its trees is absent; every event whose
  trees are all present survives.
- **I3:** The pass never changes `W`'s cursor; an absent cursor fails closed with
  nothing changed.
- **I4:** A failure at any step leaves either the pre-pass state or a state the
  next pass repairs (a prune committed before a failed re-pin leaves only present
  unpinned roots).
- **I5:** Stale-pin deletion keeps its repository-wide rule: a pin is deleted only
  if its tree is a durable root of no worktree.
