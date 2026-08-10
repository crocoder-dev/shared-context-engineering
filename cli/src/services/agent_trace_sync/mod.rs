//! Synchronization of a repository's local Agent Trace capture database with
//! the control-plane Agent Trace ingestion API.

pub mod control_plane;

#[cfg(test)]
pub(crate) mod test_http_server;

use std::fmt;

use crate::services::agent_trace_export::{
    AgentTraceAgentTraceExportRow, AgentTraceDiffTraceExportRow, AgentTraceMessageExportRow,
    AgentTracePartExportRow,
};

/// Bound on consecutive `409`/ambiguous-batch-failure reconciliation attempts
/// for one stream, matching the order of magnitude of existing retry
/// constants (`TOKEN_REFRESH_MAX_ATTEMPTS = 3` in `auth.rs`). Exhausting it
/// fails the stream rather than looping unboundedly.
pub const RECONCILIATION_MAX_ATTEMPTS: u32 = 5;

/// Exposes the `source_row_id` each of the four PR #198 export row types
/// carries, so the sync engine can validate and advance cursors generically
/// without a per-stream copy of the same logic.
pub trait AgentTraceExportRow {
    fn source_row_id(&self) -> i64;
}

impl AgentTraceExportRow for AgentTraceMessageExportRow {
    fn source_row_id(&self) -> i64 {
        self.source_row_id
    }
}

impl AgentTraceExportRow for AgentTracePartExportRow {
    fn source_row_id(&self) -> i64 {
        self.source_row_id
    }
}

impl AgentTraceExportRow for AgentTraceDiffTraceExportRow {
    fn source_row_id(&self) -> i64 {
        self.source_row_id
    }
}

impl AgentTraceExportRow for AgentTraceAgentTraceExportRow {
    fn source_row_id(&self) -> i64 {
        self.source_row_id
    }
}

/// Result of one batch-ingest attempt, as classified by the caller-supplied
/// ingest closure. `Conflict` and `Ambiguous` carry no data: reconciliation
/// always re-derives truth from a fresh `/state` call rather than trusting
/// anything about the failed attempt itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchAttemptOutcome {
    /// The batch was accepted. `accepted` and `cursor` are the server
    /// response's own fields, validated by the engine before the stream
    /// cursor advances.
    Accepted { accepted: usize, cursor: i64 },
    /// The server rejected the batch with a cursor conflict (`409`).
    Conflict,
    /// The batch outcome could not be determined (`5xx`, a transport
    /// failure, or an invalid response).
    Ambiguous,
}

/// Terminal failure of [`sync_stream`].
#[derive(Debug)]
pub enum StreamSyncError {
    /// The local-row reader closure failed.
    Read(String),
    /// The `/state`-refresh closure failed.
    Refresh(String),
    /// A syntactically successful batch response did not match the rows
    /// that were sent (`accepted != rows.len()` or
    /// `cursor != rows.last().source_row_id()`).
    InvalidResponse(String),
    /// The reconciliation loop exceeded [`RECONCILIATION_MAX_ATTEMPTS`]
    /// without converging.
    DidNotConverge,
}

impl fmt::Display for StreamSyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(reason) => write!(f, "failed to read local rows: {reason}"),
            Self::Refresh(reason) => write!(f, "failed to refresh authoritative cursor: {reason}"),
            Self::InvalidResponse(reason) => {
                write!(f, "control-plane batch response did not match the sent rows: {reason}")
            }
            Self::DidNotConverge => write!(
                f,
                "stream did not converge after {RECONCILIATION_MAX_ATTEMPTS} reconciliation attempts"
            ),
        }
    }
}

impl std::error::Error for StreamSyncError {}

/// Outcome of a fully converged [`sync_stream`] run for one stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamSyncOutcome {
    pub uploaded: usize,
    pub initial_cursor: i64,
    pub final_cursor: i64,
    pub batches: usize,
}

