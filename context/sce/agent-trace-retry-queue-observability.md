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

## Reconciliation metrics contract
- Reconciliation mapping metrics entrypoint: `cli/src/services/hosted_reconciliation.rs` -> `summarize_reconciliation_metrics`.
- Snapshot shape (`ReconciliationMetricsSnapshot`) tracks:
  - `mapped` / `unmapped` counts
  - confidence histogram buckets (`high`/`medium`/`low`)
  - run runtime (`runtime_ms`)
  - optional error class (`signature`, `payload`, `store`).
- Error-class normalization helper: `classify_reconciliation_error`.

## Persistence schema additions
- `cli/src/services/local_db.rs` core migrations now include:
  - `trace_retry_queue` (DB-first fallback queue storage)
  - `reconciliation_metrics` (runtime metric snapshots)
- Added indexes:
  - `idx_trace_retry_queue_created_at`
  - `idx_reconciliation_metrics_created_at`

## Verification evidence
- `cargo test --manifest-path cli/Cargo.toml hooks::tests::retry_processor_recovers_failed_notes_write_and_emits_success_metric`
- `cargo test --manifest-path cli/Cargo.toml hooks::tests::retry_processor_requeues_when_db_write_still_fails`
- `cargo test --manifest-path cli/Cargo.toml hosted_reconciliation::tests::reconciliation_metrics_capture_mapped_unmapped_histogram_runtime_and_error_class`
- `cargo test --manifest-path cli/Cargo.toml hosted_reconciliation::tests::reconciliation_error_classification_labels_signature_and_payload_failures`
- `cargo test --manifest-path cli/Cargo.toml local_db::tests::core_schema_migrations_create_required_tables_and_indexes`
- `cargo build --manifest-path cli/Cargo.toml`
