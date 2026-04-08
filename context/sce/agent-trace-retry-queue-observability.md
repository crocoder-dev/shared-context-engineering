# Agent Trace retry queue and observability metrics

## Current status
- This contract is no longer active in local hook runtime.
- The current `cli/src/services/hooks.rs` no longer runs retry replay from hook entrypoints.

## Status
- Plan: `agent-trace-attribution-no-git-wrapper`
- Task: `T14`
- Implementation state: done

## Canonical contract
- Retry processing entrypoint: `cli/src/services/hooks.rs` -> `process_trace_retry_queue`.
- The retained `process_runtime_retry_queue` wrapper is not invoked by the current trace-removal runtime baseline; `sce hooks post-commit` and `sce hooks post-rewrite` now no-op before retry replay.
- Queue contract now supports dequeue + enqueue replay via `TraceRetryQueue::{dequeue_next, enqueue}`.
- Retry pass processes up to `max_items` entries per invocation and avoids same-pass duplicate processing for the same trace ID.
- If runtime invocation is re-enabled later, the retained wrapper still uses bounded `max_items = 16` replay.
- Recovery write behavior is target-scoped:
  - Failed notes target retries through `TraceNotesWriter`.
  - Failed DB target retries through `TraceRecordStore` using metadata idempotency key (`dev.crocoder.sce.idempotency_key`) when present.
- Retry metrics are emitted per attempted replay through `RetryMetricsSink` with:
  - `commit_sha`
  - `trace_id`
  - runtime histogram input (`runtime_ms`)
  - `error_class` (from `PersistenceFailure.class` when writes fail)
  - remaining failed targets.
- The retained runtime wrapper still formats deterministic retry observability summary text, but current hook command output does not surface it because retry replay is not invoked.

## Persistence schema additions
- `cli/src/services/local_db.rs` core migrations now include:
  - `trace_retry_queue` (DB-first fallback queue storage)
- Added indexes:
  - `idx_trace_retry_queue_created_at`

## Verification evidence
- `nix flake check`
