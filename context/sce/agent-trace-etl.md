# Agent Trace ETL

`cli/src/services/agent_trace_etl/mod.rs` owns the shared mechanics and the table ETL slices between the repository-scoped multiprocess-WAL `agent-trace.db` source and the lock-owned `agent-trace-sync.db` DWH replica. `AgentTraceEtl` accepts an open `RepositoryAgentTraceDb`, a repository ID, and an `AgentTraceDwhReplica`; it verifies source metadata, obtains the stored `source_instance_id`, and runs the `agent_traces` table. The sibling `conversation_messages_etl` and `conversation_parts_etl` modules apply the same mechanics to logical `messages` and source-lineage-scoped `parts` rows, each with its own table watermark. None of these pipelines acquires credentials, invokes `pull()`/`push()`, or depends on CLI orchestration.

## Incremental run contract

`AgentTraceEtl::default()` uses a bounded batch size of 500. `with_batch_size()` provides a validated test/configuration seam. A run reads the `(repository_id, source_instance_id, agent_traces)` watermark, extracts rows with `id > watermark ORDER BY id ASC LIMIT batch_size`, and repeats until the source returns an empty batch. The cursor advances only to the last row successfully inserted or verified by the destination transaction. A no-new-rows run is a no-op.

`AgentTraceEtlStats` reports extracted, inserted, already-present, and batch counts plus before/after watermarks. Every source row between the old and committed new watermark is therefore loaded or matched before progress is persisted. A failed destination batch leaves facts, dimensions, and watermark unchanged, so a later run replays the complete batch. Equal-hash logical replays are counted as already present; an unequal hash is an integrity conflict, including when it came from another source instance.

## Transaction and lineage invariants

Each source batch is copied into owned `SourceAgentTrace` values in a short plain-`BEGIN` read transaction. The snapshot ends before transformation, hashing, or destination work, and only Busy/database-locked contention receives bounded retry with best-effort rollback before a new `BEGIN`. Concurrent source writers are not held behind ETL extraction.

Each destination batch uses one `BEGIN IMMEDIATE` transaction for repository/source dimensions, Agent Trace facts, and watermark advancement. `trace_json` is preserved byte-for-byte and its exact UTF-8 bytes receive lowercase SHA-256 hashing. Logical fact identity is `(repository_id, agent_trace_id)` while watermark identity independently includes `source_instance_id`, allowing independent source lineages to share logical facts without duplicating them.

## Replica and reconstruction boundary

`AgentTraceDwhReplica` remains the sole owner of the sync connection and bridge lock. Its `run_agent_trace_etl()` method delegates to `AgentTraceEtl` while preserving that ownership; `ConversationEtl` composes the message and part runners through the same lock-owned destination without acquiring credentials or invoking transport. ETL never pulls or pushes remote state. The local sync database is a durable transaction/replay boundary and is reconstructible from the remote DWH: crash or local-file loss is handled by reopening/replaying source rows and, when required, remote reconstruction. The messages and parts runners preserve the same short-source-transaction, exact-cursor, and local fact-plus-watermark invariants; their synchronized source fields are append-only/immutable for ETL purposes. Source audit found no production `UPDATE` of those fields beyond the schema-maintenance `updated_at` triggers, so update CDC remains deliberately out of scope.

The code-change ETL foundation now includes destination-independent `diff_traces` extraction and transformation in `cli/src/services/code_changes_etl.rs`. It copies the exact ordered integer-ID projection in a short read snapshot, then strictly normalizes `patch` and `structured` payloads through the canonical parser, preserves source metadata, counts `ParsedPatch` files and touched-line kinds with checked destination-sized metrics, and hashes the original payload bytes with lowercase SHA-256. Destination identity, watermark advancement, and public runner orchestration remain future work.

## Source contention retry

A private `run_with_source_contention_retry` wraps each extraction attempt with a bounded backoff policy (5 attempts, 1s per-attempt timeout, 25ms..200ms backoff — deliberately smaller than the connection-open retry budget in `crate::services::db`, since contention on a non-blocking read is expected to be rare and self-clearing).

`is_transient_source_contention` classifies only two textual forms as retryable:

- Turso's typed `Busy` error, whose SDK-mapped message is exactly `"database is locked"`.
- The narrow `"table is locked"` textual form used when the underlying `LimboError::TableLocked` case falls through to a generic error variant (its Display is `"Runtime error: database table is locked"`).

Every other error — including genuine extraction/mapping failures such as a missing table or column — fails on the first attempt without retry. Because `TursoTransaction::execute`/`query_map` (used inside `read_transaction`) wrap the underlying `turso::Error` into a formatted `anyhow::Error` message rather than preserving it as a typed source, classification matches on the resulting message text; this is why the SDK's textual "database is locked" content, not a `downcast_ref::<turso::Error>()`, is the retry signal.

Before every retried attempt (not the first), the retry loop calls `TursoDb::rollback_best_effort()` (`pub(crate)` in `cli/src/services/db/mod.rs`) to clear a stale failed transaction before issuing the next `BEGIN`.

See also: [agent-trace-db.md](agent-trace-db.md), [agent-trace-dwh-db.md](agent-trace-dwh-db.md), [agent-trace-dwh-replica.md](agent-trace-dwh-replica.md), [shared-turso-db.md](shared-turso-db.md), [../overview.md](../overview.md), [../architecture.md](../architecture.md), [../glossary.md](../glossary.md), [conversation ETL append-only watermark decision](../decisions/2026-08-08-conversation-etl-append-only-watermarks.md)
