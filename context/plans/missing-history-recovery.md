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
a pushed commit tree) is not evidence that the clone is unchanged. Repair runs
only from the explicit `sce doctor --fix` reconciliation pass; the agent hook
path never destructively resets history, and plain `sce doctor` keeps relying on
the existing reconciliation recommendation.

The plan distinguishes three cases for the target worktree `W`:

1. **Missing non-cursor trees:** prune every `W` event whose `before_tree` or
   `after_tree` is absent (with its `mutation_trace_event_active_scopes` rows)
   and re-pin the surviving unpinned roots. This repair already exists as
   uncommitted work in progress in the working tree (`repair_missing_pins`,
   `GitSnapshotService::{absent_trees, pin_trees}`,
   `MutationTraceStore::{load_mutation_event_trees, prune_mutation_events}`,
   new doctor counts and failure kinds, tests, context). The plan preserves it
   and lands it as T01.
2. **Missing cursor tree:** the pass keeps failing closed with
   `unrecoverable_cursor` and changes nothing. This state is transient: the next
   agent hook boundary commits a new present cursor and turns the old cursor into
   an event `before_tree`, which case 1 then prunes. No separate cursor-recovery
   mechanism is built.
3. **Existing cursor tree:** no destructive action and no claim that the clone is
   unchanged. Reconciliation continues under the stated heuristic limitation.

## Acceptance criteria

- [x] AC1: With a durable `main` cursor that exists and events that reference
  absent trees, `sce doctor --fix` prunes exactly the events with an absent
  `before_tree` or `after_tree` (and their active-scope rows), keeps every other
  event, re-pins the surviving unpinned roots, and completes.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml ref_reconciliation_prunes_events_whose_trees_are_absent_and_repins_the_rest` runs one test and it passes (confirm with `-- --list`).
- [x] AC2: With a durable `main` cursor tree that is absent from the object
  store, `sce doctor --fix` fails with `unrecoverable_cursor`, prunes no event,
  changes no pin, and its message tells the operator to let one agent tool run
  in this checkout and then rerun `sce doctor --fix`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml ref_reconciliation_absent_cursor_tree_fails_closed_without_pruning` runs one test and it passes, and the `UnrecoverableCursor` display text contains `rerun \`sce doctor --fix\``.
- [x] AC3: No agent hook boundary prunes events, rewrites the cursor, or abandons
  scopes because of absent trees.
  - Validate: `rg -n "prune_mutation_events|repair_missing_pins" cli/src/services/hooks cli/src/services/mutation_trace/runtime/coordinator.rs` returns no matches.
- [x] AC4: No per-clone identity is introduced: no checkout or clone ID file, no new identity column or migration, and `resolve_worktree_id` still returns `main` for the primary worktree.
  - Validate: `ls cli/migrations/agent-trace-repository/` gains no file from this plan's commits, and `rg -n "checkout_id|clone_id" cli/src` returns no matches.
- [x] AC5: Durable context describes the three cases and the heuristic limitation,
  and no context file claims SCE currently tracks checkout or clone identity.
  - Validate: `rg -in "creates/reuses checkout identity|Stable UUIDv7 identifier assigned" context/glossary.md` returns no matches, and `context/cli/mutation-trace-ref-reconciliation.md` states that an existing tree is not evidence of an unchanged clone.

### Full validation

- `nix flake check`

### Context sync

- `context/cli/mutation-trace-ref-reconciliation.md`
- `context/cli/doctor-ref-reconciliation.md`
- `context/cli/mutation-trace-runtime-coordinator.md`
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

- **In scope:** `cli/src/services/mutation_trace/runtime/{ref_reconciliation,ref_maintenance,ref_doctor,git_snapshot}.rs`, `cli/src/services/mutation_trace/store.rs`, `cli/src/services/doctor/{mod,render}.rs`, focused tests under `cli/src/services/mutation_trace/runtime/ref_maintenance/tests*` and the doctor test modules, and the context files listed under Context sync.
- **Out of scope:** recovering an absent cursor through a new cursor transition; plain `sce doctor` detection of absent trees; concurrent independent clones of the same remote sharing one DB; clone-qualified worktree IDs; any change to the `refs/sce/mutation-cursor/<worktree>/` namespace; the orphaned legacy-format worktree row (`01a09050-…`) and other non-target worktrees; the `AGENTS.md` Turso-shell update already in the working tree (a separate commit).
- **Constraints:**
  - Preserve the `remove-checkout-id` architecture: no checkout ID, clone ID, persistent marker, or schema migration for identity.
  - Destructive DB changes happen only in the explicit `doctor --fix` pass, under the held worktree lock, behind a revision CAS, and are idempotent across a crash between steps.
  - The verified maintenance connection stays read-only; a writable connection is opened on the verified path only when a prune is required, after the read-only one is dropped.
  - Code stays comment-free, matching the surrounding modules.
