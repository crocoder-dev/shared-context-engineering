# Agent Trace ETL

`cli/src/services/agent_trace_etl/mod.rs` is the incremental bridge between the repository-scoped, multiprocess-WAL `agent-trace.db` source (see [agent-trace-db.md](agent-trace-db.md)) and the Agent Trace DWH replica destination (see [agent-trace-dwh-replica.md](agent-trace-dwh-replica.md), [agent-trace-dwh-db.md](agent-trace-dwh-db.md)). Not yet wired into `AgentTraceDwhReplica`, CLI, or lifecycle; this document currently covers only `agent_traces` source extraction.

## Current scope: source extraction only

`extract_agent_trace_batch(db: &RepositoryAgentTraceDb, watermark: i64, batch_size: u32) -> Result<Vec<SourceAgentTrace>>` extracts one bounded, ordered batch of `agent_traces` rows with `id > watermark`, up to `batch_size` rows (`ORDER BY id ASC LIMIT`), copied into owned `SourceAgentTrace` values (`id`, `commit_id`, `commit_time_ms`, `trace_json`, `agent_trace_id`, `url`, nullable `remote_url`) with no transformation or hashing. Rejects `batch_size == 0`.

Not yet implemented: transformation/hashing, destination loading, watermark read/advance, and the public multi-batch run loop — later tasks in the `incremental-agent-trace-etl-transactional-watermarks` plan.

## Non-blocking read transaction

Extraction runs inside `TursoDb::read_transaction` (see [shared-turso-db.md](shared-turso-db.md)), which issues a plain `BEGIN` instead of `BEGIN IMMEDIATE`. This never reserves the source database's write lock, so concurrent hook writers on other connections are never blocked while a batch is being read. The transaction commits (releasing its snapshot) before `extract_agent_trace_batch` returns. Rows written after a snapshot was taken are not included in that batch; a later batch (larger watermark) or a later run (same watermark) picks them up.

## Source contention retry

A private `run_with_source_contention_retry` wraps each extraction attempt with a bounded backoff policy (5 attempts, 1s per-attempt timeout, 25ms..200ms backoff — deliberately smaller than the connection-open retry budget in `crate::services::db`, since contention on a non-blocking read is expected to be rare and self-clearing).

`is_transient_source_contention` classifies only two textual forms as retryable:

- Turso's typed `Busy` error, whose SDK-mapped message is exactly `"database is locked"`.
- The narrow `"table is locked"` textual form used when the underlying `LimboError::TableLocked` case falls through to a generic error variant (its Display is `"Runtime error: database table is locked"`).

Every other error — including genuine extraction/mapping failures such as a missing table or column — fails on the first attempt without retry. Because `TursoTransaction::execute`/`query_map` (used inside `read_transaction`) wrap the underlying `turso::Error` into a formatted `anyhow::Error` message rather than preserving it as a typed source, classification matches on the resulting message text; this is why the SDK's textual "database is locked" content, not a `downcast_ref::<turso::Error>()`, is the retry signal.

Before every retried attempt (not the first), the retry loop calls `TursoDb::rollback_best_effort()` (`pub(crate)` in `cli/src/services/db/mod.rs`) to clear a stale failed transaction before issuing the next `BEGIN`.

## Metadata

The stable `source_instance_id` for a batch comes from the existing `RepositoryAgentTraceDb::verify_or_initialize_repository_metadata(repository_id)` API (see [agent-trace-db.md](agent-trace-db.md)); extraction itself takes no `repository_id` parameter and performs no metadata lookup. A production entrypoint that accepts `repository_id` and resolves `source_instance_id` this way is expected from a later task in the same plan.

See also: [shared-turso-db.md](shared-turso-db.md), [agent-trace-db.md](agent-trace-db.md), [agent-trace-dwh-db.md](agent-trace-dwh-db.md), [agent-trace-dwh-replica.md](agent-trace-dwh-replica.md), [../overview.md](../overview.md), [../architecture.md](../architecture.md)
