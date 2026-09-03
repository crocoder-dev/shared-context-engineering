# Plan: mutation-scope-runtime-integration

## Change summary

Make the already-verified `protocol::abandon()` action reachable from production
runtime code, so a future harness adapter has a safe way to end a mutation scope
it can prove is stale but for which it never observed a trustworthy final
worktree boundary. Today `cli/src/services/mutation_trace/runtime/` exposes only
`coordinate()` — an *observed*-boundary path (`Start`/`Advance`/`Close`/`Flush`)
that always captures a Git snapshot. A dead agent process has no terminal
observation, so an adapter has no way to retire its scope. A dead execution can
therefore leave its scope `Active` indefinitely. This is unsafe: a later `Start`
observes the worktree before activating its successor — `commit` computes
`active_scopes`/`attribution` against the state as it existed *before* the same
call's own scope-lifecycle transition — so changes made after the dead execution
may be incorrectly classified as `AiExclusive` to the stale scope. Once another
scope starts, subsequent overlapping intervals also become `AiContended` against
the zombie.

This change adds a second runtime entrypoint, `abandon_scope()`, that shares
`coordinate()`'s safety prefix (worktree lock → external-taint fence → checkout
identity → DB) but deliberately takes **no Git snapshot**: abandonment means the
final mutation boundary was never observed, and snapshotting would silently give
it `Close`'s observation semantics. It extracts that shared prefix into one
internal `ProtectedWorktree` primitive so the two entrypoints cannot drift,
exposes the smallest read seam the new path needs from `MutationTraceStore`, and
records the mutation-scope lifecycle contract every later harness adapter (Codex,
Claude Code, OpenCode, Pi) must uphold.

This extends existing behavior and preserves it: `coordinate()`'s externally
observable ordering, error variants, and outcomes are unchanged, `Abandon` does
**not** become a `RuntimeBoundary` variant, and nothing here changes
`spec/mutation_cursor.qnt`, `protocol.rs` semantics, the mutation-trace SQL
schema, migrations, or `diff_traces`. No harness is wired by this plan.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: The `coordinate()` path runs through the shared protected-worktree
  primitive with its current externally observable ordering and error semantics
  intact: worktree lock → external marker inspect/persist → checkout identity →
  DB provider → snapshot/recovery/protocol/CAS → explicit marker clear, with
  `CoordinateError::LockAcquisition`, `ExternalTaintMarker { Inspect | Persist }`,
  `AgentTraceDbUnavailable`, and `MarkerClearAfterCommit { source, committed }`
  produced on exactly the same conditions as before.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::coordinator::` — the pre-existing fence, lock-contention, and `MarkerClearAfterCommit` tests pass unmodified in assertion content.
- [x] AC2: Abandoning an `Active` scope changes only that scope's status to
  `Abandoned`, advances its worktree `revision` by exactly one, sets
  `needs_rebaseline = true`, leaves `cursor_tree` / `tainted` / `failure_kind`
  unchanged, and writes no `mutation_trace_events`,
  `mutation_trace_event_active_scopes`, or `mutation_trace_processed_events` row.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::scope_runtime::` — a test reads the durable rows back after a successful abandonment and asserts each field and each absent row.
- [x] AC3: `abandon_scope()` performs no Git snapshot, tree pin, tree diff, ref
  reconciliation, scope registration, or worktree initialization; it reads and
  transitions only already-durable mutation-scope state.
  - Validate: inspect `cli/src/services/mutation_trace/runtime/scope_runtime.rs` — it names none of `GitSnapshotService`, `SnapshotCapture`, `capture_tree`, `pin_tree`, `diff_trees`, `reconcile_worktree`, `initialize_worktree`, or `register_scope`; confirm with `rg -n 'GitSnapshotService|SnapshotCapture|capture_tree|pin_tree|diff_trees|reconcile_worktree|initialize_worktree|register_scope' cli/src/services/mutation_trace/runtime/scope_runtime.rs` returning no non-test hit.
- [x] AC4: A target scope already `Closed` or `Abandoned` settles as a successful
  terminal no-op: no revision change, no row write, and the external-taint marker
  is cleared.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::scope_runtime::` — separate `Closed` and `Abandoned` tests assert the outcome variant, the unchanged durable revision, and that the marker file is gone.
- [x] AC5: A target `ScopeId` with no durable row, and one whose row is
  `NeverSeen`, both return the recovery-required outcome, leave the
  external-taint marker armed on disk, and commit no normal abandonment — no
  durable row changes, and the worktree revision does not advance. This
  deliberately forces the next `coordinate()` into conservative strong recovery
  (see **Design decisions**, D1).
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::scope_runtime::` — separate missing-scope and `NeverSeen` tests assert the reason variant, that `<git-dir>/sce/mutation-cursor-tainted` still exists, and that the worktree revision and every scope status are unchanged.
- [x] AC6: An external-taint marker already present when `abandon_scope()` is
  called returns the recovery-required outcome for that reason without clearing
  the marker and without invoking the caller-supplied DB provider.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::scope_runtime::` — a test arms the marker first, passes a provider that sets a flag and returns `Err`, and asserts the recovery-required reason, the still-present marker, and that the provider flag was never set.
- [x] AC7: A target scope whose durable `worktree_id` is not the `WorktreeId` this
  invocation derived from its own checkout is rejected as an error, and neither
  the scope row nor either worktree row is modified.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::` — a two-linked-worktree test abandons worktree A's scope through worktree B's checkout, asserts the error, and asserts both worktree revisions and the scope status are unchanged.
- [x] AC8: A CAS conflict makes `abandon_scope()` reload the durable projection and
  recompute the abandonment from that fresh state, bounded by the same retry limit
  the coordinator uses; when the competing writer left the scope terminal, the
  retry settles as the terminal no-op outcome rather than overwriting it.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::` — a real-thread CAS-race test against one on-disk DB asserts the settled outcome and that the scope's final status is the competitor's, not a second abandonment.
- [x] AC9: An `Active` target on a worktree at `revision: u64::MAX` produces an
  explicit revision-exhaustion error, never an abandonment success and never a
  terminal no-op.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::scope_runtime::` — a test seeds `u64::MAX` and asserts the distinct error variant.
- [x] AC10: With no inherited marker, a DB-provider `Err` or a persistence failure
  after the marker is armed leaves the marker on disk; a successful abandonment
  and a proven-terminal no-op each clear it; a `clear()` failure after either
  completes returns an error that carries the already-completed outcome rather
  than reporting the durable transition as failed.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::` — one test per row of that table, the clear-failure test asserting the carried outcome matches the durable state.
- [x] AC11: Against a real Git repository and a real repository-scoped Agent Trace
  DB: `Start(A)` → edit → `abandon_scope(A)` → a further unobserved edit →
  `coordinate(Start(B))` leaves the worktree cursor at the tree observed at
  `Start(B)`, emits no `MutationEvent` for the ambiguous A→B interval, leaves A
  `Abandoned`, and leaves B `Active`.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::tests::` — the end-to-end sequence test asserts the cursor tree, the absence of any `mutation_trace_events` row for the gap, and both scope statuses.
