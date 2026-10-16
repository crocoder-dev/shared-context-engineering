# Plan: missing-history-recovery

## Change summary

Make sequential re-clones of the same remote recoverable without any per-clone
identity. The repository-scoped Agent Trace DB is keyed by remote identity and
the primary worktree is always `main`, so a fresh clone inherits the deleted
clone's `main` cursor row and `mutation_trace_events` while their snapshot trees
and `refs/sce/mutation-cursor/main/*` pins stayed in the deleted `.git`
(observed 2026-10-01: 1017 unpinned roots, 985 absent objects, every
`sce doctor --fix` failing with `missing_required_pins`).

Detection is a heuristic: a durable tree that is absent from the current Git
object store is history this checkout cannot have produced or kept. It detects
missing history, not clone identity. A durable tree that does exist (for example
a pushed commit tree) is not evidence that the clone is unchanged. Recovery runs
only from the explicit `sce doctor --fix` reconciliation pass. Plain `sce doctor`
reports the condition read-only, and the agent hook path never destructively
resets history.

The plan distinguishes three cases for the target worktree `W`:

1. **Missing non-cursor trees:** prune every `W` event whose `before_tree` or
   `after_tree` is absent (with its `mutation_trace_event_active_scopes` rows)
   and re-pin the surviving unpinned roots. This repair already exists as
   uncommitted work in progress in the working tree (`repair_missing_pins`,
   `GitSnapshotService::{absent_trees, pin_trees}`,
   `MutationTraceStore::{load_mutation_event_trees, prune_mutation_events}`,
   new doctor counts and failure kinds, tests, context). The plan preserves it
   and lands it as T01.
2. **Missing cursor tree:** today the repair stops with `unrecoverable_cursor`.
   Replace that stop with a validated cursor transition that reuses the existing
   Quint-refined recovery: capture and pin the current worktree tree, then commit
   `protocol::recover(protocol::database_failure(state, W), W, observed_tree)`
   through `DurableTransition::between` and the revision-CAS `MutationTraceStore::commit`.
   The cursor becomes the captured tree, the revision advances by exactly one,
   and every live `W` scope is abandoned. Case 1 pruning then removes the events
   that still reference absent trees. `unrecoverable_cursor` remains the failure
   when that transition cannot be established (for example the capture, the pin,
   or the stuttering `recover` at `revision: u64::MAX`).
3. **Existing cursor tree:** no destructive action and no claim that the clone is
   unchanged. Reconciliation continues under the stated heuristic limitation.

## Acceptance criteria

