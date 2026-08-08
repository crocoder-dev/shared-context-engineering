//! Agent Trace ETL: the incremental bridge from the repository-scoped
//! `agent-trace.db` source to the Agent Trace DWH replica destination.
//!
//! This module currently owns short, non-blocking source extraction for the
//! `agent_traces` table: bounded, ordered batches copied into owned
//! [`SourceAgentTrace`] values from a consistent read snapshot that never
//! reserves the source database's write lock, so concurrent hook writers are
//! never blocked. Only typed/transient Turso `Busy` or database-locked
//! contention is retried; every other extraction error fails immediately.
//! Transformation, hashing, and destination loading are out of scope here.

use std::thread;

use anyhow::{Context, Result};

use crate::services::{
    agent_trace_db::repository::RepositoryAgentTraceDb, resilience::RetryPolicy,
};

/// One immutable Agent Trace row copied out of the repository source
/// database. Field values are exact copies of the source row; no
/// transformation or hashing happens during extraction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceAgentTrace {
    pub id: i64,
    pub commit_id: String,
    pub commit_time_ms: i64,
    pub trace_json: String,
    pub agent_trace_id: String,
    pub url: String,
    pub remote_url: Option<String>,
}

const SELECT_AGENT_TRACE_BATCH_SQL: &str =
    "SELECT id, commit_id, commit_time_ms, trace_json, agent_trace_id, url, remote_url
FROM agent_traces
WHERE id > ?1
ORDER BY id ASC
LIMIT ?2";

/// Bounded backoff for source extraction contention retries. Contention on a
/// short, non-blocking read transaction is expected to be rare and
/// self-clearing, so the budget stays small relative to the connection-open
/// retry policy in `crate::services::db`.
const SOURCE_CONTENTION_RETRY_POLICY: RetryPolicy = RetryPolicy {
    max_attempts: 5,
    timeout_ms: 1_000,
    initial_backoff_ms: 25,
    max_backoff_ms: 200,
};

/// Extract one bounded, ordered batch of `agent_traces` rows with `id`
/// greater than `watermark`, up to `batch_size` rows, from a short consistent
/// read transaction.
///
/// The read transaction commits (releasing its snapshot) before this function
/// returns; it never reserves the source database's write lock, so concurrent
/// writers can continue while extraction is in progress. Rows written after
/// the snapshot was taken are not included in this batch; a later batch or
/// run picks them up.
///
/// Only typed/transient Turso `Busy` or database-locked contention triggers a
/// retry, with bounded backoff and a best-effort rollback before every new
/// `BEGIN`. Every other error, including extraction/mapping failures, is
/// returned immediately without retry.
pub fn extract_agent_trace_batch(
    db: &RepositoryAgentTraceDb,
    watermark: i64,
    batch_size: u32,
) -> Result<Vec<SourceAgentTrace>> {
    anyhow::ensure!(
        batch_size > 0,
        "agent trace extraction batch_size must be greater than zero"
    );

    run_with_source_contention_retry(
        |_attempt| {
            db.read_transaction(|txn| {
                txn.query_map(
                    SELECT_AGENT_TRACE_BATCH_SQL,
                    (watermark, i64::from(batch_size)),
                    source_agent_trace_from_row,
                )
            })
        },
        || db.rollback_best_effort(),
    )
}

/// Run `operation` with bounded retry limited to transient source contention.
///
/// `before_retry` runs before every retried attempt (not before the first),
/// so callers can issue a best-effort rollback ahead of the next `BEGIN`.
/// Extracted as its own function so the retry/classification behavior is
/// unit-testable without a real database.
fn run_with_source_contention_retry<T>(
    mut operation: impl FnMut(u32) -> Result<T>,
    mut before_retry: impl FnMut(),
) -> Result<T> {
    let mut attempt = 1;

    loop {
        match operation(attempt) {
            Ok(value) => return Ok(value),
            Err(error) => {
                let attempts_remain = attempt < SOURCE_CONTENTION_RETRY_POLICY.max_attempts;
                if !attempts_remain || !is_transient_source_contention(&error) {
                    return Err(error);
                }

                before_retry();
                thread::sleep(SOURCE_CONTENTION_RETRY_POLICY.backoff_for_attempt(attempt + 1));
                attempt += 1;
            }
        }
    }
}

/// Recognize transient source contention worth retrying: Turso's typed `Busy`
/// error (whose message is the SQLite "database is locked" text) and the
/// narrow "table is locked" textual form used when typed classification is
/// unavailable. Every other error, including genuine extraction/mapping
/// failures, is not retried.
fn is_transient_source_contention(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_lowercase();
    message.contains("database is locked") || message.contains("table is locked")
}

