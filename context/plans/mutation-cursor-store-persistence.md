# Plan: mutation-cursor-store-persistence

## Change summary

Adds a durable persistence layer for the verified mutation-cursor protocol
(`cli/src/services/mutation_trace/`), storing the protocol's worktree/scope/
processed-event/mutation-event state in the repository-scoped Agent Trace DB
(`RepositoryAgentTraceDb`) via a new additive migration and a new `store.rs`
module. This is the third build-out step for the module, following the pure
kernel (`mutation-cursor-protocol-kernel`) and its Quint Connect verification
harness (`mutation-cursor-quint-connect`); it extends that work rather than
replacing it, and `protocol.rs` remains exactly as pure as those two plans
left it.

The persistence boundary is one-directional and structural:
`protocol.rs` (pure semantics) -> `DurableTransition` (a persistence
projection built by pure structural diffing, not protocol interpretation) ->
`store.rs` (SQL translation) -> `RepositoryAgentTraceDb`. `protocol.rs` never
depends on SQL or the DB adapter, and `store.rs` never branches on protocol
meaning (boundary kind, contention, taint) — it only diffs before/after
`ProtocolState` values.

Two things are deliberately excluded from the database: `AttemptState`
(explicitly transient in the domain model — no `mutation_trace_attempts`
table) and `external_taint` (a `database_failure()` cannot use the database
it just failed against as the authoritative record that the write was
uncertain; a later plan represents it as a filesystem write-ahead marker).

The runtime read path is split in two. The hot path (`load_worktree`) loads
one worktree, only its currently `Active` scopes, an optionally referenced
scope even when that scope is terminal (`NeverSeen`/`Closed`/`Abandoned`),
and an optional `EventKey` replay row — never historical
`mutation_trace_events` rows and never a terminal scope it was not
explicitly asked for, so the read stays bounded as closed/abandoned scopes
accumulate over time. A separate cold path (`load_mutation_event`)
reconstructs one historical `MutationEvent` by `(worktree, revision)` only on
explicit request. `DurableTransition::between` is a strict structural
firewall: it validates shape (single worktree, no unrelated changes,
revision advances by exactly one when a transition exists, at most one new
processed event, at most one new mutation event) and rejects a structurally
impossible before/after pair, without ever interpreting protocol semantics.
The CAS primitive keeps three outcomes distinct: a stale revision is a
`Conflict` the DB primitive never retries, a transient DB failure retries
the whole transaction, and a deterministic SQL/constraint failure returns an
error without retry.

## Acceptance criteria

- [ ] AC1: Mutation state lives in the repository-scoped `agent-trace.db`.
  - Validate: `cli/src/services/mutation_trace/store.rs` reads/writes only through `RepositoryAgentTraceDb`; round-trip tests in T09 pass.
