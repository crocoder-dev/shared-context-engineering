# Snapshot-ref maintenance state and verified-existing DB opener

Supporting contracts for [`mutation-trace-ref-reconciliation.md`](mutation-trace-ref-reconciliation.md): the durable per-worktree maintenance state file shared by the hook advisory, `sce doctor`, and `reconcile_explicit`, and the dedicated Agent Trace DB opener reconciliation uses. Related: [`mutation-scope-ref-advisory.md`](mutation-scope-ref-advisory.md), [`doctor-ref-reconciliation.md`](doctor-ref-reconciliation.md), [`agent-trace-storage.md`](agent-trace-storage.md).

## Verified-existing Agent Trace DB opener

Reconciliation never uses the hook opener
(`open_agent_trace_db_for_hook_runtime`), which can create directories and files,
repair migration metadata, and insert repository metadata. The dedicated path is
`resolve_existing_agent_trace_storage_for_maintenance` plus
`RepositoryAgentTraceDb::open_verified_existing_at`, built on
`TursoDb::open_existing_without_migrations_at`:

- The Turso open is non-creating and read-only (`read_only(true)` with
  `experimental_multiprocess_wal(true)` preserved), so existence and open are one
  OS call. A failed open is classified `Missing` only when the path is then
  absent. No pre-existence check and no parent-directory creation.
- It never runs migrations, `repair_missing_repository_schema_migration_metadata`
  or `verify_or_initialize_repository_metadata`.
- Before any ref listing it verifies schema readiness (all expected migrations),
  metadata presence, stored `repository_id` against the resolved identity, and a
  valid `source_instance_id` (`verify_existing_repository_metadata`).
- Typed failures `ExistingRepositoryDbError`: `Missing`, `Unreadable`,
  `IncompatibleSchema`, `MissingMetadata`, `RepositoryMismatch`. They stay
  downcastable from `ReconcileError::AgentTraceDbUnavailable`.

Per-DB-state guarantees: a **missing** DB creates no DB file, parent directory,
WAL or shared-memory file and no ref is listed or deleted; an **existing invalid**
DB may incur incidental Turso WAL/shared-memory bookkeeping but no migration,
repair, initialization, or application-table write and no ref deletion; an
**existing valid** DB is verified before reconciliation with no initialization.
Logical immutability is the guarantee, not filesystem byte-identity.

Limitations: Turso opens `-wal`/`-tshm` sidecars by path, so only a coherent
replacement of main file and sidecars is meaningful; the process-wide Turso
registry may share an already-open instance (the read-only flag is then not
enforced); a DB replaced between verification and the delete transaction is not
detected, the guard being that roots are read from the verified connection under
the held lock.

Plain `sce doctor` shares the non-creating read-only open through
`RepositoryAgentTraceDb::open_existing_schema_ready_at`, which inspects schema
readiness without requiring metadata.

## Maintenance state file

`<git-dir>/sce/ref-maintenance.json`, beside `mutation-cursor.lock` and the
taint marker. Versioned, capped at `MAINTENANCE_STATE_MAX_BYTES` (4096), written
only under `WorktreeLock` by atomic staging-and-swap (temp file in the same
directory, rename, best-effort parent-directory sync; the filesystem is the
statically dispatched `StateFilesystem` seam). No in-memory registry or
process-global state; the wall clock is an injected `Fn() -> i64` (unix ms).

Fields: past-event timestamps `anchor`, `last_success`, `last_attempt`,
`last_advised`; `last_attempt_outcome` (`completed` | `failed`); `last_report`
counts; `consecutive_failures`; `last_failure`. **No field is a deadline.**
Eligibility is computed at read time as `now − reference ≥ window`, so rereading
cannot extend it. Constants: `RECONCILIATION_ADVISORY_AFTER` 24 h,
`FUTURE_SKEW_TOLERANCE` 5 min.

Transitions: explicit success sets `last_attempt` = `last_success` = now, stores
the report, and clears the failure streak and fields; explicit failure sets
`last_attempt`, outcome `failed`, increments `consecutive_failures`, and leaves
`last_success` untouched; lock contention changes nothing. A missing, unparsable,
wrong-version, or oversized file is treated as default and rewritten by the next
recorded operation; it never fails an explicit pass.

Timestamps: a value strictly more than `FUTURE_SKEW_TOLERANCE` ahead of `now` is
invalid; one within the tolerance (inclusive) is valid, preserved byte-for-byte,
and evaluated as age 0. **Successful state-writing operations** (a successful
advisory write, a successful explicit record) normalize invalid timestamps once
under the lock and persist them, so afterwards every stored timestamp satisfies
`t ≤ now + FUTURE_SKEW_TOLERANCE`. **Read-only or unsuccessful operations**
(plain `sce doctor`, lock-busy passes, unreadable state, a `NotApplied` write, an
interrupted write) never mutate the file, evaluate invalid timestamps
conservatively as eligible/recommended, and never report them as normalized.

`evaluate_recommendation(state, now)` is the pure function shared by the advisory
and read-only doctor. An outstanding failed explicit pass is recommended
independently of staleness and of `last_advised`.

