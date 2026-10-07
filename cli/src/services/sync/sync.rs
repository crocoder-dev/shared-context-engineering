//! Consumes the already-shipped [`crate::services::agent_trace_sync`] engine
//! and [`crate::services::agent_trace_sync::control_plane`] client as-is; adds
//! no local sync cursor or persisted progress of its own.

use std::cell::RefCell;
use std::fmt;
use std::future::{poll_fn, Future};
use std::path::Path;
use std::rc::Rc;
use std::task::Poll;

use chrono::{DateTime, SecondsFormat, Utc};

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_export::{AgentTraceExportReader, AGENT_TRACE_EXPORT_BATCH_SIZE};
use crate::services::agent_trace_storage::{resolve_agent_trace_storage, AgentTraceStorageContext};
use crate::services::agent_trace_sync::control_plane::{
    AgentTraceCursors, AgentTraceIngestionBatchRequest, AgentTraceIngestionBatchResponse,
    AgentTraceIngestionStateRequest, AuthenticatedControlPlaneClient, ControlPlaneError,
    IngestionStream,
};
use crate::services::agent_trace_sync::{
    sync_stream, AgentTraceExportRow, BatchAttemptOutcome, StreamSyncError,
};
use crate::services::auth;
use crate::services::config;
use crate::services::sync::progress::{NoopProgressReporter, ProgressReporter};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentTraceSyncReport {
    pub repository_id: String,
    pub source_instance_id: String,
    pub streams: StreamSyncReports,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamSyncReports {
    pub messages: StreamSyncReport,
    pub parts: StreamSyncReport,
    pub diff_traces: StreamSyncReport,
    pub agent_traces: StreamSyncReport,
}

/// One stream's converged sync outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamSyncReport {
    pub uploaded: usize,
    pub initial_cursor: i64,
    pub final_cursor: i64,
    pub batches: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncProgressEvent {
    Started {
        timestamp: String,
    },
    BatchAccepted {
        stream: &'static str,
        batch_rows: usize,
        uploaded: usize,
        cursor: i64,
    },
    StreamCompleted {
        stream: &'static str,
        uploaded: usize,
        cursor: i64,
        batches: usize,
    },
    Finished {
        timestamp: String,
    },
}

/// Supplies timestamps for one trace-sync invocation.
pub trait SyncProgressClock {
    fn now(&self) -> DateTime<Utc>;
}

/// Uses the system UTC clock for production sync invocations.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemSyncProgressClock;

impl SyncProgressClock for SystemSyncProgressClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

fn timestamp<C>(clock: &C) -> String
where
    C: SyncProgressClock,
{
    clock.now().to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

/// Terminal failure of `sce sync`.
#[derive(Debug)]
pub enum TraceSyncError {
    /// Local repository/storage/config resolution failed.
    Runtime(String),
    /// The initial `/state` call failed terminally.
    ControlPlane(ControlPlaneError),
    /// One stream failed to converge.
    Stream {
        stream: &'static str,
        source: StreamSyncError,
    },
}

impl fmt::Display for TraceSyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(reason) => write!(f, "{reason}"),
            Self::ControlPlane(error) => write!(f, "{error}"),
            Self::Stream { stream, source } => write!(f, "'{stream}' stream sync failed: {source}"),
        }
    }
}

impl std::error::Error for TraceSyncError {}

impl TraceSyncError {
    /// True when this failure means the caller has no usable `WorkOS`
    /// credentials, whether that surfaced from the initial `/state` call
    /// (`ControlPlane`) or from a stream's batch/refresh path (`Stream`).
    /// `Runtime` never carries a `ControlPlaneError` and is never an
    /// authentication failure.
    #[allow(dead_code)]
    pub fn is_authentication_failure(&self) -> bool {
        match self {
            Self::Runtime(_) => false,
            Self::ControlPlane(error) => error.is_authentication_failure(),
            Self::Stream { source, .. } => source.is_authentication_failure(),
        }
    }

    /// True when the failure came from local credential storage, whether it
    /// surfaced during the initial state request or a stream batch/refresh
    /// path.
    pub fn is_storage_failure(&self) -> bool {
        match self {
            Self::ControlPlane(error) => error.is_storage_failure(),
            Self::Stream { source, .. } => source.is_storage_failure(),
            Self::Runtime(_) => false,
        }
    }
}

