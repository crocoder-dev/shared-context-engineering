//! Small mechanics shared by incremental source-to-DWH table ETLs.
//!
//! Row extraction, transformation, identity validation, and loading remain
//! table-specific. This module only owns the bounded retry, configuration,
//! watermark, and common batch-accounting seams.

use std::thread;

use anyhow::{ensure, Result};

use crate::services::{
    agent_trace_dwh_db::{AgentTraceDwhDb, AgentTraceDwhDbSpec},
    db::TursoTransaction,
    resilience::RetryPolicy,
};

/// Bounded backoff for short source read transactions. Only transient source
/// contention is retried; extraction and mapping errors fail immediately.
pub(crate) const SOURCE_CONTENTION_RETRY_POLICY: RetryPolicy = RetryPolicy {
    max_attempts: 5,
    timeout_ms: 1_000,
    initial_backoff_ms: 25,
    max_backoff_ms: 200,
};

/// Statistics for one atomic destination batch, shared by table runners.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TableBatchStats {
    pub inserted: u64,
    pub already_present: u64,
    pub watermark: i64,
}

/// Validate a positive source batch size while preserving the caller's
/// table-specific diagnostic prefix.
pub(crate) fn validate_batch_size(batch_size: u32, operation: &str) -> Result<()> {
    ensure!(
        batch_size > 0,
        "{operation} batch_size must be greater than zero"
    );
    Ok(())
}

/// Run a short source read with bounded retry for transient lock contention.
/// `before_retry` clears a failed read transaction before the next `BEGIN`.
pub(crate) fn run_with_source_contention_retry<T>(
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

/// Recognize only the transient lock errors emitted by source reads.
pub(crate) fn is_transient_source_contention(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_lowercase();
    message.contains("database is locked") || message.contains("table is locked")
}

/// Read a table watermark, treating an absent row as zero.
pub(crate) fn read_watermark(
    db: &AgentTraceDwhDb,
    repository_id: &str,
    source_instance_id: &str,
    source_table: &str,
) -> Result<i64> {
    db.query_map(
        "SELECT COALESCE(last_extracted_source_row_id, 0) FROM etl_watermarks WHERE repository_id = ?1 AND source_instance_id = ?2 AND source_table = ?3",
        (repository_id, source_instance_id, source_table),
        |row| row.get::<i64>(0).map_err(Into::into),
    )?
    .into_iter()
    .next()
    .map_or(Ok(0), Ok)
}

/// Atomically upsert a table watermark in the caller's destination
/// transaction.
pub(crate) fn upsert_watermark(
    txn: &TursoTransaction<'_, AgentTraceDwhDbSpec>,
    repository_id: &str,
    source_instance_id: &str,
    source_table: &str,
    watermark: i64,
) -> Result<u64> {
    txn.execute(
        "INSERT INTO etl_watermarks (repository_id, source_instance_id, source_table, last_extracted_source_row_id) VALUES (?1, ?2, ?3, ?4) ON CONFLICT (repository_id, source_instance_id, source_table) DO UPDATE SET last_extracted_source_row_id = excluded.last_extracted_source_row_id, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        (repository_id, source_instance_id, source_table, watermark),
    )
}