- [ ] AC1: With a durable `main` cursor that exists and events that reference
  absent trees, `sce doctor --fix` prunes exactly the events with an absent
  `before_tree` or `after_tree` (and their active-scope rows), keeps every other
  event, re-pins the surviving unpinned roots, and completes.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml ref_reconciliation_prunes_events_whose_trees_are_absent_and_repins_the_rest` runs one test and it passes (confirm with `-- --list`).
- [ ] AC2: With a durable `main` cursor tree that is absent from the object store,
  `sce doctor --fix` moves the cursor to the freshly captured and pinned
  worktree tree, advances the revision by exactly one, abandons every live
  `main` scope, prunes the events that reference absent trees, and completes.
  - Validate: a focused `ref_maintenance` test named for this case passes, and asserts cursor, revision, scope statuses, surviving events, and pins.
- [ ] AC3: A missing-cursor recovery that loses its revision CAS changes nothing
  durable: no cursor change, no pruned event, no abandoned scope.
  - Validate: a focused `ref_maintenance` test that advances the revision between the read and the commit passes and asserts the DB rows are unchanged.
- [ ] AC4: Plain `sce doctor` (text and JSON) reports when `main` has durable
  snapshot trees absent from the object store, names `sce doctor --fix`, and
  writes nothing to the DB, refs, or `ref-maintenance.json`.
  - Validate: a focused doctor test asserts the problem in text and JSON output and byte-identical DB, refs, and state before and after.
- [ ] AC5: No agent hook boundary prunes events, rewrites the cursor, or abandons
  scopes because of absent trees.
  - Validate: `rg -n "prune_mutation_events|repair_missing_pins" cli/src/services/hooks cli/src/services/mutation_trace/runtime/coordinator.rs` returns no matches.
- [ ] AC6: No per-clone identity is introduced: no checkout or clone ID file, no new identity column or migration, and `resolve_worktree_id` still returns `main` for the primary worktree.
  - Validate: `ls cli/migrations/agent-trace-repository/` gains no file from this plan's commits, and `rg -n "checkout_id|clone_id" cli/src` returns no matches.

### Full validation

- `nix flake check`

### Context sync

- `context/cli/mutation-trace-ref-reconciliation.md`
- `context/cli/doctor-ref-reconciliation.md`
- `context/cli/mutation-trace-runtime-coordinator.md`
- `context/sce/agent-trace-hook-doctor.md`
- `context/glossary.md`
- `context/context-map.md`

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/mutation_trace/runtime/{ref_reconciliation,ref_maintenance,ref_doctor,git_snapshot}.rs`, `cli/src/services/mutation_trace/store.rs`, `cli/src/services/doctor/{mod,render,inspect}.rs`, focused tests under `cli/src/services/mutation_trace/runtime/ref_maintenance/tests*` and the doctor test modules, and the context files listed under Context sync.
- **Out of scope:** concurrent independent clones of the same remote sharing one DB; clone-qualified worktree IDs; any change to the `refs/sce/mutation-cursor/<worktree>/` namespace; the orphaned legacy-format worktree row (`01a09050-…`) and other non-target worktrees; the `AGENTS.md` Turso-shell update already in the working tree (a separate commit).
- **Constraints:**
  - Preserve the `remove-checkout-id` architecture: no checkout ID, clone ID, persistent marker, or schema migration for identity.
  - Destructive DB changes happen only in the explicit `doctor --fix` pass, under the held worktree lock, behind a revision CAS, and are idempotent across a crash between steps.
  - The verified maintenance connection stays read-only; a writable connection is opened on the verified path only when a write is required, after the read-only one is dropped.
  - The missing-cursor transition reuses `protocol::database_failure` + `protocol::recover` + `DurableTransition::between`; it adds no new protocol action.
  - Code stays comment-free, matching the surrounding modules.
- **Non-goal:** proving clone identity. An existing tree is never treated as proof that history is local.

## Invariants

- I1: After a completed pass, every durable root of `W` (cursor and event trees) is present in the object store and pinned under `W`'s namespace.
- I2: An event is pruned only if one of its trees is absent; every event whose trees are all present survives.
- I3: The cursor changes only through the `recover` transition, which advances the revision by exactly one and is committed only if the revision is unchanged since it was read.
- I4: A failure at any step leaves either the pre-pass state or a state the next pass repairs (pins added before a lost CAS are orphans that a later pass deletes; a prune committed before a failed re-pin leaves only present unpinned roots).
- I5: Stale-pin deletion keeps its repository-wide rule: a pin is deleted only if its tree is a durable root of no worktree.

## Assumptions

- Missing-cursor recovery treats the dead clone's live scopes as abandoned (via `database_failure`'s external taint), because no harness in the new clone can ever close them.
- Plain-doctor detection runs one batched `git cat-file --batch-check` over `W`'s unpinned durable roots, reusing `absent_trees`, so its cost is one DB read plus one Git process.
- The working-tree repair is landed as-is in T01, except for any fixes its own verification requires.

## Task stack