#[allow(dead_code)]
pub async fn run_current_sync(repo_root: &Path) -> Result<AgentTraceSyncReport, TraceSyncError> {
    let mut progress = NoopProgressReporter;
    run_current_sync_with_progress(repo_root, &mut progress).await
}

/// Production entry point with an injectable progress sink.
pub async fn run_current_sync_with_progress<S>(
    repo_root: &Path,
    progress: &mut S,
) -> Result<AgentTraceSyncReport, TraceSyncError>
where
    S: ProgressReporter<SyncProgressEvent>,
{
    let clock = SystemSyncProgressClock;
    run_current_sync_with_progress_and_clock(repo_root, progress, &clock).await
}

/// Production sync entry point with injectable progress sink and clock.
pub async fn run_current_sync_with_progress_and_clock<S, C>(
    repo_root: &Path,
    progress: &mut S,
    clock: &C,
) -> Result<AgentTraceSyncReport, TraceSyncError>
where
    S: ProgressReporter<SyncProgressEvent>,
    C: SyncProgressClock,
{
    progress.report(SyncProgressEvent::Started {
        timestamp: timestamp(clock),
    });
    let result = run_current_sync_without_progress(repo_root, progress).await;
    progress.report(SyncProgressEvent::Finished {
        timestamp: timestamp(clock),
    });
    result
}

async fn run_current_sync_without_progress<S>(
    repo_root: &Path,
    progress: &mut S,
) -> Result<AgentTraceSyncReport, TraceSyncError>
where
    S: ProgressReporter<SyncProgressEvent>,
{
    let storage_config = config::resolve_agent_trace_storage_runtime_config(repo_root)
        .map_err(|error| TraceSyncError::Runtime(format!("{error:#}")))?;
    let context = AgentTraceStorageContext {
        repository_root: repo_root,
        explicit_repository_id: storage_config.repository_id.as_deref(),
        repository_remote: &storage_config.repository_remote,
    };
    let storage = resolve_agent_trace_storage(&context)
        .await
        .map_err(|error| TraceSyncError::Runtime(format!("{error:#}")))?;
    let result = async {
        let storage = &storage;

        let auth_config = config::resolve_auth_runtime_config(repo_root)
            .map_err(|error| TraceSyncError::Runtime(format!("{error:#}")))?;
        let client = AuthenticatedControlPlaneClient::new(
            reqwest::Client::new(),
            auth_config.control_plane_base_url.value.unwrap_or_default(),
            auth::WORKOS_DEFAULT_BASE_URL,
            auth_config.workos_client_id.value.unwrap_or_default(),
        );

        run_sync_against_without_progress(
            &storage.metadata.repository_id,
            &storage.metadata.source_instance_id,
            &storage.db,
            &client,
            progress,
        )
        .await
    }
    .await;
    drop(storage);
    result
}

async fn run_sync_against_without_progress<S>(
    repository_id: &str,
    source_instance_id: &str,
    db: &RepositoryAgentTraceDb,
    client: &AuthenticatedControlPlaneClient,
    progress: &mut S,
) -> Result<AgentTraceSyncReport, TraceSyncError>
where
    S: ProgressReporter<SyncProgressEvent>,
{
    let reader = AgentTraceExportReader::new(db);

    run_sync_async(repository_id, source_instance_id, &reader, client, progress).await
}

