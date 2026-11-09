pub mod control_plane;

use std::fmt;

use crate::services::agent_trace_export::{
    AgentTraceAgentTraceExportRow, AgentTraceMessageExportRow, AgentTracePartExportRow,
};
use crate::services::error::AuthStorageFailure;
use control_plane::ControlPlaneError;

pub const RECONCILIATION_MAX_ATTEMPTS: u32 = 5;

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

impl AgentTraceExportRow for AgentTraceAgentTraceExportRow {
    fn source_row_id(&self) -> i64 {
        self.source_row_id
    }
}

#[derive(Debug)]
pub enum BatchAttemptOutcome {
    Accepted { accepted: usize, cursor: i64 },
    Conflict,
    Ambiguous,
    Terminal(ControlPlaneError),
}

#[derive(Debug)]
pub enum StreamSyncError {
    Read(String),
    Refresh(ControlPlaneError),
    InvalidResponse(String),
    Terminal(ControlPlaneError),
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
    pub fn is_authentication_failure(&self) -> bool {
        match self {
            Self::Refresh(error) | Self::Terminal(error) => error.is_authentication_failure(),
            Self::Read(_) | Self::InvalidResponse(_) | Self::DidNotConverge => false,
        }
    }

    pub fn storage_failure(&self) -> Option<AuthStorageFailure> {
        match self {
            Self::Refresh(error) | Self::Terminal(error) => error.storage_failure(),
            Self::Read(_) | Self::InvalidResponse(_) | Self::DidNotConverge => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamSyncOutcome {
    pub uploaded: usize,
    pub initial_cursor: i64,
    pub final_cursor: i64,
    pub batches: usize,
}

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