- [ ] AC2: New storage is introduced through additive migration `003`, with `001`/`002` byte-unchanged by this PR.
  - Validate: `git diff --exit-code <base-branch>...HEAD -- cli/migrations/agent-trace-repository/001_repository_schema.sql cli/migrations/agent-trace-repository/002_repository_source_instance_id.sql` (compared against this PR's base branch/merge base, not the working tree) exits `0`; `003_mutation_trace_protocol.sql` exists.
- [ ] AC3: Revision preserves all `u64` values exactly, including `u64::MAX`.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace::store::` (revision codec round-trip test covering `0`, `1`, `i64::MAX`, `i64::MAX + 1`, `u64::MAX`).
- [ ] AC4: Worktree/scope/`EventKey`/`MutationEvent` data round-trip exactly, including full `MutationEvent` decoding (`Attribution`, `Boundary`, `active_scopes`) after the DB is closed and reopened.
  - Validate: T09's real-protocol round-trip tests, including the `load_mutation_event` cold-reload assertions for every transition that emits a `MutationEvent`.
- [ ] AC5: `AttemptState` is never persisted.
  - Validate: `grep -n mutation_trace_attempts cli/migrations/agent-trace-repository/003_mutation_trace_protocol.sql` finds nothing; `DurableTransition` has no `AttemptState` field.
- [ ] AC6: `external_taint` is never treated as DB-authoritative durable state.
  - Validate: `grep -n external_taint cli/migrations/agent-trace-repository/003_mutation_trace_protocol.sql` finds nothing; `database_failure` produces no `DurableTransition` (T05 test).
- [ ] AC7: No persistence code determines protocol semantics or attribution.
  - Validate: `DurableTransition::between` contains no boundary-kind/contention/taint conditionals (T05 done-when); inspection of `store.rs`.
- [ ] AC8: Every durable protocol transition is one `BEGIN IMMEDIATE` transaction.
  - Validate: `store.commit` routes exclusively through `execute_transactional_cas_batch` (T06/T07); T08 atomic-rollback test.
- [ ] AC9: CAS is guarded by the expected worktree revision.
  - Validate: the guard statement is `UPDATE mutation_trace_worktrees ... WHERE worktree_id = ? AND revision = ?` (T06); T08 two-writer test.
- [ ] AC10: Two writers from one revision cannot both commit.
  - Validate: T08's concurrent-writers test — two independent `RepositoryAgentTraceDb` handles/connections against the same physical database, committing concurrently from the same loaded revision — asserts exactly one `Applied` and one `Conflict`.
- [ ] AC11: Partial failure rolls back all worktree/scope/event changes.
  - Validate: T08 injected-failure test asserts revision, scope status, processed event, mutation event, and active scopes are all unchanged after rollback.
- [ ] AC12: Process restart reconstructs the same durable protocol projection.
  - Validate: T09 tests that drop and reopen the DB handle before reloading.
- [ ] AC13: Historical mutation events are not loaded on each boundary, terminal (`Closed`/`Abandoned`/`NeverSeen`) historical scopes are not loaded on each boundary unless explicitly referenced, and a referenced scope belonging to a different worktree is rejected rather than silently loaded or reassigned.
  - Validate: `MutationTraceStore::load_worktree` issues no query against `mutation_trace_events`, loads only currently `Active` scopes plus the explicitly referenced scope (if any), and returns `Err` when that referenced scope's persisted `worktree_id` does not match the requested worktree (T03 done-when).
- [ ] AC14: Existing Quint Connect and protocol tests remain green.
  - Validate: `nix flake check` (runs `cli-tests`, including `mutation_trace::mbt`, and the dedicated `mutation-trace-quint-connect` check).
- [ ] AC15: No Git/filesystem lock/hook/coordinator integration is added.
  - Validate: no `coordinator.rs` or `git_snapshot.rs` file is created; `grep -RnE "std::(fs|process)|tokio::(fs|process)" cli/src/services/mutation_trace/` shows no non-test production usage.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated` (lightweight post-task hygiene baseline; unaffected by this Rust-only change)

### Context sync

- `context/cli/mutation-trace-store.md` (new — authored by T11)
- `context/context-map.md` (add the new domain-file entry)
- `context/cli/mutation-trace-protocol.md` ("Target end-state architecture" section: `store.rs` now exists as a real database call site, while `coordinator.rs`/`git_snapshot.rs` remain future work)
- `context/overview.md` (the sentence stating the module "is not yet wired into any hook, command, or database call site" needs to reflect that a database call site now exists)
- `context/sce/shared-turso-db.md` (new generic `execute_transactional_cas_batch` primitive added to `TursoDb<M>`, alongside the existing `execute_transactional_insert_pair_if_absent`, including its CAS-conflict/retryable-failure/deterministic-failure distinction)

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/migrations/agent-trace-repository/003_mutation_trace_protocol.sql` (new); `cli/src/services/mutation_trace/store.rs` (new); a new generic `TransactionStatement`/`execute_transactional_cas_batch` primitive on `TursoDb<M>` in `cli/src/services/db/mod.rs`; tests within `mutation_trace` and `db`/`agent_trace_db`; `context/cli/mutation-trace-store.md` (new).
- **Out of scope:** Git snapshots, `GIT_INDEX_FILE`, Git object storage; the filesystem worktree lock and external-taint marker; `coordinator.rs`; real hook events and Claude/Codex/OpenCode/Pi wiring; Agent Trace diff generation; a retry-after-CAS-conflict loop; changes to Quint semantics or `protocol.rs` semantics; scope garbage collection or any deletion of terminal (`Closed`/`Abandoned`) scope rows.
- **Constraints:** `protocol.rs` stays free of SQL/DB/`RepositoryAgentTraceDb` dependencies; `DurableTransition::between` performs structural diffing only, never protocol interpretation, and rejects a structurally malformed before/after pair rather than silently accepting it; revision is stored as an 8-byte big-endian `BLOB`, enforced by `CHECK (typeof(revision) = 'blob' AND length(revision) = 8)` on every column that stores one; every durable transition commits inside exactly one `BEGIN IMMEDIATE` transaction guarded by the expected worktree revision, with a normal CAS conflict (`Ok(false)`) and a deterministic SQL/constraint failure both left unretried by the CAS primitive, while only a genuinely transient DB failure retries the whole transaction; the hot-path worktree read loads only `Active` scopes plus an explicitly referenced scope, never the full historical scope set and never `mutation_trace_events`; enum codecs are explicit (no `Debug`/serde-derived DB representation).
- **Non-goal:** do not modify `REQUIRED_REPOSITORY_SCHEMA_TABLES`'s baseline-repair logic to treat `003` as part of `001`'s metadata-repair case; do not replace or refactor the existing `execute_transactional_insert_pair_if_absent` primitive — the new generic CAS batch primitive is additive alongside it; do not change `resilience.rs`'s retry-on-any-`Err` behavior or any other caller of `run_with_retry_sync` to add this classification.

## Assumptions

- Plan slug (`mutation-cursor-store-persistence`) continues the `mutation-cursor-*` naming already used by `mutation-cursor-protocol-kernel` and `mutation-cursor-quint-connect`.
- File-backed DB round-trip tests (T09, T10) reuse the existing `std::env::temp_dir()`-based unique-path helper pattern already established in `cli/src/services/agent_trace_db/repository.rs`'s tests, rather than adding a `tempfile`-style dependency.
- T06's new CAS batch primitive coexists with the existing `execute_transactional_insert_pair_if_absent`; no other call site is migrated to it in this plan's scope.
- Updating the outdated "not yet wired into any hook, command, or database call site" framing in `context/cli/mutation-trace-protocol.md` and `context/overview.md` is handled by task context synchronization, not by a plan task, since it is a root/shared-file update rather than new content this plan's tasks author.
- AC2's validation compares the two untouched migration files against this PR's base branch (currently `quint-connect` for PR #241) or its merge base, not a hardcoded commit SHA, so the check stays correct as the branch advances.
- T06's retryable-vs-deterministic classification is implemented locally to `execute_transactional_cas_batch` — for example, by having its retried closure return a classified outcome that `run_with_retry_sync` still sees as `Ok` (so it never retries a deterministic failure), with the caller re-raising that failure as an `Err` after the closure returns — rather than by changing `resilience.rs` itself.

## Task stack

- [x] T01: `Add migration 003 for mutation-trace protocol tables` (status:done)
  - Task ID: T01
  - Scope: In — `cli/migrations/agent-trace-repository/003_mutation_trace_protocol.sql` defining `mutation_trace_worktrees` (revision `BLOB` constrained by `CHECK (typeof(revision) = 'blob' AND length(revision) = 8)`), `mutation_trace_scopes` (+ `idx_mutation_trace_scopes_worktree` and a new composite `idx_mutation_trace_scopes_worktree_status` index on `(worktree_id, status)` for the bounded hot-path scope lookup), `mutation_trace_processed_events`, `mutation_trace_events` (+ the same `typeof`/`length` revision `CHECK`, plus payload-consistency `CHECK` constraints), and `mutation_trace_event_active_scopes`. Out — any Rust code consuming these tables (T02+).
  - Dependencies: none
  - Done when: a fresh `RepositoryAgentTraceDb::new_at` at a clean path applies `001`+`002`+`003` and all five tables exist with the specified columns, constraints, and indexes; a row violating a `CHECK` constraint (for example `ai_exclusive` attribution with a `NULL` `attribution_scope_id`) is rejected; a `TEXT` value of length 8 assigned to a `revision` column is rejected by the `typeof(revision) = 'blob'` check even though its length matches.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db::repository::`; a new targeted test asserting the `003` tables, indexes, and constraints (including the TEXT-vs-BLOB revision case) behave as specified.
  - Completed: 2026-08-27
  - Files changed: `cli/migrations/agent-trace-repository/003_mutation_trace_protocol.sql` (new); `cli/src/services/agent_trace_db/repository.rs`
  - Result: Added migration `003_mutation_trace_protocol.sql` defining `mutation_trace_worktrees`, `mutation_trace_scopes` (+ `idx_mutation_trace_scopes_worktree`, `idx_mutation_trace_scopes_worktree_status`), `mutation_trace_processed_events` (+ `idx_mutation_trace_processed_events_worktree`), `mutation_trace_events`, and `mutation_trace_event_active_scopes`, all discovered automatically by `build.rs`'s directory scan. Revision columns use `BLOB NOT NULL CHECK (typeof(revision) = 'blob' AND length(revision) = 8)`; enum-shaped columns use `TEXT` with `CHECK (... IN (...))` allow-lists following the existing `role`/`payload_type` convention; `mutation_trace_events` additionally enforces attribution/boundary payload-consistency `CHECK`s (`ai_exclusive` requires a non-null `attribution_scope_id`; hook boundaries require non-null `boundary_scope_id`/`boundary_event_id`, `flush` requires both null). Updated `open_at_initializes_the_full_schema_from_one_migration` to assert the new migration ID and the five new tables/indexes, and added two new targeted tests (`mutation_trace_worktrees_revision_must_be_a_blob_not_matching_length_text`, `mutation_trace_events_ai_exclusive_attribution_requires_a_scope_id`) proving the TEXT-vs-BLOB revision rejection and the `ai_exclusive`-requires-scope rejection, each paired with a positive control insert.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db::repository::` — passed, 18/18 (including the two new tests and the updated baseline-schema test); `./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check` — passed (no diff).
  - Done checks: fresh DB applies `001`+`002`+`003` with all five tables/indexes present (verified by the updated baseline test); `ai_exclusive` attribution with a `NULL` `attribution_scope_id` is rejected (verified); an 8-byte TEXT value assigned to `revision` is rejected by `typeof(revision) = 'blob'` (verified); `git diff --exit-code` on `001`/`002` shows zero changes (verified).
  - Context impact: local — additive schema-only migration; no Rust code consumes these new tables yet (T02+ wire codecs, loads, and commits against them). No durable context synchronization is required for this task; the plan's `Context sync` entries are authored by T11 once the full store lands.
  - Context synchronization: synced

- [ ] T02: `Add revision and enum domain<->SQL codecs` (status:todo)
  - Task ID: T02
  - Scope: In — create `cli/src/services/mutation_trace/store.rs` with `encode_revision`/`decode_revision` (`u64` <-> 8-byte big-endian `BLOB`) and explicit codecs for `ActorKind`, `FailureKind`, `ScopeStatus`, `Attribution`'s discriminant, and `Boundary`'s discriminant. Out — any query, projection, or commit logic (T03+).
  - Dependencies: T01
  - Done when: `encode_revision`/`decode_revision` round-trip exactly for `0`, `1`, `i64::MAX`, `i64::MAX + 1`, and `u64::MAX`; every enum variant round-trips through its codec; no codec relies on `Debug` formatting or implicit serde representation.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace::store::`
  - Context synchronization: pending

- [ ] T03: `Add bounded WorktreeProjection load and cold-path MutationEvent read` (status:todo)
  - Task ID: T03
  - Scope: In — `WorktreeProjection` (+ `into_protocol_state`) and `MutationTraceStore` wrapping `&RepositoryAgentTraceDb`. `load_worktree(worktree: &WorktreeId, scope: Option<&ScopeId>, event_key: Option<&EventKey>)` loads exactly one worktree row, only its currently `Active` scopes plus the explicitly referenced `scope` when supplied (even when that scope is `NeverSeen`/`Closed`/`Abandoned`), and 0 or 1 matching processed-event row for `event_key`. A separate cold-path `load_mutation_event(worktree: &WorktreeId, revision: u64) -> Result<Option<MutationEvent>>` reads one `mutation_trace_events` row plus its `mutation_trace_event_active_scopes` rows and reconstructs a complete `MutationEvent`, decoding `Attribution` exactly (including `AiExclusive(scope_id)`) and the complete `Boundary`. When `scope` is supplied and the persisted `ScopeState` for that `ScopeId` has a `worktree_id` different from the requested `worktree`, `load_worktree` returns `Err` — it never silently omits the scope, never includes it in the projection, and never reassigns it to the requested worktree, preserving the permanent `ScopeId` -> `WorktreeId` identity `register_scope` already enforces. Out — initialization/commit logic (T04/T07); calling `load_mutation_event` from `load_worktree` or from any hook-boundary path.
  - Dependencies: T01, T02
  - Done when: `load_worktree` returns `None` for a missing worktree and `Some(projection)` otherwise, with `scopes` containing every currently `Active` scope on that worktree plus `scope` when supplied regardless of its status, and never a `Closed`/`Abandoned`/`NeverSeen` scope that was not explicitly referenced; `attempts`, `mutation_events`, and `external_taint` stay empty; the method issues no query against `mutation_trace_events`. Specifically: a referenced scope on the requested worktree is included regardless of status; a referenced terminal (`Closed`/`Abandoned`/`NeverSeen`) scope on the requested worktree is included; an unreferenced terminal historical scope is excluded; a referenced scope whose persisted `worktree_id` belongs to a different worktree returns `Err`. `load_mutation_event` returns `None` when no row exists at that `(worktree, revision)` and otherwise reconstructs a `MutationEvent` whose `before_tree`/`after_tree`/`revision`/`tainted`/`failure_kind`/`attribution`/`boundary`/`active_scopes` exactly match what `store.commit` persisted.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace::store::`
  - Context synchronization: pending

- [ ] T04: `Add worktree/scope initialization operations` (status:todo)
  - Task ID: T04
  - Scope: In — `initialize_worktree(worktree_id, initial_tree)` and `register_scope(scope_id, worktree_id, actor_kind)` on `MutationTraceStore`. Out — the CAS commit path (T06/T07).
  - Dependencies: T03
  - Done when: `initialize_worktree` inserts `revision=0`/healthy/not-tainted/not-needs-rebaseline only when the worktree is missing and never overwrites an existing cursor; `register_scope` inserts `NeverSeen` when missing, returns the existing state when worktree+actor match, and errors on a worktree or actor mismatch for an existing `scope_id`.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace::store::`
  - Context synchronization: pending

- [ ] T05: `Add DurableTransition structural diff type` (status:todo)
  - Task ID: T05
  - Scope: In — `DurableTransition` and `DurableTransition::between(before, after, worktree) -> Result<Option<Self>>` performing pure structural diffing only, enforcing: the target worktree exists in both `before` and `after` and is never added or removed; no unrelated worktree changes; when a durable transition exists, its worktree's next revision is exactly `expected_revision + 1` computed via checked `u64` arithmetic; no scope is added or deleted; a changed scope belongs to the target worktree; `ScopeState.worktree_id` and `ScopeState.actor_kind` never change (only `status` may); `processed_events` may only gain entries, never lose them, with at most one new entry whose scope belongs to the target worktree; `mutation_events` may only gain entries, never lose them, with at most one new entry belonging to the target worktree; `AttemptState`/`external_taint` differences are ignored. Out — SQL/DB code (T06/T07).
  - Dependencies: T02
  - Done when: `between()` returns `Ok(None)` for a `database_failure`-only transition and for a no-change `Flush`; returns `Ok(Some(..))` with the correct shape for `Start`/`Advance`/`Close`, `taint`, `abandon`, and `recover` transitions exercised directly against `protocol::*` outputs; the function contains no boundary-kind, contention, or taint conditionals; it returns `Err` for a malformed `before`/`after` pair covering at least: an `actor_kind` change, a scope's `worktree_id` change, a processed `EventKey` disappearing, a `MutationEvent` disappearing, an unrelated worktree changing, a revision jump by more than 1, a revision decrease, and a scope unexpectedly appearing or disappearing.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace::store::`
  - Context synchronization: pending

- [ ] T06: `Add generic transactional CAS batch primitive to TursoDb` (status:todo)
  - Task ID: T06
  - Scope: In — `TransactionStatement` and `TursoDb::execute_transactional_cas_batch(operation_name, retry_hint, guard, statements)` in `cli/src/services/db/mod.rs`, with a retryability contract distinct from the shared `run_with_retry_sync` helper's plain any-`Err`-retries behavior: a guard affecting 0 rows commits as a no-op and returns `Ok(false)` (a normal CAS conflict) without running any statement and without being retried; a guard affecting 1 row runs every statement inside the same `BEGIN IMMEDIATE` transaction and returns `Ok(true)`; a retryable DB failure (lock/busy/other transient condition) retries the entire transaction from `BEGIN IMMEDIATE`; a deterministic failure (SQL/schema/constraint/invariant violation) returns `Err` without being retried. This adds the minimum local retryability classification needed for that behavior — for example, the retried closure returns a classified outcome that `run_with_retry_sync` still treats as `Ok` so it never retries a deterministic failure, and the caller re-raises that failure as `Err` once the closure returns — without changing `resilience.rs` or any other caller of `run_with_retry_sync`. Out — mutation-trace-specific SQL (T07).
  - Dependencies: none
  - Done when: a guard affecting 0 rows commits as a no-op and returns `Ok(false)` without running any statement or waiting for a retry backoff; a guard affecting 1 row runs every statement and returns `Ok(true)`; an injected deterministic mid-batch failure rolls back the entire transaction (including the guard's own effect) and surfaces as `Err` after exactly one attempt, never reported as a CAS conflict; an injected retryable DB failure retries the whole transaction from `BEGIN IMMEDIATE` (never individual statements) up to the configured attempt count.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml db::`
  - Context synchronization: pending

- [ ] T07: `Implement MutationTraceStore::commit` (status:todo)
  - Task ID: T07
  - Scope: In — `CasResult` and `MutationTraceStore::commit(transition)`, translating a `DurableTransition` into the worktree CAS `UPDATE` plus scope `UPDATE`s plus processed-event `INSERT` plus mutation-event `INSERT` plus active-scope `INSERT`s, via `execute_transactional_cas_batch`. Out — concurrency/rollback/round-trip test coverage (T08/T09).
  - Dependencies: T04, T05, T06
  - Done when: `commit()` returns `CasResult::Applied` with every included write visible when the worktree's on-disk revision matches `expected_revision`, and `CasResult::Conflict` with no visible write otherwise; a deterministic failure surfaced by `execute_transactional_cas_batch` propagates out of `commit()` as an `Err`, never as `CasResult::Conflict`.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace::store::`
  - Context synchronization: pending

- [ ] T08: `Add CAS and concurrency test coverage for store.commit` (status:todo)
  - Task ID: T08
  - Scope: In — tests for: two writers committing from the same revision against one physical repository-scoped `agent-trace.db`, using two independent `RepositoryAgentTraceDb` handles/connections opened against that same database file (one `MutationTraceStore` per handle), with both writers loading worktree revision `N` before either commits and executing their commits from separate threads (or an equivalent that exercises two independent DB connections rather than one handle invoked twice in sequence) — exactly one result `CasResult::Applied`, the other `CasResult::Conflict`; atomic rollback on an injected deterministic mid-transaction failure; `u64::MAX` round-trip through the real DB; `(scope_id, event_id)` replay-uniqueness rejection; strong recovery (all active scopes abandoned) and needs-only recovery (surviving active scopes stay active). Out — production code changes beyond what T07 already provides; process-spawning or other multiprocess test infrastructure (two independent DB handles on separate threads are sufficient for this PR).
  - Dependencies: T07
  - Done when: all five scenarios above are covered by passing tests; the two-writer test is not satisfied by calling `commit` twice sequentially through one shared `RepositoryAgentTraceDb` handle; after both commits, reopening the database shows the worktree revision advanced exactly once and only the winning transition's durable effects (scope status, processed event, mutation event, active scopes) are present; the atomic-rollback test observes revision, scope status, processed event, mutation event, and active scopes all unchanged after the injected failure.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace::store::`
  - Context synchronization: pending

- [ ] T09: `Add real-protocol round-trip persistence tests` (status:todo)
  - Task ID: T09
  - Scope: In — tests driving load (`load_worktree`, bounded to `Active` scopes plus the transition's referenced scope) -> `protocol::prepare`/`commit` (or `taint`/`database_failure`/`abandon`/`recover`) -> `DurableTransition::between` -> `store.commit` -> drop DB handle -> reopen -> reload, for `Start`, `Advance`, `Close`, `Flush` with change, `Flush` without change, taint, abandon, recover, contended mutation, and a replayed `EventKey`. For every transition that emits a `MutationEvent`, additionally reload it after reopening with `load_mutation_event(worktree, revision)` and compare it field-for-field (including exact `Attribution`/`Boundary` decoding) against the `MutationEvent` the original protocol transition produced. Out — new production code, unless a genuine T01-T07 gap surfaces.
  - Dependencies: T07
  - Done when: for every listed transition, the reloaded worktree/scope projection after reopening the DB matches the durable projection produced by the original protocol transition, and for every transition that emits a `MutationEvent`, `load_mutation_event` after reopening reconstructs it exactly.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace::store::`
  - Context synchronization: pending

- [ ] T10: `Add migration and lifecycle tests for migration 003` (status:todo)
  - Task ID: T10
  - Scope: In — tests proving a fresh DB applies `001`+`002`+`003`; an existing `001`+`002`-only DB gets `003` applied through the `sce setup`/lifecycle path; the no-migration hook-runtime path does not apply `003` and still reports the existing "Run 'sce setup'." guidance when schema is incomplete. Out — changes to `REQUIRED_REPOSITORY_SCHEMA_TABLES` baseline-repair semantics.
  - Dependencies: T01
  - Done when: all three scenarios pass without modifying the baseline-repair function's treatment of `001` metadata.
  - Verify: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db::`
  - Context synchronization: pending

- [ ] T11: `Document the mutation-trace store` (status:todo)
  - Task ID: T11
  - Scope: In — `context/cli/mutation-trace-store.md` covering repository-DB ownership, `WorktreeId` as the persistence partition, the 8-byte big-endian revision encoding, `AttemptState`/`external_taint` non-persistence, and the store's non-goals (no Git I/O, no attribution decisions, no retry-after-`Conflict`); a `context/context-map.md` entry for the new file. Out — edits to any other existing `context/` file (left to task context synchronization).
  - Dependencies: T01-T10
  - Done when: the new file exists, is linked from `context/context-map.md`, and every claim in it is checked against the code produced by T01-T10.
  - Verify: manual inspection cross-referencing the file's claims against `store.rs`, the migration, and `db/mod.rs`.
  - Context synchronization: pending

## Open questions

None. The change request already resolves every architectural decision (schema shape, CAS mechanics, which fields are excluded from persistence) precisely, and each decision checks out against the current `protocol.rs`/`types.rs` domain model and the existing Turso adapter conventions verified while authoring this plan.
