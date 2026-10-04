# Plan: mutation-cursor-health-invariant

## Change summary

The verified mutation-cursor model (`spec/mutation_cursor.qnt`) stores durable
snapshot health twice, as `tainted: bool` and `failureKind: FailureKind`, and
proves `FailureKindMatchesTaint` (`tainted == (failureKind != Healthy)`) for
worktrees and `MutationFailureKindMatchesTaint` for mutation events. Production
does not enforce that link. Migration `004_mutation_trace_protocol.sql` checks
each column on its own (`tainted IN (0, 1)`, `failure_kind IN ('healthy',
'snapshot_failure')`) on both `mutation_trace_worktrees` and
`mutation_trace_events`. The store decoders in
`cli/src/services/mutation_trace/store.rs` (`worktree_state_row_from_turso`,
`mutation_event_row_from_turso`, `mutation_event_page_row_from_turso`) accept
any combination. So `(tainted=0, snapshot_failure)` and `(tainted=1, healthy)`
can be stored and loaded even though the protocol says they cannot exist.

Different runtime layers read different halves of the pair:

- `protocol::attribution_for` uses `failure_kind != Healthy`.
- `protocol::taint` uses `tainted` in its guard.
- `protocol::recover` uses `tainted` both in its no-op guard and to choose
  `abandon_live_scopes`.
- The coordinator's `needs_recovery` uses `tainted || needs_rebaseline`.
- The coordinator's `run_taint_retry_loop_inner` reads `persisted_taint` back
  from `tainted`.
- `mutation_attribution::transition_origin` uses
  `!tainted && failure_kind == Healthy`.

The Quint `taint`, `recover`, `recoverNeeded`, `verifyTaint` and `verifyRecover`
use `state.tainted`. With an invalid stored pair, these layers disagree. For
example, `(tainted=false, SnapshotFailure, needs_rebaseline=false)` is "no
recovery needed" to the coordinator but unhealthy to attribution.
`(tainted=false, SnapshotFailure, needs_rebaseline=true)` enters recovery but
takes the weak path, which keeps active scopes, instead of the
snapshot-failure path, which abandons them.

The normal protocol writers (`taint`, `recover`, and the event materialization
in `commit`) always set or copy both fields together, and no current test
fixture inserts an inconsistent pair. So this plan does not claim that a normal
transition writes a bad pair. The defect is that the durable boundary accepts
states the verified protocol forbids, and runtime layers disagree on what such
a state means.

The fix makes `FailureKind` the semantic source of truth and keeps `tainted` as
a redundant stored bit that must always match it:

1. Every semantic health decision, in Rust and in Quint, reads
   `failure_kind != Healthy`.
2. A new forward migration `006_mutation_trace_health_invariant.sql` rebuilds
   both tables with a cross-column `CHECK`. While copying, it normalizes legacy
   rows using `failure_kind` as the authority.
3. One shared store validator rejects an inconsistent pair whenever a
   `WorktreeState` or mutation-event row is decoded.

This extends existing behavior. `tainted` stays in the durable and verified
representation: `WorktreeState`, `MutationEvent`, the Quint state, and both
tables. The only place it is removed is the derived store read projection
`MutationEventPageRow`. The change adds no `FailureKind` variants and changes no
transition on any state the protocol can reach.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation. Each criterion names the one task that owns it, and
no criterion depends on work from a later task than its owner.

- [x] AC1 (owner: T01): Protocol and coordinator health decisions use `FailureKind`.
  - `protocol::recover` treats a worktree as needing recovery when `failure_kind != Healthy || external_taint || needs_rebaseline`.
  - `protocol::recover` picks strong recovery (abandon every live scope on the worktree) when `failure_kind != Healthy || external_taint`. It picks weak recovery (keep live scopes) only when `failure_kind == Healthy && !external_taint && needs_rebaseline`.
  - `protocol::taint` is a guarded no-op when `failure_kind != Healthy` or the worktree is externally tainted.
  - Coordinator `needs_recovery` returns `true` for `external_taint || failure_kind != Healthy || needs_rebaseline`.
  - `run_taint_retry_loop_inner`'s "already unhealthy" read-back reports `failure_kind != Healthy`.
  - In non-test code in `protocol.rs` and `runtime/coordinator.rs`, no read of `.tainted` decides health. The remaining reads only copy the bit into a new `WorktreeState`/`MutationEvent`.
  
  This criterion does not cover `MutationEventPageRow` or `mutation_attribution::transition_origin`. Those belong to AC8.
  - Validate: `nix shell nixpkgs#ripgrep -c rg -n '\.tainted\b' cli/src/services/mutation_trace/protocol.rs cli/src/services/mutation_trace/runtime/coordinator.rs`. Inspect every hit outside a `#[cfg(test)]` module and confirm it is a field copy, never part of a branch condition, guard or boolean health predicate.
