//! `sce trace sync` orchestration: resolves the current repository's Agent
//! Trace storage, fetches authoritative control-plane cursors, then
//! synchronizes each of the four independent capture streams in the fixed
//! `messages -> parts -> diff_traces -> agent_traces` order.
//!
//! Consumes the already-shipped [`crate::services::agent_trace_sync`] engine
//! and [`crate::services::agent_trace_sync::control_plane`] client as-is; adds
//! no local sync cursor or persisted progress of its own.

use std::cell::RefCell;
use std::fmt;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::Context;
use tokio::runtime::Runtime;

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

static SYNC_RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Full sync result across all four capture streams, ready for rendering.
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

/// Terminal failure of `sce trace sync`.
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

/// Resolves the current repository's Agent Trace storage (the same
/// `ContextWithRepoRoot`/`AgentTraceStorageContext`/`resolve_agent_trace_storage`
/// path `sce trace status` uses) and control-plane configuration, then
/// synchronizes all four capture streams.
pub fn run_current_sync(repo_root: &Path) -> Result<AgentTraceSyncReport, TraceSyncError> {
    let storage_config = config::resolve_agent_trace_storage_runtime_config(repo_root)
        .map_err(|error| TraceSyncError::Runtime(format!("{error:#}")))?;
    let context = AgentTraceStorageContext {
        repository_root: repo_root,
        explicit_repository_id: storage_config.repository_id.as_deref(),
        repository_remote: &storage_config.repository_remote,
    };
    let storage = resolve_agent_trace_storage(&context)
        .map_err(|error| TraceSyncError::Runtime(format!("{error:#}")))?;

    let auth_config = config::resolve_auth_runtime_config(repo_root)
        .map_err(|error| TraceSyncError::Runtime(format!("{error:#}")))?;
    let client = AuthenticatedControlPlaneClient::new(
        reqwest::Client::new(),
        auth_config.control_plane_base_url.value.unwrap_or_default(),
        auth::WORKOS_DEFAULT_BASE_URL,
        auth_config.workos_client_id.value.unwrap_or_default(),
    );

    run_sync_against(
        &storage.metadata.repository_id,
        &storage.metadata.source_instance_id,
        &storage.db,
        &client,
    )
}

