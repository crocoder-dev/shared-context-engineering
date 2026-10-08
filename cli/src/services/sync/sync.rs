//! Consumes the already-shipped [`crate::services::agent_trace_sync`] engine
//! and [`crate::services::agent_trace_sync::control_plane`] client as-is; adds
//! no local sync cursor or persisted progress of its own.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::future::{poll_fn, Future};
use std::path::Path;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

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
use crate::services::sync::progress::ProgressReporter;

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
    let auth_config = config::resolve_auth_runtime_config(repo_root)
        .map_err(|error| TraceSyncError::Runtime(format!("{error:#}")))?;
    let client = AuthenticatedControlPlaneClient::new(
        reqwest::Client::new(),
        auth_config.control_plane_base_url.value.unwrap_or_default(),
        auth::WORKOS_DEFAULT_BASE_URL,
        auth_config.workos_client_id.value.unwrap_or_default(),
    );

    let result = run_sync_against_without_progress(
        &storage.metadata.repository_id,
        &storage.metadata.source_instance_id,
        &storage.db,
        &client,
        progress,
    )
    .await;
    client.wait_for_pending_refresh().await;
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
    let terminated = Cell::new(false);

    let diff_traces = StreamSyncReport {
        uploaded: 0,
        initial_cursor: state.cursors.diff_traces,
        final_cursor: state.cursors.diff_traces,
        batches: 0,
    };

    let (messages, parts, agent_traces) = join_three_to_completion(
        sync_one_stream(
            client,
            repository_id,
            source_instance_id,
            IngestionStream::Messages,
            state.cursors.messages,
            "messages",
            async |cursor, limit| reader.read_messages_after(cursor, limit).await,
            async |request| client.ingest_messages(&request).await,
            Rc::clone(&progress),
            &terminated,
        ),
        sync_one_stream(
            client,
            repository_id,
            source_instance_id,
            IngestionStream::Parts,
            state.cursors.parts,
            "parts",
            async |cursor, limit| reader.read_parts_after(cursor, limit).await,
            async |request| client.ingest_parts(&request).await,
            Rc::clone(&progress),
            &terminated,
        ),
        sync_one_stream(
            client,
            repository_id,
            source_instance_id,
            IngestionStream::AgentTraces,
            state.cursors.agent_traces,
            "agent_traces",
            async |cursor, limit| reader.read_agent_traces_after(cursor, limit).await,
            async |request| client.ingest_agent_traces(&request).await,
            Rc::clone(&progress),
            &terminated,
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

async fn join_three_to_completion<A, B, C, OA, OB, OC, E>(
    a: A,
    b: B,
    c: C,
) -> Result<(OA, OB, OC), E>
where
    A: Future<Output = Result<OA, E>>,
    B: Future<Output = Result<OB, E>>,
    C: Future<Output = Result<OC, E>>,
{
    let mut a = Box::pin(a);
    let mut b = Box::pin(b);
    let mut c = Box::pin(c);
    let mut a_state = JoinSlot::Running;
    let mut b_state = JoinSlot::Running;
    let mut c_state = JoinSlot::Running;
    let mut first_error = None;

    poll_fn(|context| {
        a_state.poll(a.as_mut(), context, &mut first_error);
        b_state.poll(b.as_mut(), context, &mut first_error);
        c_state.poll(c.as_mut(), context, &mut first_error);

        if !(a_state.is_done() && b_state.is_done() && c_state.is_done()) {
            return Poll::Pending;
        }
        if let Some(error) = first_error.take() {
            return Poll::Ready(Err(error));
        }
        Poll::Ready(Ok((
            a_state.take_output(),
            b_state.take_output(),
            c_state.take_output(),
        )))
    })
    .await
}

enum JoinSlot<O> {
    Running,
    Succeeded(O),
    Failed,
    Taken,
}

impl<O> JoinSlot<O> {
    fn poll<F, E>(
        &mut self,
        future: Pin<&mut F>,
        context: &mut Context<'_>,
        first_error: &mut Option<E>,
    ) where
        F: Future<Output = Result<O, E>>,
    {
        if !matches!(self, Self::Running) {
            return;
        }
        match future.poll(context) {
            Poll::Pending => {}
            Poll::Ready(Ok(output)) => *self = Self::Succeeded(output),
            Poll::Ready(Err(error)) => {
                if first_error.is_none() {
                    *first_error = Some(error);
                }
                *self = Self::Failed;
            }
        }
    }

    fn is_done(&self) -> bool {
        !matches!(self, Self::Running)
    }

    fn take_output(&mut self) -> O {
        match std::mem::replace(self, Self::Taken) {
            Self::Succeeded(output) => output,
            _ => unreachable!("join output is taken only after every future succeeded"),
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
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
    terminated: &'a Cell<bool>,
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
            if terminated.get() {
                return Ok(Vec::new());
            }
            let rows = read_after(cursor, limit).await;
            if terminated.get() {
                return Ok(Vec::new());
            }
            rows.map_err(|error| StreamSyncError::Read(format!("{error:#}")))
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
    .map_err(|source| {
        terminated.set(true);
        TraceSyncError::Stream {
            stream: stream_label,
            source,
        }
    })?;

    if !terminated.get() {
        progress
            .borrow_mut()
            .report(SyncProgressEvent::StreamCompleted {
                stream: stream_label,
                uploaded: outcome.uploaded,
                cursor: outcome.final_cursor,
                batches: outcome.batches,
            });
    }

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

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Duration;

    use tokio::sync::oneshot;

    use super::{join_three_to_completion, sync_one_stream, TraceSyncError};
    use crate::services::agent_trace_db::MessageRole;
    use crate::services::agent_trace_export::AgentTraceMessageExportRow;
    use crate::services::agent_trace_sync::control_plane::{
        AgentTraceIngestionBatchResponse, AuthenticatedControlPlaneClient, ControlPlaneError,
        IngestionStream,
    };
    use crate::services::agent_trace_sync::StreamSyncError;
    use crate::services::sync::progress::NoopProgressReporter;

    const NOT_COMPLETED_PROBE: Duration = Duration::from_millis(50);

    #[tokio::test]
    async fn sibling_failure_does_not_cancel_in_flight_credential_save() {
        let (save_started_tx, save_started_rx) = oneshot::channel::<()>();
        let (release_tx, release_rx) = oneshot::channel::<()>();
        let failing_finished = Rc::new(Cell::new(false));
        let save_completed = Rc::new(Cell::new(false));
        let succeeding_finished = Rc::new(Cell::new(false));

        let failing = {
            let failing_finished = Rc::clone(&failing_finished);
            async move {
                save_started_rx.await.expect("save start signal");
                failing_finished.set(true);
                Err::<(), _>("parts stream failed")
            }
        };
        let saving = {
            let save_completed = Rc::clone(&save_completed);
            async move {
                save_started_tx.send(()).expect("signal save start");
                release_rx.await.expect("save release signal");
                save_completed.set(true);
                Ok("messages report")
            }
        };
        let succeeding = {
            let succeeding_finished = Rc::clone(&succeeding_finished);
            async move {
                succeeding_finished.set(true);
                Ok("agent_traces report")
            }
        };

        let mut join = std::pin::pin!(join_three_to_completion(failing, saving, succeeding));

        let probe = tokio::time::timeout(NOT_COMPLETED_PROBE, join.as_mut()).await;
        assert!(
            probe.is_err(),
            "join completed before the in-flight save finished"
        );
        assert!(failing_finished.get());
        assert!(!save_completed.get());

        release_tx.send(()).expect("release save");
        let result = join.await;

        assert_eq!(result, Err("parts stream failed"));
        assert!(save_completed.get());
        assert!(succeeding_finished.get());
    }

    fn message_row(source_row_id: i64) -> AgentTraceMessageExportRow {
        AgentTraceMessageExportRow {
            source_row_id,
            session_id: "session".to_string(),
            message_id: format!("message-{source_row_id}"),
            role: MessageRole::User,
            generated_at_unix_ms: source_row_id,
        }
    }

    #[tokio::test]
    async fn terminal_stream_failure_stops_sibling_after_its_in_flight_batch() {
        let client = AuthenticatedControlPlaneClient::new(
            reqwest::Client::builder()
                .tls_certs_only(Vec::new())
                .build()
                .expect("test http client"),
            "http://127.0.0.1:1",
            "http://127.0.0.1:1",
            "client",
        );
        let mut reporter = NoopProgressReporter;
        let progress = Rc::new(RefCell::new(&mut reporter));
        let terminated = Cell::new(false);
        let sibling_ingest_started = Cell::new(false);
        let terminal_observed = Cell::new(false);
        let sibling_reads = Cell::new(0usize);
        let sibling_ingests = Cell::new(0usize);
        let sibling_ingest_completed = Cell::new(false);

        let failing = sync_one_stream(
            &client,
            "repo",
            "source",
            IngestionStream::Messages,
            0,
            "messages",
            async |cursor, _limit| Ok(vec![message_row(cursor + 1)]),
            async |_request| {
                while !sibling_ingest_started.get() {
                    tokio::task::yield_now().await;
                }
                terminal_observed.set(true);
                Err(ControlPlaneError::Forbidden("denied".to_string()))
            },
            Rc::clone(&progress),
            &terminated,
        );
        let sibling = sync_one_stream(
            &client,
            "repo",
            "source",
            IngestionStream::Parts,
            0,
            "parts",
            async |cursor, _limit| {
                sibling_reads.set(sibling_reads.get() + 1);
                Ok(vec![message_row(cursor + 1)])
            },
            async |request| {
                sibling_ingests.set(sibling_ingests.get() + 1);
                sibling_ingest_started.set(true);
                while !terminal_observed.get() {
                    tokio::task::yield_now().await;
                }
                sibling_ingest_completed.set(true);
                Ok(AgentTraceIngestionBatchResponse {
                    accepted: request.rows.len(),
                    cursor: request.rows[0].source_row_id,
                })
            },
            Rc::clone(&progress),
            &terminated,
        );
        let idle = async { Ok::<_, TraceSyncError>(()) };

        let result = join_three_to_completion(failing, sibling, idle).await;

        assert!(matches!(
            result,
            Err(TraceSyncError::Stream {
                stream: "messages",
                source: StreamSyncError::Terminal(ControlPlaneError::Forbidden(_)),
            })
        ));
        assert!(sibling_ingest_completed.get());
        assert_eq!(sibling_ingests.get(), 1);
        assert_eq!(sibling_reads.get(), 1);
    }

    #[tokio::test]
    async fn terminal_stream_failure_discards_sibling_rows_read_after_termination() {
        let client = AuthenticatedControlPlaneClient::new(
            reqwest::Client::builder()
                .tls_certs_only(Vec::new())
                .build()
                .expect("test http client"),
            "http://127.0.0.1:1",
            "http://127.0.0.1:1",
            "client",
        );
        let mut reporter = NoopProgressReporter;
        let progress = Rc::new(RefCell::new(&mut reporter));
        let terminated = Cell::new(false);
        let sibling_read_started = Cell::new(false);
        let sibling_ingests = Cell::new(0usize);

        let failing = sync_one_stream(
            &client,
            "repo",
            "source",
            IngestionStream::Messages,
            0,
            "messages",
            async |cursor, _limit| Ok(vec![message_row(cursor + 1)]),
            async |_request| {
                while !sibling_read_started.get() {
                    tokio::task::yield_now().await;
                }
                Err(ControlPlaneError::Forbidden("denied".to_string()))
            },
            Rc::clone(&progress),
            &terminated,
        );
        let sibling = sync_one_stream(
            &client,
            "repo",
            "source",
            IngestionStream::Parts,
            0,
            "parts",
            async |cursor, _limit| {
                sibling_read_started.set(true);
                while !terminated.get() {
                    tokio::task::yield_now().await;
                }
                Ok(vec![message_row(cursor + 1)])
            },
            async |request| {
                sibling_ingests.set(sibling_ingests.get() + 1);
                Ok(AgentTraceIngestionBatchResponse {
                    accepted: request.rows.len(),
                    cursor: request.rows[0].source_row_id,
                })
            },
            Rc::clone(&progress),
            &terminated,
        );
        let idle = async { Ok::<_, TraceSyncError>(()) };

        let result = join_three_to_completion(failing, sibling, idle).await;

        assert!(matches!(
            result,
            Err(TraceSyncError::Stream {
                stream: "messages",
                source: StreamSyncError::Terminal(ControlPlaneError::Forbidden(_)),
            })
        ));
        assert!(terminated.get());
        assert_eq!(sibling_ingests.get(), 0);
    }
}
