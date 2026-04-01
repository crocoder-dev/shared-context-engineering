# Agent Trace retry queue and observability metrics

## Status
- Plan: `agent-trace-attribution-no-git-wrapper`
- Task: `T14`
- Implementation state: done

## Canonical contract
- Retry processing entrypoint: `cli/src/services/hooks.rs` -> `process_trace_retry_queue`.
- Production runtime invocation now runs after both `sce hooks post-commit` and `sce hooks post-rewrite` finalization paths through `process_runtime_retry_queue`.
- Queue contract now supports dequeue + enqueue replay via `TraceRetryQueue::{dequeue_next, enqueue}`.
- Retry pass processes up to `max_items` entries per invocation and avoids same-pass duplicate processing for the same trace ID.
- Runtime retry passes currently use a bounded `max_items = 16` per hook invocation.
- Recovery write behavior is target-scoped:
  - Failed notes target retries through `TraceNotesWriter`.
  - Failed DB target retries through `TraceRecordStore` using metadata idempotency key (`dev.crocoder.sce.idempotency_key`) when present.
- Retry metrics are emitted per attempted replay through `RetryMetricsSink` with:
  - `commit_sha`
  - `trace_id`
  - runtime histogram input (`runtime_ms`)
  - `error_class` (from `PersistenceFailure.class` when writes fail)
  - remaining failed targets.
- Hook command output now includes deterministic retry observability summary text: attempted/recovered/requeued counts plus transient/permanent failure counts.

## Persistence schema additions
- `cli/src/services/local_db.rs` core migrations now include:
  - `trace_retry_queue` (DB-first fallback queue storage)
- Added indexes:
  - `idx_trace_retry_queue_created_at`

## Verification evidence
- `nix flake check`
