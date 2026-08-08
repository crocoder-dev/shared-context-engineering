# Plan: incremental-agent-trace-etl-transactional-watermarks

## Change summary

Add the first production ETL slice between the repository-scoped multiprocess-WAL `agent-trace.db` source and the lock-owned `agent-trace-sync.db` DWH replica. The new `AgentTraceEtl` service incrementally extracts bounded `agent_traces.id` batches in short source read transactions, preserves and hashes `trace_json`, and loads facts plus the per-repository/per-source-instance/per-table watermark in one destination transaction.

This extends the source identity, DWH schema, and `AgentTraceDwhReplica` boundaries established by the preceding work. It proves replay, idempotency, integrity-conflict, contention, crash, and reconstruction semantics for `agent_traces` only, while leaving pull/push, credentials, CLI orchestration, and all other source tables outside the ETL layer.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [ ] AC1: `AgentTraceEtl` exposes a CLI-independent run API that accepts an open repository source and lock-owning `AgentTraceDwhReplica`, validates the requested repository through `RepositoryAgentTraceDb` metadata, obtains the stored `source_instance_id` from that metadata, uses `agent_traces` as the source table, and never acquires credentials or invokes replica `pull()`/`push()`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_etl_api`
- [ ] AC2: extraction starts from `0` when the `(repository_id, source_instance_id, agent_traces)` watermark is missing, repeatedly executes `WHERE id > ? ORDER BY id LIMIT ?`, advances only to the last row actually extracted and loaded, honors a bounded batch size, and makes a repeated run with no new source rows a no-op.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_etl_incremental`
- [ ] AC3: each source batch is copied into explicit Rust-owned `SourceAgentTrace` values within a short consistent read transaction that ends before hashing or destination work; concurrent source writers can continue, rows acknowledged after a snapshot are picked up by a later batch or run, and only typed/transient Busy or database-locked contention is retried with bounded backoff plus best-effort rollback before every new `BEGIN`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_etl_source`
- [ ] AC4: every loaded Agent Trace preserves `trace_json` byte-for-byte, stores lowercase hexadecimal SHA-256 of those exact bytes, and preserves the requested commit, URL, and nullable remote URL fields.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_etl_transform`
- [ ] AC5: each destination batch idempotently ensures repository and source-instance dimensions; inserts a missing `(repository_id, agent_trace_id)` fact; counts an existing equal-hash fact as already present without duplication; and fails loudly with both hashes on an unequal-hash integrity conflict, including when the conflicting fact came from another source instance.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_etl_identity`
- [ ] AC6: destination fact writes and watermark advancement for a batch commit in one local DWH transaction, so any insert, verification, injected processing, or commit failure leaves no partial batch and does not advance the watermark; a clean rerun replays the complete failed batch.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_etl_atomic`
- [ ] AC7: tests prove initial extraction, incremental growth, batch boundaries, watermark-behind replay, independent source-instance watermarks, same-logical-trace behavior across source instances, the crash/reconstruction replay model, and the invariant that every source row between old and committed new watermarks exists in the DWH with matching content and hash.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_etl_watermark_never_skips`
- [ ] AC8: durable architecture context describes `agent-trace.db` as source of truth, `AgentTraceEtl` as the incremental deterministic bridge, `agent-trace-sync.db` as the durable local ETL commit boundary and reconstructible sync replica, the remote DWH as the aggregated synchronized warehouse, and explicitly records both transaction invariants and pull/push separation.
  - Validate: inspect `context/architecture.md`, `context/overview.md`, `context/glossary.md`, `context/sce/agent-trace-db.md`, `context/sce/agent-trace-dwh-db.md`, `context/sce/agent-trace-dwh-replica.md`, and the new ETL domain context against the implemented API and tests.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- Update `context/overview.md`, `context/architecture.md`, and `context/glossary.md` for the new ETL boundary and transaction terminology.
- Add an Agent Trace ETL domain file under `context/sce/`, index it from `context/context-map.md`, and update `context/sce/agent-trace-db.md`, `context/sce/agent-trace-dwh-db.md`, `context/sce/agent-trace-dwh-replica.md`, and `context/sce/shared-turso-db.md` to current behavior.
- Repair the stale `context/sce/shared-turso-db.md` statement that hook runtime retains lazy schema initialization; code and the source DB context show the hook path is no-migration/readiness-only.

## Constraints and non-goals

- **In scope:** a minimal transaction seam in `cli/src/services/db/`; source extraction and Busy retry for repository `agent_traces`; transformation and SHA-256 hashing; transactional DWH dimension/fact/watermark loading through `AgentTraceDwhReplica`; ETL stats/configuration; focused database/concurrency/failure tests; and architecture documentation.
- **Out of scope:** messages, message parts, diff trace/code-change or post-commit ETL; generic reprocessing; control-plane calls; DWH provisioning; credential retrieval or auth changes; CLI/lifecycle/doctor/hook orchestration; `sce trace sync`; background or hook-triggered synchronization; archive/search/FTS/analytics behavior.
- **Constraints:** do not change the live source write path; do not hold a source read transaction during hashing, destination writes, or network work; do not use timestamps or a separately queried `MAX(id)` as the cursor; do not store cursor state outside synced DWH `etl_watermarks`; do not enable multiprocess WAL on `agent-trace-sync.db`; keep the bridge lock scoped to the destination replica; and use the existing `sha2` dependency with no new hashing crate.
- **Non-goal:** do not introduce a broad generic ETL trait hierarchy or redesign the full database abstraction; reusable primitives are limited to transaction execution, watermark identity, batch accounting, and source contention handling needed by this first table.

## Assumptions

- The production ETL entrypoint accepts `repository_id` but obtains `source_instance_id` by calling the existing `RepositoryAgentTraceDb::verify_or_initialize_repository_metadata(repository_id)` API; it does not accept or derive a caller-selected source identity.
- The default batch size is `500`, with a validated internal/test configuration seam for smaller batches.
- The minimal `TursoDb` transaction API is a synchronous closure over a transaction-scoped execute/query surface, commits only after the closure succeeds, and explicitly rolls back on closure failure; raw `turso::Transaction` does not escape the DB module.
- Production ETL accepts `&AgentTraceDwhReplica`; domain load logic may operate on its lock-lifetime-bound `AgentTraceDwhDb` internally so local DWH fixtures can test correctness without a remote server, while no production caller bypasses replica ownership.
- Source contention classification prefers `turso::Error::Busy` and narrowly recognizes the SDK's database-locked form when typed classification is unavailable; arbitrary extraction, mapping, integrity, or destination errors are never retried.
- Failure-path coverage uses a test-only injection seam in the destination batch loader rather than adding a production failure mode.

## Task stack

- [x] T01: `Add a minimal transactional TursoDb operation seam` (status:done)
  - Task ID: T01
  - Goal: Let a domain service perform multiple parameterized reads/writes and one explicit commit on the existing connection without leaking raw Turso transaction details.
  - Boundaries (in/out of scope): In — `TursoConnectionCore`/`TursoDb` transaction-scoped execute and fully fetched query helpers, closure error rollback, commit handling, and focused commit/rollback tests usable by Sync-backed DWH connections. Out — nested transactions, savepoints, encrypted DB transactions, transaction retries, schema migration changes, and ETL behavior.
  - Dependencies: none
  - Done when: one closure can read and write through a typed transaction handle; success commits; any closure error rolls back all writes; commit/rollback errors retain database context; the API works for `AgentTraceDwhDb` created through the existing connection wrapper; and targeted tests pass.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml turso_transaction`; `nix develop -c sh -c 'cd cli && cargo fmt'`.
  - Evidence: Added `TursoTransaction<'a, M: DbSpec>` (transaction-scoped `execute`/`query_map`, no retry) and `TursoDb::<M>::transaction` (`BEGIN IMMEDIATE` / commit-on-`Ok` / best-effort `ROLLBACK` on closure error or failed commit, error messages carry `M::db_name()` context) in `cli/src/services/db/mod.rs`. No raw `turso::Transaction` escapes the module. Added `turso_transaction_tests` covering closure commit visibility, closure-error rollback, database-context error messages, and use against `AgentTraceDwhDb` opened through the existing `new_at` wrapper. No deviations from plan assumptions.
  - Verification run: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml turso_transaction` — 4 passed. `nix develop -c sh -c 'cd cli && cargo fmt'` — applied, tests rerun and still pass.

- [x] T02: `Implement short incremental Agent Trace source extraction with Busy retry` (status:done)
  - Task ID: T02
  - Goal: Extract one bounded, ordered `agent_traces.id` batch into owned source models from a consistent source snapshot without interfering with concurrent hook writers.
  - Boundaries (in/out of scope): In — an `agent_trace_etl` service module, `SourceAgentTrace`, exact projection/mapping, `id > watermark ORDER BY id LIMIT batch_size`, explicit short source read transaction, source metadata lookup, bounded Busy/database-locked-only retry, best-effort rollback before each `BEGIN`, and extraction/contention/concurrent-writer tests. Out — hashing, DWH writes, watermark mutation, destination identity handling, pull/push, and other source tables.
  - Dependencies: T01
  - Done when: extraction returns owned rows in ascending source-ID order; commits/releases its read transaction before return; empty and partial batches behave correctly; metadata supplies the stable source-instance identity; transient contention retries and recovers from stale failed transactions; non-contention errors fail immediately; independent writers continue and later acknowledged rows remain extractable; and targeted tests pass.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_etl_source`; `nix develop -c sh -c 'cd cli && cargo fmt'`.
  - Evidence: Added `cli/src/services/agent_trace_etl/mod.rs` with `SourceAgentTrace`, `extract_agent_trace_batch(db, watermark, batch_size)` (`id > ?1 ORDER BY id ASC LIMIT ?2`), and `run_with_source_contention_retry` (bounded backoff, classifies only Turso `Busy`/"database is locked"/"table is locked" text as retryable, rollback before each retried `BEGIN`, everything else fails on the first attempt). Registered the module in `cli/src/services/mod.rs`. Added `TursoDb::read_transaction` in `cli/src/services/db/mod.rs` (plain `BEGIN`, not `BEGIN IMMEDIATE`, refactored alongside the existing `transaction` through a shared `run_transaction` helper) so source reads never reserve the source database's write lock; widened `rollback_best_effort` to `pub(crate)` and `RetryPolicy::backoff_for_attempt` (`cli/src/services/resilience.rs`) to `pub(crate)` for reuse by the ETL retry loop. Metadata lookup for the stable source-instance identity reuses the existing `RepositoryAgentTraceDb::verify_or_initialize_repository_metadata` API directly (no new wrapper). 10 new tests in `agent_trace_etl_source_tests` cover ascending/bounded/partial batches, no-op reruns, later-batch/later-run visibility of rows written after a snapshot, a real concurrent-writer-not-blocked integration test (two connections, one holding an open read transaction while the other inserts), retry/rollback/classification unit tests, and batch-size validation. No deviations from plan assumptions.
  - Verification run: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_etl_source` — 10 passed. `nix develop -c sh -c 'cd cli && cargo fmt'` — applied (reformatted the new module and the `db/mod.rs` refactor). Full suite `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` — 232 passed, 0 failed, 1 ignored (pre-existing).

- [ ] T03: `Load transformed Agent Traces and watermarks atomically` (status:todo)
  - Task ID: T03
  - Goal: Implement the deterministic transform and one-batch destination transaction that ensures lineage, inserts or verifies logical traces, and advances the exact extracted watermark.
  - Boundaries (in/out of scope): In — verbatim `trace_json` transformation, lowercase SHA-256, typed destination model/accounting, watermark read/upsert for `agent_traces`, idempotent repository/source dimensions, explicit same-hash replay versus different-hash conflict, atomic fact-plus-watermark load, and test-only destination failure injection. Out — the multi-batch public run loop, source retry behavior, transport synchronization, credentials, and non-Agent-Trace facts.
  - Dependencies: T01, T02
  - Done when: a batch transaction inserts missing facts, verifies existing equal hashes, rejects unequal hashes with repository/trace/existing/incoming details, advances only to the batch's last source ID after every row was loaded or verified, and rolls back facts/dimensions/watermark together on any failure; transformation and atomic failure/replay tests pass.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_etl_transform`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_etl_atomic`; `nix develop -c sh -c 'cd cli && cargo fmt'`.

- [ ] T04: `Complete the AgentTraceEtl run loop, invariant tests, and architecture contract` (status:todo)
  - Task ID: T04
  - Goal: Expose the production ETL API through `AgentTraceDwhReplica`, prove end-to-end batching/replay/crash/source-lineage behavior, and record the resulting architecture for future table ETLs.
  - Boundaries (in/out of scope): In — configurable/default batch loop and `AgentTraceEtlStats`, replica-owned public API, initial/no-op/growth/batch-boundary tests, behind-watermark replay, two-source-instance tests, integrity conflict rollback, destination failure rerun, source-write/contention integration coverage, focused watermark-never-skips assertion, reconstruction/crash-semantics coverage, and requested durable context updates. Out — invoking pull/push, credential acquisition, CLI wiring, remote orchestration, and messages/parts/code-change implementations.
  - Dependencies: T01, T02, T03
  - Done when: successful runs report extracted/inserted/already-present/batch counts and before/after watermarks; every requested scenario passes; no path can commit watermark `N` without loading or matching every extracted row through `N`; independently keyed source watermarks and cross-source logical identity behave as specified; ETL contains no pull/push or credential logic; and durable context states the local transaction and short-source-transaction invariants plus constraints relevant to future message/part ETLs.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_etl`; `nix develop -c sh -c 'cd cli && cargo fmt'`; inspect the ETL module for absence of `pull`, `push`, auth-token, and CLI dependencies.

## Open questions

None. The request already narrows the smallest useful proof to `agent_traces`, fixes the identity and transaction semantics, and explicitly defers orchestration and additional tables.