/// Testable core: synchronizes all four streams given an already-resolved
/// repository identity, an open Agent Trace database, and a configured
/// control-plane client (production or test-double).
pub(crate) fn run_sync_against(
    repository_id: &str,
    source_instance_id: &str,
    db: &RepositoryAgentTraceDb,
    client: &AuthenticatedControlPlaneClient,
) -> Result<AgentTraceSyncReport, TraceSyncError> {
    let runtime = shared_runtime()?;
    let reader = AgentTraceExportReader::new(db);

    let state_request = AgentTraceIngestionStateRequest {
        repository_id: repository_id.to_string(),
        source_instance_id: source_instance_id.to_string(),
    };
    let state = runtime
        .block_on(client.ingestion_state(&state_request))
        .map_err(TraceSyncError::ControlPlane)?;

    let messages = sync_one_stream(
        runtime,
        client,
        repository_id,
        source_instance_id,
        IngestionStream::Messages,
        state.cursors.messages,
        "messages",
        |cursor, limit| reader.read_messages_after(cursor, limit),
        |request| runtime.block_on(client.ingest_messages(request)),
    )?;
    let parts = sync_one_stream(
        runtime,
        client,
        repository_id,
        source_instance_id,
        IngestionStream::Parts,
        state.cursors.parts,
        "parts",
        |cursor, limit| reader.read_parts_after(cursor, limit),
        |request| runtime.block_on(client.ingest_parts(request)),
    )?;
    let diff_traces = sync_one_stream(
        runtime,
        client,
        repository_id,
        source_instance_id,
        IngestionStream::DiffTraces,
        state.cursors.diff_traces,
        "diff_traces",
        |cursor, limit| reader.read_diff_traces_after(cursor, limit),
        |request| runtime.block_on(client.ingest_diff_traces(request)),
    )?;
    let agent_traces = sync_one_stream(
        runtime,
        client,
        repository_id,
        source_instance_id,
        IngestionStream::AgentTraces,
        state.cursors.agent_traces,
        "agent_traces",
        |cursor, limit| reader.read_agent_traces_after(cursor, limit),
        |request| runtime.block_on(client.ingest_agent_traces(request)),
    )?;

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

/// Synchronizes one stream via the T04 engine. Genuine `409`/`5xx`/transport
/// ambiguity reconciles through a real `/state` refetch; a terminal
/// control-plane failure (missing/invalid auth, `400`, `403`) short-circuits
/// the reconciliation closure with that failure instead of issuing another
/// network call, so a `403` never mutates local state or retries.
#[allow(clippy::too_many_arguments)]
fn sync_one_stream<T, ReadFn, IngestFn>(
    runtime: &Runtime,
    client: &AuthenticatedControlPlaneClient,
    repository_id: &str,
    source_instance_id: &str,
    stream: IngestionStream,
    initial_cursor: i64,
    stream_label: &'static str,
    mut read_after: ReadFn,
    mut ingest: IngestFn,
) -> Result<StreamSyncReport, TraceSyncError>
where
    T: AgentTraceExportRow + Clone,
    ReadFn: FnMut(i64, usize) -> anyhow::Result<Vec<T>>,
    IngestFn: FnMut(
        &AgentTraceIngestionBatchRequest<T>,
    ) -> Result<AgentTraceIngestionBatchResponse, ControlPlaneError>,
{
    let terminal: RefCell<Option<ControlPlaneError>> = RefCell::new(None);

    let outcome = sync_stream(
        initial_cursor,
        AGENT_TRACE_EXPORT_BATCH_SIZE,
        |cursor, limit| {
            read_after(cursor, limit).map_err(|error| StreamSyncError::Read(format!("{error:#}")))
        },
        |cursor, rows: &[T]| {
            let request = AgentTraceIngestionBatchRequest {
                repository_id: repository_id.to_string(),
                source_instance_id: source_instance_id.to_string(),
                stream,
                expected_cursor: cursor,
                rows: rows.to_vec(),
            };
            match ingest(&request) {
                Ok(response) => BatchAttemptOutcome::Accepted {
                    accepted: response.accepted,
                    cursor: response.cursor,
                },
                Err(ControlPlaneError::Conflict(_)) => BatchAttemptOutcome::Conflict,
                Err(error) if is_stream_terminal(&error) => {
                    *terminal.borrow_mut() = Some(error);
                    BatchAttemptOutcome::Ambiguous
                }
                Err(_) => BatchAttemptOutcome::Ambiguous,
            }
        },
        || {
            if let Some(error) = terminal.borrow_mut().take() {
                return Err(StreamSyncError::Refresh(error.to_string()));
            }

            let state_request = AgentTraceIngestionStateRequest {
                repository_id: repository_id.to_string(),
                source_instance_id: source_instance_id.to_string(),
            };
            let response = runtime
                .block_on(client.ingestion_state(&state_request))
                .map_err(|error| StreamSyncError::Refresh(error.to_string()))?;
            Ok(cursor_for_stream(&response.cursors, stream))
        },
    )
    .map_err(|source| TraceSyncError::Stream {
        stream: stream_label,
        source,
    })?;

    Ok(StreamSyncReport {
        uploaded: outcome.uploaded,
        initial_cursor: outcome.initial_cursor,
        final_cursor: outcome.final_cursor,
        batches: outcome.batches,
    })
}

/// A control-plane failure that cannot be resolved by reconciling with
/// `/state`: missing/invalid credentials, an unrecoverable `401`, a `400`, or
/// a `403` ownership rejection. `5xx`, transport failures, and invalid batch
/// responses are genuinely ambiguous and reconcile via a real `/state` call.
fn is_stream_terminal(error: &ControlPlaneError) -> bool {
    matches!(
        error,
        ControlPlaneError::MissingCredentials
            | ControlPlaneError::AuthenticationFailed(_)
            | ControlPlaneError::BadRequest(_)
            | ControlPlaneError::Forbidden(_)
            | ControlPlaneError::Storage(_)
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

fn shared_runtime() -> Result<&'static Runtime, TraceSyncError> {
    if let Some(runtime) = SYNC_RUNTIME.get() {
        return Ok(runtime);
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .context("failed to create trace sync command runtime")
        .map_err(|error| TraceSyncError::Runtime(format!("{error:#}")))?;

    Ok(SYNC_RUNTIME.get_or_init(|| runtime))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::*;
    use crate::services::agent_trace_db::{
        AgentTraceInsert, DiffTraceInsert, InsertMessageInsert, InsertPartInsert, MessageRole,
        PartType, PAYLOAD_TYPE_PATCH,
    };
    use crate::services::agent_trace_sync::control_plane::CredentialStore;
    use crate::services::agent_trace_sync::test_http_server::{CannedResponse, TestHttpServer};
    use crate::services::auth::TokenResponse;
    use crate::services::token_storage::StoredTokens;

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
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .expect("build test reqwest client");
        AuthenticatedControlPlaneClient::with_credential_store(
            http,
            server.base_url.clone(),
            server.base_url.clone(),
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

    #[test]
    fn full_sync_uploads_all_four_streams_and_second_run_is_naturally_incremental() {
        let db_path = unique_test_db_path("full-sync");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        let metadata = db
            .verify_or_initialize_repository_metadata("repo-full-sync")
            .expect("metadata should initialize");
        seed_one_row_per_stream(&db);

        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(200, &state_response(0, 0, 0, 0)));
        server.queue_response(CannedResponse::json(200, &batch_response(1)));
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
        .expect("first sync should succeed");

        assert_eq!(server.call_count(), 5);
        for stream in [
            report.streams.messages,
            report.streams.parts,
            report.streams.diff_traces,
            report.streams.agent_traces,
        ] {
            assert_eq!(stream.uploaded, 1);
            assert_eq!(stream.initial_cursor, 0);
            assert_eq!(stream.final_cursor, 1);
            assert_eq!(stream.batches, 1);
        }

        server.queue_response(CannedResponse::json(200, &state_response(1, 1, 1, 1)));

        let second_report = run_sync_against(
            &metadata.repository_id,
            &metadata.source_instance_id,
            &db,
            &client,
        )
        .expect("second sync should succeed");

        assert_eq!(
            server.call_count(),
            6,
            "second run should only re-check /state and re-read no already-synced rows"
        );
        for stream in [
            second_report.streams.messages,
            second_report.streams.parts,
            second_report.streams.diff_traces,
            second_report.streams.agent_traces,
        ] {
            assert_eq!(stream.uploaded, 0);
            assert_eq!(stream.initial_cursor, 1);
            assert_eq!(stream.final_cursor, 1);
            assert_eq!(stream.batches, 0);
        }

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

    #[test]
    fn forbidden_state_response_fails_without_mutating_local_metadata() {
        let db_path = unique_test_db_path("forbidden");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
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