- [x] AC12: Abandoning stale scope A while unrelated scope B is legitimately
  `Active` on the same worktree leaves B `Active` through the subsequent
  `needs_rebaseline` recovery.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::tests::` — the surviving-scope test asserts B's status after the next `coordinate()` call.
- [x] AC13: `runtime/mod.rs` re-exports, at `pub(crate)`, exactly `coordinate`,
  `CoordinateError`, `CoordinateOutcome`, `ExternalTaintOperation`,
  `RuntimeBoundary`, `abandon_scope`, `AbandonScopeError`, and
  `AbandonScopeOutcome` (with its reason type). `ExternalTaintOperation` is part
  of `CoordinateError::ExternalTaintMarker`'s own public shape — a crate-visible
  `CoordinateError` a caller cannot match on is not a usable seam — so it must
  cross the boundary alongside it. Since T01 it lives in `protected_worktree.rs`
  and reaches the seam through `coordinator.rs`'s existing
  `pub use super::protected_worktree::ExternalTaintOperation`, so the type
  becomes crate-visible **without** `protected_worktree` becoming a public
  module. Every `mod` declaration in `runtime/mod.rs` stays private, and nothing
  else is re-exported from `git_snapshot`, `external_taint`, `worktree_lock`,
  `ref_reconciliation`, or `protected_worktree` — in particular
  `ProtectedWorktree`, `ProtectedWorktreeError`, and `WORKTREE_LOCK_TIMEOUT`
  remain internal to `runtime`.
  - Validate: inspect `cli/src/services/mutation_trace/runtime/mod.rs` for exactly those re-exports and confirm every `mod` declaration there is still private (`rg -n '^\s*(pub(\(crate\))?\s+)?mod |pub\(crate\) use' cli/src/services/mutation_trace/runtime/mod.rs`), and `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` stays clean.
- [x] AC14: `spec/mutation_cursor.qnt`, the Quint refinement matrix in
  `cli/src/services/mutation_trace/mod.rs`, `protocol.rs`'s transition semantics,
  `004_mutation_trace_protocol.sql`, and the migration set carry no change
  attributable to this PR. The baseline is this PR's own base branch, `ref-rec`,
  not `main`: the mutation-cursor protocol, the Quint model, the migration, and
  the runtime coordinator already exist on `ref-rec`, so a `main` baseline would
  report the entire stack below this PR as if it were this PR's change.
  - Validate: `git fetch origin` first, then `git diff --stat origin/ref-rec...HEAD -- spec/mutation_cursor.qnt cli/src/services/mutation_trace/protocol.rs cli/migrations/agent-trace-repository/` produces empty output. Use `origin/ref-rec`, not a local `ref-rec`, which can lag behind a rewritten base and would report the whole rebased stack as this PR's change. Additionally, the `mutation-trace-quint-connect` and Quint checks inside `nix flake check` stay green.
- [x] AC15: `context/cli/mutation-scope-runtime.md` exists and states, in the
  repository's own terms: a mutation scope is one independently mutation-capable
  execution (so concurrent main agent and subagent need distinct `ScopeId`s);
  `Start`/`Advance`/`Close` semantics including that a failed tool still requires
  `Advance` and that a `ScopeId` is never reused after a terminal status;
  `abandon_scope()` requires positive staleness evidence and must not be inferred
  from `ActorKind`; the `abandon` → `coordinate(Start(successor))` sequence and
  what each abandonment outcome implies for it; that abandonment is not a
  `RuntimeBoundary` and needs no Quint change; that a missing or `NeverSeen`
  target deliberately forces conservative strong recovery which may invalidate
  other live scopes on the worktree, with the reason that outranks the lost
  evidence (**Design decisions**, D1); and that `AiExclusive(scope)` means scope
  exclusivity, not standalone proof that no human edited the worktree.
  - Validate: read `context/cli/mutation-scope-runtime.md` and confirm each of those statements is present and consistent with the shipped code.

### Full validation

- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::`
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`
- `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`
- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/cli/mutation-scope-runtime.md` — new: the mutation-scope lifecycle and
  harness-adapter contract, including the attribution boundary.
- `context/cli/mutation-trace-runtime-coordinator.md` — the protected-worktree
  primitive, the two-entrypoint runtime surface, and the `pub(crate)` export seam.
- `context/cli/mutation-trace-external-taint.md` — the fence's abandonment-path
  completion semantics (clear on abandoned/terminal, stay armed on
  recovery-required, marker-clear-after-completion).
- `context/cli/mutation-trace-protocol.md` — that `protocol::abandon` now has a
  production call site, and that abandonment is not a `RuntimeBoundary`.
- `context/cli/mutation-trace-store.md` — the new bounded scope read seam.
- `context/context-map.md` and `context/overview.md` — index and status lines for
  the new module and context file.
- `context/patterns.md` — repair the recorded unit-testing pattern, which still
  describes unique `std::env::temp_dir()` paths as the mutation-trace fixture
  convention while `runtime/tests.rs` has moved to RAII `tempfile::TempDir`.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/mutation_trace/runtime/` (new
  `protected_worktree.rs` and `scope_runtime.rs`, edits to `coordinator.rs`,
  `mod.rs`, `tests.rs`), the read seam in
  `cli/src/services/mutation_trace/store.rs`, and the durable context files listed
  under **Context sync**.
- **Out of scope:** Codex, Claude Code, OpenCode, and Pi hook/plugin/extension
  wiring; each harness's concrete `ScopeId` / `EventId` format; harness-specific
  stale-process detection; adding `Abandon` to `RuntimeBoundary`; changes to
  `spec/mutation_cursor.qnt`, `protocol::abandon`, the mutation-trace SQL schema,
  or migrations; a new mutation-cursor table; `diff_traces` redesign;
  mutation-history retention; repository-scoped unowned checkout-identity ref
  cleanup; human-vs-AI authorship proof; a daemon or background liveness monitor;
  a `mutation_scope_adapter.qnt` model.
- **Constraints:**
  - The protected prefix ordering is safety-critical and must not move: worktree
    lock **before** external-marker inspect/persist **before** checkout identity,
    DB acquisition, and any runtime work. The guard never clears the marker in
    `Drop`; only an explicit successful completion clears it.
  - `abandon_scope()` must classify the target's durable state *before* invoking
    `protocol::abandon`. `abandon` is a guarded no-op for `NeverSeen`, `Closed`,
    `Abandoned`, an unknown scope, a missing `WorktreeState`, and
    `revision == u64::MAX` alike, and returns an unchanged state in every case —
    the runtime cannot recover the reason by diffing its output.
  - The bounded retry limit is the coordinator's existing
    `MAX_CAS_RETRY_ATTEMPTS` (5, no backoff); reuse it rather than introducing a
    second constant.
  - `#[allow(dead_code)]` on `pub mod mutation_trace` in
    `cli/src/services/mod.rs` covers unused items, not necessarily unused
    `pub(crate) use` re-exports. `clippy --all-targets -- -D warnings` is the
    gate; keep it green using the module's existing allowance precedent rather
    than by adding a placeholder consumer.
  - Filesystem-touching tests follow the repository's Nix-sandbox-safe inline
    `#[cfg(test)] mod tests` convention with RAII `tempfile::TempDir` fixtures, as
    `runtime/tests.rs` already does.
