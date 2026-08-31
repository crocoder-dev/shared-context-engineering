# Plan: mutation-cursor-ref-reconciliation

## Change summary

The mutation-cursor runtime pins every captured Git tree under
`refs/sce/mutation-cursor/<worktree-id>/<tree-sha>` in the repository's normal,
shared object database and refs namespace, so a durable snapshot stays
resolvable through `git gc`/`git prune`
(`context/cli/mutation-trace-runtime-coordinator.md`,
`context/plans/mutation-cursor-runtime-coordinator.md`). Those pins are
**create-only** today: `GitSnapshotService::pin_tree` adds a ref on every
`coordinate()` invocation and nothing ever removes one, so storage grows
without bound in proportion to mutation-cursor invocation volume — every
distinct tree a worktree ever observes leaves one permanent ref and the
objects it protects.

This plan adds a **conservative per-worktree ref reconciliation pass** that
removes only SCE-owned pins that are provably outside the **repository-wide**
durable mutation-cursor root set and provably cannot belong to an in-flight
mutation-cursor transition. It is the deferred step 3 of the runtime
completion sequence in `context/plans/mutation-cursor-runtime-coordinator.md`
("Follow-up PR — Runtime completion sequence"), required before any harness
adapter becomes a production consumer of `coordinate()`.

The design is deliberately asymmetric: **keeping an unnecessary ref costs disk
space; deleting a required ref destroys durable evidence.** False retention is
acceptable; false deletion is not. The pass therefore:

- reads two durable root sets through new bounded, read-only
  `MutationTraceStore` queries: the target worktree's own durable tree roots
  (`load_tree_roots`), and the union of durable tree roots across **every**
  worktree in the repository (`load_all_tree_roots`) — linked worktrees share
  one Git object database, so a ref owned by worktree A may be the last SCE
  ref protecting a tree that only worktree B durably requires — and it is that
  retained ref, not B's database row, that keeps the object reachable to Git.
  Each of these APIs executes **exactly one SQL statement** — a `UNION` of the
  `cursor_tree`, `before_tree`, and `after_tree` columns — so its complete
  logical root set is observed through a single coherent database snapshot,
  never assembled in Rust from multiple independent `SELECT`s that a
  concurrent mutation-cursor commit could tear across;
- lists only that worktree's pins, validating each against its ref target;
- if any of the **target worktree's own** durable roots has **no** live pin,
  fails closed and deletes nothing (the local consistency invariant — a
  per-worktree check);
- otherwise deletes exactly the target worktree's pins whose tree is outside
  the **repository-wide** durable root set (the deletion safety invariant),
  in one atomic `git update-ref --stdin` transaction, each delete conditioned
  on the tree SHA observed at inventory time;
- does all of this while holding that worktree's existing
  `<git-dir>/sce/mutation-cursor.lock` (`WorktreeLock`), the same lock
  `coordinate()` holds across `pin → recovery → prepare → CAS → marker clear
  → return`, which is what makes the pin→CAS race structurally impossible.

This extends the existing `mutation-cursor-runtime-coordinator` and
`mutation-cursor-external-taint` work. It changes no mutation-cursor protocol
semantics, adds no database state, needs no migration, and does not touch the
`ExternalTaintMarker`, `protocol.rs`, `spec/mutation_cursor.qnt`, or the Quint
refinement matrix. It does not run `git gc`/`git prune` — it removes only the
SCE refs Git's own GC already knows how to act on, and lets Git reclaim the
now-unreachable objects on its own schedule. It adds no harness, hook, or
command wiring; `reconcile_worktree()` stays reachable only from within
`runtime`, exactly like `coordinate()`.

## Core invariants

Two separate invariants govern the pass. Conflating them — deciding deletion
from the target worktree's roots alone — is the cross-worktree safety bug this
design exists to avoid.

### Local consistency invariant

For the target worktree `W`:

```
durable_roots(W) ⊆ pinned_trees(W)
```

If this is false — some tree `W`'s own durable evidence references has no live
pin — reconciliation **fails closed and deletes nothing**
(`ReconcileError::MissingRequiredPins`). This check is strictly per-worktree:
a missing pin in some *other* worktree `B` never makes `A`'s pass fail.

### Deletion safety invariant

A ref `refs/sce/mutation-cursor/<W>/T` owned by `W` may be deleted only if `T`
is absent from the durable roots of **every** worktree in the repository:

```
delete W/T   ⟺   T ∉ durable_roots(repository)
              =  T ∉ ⋃_{V ∈ worktrees} durable_roots(V)
```

A durable DB `TreeId` does not itself make a Git object reachable. It is a
**logical durability requirement**: it tells reconciliation that at least one
SCE Git ref protecting that tree must be retained, and that retained Git ref
is what supplies **physical Git reachability** to Git GC. Linked worktrees
share one Git object database, so an `A`-owned ref can be the last SCE ref
protecting a tree that only worktree `B` durably requires
(`B`'s `cursor_tree`, or a `before_tree` / `after_tree` of one of `B`'s
historical events). Deciding staleness as `actual_A − durable_roots(A)` alone
would let `A`'s pass delete that last ref, leaving nothing to protect `B`'s
durable cursor/evidence tree from a later `git gc`. The **retention set** is
therefore repository-wide; the
**lock** stays per-worktree (Q1, Q2). If `B` requires `T` and `A` also has a
`T` pin, `A` retains it — `A`'s otherwise-stale ref then acts as conservative
accidental backup reachability for `B`'s degraded state.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation. All `cargo` invocations go through
`./scripts/run-cli-cargo.sh` (per `context/patterns.md`); test paths use the
crate module path `services::mutation_trace::runtime::…` /
`services::mutation_trace::store::…`.

- [ ] AC1: The store layer exposes two durable-root reads and each is exact.
  `load_tree_roots(W)` returns exactly the union of `W`'s
  `mutation_trace_worktrees.cursor_tree` and every
  `mutation_trace_events.before_tree` / `mutation_trace_events.after_tree` row
  for `W`, deduplicated — and nothing from any other worktree, any other
  table, or `AttemptState`/`external_taint`; a worktree with no durable row
  yields the empty set (not an error). `load_all_tree_roots()` returns the
  union of those same three `TreeId` columns across **every** worktree in the
  repository, deduplicated; an empty repository yields the empty set (not an
  error). A tree that two worktrees both reference appears once in
  `load_all_tree_roots()`.
  Each root-set API executes **exactly one SQL statement** covering the
  cursor, before-tree, and after-tree roots (a `UNION` of the three columns,
  driven through a single `query_map` call), so a concurrent mutation-cursor
  commit — which atomically moves `cursor_tree` from `T` to `X` and inserts a
  `MutationEvent { before_tree = T, after_tree = X }` in the same transaction —
  cannot expose a mixed pre/post-commit root set: the one statement observes
  either the pre-commit snapshot (`cursor_tree` still contains `T`) or the
  post-commit snapshot (`before_tree` contains `T`), and `T` is retained under
  both. There is no snapshot in which `cursor_tree` no longer contains `T`
  while `before_tree` does not yet contain it. This must not depend on any
  ordering of separate reads ("query cursor first"); it is a structural
  property of the single-statement snapshot. A new DB transaction API is
  **not** required if one `SELECT`/`UNION` statement already provides these
  snapshot semantics.

  Two separable properties, proven by two separate tests, must not be
  conflated:

  - **State-transition retention** (`…retains_previous_cursor_after_atomic_cursor_advance`):
    before an atomic `cursor T → X` + `event T → X` advance, `T` is a durable
    root via `cursor_tree`; after it, `T` is a durable root via `before_tree`.
    A pre/post read straddling the advance still sees `T` both times. This is
    necessary but **not sufficient** — a torn multi-read implementation would
    also pass it, so this test is explicitly *not* evidence of snapshot
    isolation.
  - **Single-statement snapshot enforcement**
    (`…reads_every_durable_root_in_one_sql_statement`, for both
    `load_all_tree_roots` and `load_tree_roots`): the deterministic regression
    for the actual concurrency boundary. It (a) constructs the torn set
    explicitly — an events read, the atomic advance committed between the
    reads, then a worktrees read, unioned in Rust, which loses `T` — and (b)
    asserts, via the `TursoDb` read-statement counter
    (`crate::services::db::count_read_statements`), that one production
    `load_*_tree_roots` call issues **exactly one** read statement, so it can
    never enter that interleaving and always retains `T`. Reimplementing
    either query as two independent `SELECT`s (cursor, then events, or events,
    then cursor) makes the counter observe `2` and fails the test. **One SQL
    statement is the concurrency boundary** because it is the unit of DB
    snapshot isolation: everything the statement reads comes from a single
    coherent MVCC snapshot, whereas two statements are two snapshots a
    concurrent commit can fall between.
  - Validate: `services::mutation_trace::store::tests::load_tree_roots_returns_cursor_and_every_event_tree_deduplicated`,
    `services::mutation_trace::store::tests::load_tree_roots_excludes_other_worktrees_trees`,
    `services::mutation_trace::store::tests::load_tree_roots_is_empty_for_an_unmaterialized_worktree`,
    `services::mutation_trace::store::tests::load_tree_roots_remains_worktree_scoped`,
    `services::mutation_trace::store::tests::load_all_tree_roots_returns_every_worktree_cursor_and_event_tree_deduplicated`,
    `services::mutation_trace::store::tests::load_all_tree_roots_deduplicates_a_tree_shared_by_multiple_worktrees`,
    `services::mutation_trace::store::tests::load_all_tree_roots_is_empty_for_an_empty_repository`,
    `services::mutation_trace::store::tests::load_all_tree_roots_retains_previous_cursor_after_atomic_cursor_advance` (state-transition retention only — not proof of snapshot isolation),
    `services::mutation_trace::store::tests::load_all_tree_roots_reads_every_durable_root_in_one_sql_statement` and
    `services::mutation_trace::store::tests::load_tree_roots_reads_every_durable_root_in_one_sql_statement` (the deterministic single-statement snapshot regression: torn two-read set constructed explicitly, then production path asserted to issue exactly one read statement via `count_read_statements`)
- [ ] AC2: An orphan pin — a pinned tree that is in no durable root anywhere
  in the repository, the observable post-crash / post-no-op state `pin exists
  ∧ durable root does not` — is deleted by reconciliation, whether or not the
  worktree has a durable row at all; `git for-each-ref` no longer lists that
  ref afterward.
  The reconciler does not care how that orphan state arose, so the tests
  construct it directly: capture a tree, pin it via `GitSnapshotService`,
  create no durable root for it, then run the pass.
  - Validate: `services::mutation_trace::runtime::ref_reconciliation::tests::orphan_pin_with_a_worktree_row_is_deleted`,
    `services::mutation_trace::runtime::ref_reconciliation::tests::orphan_pin_with_no_worktree_row_is_deleted`,
    and end-to-end `services::mutation_trace::runtime::tests::a_pin_with_no_durable_root_is_reclaimed_by_a_later_reconciliation`
- [ ] AC3: A pin whose tree is the worktree's current
  `mutation_trace_worktrees.cursor_tree` survives reconciliation even when no
  `mutation_trace_events` row references that tree.
  - Validate: `services::mutation_trace::runtime::ref_reconciliation::tests::current_cursor_pin_is_retained_without_a_referencing_event`
- [ ] AC4: Pins for the `before_tree` and `after_tree` of historical
  `mutation_trace_events` rows survive reconciliation after the worktree's
  cursor has moved on to a later tree, so a future `diff_trees(before, after)`
  over that historical interval stays possible.
  - Validate: `services::mutation_trace::runtime::ref_reconciliation::tests::historical_event_before_and_after_pins_are_retained_after_the_cursor_advances`
- [ ] AC5: A reconciliation pass cannot observe or delete a pin that a
  concurrent `coordinate()` has created but not yet committed durably. A
  deterministic test holds one worktree's `WorktreeLock`, starts
  `ref_reconciliation::reconcile_worktree_inner` (the `pub(super)` seam) on
  another thread with a channel-signalling `on_lock_contention` closure and
  proves it blocks on that same lock, then — still holding the lock — makes a
  pinned tree X a durable root, releases the lock, and asserts the
  now-unblocked reconciliation retains X (`deleted == 0`). No sleeps; the
  proof is the lock happens-before edge.
  - Validate: `services::mutation_trace::runtime::tests::reconciliation_blocks_on_the_worktree_lock_and_retains_a_pin_that_becomes_durable_under_it`