- [x] AC2 (owner: T01): Synthetic, pure regression state `failure_kind=SnapshotFailure, tainted=false, needs_rebaseline=true`, with active scope A on the worktree and no external taint. `protocol::recover(state, wt, observed)` takes the strong path: A becomes `Abandoned`, `cursor_tree == observed`, `failure_kind == Healthy`, `tainted == false`, `needs_rebaseline == false`, and the revision advances by one.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace` passes, including the new named protocol test for this state.
- [x] AC3 (owner: T01): Coordinator recovery admission: `needs_recovery` returns `true` for `failure_kind=SnapshotFailure, tainted=false, needs_rebaseline=false` with no external taint. The test calls the private function from `coordinator.rs`'s own `#[cfg(test)] mod tests`; its visibility does not change.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace` passes, including the new named coordinator test.
- [x] AC4 (owner: T01): Existing semantics hold on consistent states.
  - `failure_kind=Healthy, tainted=false, needs_rebaseline=true` with active scope A recovers weakly: A stays `Active`, cursor = observed, `needs_rebaseline == false`.
  - `failure_kind=SnapshotFailure, tainted=true` with active scope A recovers strongly: A becomes `Abandoned` and the worktree is `Healthy`/untainted.
  - All existing protocol, coordinator and attribution tests pass unchanged.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace` passes, including explicit named tests for both cases (new tests, or existing ones named in the task's completion record).
- [x] AC5 (owner: T01): The Quint model makes its semantic health decisions from `failureKind != Healthy`.
  - `taint` and `verifyTaint` guard on `state.failureKind != Healthy`.
  - `recover` and `verifyRecover` admit on `failureKind != Healthy or externalTaint or needsRebaseline`.
  - `recoverNeeded`'s `abandonLiveScopes` is `state.failureKind != Healthy or externalTaint.contains(worktree)`.
  - The model keeps both state fields. `FailureKindMatchesTaint` and `MutationFailureKindMatchesTaint` are unchanged in definition and still listed in the safety invariants.
  
  These edits are behaviorally equivalent on every reachable state, because `FailureKindMatchesTaint` makes `state.tainted` and `state.failureKind != Healthy` the same value there. All existing Quint runs and Quint Connect traces pass unchanged.
  
  Reads of `tainted` stay allowed when they copy state forward, build a `MutationEvent`, record the representation, state `FailureKindMatchesTaint`/`MutationFailureKindMatchesTaint`, or assert expected values in `test*` runs. They are not allowed in a semantic health or recovery decision: the `taint` guard, the `recover` guard, the strong/weak choice, `abandonLiveScopes`, the `verify*` preconditions, or any other health predicate.
  - Validate:
    - `nix run .#quint -- typecheck spec/mutation_cursor.qnt`
    - `nix run .#quint -- test spec/mutation_cursor.qnt --match '^test.*'`
    - `nix build .#checks.x86_64-linux.mutation-trace-quint-connect --print-build-logs`
    - Inspection: list every `tainted` read with `nix shell nixpkgs#ripgrep -c rg -n 'tainted' spec/mutation_cursor.qnt`. Confirm that each one is a copy, a construction, a representation-equality invariant or a test assertion, and that none decides semantic health or recovery behavior.