- **Non-goal:** a general-purpose "runtime operation" abstraction over
  `coordinate()` and `abandon_scope()`. The two paths deliberately differ (one
  observes, one does not); the shared piece is the protected prefix only.

## Design decisions

Decided before implementation. Do not reopen these during T01–T05; a change of
mind is a new plan revision, not a task-time judgement call.

### D1: A missing or `NeverSeen` target forces conservative strong recovery, and that may invalidate other live scopes

`abandon_scope()` on a `ScopeId` with no durable row, or one whose row is still
`NeverSeen`, returns the recovery-required outcome and leaves the external-taint
marker armed. The next `coordinate()` on that worktree therefore performs
*inherited-taint* recovery, and `protocol::recover` abandons **every** live scope
on a worktree recovering from external taint — not only the scope the adapter
named.

That consequence is accepted deliberately, not overlooked. The reasoning:

```text
execution lifecycle not durably observed
        ↓
filesystem interval may contain unknown mutations
        ↓
cannot safely preserve exclusive attribution assumptions
        ↓
force conservative recovery
```

A missing row means the scope's `Start` never committed while the execution may
well have run and edited files; a `NeverSeen` row means the identity exists but
no accepted `Start` was ever observed for it. Neither proves the execution
mutated nothing. The runtime cannot bound what happened inside that interval, so
it cannot let any scope keep an exclusivity claim that spans it.

**The tradeoff, stated plainly:** this is a false-negative cost. Legitimately
live mutation scopes on the same worktree can be abandoned by a recovery they did
nothing to cause, and the evidence for their in-flight intervals is discarded.
That cost is acceptable because the alternative is a false positive — attributing
an interval exclusively to a scope while an unobserved execution may have been
mutating the same worktree. Preserving attribution safety outranks preserving
potentially valid evidence.

This is *not* in tension with AC12. AC12 covers the `Abandoned` outcome, whose
`needs_rebaseline`-only recovery preserves live scopes by design. D1 covers the
recovery-required outcomes, where the stronger external-taint recovery is the
whole point.

### D2: The DB is never consulted before the external-taint fence is armed

The protected ordering stays exactly as `coordinate()` already has it:

```text
WorktreeLock
    ↓
inspect / persist external-taint marker
    ↓
checkout identity
    ↓
DB
    ↓
runtime operation
```

The alternative of resolving the target scope first, so an unresolvable
`ScopeId` could be rejected without arming the fence —

```text
lock
  → DB lookup
  → maybe arm marker
```

— is **rejected**. Any failure between reading the DB and establishing the fence
(process death, `SIGKILL`, an I/O error, a panic) reopens exactly the uncertainty
window the external-taint marker exists to close: the invocation would have
touched durable state, or decided something about it, with no worktree-local
signal left behind for the next invocation. The fence must be armed write-ahead
of every fallible step that follows it, including the scope lookup that decides
D1's outcome.

The one lookup-order concession already in the plan is the inherited-marker
short-circuit: when a marker was *already* present on entry, `abandon_scope()`
returns recovery-required without invoking the DB provider at all (AC6). That
does not weaken the fence — the fence is already armed by an earlier invocation,
which is precisely why there is nothing left for this one to decide.

## Assumptions

- Module and type names follow the change request's suggestions
  (`runtime/protected_worktree.rs`, `runtime/scope_runtime.rs`,
  `ProtectedWorktree::acquire`, `abandon_scope`, `AbandonScopeOutcome`,
  `AbandonScopeError`); adjust to whatever reads best beside the existing
  `coordinator.rs` naming, since only the semantic distinctions are contractual.
- `abandon_scope()` takes the same caller-supplied
  `open_db: impl FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>` provider
  shape as `coordinate()`, so DB acquisition falls inside the same fence.
- The recovery-required reasons are modelled as a distinct enum
  (`InheritedExternalTaint`, `MissingScope`, `NeverSeenScope`,
  `MissingWorktreeState`) carried in the outcome, so callers and tests can match
  on them.
- `abandon_scope()` goes through the same protected-worktree acquisition path as
  `coordinate()` and therefore uses the `WORKTREE_LOCK_TIMEOUT` owned by
  `runtime/protected_worktree.rs` (T01 moved the constant there with the prefix);
  it must not declare a second mutation-scope lock timeout. `ref_reconciliation`
  keeps its separately owned `RECONCILIATION_LOCK_TIMEOUT` — that remains
  intentional, matching by value but not by ownership, since a reconciliation
  pass is an operation that genuinely differs.

## Task stack