- [ ] AC6: Given target worktree W whose local durable root set is `{A, B}`,
  W's pins `{A, X}` (root B has no pin), and no other worktree durably
  referencing X, reconciliation returns a distinct `missing-required-pins`
  error naming B, deletes zero refs, and leaves both A's and X's pins in
  place.
  - Validate: `services::mutation_trace::runtime::ref_reconciliation::tests::a_missing_required_pin_fails_closed_and_deletes_nothing`
- [ ] AC7: A ref inside `refs/sce/mutation-cursor/<worktree-id>/` whose target
  is not a tree object, or whose name suffix disagrees with its target SHA, or
  whose `for-each-ref` line does not parse, makes `list_pins` return
  `PinInventoryError::MalformedRef { ref_name, reason }`, which
  `reconcile_worktree` maps to `ReconcileError::MalformedPin { ref_name,
  reason }`; reconciliation then deletes nothing.
  - Validate: `services::mutation_trace::runtime::git_snapshot::tests::list_pins_rejects_a_ref_whose_target_is_not_a_tree`,
    `services::mutation_trace::runtime::git_snapshot::tests::list_pins_rejects_a_ref_whose_name_disagrees_with_its_target`,
    `services::mutation_trace::runtime::ref_reconciliation::tests::a_malformed_namespace_ref_fails_closed_and_deletes_nothing`
- [ ] AC8: Running reconciliation twice with no intervening state change:
  the first run deletes the stale pins and returns success; the second run
  deletes zero and returns success with identical `local_required`/retained
  counts.
  - Validate: `services::mutation_trace::runtime::ref_reconciliation::tests::reconciliation_is_idempotent`
- [ ] AC9: With linked worktrees A and B sharing one object database and ref
  namespace, `reconcile_worktree` for A operates only under
  `refs/sce/mutation-cursor/<A-id>/`, acquires only A's `WorktreeLock`, leaves
  every `refs/sce/mutation-cursor/<B-id>/` ref untouched, requires no pause in
  B's coordinating, and does not make unresolvable any object that a B ref
  still names, **or that a B durable root still requires SCE to protect with a
  retained ref** — even when A and B pinned byte-identical tree content. (A B
  durable root is a logical durability requirement, not itself a Git
  reachability edge: it obliges reconciliation to keep at least one SCE Git ref
  protecting that tree, and that retained ref is what keeps the object
  reachable to Git.) This includes the cross-worktree
  degraded-state case: B durably references a tree T, B's own
  `refs/sce/mutation-cursor/<B-id>/T` is deliberately absent, A owns
  `refs/sce/mutation-cursor/<A-id>/T`, and A does not durably reference T —
  `reconcile_worktree(A)` must **retain** `refs/sce/mutation-cursor/<A-id>/T`
  because T is a repository-wide durable root, and the retained ref — not B's
  database row itself — keeps T resolvable via
  `git cat-file -t` afterward. This is the canonical proof that reconciliation
  cannot convert another worktree's degraded-but-recoverable state into
  evidence loss.
  - Validate: `services::mutation_trace::runtime::tests::reconcile_one_linked_worktree_leaves_the_other_worktrees_pins_and_shared_objects_intact`,
    `services::mutation_trace::runtime::tests::reconcile_a_retains_its_pin_when_another_worktree_durably_requires_the_same_tree`
- [ ] AC10: The stale-pin deletion is one atomic `git update-ref --stdin`
  transaction in which every `delete` is conditioned on the tree SHA recorded
  at inventory time; if any inventoried ref no longer points at that SHA when
  the transaction runs, the whole transaction aborts and **no** ref is
  deleted. This is proven directly against `GitSnapshotService::delete_pins`
  (inventory `R → X`; mutate `R → Y`; call `delete_pins` with the inventoried
  `R → X`; assert the transaction fails and no ref was deleted); the public
  `reconcile_worktree` path is only asserted to route its stale batch through
  `delete_pins`, not to independently schedule a mid-pass race.
  - Validate: `services::mutation_trace::runtime::git_snapshot::tests::delete_pins_aborts_the_whole_transaction_when_one_ref_no_longer_matches_its_expected_value`
- [ ] AC11: A reconciliation pass performs no mutation-cursor protocol or
  durability write: after a pass, `mutation_trace_worktrees` /
  `mutation_trace_scopes` / `mutation_trace_events` /
  `mutation_trace_processed_events` / `mutation_trace_event_active_scopes` row
  counts and the target worktree's `revision` / `tainted` / `failure_kind` /
  `needs_rebaseline` / `cursor_tree` are byte-identical to before; no
  `<git-dir>/sce/mutation-cursor-tainted` marker is created; and
  `cli/migrations/agent-trace-repository/` still contains exactly
  `001`/`002`/`003`.
  - Validate: `services::mutation_trace::runtime::tests::reconciliation_makes_no_protocol_or_marker_write`;
    inspection: `ls cli/migrations/agent-trace-repository/` shows only the three existing files
- [ ] AC12: A reconciliation pass invokes no `git gc` / `git prune` / `git
  reflog expire` and no object-reclaiming command. Set up an object O that is
  reachable before the pass **only** through a stale SCE pin
  `refs/sce/mutation-cursor/<W>/O` (O is in no durable root). Reconciliation
  removes that stale ref; immediately afterward O is unreachable, yet
  `git cat-file -t` still resolves O — because reconciliation deleted the ref
  but ran no `git gc` / `git prune`, so Git has not yet reclaimed the
  now-unreachable object. The assertion is only about immediate
  post-reconciliation resolvability, before any explicit GC; the plan does not
  rely on the object surviving indefinitely.
  - Validate: `services::mutation_trace::runtime::ref_reconciliation::tests::reconciliation_deletes_refs_without_reclaiming_objects`

### Full validation

Repository-wide checks `/validate` runs after the last task.

- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::`
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`
- `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`
- `nix flake check` (runs `checks.cli-tests`, `checks.cli-clippy`,
  `checks.cli-fmt`, `checks.mutation-trace-quint-connect`)
- `nix run .#pkl-check-generated`
- Confirm the existing Quint / Quint Connect model-based-testing harness
  (`checks.cli-tests` / `checks.mutation-trace-quint-connect`) stays green;
  **no `spec/mutation_cursor.qnt` behavior change is expected or made.**

### Context sync