- [x] AC6 (owner: T02): After migration `006`, both `mutation_trace_worktrees` and `mutation_trace_events` accept `(tainted=0, 'healthy')` and `(tainted=1, 'snapshot_failure')`, and reject `(tainted=0, 'snapshot_failure')` and `(tainted=1, 'healthy')` with a `CHECK` error. The existing per-column allow-lists, the revision `BLOB` check, the attribution and boundary `CHECK`s, and the primary keys still apply.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db` passes, including eight new schema-constraint tests (four combinations × two tables) and the existing `mutation_trace_*` schema tests.
- [x] AC7 (owner: T02): A database migrated through `005` only that contains inconsistent legacy rows is upgraded by `006`:
  - a worktree row `('snapshot_failure', tainted=0)` becomes `tainted=1`, and a worktree row `('healthy', tainted=1)` becomes `tainted=0`;
  - the same two normalizations apply to `mutation_trace_events` rows.
  
  Every other column (`worktree_id`, `cursor_tree`, `revision`, `needs_rebaseline`, `created_at`, `updated_at`, event trees, attribution and boundary fields, event `created_at`) is preserved byte-for-byte. Consistent rows are unchanged. `mutation_trace_event_active_scopes`, `mutation_trace_scopes`, `mutation_trace_processed_events` and `mutation_trace_scope_provenance` rows are untouched. `__sce_migrations` records `006_mutation_trace_health_invariant`.
  
  Running `006`'s SQL body again on an already-migrated database succeeds and changes nothing. This proves only that the SQL body can be re-run. It does not prove the migration runner is safe under concurrency.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db` passes, including the new legacy-normalization test(s) and the test that runs the SQL body twice.
- [ ] AC8 (owner: T03): Store decoding validates the redundant health pair, and `mutation_attribution::transition_origin` decides health from `FailureKind` alone.
  - `store.rs` has one shared validator for the pair. It returns `Ok` for `(false, Healthy)` and `(true, SnapshotFailure)` and `Err` for `(false, SnapshotFailure)` and `(true, Healthy)`.
  - All three decoders (`worktree_state_row_from_turso`, `mutation_event_row_from_turso`, `mutation_event_page_row_from_turso`) call it and pass the `Err` up with `Result` and the column/table context. No ordinary read silently repairs a value.
  - With a `005`-only fixture holding an inconsistent row, `load_worktree` and the mutation-event read paths return `Err` instead of a value.
  - The derived read projection `MutationEventPageRow` no longer has a `tainted` field. Its decoder still selects and validates the column.
  - `transition_origin` treats an event as healthy exactly when `failure_kind == FailureKind::Healthy`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace` passes, including the four-case validator test and the decoder rejection tests. Inspect `MutationEventPageRow` in `store.rs` (no `tainted` field) and `transition_origin` in `runtime/mutation_attribution.rs` (no `tainted` read).
- [ ] AC9 (owner: T03): Across the whole mutation-trace implementation, no non-test `.tainted` read decides health. Every remaining read only copies, persists (SQL bind/encode), validates (through the shared validator) or materializes the redundant bit. This holds only after T03, which removes the last semantic read (`transition_origin`'s `row.tainted`).
  - Validate: `nix shell nixpkgs#ripgrep -c rg -n '\.tainted\b|\btainted\b' cli/src/services/mutation_trace --glob '!**/tests.rs' --glob '!**/mbt/**'`. Inspect every hit outside a `#[cfg(test)]` module and confirm that each one is a copy, persist, validate or materialize use, never a branch condition, guard or boolean health predicate.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix run .#quint -- typecheck spec/mutation_cursor.qnt`
