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
use crate::services::agent_trace_storage::{
    resolve_agent_trace_storage, AgentTraceStorageContext, ResolvedAgentTraceStorage,
};
use crate::services::agent_trace_sync::control_plane::{
    AgentTraceCursors, AgentTraceIngestionBatchRequest, AgentTraceIngestionBatchResponse,
    AgentTraceIngestionStateRequest, AuthenticatedControlPlaneClient, ControlPlaneError,
    IngestionStream,
};
use crate::services::agent_trace_sync::{
    sync_stream, AgentTraceExportRow, BatchAttemptOutcome, StreamSyncError, SyncFuture,
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

struct SyncStorageGuard(Option<ResolvedAgentTraceStorage>);

impl SyncStorageGuard {
    fn storage(&self) -> &ResolvedAgentTraceStorage {
        self.0
            .as_ref()
            .expect("sync storage remains owned until cleanup")
    }
}

impl Drop for SyncStorageGuard {
    fn drop(&mut self) {
        tokio::task::block_in_place(|| drop(self.0.take()));
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
    let storage = tokio::task::block_in_place(|| resolve_agent_trace_storage(&context))
        .map_err(|error| TraceSyncError::Runtime(format!("{error:#}")))?;
    let storage_guard = SyncStorageGuard(Some(storage));
    let result = async {
        let storage = storage_guard.storage();

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
    drop(storage_guard);
    result
}

#[cfg(test)]
pub(crate) async fn run_sync_against(
    repository_id: &str,
    source_instance_id: &str,
    db: &RepositoryAgentTraceDb,
    client: &AuthenticatedControlPlaneClient,
) -> Result<AgentTraceSyncReport, TraceSyncError> {
    let mut progress = NoopProgressReporter;
    run_sync_against_with_progress(repository_id, source_instance_id, db, client, &mut progress)
        .await
}

#[cfg(test)]
pub(crate) async fn run_sync_against_with_progress<S>(
    repository_id: &str,
    source_instance_id: &str,
    db: &RepositoryAgentTraceDb,
    client: &AuthenticatedControlPlaneClient,
    progress: &mut S,
) -> Result<AgentTraceSyncReport, TraceSyncError>
where
    S: ProgressReporter<SyncProgressEvent>,
{
    let clock = SystemSyncProgressClock;
    run_sync_against_with_progress_and_clock(
        repository_id,
        source_instance_id,
        db,
        client,
        progress,
        &clock,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn run_sync_against_with_progress_and_clock<S, C>(
    repository_id: &str,
    source_instance_id: &str,
    db: &RepositoryAgentTraceDb,
    client: &AuthenticatedControlPlaneClient,
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
    let result =
        run_sync_against_without_progress(repository_id, source_instance_id, db, client, progress)
            .await;
    progress.report(SyncProgressEvent::Finished {
        timestamp: timestamp(clock),
    });
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
            |cursor, limit| reader.read_messages_after(cursor, limit),
            |request| Box::pin(async move { client.ingest_messages(&request).await }),
            Rc::clone(&progress),
        ),
        sync_one_stream(
            client,
            repository_id,
            source_instance_id,
            IngestionStream::Parts,
            state.cursors.parts,
            "parts",
            |cursor, limit| reader.read_parts_after(cursor, limit),
            |request| Box::pin(async move { client.ingest_parts(&request).await }),
            Rc::clone(&progress),
        ),
        sync_one_stream(
            client,
            repository_id,
            source_instance_id,
            IngestionStream::AgentTraces,
            state.cursors.agent_traces,
            "agent_traces",
            |cursor, limit| reader.read_agent_traces_after(cursor, limit),
            |request| Box::pin(async move { client.ingest_agent_traces(&request).await }),
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
    ReadFn: FnMut(i64, usize) -> anyhow::Result<Vec<T>> + 'a,
    IngestFn: FnMut(
            AgentTraceIngestionBatchRequest<T>,
        )
            -> SyncFuture<'a, Result<AgentTraceIngestionBatchResponse, ControlPlaneError>>
        + 'a,
{
    let uploaded = Rc::new(RefCell::new(0usize));

    let outcome = sync_stream(
        initial_cursor,
        AGENT_TRACE_EXPORT_BATCH_SIZE,
        |cursor, limit| {
            let result = read_after(cursor, limit)
                .map_err(|error| StreamSyncError::Read(format!("{error:#}")));
            Box::pin(std::future::ready(result))
        },
        |cursor, rows: &[T]| {
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
            Box::pin(async move {
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
            })
        },
        || {
            let state_request = AgentTraceIngestionStateRequest {
                repository_id: repository_id.to_string(),
                source_instance_id: source_instance_id.to_string(),
            };
            Box::pin(async move {
                let response = client
                    .ingestion_state(&state_request)
                    .await
                    .map_err(StreamSyncError::Refresh)?;
                Ok(cursor_for_stream(&response.cursors, stream))
            })
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use chrono::{DateTime, Utc};
    use serde_json::json;

    use super::*;
    use crate::services::agent_trace_db::{
        AgentTraceInsert, DiffTraceInsert, InsertMessageInsert, InsertPartInsert, MessageRole,
        PartType, PAYLOAD_TYPE_PATCH,
    };
    use crate::services::agent_trace_export::JS_MAX_SAFE_INTEGER;
    use crate::services::agent_trace_sync::control_plane::CredentialStore;
    use crate::services::agent_trace_sync::test_http_server::{
        CannedResponse, CapturedRequest, ConcurrentBatchTestServer, TestHttpServer,
    };
    use crate::services::auth::TokenResponse;
    use crate::services::token_storage::StoredTokens;

    struct TestDb(Option<RepositoryAgentTraceDb>);

    fn storage_guard_fixture(sandbox: &Path) -> SyncStorageGuard {
        let repo = sandbox.join("repo");
        fs::create_dir_all(&repo).unwrap();
        let output = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(output.status.success());
        let context = AgentTraceStorageContext {
            repository_root: &repo,
            explicit_repository_id: Some("sync-cleanup-boundary"),
            repository_remote: "origin",
        };
        let storage = tokio::task::block_in_place(|| {
            crate::services::agent_trace_storage::resolve_agent_trace_storage_at_state_root(
                &context,
                &sandbox.join("state"),
            )
            .unwrap()
        });
        SyncStorageGuard(Some(storage))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn storage_guard_cleans_up_on_error_before_returning() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut db_path = PathBuf::new();
        let result: Result<(), TraceSyncError> = async {
            let guard = storage_guard_fixture(sandbox.path());
            db_path = guard.storage().db_path.clone();
            seed_one_row_per_stream(&guard.storage().db);
            tokio::task::yield_now().await;
            Err(TraceSyncError::Runtime(
                "fixture failure after storage opened".into(),
            ))?
        }
        .await;
        assert!(
            matches!(result, Err(TraceSyncError::Runtime(reason)) if reason == "fixture failure after storage opened")
        );
        tokio::task::block_in_place(|| {
            let db = RepositoryAgentTraceDb::open_without_migrations_at(&db_path).unwrap();
            assert_eq!(diff_trace_rows(&db).len(), 1);
        });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn storage_guard_cleans_up_when_polled_future_is_abandoned() {
        let sandbox = tempfile::tempdir().unwrap();
        let db_path = RefCell::new(None);
        let mut future = Box::pin(async {
            let guard = storage_guard_fixture(sandbox.path());
            seed_one_row_per_stream(&guard.storage().db);
            *db_path.borrow_mut() = Some(guard.storage().db_path.clone());
            std::future::pending::<()>().await;
            drop(guard);
        });
        poll_fn(|cx| {
            assert!(future.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        assert!(db_path.borrow().is_some());
        drop(future);
        tokio::task::block_in_place(|| {
            let db = RepositoryAgentTraceDb::open_without_migrations_at(
                db_path.borrow().as_ref().unwrap(),
            )
            .unwrap();
            assert_eq!(diff_trace_rows(&db).len(), 1);
        });
    }

    impl TestDb {
        fn new(path: &Path) -> Self {
            Self(Some(tokio::task::block_in_place(|| {
                RepositoryAgentTraceDb::new_at(path).expect("test DB should open")
            })))
        }
    }

    impl std::ops::Deref for TestDb {
        type Target = RepositoryAgentTraceDb;

        fn deref(&self) -> &Self::Target {
            self.0
                .as_ref()
                .expect("test DB remains owned until cleanup")
        }
    }

    impl Drop for TestDb {
        fn drop(&mut self) {
            tokio::task::block_in_place(|| drop(self.0.take()));
        }
    }

    fn unique_test_db_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "sce-trace-sync-{label}-{}-{nonce}",
                std::process::id()
            ))
            .join("agent-trace.db")
    }

    fn remove_test_db(db_path: &std::path::Path) {
        if let Some(parent) = db_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    /// Always reports a still-valid token; the sync tests exercise streaming
    /// behavior, not auth refresh (covered by `control_plane`'s own tests).
    struct AlwaysValidCredentialStore;

    impl CredentialStore for AlwaysValidCredentialStore {
        fn load(&self) -> Result<Option<StoredTokens>, ControlPlaneError> {
            Ok(Some(StoredTokens {
                access_token: "valid-access-token".to_string(),
                token_type: "Bearer".to_string(),
                expires_in: 3_600,
                refresh_token: "refresh-token".to_string(),
                scope: None,
                stored_at_unix_seconds: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system clock before unix epoch")
                    .as_secs(),
            }))
        }

        fn save(&self, _token: &TokenResponse) -> Result<StoredTokens, ControlPlaneError> {
            panic!("no token refresh expected in this test");
        }
    }

    fn test_client(server: &TestHttpServer) -> AuthenticatedControlPlaneClient {
        test_client_at(&server.base_url)
    }

    fn test_client_at(base_url: &str) -> AuthenticatedControlPlaneClient {
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .expect("build test reqwest client");
        AuthenticatedControlPlaneClient::with_credential_store(
            http,
            base_url.to_string(),
            base_url.to_string(),
            "test-client-id",
            Box::new(AlwaysValidCredentialStore),
        )
    }

    fn seed_one_row_per_stream(db: &RepositoryAgentTraceDb) {
        db.insert_messages(vec![InsertMessageInsert {
            session_id: "sess-1".to_string(),
            message_id: "msg-1".to_string(),
            role: MessageRole::User,
            generated_at_unix_ms: 1_700_000_000_000,
        }])
        .expect("seed message");
        db.insert_parts(vec![InsertPartInsert {
            part_type: PartType::Text,
            text: "hello".to_string(),
            session_id: "sess-1".to_string(),
            message_id: "msg-1".to_string(),
            generated_at_unix_ms: 1_700_000_000_000,
        }])
        .expect("seed part");
        db.insert_diff_trace(DiffTraceInsert {
            time_ms: 1_700_000_000_000,
            session_id: "sess-1",
            patch: "Index: a\n",
            model_id: None,
            tool_name: "opencode",
            tool_version: None,
            payload_type: PAYLOAD_TYPE_PATCH,
        })
        .expect("seed diff_trace");
        db.insert_agent_trace(AgentTraceInsert {
            commit_id: "abc123",
            commit_time_ms: 1_700_000_000_000,
            trace_json: "{\"steps\":[]}",
            agent_trace_id: "trace-1",
            url: "https://example.com/trace",
            remote_url: "",
        })
        .expect("seed agent_trace");
    }

    fn seed_messages(db: &RepositoryAgentTraceDb, count: usize) {
        db.insert_messages(
            (1..=count)
                .map(|index| InsertMessageInsert {
                    session_id: "sess-progress".to_string(),
                    message_id: format!("msg-progress-{index}"),
                    role: MessageRole::User,
                    generated_at_unix_ms: 1_700_000_000_000
                        + i64::try_from(index).expect("test message count fits in i64"),
                })
                .collect(),
        )
        .expect("seed progress messages");
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct DiffTraceRowSnapshot {
        id: i64,
        time_ms: i64,
        session_id: String,
        patch: String,
        created_at: String,
        model_id: Option<String>,
        tool_name: Option<String>,
        tool_version: Option<String>,
        payload_type: String,
    }

    fn diff_trace_rows(db: &RepositoryAgentTraceDb) -> Vec<DiffTraceRowSnapshot> {
        db.query_map(
            "SELECT id, time_ms, session_id, patch, created_at, model_id, tool_name, tool_version, payload_type FROM diff_traces ORDER BY id ASC",
            (),
            |row| {
                Ok(DiffTraceRowSnapshot {
                    id: row.get(0)?,
                    time_ms: row.get(1)?,
                    session_id: row.get(2)?,
                    patch: row.get(3)?,
                    created_at: row.get(4)?,
                    model_id: row.get(5)?,
                    tool_name: row.get(6)?,
                    tool_version: row.get(7)?,
                    payload_type: row.get(8)?,
                })
            },
        )
        .expect("diff_traces snapshot query should succeed")
    }

    fn insert_diff_trace_row(db: &RepositoryAgentTraceDb, id: i64, time_ms: i64) {
        db.execute(
            "INSERT INTO diff_traces (id, time_ms, session_id, patch, model_id, tool_name, tool_version, payload_type) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                id,
                time_ms,
                "sess-1",
                "Index: a\n",
                Option::<&str>::None,
                Option::<&str>::None,
                Option::<&str>::None,
                PAYLOAD_TYPE_PATCH,
            ),
        )
        .expect("direct diff_trace insert should succeed");
    }

    fn diff_traces_compatibility_report(server_cursor: i64) -> StreamSyncReport {
        StreamSyncReport {
            uploaded: 0,
            initial_cursor: server_cursor,
            final_cursor: server_cursor,
            batches: 0,
        }
    }

    fn assert_state_request(
        request: &CapturedRequest,
        repository_id: &str,
        source_instance_id: &str,
    ) {
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/agent-trace/ingestion/state");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&request.body)
                .expect("state request body should be JSON"),
            json!({
                "repositoryId": repository_id,
                "sourceInstanceId": source_instance_id,
            })
        );
    }

    fn state_request_count(requests: &[CapturedRequest]) -> usize {
        requests
            .iter()
            .filter(|request| request.path == "/agent-trace/ingestion/state")
            .count()
    }

    fn sorted_batch_streams(requests: &[CapturedRequest]) -> Vec<String> {
        let mut streams = requests
            .iter()
            .filter(|request| request.path == "/agent-trace/ingestion/batch")
            .map(|request| {
                serde_json::from_str::<serde_json::Value>(&request.body)
                    .expect("batch request body should be JSON")["stream"]
                    .as_str()
                    .expect("batch request should name its stream")
                    .to_string()
            })
            .collect::<Vec<_>>();
        streams.sort();
        streams
    }

    fn state_response(
        messages: i64,
        parts: i64,
        diff_traces: i64,
        agent_traces: i64,
    ) -> serde_json::Value {
        json!({
            "cursors": {
                "messages": messages,
                "parts": parts,
                "diffTraces": diff_traces,
                "agentTraces": agent_traces,
            }
        })
    }

    fn batch_response(cursor: i64) -> serde_json::Value {
        json!({ "accepted": 1, "cursor": cursor })
    }

    struct FixedProgressClock {
        timestamps: RefCell<Vec<DateTime<Utc>>>,
    }

    impl SyncProgressClock for FixedProgressClock {
        fn now(&self) -> DateTime<Utc> {
            self.timestamps.borrow_mut().remove(0)
        }
    }

    fn fixed_progress_clock() -> FixedProgressClock {
        FixedProgressClock {
            timestamps: RefCell::new(vec![
                DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
                    .expect("valid start timestamp")
                    .with_timezone(&Utc),
                DateTime::parse_from_rfc3339("2026-01-02T03:04:06Z")
                    .expect("valid end timestamp")
                    .with_timezone(&Utc),
            ]),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn progress_events_cover_batches_empty_streams_and_fixed_order() {
        let db_path = unique_test_db_path("progress-events");
        let db = TestDb::new(&db_path);
        let metadata = db
            .verify_or_initialize_repository_metadata("repo-progress-events")
            .expect("metadata should initialize");
        seed_messages(&db, 201);

        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(200, &state_response(0, 0, 0, 0)));
        server.queue_response(CannedResponse::json(
            200,
            &json!({
                "accepted": 100,
                "cursor": 100,
            }),
        ));
        server.queue_response(CannedResponse::json(
            200,
            &json!({
                "accepted": 100,
                "cursor": 200,
            }),
        ));
        server.queue_response(CannedResponse::json(
            200,
            &json!({
                "accepted": 1,
                "cursor": 201,
            }),
        ));
        let client = test_client(&server);
        let events = RefCell::new(Vec::new());
        let clock = fixed_progress_clock();

        let report = run_sync_against_with_progress_and_clock(
            &metadata.repository_id,
            &metadata.source_instance_id,
            &db,
            &client,
            &mut |event| events.borrow_mut().push(event),
            &clock,
        )
        .await
        .expect("sync should succeed");

        assert_eq!(report.streams.messages.uploaded, 201);
        assert_eq!(report.streams.messages.batches, 3);
        assert_eq!(
            events.into_inner(),
            vec![
                SyncProgressEvent::Started {
                    timestamp: "2026-01-02T03:04:05Z".to_string(),
                },
                SyncProgressEvent::StreamCompleted {
                    stream: "parts",
                    uploaded: 0,
                    cursor: 0,
                    batches: 0,
                },
                SyncProgressEvent::StreamCompleted {
                    stream: "agent_traces",
                    uploaded: 0,
                    cursor: 0,
                    batches: 0,
                },
                SyncProgressEvent::BatchAccepted {
                    stream: "messages",
                    batch_rows: 100,
                    uploaded: 100,
                    cursor: 100,
                },
                SyncProgressEvent::BatchAccepted {
                    stream: "messages",
                    batch_rows: 100,
                    uploaded: 200,
                    cursor: 200,
                },
                SyncProgressEvent::BatchAccepted {
                    stream: "messages",
                    batch_rows: 1,
                    uploaded: 201,
                    cursor: 201,
                },
                SyncProgressEvent::StreamCompleted {
                    stream: "messages",
                    uploaded: 201,
                    cursor: 201,
                    batches: 3,
                },
                SyncProgressEvent::Finished {
                    timestamp: "2026-01-02T03:04:06Z".to_string(),
                },
            ]
        );

        remove_test_db(&db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn full_sync_uploads_three_remote_streams_and_second_run_is_naturally_incremental() {
        let db_path = unique_test_db_path("full-sync");
        let db = TestDb::new(&db_path);
        let metadata = db
            .verify_or_initialize_repository_metadata("repo-full-sync")
            .expect("metadata should initialize");
        seed_one_row_per_stream(&db);
        let diff_traces_before = diff_trace_rows(&db);
        assert_eq!(diff_traces_before.len(), 1);
        let unsynced_diff_traces = diff_traces_compatibility_report(0);

        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(200, &state_response(0, 0, 0, 0)));
        server.queue_response(CannedResponse::json(200, &batch_response(1)));
        server.queue_response(CannedResponse::json(200, &batch_response(1)));
        server.queue_response(CannedResponse::json(200, &batch_response(1)));
        let client = test_client(&server);

        let report = run_sync_against(
            &metadata.repository_id,
            &metadata.source_instance_id,
            &db,
            &client,
        )
        .await
        .expect("first sync should succeed");

        let first_run_requests = server.captured_requests();
        assert_eq!(first_run_requests.len(), 4);
        assert_state_request(
            &first_run_requests[0],
            &metadata.repository_id,
            &metadata.source_instance_id,
        );
        assert_eq!(state_request_count(&first_run_requests), 1);
        assert_eq!(
            sorted_batch_streams(&first_run_requests),
            vec!["agent_traces", "messages", "parts"]
        );
        for stream in [
            report.streams.messages,
            report.streams.parts,
            report.streams.agent_traces,
        ] {
            assert_eq!(stream.uploaded, 1);
            assert_eq!(stream.initial_cursor, 0);
            assert_eq!(stream.final_cursor, 1);
            assert_eq!(stream.batches, 1);
        }
        assert_eq!(report.streams.diff_traces, unsynced_diff_traces);
        assert_eq!(diff_trace_rows(&db), diff_traces_before);

        server.queue_response(CannedResponse::json(200, &state_response(1, 1, 0, 1)));

        let second_report = run_sync_against(
            &metadata.repository_id,
            &metadata.source_instance_id,
            &db,
            &client,
        )
        .await
        .expect("second sync should succeed");

        let all_requests = server.captured_requests();
        assert_eq!(
            all_requests.len(),
            5,
            "second run should only re-check /state and re-read no already-synced rows"
        );
        assert_eq!(all_requests[4].path, "/agent-trace/ingestion/state");
        assert_eq!(
            sorted_batch_streams(&all_requests),
            vec!["agent_traces", "messages", "parts"]
        );
        assert!(!sorted_batch_streams(&all_requests).contains(&"diff_traces".to_string()));
        for stream in [
            second_report.streams.messages,
            second_report.streams.parts,
            second_report.streams.agent_traces,
        ] {
            assert_eq!(stream.uploaded, 0);
            assert_eq!(stream.initial_cursor, 1);
            assert_eq!(stream.final_cursor, 1);
            assert_eq!(stream.batches, 0);
        }
        assert_eq!(second_report.streams.diff_traces, unsynced_diff_traces);
        assert_eq!(diff_trace_rows(&db), diff_traces_before);

        let db_file_name = db_path
            .file_name()
            .expect("db path has a file name")
            .to_string_lossy()
            .into_owned();
        assert!(
            !db_path
                .parent()
                .expect("db has parent dir")
                .read_dir()
                .expect("read db dir")
                .filter_map(Result::ok)
                .any(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    // Turso may write WAL/SHM sidecar files alongside the DB
                    // itself; only reject files unrelated to that one DB, which
                    // would indicate a local sync cursor/DB sneaking in.
                    !name.starts_with(&db_file_name)
                }),
            "sync must not create any local cursor file/database/table on disk"
        );

        remove_test_db(&db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sync_succeeds_when_local_diff_trace_row_is_rejected_by_export_reader() {
        let db_path = unique_test_db_path("rejected-diff-trace");
        let db = TestDb::new(&db_path);
        let metadata = db
            .verify_or_initialize_repository_metadata("repo-rejected-diff-trace")
            .expect("metadata should initialize");
        seed_messages(&db, 1);
        insert_diff_trace_row(&db, 1, JS_MAX_SAFE_INTEGER + 1);
        let diff_traces_before = diff_trace_rows(&db);

        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(200, &state_response(0, 0, 0, 0)));
        server.queue_response(CannedResponse::json(200, &batch_response(1)));
        let client = test_client(&server);

        let report = run_sync_against(
            &metadata.repository_id,
            &metadata.source_instance_id,
            &db,
            &client,
        )
        .await
        .expect("sync should not read local diff_traces rows");

        let requests = server.captured_requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(sorted_batch_streams(&requests), vec!["messages"]);
        assert_eq!(report.streams.messages.uploaded, 1);
        assert_eq!(
            report.streams.diff_traces,
            diff_traces_compatibility_report(0)
        );
        assert_eq!(diff_trace_rows(&db), diff_traces_before);

        remove_test_db(&db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn diff_traces_report_echoes_server_cursor_without_uploading_newer_local_rows() {
        let db_path = unique_test_db_path("diff-traces-compat");
        let db = TestDb::new(&db_path);
        let metadata = db
            .verify_or_initialize_repository_metadata("repo-diff-traces-compat")
            .expect("metadata should initialize");
        seed_one_row_per_stream(&db);
        insert_diff_trace_row(&db, 124, 1_700_000_000_124);
        let diff_traces_before = diff_trace_rows(&db);
        assert_eq!(diff_traces_before.len(), 2);

        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(200, &state_response(1, 1, 123, 1)));
        let client = test_client(&server);

        let report = run_sync_against(
            &metadata.repository_id,
            &metadata.source_instance_id,
            &db,
            &client,
        )
        .await
        .expect("sync should succeed");

        let requests = server.captured_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(state_request_count(&requests), 1);
        assert!(sorted_batch_streams(&requests).is_empty());
        assert_eq!(
            report.streams.diff_traces,
            diff_traces_compatibility_report(123)
        );
        assert_eq!(diff_trace_rows(&db), diff_traces_before);

        remove_test_db(&db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_sync_overlaps_all_three_stream_batches_after_one_state_request() {
        let db_path = unique_test_db_path("concurrent-overlap");
        let db = TestDb::new(&db_path);
        let metadata = db
            .verify_or_initialize_repository_metadata("repo-concurrent-overlap")
            .expect("metadata should initialize");
        seed_one_row_per_stream(&db);

        let server = ConcurrentBatchTestServer::start(Duration::from_millis(100));
        server.queue_state_response(CannedResponse::json(200, &state_response(0, 0, 0, 0)));
        for stream in ["messages", "parts", "agent_traces"] {
            server.queue_batch_response(stream, 0, CannedResponse::json(200, &batch_response(1)));
        }
        let client = test_client_at(&server.base_url);

        let report = run_sync_against(
            &metadata.repository_id,
            &metadata.source_instance_id,
            &db,
            &client,
        )
        .await
        .expect("concurrent sync should succeed");

        assert_eq!(server.state_request_count(), 1);
        assert_eq!(server.captured_requests().len(), 4);
        assert_eq!(server.max_in_flight(), 3);
        for stream in ["messages", "parts", "agent_traces"] {
            assert_eq!(server.max_in_flight_for(stream), 1, "stream {stream}");
            assert_eq!(server.expected_cursors_for(stream), vec![0]);
        }
        assert_eq!(server.max_in_flight_for("diff_traces"), 0);
        assert!(server.expected_cursors_for("diff_traces").is_empty());
        for stream in [
            report.streams.messages,
            report.streams.parts,
            report.streams.agent_traces,
        ] {
            assert_eq!(stream.uploaded, 1);
            assert_eq!(stream.final_cursor, 1);
            assert_eq!(stream.batches, 1);
        }

        remove_test_db(&db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_sync_keeps_batches_sequential_within_one_stream() {
        let db_path = unique_test_db_path("concurrent-ordering");
        let db = TestDb::new(&db_path);
        let metadata = db
            .verify_or_initialize_repository_metadata("repo-concurrent-ordering")
            .expect("metadata should initialize");
        seed_messages(&db, 201);

        let server = ConcurrentBatchTestServer::start(Duration::from_millis(50));
        server.queue_state_response(CannedResponse::json(200, &state_response(0, 0, 0, 0)));
        server.queue_batch_response(
            "messages",
            0,
            CannedResponse::json(
                200,
                &json!({
                    "accepted": 100,
                    "cursor": 100,
                }),
            ),
        );
        server.queue_batch_response(
            "messages",
            100,
            CannedResponse::json(
                200,
                &json!({
                    "accepted": 100,
                    "cursor": 200,
                }),
            ),
        );
        server.queue_batch_response(
            "messages",
            200,
            CannedResponse::json(
                200,
                &json!({
                    "accepted": 1,
                    "cursor": 201,
                }),
            ),
        );
        let client = test_client_at(&server.base_url);

        let report = run_sync_against(
            &metadata.repository_id,
            &metadata.source_instance_id,
            &db,
            &client,
        )
        .await
        .expect("ordered multi-batch sync should succeed");

        assert_eq!(server.state_request_count(), 1);
        assert_eq!(server.max_in_flight(), 1);
        assert_eq!(server.max_in_flight_for("messages"), 1);
        assert_eq!(server.expected_cursors_for("messages"), vec![0, 100, 200]);
        assert_eq!(report.streams.messages.uploaded, 201);
        assert_eq!(report.streams.messages.final_cursor, 201);
        assert_eq!(report.streams.messages.batches, 3);

        remove_test_db(&db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invalid_state_cursor_fails_before_any_batch_request() {
        let db_path = unique_test_db_path("invalid-cursor");
        let db = TestDb::new(&db_path);
        let metadata = db
            .verify_or_initialize_repository_metadata("repo-invalid-cursor")
            .expect("metadata should initialize");
        seed_one_row_per_stream(&db);

        let server = TestHttpServer::start();
        // `agentTraces` is one past JS_MAX_SAFE_INTEGER, outside the wire
        // contract's representable cursor range.
        server.queue_response(CannedResponse::json(
            200,
            &state_response(0, 0, 0, 9_007_199_254_740_992),
        ));
        let client = test_client(&server);

        let error = run_sync_against(
            &metadata.repository_id,
            &metadata.source_instance_id,
            &db,
            &client,
        )
        .await
        .expect_err("out-of-range /state cursor should fail sync");

        assert!(matches!(
            error,
            TraceSyncError::ControlPlane(ControlPlaneError::InvalidResponse(_))
        ));
        assert_eq!(
            server.call_count(),
            1,
            "an invalid /state cursor must fail before any /batch request is sent"
        );

        remove_test_db(&db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn terminal_batch_status_fails_without_state_reconciliation() {
        let db_path = unique_test_db_path("terminal-batch");
        let db = TestDb::new(&db_path);
        let metadata = db
            .verify_or_initialize_repository_metadata("repo-terminal-batch")
            .expect("metadata should initialize");
        seed_one_row_per_stream(&db);

        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(200, &state_response(0, 1, 1, 1)));
        server.queue_response(CannedResponse::json(
            404,
            &json!({"message": "unknown ingestion route"}),
        ));
        let client = test_client(&server);

        let error = run_sync_against(
            &metadata.repository_id,
            &metadata.source_instance_id,
            &db,
            &client,
        )
        .await
        .expect_err("a terminal 404 /batch response should fail the sync");

        assert!(
            matches!(
                error,
                TraceSyncError::Stream {
                    stream: "messages",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
        let requests = server.captured_requests();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/agent-trace/ingestion/state")
                .count(),
            1,
            "terminal /batch statuses must not trigger a /state refetch"
        );
        let batch_count = requests
            .iter()
            .filter(|request| request.path == "/agent-trace/ingestion/batch")
            .count();
        assert_eq!(
            batch_count, 1,
            "only `messages` should send a /batch request, and a terminal status must not resend it; observed {batch_count}"
        );

        remove_test_db(&db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn progress_events_end_after_terminal_failure() {
        let db_path = unique_test_db_path("progress-failure");
        let db = TestDb::new(&db_path);
        let metadata = db
            .verify_or_initialize_repository_metadata("repo-progress-failure")
            .expect("metadata should initialize");
        seed_one_row_per_stream(&db);

        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(200, &state_response(0, 0, 0, 0)));
        for _ in 0..3 {
            server.queue_response(CannedResponse::json(
                404,
                &json!({"message": "unknown ingestion route"}),
            ));
        }
        let client = test_client(&server);
        let events = RefCell::new(Vec::new());
        let clock = fixed_progress_clock();

        let error = run_sync_against_with_progress_and_clock(
            &metadata.repository_id,
            &metadata.source_instance_id,
            &db,
            &client,
            &mut |event| events.borrow_mut().push(event),
            &clock,
        )
        .await
        .expect_err("terminal batch failure should be reported");

        assert!(matches!(error, TraceSyncError::Stream { .. }));
        assert_eq!(
            events.into_inner(),
            vec![
                SyncProgressEvent::Started {
                    timestamp: "2026-01-02T03:04:05Z".to_string(),
                },
                SyncProgressEvent::Finished {
                    timestamp: "2026-01-02T03:04:06Z".to_string(),
                },
            ]
        );
        remove_test_db(&db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn malformed_2xx_batch_response_still_reconciles_via_state() {
        let db_path = unique_test_db_path("malformed-batch");
        let db = TestDb::new(&db_path);
        let metadata = db
            .verify_or_initialize_repository_metadata("repo-malformed-batch")
            .expect("metadata should initialize");
        seed_messages(&db, 1);

        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(200, &state_response(0, 0, 0, 0)));
        // Syntactically successful but undecodable as `AgentTraceIngestionBatchResponse`.
        server.queue_response(CannedResponse::json(200, &json!({"unexpected": "shape"})));
        // Only messages has pending data in this test. The malformed successful
        // batch response must cause an explicit state reconciliation before the
        // same batch is retried.
        server.queue_response(CannedResponse::json(200, &state_response(0, 0, 0, 0)));
        server.queue_response(CannedResponse::json(200, &batch_response(1)));
        let client = test_client(&server);

        let report = run_sync_against(
            &metadata.repository_id,
            &metadata.source_instance_id,
            &db,
            &client,
        )
        .await
        .expect("an undecodable 2xx /batch body should still reconcile via /state and succeed");

        assert_eq!(server.call_count(), 4);
        let requests = server.captured_requests();
        assert_eq!(
            requests
                .iter()
                .map(|request| (request.method.as_str(), request.path.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("POST", "/agent-trace/ingestion/state"),
                ("POST", "/agent-trace/ingestion/batch"),
                ("POST", "/agent-trace/ingestion/state"),
                ("POST", "/agent-trace/ingestion/batch"),
            ],
            "an undecodable 2xx /batch body must reconcile via /state before retrying the same batch"
        );
        assert_eq!(report.streams.messages.uploaded, 1);
        assert_eq!(report.streams.messages.final_cursor, 1);
        assert_eq!(report.streams.parts.uploaded, 0);
        assert_eq!(report.streams.diff_traces.uploaded, 0);
        assert_eq!(report.streams.agent_traces.uploaded, 0);

        remove_test_db(&db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn forbidden_state_response_fails_without_mutating_local_metadata() {
        let db_path = unique_test_db_path("forbidden");
        let db = TestDb::new(&db_path);
        let metadata_before = db
            .verify_or_initialize_repository_metadata("repo-forbidden")
            .expect("metadata should initialize");

        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(
            403,
            &json!({"error": "not the owner"}),
        ));
        let client = test_client(&server);

        let error = run_sync_against(
            &metadata_before.repository_id,
            &metadata_before.source_instance_id,
            &db,
            &client,
        )
        .await
        .expect_err("403 should fail the sync");

        assert!(matches!(
            error,
            TraceSyncError::ControlPlane(ControlPlaneError::Forbidden(_))
        ));

        let metadata_after = db
            .verify_or_initialize_repository_metadata("repo-forbidden")
            .expect("metadata should still verify");
        assert_eq!(
            metadata_after.source_instance_id,
            metadata_before.source_instance_id
        );
        assert_eq!(
            server.call_count(),
            1,
            "no reconciliation call should follow a terminal /state failure"
        );

        remove_test_db(&db_path);
    }
}