- [x] T01: `Extract the protected-worktree runtime guard` (status:done)
  - Task ID: T01
  - Scope: In — new `cli/src/services/mutation_trace/runtime/protected_worktree.rs` owning git-dir resolution, `WorktreeLock` acquisition, external-marker inspect + persist, checkout identity, `WorktreeId` derivation, and an explicit `complete`/`clear` step; refactor `coordinator.rs`'s `coordinate_inner` / `coordinate_protected` prefix onto it; keep the `on_lock_contention` test seam working. Out — any new entrypoint, any store change, any behavior change to the pipeline below the prefix, any `runtime/mod.rs` export change.
  - Dependencies: none
  - Done when: the guard exposes the derived `WorktreeId`, whether a marker was already present before this invocation, and an explicit completion that clears the marker; it never clears the marker in `Drop`; it holds the `WorktreeLock` for its own lifetime; `coordinate()` produces identical outcomes and identical `CoordinateError` variants on every existing path, with the existing coordinator fence/lock tests passing without assertion changes; focused tests cover the guard itself for lock-timeout failure, an inherited marker being reported, a fresh marker being armed, and the marker surviving a dropped guard that was never completed.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::`; `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`; `git diff` on `coordinator.rs` shows no reordering of lock → marker → checkout → DB.
  - Completed: 2026-09-03
  - Files changed:
    - `cli/src/services/mutation_trace/runtime/protected_worktree.rs` (new)
    - `cli/src/services/mutation_trace/runtime/mod.rs`
    - `cli/src/services/mutation_trace/runtime/coordinator.rs`
    - `cli/src/services/mutation_trace/runtime/ref_reconciliation.rs`
  - Result: `protected_worktree.rs` now owns the safety prefix as `ProtectedWorktree`,
    running resolve `git_dir` → `WorktreeLock` → marker inspect → marker persist →
    checkout identity → `WorktreeId` in the coordinator's existing order. It exposes
    `worktree_id()`, `inherited_external_taint()`, and a consuming `complete()` that
    clears the marker while the lock is still held; `Drop` releases only the lock and
    never clears the marker. `acquire` uses the relocated `WORKTREE_LOCK_TIMEOUT`
    (10s, value unchanged), `pub(super) acquire_inner` carries the `on_lock_contention`
    seam, and a private `acquire_with_timeout` serves the guard's own timeout test.
    `coordinate_inner` now acquires the guard and maps `ProtectedWorktreeError` onto
    the pre-existing `CoordinateError` variants (git-dir resolution and checkout
    identity → `Other`, lock → `LockAcquisition`, fence → `ExternalTaintMarker` with
    the same operation); `coordinate_protected` lost its `git_dir` parameter and takes
    the guard's `&WorktreeId`. `ExternalTaintOperation` moved to `protected_worktree.rs`
    and is `pub use`-re-exported from `coordinator.rs`, so `CoordinateError`'s shape is
    unchanged. No coordinator test assertion was modified — only test-module imports
    were added. Per an explicit user instruction during implementation, every comment
    this task wrote or touched was then removed from the code, including
    `ref_reconciliation.rs`'s pre-existing `RECONCILIATION_LOCK_TIMEOUT` doc comment
    (which had named `WORKTREE_LOCK_TIMEOUT`'s old home); the rationale it carried is
    preserved in `context/cli/mutation-trace-ref-reconciliation.md`.
  - Verify results:
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::` — passed: 96 passed, 0 failed (91 pre-existing plus the 5 new guard tests).
    - `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — passed, clean.
    - `git diff` on `coordinator.rs` — confirmed: the prefix steps moved verbatim, lock → marker inspect → marker persist → checkout identity → `open_db` order intact, no reordering.
    - Also run: `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check` — passed after formatting the new file.
  - Context impact: local to `cli/src/services/mutation_trace/runtime/`. No public
    interface, schema, migration, spec, or protocol change; `coordinate()`'s signature,
    outcomes, and error variants are unchanged. Durable context affected:
    `context/cli/mutation-trace-runtime-coordinator.md` (the prefix is now the
    `ProtectedWorktree` primitive rather than inline coordinator code) and
    `context/cli/mutation-trace-external-taint.md` (the fence's arm/clear ownership
    moved to that guard, whose `Drop` never clears).
  - Context synchronization: synced

- [x] T02: `Expose a bounded scope read on MutationTraceStore` (status:done)
  - Task ID: T02
  - Scope: In — promote the existing private `MutationTraceStore::load_scope` to the smallest public read seam `scope_runtime` needs (one `mutation_trace_scopes` row → `Option<ScopeState>`), with a doc comment stating it is a cold-path read that never widens into a projection; tests for an existing scope, a missing scope, and a scope belonging to another worktree. Out — any change to `load_worktree`'s hook-boundary semantics or error contract, any schema/migration change, any new query, any write path.
  - Dependencies: none
  - Done when: the read returns the durable `ScopeState` (status, `actor_kind`, `worktree_id`) for a known `ScopeId` and `None` for an unknown one, without consulting `mutation_trace_events` or the worktree row; `load_worktree`'s existing behavior, including its `Err` on a mismatched or missing effective referenced scope, is untouched; the mismatched-worktree case is proven to be the caller's decision, not a store-level rejection.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::store::`; `git diff cli/migrations/` is empty.
  - Completed: 2026-09-03
  - Files changed:
    - `cli/src/services/mutation_trace/store.rs`
  - Result: `MutationTraceStore::load_scope` is now `pub`, relocated from the private
    helper block to sit beside the other public reads (after `load_all_tree_roots`,
    before `load_worktree_state`). Its signature and body are byte-identical —
    `pub fn load_scope(&self, scope_id: &ScopeId) -> Result<Option<ScopeState>>`,
    one `SELECT_SCOPE_BY_ID_SQL` `query_map` through `scope_row_from_turso` — so
    both existing internal callers (`register_scope`, `load_worktree`) are
    unaffected and no new query was added. The new doc comment states that it is a
    cold-path single-row read that reads one `mutation_trace_scopes` row and
    nothing else, never consults `mutation_trace_events`,
    `mutation_trace_processed_events`, or the scope's `mutation_trace_worktrees`
    row, must not widen into a projection (naming `load_worktree` as the projection
    seam), and never adjudicates worktree identity — a scope on another worktree is
    returned as-is because comparing the two is the caller's decision. Three tests
    were added to the existing inline `#[cfg(test)] mod tests`, using its
    `test_db_path` / `insert_worktree` / `insert_scope` fixtures:
    `load_scope_returns_the_durable_state_for_a_known_scope` (seeds an event row,
    an active-scope row, and a processed-event row alongside the scope, then
    asserts the exact `ScopeState`, proving those tables are not consulted),
    `load_scope_returns_none_for_an_unknown_scope`, and
    `load_scope_returns_a_scope_belonging_to_another_worktree` (a scope on `wt-2`
    with no `wt-2` worktree row returns `Ok(Some(..))` carrying `wt-2`, while the
    same scope through `load_worktree(wt-1, ..)` still errors — proving the
    mismatch is the caller's decision and that `load_worktree`'s contract is
    untouched). No schema, migration, or write path was touched.
  - Verify results:
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::store::` — passed: 86 passed, 0 failed (83 pre-existing plus the 3 new tests); the three new tests were also run in isolation via the `...::tests::load_scope` filter and all passed.
    - `git diff cli/migrations/` — empty, confirmed.
    - Also run: `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — passed, clean, with no placeholder consumer added for the newly public method.
    - Also run: `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check` — passed.
  - Context impact: local to `cli/src/services/mutation_trace/store.rs`. One method
    widened from private to `pub` on `MutationTraceStore`; no signature, schema,
    migration, spec, protocol, or behavior change, and `load_worktree`'s error
    contract is untouched. Durable context affected:
    `context/cli/mutation-trace-store.md` (the new bounded scope read seam and its
    deliberate non-adjudication of worktree identity).
  - Context synchronization: synced