- `context/cli/mutation-trace-store.md` — document the two new bounded
  read-only queries: `load_tree_roots` (cursor + historical event trees for
  one worktree, deduplicated) and `load_all_tree_roots` (the same three
  `TreeId` columns unioned across every worktree, deduplicated, for the
  reconciler's repository-wide retention set); state that **each query is
  backed by a single SQL statement (`UNION` of `cursor_tree` / `before_tree` /
  `after_tree`), so its full root set is read from one coherent database
  snapshot** — not multiple independent `SELECT`s unioned in Rust — which is
  what keeps a concurrent atomic `cursor T → X` + `event T → X` commit from
  exposing a torn root set; update "Non-goals" so it no
  longer implies the store exposes no durable-tree read for reconciliation
  while keeping "no row deletion".
- `context/cli/mutation-trace-runtime-coordinator.md` — document
  `GitSnapshotService::list_pins` (returns `Result<Vec<PinnedRef>,
  PinInventoryError>`) / `delete_pins`; document the new
  `runtime::ref_reconciliation` module (`reconcile_worktree`, the `pub(super)`
  `reconcile_worktree_inner` test seam, `ReconciliationReport`,
  `ReconcileError`, `RECONCILIATION_LOCK_TIMEOUT`, the fail-closed rules, and
  the two-invariant model — per-worktree local consistency check
  (`load_tree_roots`) vs. repository-wide deletion retention set
  (`load_all_tree_roots`): an A-owned ref is retained whenever any worktree
  still durably needs its tree; note that each of these reads is one SQL
  statement / one DB snapshot, which is what makes the repository-wide read
  safe against a concurrent atomic `cursor T → X` + `event T → X` commit
  without a repository-global lock);
  correct
  the "one ref per pinned tree, **create-only**" on-disk-layout note to
  "create-only per invocation, reclaimed by the per-worktree reconciliation
  pass"; record that the `WorktreeLock` now also guards reconciliation; extend
  the testing boundary.
- `context/cli/mutation-trace-protocol.md` — in "Target end-state
  architecture", note that ref reconciliation is imperative durability
  maintenance outside the verified protocol: it never advances the cursor,
  chooses attribution, changes scope state, or creates a `MutationEvent`.
- `context/context-map.md` — refresh the `mutation-trace-runtime-coordinator.md`
  and `mutation-trace-store.md` line annotations.
- `context/overview.md` — extend the `mutation_trace/runtime/` sentence to
  mention the per-worktree ref-reconciliation maintenance pass.
- `spec/mutation_cursor.md` — under "Failure and durability boundary" /
  "Implementation refinement", record that SCE-owned snapshot refs are
  reclaimed by an imperative per-worktree maintenance pass (never modeled),
  that Git performs object GC itself on its normal schedule, and that **SCE
  deletes only its own refs, never Git objects directly**.
- Verify-only pass (expected: no edit): `context/architecture.md`,
  `context/glossary.md`, `context/patterns.md`,
  `context/cli/mutation-trace-external-taint.md` (reconciliation must not
  touch the marker).

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:**
  - `cli/src/services/mutation_trace/store.rs` — two new bounded, read-only
    queries (`load_tree_roots` for one worktree, `load_all_tree_roots` for the
    whole repository), each backed by **one SQL statement** (a single
    `SELECT`/`UNION` constant covering `cursor_tree`, `before_tree`, and
    `after_tree`) and **one** `query_map` call, plus its `TreeId` row mapper,
    and inline tests. No write path, no schema, no migration change.
  - `cli/src/services/mutation_trace/runtime/git_snapshot.rs` — new
    worktree-scoped pin inventory (`list_pins`, returning `Result<Vec<PinnedRef>,
    PinInventoryError>`) and conditional atomic deletion (`delete_pins`)
    primitives on `GitSnapshotService`, plus a small `PinnedRef` value type
    and the `PinInventoryError` enum, and inline tests. `capture_tree` /
    `pin_tree` / `diff_trees` are unchanged.
  - `cli/src/services/mutation_trace/runtime/ref_reconciliation.rs` (new) —
    the per-worktree reconciliation algorithm, `ReconciliationReport`,
    `ReconcileError`, the module-owned `RECONCILIATION_LOCK_TIMEOUT` constant,
    the `pub fn reconcile_worktree` entrypoint (module-private to `runtime`,
    like `coordinate`) and the `pub(super) fn reconcile_worktree_inner(..,
    on_lock_contention)` test seam (visible only within `runtime`), and
    inline tests.
  - `cli/src/services/mutation_trace/runtime/mod.rs` — `mod ref_reconciliation;`
    (private, matching `mod coordinator;`).
  - `cli/src/services/mutation_trace/runtime/tests.rs` — cross-module
    integration tests through `reconcile_worktree` against real Git
    repositories and a real repository-scoped Agent Trace DB.
  - The durable context/spec docs named in "Context sync".
- **Out of scope:** harness adapters and any hook/command/`diff_traces`
  wiring; a `pub(crate)` re-export of `reconcile_worktree`; deciding *when*
  reconciliation runs (harness-wiring PR — see Design decisions Q15);
  auto-sync; control-plane changes; a daemon or background process; a new
  persistent reconciliation bookkeeping table or cursor; repairing / recreating
  a missing required ref (detect-and-fail-closed only, see Q7); a
  repository-wide `refs/sce/**` scan or general-purpose ref cleanup;
  cross-machine locking; a repository-global lock; `git gc` / `git prune` /
  `git reflog expire` invocation; a generic Git abstraction redesign
  (`capabilities::GitOps`); changing `pin_tree`'s create-only-per-invocation
  behavior; changes to `protocol.rs`, `types.rs`, `store.rs`'s write path,
  `spec/mutation_cursor.qnt`, or the Quint refinement matrix; new mutation
  attribution rules; new `FailureKind`; new DB migration; the still-separate
  `mutation-cursor-external-taint` concerns (that plan has landed).
- **Constraints:**
  - No new Cargo dependencies. Reconciliation reuses `WorktreeLock`
    (`std::fs::File` advisory locks) via `worktree_lock::acquire_inner`
    (already `pub(super)`), `checkout::{resolve_git_dir, read_checkout_id}`,
    `GitSnapshotService`, `MutationTraceStore`, and the `git` plumbing
    subprocess pattern already in `git_snapshot.rs`.
  - Reconciliation owns its own bounded lock timeout,
    `const RECONCILIATION_LOCK_TIMEOUT: Duration = Duration::from_secs(10)`
    in `ref_reconciliation.rs`. Its value intentionally matches the
    coordinator's private `WORKTREE_LOCK_TIMEOUT` but is **not** a shared
    abstraction — the coordinator constant stays private to `coordinator.rs`,
    and neither is moved into `worktree_lock.rs` (there is no semantic reason
    the two timeouts must always stay identical).
  - New Git plumbing is limited to `git for-each-ref` (already in the
    coordinator plan's validated command set) and `git update-ref --stdin`
    (T02 validates its transaction semantics experimentally against this
    repository's Git). No `git gc` / `git prune` / `git reflog`.
  - `git_snapshot.rs`'s ref namespace constant `REF_NAMESPACE`
    (`refs/sce/mutation-cursor`) and `pin_ref_name` layout are the single
    source of truth for the pin path; `list_pins` / `delete_pins` derive their
    prefix from the same constant.
  - The DB is supplied to `reconcile_worktree` by a caller-provided
    `open_db: impl FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>`
    provider, exactly like `coordinate()` — `reconcile_worktree` never
    resolves repository identity or opens the DB itself.
  - `reconcile_worktree` never accepts a caller-supplied `WorktreeId`,
    `TreeId`, or ref name: worktree identity is derived from `repository_root`
    → `resolve_git_dir` → `read_checkout_id`, exactly as `coordinate()`
    derives it.
  - Same-worktree pin inventory, root read, and deletion happen only while
    that worktree's `WorktreeLock` is held.
  - `cargo clippy` runs with `pedantic`/`warnings` denied workspace-wide.
- **Non-goal:** treating a reconciliation failure as mutation-cursor external
  taint or any protocol failure. Reconciliation is maintenance: a failure to
  delete an obsolete ref means storage cleanup did not complete, **not** that
  mutation evidence is untrustworthy. It never arms `ExternalTaintMarker`,
  calls `protocol::database_failure` / `protocol::taint`, mutates
  mutation-cursor protocol state, produces a new `FailureKind`, changes
  attribution, or triggers recovery.

## Assumptions

The user's change request states the example names/signatures are not
mandatory ("Do not prescribe this exact name or signature if the current
repository conventions suggest something better"). The following are recorded
local choices consistent with the existing `runtime/` conventions, not new
requirements.

- The store queries are
  `MutationTraceStore::load_tree_roots(&self, worktree: &WorktreeId) ->
  Result<std::collections::BTreeSet<TreeId>>` and
  `MutationTraceStore::load_all_tree_roots(&self) ->
  Result<std::collections::BTreeSet<TreeId>>`, cold-path reads siblings to
  `load_mutation_event` (which is likewise never called from `load_worktree`
  or any hook-boundary path). Each method issues **exactly one SQL statement**
  and **one** `query_map` call: a `UNION` of the three `TreeId` columns
  (`mutation_trace_worktrees.cursor_tree`, `mutation_trace_events.before_tree`,
  `mutation_trace_events.after_tree`) — `load_tree_roots` with each arm scoped
  `WHERE worktree_id = ?1`, `load_all_tree_roots` with no `WHERE worktree_id`
  clause — mapping the single `tree` column into `TreeId` and collecting
  directly into `BTreeSet<TreeId>`. No other table contributes. The
  constituent tables are **never** read with separate `query_map` calls and
  unioned in Rust: the whole root set must come from one statement / one
  database snapshot (see AC1 and "The challenge interleavings"). `UNION` is
  chosen over `UNION ALL` because the API returns a set and duplicate
  `TreeId`s are irrelevant; `UNION ALL` plus Rust `BTreeSet` dedup would be
  acceptable only if the whole operation stays one SQL statement. A single
  scope-parameterized query in place of the two methods would also be
  acceptable if it fits repository conventions better — the two-method,
  one-statement-each shape is the recorded local choice. The recorded SQL
  constants are `SELECT_TREE_ROOTS_BY_WORKTREE_SQL` and
  `SELECT_ALL_TREE_ROOTS_SQL` (not per-table constants such as
  `SELECT_WORKTREE_CURSOR_ROOTS` / `SELECT_EVENT_ROOTS`).
- The Git pin-inventory primitive is
  `GitSnapshotService::list_pins(&self, worktree_id: &WorktreeId) ->
  Result<Vec<PinnedRef>, PinInventoryError>`, where
  `PinnedRef { ref_name: String, tree: TreeId }` and

  ```rust
  pub enum PinInventoryError {
      /// `git for-each-ref` itself failed to execute or exited non-zero.
      Git(anyhow::Error),
      /// A ref under the SCE namespace is not shaped like a `pin_tree` output:
      /// a non-tree target, a name/target SHA mismatch, an unparseable
      /// `for-each-ref` line, or an unexpected extra path segment. `reason`
      /// carries the specific discriminant for tests and `Display`.
      MalformedRef { ref_name: String, reason: String },
  }
  ```

  This makes malformed SCE-namespace state separately matchable from a generic
  `git for-each-ref` execution failure. The conditional-deletion primitive is
  `GitSnapshotService::delete_pins(&self, pins: &[PinnedRef]) ->
  anyhow::Result<()>` — a transaction failure (including a failed old-value
  check) is a plain `Err` the reconciler maps to
  `ReconcileError::DeleteTransaction`.
- The entrypoint is
  `reconcile_worktree(repository_root: &Path, open_db: impl FnOnce() ->
  anyhow::Result<RepositoryAgentTraceDb>) -> Result<ReconciliationReport,
  ReconcileError>`, a `pub fn` in the private `mod ref_reconciliation`
  (module-private to `runtime`, exactly like `coordinate` in the private
  `mod coordinator` — never re-exported outside mutation-trace `runtime`). It
  is a one-line delegation to
  `pub(super) fn reconcile_worktree_inner(.., on_lock_contention: impl
  FnOnce())` (mirroring `coordinate` / `coordinate_inner`, and matching
  `worktree_lock::acquire_inner`'s existing `pub(super)`), with production
  passing a no-op contention closure. `pub(super)` keeps the seam visible to
  `runtime` and `runtime::tests` (where T04 lives) but invisible outside
  `runtime` even if `ref_reconciliation` later becomes `pub`.
- `ReconciliationReport { local_required: usize, retained: usize, deleted:
  usize }` (see Design decisions — Report shape). `local_required` is
  `load_tree_roots(W).len()`; `retained` is `actual_W.len() − deleted`.
  `retained == local_required` is **not** an invariant and the plan does not
  claim it — a pin retained only because another worktree durably needs its
  tree counts toward `retained` but not `local_required`. The only relation
  that holds on the `Ok` path is `local_required ≤ retained`.
- `ReconcileError` is a matchable enum with `Display` + `std::error::Error`
  (mirroring `CoordinateError`), with a distinct variant for **every** fallible
  step and no `Other` catch-all:

  ```rust
  pub enum ReconcileError {
      GitDir(anyhow::Error),               // resolve_git_dir
      Lock(WorktreeLockError),             // WorktreeLock acquisition
      CheckoutIdentity(anyhow::Error),     // read_checkout_id returned Err (corrupt/unreadable)
      AgentTraceDbUnavailable(anyhow::Error), // open_db() provider failed
      SnapshotService(anyhow::Error),      // GitSnapshotService::new
      PinInventory(anyhow::Error),         // PinInventoryError::Git
      MalformedPin { ref_name: String, reason: String }, // PinInventoryError::MalformedRef
      DurableRoots(anyhow::Error),         // load_tree_roots / load_all_tree_roots
      MissingRequiredPins { missing: Vec<TreeId> }, // fail-closed (local consistency), nothing deleted
      DeleteTransaction(anyhow::Error),    // delete_pins
  }
  ```

  `read_checkout_id() == Ok(None)` is **not** an error — it means
  reconciliation has no current checkout identity from which to derive a
  `WorktreeId` and its owned ref prefix (not a claim that none ever existed).
  The reconciler has
  already acquired `WorktreeLock(W)` by that point, and it releases the lock
  and returns a clean no-op `Ok(ReconciliationReport { local_required: 0,
  retained: 0, deleted: 0 })` without creating an identity (no DB or Git-ref
  work); only `Err` maps to `CheckoutIdentity`. `AgentTraceDbUnavailable` here is a reconciliation
  maintenance error only: it never arms `ExternalTaintMarker` and never
  becomes `CoordinateError::AgentTraceDbUnavailable`, because no mutation
  boundary is being coordinated.
- Test module/function names follow the crate module paths named in the
  acceptance criteria.
- Reconciliation uses its own module-owned `RECONCILIATION_LOCK_TIMEOUT`
  (`Duration::from_secs(10)` in `ref_reconciliation.rs`); it does not
  reference the coordinator's private `WORKTREE_LOCK_TIMEOUT`. The values match
  by intent, not by a shared constant.

## Task stack

- [x] T01: `Add worktree-scoped and repository-wide durable TreeId root queries to MutationTraceStore` (status:done)
  - Task ID: T01
  - Scope: In — `cli/src/services/mutation_trace/store.rs`:
    `pub fn load_tree_roots(&self, worktree: &WorktreeId) ->
    Result<BTreeSet<TreeId>>` and
    `pub fn load_all_tree_roots(&self) -> Result<BTreeSet<TreeId>>`, both
    cold-path reads. Each is backed by **exactly one SQL statement** and
    driven through **one** `query_map` call — not one `query_map` per backing
    table with the results unioned in Rust. Also in —
    `cli/src/services/db/mod.rs`: a `#[cfg(test)]` `pub(crate)`
    `count_read_statements` seam plus a per-thread read-statement counter the
    `TursoDb` read methods bump (test-only; no production behavior change). The
    two recorded constants:

    - `SELECT_TREE_ROOTS_BY_WORKTREE_SQL` —
      ```sql
      SELECT cursor_tree AS tree FROM mutation_trace_worktrees WHERE worktree_id = ?1
      UNION
      SELECT before_tree AS tree FROM mutation_trace_events    WHERE worktree_id = ?1
      UNION
      SELECT after_tree  AS tree FROM mutation_trace_events    WHERE worktree_id = ?1
      ```
    - `SELECT_ALL_TREE_ROOTS_SQL` — the same statement with **no**
      `WHERE worktree_id` clause on any arm:
      ```sql
      SELECT cursor_tree AS tree FROM mutation_trace_worktrees
      UNION
      SELECT before_tree AS tree FROM mutation_trace_events
      UNION
      SELECT after_tree  AS tree FROM mutation_trace_events
      ```

    Each statement maps its single `tree` column into `TreeId` and collects
    directly into `BTreeSet<TreeId>`. `UNION` (set semantics) is intentional
    because duplicate `TreeId`s are irrelevant to a set-typed API;
    `UNION ALL` + Rust `BTreeSet` dedup is acceptable **only** if the whole
    operation stays one SQL statement. An empty set (not `Err`) when the
    scoped worktree has no `mutation_trace_worktrees` row and when the
    repository has no rows at all. Read-only, cold path, no write changes, no
    schema changes, no migration. Inline `#[cfg(test)] mod tests` cases
    seeded via raw SQL (matching the existing `insert_worktree` /
    `insert_mutation_event` test helpers in this file), covering
    worktree-scoped exactness, repository-wide exactness, deduplication of a
    tree shared by multiple worktrees, an empty repository, multiple
    worktrees, **and the single-statement snapshot regression** described
    below. A small test-only seam is added to
    `cli/src/services/db/mod.rs`: `count_read_statements(body) -> (T, usize)`
    (`#[cfg(test)]`, `pub(crate)`), backed by a per-thread counter each
    `TursoDb` read method (`query` / `query_values` / `query_map`) bumps once
    in its synchronous prelude, before the retry wrapper. No production
    behavior change — the increments are `#[cfg(test)]`-only.
    Out — any write path, `commit` change, schema/migration change, the Git
    primitives (T02), the reconciliation algorithm (T03), calling either
    query from anywhere (T03).
  - Concurrency regression (in this task): two deterministic tests, no sleeps,
    no probabilistic race, no stress loop. (a)
    `load_all_tree_roots_retains_previous_cursor_after_atomic_cursor_advance` —
    a **state-transition** test only: pre-advance `T` is a root via
    `cursor_tree`, post-advance `T` is a root via `before_tree`. It is
    explicitly *not* proof of snapshot isolation (a torn multi-read
    implementation would also pass it). (b)
    `load_all_tree_roots_reads_every_durable_root_in_one_sql_statement` and
    `load_tree_roots_reads_every_durable_root_in_one_sql_statement` — the
    **enforcement** regression. Each models the transition
    `B.cursor_tree = T` → (atomically, one DB transaction) `cursor_tree := X`
    **and** `INSERT mutation_trace_events { before_tree = T, after_tree = X }`,
    then: (1) constructs the torn set that a multi-read implementation would
    produce — read the event trees (empty), commit the atomic advance, read
    the cursor trees (`{X}`), union in Rust → `{X}`, missing `T`; (2) asserts
    the production `load_*_tree_roots` call, wrapped in
    `count_read_statements`, issues **exactly one** read statement and returns
    a set containing `T`. Reimplementing either query as two independent
    `SELECT`s makes `count_read_statements` observe `2` and fails the test —
    verified in-session by temporarily splitting `load_all_tree_roots` into
    two `query_map` calls (`left: 2, right: 1`). The property this locks in:
    one `load_*_tree_roots` invocation = one SQL statement = one DB snapshot,
    which is the concurrency boundary because a single statement reads from one
    coherent MVCC snapshot while two statements are two snapshots a concurrent
    commit can fall between.
  - Dependencies: none
  - Done when: `load_tree_roots(W)` returns exactly `{cursor_tree(W)} ∪
    {before_tree, after_tree : mutation_trace_events row for W}`, deduplicated,
    and `Ok(empty set)` for a worktree with no durable row;
    `load_all_tree_roots()` returns the union of those same three columns
    across **every** worktree, deduplicated, and `Ok(empty set)` for an empty
    repository; **each method executes exactly one SQL statement through one
    `query_map` call** — one `UNION` constant, no Rust-side union of per-table
    result vectors — enforced at runtime by the
    `…reads_every_durable_root_in_one_sql_statement` regression via
    `count_read_statements`, not merely by inspection; neither query ever
    returns a tree sourced from
    `mutation_trace_scopes` / `mutation_trace_processed_events` /
    `mutation_trace_event_active_scopes`, and `load_tree_roots` never returns
    a tree belonging to another worktree; neither method is reachable from
    `load_worktree` or a hook-boundary path (both are sibling cold-path reads
    like `load_mutation_event`); the single-statement snapshot regression
    passes.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::store::`;
    `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`;
    `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`.
  - Completed: 2026-08-31
  - Files changed: `cli/src/services/mutation_trace/store.rs`,
    `cli/src/services/db/mod.rs`
  - Result: Added two SQL constants — `SELECT_TREE_ROOTS_BY_WORKTREE_SQL`
    (`?1`-scoped `UNION` of `cursor_tree` / `before_tree` / `after_tree`) and
    `SELECT_ALL_TREE_ROOTS_SQL` (the same `UNION` with no `WHERE` clause) —
    beside the existing `SELECT_*` constants, plus a free
    `tree_root_row_from_turso(&turso::Row) -> Result<TreeId>` row mapper next
    to the other `*_row_from_turso` functions. Added
    `pub fn load_tree_roots(&self, worktree: &WorktreeId) -> Result<BTreeSet<TreeId>>`
    and `pub fn load_all_tree_roots(&self) -> Result<BTreeSet<TreeId>>` on
    `MutationTraceStore`, immediately after `load_mutation_event` (both
    cold-path siblings, never reached from `load_worktree` or a hook-boundary
    path). Each method is one `self.db.query_map(<constant>, .., tree_root_row_from_turso)`
    call collecting `.into_iter().collect()` into `BTreeSet<TreeId>` — no
    Rust-side union of per-table result vectors. turso accepts the reused `?1`
    placeholder across the three `UNION` arms (verified by the passing tests).
    In `db/mod.rs`, added a `#[cfg(test)]` statement-count seam:
    `pub(crate) fn count_read_statements(body) -> (T, usize)` backed by a
    per-thread `READ_STATEMENTS_ISSUED` cell that `TursoDb::{query,
    query_values, query_map}` each bump once in their synchronous prelude
    (before `run_with_retry_sync`, so retries never inflate the count).
    Production builds are unaffected (`#[cfg(test)]`).
    Added inline `#[cfg(test)] mod tests` cases plus local helpers
    (`insert_worktree_with_cursor`, `insert_event_trees`, `tree_set`,
    `apply_atomic_cursor_advance`, `select_trees`): the 8 exactness/dedup/empty
    cases, the renamed state-transition test
    `load_all_tree_roots_retains_previous_cursor_after_atomic_cursor_advance`
    (pre `T` via `cursor_tree`, post `T` via `before_tree` across an atomic
    `execute_transactional_cas_batch` advance; docstring states it is *not*
    snapshot-isolation proof), and the two deterministic enforcement
    regressions
    `load_all_tree_roots_reads_every_durable_root_in_one_sql_statement` /
    `load_tree_roots_reads_every_durable_root_in_one_sql_statement`, which
    build the torn two-read set explicitly and assert the production path
    issues exactly one read statement via `count_read_statements`.
  - Verify (actual): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::store::`
    — 83 passed, 0 failed (10 new `load_*_tree_roots*` tests among them).
    `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::db`
    — 22 passed (statement-count seam adds no regression).
    `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` — full
    suite 855 passed, 0 failed.
    `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`
    — no warnings.
    `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`
    — clean.
    Regression proven: temporarily reimplementing `load_all_tree_roots` as two
    `query_map` calls made
    `load_all_tree_roots_reads_every_durable_root_in_one_sql_statement` fail
    with `left: 2, right: 1`, while the state-transition test still passed —
    confirming the counter test is the one that catches the torn-read
    regression. Reverted.
  - Deviations: The single-statement snapshot regression was strengthened from
    the plan's original pre/post-only design (which the plan text now records
    as insufficient) to the `count_read_statements` enforcement test, and the
    weak test was renamed
    `load_all_tree_roots_retains_previous_cursor_after_atomic_cursor_advance`
    with an honest docstring. This required a `#[cfg(test)]`-only
    read-statement counter in `cli/src/services/db/mod.rs` (added to scope
    above) — the smallest deterministic seam; no production behavior change,
    no new API on `load_tree_roots` / `load_all_tree_roots`. Recorded
    assumption names and the two-method / one-statement-each shape were used
    verbatim. Test fixtures seed events with
    `attribution_kind = 'ineligible_unscoped'` / `boundary_kind = 'flush'` (the
    minimal CHECK-satisfying shape) since only the tree columns matter here.
  - Context impact: Domain. Adds two new public read-only methods to
    `MutationTraceStore`'s contract (`load_tree_roots`, `load_all_tree_roots`);
    no schema, migration, write-path, architectural, or cross-domain change.
    Durable context to refresh per the plan's Context sync section:
    `context/cli/mutation-trace-store.md` (document the two bounded queries and
    the single-statement snapshot property; update "Non-goals"), and the
    line annotations in `context/context-map.md`. No call site exists yet
    (T03 wires the reconciler).
  - Context synchronization: synced

- [x] T02: `Add worktree-scoped pin inventory and conditional atomic deletion to GitSnapshotService` (status:done)
  - Task ID: T02
  - Scope: In — `cli/src/services/mutation_trace/runtime/git_snapshot.rs`: a
    `PinnedRef { ref_name: String, tree: TreeId }` value type; the
    `PinInventoryError` enum (`Git(anyhow::Error)` for a `git for-each-ref`
    execution/exit failure; `MalformedRef { ref_name: String, reason: String }`
    for a non-tree target, a name/target SHA mismatch, an unparseable line, or
    an unexpected extra path segment) with `Display` + `std::error::Error`;
    `list_pins(&self, worktree_id: &WorktreeId) -> Result<Vec<PinnedRef>,
    PinInventoryError>` running `git for-each-ref
    --format=<refname/objectname/objecttype>` constrained to the single path
    prefix `refs/sce/mutation-cursor/<worktree_id>/` (derived from
    `REF_NAMESPACE` plus a trailing `/`), parsing each line, and returning
    `Err(PinInventoryError::MalformedRef { .. })` for any ref whose
    `objecttype` is not `tree` or whose final path component does not equal its
    `objectname`; `delete_pins(&self, pins: &[PinnedRef]) -> anyhow::Result<()>`
    feeding one `git update-ref --stdin` transaction of `delete SP <ref_name>
    SP <expected_tree_sha> LF` lines (a no-op returning `Ok(())` for an empty
    slice), so the whole batch aborts and deletes nothing if any ref no longer
    matches its expected value; inline `#[cfg(test)] mod tests` extending the
    existing real-`git init` test pattern in this file, including one test that
    experimentally pins several trees, mutates one ref's value between
    inventory and delete, and asserts `delete_pins` fails with every ref intact
    (the canonical AC10 proof), plus tests matching
    `PinInventoryError::MalformedRef` separately from `PinInventoryError::Git`.
    Out — the reconciliation algorithm and `WorktreeLock` acquisition (T03);
    the store query (T01); `capture_tree` / `pin_tree` / `diff_trees` changes;
    any repository-wide ref enumeration.
  - Dependencies: none
  - Done when: `list_pins(W)` returns one `PinnedRef` per ref under exactly
    `refs/sce/mutation-cursor/<W>/` and never a ref under another worktree's
    prefix or any unrelated namespace; a ref in that namespace whose target is
    not a tree, or whose name suffix disagrees with its target SHA, makes
    `list_pins` return `Err(PinInventoryError::MalformedRef { .. })`, matchable
    separately from `PinInventoryError::Git(..)` (a `git for-each-ref`
    execution failure); `delete_pins` removes exactly the supplied refs when
    every expected value still matches, removes nothing and returns `Err` when
    any expected value has changed, and is a successful no-op for an empty
    slice; the `git update-ref --stdin` transaction semantics relied on are
    demonstrated by a test against this repository's Git.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::git_snapshot::`;
    `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`;
    `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`.
  - Completed: 2026-08-31
  - Files changed: `cli/src/services/mutation_trace/runtime/git_snapshot.rs`
  - Result: Added `PinnedRef { ref_name: String, tree: TreeId }`
    (`#[derive(Clone, Debug, Eq, PartialEq)]`) and the `PinInventoryError` enum
    (`Git(anyhow::Error)`, `MalformedRef { ref_name: String, reason: String }`)
    with a manual `Display` + `std::error::Error` impl, matching the
    `CoordinateError` convention in `coordinator.rs`. Added a private
    `pin_ref_prefix(worktree_id) -> String` helper
    (`refs/sce/mutation-cursor/<worktree-id>/`) and routed the existing
    `pin_ref_name` through it so the prefix has one source of truth. Added a
    free `parse_pin_line(line, prefix) -> Result<PinnedRef, PinInventoryError>`
    that enforces exactly-three space-separated `for-each-ref` fields, an
    `objecttype` of `tree`, no extra path segment after the worktree prefix,
    and refname-suffix equal to the target SHA — every failure is
    `PinInventoryError::MalformedRef` with a discriminating `reason`. Added
    `GitSnapshotService::list_pins(&self, worktree_id: &WorktreeId) ->
    std::result::Result<Vec<PinnedRef>, PinInventoryError>` running
    `git for-each-ref --format=%(refname) %(objectname) %(objecttype)`
    constrained to the single prefix (a `git for-each-ref` execution/exit
    failure maps to `PinInventoryError::Git`), and
    `GitSnapshotService::delete_pins(&self, pins: &[PinnedRef]) ->
    anyhow::Result<()>` feeding one `git update-ref --stdin` transaction of
    `delete SP <ref> SP <expected_tree_sha> LF` lines (empty slice is an
    `Ok(())` no-op), spawned with the same `current_dir` + `GIT_DIR` env as
    `run_git`. `capture_tree` / `pin_tree` / `diff_trees` are unchanged. Added
    8 inline tests plus helpers (`other_worktree_id`, `capture_with_file`,
    `ref_target`, `ref_exists`): prefix-scoped isolation, empty inventory,
    `list_pins_rejects_a_ref_whose_target_is_not_a_tree`,
    `list_pins_rejects_a_ref_whose_name_disagrees_with_its_target`,
    extra-path-segment rejection, the `Git` variant on a removed git-dir,
    `delete_pins` exact removal, empty-slice no-op, and
    `delete_pins_aborts_the_whole_transaction_when_one_ref_no_longer_matches_its_expected_value`
    (inventory `R → X` for two refs, move one to `Y`, call `delete_pins` with
    the inventoried pair, assert the whole transaction fails and both refs
    survive — the canonical AC10 proof against this repository's Git).
  - Verify (actual):
    `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::git_snapshot::`
    — 22 passed, 0 failed (8 new). Broader
    `services::mutation_trace::runtime::` — 65 passed, 0 failed (the
    `pin_ref_name` refactor regressed nothing).
    `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`
    — no warnings.
    `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`
    — clean.
  - Deviations: None. Recorded assumption names and signatures (`PinnedRef`,
    `PinInventoryError` with its two variants, `list_pins` / `delete_pins`
    signatures) were used verbatim. `list_pins` also rejects a ref with an
    extra path segment after the worktree prefix (the plan's "unexpected extra
    path segment" case), tested. The `Git`-variant test forces a
    `git for-each-ref` execution failure by removing the resolved git-dir.
  - Context impact: Domain. Adds new public items to `GitSnapshotService`'s
    runtime-internal surface (`PinnedRef`, `PinInventoryError`, `list_pins`,
    `delete_pins`); no schema, migration, protocol, marker, or cross-domain
    change, and `git_snapshot` stays private to `mutation_trace::runtime`. No
    call site exists yet (T03 wires the reconciler). Durable context to refresh
    per the plan's Context sync section:
    `context/cli/mutation-trace-runtime-coordinator.md` (document `list_pins`
    returning `Result<Vec<PinnedRef>, PinInventoryError>` and `delete_pins`,
    and extend the testing boundary).
  - Context synchronization: synced

- [ ] T03: `Implement per-worktree reconciliation under WorktreeLock` (status:todo)
  - Task ID: T03
  - Scope: In — new
    `cli/src/services/mutation_trace/runtime/ref_reconciliation.rs` and
    `mod ref_reconciliation;` in `runtime/mod.rs`. `ReconciliationReport {
    local_required, retained, deleted }`; the module-owned
    `const RECONCILIATION_LOCK_TIMEOUT: Duration = Duration::from_secs(10)`;
    `ReconcileError` with `Display` + `Error` and one variant per fallible
    step, no `Other` catch-all: `GitDir(anyhow::Error)`,
    `Lock(WorktreeLockError)`, `CheckoutIdentity(anyhow::Error)`,
    `AgentTraceDbUnavailable(anyhow::Error)`, `SnapshotService(anyhow::Error)`,
    `PinInventory(anyhow::Error)`, `MalformedPin { ref_name: String, reason:
    String }`, `DurableRoots(anyhow::Error)`, `MissingRequiredPins { missing:
    Vec<TreeId> }`, `DeleteTransaction(anyhow::Error)`. `pub fn
    reconcile_worktree(repository_root, open_db)` (module-private to `runtime`)
    delegating to `pub(super) fn reconcile_worktree_inner(repository_root,
    open_db, on_lock_contention)`. Algorithm, with each step's error mapping,
    entirely under the lock:
    `resolve_git_dir(repository_root)` (`Err ⇒ GitDir`) →
    `worktree_lock::acquire_inner(&git_dir, RECONCILIATION_LOCK_TIMEOUT,
    on_lock_contention)` (`Err ⇒ Lock`) →
    `checkout::read_checkout_id(&git_dir)`: `Ok(None) ⇒ return
    ReconciliationReport { local_required: 0, retained: 0, deleted: 0 }` (the
    lock is already held; reconciliation has no current checkout identity from
    which to derive a `WorktreeId` and its owned
    `refs/sce/mutation-cursor/<worktree-id>/` prefix, so it performs a clean
    no-op without creating an identity and runs no DB/Git-ref work — see
    Q3/Q5); `Err ⇒ CheckoutIdentity` (a corrupt/unreadable id is **not** an
    absent id); `Ok(Some(id)) ⇒ WorktreeId(id)` →
    `open_db()` (`Err ⇒ AgentTraceDbUnavailable`; maintenance error only —
    never arms the taint marker, never `CoordinateError`) →
    `GitSnapshotService::new(repository_root)` (`Err ⇒ SnapshotService`) →
    `actual = list_pins(&W)` (inventory **first**;
    `Err(PinInventoryError::Git) ⇒ PinInventory`,
    `Err(PinInventoryError::MalformedRef { ref_name, reason }) ⇒ MalformedPin {
    ref_name, reason }`) →
    `store = MutationTraceStore::new(&db)` →
    `required_local = store.load_tree_roots(&W)` (`Err ⇒ DurableRoots`) →
    `missing_local = required_local − {p.tree for p in actual}`; if non-empty ⇒
    `Err(MissingRequiredPins { missing: missing_local })` deleting nothing
    (the **local consistency invariant** — a strictly per-worktree check, not
    repository-wide) →
    `required_repository = store.load_all_tree_roots()` (`Err ⇒ DurableRoots`)
    →
    `stale = [p in actual : p.tree ∉ required_repository]` (the **deletion
    safety invariant** — an A-owned ref is removed only when no worktree in
    the repository durably needs its tree); if empty ⇒ report with
    `deleted: 0`; else `delete_pins(&stale)` (`Err ⇒ DeleteTransaction`) then
    report `{ local_required: required_local.len(), retained: actual.len() −
    stale.len(), deleted: stale.len() }`. Inline `#[cfg(test)] mod tests`
    against a real temp-file `RepositoryAgentTraceDb` and a real
    `GitSnapshotService` over a temp `git init` repo: orphan pin (with and
    without a worktree row) deleted; current-cursor pin retained with no
    referencing event; historical event before/after pins retained after the
    cursor advances; a pin whose tree is absent from the target worktree's
    roots but present in another worktree's durable rows (seeded via raw SQL)
    is **retained** — repository-wide retention; `MissingRequiredPins`
    fail-closed deleting nothing even when another worktree's row would cover
    the missing tree; malformed namespace ref fail-closed deleting nothing;
    idempotence; refs-deleted-without-object-reclamation; report counts
    including the `retained > local_required` case. Out — the deterministic
    lock-race regression (T04); cross-module
    integration and linked-worktree tests (T05); any harness/command wiring; a
    `pub(crate)` re-export; deciding invocation timing.
  - Dependencies: T01, T02
  - Done when: `reconcile_worktree` acquires the worktree's `WorktreeLock`
    (via `worktree_lock::acquire_inner` with `RECONCILIATION_LOCK_TIMEOUT`)
    before any pin or DB read and holds it until return; derives `WorktreeId`
    only from `repository_root` (never a caller argument); distinguishes
    `read_checkout_id` → `Ok(None)` (clean `{ local_required: 0, retained: 0,
    deleted: 0 }` no-op, lock already held, no identity created) from `Err`
    (`ReconcileError::CheckoutIdentity`); inventories pins before reading
    durable roots; reads the target worktree's own roots (`load_tree_roots`)
    for the fail-closed check and the repository-wide roots
    (`load_all_tree_roots`) for the deletion decision; every fallible step
    maps to its dedicated `ReconcileError` variant with no `Other` fallback;
    fails closed with a distinct error and deletes nothing when any of the
    **target worktree's** durable roots lacks a pin or the namespace contains
    a malformed ref; otherwise deletes exactly the pins whose tree is outside
    the **repository-wide** root set via one atomic `delete_pins` call and
    returns a `ReconciliationReport` whose counts match; a checkout id present
    but no durable row for W yields `{ local_required: 0, retained: R,
    deleted: N }` where the retained `R` pins are exactly those W-owned pins
    another worktree still durably needs and the deleted `N` are the rest,
    with no error, and `{ 0, 0, 0 }` when there are also no pins; `open_db()`
    failure is `AgentTraceDbUnavailable` and
    provably never touches `ExternalTaintMarker`;
    `reconcile_worktree_inner` is `pub(super)`; every listed inline test passes.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::ref_reconciliation::`;
    `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`;
    `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`.
  - Context synchronization: pending

- [ ] T04: `Add the deterministic pin-to-CAS synchronization regression` (status:todo)
  - Task ID: T04
  - Scope: In — `cli/src/services/mutation_trace/runtime/tests.rs` (a child of
    `runtime`, so it can reach the `pub(super)` seam): one deterministic
    concurrency test that (a) acquires a real `WorktreeLock` for worktree W on
    the main thread via `worktree_lock::acquire_inner`; (b) spawns a worker
    calling `ref_reconciliation::reconcile_worktree_inner` for W with an
    `on_lock_contention` closure that signals a `std::sync::mpsc` channel; (c)
    waits (bounded) on that channel for the contention signal and asserts the
    worker has **not** completed while the lock is held; (d) still holding the
    lock, pins a tree X via `GitSnapshotService` and makes X a durable root (a
    committed baseline / `initialize_worktree` + cursor at X, or a real prior
    `coordinate()` whose lock is then re-taken by the test); (e) drops the
    lock; (f) joins the worker and asserts it returns `Ok` with `deleted == 0`
    and X's ref still present. Reuse the
    `two_threads_on_the_same_worktree_serialize` structure already in this
    file. Out — any production-code change (the `pub(super)` seam already
    exists from T03); the retained-root / linked-worktree / no-write scenarios
    (T05).
  - Dependencies: T03
  - Done when: the test proves — via the `WorktreeLock` happens-before edge,
    with no `sleep`-based timing — that a reconciliation pass blocks for the
    entire interval another holder owns the lock, and that once it proceeds it
    observes X among the durable roots and retains X rather than deleting it;
    `services::mutation_trace::runtime::` passes.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::tests::reconciliation_blocks_on_the_worktree_lock`;
    `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::`.
  - Context synchronization: pending

- [ ] T05: `Add retained-root, orphan, missing-pin, idempotence, delete_pins-routing and linked-worktree integration tests` (status:todo)
  - Task ID: T05
  - Scope: In — `cli/src/services/mutation_trace/runtime/tests.rs`:
    cross-module tests driving only the public `reconcile_worktree` (and the
    public `coordinate`) API against real `git init` / `git worktree add`
    repositories and real temp-file `RepositoryAgentTraceDb`s (DB in a sibling
    temp dir, outside the worktree, so it never perturbs a captured tree —
    matching the `mutation-cursor-external-taint` T04 precedent):
    a `coordinate()` baseline (to establish checkout identity and a durable
    row) then an orphan pin created directly — capture a tree X and
    `GitSnapshotService::pin_tree` it under W's namespace with **no** durable
    root — then a later `reconcile_worktree` that deletes exactly that orphan
    (this simulates the observable post-crash / post-no-op state `pin exists ∧
    durable root does not`; the reconciler does not care how it arose, so no
    coordinator crash seam is used); a current cursor with no referencing event
    retained; historical `before_tree`/`after_tree` pins retained after the
    cursor advances through several `coordinate()` boundaries; two consecutive
    `reconcile_worktree` calls (idempotence); a normal pass whose stale batch
    is confirmed to have gone through `delete_pins` (the `refs/sce/...` entries
    are absent afterward) — the conditional-delete *atomicity* race itself is
    proven in T02 against `delete_pins` directly, not re-scheduled through the
    public reconciler (no `after-inventory / before-delete` seam exists);
    linked worktrees A and B over one shared DB where `reconcile_worktree(A)`
    leaves B's pins and shared objects intact and needs no pause in B,
    including the byte-identical-tree-content case; the cross-worktree
    degraded-state case — B durably references a tree T, B's own
    `refs/sce/mutation-cursor/<B-id>/T` is deliberately absent, A owns
    `refs/sce/mutation-cursor/<A-id>/T`, A does not durably reference T, and
    `reconcile_worktree(A)` **retains** A's T pin (T is a repository-wide
    durable root) with T still resolvable via `git cat-file -t` afterward
    (`reconcile_a_retains_its_pin_when_another_worktree_durably_requires_the_same_tree`,
    AC9); and a pass that leaves every `mutation_trace_*` row count, the
    worktree's protocol fields, the absence of the taint marker, and the
    migration directory unchanged, and an object reachable before the pass
    only through a stale SCE pin still resolvable via `git cat-file -t`
    immediately after that pin is deleted (no `git gc`). Out — any
    production-code change; documentation edits (handled by each task's own
    context synchronization).
  - Dependencies: T04
  - Done when: every scenario passes through the public entrypoints only, with
    no production-code change, including the cross-worktree degraded-state
    retention regression; `services::mutation_trace::runtime::` and the full
    CLI test suite pass.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::tests`;
    `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`.
  - Context synchronization: pending

## Design decisions

### Q1 — Why the existing per-worktree `WorktreeLock` is sufficient

`coordinate()` acquires `<git-dir>/sce/mutation-cursor.lock` (an OS advisory
lock via `std::fs::File::try_lock`, RAII-released) **before** arming the
external-taint marker and resolving checkout identity, and holds it across the
entire pipeline: `capture + pin → recover-if-needed → prepare/commit → DB CAS
→ marker clear → return`
(`context/cli/mutation-trace-runtime-coordinator.md`). The pin is therefore
created strictly inside the lock hold, and the tree stays "possibly not yet
durable" only until the DB CAS inside that same hold.

`reconcile_worktree` acquires the **same** lock file with the **same**
primitive (`worktree_lock::acquire_inner`, bounded by its own
`RECONCILIATION_LOCK_TIMEOUT` of 10s — a value that matches the coordinator's
private `WORKTREE_LOCK_TIMEOUT` by intent, not a shared constant) before it
lists pins, reads durable roots, or deletes anything.
OS advisory locks on one file are mutually exclusive, so the reconciler's
critical section runs entirely before `coordinate()` takes the lock or
entirely after `coordinate()` releases it — never interleaved. In the "before"
case the in-flight tree is not pinned yet, so it cannot be a deletion
candidate. In the "after" case `coordinate()` has already resolved the tree's
fate: committed (it is now a durable root and appears in the durable-root
queries → retained) or not committed (a genuine orphan, safe to delete). There
is no third state a reconciler can observe. No new lock, and no
repository-global lock, is needed.

The retention set the pass computes is repository-wide (Q2, Q4), but that is a
broadening of a **read**, not of the **lock**. An in-flight tree in worktree
`W` can only be created while `W`'s lock is held (above); a *concurrent*
worktree `B` always creates its own `refs/sce/mutation-cursor/<B-id>/X` before
`B` commits `X` durably. That repository-wide read is protected from a torn
view of a concurrent commit not by any lock but by being **one SQL
statement** — a single `UNION` of `cursor_tree` / `before_tree` /
`after_tree` read through one DB snapshot (Q4, AC1), so an atomic
`cursor T → X` + `event T → X` commit on `B` cannot expose a root set that
omits `T`. So a per-worktree lock plus a single-statement repository-wide
read is sufficient — see Q2, Q4, and "The challenge interleavings".

### Q2 — Linked worktrees: per-worktree lock, repository-wide retention set

Linked worktrees share one object database and one default ref namespace, but
each has a distinct `git-dir` and therefore a distinct
`<git-dir>/sce/mutation-cursor.lock` and a distinct `WorktreeId` (checkout
id). `pin_tree` scopes every ref by the `worktree_id` path segment
(`refs/sce/mutation-cursor/<worktree-id>/<tree-sha>`). `reconcile_worktree(A)`
still **mutates** only A's world:

- it lists only `refs/sce/mutation-cursor/<A-id>/` (Q10 — `git for-each-ref`
  constrained to that exact prefix);
- it deletes only refs it inventoried under A's prefix;
- it writes no row, and takes no lock other than A's.

But the deletion *decision* cannot be made from A's rows alone. Because the
object database is shared, an A-owned ref may be the last SCE ref protecting a
tree that **only worktree B** durably requires — B's `cursor_tree`,
or a `before_tree` / `after_tree` of one of B's historical events. B's
database row does not itself hold that tree reachable to Git; it means
reconciliation must keep some SCE ref protecting it. If A
decided staleness as `actual_A − load_tree_roots(A)`, it could delete
`refs/sce/mutation-cursor/<A-id>/T` while B still requires T, leaving **no** SCE
ref to protect T — so a later `git gc` could reclaim the objects and B's
durable cursor/evidence tree becomes unresolvable. That violates the core rule
that a pass must never remove a ref whose target tree any durable
mutation-cursor state in the repository still requires SCE to protect.

So A **must read** B's durable `TreeId`s — via `load_all_tree_roots()`, a
single SQL statement `UNION`-ing `cursor_tree` / `before_tree` / `after_tree`
across every worktree, read through one DB snapshot (Q4, AC1) — and remove an
A-owned ref only when its tree is in **no** worktree's durable root set:

```
stale_A = actual_A − load_all_tree_roots()
```

The corrected statement of the cross-worktree boundary: **A never mutates B's
refs or rows, but A must consider B's durable `TreeId`s when deciding whether
an A-owned ref is safe to remove from the shared Git object database.**

A repository-global lock is still not needed. The pin→CAS race is closed by
A's own per-worktree lock (Q1); a concurrent B always creates its own pin
before committing its tree; and the repository-wide root *read* is protected
from a torn view of B's atomic `cursor T → X` + `event T → X` commit by being
**one SQL statement** over one DB snapshot (Q4, AC1), not by any lock. So a
repository-wide single-statement *read* under a per-worktree *lock* is
race-free (see "The challenge interleavings"). Object identity is
content-addressed: if A and B pinned byte-identical content they pinned the
*same* object under two refs, and deleting A's ref cannot unreach it while B's
ref names it; object reclamation is Git's job, performed only for genuinely
unreachable objects on Git's own schedule (Q12). A repository-global lock
would only add contention with live coordinator traffic on unrelated
worktrees for no safety gain.

### Q3 — How the reconciler obtains the correct `WorktreeId`

It does not accept one. `reconcile_worktree(repository_root, open_db)` derives
identity exactly as `coordinate()` does: `repository_root` →
`checkout::resolve_git_dir` (returns the worktree-specific git-dir, including
for a linked worktree) → `checkout::read_checkout_id(&git_dir)` →
`WorktreeId(id)`. It uses `read_checkout_id` (not `get_or_create_checkout_id`)
so a read-only maintenance pass never creates an identity as a side effect.
The safety argument rests on **derivable current identity**, not on a claim
about history: `read_checkout_id` → `Ok(None)` means reconciliation has no
current checkout identity — no readable canonical `<git-dir>/sce/checkout-id`
— from which it can derive a `WorktreeId` and therefore no safe worktree-owned
`refs/sce/mutation-cursor/<worktree-id>/` prefix to reconcile. It is **not** a
claim that no `WorktreeId` or ref ever existed for this checkout, only that
none can be safely derived now. So there is nothing for the pass to inventory,
validate, or delete — it
releases the already-acquired `WorktreeLock` and returns an empty
`ReconciliationReport { local_required: 0, retained: 0, deleted: 0 }` without
creating checkout identity. `read_checkout_id` → `Err(_)` is a different thing
entirely: an
unreadable or corrupt checkout id is not an absent one, so it maps to
`ReconcileError::CheckoutIdentity` rather than a clean no-op.

### Q4 — The complete durable root set (per-worktree and repository-wide)

Verified against `cli/migrations/agent-trace-repository/003_mutation_trace_protocol.sql`
and `store.rs`. The five mutation-cursor tables and their `TreeId`-typed
columns:

| Table | `TreeId` columns |
| --- | --- |
| `mutation_trace_worktrees` | `cursor_tree` |
| `mutation_trace_events` | `before_tree`, `after_tree` |
| `mutation_trace_scopes` | none |
| `mutation_trace_processed_events` | none |
| `mutation_trace_event_active_scopes` | none |

`AttemptState` is never persisted (transient), and `external_taint` is never
DB-authoritative — both explicitly excluded by `store.rs` and migration `003`.
So the complete durable root set for one worktree is:

```
{ mutation_trace_worktrees.cursor_tree(W) }
  ∪ { mutation_trace_events.before_tree : row for W }
  ∪ { mutation_trace_events.after_tree  : row for W }
```

`load_tree_roots(W)` returns exactly this for one `W`, deduplicated, as a
`BTreeSet<TreeId>`. `load_all_tree_roots()` returns `⋃_V` of the same
expression across every worktree `V` — the same three columns
(`mutation_trace_worktrees.cursor_tree`, `mutation_trace_events.before_tree`,
`mutation_trace_events.after_tree`) with **no** `WHERE worktree_id` clause —
deduplicated. No other table is part of the durable root set in either query.

Cursor advances that emit a `MutationEvent` record the prior cursor as that
event's `before_tree`, so it stays a root; a cursor that moved via recovery
(no event) is correctly *not* a root and its pin becomes reclaimable **once no
worktree needs that tree**. Both queries are read-only over existing columns —
no migration is needed.

**Single-statement snapshot semantics.** Each of `load_tree_roots` and
`load_all_tree_roots` produces its complete root set from **one SQL
statement** — a `UNION` of the `cursor_tree`, `before_tree`, and `after_tree`
arms — read through **one** `query_map` call. The constituent tables are
never queried with independent `SELECT`s whose result vectors are later
unioned in Rust. This matters because a mutation-cursor commit atomically
performs, in a single DB transaction:

```
cursor_tree: T → X
INSERT MutationEvent { before_tree = T, after_tree = X }
```

If the root set were assembled from two independent `SELECT`s, one could read
`mutation_trace_events` before that transaction (T absent there) and
`mutation_trace_worktrees` after it (cursor already X), yielding a set that
omits T entirely — even though T is still a live durable root. The
one-statement read cannot do this: it observes either the pre-commit snapshot
(`cursor_tree` contains T) or the post-commit snapshot (`before_tree`
contains T, `after_tree` contains X). There is **no** snapshot in which
`cursor_tree` no longer contains T **and** `before_tree` does not yet contain
T, because the cursor update and the event insert commit together and the
query sees them through one statement. The safety does **not** rest on
"we query the cursor table first" or any ordering of separate reads — it is
structural.

The reconciler uses the per-worktree set for the fail-closed **local
consistency** check and the repository-wide set for the **deletion safety**
check (Q7, "Core invariants").

**Two distinct concurrency arguments, kept separate.** This design defends
against two unrelated races, with two different mechanisms:

1. **Git pin → DB CAS race** (same worktree): a `coordinate()` that has
   pinned a tree but not yet committed it durably. Guarded by the
   per-worktree `WorktreeLock` — the reconciler takes the same lock file
   `coordinate()` holds across `pin → CAS → return`, so it can never observe
   a pinned-but-uncommitted tree (Q1).
2. **Repository-wide durable-root read vs. a concurrent commit on another
   worktree**: a torn view of the cursor/event tables while some other
   worktree's `coordinate()` commits. Guarded by the **single SQL statement
   snapshot** above — the whole root set comes from one coherent DB snapshot,
   so an atomic `cursor T → X` + `event T → X` commit on another worktree can
   never expose a mixed pre/post-commit root set (this Q4, and "The challenge
   interleavings").

The complete model: **same worktree — `WorktreeLock` prevents seeing a
pinned-but-uncommitted tree; other worktrees — each coordinator creates its
pin before its atomic DB commit, and reconciliation reads the cursor/event
durable roots from one DB snapshot.** No repository-global lock is needed.

### Q5 — Worktree never materialized in the DB

T03's ordering is `resolve_git_dir` → **acquire `WorktreeLock`** →
`read_checkout_id` → …. So `read_checkout_id` → `Ok(None)` means: the
reconciler has already acquired `WorktreeLock(W)`, then finds no checkout
identity, then **releases the lock and returns**
`ReconciliationReport { local_required: 0, retained: 0, deleted: 0 }` without
creating identity and without any DB or Git-ref work (reconciliation cannot
derive a current `WorktreeId`, so it has no worktree-owned ref prefix to
reconcile — Q3). The return happens *after* lock acquisition, not
before it. `read_checkout_id` → `Err(_)` ⇒ `ReconcileError::CheckoutIdentity`
(a corrupt id is not an absent id). If the checkout id exists but there is no
`mutation_trace_worktrees` row (a `coordinate()` that pinned then failed
before `initialize_worktree`, or a different Agent Trace path created the
checkout id), `load_tree_roots(W)` returns the empty set, `required_local` is
empty, `missing_local` is empty, and every pin under the prefix that is also
absent from `load_all_tree_roots()` is stale and deleted — the orphan-pin
case (Q7 note, AC2). Safe: nothing durable references those trees and the
lock guarantees nothing in-flight does.

### Q6 — No refs for a worktree

`list_pins` returns an empty vector. If `required_local` is also empty ⇒
`ReconciliationReport { 0, 0, 0 }`, success, idempotent. If `required_local`
is non-empty ⇒ every locally required tree is missing a pin ⇒
`MissingRequiredPins` fail-closed (Q7).

### Q7 — A durable root of the *target* worktree has no corresponding ref

Fail closed. `missing_local = load_tree_roots(W) − {p.tree for p in actual_W}`
non-empty ⇒ `Err(ReconcileError::MissingRequiredPins { missing: missing_local
})`, **delete nothing**. This is the **local consistency invariant**
violation: a tree `W`'s own durable evidence still references has lost its
pin, so a `git gc` could already have reclaimed its objects.

This check is deliberately **local**, never repository-wide. A missing pin in
some *other* worktree `B` is not a reason to abort `A`'s pass — requiring
every worktree to hold a complete pin set before `A` could reconcile would
let one worktree's degradation block maintenance everywhere, for no safety
gain. Instead: if `B` requires `T` and `A` also has a `T` pin, `A` must
**retain** `A/T` because `T ∈ load_all_tree_roots()` (the deletion safety
invariant, Q2). `A`'s otherwise-stale ref then acts as conservative
accidental backup reachability for `B`'s degraded state — reconciliation of
`A` cannot be the step that turns `B`'s degraded-but-recoverable state into
evidence loss (AC9, and the cross-worktree challenge interleaving).

Automatic repair of a genuinely missing pin is out of scope — recreating the
ref only restores the guarantee if the underlying object still exists, which
requires separate reasoning this PR does not attempt. The first version
detects the inconsistency and stops; a later PR may add repair.

### Q8 — Partial cleanup after one delete succeeds and a later one fails

Cannot happen. `delete_pins` issues **one** `git update-ref --stdin`
transaction containing every stale ref's `delete` command. `git update-ref
--stdin` applies all commands in a single ref transaction, committed
atomically at end of input; if any command fails (including a failed old-value
check) the whole transaction aborts and **no** ref is changed
(`git update-ref` documentation; T02 demonstrates this against this
repository's Git). So the outcome is binary: all stale refs deleted, or none
deleted and `Err(ReconcileError::DeleteTransaction)`. On `Err` the caller
re-runs the pass later; the operation is idempotent (Q, AC8).

### Q9 — One-by-one vs. `git update-ref --stdin` atomic batch

Atomic batch, for an obvious safety property: "every stale ref is deleted, or
none is, and each delete is conditioned on the exact SHA observed at inventory
time" (stale = inventoried under `W`'s prefix ∧ tree ∉
`load_all_tree_roots()`). One-by-one conditional `git update-ref -d <ref>
<oldvalue>` calls would
leave a half-cleaned namespace on a mid-sequence failure and force this plan
to define partial-cleanup semantics; the batch removes that question entirely
(Q8). T02 validates the exact stdin format
(`delete SP <ref> SP <expected-sha> LF` per line, no explicit
`start`/`prepare`/`commit` needed) and the abort-on-mismatch behavior
experimentally, directly against `GitSnapshotService::delete_pins` (inventory
`R → X`, mutate `R → Y`, call `delete_pins` with the inventoried `R → X`,
assert the whole transaction fails and no ref was deleted). That direct test
is the canonical proof for AC10. The public `reconcile_worktree` integration
test (T05) does **not** independently schedule an `after-inventory /
before-delete` race — no such deterministic seam exists on the public path —
it only asserts that a normal pass routes its stale batch through
`delete_pins` (stale refs gone afterward).

### Q10 — Inventory from ref names, ref targets, or both

Both, and they must agree. `list_pins` runs `git for-each-ref
--format='%(refname) %(objectname) %(objecttype)'
refs/sce/mutation-cursor/<W>/` and, for each line, requires
`objecttype == "tree"` and the final `refname` path component to equal
`objectname`. The **target** (`objectname`) is the authoritative tree identity
— it is what Git reachability actually keys on and what the conditional delete
checks — but requiring agreement with the name catches a tampered or
corrupted ref. A disagreement, a non-tree target, or an unparseable line is
`Err(PinInventoryError::MalformedRef { ref_name, reason })` — a variant
matchable separately from `PinInventoryError::Git(_)` (a `git for-each-ref`
that failed to run or exited non-zero) (Q11). `list_pins`'s signature is
therefore `Result<Vec<PinnedRef>, PinInventoryError>`, not
`anyhow::Result<Vec<PinnedRef>>`.

### Q11 — Malformed / unexpected refs inside the SCE namespace

Fail closed. `refs/sce/mutation-cursor/**` is exclusively SCE-owned and every
ref in it is created only by `pin_tree` (`git update-ref <ref> <tree-sha>`).
Anything else — a non-tree target, a name/target mismatch, an unparseable
`for-each-ref` line, an extra path segment — means the reconciler's model of
the namespace is wrong, and cleanup must not proceed on a namespace it does
not fully understand. `list_pins` returns
`Err(PinInventoryError::MalformedRef { ref_name, reason })`;
`reconcile_worktree` maps it deterministically to
`ReconcileError::MalformedPin { ref_name, reason }` and deletes nothing. The
sibling mapping is `PinInventoryError::Git(e) → ReconcileError::PinInventory(e)`
— a `git for-each-ref` execution failure is a different, separately matchable
outcome. (Ignoring malformed refs would also preserve the never-false-delete
invariant, but "abort on the unexpected" is the more defensible rule for a
destructive maintenance pass and is trivially observable in a test.)

### Q12 — Observable error / result contract

`reconcile_worktree` returns `Result<ReconciliationReport, ReconcileError>`.
Every fallible step in the algorithm owns exactly one variant; there is no
`Other` catch-all:

| Step / situation | Result |
| --- | --- |
| success | `Ok(ReconciliationReport { local_required, retained, deleted })` |
| `read_checkout_id` → `Ok(None)` (lock already held; no current checkout identity to derive a `WorktreeId` / owned ref prefix from — no-op, no identity created) | `Ok(ReconciliationReport { 0, 0, 0 })` (lock released on return) |
| `resolve_git_dir` failure | `Err(ReconcileError::GitDir(_))` |
| `WorktreeLock` acquisition (timeout / I/O) | `Err(ReconcileError::Lock(WorktreeLockError))` |
| `read_checkout_id` → `Err(_)` (corrupt / unreadable id, **not** absent) | `Err(ReconcileError::CheckoutIdentity(_))` |
| `open_db()` provider failure (maintenance error only — never arms the taint marker, never `CoordinateError`) | `Err(ReconcileError::AgentTraceDbUnavailable(_))` |
| `GitSnapshotService::new` failure | `Err(ReconcileError::SnapshotService(_))` |
| `git for-each-ref` execution failure (`PinInventoryError::Git`) | `Err(ReconcileError::PinInventory(_))` |
| malformed namespace ref (`PinInventoryError::MalformedRef`) | `Err(ReconcileError::MalformedPin { ref_name, reason })`, nothing deleted |
| `load_tree_roots` / `load_all_tree_roots` failure (DB query error, migration `003` absent) | `Err(ReconcileError::DurableRoots(_))` |
| a **target-worktree** durable root has no pin (local consistency invariant) | `Err(ReconcileError::MissingRequiredPins { missing: Vec<TreeId> })`, nothing deleted |
| `delete_pins` transaction failure (incl. a ref that changed since inventory) | `Err(ReconcileError::DeleteTransaction(_))`, nothing deleted (Q8) |

`read_checkout_id() == Ok(None)` (clean no-op) and `read_checkout_id() ==
Err(_)` (`CheckoutIdentity`) are never conflated. An `open_db()` failure here
is a reconciliation maintenance error and nothing more — it does **not**
become `CoordinateError::AgentTraceDbUnavailable` and does **not** arm
`ExternalTaintMarker`, because no mutation boundary is being coordinated.
Partial cleanup is not a representable outcome. Every error variant leaves the
ref namespace in a consistent state (either untouched, or — only on `Ok` —
with exactly the stale refs gone).

### Q13 — Does reconciliation need `ExternalTaintMarker`? No.

The external-taint marker exists so that a lost mutation-observation interval
(a DB write that could not be recorded) leaves a signal for the next
invocation to rebaseline conservatively. A reconciliation failure loses no
observation interval and casts no doubt on any committed `MutationEvent` — it
only means an obsolete ref (and the disk it holds) was not reclaimed this
time. Arming the marker on a reconciliation failure would force a spurious
conservative recovery and scope abandonment on the next `coordinate()`,
turning a storage-cleanup hiccup into lost attribution. Reconciliation
therefore never constructs, inspects, persists, or clears `ExternalTaintMarker`,
never calls `protocol::database_failure` / `protocol::taint` /
`protocol::recover`, and never writes any `mutation_trace_*` row (AC11).

### Q14 — Does this need Quint / model updates? No.

`spec/mutation_cursor.qnt` models the protocol state machine:
`worktrees.cursorTree`, `worktrees.revision`, scope lifecycle,
`processedEvents`, `mutationEvents`, attribution, taint, `externalTaint`,
recovery. `spec/mutation_cursor.md` states explicitly that "Git commands and
snapshot mechanics are not modeled" and that object reclamation / OS timing
are out of the model. Ref reconciliation:

- does not choose or change attribution;
- does not advance the cursor or the revision;
- does not change any scope's status;
- does not create, modify, or delete a `MutationEvent`;
- only maintains Git ref *reachability* for trees the protocol has already
  decided are durable.

It operates entirely below the model boundary, on the imperative
snapshot-storage substrate the model abstracts away. No `spec/mutation_cursor.qnt`
change, and no Quint refinement-matrix entry, is warranted; the `mbt/`
harness and `checks.mutation-trace-quint-connect` must stay green unchanged.

### Q15 — Future invocation point

Not decided here — the deliverable is the safe primitive, not its schedule. A
reconciliation failure must never turn a successfully committed mutation
boundary into a failed one, so it must not be inlined into `coordinate()`'s
result path. Likely future call sites, for the harness-wiring PR to choose
among: a `Close` boundary (bounded per-scope frequency), a `Flush` boundary,
or an explicit `sce` maintenance / `sce doctor --fix` path. Recorded as
candidates only; this PR wires none of them and adds no `pub(crate)`
re-export.

### Report shape

`ReconciliationReport { local_required: usize, retained: usize, deleted:
usize }`.

- `local_required` = `load_tree_roots(W).len()` — the target worktree's own
  durable-root count, the left side of the local consistency invariant.
- `deleted` = the stale-pin count actually removed (inventoried under `W`'s
  prefix, tree absent from `load_all_tree_roots()`).
- `retained` = `actual_W.len() − deleted`.

`retained == local_required` is **not** an invariant and this plan no longer
claims it. Counter-example: `local_required = {A}`,
`load_all_tree_roots() = {A, B}`, `actual_W = {A, B}` ⇒ `local_required = 1`,
`deleted = 0`, `retained = 2` — `W`'s `B`-pin is retained because another
worktree durably needs `B`. The only relation that holds on the `Ok` path is
`local_required ≤ retained`: the fail-closed check guarantees every locally
required tree is pinned, and `W` may additionally retain pins other worktrees
need.

A `repository_required` field was considered and rejected — computing it is a
second `len()` with no operational value a caller acts on. A dedicated struct
(not a bare tuple) still leaves room for a future `dry_run` / `skipped` field
without a breaking change.

### The challenge interleavings

**`pin X` → reconciler doesn't see X in DB → reconciler deletes X → DB commits
X.** Impossible by construction. `coordinate()` holds `WorktreeLock(W)` from
before `pin X` until after the DB CAS that commits X. `reconcile_worktree(W)`
acquires the *same* `WorktreeLock(W)` before it lists pins or reads roots.
Mutual exclusion on that one lock file means the reconciler's list-pins →
read-roots → delete sequence runs wholly before `coordinate()` takes the lock
(X not pinned yet → not a candidate) or wholly after `coordinate()` releases
it (X already committed → in `load_tree_roots(W)` → retained; or X never
committed → true orphan → safe to delete). The reconciler can only observe a
pinned-but-uncommitted X by holding the lock while `coordinate()` also holds
it, which cannot happen. The conditional `git update-ref -d <ref>
<expected-sha>` in the atomic batch is a second line of defence for the
lock-assumption-violated or external-tampering case: a ref whose value moved
since inventory fails its delete and aborts the whole transaction. That
conditional-delete atomicity is proven directly against
`GitSnapshotService::delete_pins` in T02 (AC10) — not by scheduling a
mid-pass race through the public `reconcile_worktree`, which has no
deterministic `after-inventory / before-delete` seam.

**`reconcile_worktree(A)` reads global roots while `coordinate()` coordinates
a new tree X on worktree B concurrently.** A mutates only refs under
`refs/sce/mutation-cursor/<A-id>/`, writes no DB row at all, and takes no lock
but A's — but A **does read** B's durable `TreeId`s through
`load_all_tree_roots()`, and that read is a snapshot that may miss X. Safe
without a global lock:

- `coordinate()` on B holds `WorktreeLock(B)` continuously from before
  `pin X` through the DB CAS that commits X. So **B creates
  `refs/sce/mutation-cursor/<B-id>/X` before X becomes durable.**
- If A's `load_all_tree_roots()` ran before B committed X, the only ref that
  must protect X once X is durable is B's own `.../<B-id>/X`, which B
  provably created first. A deleting some *unrelated* old A-owned pin cannot
  make X unreachable. Any tree A itself pinned and abandoned is a true orphan
  regardless of B.
- If A's read ran after B committed X, A sees X in the union and retains any
  A-owned pin to X.

So broadening the **retention-set read** to repository scope, while keeping
the **lock** per-worktree, closes the race.

**Torn root read across an atomic `cursor T → X` + `event T → X` commit on
another worktree.** This is a *different* race from the two above and needs a
*different* mechanism. Consider:

```
worktree B: cursor_tree = T,  B's own Git pin for T missing
worktree A: owns refs/sce/mutation-cursor/<A-id>/T  (currently the last Git
                                                     ref protecting T)
```

A's `reconcile` computes `required_repository = load_all_tree_roots()`.
Concurrently, B's `coordinate()` runs its atomic DB transaction: `cursor_tree
T → X` **and** `INSERT MutationEvent { before_tree = T, after_tree = X }`. If
A assembled the root set from two independent `SELECT`s, this ordering is
possible:

```
A: SELECT ... FROM mutation_trace_events   → T not present yet
B: atomic commit (cursor T→X, event T→X)
A: SELECT ... FROM mutation_trace_worktrees → cursor is X
⇒ A derives required_repository = {X}, missing T
⇒ A/T looks stale ⇒ A deletes A/T ⇒ T has no protecting Git ref
⇒ a later git gc reclaims T ⇒ B's durable evidence points at a missing tree
```

The `WorktreeLock` does **not** help here — it is per-worktree, and A and B
hold different locks. What closes this race is the **single SQL statement**
(AC1, Q4): `load_all_tree_roots()` reads `cursor_tree`, `before_tree`, and
`after_tree` in one `UNION` statement through one DB snapshot. That snapshot
is either entirely before B's commit (`cursor_tree` contains T ⇒ T retained)
or entirely after it (`before_tree` contains T ⇒ T retained). There is no
snapshot in which `cursor_tree` no longer contains T **and** `before_tree`
does not yet contain T, because B's cursor update and event insert commit
atomically and A's one statement observes them together. This is a structural
property of the one-statement read, not of any ordering of separate `SELECT`s
— "we happen to query the cursor table first" is explicitly **not** the
argument.

**Cross-worktree degraded state: B durably references T, B's own T pin is
missing, A has a locally-stale pin to T, `reconcile(A)` runs.**
`load_tree_roots(A)` does not contain T (A does not durably reference it), so
`refs/sce/mutation-cursor/<A-id>/T` looks locally stale. But
`load_all_tree_roots()` **does** contain T (it is one of B's durable roots),
so `stale_A = actual_A − load_all_tree_roots()` excludes T and A **retains**
`refs/sce/mutation-cursor/<A-id>/T`. B's database row does not itself hold T
reachable — it is `load_all_tree_roots()` recognizing T as durably required
that causes reconciliation to retain the A-owned ref, and that retained Git
ref is what keeps T reachable; `git cat-file -t T` still resolves it.
Reconciliation of A therefore cannot
be the step that makes T unreachable — B's state stays
degraded-but-recoverable exactly as it was before the pass. This is AC9's
degraded-state regression and the T05 test
`reconcile_a_retains_its_pin_when_another_worktree_durably_requires_the_same_tree`.

The shared object database is otherwise safe because Git reachability comes
only from Git refs and history: deleting A's ref to a content-addressed object
cannot unreach that object while any B ref or repository history still names
it. A B durable root or B cursor is not itself a Git reachability edge — it is
a logical durability requirement that, through `load_all_tree_roots()`, makes
reconciliation retain an SCE Git ref protecting that tree, and that retained
ref is what supplies Git reachability. This PR runs no
`git gc` — Git reclaims unreachable objects itself, later, and only when they
are genuinely unreachable.

## Open questions

None. The durable root set is confirmed against migration `003` and `store.rs`
(Q4). The two-invariant model is settled: the fail-closed **local
consistency** check is per-worktree (`load_tree_roots(W)`), and the
**deletion safety** check uses the **repository-wide** durable root set
(`load_all_tree_roots()`) so an A-owned ref is never removed while any
worktree still durably needs its tree ("Core invariants", Q2, Q7). Each
root-set API reads its complete set from **one SQL statement** (`UNION` of
`cursor_tree` / `before_tree` / `after_tree`) over one DB snapshot, never
multiple independent `SELECT`s unioned in Rust, so a concurrent atomic
`cursor T → X` + `event T → X` commit on another worktree cannot expose a
torn root set that omits `T` (AC1, Q4, T01, "The challenge interleavings"). The
synchronization model reuses the existing per-worktree `WorktreeLock` via
`worktree_lock::acquire_inner` with its own bounded
`RECONCILIATION_LOCK_TIMEOUT` and **no** repository-global lock. Two distinct
concurrency arguments, two mechanisms: the same-worktree Git pin → DB CAS
race is closed by the per-worktree `WorktreeLock`; the repository-wide
durable-root read is protected from a torn view of another worktree's atomic
`cursor T → X` + `event T → X` commit by each root-set API executing
**exactly one SQL statement** (`UNION` of `cursor_tree` / `before_tree` /
`after_tree`) over one DB snapshot — never multiple independent `SELECT`s
unioned in Rust (Q1, Q2, Q4, AC1, T01, "The challenge interleavings"). The
pin→CAS race is closed by
construction and its deterministic proof uses the `pub(super)
reconcile_worktree_inner` seam (T04, AC5); the pin-inventory error model is
the two-variant `PinInventoryError` mapped deterministically into
`ReconcileError` (Q10, Q11, Q12); `ReconcileError` has one variant per
fallible step and no `Other` catch-all (Q12); conditional-delete atomicity is
proven directly against `delete_pins` (T02, AC10); the batch-delete safety
property is atomic-or-nothing (Q8, Q9) pending only T02's experimental
confirmation of `git update-ref --stdin` semantics, which is a task
verification step, not a design risk. Invocation timing is intentionally
deferred to the harness-wiring PR (Q15).

Open questions: None.

## Final quality check

1. **Synchronization strategy:** per-worktree, reusing the existing
   `<git-dir>/sce/mutation-cursor.lock` `WorktreeLock` via
   `worktree_lock::acquire_inner`, bounded by the module-owned
   `RECONCILIATION_LOCK_TIMEOUT` (10s, matching the coordinator's private
   `WORKTREE_LOCK_TIMEOUT` by intent, not a shared constant).
   `reconcile_worktree` acquires it before any pin inventory, durable-root
   read, or ref deletion, and holds it until return — the same lock file
   `coordinate()` holds across `pin → CAS → return`. No new lock, **no
   repository-global lock**; linked worktrees reconcile independently under
   their own locks. Only the **retention-set read** is broadened to
   repository scope (item 3), not the lock — and that broadened read is kept
   coherent by being one SQL statement / one DB snapshot (item 4), not by a
   lock.
2. **Why the pin→CAS race is impossible:** `coordinate()` holds
   `WorktreeLock(W)` continuously from before `pin X` through the DB CAS that
   commits X; `reconcile_worktree(W)` takes the same lock before it can
   observe any pin. Mutual exclusion forces the reconciler's entire
   inventory→diff→delete sequence to run either before X is pinned or after X
   is either committed (⇒ a durable root ⇒ retained) or abandoned (⇒ a true
   orphan ⇒ safe). The conditional atomic `git update-ref --stdin` delete is a
   second-line guard against a ref value changing after inventory. A
   *concurrent* worktree B needs no shared lock: B always creates its own pin
   before committing its tree, and A's repository-wide root read is a single
   SQL statement over one DB snapshot, so it cannot tear across B's atomic
   `cursor T → X` + `event T → X` commit — A's single-statement repository-wide
   read plus A's own lock is sufficient (Q2, Q4, "The challenge
   interleavings").
3. **Two invariants:** (a) **local consistency** — `durable_roots(W) ⊆
   pinned_trees(W)`; if false, fail closed and delete nothing
   (`MissingRequiredPins`), a per-worktree check via `load_tree_roots(W)`.
   (b) **deletion safety** — delete `W/T` only if `T ∉ durable_roots(repository)`,
   the repository-wide set via `load_all_tree_roots()`, so an A-owned ref is
   retained whenever any worktree still durably needs its tree.
4. **Durable root set:** `mutation_trace_worktrees.cursor_tree` ∪
   `mutation_trace_events.before_tree` ∪ `mutation_trace_events.after_tree` —
   per worktree for `load_tree_roots`, unioned across all worktrees for
   `load_all_tree_roots`, deduplicated. Each API produces its full set from
   **one SQL statement** (a `UNION` of those three columns) through one
   `query_map` call / one DB snapshot — never separate per-table `SELECT`s
   unioned in Rust — so a concurrent atomic `cursor T → X` + `event T → X`
   commit cannot expose a torn root set (AC1, Q4, T01). No other
   mutation-cursor table stores a `TreeId`; `AttemptState` and
   `external_taint` are never persisted. No migration needed.
5. **Task stack:** T01 `load_tree_roots` + `load_all_tree_roots` read-only
   store queries (one SQL statement each — `SELECT_TREE_ROOTS_BY_WORKTREE_SQL`
   / `SELECT_ALL_TREE_ROOTS_SQL`, one `query_map` per API — plus a
   state-transition retention test and, separately, the deterministic
   single-statement enforcement regression that asserts one
   `load_*_tree_roots` call issues exactly one read statement via the
   `#[cfg(test)]` `count_read_statements` seam) → T02 typed
   `GitSnapshotService::list_pins`
   (`Result<Vec<PinnedRef>, PinInventoryError>`) + conditional atomic
   `delete_pins` (canonical AC10 proof) → T03 `runtime/ref_reconciliation.rs`
   per-worktree pass under `WorktreeLock` (`RECONCILIATION_LOCK_TIMEOUT`,
   `pub(super) reconcile_worktree_inner` seam, complete `ReconcileError`
   contract, local fail-closed check + repository-wide deletion set) → T04
   deterministic pin→CAS lock-race regression through the `pub(super)` seam →
   T05 public-API integration scenarios (retained-root / orphan-as-`pin
   without root` / missing-pin / idempotence / linked-worktree /
   cross-worktree degraded-state retention / no-write). No separate T06 — the
   repository's existing plan convention (context sync per task) is retained.
6. **`ReconciliationReport`:** `{ local_required: usize, retained: usize,
   deleted: usize }`. `retained == local_required` is **not** an invariant
   (removed from the plan); the only relation on the `Ok` path is
   `local_required ≤ retained`.
7. **Acceptance-criteria count:** 12 (AC9 now also carries the cross-worktree
   degraded-state regression).
8. **Unresolved design questions that should block implementation:** None.