- `nix run .#quint -- test spec/mutation_cursor.qnt --match '^test.*'`
- `nix build .#checks.x86_64-linux.mutation-trace-quint-connect --print-build-logs`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db`

### Context sync

- `context/cli/mutation-trace-protocol.md` (T01): record that `FailureKind` is the semantic health source of truth and that `tainted` is a redundant bit enforced to match it. State the recovery admission predicate, the strong/weak recovery split and the `taint` guard in `failure_kind` terms.
- `context/cli/mutation-trace-runtime-coordinator.md` (T01): change "recover first if the worktree is tainted" and the "already-tainted no-op reads back the current flag" wording to the `failure_kind != Healthy` predicate.
- `spec/mutation_cursor.md` (T01): changed as part of T01 itself (the `worktrees.tainted` / `failureKind` table row and the recovery prose).
- `context/cli/mutation-trace-store.md` (T02, T03):
  - T02: record migration `006_mutation_trace_health_invariant.sql`: the rebuild of `mutation_trace_worktrees` and `mutation_trace_events`, the cross-column health `CHECK`, legacy normalization with `failure_kind` as the authority, and the re-runnable SQL body. State that the runner's existing non-atomic check/apply/record behavior is unchanged.
  - T03: record the shared fail-closed decode validator used by all three row decoders, and that `MutationEventPageRow` no longer carries `tainted`.
- `context/cli/mutation-trace-agent-attribution.md` (T03): change the `MutationAi(scope)` origin rule ("untainted, `FailureKind::Healthy`") so health comes from `failure_kind` alone, with the pair kept consistent by the store validator.
- `context/context-map.md` (T02): extend the `mutation-trace-store.md` annotation to mention migration `006`'s health invariant.
- `context/cli/mutation-trace-runtime-materialization.md`: inspected. Its only claim (`NeverSeen` ≠ a healthy `WorktreeState { tainted: false, failure_kind: Healthy }`) is still accurate. No change expected.

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
  - `cli/src/services/mutation_trace/protocol.rs`
  - `cli/src/services/mutation_trace/runtime/coordinator.rs`
  - `cli/src/services/mutation_trace/runtime/mutation_attribution.rs`
  - `cli/src/services/mutation_trace/store.rs`, including removing `tainted` from the derived `MutationEventPageRow` projection
  - the refinement-matrix rows in `cli/src/services/mutation_trace/mod.rs`
  - the tests next to the files above
  - the new `cli/migrations/agent-trace-repository/006_mutation_trace_health_invariant.sql`
  - migration and schema tests in `cli/src/services/agent_trace_db/repository.rs`, including the existing expected-migration-ID lists and the "never applies 003, 004, or 005" hook-runtime assertion text
  - `spec/mutation_cursor.qnt` and `spec/mutation_cursor.md`
  - the context files listed under Context sync
- **Out of scope:**
  - removing `tainted` from `WorktreeState`, `MutationEvent`, the Quint state, `mutation_trace_worktrees` or `mutation_trace_events`. These keep the bit. Only the derived `MutationEventPageRow` projection drops it.
  - any other change to the mutation-event schema
  - new `FailureKind` variants or any change to the failure taxonomy
  - external-taint marker semantics
  - `needsRebaseline` semantics
  - adapter lifecycle semantics
  - the Codex doctor issue
  - the migration runner (`cli/src/services/db/mod.rs`), including its existing non-atomic `is_migration_applied` → `execute_batch` → `INSERT __sce_migrations` sequence and the concurrent-opener metadata race it allows
  - redesigning how migrations are written or applied
  - any other mutation-cursor cleanup
- **Constraints:**
  - Migration `004` (and `005`) are not modified.
  - `006` is forward-only. Its SQL body must be re-runnable after a completed or partially observed attempt. The runner (`cli/src/services/db/mod.rs::apply_migration`) runs the file with `execute_batch` and only afterwards inserts the `__sce_migrations` row, so if execution finishes but the metadata insert does not, the next open runs the body again against an already-rebuilt schema. Being re-runnable does not make the runner safe under concurrency. Two processes can both pass `is_migration_applied == false` and then race on recording the ID. That race is existing behavior and out of scope.
  - The rebuild runs inside one explicit transaction in the migration file, so the database never holds a half-rebuilt table. T02's probe test confirms the transaction is supported before relying on it.
  - Store decoding fails closed with `anyhow::Result`. It never repairs a value.
  - Task order is T01 → T02 → T03, so no intermediate commit leaves production unable to read an existing database. T01 changes no persistence. T02 normalizes any legacy inconsistent row before T03 starts rejecting such rows. Production repository opens already refuse a database with missing migrations: the hook runtime fails `ensure_schema_ready_for_hooks` with "Run 'sce setup'", and setup/lifecycle opens fall back to `new_at`, which runs migrations. So after T03 no production reader sees a pre-`006` row without `006` having run first.
  - All commands run through Nix as in `AGENTS.md`.
- **Non-goal:** a representation redesign that derives `tainted` from `failure_kind` or drops the duplicate field from the domain types, Quint state or tables. That is a separate later cleanup.

## Assumptions

- The plan file is `context/plans/mutation-cursor-health-invariant.md` and the migration ID is `006_mutation_trace_health_invariant`. The repository's highest migration is `005_mutation_scope_provenance.sql`, and `cli/build.rs` embeds migrations in file order.
- The cross-column check is written as `CHECK (tainted = CASE WHEN failure_kind = 'healthy' THEN 0 ELSE 1 END)` and added as a table-level constraint next to the existing column checks on both rebuilt tables. Every other column definition, default, `CHECK` and primary key is copied verbatim from `004`.
- Rebuild shape for each table, all inside `BEGIN IMMEDIATE; … COMMIT;`:
  1. `DROP TABLE IF EXISTS <table>_v006;` (defensive; with the transaction, an interrupted run rolls back and leaves no staging table)
  2. `CREATE TABLE <table>_v006 (…)` with the new check
  3. `INSERT INTO <table>_v006 (…) SELECT …, CASE WHEN failure_kind = 'healthy' THEN 0 ELSE 1 END AS tainted, … FROM <table>;` with an explicit column list
  4. `DROP TABLE <table>;`
  5. `ALTER TABLE <table>_v006 RENAME TO <table>;`
  
  Neither table has secondary indexes, triggers, views, or foreign keys pointing at it (checked against `004`/`005`), so nothing else needs re-creating. Re-running the body against an already-rebuilt schema reproduces the same schema and rows, because normalization is a fixed point on consistent data.
- Normalization copies `created_at`/`updated_at` and `revision` unchanged. It is a repair of the stored representation, not a protocol transition, so it does not bump revisions or timestamps.
- T02 starts with a probe test that proves the installed `turso` (`0.8.1`, `cli/Cargo.toml`) supports what the rebuild needs:
  - `BEGIN IMMEDIATE`/`COMMIT` inside `execute_batch`
  - `ALTER TABLE … RENAME TO`
  - `DROP TABLE`
  - enforcement of a `CASE`-expression table `CHECK`
  
  If any of these is unsupported, T02 stops as `blocked` and the plan is revised. The task does not invent another mechanism.
- The legacy-row fixture builds a database migrated through `005` only. It opens with `RepositoryAgentTraceDb::open_without_migrations_at`, executes the first five entries of `generated_migrations::AGENT_TRACE_REPOSITORY_MIGRATIONS`, records their IDs in `__sce_migrations`, inserts the inconsistent rows, then reopens with `RepositoryAgentTraceDb::new_at` so that the real runner applies `006`. T03 reuses the same fixture to put inconsistent rows in front of the decoders.
- The shared validator is a private `store.rs` function, `validate_health_encoding(tainted: bool, failure_kind: FailureKind) -> Result<()>`. Its error text has a stable `bail!` prefix naming both values. Each decoder adds its table/column context with `.context(...)` or `.with_context(...)`.
- `MutationEventPageRow` is a derived store read projection that only the attribution page reader uses. It is not part of the durable or verified domain representation, so removing its `tainted` field is in scope, while `WorktreeState`, `MutationEvent`, the Quint state and both tables keep the bit. The removal and the `transition_origin` switch go together in T03:
  - If `transition_origin` stopped reading `row.tainted` earlier, `MutationEventPageRow::tainted` would become a never-read `pub` field. `sce` is a binary-only crate (`cli/Cargo.toml` has `[[bin]]` and no `lib.rs`), so that triggers `dead_code` and fails `cli-clippy`.
  - Keeping the conjunction `!tainted && failure_kind == Healthy` until T03 is safe. On `(false, SnapshotFailure)` it already agrees with `failure_kind`. `(true, Healthy)` only makes attribution more conservative, and T03 makes that state unloadable.
- In Rust, semantic reads compare against `FailureKind::Healthy` directly (`failure_kind != FailureKind::Healthy`), as `attribution_for` already does. No new health helper type is introduced.
- The `taint` guard change has the same effect on every consistent state. On a synthetic `(tainted=false, SnapshotFailure)` it becomes a no-op where it used to re-taint. On a synthetic `(tainted=true, Healthy)` it now records `SnapshotFailure` where it used to no-op. Both follow the source-of-truth rule. Store validation prevents either state from being loaded in production.

## Task stack

- [x] T01: `Derive mutation-cursor health decisions from failure_kind` (status:done)
  - Task ID: T01
  - Scope: In:
    - Rust `protocol::taint` guard; `protocol::recover` no-op guard and `abandon_live_scopes`
    - coordinator `needs_recovery` and the "already unhealthy" read-back in `run_taint_retry_loop_inner`
    - Quint `taint`, `recover`, `recoverNeeded` (`abandonLiveScopes`), `verifyTaint` and `verifyRecover` guards
    - doc comments on the changed Rust functions and the `FailureKindMatchesTaint`/`MutationFailureKindMatchesTaint` rows of the `mod.rs` refinement matrix
    - `spec/mutation_cursor.md` prose stating that `failureKind` is the health source of truth, the recovery predicates, and the behavioral-equivalence note
    - new deterministic tests:
      - the AC2 strong-recovery regression (pure `protocol::recover`)
      - the AC3 `needs_recovery` regression inside `coordinator.rs`'s test module
      - a `taint` no-op test on `(tainted=false, SnapshotFailure)`
      - named healthy-rebaseline weak-recovery and consistent snapshot-failure strong-recovery tests for AC4, added if no existing test already pins exactly those assertions
    
    Out:
    - any schema, migration, or store decoding change
    - removing `tainted` from any type, table, projection or Quint state (`MutationEventPageRow` is T03)
    - `mutation_attribution::transition_origin` (T03; see Assumptions for why it must move together with the projection change)
    - changing `FailureKindMatchesTaint`/`MutationFailureKindMatchesTaint`
    - new Quint runs or invariants (invalid pairs are unreachable in the model)
  - Dependencies: none
  - Done when:
    - AC1, AC2, AC3, AC4 and AC5 hold. None of them depends on T02 or T03.
    - The Quint spec typechecks and all `test*` runs pass.
    - Quint Connect stays green with no change to `mbt/`.
    - `spec/mutation_cursor.md` says that semantic health decisions read `failureKind`, and that `tainted` is the redundant bit `FailureKindMatchesTaint` proves equal to it.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
    - `nix run .#quint -- typecheck spec/mutation_cursor.qnt`
    - `nix run .#quint -- test spec/mutation_cursor.qnt --match '^test.*'`
    - `nix build .#checks.x86_64-linux.mutation-trace-quint-connect --print-build-logs`
    - the AC1 Rust inspection (`protocol.rs`, `runtime/coordinator.rs`) and the AC5 Quint inspection
  - Completed: 2026-10-04
  - Files changed:
    - `cli/src/services/mutation_trace/mod.rs`
    - `cli/src/services/mutation_trace/protocol.rs`
    - `cli/src/services/mutation_trace/runtime/coordinator.rs`
    - `cli/src/services/mutation_trace/tests.rs`
    - `spec/mutation_cursor.md`
    - `spec/mutation_cursor.qnt`
  - Result:
    - Rust `protocol::taint` guard, `protocol::recover` admission and `abandon_live_scopes`, coordinator `needs_recovery`, and the `run_taint_retry_loop_inner` "already unhealthy" read-back now use `failure_kind != FailureKind::Healthy`. Doc comments updated.
    - Quint `taint`, `recoverNeeded` (`abandonLiveScopes`), `recover`, `verifyTaint` and `verifyRecover` now guard on `failureKind`. State fields, `FailureKindMatchesTaint` and `MutationFailureKindMatchesTaint` are unchanged.
    - New tests: `recover_reads_health_from_failure_kind_and_takes_the_strong_path_when_tainted_disagrees` (AC2), `needs_recovery_reads_health_from_failure_kind_even_if_tainted_disagrees` (AC3, in `coordinator.rs` tests), `taint_is_a_no_op_when_failure_kind_is_unhealthy_even_if_tainted_disagrees`.
    - AC4 is pinned by the existing tests `recover_with_only_needs_rebaseline_preserves_live_scopes` (weak) and `recover_from_snapshot_taint_abandons_live_scopes_and_rebaselines_cursor` (strong), so no duplicate tests were added.
    - `mod.rs` refinement-matrix rows and `spec/mutation_cursor.md` record `failureKind` as the semantic health source and `tainted` as the redundant bit.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`: passed (376 passed, 0 failed)
    - `nix run .#quint -- typecheck spec/mutation_cursor.qnt`: passed (exit 0)
    - `nix run .#quint -- test spec/mutation_cursor.qnt --match '^test.*'`: passed (41 passing)
    - `nix build .#checks.x86_64-linux.mutation-trace-quint-connect --print-build-logs`: passed (exit 0, no `mbt/` change)
    - AC1 inspection: passed. In `protocol.rs` and `runtime/coordinator.rs`, the only non-test `.tainted` reads are field copies at `protocol.rs:363`, `:384` and `:515`. This covers the protocol and coordinator only. `runtime/mutation_attribution.rs::transition_origin` still reads `row.tainted` by design; that repository-wide guarantee belongs to T03 / AC9.
    - AC5 inspection: passed. The remaining `tainted` reads in `spec/mutation_cursor.qnt` are type fields, constructions, copies (`commit` event build, `abandonLiveScope`), the two equality invariants, one `test*` assertion and a comment.
  - Context impact: localized to the mutation-trace domain. `context/cli/mutation-trace-protocol.md` and `context/cli/mutation-trace-runtime-coordinator.md` were synchronized to describe health and recovery in `failure_kind` terms per the plan's Context sync section. No root-level architecture or terminology change.
  - Context synchronization: synced