- [x] T03: `Implement abandon_scope() on the protected runtime path` (status:done)
  - Task ID: T03
  - Scope: In — new `cli/src/services/mutation_trace/runtime/scope_runtime.rs` with `abandon_scope`, `AbandonScopeOutcome`, its recovery-reason enum, and `AbandonScopeError`, built on T01's guard and T02's read; inline `#[cfg(test)] mod tests` against a real temp-file `RepositoryAgentTraceDb` covering active abandonment, `Closed`/`Abandoned` terminal no-op, missing and `NeverSeen` recovery-required, inherited-marker short-circuit before the DB provider, worktree-identity rejection, revision exhaustion, CAS reload/recompute/retry, DB-provider failure, and marker-clear-after-completion. Out — any Git snapshot, pin, diff, reconciliation, scope registration, or worktree initialization; any `runtime/mod.rs` export; any real-Git cross-worktree test (T04).
  - Dependencies: T01, T02
  - Done when: an inherited marker returns recovery-required for that reason without invoking `open_db` and without clearing the marker; a missing or `NeverSeen` scope returns recovery-required, writes nothing, and leaves the marker armed, per **Design decisions** D1 — the scope lookup happens after the fence is armed, never before it (D2); a `Closed`/`Abandoned` scope returns the terminal no-op with the current revision and clears the marker; an `Active` scope belonging to another `WorktreeId` is an error that writes nothing; an `Active` scope on a worktree at `u64::MAX` returns a distinct revision-exhaustion error; otherwise the durable transition sets the scope `Abandoned`, advances revision by exactly one, sets `needs_rebaseline`, writes no event or processed-event row, and clears the marker; a `CasResult::Conflict` reloads and recomputes from fresh state within `MAX_CAS_RETRY_ATTEMPTS`, settling as the terminal no-op when a competitor won; a failing `clear()` after a completed abandonment or terminal no-op returns an error carrying that completed outcome; the module names no Git-snapshot or reconciliation symbol.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::scope_runtime::`; `rg -n 'GitSnapshotService|SnapshotCapture|capture_tree|pin_tree|diff_trees|reconcile_worktree|initialize_worktree|register_scope' cli/src/services/mutation_trace/runtime/scope_runtime.rs` returns nothing; `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`.
  - Completed: 2026-09-03
  - Files changed:
    - `cli/src/services/mutation_trace/runtime/scope_runtime.rs` (new)
    - `cli/src/services/mutation_trace/runtime/mod.rs`
    - `cli/src/services/mutation_trace/runtime/coordinator.rs`
  - Result: `scope_runtime.rs` adds the runtime's second protected entrypoint,
    `abandon_scope(repository_root, &ScopeId, open_db)`, taking the same
    caller-supplied DB-provider shape as `coordinate()`. It acquires T01's
    `ProtectedWorktree` (so the prefix ordering, the `WORKTREE_LOCK_TIMEOUT`, and
    the fence's arm/clear ownership are shared, not duplicated) and captures no Git
    snapshot; the module names none of `GitSnapshotService`, `SnapshotCapture`,
    `capture_tree`, `pin_tree`, `diff_trees`, `reconcile_worktree`,
    `initialize_worktree`, or `register_scope`. An inherited marker short-circuits
    to `RecoveryRequired { InheritedExternalTaint }` before `open_db` is called
    (AC6, D2's stated concession); every other decision is made after the fence is
    armed. Inside the fence, a bounded loop over the coordinator's existing
    `MAX_CAS_RETRY_ATTEMPTS` runs T02's `load_scope` first — because the projection
    seam treats both of that read's cases as errors and neither is one: a missing
    row is D1's `MissingScope` recovery, and a foreign `worktree_id` is the typed
    `WorktreeIdentityMismatch` rejection — then loads
    `load_worktree(worktree, Some(scope), None)` (`None` -> `MissingWorktreeState`)
    and classifies from that fresh projection: `NeverSeen` -> recovery-required,
    `Closed`/`Abandoned` -> `AlreadyTerminal` with the current revision, `Active` ->
    `protocol::abandon` + `DurableTransition::between` + `store.commit`. A `None`
    transition on a proven-live scope in a projection whose `external_taint` is
    always empty can only mean an unadvanceable revision, so it maps to
    `RevisionExhausted`; `CasResult::Conflict` re-enters the loop and re-classifies
    from scratch, so a competitor that closed the scope settles as `AlreadyTerminal`
    rather than being overwritten. The marker is cleared only for `Abandoned` and
    `AlreadyTerminal`; every `RecoveryRequired` and every error leaves it armed, and
    a failing `clear()` returns `MarkerClearAfterCompletion { source, completed }`
    carrying the already-settled outcome. Two supporting edits: `runtime/mod.rs`
    gained the private `mod scope_runtime;` declaration (no `pub(crate) use` — that
    remains T05's), and `coordinator.rs`'s `MAX_CAS_RETRY_ATTEMPTS` widened from
    private to `pub(super)` so the retry limit is reused rather than redeclared, as
    the plan's constraints require. A private `abandon_scope_inner` carries an
    `after_load` seam (mirroring `coordinate_inner`) for the CAS tests. The inline
    `#[cfg(test)] mod tests` uses an RAII `tempfile::TempDir` fixture holding a real
    `git init` repository and a real temp-file `RepositoryAgentTraceDb`, with 14
    tests covering every `Done when` clause. A post-review amendment added the
    14th, `a_persistence_failure_rolls_back_the_whole_transition_and_leaves_the_fence_armed`,
    closing AC10's remaining row: it drives the real `MutationTraceStore::commit`
    path and, through the `after_load` seam, creates a `UNIQUE` index on
    `mutation_trace_scopes(status)` that the seeded rows already satisfy and only
    the `abandoned` status the transition is about to write violates. The worktree
    row is untouched, so the transaction's CAS guard still matches and applies,
    and the later scope-status statement then fails — proving the batch rolls the
    guard back rather than leaving a partially updated worktree. No store or
    runtime code was changed for it. Per an explicit user instruction during this
    task, every comment this task wrote was then removed from
    `scope_runtime.rs`; the rationale they carried is preserved in
    `context/cli/mutation-trace-scope-abandonment.md`.
  - Verify results:
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::scope_runtime::` — passed: 14 passed, 0 failed.
    - `rg -n 'GitSnapshotService|SnapshotCapture|capture_tree|pin_tree|diff_trees|reconcile_worktree|initialize_worktree|register_scope' cli/src/services/mutation_trace/runtime/scope_runtime.rs` — no match (exit 1), run as `nix run nixpkgs#ripgrep --` per the repository's bash-tool policy.
    - `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — passed, clean, with no placeholder consumer added; the pre-existing `#[allow(dead_code)] pub mod mutation_trace;` in `cli/src/services/mod.rs` covers the not-yet-exported items.
    - Also run: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::` — passed: 284 passed, 0 failed, confirming the coordinator, runtime, store, protocol, and MBT suites are unaffected.
    - Also run: `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check` — passed after formatting the new file.
  - Context impact: local to `cli/src/services/mutation_trace/runtime/`. No public
    interface, schema, migration, spec, or protocol change; `coordinate()`'s
    signature, outcomes, and error variants are unchanged, `protocol::abandon` was
    not touched, and nothing new is reachable outside `runtime` yet. Durable context
    affected: `context/cli/mutation-trace-runtime-coordinator.md` (the runtime now
    has two entrypoints over one protected prefix, and `MAX_CAS_RETRY_ATTEMPTS` is
    the shared retry limit for both), `context/cli/mutation-trace-external-taint.md`
    (the fence's abandonment-path completion semantics: cleared on
    abandoned/terminal, left armed on recovery-required and on every error), and
    `context/cli/mutation-trace-protocol.md` (`protocol::abandon` now has a
    production call site, and abandonment is not a `RuntimeBoundary`).
  - Context synchronization: synced

- [x] T04: `Add cross-runtime abandonment safety regressions` (status:done)
  - Task ID: T04
  - Scope: In — integration tests in `cli/src/services/mutation_trace/runtime/tests.rs` driving `coordinate()` and `abandon_scope()` together against real `git init` / `git worktree add` repositories and a real repository-scoped Agent Trace DB: the abandon → unobserved edit → successor `Start` rebaseline sequence with no evidence for the gap; a concurrently active unrelated scope surviving that recovery; abandoning a scope through the wrong checkout; and a real-thread CAS race between `abandon_scope()` and a competing writer. Out — the single-module cases T03 already covers with a temp-file DB and no real Git.
  - Dependencies: T03
  - Done when: `Start(A)` → edit → `abandon_scope(A)` → unobserved edit → `coordinate(Start(B))` leaves the cursor at the tree observed at `Start(B)`, emits no `mutation_trace_events` row for the A→B interval, leaves A `Abandoned` and B `Active`; a second scope B active across `abandon_scope(A)` is still `Active` after the next `coordinate()`; abandoning worktree A's scope through worktree B's checkout errors and changes no row in either worktree; the CAS-race test settles deterministically on the competitor's terminal status without a second abandonment.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::tests::`; `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::`.
  - Completed: 2026-09-03
  - Files changed:
    - `cli/src/services/mutation_trace/runtime/tests.rs`
    - `cli/src/services/mutation_trace/runtime/scope_runtime.rs`
  - Result: `runtime/tests.rs` gained four public-entrypoint integration tests that
    drive `coordinate()` and `abandon_scope()` together over real `git init` /
    `git worktree add` repositories and a real repository-scoped Agent Trace DB,
    reusing the file's existing RAII `TestRepo` / `LinkedTestRepo` fixtures.
    `an_abandoned_scope_rebaselines_the_successor_start_without_evidence_for_the_gap`
    runs baseline `Flush` → `Start(A)` → a real edit → `abandon_scope(A)` → a second
    unobserved edit → `coordinate(Start(B))`, asserting the abandonment advanced the
    revision by exactly one, that `Start(B)` observed a tree different from the
    baseline, that the durable cursor sits at the tree observed at `Start(B)` with
    `needs_rebaseline`/`tainted` cleared and `failure_kind` healthy, that A is
    `Abandoned` and B `Active`, and that the whole gap carries no evidence — both
    `mutation_trace_events` being empty and `load_mutation_event` being `None` for
    every revision from `Start(A)` through `Start(B)`.
    `abandoning_a_stale_scope_leaves_an_unrelated_live_scope_active_through_the_recovery`
    starts two scopes with different `ActorKind`s, abandons only the stale one, then
    drives a real `Advance` on the live one; the live scope is still `Active`, the
    stale one `Abandoned`, and the `needs_rebaseline` recovery consumes the ambiguous
    interval rather than attributing it — the AC12 counterpart to D1's stronger
    external-taint recovery.
    `abandoning_a_scope_through_another_worktrees_checkout_is_rejected_without_writing`
    materializes two linked worktrees over one shared DB, starts a scope on the main
    checkout, and abandons it through the linked checkout: the error is
    `WorktreeIdentityMismatch` carrying both `WorktreeId`s, neither worktree's
    revision moved, the scope is still `Active`, and the fence is armed only on the
    invoking (linked) worktree, not on the target's.
    `a_real_thread_cas_race_settles_on_the_competitors_terminal_status` runs a genuine
    OS thread with its own DB handle against the same on-disk DB. Released through
    `abandon_scope_inner`'s `after_load` seam while the abandonment sits between its
    load and its commit, the competitor performs a real store-level `Close` —
    `load_worktree` → `protocol::prepare` → `protocol::commit` →
    `DurableTransition::between` → `MutationTraceStore::commit`, asserting
    `CasResult::Applied` — and reports its committed revision back over a channel. The
    competitor writes through the store rather than through `coordinate()` on purpose:
    `abandon_scope()` holds the worktree lock for its whole body, so a competitor
    taking the same lock would serialize instead of racing. The abandonment then loses
    its CAS, reloads, re-classifies the now-`Closed` scope, and settles as
    `AlreadyTerminal { status: Closed, revision: <competitor's> }`; the durable scope
    status stays `Closed` and the revision stays at the competitor's, proving no second
    abandonment was written. One supporting edit outside the test file:
    `scope_runtime.rs`'s private `abandon_scope_inner` widened to `pub(super)` so the
    sibling `tests` module can reach its `after_load` seam, exactly as
    `coordinator.rs` already exposes `pub(super) fn coordinate_inner` for the same
    reason. No behavior, signature, or runtime logic changed. The plan's **Open
    questions** allowed dropping the CAS race from T04 if reuse required exporting a
    test helper across modules; no helper was exported, so AC8's real-thread
    requirement was implemented rather than dropped. Consistent with T01 and T03, no
    comments were added to the code.
  - Verify results:
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::tests::` — passed: 27 passed, 0 failed (23 pre-existing plus the 4 new integration tests).
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::` — passed: 288 passed, 0 failed (284 before this task plus the 4 new tests), confirming the coordinator, scope-runtime, store, protocol, and MBT suites are unaffected.
    - Also run: `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — passed, clean; the CAS-race test carries `#[allow(clippy::too_many_lines)]`, matching the file's existing precedent for long integration tests.
    - Also run: `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check` — passed after `cargo fmt`.
  - Context impact: local to `cli/src/services/mutation_trace/runtime/`. Test-only
    except for one visibility widening (`abandon_scope_inner` private ->
    `pub(super)`), which is a sibling-module test seam, not a crate-visible surface.
    No public interface, schema, migration, spec, or protocol change; `coordinate()`
    and `abandon_scope()` behavior is untouched and nothing new is reachable outside
    `runtime`. Durable context affected:
    `context/cli/mutation-trace-runtime-coordinator.md` (the two entrypoints are now
    covered by cross-runtime integration regressions, and the `pub(super)`
    `*_inner` test seam is the recorded convention for both) and
    `context/cli/mutation-trace-scope-abandonment.md` (the abandon →
    successor-`Start` rebaseline sequence, the live-scope survival guarantee, the
    cross-checkout rejection, and why a CAS competitor must bypass the worktree lock
    to race at all). `context/patterns.md` is the plan's recorded repair target for
    the mutation-trace unit-testing fixture convention, which these tests continue
    to follow via RAII `tempfile::TempDir`.
  - Context synchronization: synced