fn source_agent_trace_from_row(row: &turso::Row) -> Result<SourceAgentTrace> {
    Ok(SourceAgentTrace {
        id: row.get(0).context("failed to read agent_traces.id")?,
        commit_id: row
            .get(1)
            .context("failed to read agent_traces.commit_id")?,
        commit_time_ms: row
            .get(2)
            .context("failed to read agent_traces.commit_time_ms")?,
        trace_json: row
            .get(3)
            .context("failed to read agent_traces.trace_json")?,
        agent_trace_id: row
            .get(4)
            .context("failed to read agent_traces.agent_trace_id")?,
        url: row.get(5).context("failed to read agent_traces.url")?,
        remote_url: row
            .get(6)
            .context("failed to read agent_traces.remote_url")?,
    })
}

#[cfg(test)]
mod agent_trace_etl_source_tests {
    use std::{
        fs,
        path::PathBuf,
        sync::mpsc,
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    use anyhow::anyhow;

    use super::*;
    use crate::services::agent_trace_db::AgentTraceInsert;

    fn unique_test_db_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "sce-agent-trace-etl-source-{label}-{}-{nonce}",
                std::process::id()
            ))
            .join("agent-trace.db")
    }

    fn remove_test_db(db_path: &std::path::Path) {
        if let Some(parent) = db_path.parent() {
            fs::remove_dir_all(parent).expect("test DB directory should be removed");
        }
    }

    fn insert_agent_trace(db: &RepositoryAgentTraceDb, commit_id: &str, agent_trace_id: &str) {
        db.insert_agent_trace(AgentTraceInsert {
            commit_id,
            commit_time_ms: 1_000,
            trace_json: r#"{"id":"trace"}"#,
            agent_trace_id,
            url: "https://sce.crocoder.dev/agent-trace/trace",
            remote_url: "https://github.com/acme/widgets",
        })
        .expect("agent trace insert should succeed");
    }

    #[test]
    fn extraction_starts_from_zero_and_returns_ascending_bounded_batch() {
        let db_path = unique_test_db_path("ascending-bounded");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("source DB should open");

        for index in 1..=3 {
            insert_agent_trace(&db, &format!("commit-{index}"), &format!("trace-{index}"));
        }

        let batch = extract_agent_trace_batch(&db, 0, 2)
            .expect("extraction from a missing watermark should succeed");

        assert_eq!(
            batch.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![1, 2],
            "extraction should start from 0 and honor the bounded batch size"
        );
        assert_eq!(batch[0].agent_trace_id, "trace-1");
        assert_eq!(batch[1].agent_trace_id, "trace-2");

        remove_test_db(&db_path);
    }

    #[test]
    fn partial_batch_returned_when_fewer_rows_remain_than_the_batch_size() {
        let db_path = unique_test_db_path("partial-batch");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("source DB should open");
        insert_agent_trace(&db, "commit-1", "trace-1");

        let batch = extract_agent_trace_batch(&db, 0, 500)
            .expect("extraction with a batch size larger than remaining rows should succeed");

        assert_eq!(
            batch.len(),
            1,
            "a partial batch should return only what exists"
        );
        assert_eq!(batch[0].id, 1);

        remove_test_db(&db_path);
    }

    #[test]
    fn repeated_extraction_with_no_new_rows_is_a_noop() {
        let db_path = unique_test_db_path("noop-rerun");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("source DB should open");
        insert_agent_trace(&db, "commit-1", "trace-1");

        let first = extract_agent_trace_batch(&db, 0, 10).expect("first extraction should succeed");
        assert_eq!(first.len(), 1);

        let last_id = first.last().expect("first batch should be non-empty").id;
        let second = extract_agent_trace_batch(&db, last_id, 10)
            .expect("re-running from the last extracted id should succeed");
        assert!(
            second.is_empty(),
            "a repeated run with no new source rows must be a no-op"
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn rows_written_after_the_extraction_snapshot_are_picked_up_by_a_later_batch_or_run() {
        let db_path = unique_test_db_path("later-visibility");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("source DB should open");
        insert_agent_trace(&db, "commit-1", "trace-1");

        let first = extract_agent_trace_batch(&db, 0, 10).expect("first extraction should succeed");
        assert_eq!(first.len(), 1);

        insert_agent_trace(&db, "commit-2", "trace-2");

        let rerun_from_zero = extract_agent_trace_batch(&db, 0, 10)
            .expect("a later run starting from the same watermark should succeed");
        assert_eq!(
            rerun_from_zero.len(),
            2,
            "a later run must see rows written after an earlier snapshot"
        );

        let next_batch = extract_agent_trace_batch(&db, first[0].id, 10)
            .expect("a later batch from the previous watermark should succeed");
        assert_eq!(
            next_batch
                .iter()
                .map(|row| row.agent_trace_id.as_str())
                .collect::<Vec<_>>(),
            vec!["trace-2"],
            "a later batch must pick up rows acknowledged after the prior snapshot"
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn concurrent_writer_is_not_blocked_by_the_source_read_transaction() {
        let db_path = unique_test_db_path("concurrent-writer");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("source DB should open");
        insert_agent_trace(&db, "commit-1", "trace-1");

        let (reader_ready_tx, reader_ready_rx) = mpsc::channel::<()>();
        let (release_reader_tx, release_reader_rx) = mpsc::channel::<()>();

        let reader_db_path = db_path.clone();
        let reader_handle = thread::spawn(move || {
            let reader_db = RepositoryAgentTraceDb::open_without_migrations_at(&reader_db_path)
                .expect("reader connection should reopen");
            reader_db.read_transaction(|txn| {
                let rows = txn.query_map(
                    SELECT_AGENT_TRACE_BATCH_SQL,
                    (0i64, 10i64),
                    source_agent_trace_from_row,
                )?;
                reader_ready_tx
                    .send(())
                    .expect("reader should signal it holds an open read transaction");
                release_reader_rx
                    .recv()
                    .expect("reader should wait to be released while holding the transaction open");
                Ok(rows)
            })
        });

        reader_ready_rx
            .recv()
            .expect("test should observe the reader's open read transaction");

        let writer_db = RepositoryAgentTraceDb::open_without_migrations_at(&db_path)
            .expect("writer connection should reopen");
        insert_agent_trace(&writer_db, "commit-2", "trace-2");

        release_reader_tx
            .send(())
            .expect("test should release the held reader transaction");
        let rows = reader_handle
            .join()
            .expect("reader thread should not panic")
            .expect("reader transaction should commit");

        assert_eq!(
            rows.len(),
            1,
            "the reader's snapshot should not include the writer's concurrent insert"
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn non_contention_errors_are_never_retried() {
        let mut attempts = 0;
        let error = run_with_source_contention_retry::<()>(
            |attempt| {
                attempts = attempt;
                Err(anyhow!("no such table: agent_traces"))
            },
            || panic!("a non-contention error must not trigger a retry"),
        )
        .expect_err("a non-contention error should fail immediately");

        assert_eq!(
            attempts, 1,
            "a non-contention error must fail on the first attempt"
        );
        assert!(error.to_string().contains("no such table"));
    }

    #[test]
    fn transient_contention_errors_are_retried_with_a_rollback_before_each_new_begin() {
        let mut attempts = 0;
        let mut rollbacks_before_retry = 0;

        let value = run_with_source_contention_retry(
            |attempt| {
                attempts = attempt;
                if attempt < 3 {
                    return Err(anyhow!("database is locked"));
                }
                Ok("recovered")
            },
            || rollbacks_before_retry += 1,
        )
        .expect("recovery after transient contention should succeed");

        assert_eq!(value, "recovered");
        assert_eq!(attempts, 3, "extraction should retry until it succeeds");
        assert_eq!(
            rollbacks_before_retry, 2,
            "a best-effort rollback should run before each retried BEGIN"
        );
    }

    #[test]
    fn table_locked_form_is_classified_as_transient_contention() {
        assert!(is_transient_source_contention(&anyhow!(
            "Runtime error: database table is locked"
        )));
        assert!(is_transient_source_contention(&anyhow!(
            "repository Agent Trace DB query failed: SELECT ...: database is locked"
        )));
        assert!(!is_transient_source_contention(&anyhow!(
            "no such column: trace_json"
        )));
    }

    #[test]
    fn contention_exhausted_after_the_retry_budget_returns_the_last_error() {
        let mut attempts = 0;
        let error = run_with_source_contention_retry::<()>(
            |attempt| {
                attempts = attempt;
                Err(anyhow!("database is locked"))
            },
            || {},
        )
        .expect_err("persistent contention should eventually fail");

        assert_eq!(
            attempts, SOURCE_CONTENTION_RETRY_POLICY.max_attempts,
            "retry should stop at the configured attempt budget"
        );
        assert!(error.to_string().contains("database is locked"));
    }

    #[test]
    fn zero_batch_size_is_rejected() {
        let db_path = unique_test_db_path("zero-batch-size");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("source DB should open");

        let error =
            extract_agent_trace_batch(&db, 0, 0).expect_err("a zero batch size must be rejected");
        assert!(error.to_string().contains("batch_size"));

        remove_test_db(&db_path);
    }
}