async fn run_sync_async<'a, S>(
    repository_id: &'a str,
    source_instance_id: &'a str,
    reader: &'a AgentTraceExportReader<'a>,
    client: &'a AuthenticatedControlPlaneClient,
    progress: &'a mut S,
) -> Result<AgentTraceSyncReport, TraceSyncError>
where
    S: ProgressReporter<SyncProgressEvent> + 'a,
{
    let state_request = AgentTraceIngestionStateRequest {
        repository_id: repository_id.to_string(),
        source_instance_id: source_instance_id.to_string(),
    };
    let state = client
        .ingestion_state(&state_request)
        .await
        .map_err(TraceSyncError::ControlPlane)?;
    let progress = Rc::new(RefCell::new(progress));

    let diff_traces = StreamSyncReport {
        uploaded: 0,
        initial_cursor: state.cursors.diff_traces,
        final_cursor: state.cursors.diff_traces,
        batches: 0,
    };

    let (messages, parts, agent_traces) = try_join_three(
        sync_one_stream(
            client,
            repository_id,
            source_instance_id,
            IngestionStream::Messages,
            state.cursors.messages,
            "messages",
            async |cursor, limit| reader.read_messages_after(cursor, limit).await,
            async |request| async move { client.ingest_messages(&request).await }.await,
            Rc::clone(&progress),
        ),
        sync_one_stream(
            client,
            repository_id,
            source_instance_id,
            IngestionStream::Parts,
            state.cursors.parts,
            "parts",
            async |cursor, limit| reader.read_parts_after(cursor, limit).await,
            async |request| async move { client.ingest_parts(&request).await }.await,
            Rc::clone(&progress),
        ),
        sync_one_stream(
            client,
            repository_id,
            source_instance_id,
            IngestionStream::AgentTraces,
            state.cursors.agent_traces,
            "agent_traces",
            async |cursor, limit| reader.read_agent_traces_after(cursor, limit).await,
            async |request| async move { client.ingest_agent_traces(&request).await }.await,
            Rc::clone(&progress),
        ),
    )
    .await?;

    Ok(AgentTraceSyncReport {
        repository_id: repository_id.to_string(),
        source_instance_id: source_instance_id.to_string(),
        streams: StreamSyncReports {
            messages,
            parts,
            diff_traces,
            agent_traces,
        },
    })
}

async fn try_join_three<A, B, C, OA, OB, OC, E>(a: A, b: B, c: C) -> Result<(OA, OB, OC), E>
where
    A: Future<Output = Result<OA, E>>,
    B: Future<Output = Result<OB, E>>,
    C: Future<Output = Result<OC, E>>,
{
    let mut a = Box::pin(a);
    let mut b = Box::pin(b);
    let mut c = Box::pin(c);
    let mut a_output = None;
    let mut b_output = None;
    let mut c_output = None;

    poll_fn(|context| {
        if a_output.is_none() {
            if let Poll::Ready(result) = a.as_mut().poll(context) {
                a_output = Some(result?);
            }
        }
        if b_output.is_none() {
            if let Poll::Ready(result) = b.as_mut().poll(context) {
                b_output = Some(result?);
            }
        }
        if c_output.is_none() {
            if let Poll::Ready(result) = c.as_mut().poll(context) {
                c_output = Some(result?);
            }
        }

        match (a_output.take(), b_output.take(), c_output.take()) {
            (Some(a), Some(b), Some(c)) => Poll::Ready(Ok((a, b, c))),
            (a, b, c) => {
                a_output = a;
                b_output = b;
                c_output = c;
                Poll::Pending
            }
        }
    })
    .await
}