/// Synchronizes one Agent Trace capture stream: reads local rows after
/// `cursor` via `read_after`, uploads them in batches bounded by
/// `batch_limit` via `ingest_batch`, and advances the cursor only from a
/// validated server response. Never infers the next cursor from
/// `cursor + rows.len()`; it always uses the server-reported `cursor`
/// (or, on reconciliation, the freshly fetched `/state` cursor).
///
/// On `Conflict` or `Ambiguous`, calls `refresh_cursor` and resumes from the
/// refreshed value: if it advanced, the next read naturally skips the
/// already-accepted rows; if unchanged, the same rows are re-read and
/// resent. Both cases share one bounded reconciliation counter.
pub fn sync_stream<T, ReadFn, IngestFn, RefreshFn>(
    initial_cursor: i64,
    batch_limit: usize,
    mut read_after: ReadFn,
    mut ingest_batch: IngestFn,
    mut refresh_cursor: RefreshFn,
) -> Result<StreamSyncOutcome, StreamSyncError>
where
    T: AgentTraceExportRow,
    ReadFn: FnMut(i64, usize) -> Result<Vec<T>, StreamSyncError>,
    IngestFn: FnMut(i64, &[T]) -> BatchAttemptOutcome,
    RefreshFn: FnMut() -> Result<i64, StreamSyncError>,
{
    let mut cursor = initial_cursor;
    let mut uploaded = 0usize;
    let mut batches = 0usize;
    let mut reconciliation_attempts = 0u32;

    loop {
        let rows = read_after(cursor, batch_limit)?;
        if rows.is_empty() {
            break;
        }

        match ingest_batch(cursor, &rows) {
            BatchAttemptOutcome::Accepted {
                accepted,
                cursor: reported_cursor,
            } => {
                let last_row_id = rows
                    .last()
                    .expect("rows checked non-empty above")
                    .source_row_id();

                if accepted != rows.len() || reported_cursor != last_row_id {
                    return Err(StreamSyncError::InvalidResponse(format!(
                        "sent {} rows up to source_row_id {last_row_id}, server reported accepted={accepted} cursor={reported_cursor}",
                        rows.len()
                    )));
                }

                cursor = reported_cursor;
                uploaded += rows.len();
                batches += 1;
                reconciliation_attempts = 0;
            }
            BatchAttemptOutcome::Conflict | BatchAttemptOutcome::Ambiguous => {
                reconciliation_attempts += 1;
                if reconciliation_attempts > RECONCILIATION_MAX_ATTEMPTS {
                    return Err(StreamSyncError::DidNotConverge);
                }

                cursor = refresh_cursor()?;
            }
        }
    }

    Ok(StreamSyncOutcome {
        uploaded,
        initial_cursor,
        final_cursor: cursor,
        batches,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::services::agent_trace_db::MessageRole;

    fn row(source_row_id: i64) -> AgentTraceMessageExportRow {
        AgentTraceMessageExportRow {
            source_row_id,
            session_id: "session-1".to_string(),
            message_id: format!("message-{source_row_id}"),
            role: MessageRole::User,
            generated_at_unix_ms: 1_700_000_000_000,
        }
    }

    /// In-memory local store: rows never disappear, only the caller-visible
    /// server-accepted cursor advances, mirroring how the real reader and
    /// control plane are independent sources of truth.
    struct FakeLocalRows {
        rows: Vec<AgentTraceMessageExportRow>,
    }

    impl FakeLocalRows {
        fn new(count: i64) -> Self {
            Self {
                rows: (1..=count).map(row).collect(),
            }
        }

        fn after(&self, cursor: i64, limit: usize) -> Vec<AgentTraceMessageExportRow> {
            self.rows
                .iter()
                .filter(|row| row.source_row_id > cursor)
                .take(limit)
                .cloned()
                .collect()
        }
    }

    #[test]
    fn empty_database_makes_no_batch_calls() {
        let local = FakeLocalRows::new(0);
        let batch_calls = RefCell::new(0usize);

        let outcome = sync_stream(
            0,
            500,
            |cursor, limit| Ok(local.after(cursor, limit)),
            |_cursor, rows: &[AgentTraceMessageExportRow]| {
                *batch_calls.borrow_mut() += 1;
                BatchAttemptOutcome::Accepted {
                    accepted: rows.len(),
                    cursor: rows.last().map_or(0, |row| row.source_row_id),
                }
            },
            || panic!("refresh should not be called when there is nothing to reconcile"),
        )
        .unwrap();

        assert_eq!(*batch_calls.borrow(), 0);
        assert_eq!(
            outcome,
            StreamSyncOutcome {
                uploaded: 0,
                initial_cursor: 0,
                final_cursor: 0,
                batches: 0,
            }
        );
    }

    #[test]
    fn one_batch_uploads_all_rows_and_advances_cursor() {
        let local = FakeLocalRows::new(3);
        let batch_calls = RefCell::new(0usize);

        let outcome = sync_stream(
            0,
            500,
            |cursor, limit| Ok(local.after(cursor, limit)),
            |_cursor, rows: &[AgentTraceMessageExportRow]| {
                *batch_calls.borrow_mut() += 1;
                BatchAttemptOutcome::Accepted {
                    accepted: rows.len(),
                    cursor: rows.last().unwrap().source_row_id,
                }
            },
            || panic!("refresh should not be called on an all-success run"),
        )
        .unwrap();

        assert_eq!(*batch_calls.borrow(), 1);
        assert_eq!(
            outcome,
            StreamSyncOutcome {
                uploaded: 3,
                initial_cursor: 0,
                final_cursor: 3,
                batches: 1,
            }
        );
    }

    #[test]
    fn more_than_five_hundred_rows_span_multiple_bounded_batches() {
        let local = FakeLocalRows::new(1_100);
        let batch_sizes = RefCell::new(Vec::new());

        let outcome = sync_stream(
            0,
            500,
            |cursor, limit| {
                assert!(limit <= 500);
                Ok(local.after(cursor, limit))
            },
            |_cursor, rows: &[AgentTraceMessageExportRow]| {
                batch_sizes.borrow_mut().push(rows.len());
                BatchAttemptOutcome::Accepted {
                    accepted: rows.len(),
                    cursor: rows.last().unwrap().source_row_id,
                }
            },
            || panic!("refresh should not be called on an all-success run"),
        )
        .unwrap();

        assert_eq!(*batch_sizes.borrow(), vec![500, 500, 100]);
        assert_eq!(outcome.uploaded, 1_100);
        assert_eq!(outcome.final_cursor, 1_100);
        assert_eq!(outcome.batches, 3);
    }

    #[test]
    fn gapped_source_ids_advance_cursor_from_last_id_not_row_count() {
        let local = FakeLocalRows {
            rows: vec![row(2), row(5), row(9)],
        };

        let outcome = sync_stream(
            0,
            500,
            |cursor, limit| Ok(local.after(cursor, limit)),
            |_cursor, rows: &[AgentTraceMessageExportRow]| BatchAttemptOutcome::Accepted {
                accepted: rows.len(),
                cursor: rows.last().unwrap().source_row_id,
            },
            || panic!("refresh should not be called on an all-success run"),
        )
        .unwrap();

        assert_eq!(outcome.uploaded, 3);
        assert_eq!(outcome.final_cursor, 9);
        let uploaded_as_cursor_delta =
            i64::try_from(outcome.uploaded).expect("uploaded count fits in i64 for this test");
        assert_ne!(
            outcome.final_cursor,
            outcome.initial_cursor + uploaded_as_cursor_delta
        );
    }

    #[test]
    fn conflict_resends_only_the_unsent_tail() {
        // Mirrors the plan's worked example: state=10, local rows 11-13, the
        // first batch 409s, refreshed state=12, only row 13 is resent with
        // expected_cursor=12.
        let local = FakeLocalRows {
            rows: vec![row(11), row(12), row(13)],
        };
        let attempts: RefCell<Vec<(i64, Vec<i64>)>> = RefCell::new(Vec::new());

        let outcome = sync_stream(
            10,
            500,
            |cursor, limit| Ok(local.after(cursor, limit)),
            |cursor, rows: &[AgentTraceMessageExportRow]| {
                let ids: Vec<i64> = rows.iter().map(|row| row.source_row_id).collect();
                attempts.borrow_mut().push((cursor, ids));
                if attempts.borrow().len() == 1 {
                    BatchAttemptOutcome::Conflict
                } else {
                    BatchAttemptOutcome::Accepted {
                        accepted: rows.len(),
                        cursor: rows.last().unwrap().source_row_id,
                    }
                }
            },
            || Ok(12),
        )
        .unwrap();

        let attempts = attempts.into_inner();
        assert_eq!(attempts, vec![(10, vec![11, 12, 13]), (12, vec![13])]);
        assert_eq!(outcome.uploaded, 1);
        assert_eq!(outcome.final_cursor, 13);
    }

    #[test]
    fn ambiguous_failure_with_advanced_refresh_does_not_resend() {
        let local = FakeLocalRows::new(3);
        let attempts: RefCell<Vec<Vec<i64>>> = RefCell::new(Vec::new());

        let outcome = sync_stream(
            0,
            500,
            |cursor, limit| Ok(local.after(cursor, limit)),
            |_cursor, rows: &[AgentTraceMessageExportRow]| {
                let ids: Vec<i64> = rows.iter().map(|row| row.source_row_id).collect();
                let is_first = attempts.borrow().is_empty();
                attempts.borrow_mut().push(ids);
                if is_first {
                    BatchAttemptOutcome::Ambiguous
                } else {
                    BatchAttemptOutcome::Accepted {
                        accepted: rows.len(),
                        cursor: rows.last().unwrap().source_row_id,
                    }
                }
            },
            // The control plane had already committed the batch server-side
            // despite the ambiguous response; /state now reflects it.
            || Ok(3),
        )
        .unwrap();

        let attempts = attempts.into_inner();
        assert_eq!(attempts, vec![vec![1, 2, 3]]);
        assert_eq!(outcome.uploaded, 0);
        assert_eq!(outcome.final_cursor, 3);
    }

    #[test]
    fn ambiguous_failure_with_unchanged_refresh_resends_once() {
        let local = FakeLocalRows::new(3);
        let attempts: RefCell<Vec<Vec<i64>>> = RefCell::new(Vec::new());

        let outcome = sync_stream(
            0,
            500,
            |cursor, limit| Ok(local.after(cursor, limit)),
            |_cursor, rows: &[AgentTraceMessageExportRow]| {
                let ids: Vec<i64> = rows.iter().map(|row| row.source_row_id).collect();
                let is_first = attempts.borrow().is_empty();
                attempts.borrow_mut().push(ids);
                if is_first {
                    BatchAttemptOutcome::Ambiguous
                } else {
                    BatchAttemptOutcome::Accepted {
                        accepted: rows.len(),
                        cursor: rows.last().unwrap().source_row_id,
                    }
                }
            },
            // /state confirms nothing was actually committed.
            || Ok(0),
        )
        .unwrap();

        let attempts = attempts.into_inner();
        assert_eq!(attempts, vec![vec![1, 2, 3], vec![1, 2, 3]]);
        assert_eq!(outcome.uploaded, 3);
        assert_eq!(outcome.final_cursor, 3);
    }

    #[test]
    fn reconciliation_bound_fails_with_did_not_converge() {
        let local = FakeLocalRows::new(3);

        let result = sync_stream(
            0,
            500,
            |cursor, limit| Ok(local.after(cursor, limit)),
            |_cursor, _rows: &[AgentTraceMessageExportRow]| BatchAttemptOutcome::Ambiguous,
            || Ok(0),
        );

        assert!(matches!(result, Err(StreamSyncError::DidNotConverge)));
    }

    #[test]
    fn invalid_response_rejects_mismatched_accepted_and_cursor() {
        let local = FakeLocalRows::new(3);

        let result = sync_stream(
            0,
            500,
            |cursor, limit| Ok(local.after(cursor, limit)),
            |_cursor, _rows: &[AgentTraceMessageExportRow]| BatchAttemptOutcome::Accepted {
                accepted: 2,
                cursor: 99,
            },
            || panic!("refresh should not be called on an invalid response"),
        );

        assert!(matches!(result, Err(StreamSyncError::InvalidResponse(_))));
    }
}