- [x] T02: `Enforce the health invariant with forward migration 006` (status:done)
  - Task ID: T02
  - Scope: In:
    - the turso capability probe test
    - `cli/migrations/agent-trace-repository/006_mutation_trace_health_invariant.sql`: rebuilds `mutation_trace_worktrees` and `mutation_trace_events` per the Assumptions rebuild shape, inside one transaction, with `failure_kind`-authoritative `tainted` normalization and a re-runnable SQL body
    - in `cli/src/services/agent_trace_db/repository.rs` tests:
      - the eight AC6 schema-constraint tests
      - the AC7 legacy-normalization test(s) for both tables, including preservation of unrelated columns and tables
      - the test that runs the SQL body twice
      - updating the existing expected-migration-ID lists and the hook-runtime "never applies" assertion to include `006`
    
    Out:
    - edits to `004`/`005`
    - migration runner changes, including its concurrent-opener metadata race
    - store decoder changes
    - any column, type, or other constraint change to either table
  - Dependencies: T01
  - Done when:
    - AC6 and AC7 hold.
    - A fresh `new_at` database and an upgraded `005`-only database produce the same `sqlite_master` SQL for both tables.
    - Every existing `agent_trace_db` and `mutation_trace` test passes against the new schema.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db`
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
  - Completed: 2026-10-04
  - Files changed:
    - `cli/migrations/agent-trace-repository/006_mutation_trace_health_invariant.sql`
    - `cli/src/services/agent_trace_db/repository.rs`
  - Result:
    - New migration `006_mutation_trace_health_invariant.sql` runs `BEGIN IMMEDIATE; … COMMIT;` and rebuilds `mutation_trace_worktrees` and `mutation_trace_events` using the Assumptions rebuild shape. Each table gets `CHECK (tainted = CASE WHEN failure_kind = 'healthy' THEN 0 ELSE 1 END)`; every other column, default, `CHECK` and primary key is copied verbatim from `004`. `tainted` is recomputed from `failure_kind` during the copy. Following the user's no-comments preference, the file has no SQL comments.
    - Probe test `turso_supports_the_transactional_table_rebuild_used_by_migration_006` runs the rebuild statements through the real runner's `execute_batch` path, using a test-only `DbSpec`. It passed, so the plan's turso capability assumption holds.
    - The `005`-only fixture uses a test-only `PreHealthInvariantDbSpec`, whose `migrations()` returns the first five embedded migrations. `001`–`005` are therefore applied and recorded by the real runner, without inserting them by hand through `open_without_migrations_at`. `db/mod.rs` is unchanged.
    - AC6 tests: `mutation_trace_{worktrees,events}_{accepts_untainted_healthy,accepts_tainted_snapshot_failure,rejects_untainted_snapshot_failure,rejects_tainted_healthy}_pair` (8), plus `migration_006_keeps_the_existing_mutation_trace_column_checks_and_primary_keys`.
    - AC7 tests:
      - `migration_006_normalizes_legacy_health_pairs_from_failure_kind_and_preserves_other_columns` checks both tables and the four untouched tables, compared through `quote(...)` projections, and checks that `006` is recorded.
      - `migration_006_sql_body_reruns_without_changing_schema_or_rows` deletes the `006` metadata row and reopens, so the real runner runs the body again.
    - Schema-parity done check: `fresh_and_upgraded_databases_share_the_migration_006_table_sql`.
    - The existing expected-migration-ID lists and the hook-runtime "never applies" message now include `006`.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db`: 41 passed, 1 failed.
      - The failure is `equal_time_same_kind_observations_use_a_stable_tie_break_and_concurrent_writes_converge`: concurrent `claude_model_state` writers hit `database is locked` after 5 retries. The test is unrelated to T02 and already flaky: it failed 6/6 runs on clean baseline `a872c962` with T02 stashed, and 4/6 runs with T02 applied.
      - With that one test skipped (`-- --skip equal_time_same_kind_observations`), the run passed: 41 passed, 0 failed, including all 13 new T02 tests.
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`: passed (385 passed, 0 failed).
    - Extra check: `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`: passed.
  - Deviation: T02 does not fix the `claude_model_state` concurrency flake, because it is outside this plan's scope. It needs separate follow-up.
  - Context impact: localized to the mutation-trace store. `context/cli/mutation-trace-store.md` was synchronized to document migration `006` (rebuild, cross-column health `CHECK`, `failure_kind`-authoritative normalization, re-runnable body, runner behavior unchanged). `context/context-map.md` was synchronized with its `mutation-trace-store.md` annotation extended to cover the same. No root architecture or terminology change.
  - Context synchronization: synced

- [ ] T03: `Reject inconsistent health pairs when decoding store rows` (status:todo)
  - Task ID: T03
  - Scope: In:
    - the private shared `validate_health_encoding` in `store.rs`, called from `worktree_state_row_from_turso`, `mutation_event_row_from_turso` and `mutation_event_page_row_from_turso`
    - removing the `tainted` field from the derived `MutationEventPageRow` projection. Its decoder still selects and validates the column but does not keep it.
    - switching `mutation_attribution::transition_origin` to `failure_kind == FailureKind::Healthy` alone
    - the four-case validator unit test
    - decoder rejection tests that use the `005`-only fixture to place an inconsistent `mutation_trace_worktrees` row and an inconsistent `mutation_trace_events` row, and assert that `load_worktree` and the mutation-event read paths return `Err`
    
    Out:
    - removing `tainted` from `WorktreeState`, `MutationEvent`, the Quint state or either table
    - silent repair on read
    - changes to the test-only `read_worktree` helper in `scope_runtime.rs` beyond what compiling requires
    - protocol, coordinator or Quint semantic changes (T01)
  - Dependencies: T02
  - Done when:
    - AC8 and AC9 hold.
    - All existing store, coordinator and attribution tests still pass, since every existing fixture writes consistent pairs.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db`
    - the AC8 inspections and the AC9 repository-wide Rust inspection
  - Context synchronization: pending

## Open questions

None. The request fixes the source of truth, migration strategy, validation
boundary, task ownership, test matrix and non-goals. The only unknown is
whether `turso` `0.8.1` supports the rebuild statements. T02 checks that with
a probe test and stops if they are missing.