/// Synchronizes one stream via the T04 engine. Genuine `409`/`5xx`/transport
/// ambiguity, including an undecodable successful batch body, reconciles
/// through a real `/state` refetch. A terminal control-plane failure
/// (missing/invalid auth, `400`, `403`, or a protocol mismatch such as `404`)
/// stops the stream immediately without issuing another `/state` request.
#[allow(clippy::too_many_arguments)]
async fn sync_one_stream<'a, T, ReadFn, IngestFn, S>(
    client: &'a AuthenticatedControlPlaneClient,
    repository_id: &'a str,
    source_instance_id: &'a str,
    stream: IngestionStream,
    initial_cursor: i64,
    stream_label: &'static str,
    mut read_after: ReadFn,
    mut ingest: IngestFn,
    progress: Rc<RefCell<&'a mut S>>,
) -> Result<StreamSyncReport, TraceSyncError>
where
    T: AgentTraceExportRow + Clone + 'a,
    S: ProgressReporter<SyncProgressEvent> + 'a,
    ReadFn: std::ops::AsyncFnMut(i64, usize) -> anyhow::Result<Vec<T>> + 'a,
    IngestFn: std::ops::AsyncFnMut(
            AgentTraceIngestionBatchRequest<T>,
        ) -> Result<AgentTraceIngestionBatchResponse, ControlPlaneError>
        + 'a,
{
    let uploaded = Rc::new(RefCell::new(0usize));

    let outcome = sync_stream(
        initial_cursor,
        AGENT_TRACE_EXPORT_BATCH_SIZE,
        async |cursor, limit| {
            read_after(cursor, limit)
                .await
                .map_err(|error| StreamSyncError::Read(format!("{error:#}")))
        },
        async |cursor, rows: &[T]| {
            let uploaded = Rc::clone(&uploaded);
            let row_count = rows.len();
            let last_row_id = rows
                .last()
                .expect("rows checked non-empty above")
                .source_row_id();
            let request = AgentTraceIngestionBatchRequest {
                repository_id: repository_id.to_string(),
                source_instance_id: source_instance_id.to_string(),
                stream,
                expected_cursor: cursor,
                rows: rows.to_vec(),
            };
            let ingest_future = ingest(request);
            let progress = Rc::clone(&progress);
            async move {
                match ingest_future.await {
                    Ok(response) => {
                        if response.accepted == row_count && response.cursor == last_row_id {
                            *uploaded.borrow_mut() += row_count;
                            progress
                                .borrow_mut()
                                .report(SyncProgressEvent::BatchAccepted {
                                    stream: stream_label,
                                    batch_rows: row_count,
                                    uploaded: *uploaded.borrow(),
                                    cursor: response.cursor,
                                });
                        }
                        BatchAttemptOutcome::Accepted {
                            accepted: response.accepted,
                            cursor: response.cursor,
                        }
                    }
                    Err(ControlPlaneError::Conflict(_)) => BatchAttemptOutcome::Conflict,
                    Err(error) if is_stream_terminal(&error) => {
                        BatchAttemptOutcome::Terminal(error)
                    }
                    Err(_) => BatchAttemptOutcome::Ambiguous,
                }
            }
            .await
        },
        async || {
            let state_request = AgentTraceIngestionStateRequest {
                repository_id: repository_id.to_string(),
                source_instance_id: source_instance_id.to_string(),
            };
            async move {
                let response = client
                    .ingestion_state(&state_request)
                    .await
                    .map_err(StreamSyncError::Refresh)?;
                Ok(cursor_for_stream(&response.cursors, stream))
            }
            .await
        },
    )
    .await
    .map_err(|source| TraceSyncError::Stream {
        stream: stream_label,
        source,
    })?;

    progress
        .borrow_mut()
        .report(SyncProgressEvent::StreamCompleted {
            stream: stream_label,
            uploaded: outcome.uploaded,
            cursor: outcome.final_cursor,
            batches: outcome.batches,
        });

    Ok(StreamSyncReport {
        uploaded: outcome.uploaded,
        initial_cursor: outcome.initial_cursor,
        final_cursor: outcome.final_cursor,
        batches: outcome.batches,
    })
}

/// A control-plane failure that cannot be resolved by reconciling with
/// `/state`: missing/invalid credentials, an unrecoverable `401`, a `400`, a
/// `403` ownership rejection, or a terminal protocol/API mismatch
/// (`404`/`405`/`415`/`422`, `ControlPlaneError::Protocol`). `5xx`, transport
/// failures, and invalid batch responses are genuinely ambiguous and
/// reconcile via a real `/state` call.
fn is_stream_terminal(error: &ControlPlaneError) -> bool {
    matches!(
        error,
        ControlPlaneError::MissingCredentials
            | ControlPlaneError::AuthenticationFailed(_)
            | ControlPlaneError::BadRequest(_)
            | ControlPlaneError::Forbidden(_)
            | ControlPlaneError::Storage(_)
            | ControlPlaneError::Protocol { .. }
    )
}

fn cursor_for_stream(cursors: &AgentTraceCursors, stream: IngestionStream) -> i64 {
    match stream {
        IngestionStream::Messages => cursors.messages,
        IngestionStream::Parts => cursors.parts,
        IngestionStream::DiffTraces => cursors.diff_traces,
        IngestionStream::AgentTraces => cursors.agent_traces,
    }
}
