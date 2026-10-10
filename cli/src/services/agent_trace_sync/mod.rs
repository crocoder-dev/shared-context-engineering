//! Synchronization of a repository's local Agent Trace capture database with
//! the control-plane Agent Trace ingestion API.

pub mod control_plane;

use std::fmt;

use crate::services::agent_trace_export::{
    AgentTraceAgentTraceExportRow, AgentTraceDiffTraceExportRow, AgentTraceMessageExportRow,
    AgentTracePartExportRow,
};
use control_plane::ControlPlaneError;

/// Bound on consecutive `409`/ambiguous-batch-failure reconciliation attempts
/// for one stream, matching the order of magnitude of existing retry
/// constants (`TOKEN_REFRESH_RETRY_POLICY` in `auth.rs`, 3 attempts). Exhausting it
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
/// anything about the failed attempt itself. `Terminal` is different: the
/// attempt is known to have failed in a way that cannot be resolved by
/// `/state`, so the stream stops without invoking its refresh closure.
#[derive(Debug)]
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
    /// The batch failed with a terminal control-plane error. The typed
    /// error is already safe to surface as a stream error and is never
    /// reconciled.
    Terminal(ControlPlaneError),
}

/// Terminal failure of [`sync_stream`].
#[derive(Debug)]
pub enum StreamSyncError {
    /// The local-row reader closure failed.
    Read(String),
    /// The `/state`-refresh closure failed.
    Refresh(ControlPlaneError),
    /// A syntactically successful batch response did not match the rows
    /// that were sent (`accepted != rows.len()` or
    /// `cursor != rows.last().source_row_id()`).
    InvalidResponse(String),
    /// The batch failed with a terminal control-plane error. Unlike
    /// [`Self::Refresh`], this does not represent a failed `/state` call.
    Terminal(ControlPlaneError),
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
            Self::Terminal(reason) => write!(f, "terminal control-plane failure: {reason}"),
            Self::DidNotConverge => write!(
                f,
                "stream did not converge after {RECONCILIATION_MAX_ATTEMPTS} reconciliation attempts"
            ),
        }
    }
}

impl std::error::Error for StreamSyncError {}

impl StreamSyncError {
    /// True only when the underlying `ControlPlaneError` (from a `Refresh`
    /// or `Terminal` failure) means the caller has no usable credentials.
    /// `Read`, `InvalidResponse`, and `DidNotConverge` never carry a
    /// `ControlPlaneError` and are never authentication failures.
    pub fn is_authentication_failure(&self) -> bool {
        match self {
            Self::Refresh(error) | Self::Terminal(error) => error.is_authentication_failure(),
            Self::Read(_) | Self::InvalidResponse(_) | Self::DidNotConverge => false,
        }
    }

    /// True only when the underlying `ControlPlaneError` (from a `Refresh`
    /// or `Terminal` failure) means local credential storage is unavailable.
    /// `Read`, `InvalidResponse`, and `DidNotConverge` never carry a
    /// `ControlPlaneError` and are never storage failures.
    pub fn is_storage_failure(&self) -> bool {
        match self {
            Self::Refresh(error) | Self::Terminal(error) => error.is_storage_failure(),
            Self::Read(_) | Self::InvalidResponse(_) | Self::DidNotConverge => false,
        }
    }
}

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
/// resent. Both cases share one bounded reconciliation counter. A `Terminal`
/// outcome stops immediately without calling `refresh_cursor`.
pub async fn sync_stream<'a, T, ReadFn, IngestFn, RefreshFn>(
    initial_cursor: i64,
    batch_limit: usize,
    mut read_after: ReadFn,
    mut ingest_batch: IngestFn,
    mut refresh_cursor: RefreshFn,
) -> Result<StreamSyncOutcome, StreamSyncError>
where
    T: AgentTraceExportRow + 'a,
    ReadFn: std::ops::AsyncFnMut(i64, usize) -> Result<Vec<T>, StreamSyncError>,
    IngestFn: for<'rows> std::ops::AsyncFnMut(i64, &'rows [T]) -> BatchAttemptOutcome,
    RefreshFn: std::ops::AsyncFnMut() -> Result<i64, StreamSyncError>,
{
    let mut cursor = initial_cursor;
    let mut uploaded = 0usize;
    let mut batches = 0usize;
    let mut reconciliation_attempts = 0u32;

    loop {
        let rows = read_after(cursor, batch_limit).await?;
        if rows.is_empty() {
            break;
        }

        match ingest_batch(cursor, &rows).await {
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
            BatchAttemptOutcome::Terminal(reason) => {
                return Err(StreamSyncError::Terminal(reason));
            }
            BatchAttemptOutcome::Conflict | BatchAttemptOutcome::Ambiguous => {
                reconciliation_attempts += 1;
                if reconciliation_attempts > RECONCILIATION_MAX_ATTEMPTS {
                    return Err(StreamSyncError::DidNotConverge);
                }

                cursor = refresh_cursor().await?;
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