- [ ] T01: `Land missing non-cursor tree repair in doctor --fix` (status:todo)
  - Task ID: T01
  - Scope: In — the uncommitted repair already in the working tree: `absent_trees`/`pin_trees`, `load_mutation_event_trees`/`prune_mutation_events`/public `load_worktree_state`, `repair_missing_pins` with `MaintenanceDb` path threading, new `ReconcileError` variants and failure kinds, `repinned`/`pruned_events` counts in doctor text and JSON, the updated and added `ref_maintenance` tests, and the three updated context files. Out — `AGENTS.md`, missing-cursor recovery (T02), plain-doctor detection (T03).
  - Dependencies: none
  - Done when: case 1 behaves as AC1, the `unrecoverable_cursor` stop is unchanged, and the change lands as one commit.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml -- ref_reconciliation reconciliation_fix json_ref_reconciliation json_completed_state outcomes_map`; `nix flake check`.
  - Context synchronization: pending

- [ ] T02: `Recover an absent cursor through the validated recover transition` (status:todo)
  - Task ID: T02
  - Scope: In — in `repair_missing_pins`, when `W`'s cursor tree is absent: capture the worktree tree, pin it, load the worktree projection, apply `protocol::recover(protocol::database_failure(..))`, commit through `DurableTransition::between` + `MutationTraceStore::commit` on the writable connection, then continue with case 1 pruning and re-pinning. Report the recovery in the pass result. Keep `UnrecoverableCursor` for a transition that cannot be established and map a lost CAS to the existing conflict failure. Add the AC2 and AC3 tests and replace `ref_reconciliation_absent_cursor_tree_fails_closed_without_pruning` with them. Out — any hook-path behavior, plain doctor output, new protocol actions.
  - Dependencies: T01
  - Done when: AC2 and AC3 hold, I1–I4 hold for the new path, and the existing reconciliation tests still pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml -- ref_reconciliation`; `nix flake check`.
  - Context synchronization: pending

- [ ] T03: `Report absent durable history in plain sce doctor` (status:todo)
  - Task ID: T03
  - Scope: In — a read-only doctor inspection that opens the verified DB read-only, loads `main`'s unpinned durable roots, classifies them with `absent_trees`, and emits one `mutation_scope_health` problem (text and JSON) naming the absent count, whether the cursor is absent, and `sce doctor --fix`; one focused test for AC4. Out — any write, any change to the reconciliation recommendation cadence in `ref-maintenance.json`, hook advisories.
  - Dependencies: T02
  - Done when: AC4 holds and doctor output for a healthy repository is unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml -- doctor`; `nix flake check`.
  - Context synchronization: pending

- [ ] T04: `Document the missing-history heuristic and its limits` (status:todo)
  - Task ID: T04
  - Scope: In — describe the three cases, invariants I1–I5, and the heuristic limitation (detects missing history, not clone identity; existing trees prove nothing) in `context/cli/mutation-trace-ref-reconciliation.md` and `context/sce/agent-trace-hook-doctor.md`; add a `missing durable history` glossary entry; fix the drifted present-tense `checkout identity` glossary entries (lines describing `sce setup` creating `<git-dir>/sce/checkout-id`) to the removed framing used by `context/cli/checkout-identity.md`; update `context/context-map.md` annotations. Out — code changes.
  - Dependencies: T03
  - Done when: context describes current behavior with no claim that SCE tracks clone identity, and `rg -in "creates/reuses checkout identity|Stable UUIDv7 identifier assigned" context/glossary.md` returns no matches.
  - Verify: `rg -in "checkout identity" context/glossary.md` shows only removed framing; manual read of the reconciliation doc's case table.
  - Context synchronization: pending

## Open questions

- Is T02 worth building? The cursor is absent only between a re-clone and the next agent hook boundary, because the next boundary's normal commit moves the cursor to a present tree and turns the old cursor into an event `before_tree`, which T01 already prunes. On 2026-10-16 the cursor here was present and only case 1 applied. The smaller alternative is to drop T02 and keep the `unrecoverable_cursor` stop, telling the operator to run any agent tool once and rerun `sce doctor --fix`.
- Is T03 worth building? `doctor --fix` already runs reconciliation unconditionally, and plain `sce doctor` already recommends `--fix` after a failed or stale pass. T03 adds a DB read and a Git process to every plain `sce doctor` run, only to surface the condition before the first reconciliation attempt. The smaller alternative is to drop T03 and rely on the existing reconciliation recommendation.