- **Non-goal:** proving clone identity. An existing tree is never treated as proof that history is local.

## Invariants

- I1: After a completed pass, every durable root of `W` (cursor and event trees) is present in the object store and pinned under `W`'s namespace.
- I2: An event is pruned only if one of its trees is absent; every event whose trees are all present survives.
- I3: The pass never changes `W`'s cursor; an absent cursor fails closed with nothing changed.
- I4: A failure at any step leaves either the pre-pass state or a state the next pass repairs (a prune committed before a failed re-pin leaves only present unpinned roots).
- I5: Stale-pin deletion keeps its repository-wide rule: a pin is deleted only if its tree is a durable root of no worktree.

## Assumptions

- The working-tree repair is landed as-is in T01, except for the `UnrecoverableCursor` guidance text and any fixes its own verification requires.
- An absent cursor is transient because every committed hook boundary replaces `cursor_tree` with the freshly captured, pinned tree (`coordinator.rs` commit path).

## Task stack

- [x] T01: `Repair missing pins and prune unrecoverable events in doctor --fix` (status:done)
  - Task ID: T01
  - Scope: In — the uncommitted repair already in the working tree: `absent_trees`/`pin_trees`, `load_mutation_event_trees`/`prune_mutation_events`/public `load_worktree_state`, `repair_missing_pins` with `MaintenanceDb` path threading, new `ReconcileError` variants and failure kinds, `repinned`/`pruned_events` counts in doctor text and JSON, the updated and added `ref_maintenance` tests, the three already-updated reconciliation context files; plus `UnrecoverableCursor` display text that tells the operator to let one agent tool run and rerun `sce doctor --fix`. Out — `AGENTS.md`, glossary and heuristic documentation (T02), any cursor-recovery transition, plain-doctor detection.
  - Dependencies: none
  - Done when: AC1, AC2, AC3 and AC4 hold and the change lands as one commit.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml -- ref_reconciliation reconciliation_fix json_ref_reconciliation json_completed_state outcomes_map`; `nix flake check`.
  - Completed: 2026-10-16
  - Files changed: `cli/src/services/mutation_trace/runtime/ref_reconciliation.rs`, `cli/src/services/mutation_trace/runtime/ref_maintenance/tests/fail_closed.rs`, `cli/src/services/doctor/mod.rs`, `context/plans/missing-history-recovery.md`
  - Result: The missing-pin repair had already landed in `8bcec9a1`; this task appended operator guidance to the `UnrecoverableCursor` message ("Let one agent tool run in this checkout so it records a present cursor, then rerun `sce doctor --fix`."), asserted it in `ref_reconciliation_absent_cursor_tree_fails_closed_without_pruning`, and replaced two `push_str(&format!(..))` calls in `reconciliation_completed_text` with `write!` to satisfy clippy `format_push_string`, which failed `nix flake check`.
  - Verify outcomes: targeted test filter set passed (53 passed, 0 failed); AC1 and AC2 single-test runs each ran exactly one test and passed; AC3 `rg` returned no matches; AC4 `rg -n "checkout_id|clone_id" cli/src` returns only the pre-existing `ReconcileError::CheckoutIdentity(_) => "checkout_identity"` failure-kind mapping (`ref_maintenance.rs:206`), no migration file was added, and `resolve_worktree_id` still returns `main` (`git_snapshot.rs:686`); `nix flake check` passed after the clippy fix.
  - Deviations: The plan assumed the repair was uncommitted; it was committed in `8bcec9a1`, so T01 lands as a follow-up commit containing only the guidance text, test assertion, and clippy fix. AC4's `rg` check matches pre-existing `checkout_identity`, which is not per-clone identity; no new identity was introduced.
  - Context impact: localized — `UnrecoverableCursor` operator guidance in `context/cli/mutation-trace-ref-reconciliation.md`; no architecture, contract, or terminology change.
  - Context synchronization: synced

- [x] T02: `Document the missing-history heuristic and its limits` (status:done)
  - Task ID: T02
  - Scope: In — describe the three cases, invariants I1–I5, and the heuristic limitation (detects missing history, not clone identity; existing trees prove nothing; an absent cursor clears after one hook boundary) in `context/cli/mutation-trace-ref-reconciliation.md`; add a `missing durable history` glossary entry; fix the drifted present-tense `checkout identity` glossary entries (lines describing `sce setup` creating `<git-dir>/sce/checkout-id`) to the removed framing used by `context/cli/checkout-identity.md`; update `context/context-map.md` annotations. Out — code changes.
  - Dependencies: T01
  - Done when: AC5 holds.
  - Verify: `rg -in "checkout identity" context/glossary.md` shows only removed framing; manual read of the reconciliation doc's case list.
  - Completed: 2026-10-16
  - Files changed: `context/cli/mutation-trace-ref-reconciliation.md`, `context/glossary.md`, `context/context-map.md`, `context/plans/missing-history-recovery.md`
  - Result: Added a `Missing durable history (heuristic)` subsection to the reconciliation doc covering the three cases, invariants I1–I5, the hook-path/plain-doctor boundaries, and the limitation that detection finds missing history, not clone identity, and that an existing tree is not evidence of an unchanged clone; reworded repair step 2 so an absent cursor is transient (one agent tool run, then rerun `sce doctor --fix`) instead of "does not belong to this checkout". Rewrote the `checkout identity` glossary entry to removed framing, added a `missing durable history` entry, and removed present-tense checkout-identity claims from the `repository identity`, `checkout registry`, `setup local bootstrap`, `ServiceLifecycle`, and `sce doctor` glossary entries. Extended the reconciliation annotation in `context/context-map.md`.
  - Verify outcomes: `rg -in "checkout identity" context/glossary.md` matches only removed framing (lines 39, 41, 47); AC5 `rg -in "creates/reuses checkout identity|Stable UUIDv7 identifier assigned" context/glossary.md` returned no matches (exit 1); `context/cli/mutation-trace-ref-reconciliation.md:174` states "An existing tree is **not evidence of an unchanged clone**"; manual read confirmed the three-case list and I1–I5.
  - Deviations: Also corrected the drifted checkout-identity claims in the `repository identity`, `setup local bootstrap`, `ServiceLifecycle`, and `sce doctor` glossary entries, which the AC5 check (`creates/reuses checkout identity` at the `setup local bootstrap` entry) required; moved the existing `TreeId` paragraph above the new subsection so it stays with the repair steps.
  - Context impact: localized — reconciliation domain doc, glossary terminology (`missing durable history`, removed `checkout identity`), and context-map annotation; no code, architecture, or contract change.
  - Context synchronization: synced

## Open questions

None. The value questions on cursor recovery and plain-doctor detection were resolved by dropping both.

## Validation Report

**Status:** validated  
**Date:** 2026-10-16

### Commands run

- `nix flake check` -> exit 0 (all checks passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml ref_reconciliation_prunes_events_whose_trees_are_absent_and_repins_the_rest -- --list` -> exit 0 (1 test listed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml ref_reconciliation_prunes_events_whose_trees_are_absent_and_repins_the_rest` -> exit 0 (1 passed, 608 filtered out)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml ref_reconciliation_absent_cursor_tree_fails_closed_without_pruning -- --list` -> exit 0 (1 test listed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml ref_reconciliation_absent_cursor_tree_fails_closed_without_pruning` -> exit 0 (1 passed, 608 filtered out)
- `rg -n "prune_mutation_events|repair_missing_pins" cli/src/services/hooks cli/src/services/mutation_trace/runtime/coordinator.rs` -> exit 1 (no matches)
- `ls cli/migrations/agent-trace-repository/` and `git status --short cli/migrations` -> exit 0 (001–007 only; no migration added or changed)
- `rg -n "checkout_id|clone_id" cli/src` -> exit 1 (no matches)
- `rg -in "creates/reuses checkout identity|Stable UUIDv7 identifier assigned" context/glossary.md` -> exit 1 (no matches)

### Success-criteria verification

- [x] AC1: `doctor --fix` prunes exactly events with absent trees, keeps the rest, re-pins, completes -> exact-name test listed once and passed
- [x] AC2: absent cursor fails closed with `unrecoverable_cursor` and operator guidance -> exact-name test listed once and passed; `UnrecoverableCursor` display text in `ref_reconciliation.rs` contains ``rerun `sce doctor --fix` ``
- [x] AC3: no hook boundary prunes or repairs -> `rg` returned no matches
- [x] AC4: no per-clone identity introduced -> `rg -n "checkout_id|clone_id" cli/src` returned no matches; migrations unchanged; `resolve_worktree_id` returns `main` for the primary worktree (`git_snapshot.rs:685-686`)
- [x] AC5: durable context describes the cases and the heuristic limit -> glossary `rg` returned no matches; `context/cli/mutation-trace-ref-reconciliation.md:141` states an existing tree is "not evidence of an unchanged clone"

### Failed checks and follow-ups

- None.

### Residual risks

- The reconciliation failure kind `checkout_identity` was renamed to `worktree_identity` (with `ReconcileError`/`ProtectedWorktreeError::CheckoutIdentity` -> `WorktreeIdentity`) after T01/T02 to clear AC4; consumers of the old failure-kind string must update.
- The `UnrecoverableCursor` message still says the cursor's durable history "does not belong to this checkout", which reads as a clone-identity claim the documented heuristic disclaims.