- [x] T05: `Export the runtime seam and record the adapter contract` (status:done)
  - Task ID: T05
  - Scope: In — `pub(crate) use` re-exports in `runtime/mod.rs` for `coordinate`, `CoordinateError`, `CoordinateOutcome`, `ExternalTaintOperation`, `RuntimeBoundary`, `abandon_scope`, `AbandonScopeError`, `AbandonScopeOutcome` and its reason type — conceptually `pub(crate) use coordinator::{coordinate, CoordinateError, CoordinateOutcome, ExternalTaintOperation, RuntimeBoundary};` plus the `scope_runtime` names, `ExternalTaintOperation` riding through `coordinator`'s own `pub use` of it because `CoordinateError::ExternalTaintMarker` carries it; keeping the `git_snapshot`, `external_taint`, `worktree_lock`, `ref_reconciliation`, and `protected_worktree` **modules** private and re-exporting nothing else from any of them (`ProtectedWorktree`, `ProtectedWorktreeError`, and `WORKTREE_LOCK_TIMEOUT` stay internal); new `context/cli/mutation-scope-runtime.md` plus its `context/context-map.md` and `context/overview.md` index entries. Out — any harness, hook, or command wiring; any change to the runtime implementation, including moving `ExternalTaintOperation` or `WORKTREE_LOCK_TIMEOUT` back out of `protected_worktree.rs`; the per-task context updates T01–T04 each own for their own domain files.
  - Dependencies: T04
  - Done when: the eight names above are reachable as `crate::services::mutation_trace::runtime::*`, including `ExternalTaintOperation` so a crate-level caller can match `CoordinateError::ExternalTaintMarker`; the five modules remain private and nothing else from them is reachable; `clippy --all-targets -- -D warnings` is clean with no placeholder consumer added to satisfy it; `context/cli/mutation-scope-runtime.md` states the scope-identity rule, the `Start`/`Advance`/`Close` semantics, the positive-evidence requirement for `abandon_scope()` and the prohibition on inferring staleness from `ActorKind`, the successor-scope sequence and what each outcome implies for it (including that a failed abandonment must not be treated as a safely started successor), that abandonment is not a `RuntimeBoundary` and requires no Quint change, the D1 strong-recovery tradeoff for a missing or `NeverSeen` target, and the `AiExclusive` attribution boundary; `context-map.md` and `overview.md` name the new module and file.
  - Verify: `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`; `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`; read `context/cli/mutation-scope-runtime.md` against the shipped `scope_runtime.rs` signatures.
  - Completed: 2026-09-03
  - Files changed:
    - `cli/src/services/mutation_trace/runtime/mod.rs`
    - `context/cli/mutation-scope-runtime.md` (new)
    - `context/context-map.md`
    - `context/overview.md`
  - Result: `runtime/mod.rs` gained two `#[allow(unused_imports)] pub(crate) use`
    blocks — `coordinator::{coordinate, CoordinateError, CoordinateOutcome,
    ExternalTaintOperation, RuntimeBoundary}` and `scope_runtime::{abandon_scope,
    AbandonRecoveryReason, AbandonScopeError, AbandonScopeOutcome}` — nine names
    total (the plan's "eight names ... and its reason type" is `AbandonRecoveryReason`
    counted separately, matching the file's own header count). Every `mod`
    declaration in the file stays private; nothing else is re-exported. The
    `#[allow(unused_imports)]` on each `use` block follows the repository's
    existing precedent for a seam with no consumer yet (`services/style.rs`,
    `services/hooks/codex/apply_patch/mod.rs`) rather than the module-level
    `#[allow(dead_code)]` on `pub mod mutation_trace`, which covers unused items,
    not unused re-exports, and rather than a placeholder consumer, which the
    plan's constraints forbid. Reachability was proven by a temporary crate-level
    test module importing and using all nine names through
    `crate::services::mutation_trace::runtime::*` (compiled and passed), and
    module privacy was proven by a second temporary test module attempting direct
    imports of `protected_worktree`, `worktree_lock`, `git_snapshot`,
    `external_taint`, and `ref_reconciliation`, each failing with `E0603: module
    ... is private`; both probes were reverted before this task's own edit, and
    `git status`/`git diff` on `cli/src/services/mod.rs` confirm it carries no
    diff from this task. New `context/cli/mutation-scope-runtime.md` records the
    adapter contract: the scope-identity rule (one independently mutation-capable
    execution per `ScopeId`, so a concurrent main agent and subagent need distinct
    ids); `Start`/`Advance`/`Close`/`Flush` semantics including that a failed tool
    still requires `Advance` (a boundary observes the worktree, not a successful
    edit) and that a `ScopeId` is never reused after a terminal status (the
    `NeverSeen` guard silently refuses to reactivate it); that `coordinate()`
    registers `(worktree_id, actor_kind)` identity on every scope-carrying
    variant, not only `Start`, with a mismatch surfacing as
    `CoordinateError::ScopeIdentityConflict`; the positive-evidence requirement
    for `abandon_scope()` and the explicit prohibition on inferring staleness
    from `ActorKind`; the abandon → `coordinate(Start(successor))` sequence as an
    outcome table, including that a failed abandonment (an `Err`, `Abandoned`
    excluded) must never be treated as a safely started successor; that
    abandonment is not a `RuntimeBoundary` and needs no Quint change; D1's
    strong-recovery tradeoff for a missing or `NeverSeen` target verbatim from the
    plan's **Design decisions**; and the `AiExclusive` attribution boundary — that
    it states scope exclusivity only, never standalone proof no human edited the
    worktree. `context-map.md` gained one index entry in file order after
    `mutation-trace-scope-abandonment.md` and before
    `mutation-trace-ref-reconciliation.md`; `overview.md`'s existing
    mutation-trace status paragraph gained a sentence naming the new
    `abandon_scope()` entrypoint, the nine re-exports, and the new context file,
    replacing its prior "the module is still not wired into any hook or command"
    citation list with the expanded one including the two new files. A pre-write
    cross-check against the shipped `scope_runtime.rs`/`coordinator.rs` found and
    corrected two drafting errors before this record was written: `Flush` carries
    no fields (not a `worktree` field — corrected from an earlier draft that
    stated `Flush { worktree }`), and `coordinate()`'s `hook_identity` /
    `register_scope` call runs for all three scope-carrying variants, not only
    `Start`. No production runtime logic, signature, or behavior changed; no code
    comments were added, consistent with T01/T03/T04's precedent on this plan.
  - Verify results:
    - `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` — passed, clean.
    - `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` — passed: 951 passed, 0 failed, 0 ignored.
    - `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check` — passed.
    - `rg -n '^\s*(pub(\(crate\))?\s+)?mod |pub\(crate\) use' cli/src/services/mutation_trace/runtime/mod.rs` — confirmed every `mod` line has no `pub`/`pub(crate)` prefix and exactly two `pub(crate) use` blocks are present.
    - Read `context/cli/mutation-scope-runtime.md` against the shipped `scope_runtime.rs`/`coordinator.rs` signatures — confirmed, after the two corrections above.
  - Context impact: this task's own deliverable *is* the context change — a new
    root-adjacent domain file plus its two index entries. No production code
    behavior changed beyond the re-export visibility itself (a compile-time-only
    seam widening); `coordinate()`'s and `abandon_scope()`'s signatures, outcomes,
    and error variants are unchanged. Durable context affected:
    `context/cli/mutation-scope-runtime.md` (new), `context/context-map.md`,
    `context/overview.md`. Two sibling domain files this task's own change made
    stale were also corrected during synchronization:
    `context/cli/mutation-trace-runtime-coordinator.md` and
    `context/cli/mutation-trace-scope-abandonment.md` (both previously described
    the `pub(crate)` re-export as future work).
  - Context synchronization: synced

## Open questions

- T04's CAS-race regression is the one test in this plan that needs real OS
  threads against one on-disk DB. `coordinator.rs` already has that machinery for
  its own CAS tests. If reusing it means exporting a test helper across modules,
  the cheaper option is to leave the abandonment CAS race in `scope_runtime.rs`'s
  own inline tests (T03) and drop it from T04, since nothing about the race needs
  real Git. Not blocking — T03's `Done when` already covers the behavior.

## Validation Report

**Status:** validated  
**Date:** 2026-09-03

### Commands run

- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::` -> exit 0 (288 passed; 0 failed; 663 filtered out)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` -> exit 0 (951 passed; 0 failed; 0 ignored)
- `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` -> exit 0 (clean, no placeholder consumer)
- `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check` -> exit 0 (clean)
- `nix flake check` -> exit 0 (all checks passed, incl. `sce-cli-clippy`, `sce-cli-fmt`, `sce-cli-tests`, `sce-mutation-trace-quint-connect`)
- `nix run .#pkl-check-generated` -> exit 0 (141 files, inventory sha256 5516770e…)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::scope_runtime::` -> exit 0 (14 passed)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::coordinator::` -> exit 0 (26 passed, assertions unmodified)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::tests::` -> exit 0 (27 passed)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::store::` -> exit 0 (86 passed)
- `rg -n 'GitSnapshotService|SnapshotCapture|capture_tree|pin_tree|diff_trees|reconcile_worktree|initialize_worktree|register_scope' cli/src/services/mutation_trace/runtime/scope_runtime.rs` -> exit 1 (no match)
- `rg -n '^\s*(pub(\(crate\))?\s+)?mod |pub\(crate\) use' cli/src/services/mutation_trace/runtime/mod.rs` -> exit 0 (every `mod` private; two `pub(crate) use` blocks)
- `git fetch origin && git diff --stat origin/ref-rec...HEAD -- spec/mutation_cursor.qnt cli/src/services/mutation_trace/protocol.rs cli/migrations/agent-trace-repository/` -> exit 0 (empty output; also empty for `cli/src/services/mutation_trace/mod.rs` and `004_mutation_trace_protocol.sql`)

### Success-criteria verification

- [x] AC1: `coordinate()` runs through the shared protected-worktree primitive with ordering and error semantics intact -> `services::mutation_trace::runtime::coordinator::` — 26 passed; the fence, lock-contention, and `MarkerClearAfterCommit` tests pass with assertion content unmodified (confirmed against T01 record).
- [x] AC2: Abandoning an `Active` scope changes only that scope's status, advances revision by one, sets `needs_rebaseline`, writes no event rows -> `scope_runtime::tests::abandoning_a_live_scope_writes_only_the_scope_and_worktree_rows` passed.
- [x] AC3: `abandon_scope()` performs no snapshot/pin/diff/reconciliation/registration/init -> `rg` over `scope_runtime.rs` returns no match (exit 1); the module reads only durable mutation-scope state.
- [x] AC4: A `Closed`/`Abandoned` target settles as a successful terminal no-op with the marker cleared -> `scope_runtime::tests::a_closed_scope_settles_as_a_terminal_no_op` and `…an_already_abandoned_scope_settles_as_a_terminal_no_op` passed.
- [x] AC5: A missing and a `NeverSeen` target both return recovery-required, leave the marker armed, commit no abandonment -> `scope_runtime::tests::a_missing_scope_row_requires_recovery_and_leaves_the_fence_armed` and `…a_never_seen_scope_requires_recovery_and_leaves_the_fence_armed` passed.
- [x] AC6: A pre-existing marker returns recovery-required for that reason without clearing it and without invoking the DB provider -> `scope_runtime::tests::an_inherited_marker_requires_recovery_without_consulting_the_db_provider` passed.
- [x] AC7: A target owned by another `WorktreeId` is rejected as an error writing nothing -> `runtime::tests::abandoning_a_scope_through_another_worktrees_checkout_is_rejected_without_writing` passed; `scope_runtime::tests::a_scope_owned_by_another_worktree_is_rejected_without_writing` passed.
- [x] AC8: A CAS conflict reloads and recomputes within the coordinator's retry limit; a competitor-terminal scope settles as the terminal no-op -> `runtime::tests::a_real_thread_cas_race_settles_on_the_competitors_terminal_status` and `scope_runtime::tests::a_cas_conflict_whose_competitor_ended_the_scope_settles_as_a_terminal_no_op` / `…a_cas_conflict_recomputes_the_abandonment_from_fresh_state` passed.
- [x] AC9: An `Active` target at `revision: u64::MAX` produces a distinct revision-exhaustion error -> `scope_runtime::tests::a_live_scope_on_an_exhausted_revision_is_a_distinct_error` passed.
- [x] AC10: Marker stays armed on DB-provider `Err` / persistence failure; cleared on abandonment and terminal no-op; a `clear()` failure returns an error carrying the completed outcome -> `scope_runtime::tests::a_db_provider_failure_leaves_the_fence_armed`, `…a_persistence_failure_rolls_back_the_whole_transition_and_leaves_the_fence_armed`, `…a_marker_clear_failure_carries_the_already_settled_outcome` passed.
- [x] AC11: Real Git + real repo-scoped DB: `Start(A)` → edit → `abandon_scope(A)` → edit → `coordinate(Start(B))` leaves the cursor at `Start(B)`'s tree, emits no event for the gap, A `Abandoned`, B `Active` -> `runtime::tests::an_abandoned_scope_rebaselines_the_successor_start_without_evidence_for_the_gap` passed.
- [x] AC12: Abandoning stale A leaves unrelated live B `Active` through the recovery -> `runtime::tests::abandoning_a_stale_scope_leaves_an_unrelated_live_scope_active_through_the_recovery` passed.
- [x] AC13: `runtime/mod.rs` re-exports exactly `coordinate`, `CoordinateError`, `CoordinateOutcome`, `ExternalTaintOperation`, `RuntimeBoundary`, `abandon_scope`, `AbandonRecoveryReason`, `AbandonScopeError`, `AbandonScopeOutcome`; every `mod` private -> inspected `mod.rs` (two `pub(crate) use` blocks, all `mod` lines private); `clippy --all-targets -- -D warnings` clean.
- [x] AC14: No change attributable to this PR in `spec/mutation_cursor.qnt`, the Quint refinement matrix, `protocol.rs`, `004_mutation_trace_protocol.sql`, or the migration set, baselined on `origin/ref-rec` -> `git diff --stat origin/ref-rec...HEAD` over those paths is empty; `nix flake check` Quint checks green.
- [x] AC15: `context/cli/mutation-scope-runtime.md` exists and states each required item -> read in full: scope-identity rule, `Start`/`Advance`/`Close` semantics (failed tool still requires `Advance`, no `ScopeId` reuse after terminal), positive-staleness-evidence requirement and the `ActorKind` prohibition, the abandon → successor-`Start` outcome table (including that a failed abandonment is not a safely started successor), abandonment is not a `RuntimeBoundary` and needs no Quint change, D1's strong-recovery tradeoff, and the `AiExclusive` attribution boundary — all present and consistent with the shipped `scope_runtime.rs` / `coordinator.rs`.

### Failed checks and follow-ups

- None.

### Residual risks

- No harness adapter exercises either entrypoint yet, so `abandon_scope()` has no production caller; the re-export blocks carry `#[allow(unused_imports)]` and coverage rests entirely on the inline and integration test suites. This is the plan's explicit end state, not a regression.
