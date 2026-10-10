//! Wire-contract DTOs for the control-plane Agent Trace ingestion API.
//!
//! These types define the request/response shapes for
//! `GET /me`, `POST /agent-trace/ingestion/state`, and
//! `POST /agent-trace/ingestion/batch`.
//! They perform no HTTP I/O and hold no cursor state themselves.

use std::fmt;
use std::future::Future;
use std::sync::Arc;

use anyhow::anyhow;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::services::agent_trace_export::{
    self, AgentTraceAgentTraceExportRow, AgentTraceDiffTraceExportRow, AgentTraceMessageExportRow,
    AgentTracePartExportRow,
};
use crate::services::auth::{self, AuthError, TokenResponse};
use crate::services::resilience::{run_with_retry, RetryPolicy};
use crate::services::token_storage::{self, StoredTokens, TokenStorageError};

/// One of the four independent Agent Trace capture streams, identified by its
/// literal wire value. These values are always the `snake_case` stream
/// identifiers (`messages`, `parts`, `diff_traces`, `agent_traces`), distinct
/// from the `camelCase` field names (`diffTraces`, `agentTraces`) used in
/// [`AgentTraceCursors`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestionStream {
    Messages,
    Parts,
    DiffTraces,
    AgentTraces,
}

/// Request body for `POST /agent-trace/ingestion/state`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceIngestionStateRequest {
    pub repository_id: String,
    pub source_instance_id: String,
}

/// Authoritative server-side cursor for each of the four capture streams, as
/// returned by `/state`. Each cursor is the last `source_row_id` the control
/// plane has accepted for that stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceCursors {
    pub messages: i64,
    pub parts: i64,
    pub diff_traces: i64,
    pub agent_traces: i64,
}

impl AgentTraceCursors {
    /// Rejects any field outside `0..=JS_MAX_SAFE_INTEGER`, the range an
    /// exportable cursor must stay within to survive JSON round-trip without
    /// truncation or casting. The control plane is the authoritative source
    /// of cursor progress, but a syntactically valid `/state` response can
    /// still carry a value the wire contract cannot represent; this rejects
    /// it before it reaches any export reader or the sync engine.
    fn validate(&self) -> Result<(), ControlPlaneError> {
        for (name, value) in [
            ("messages", self.messages),
            ("parts", self.parts),
            ("diffTraces", self.diff_traces),
            ("agentTraces", self.agent_traces),
        ] {
            agent_trace_export::validate_js_safe_integer(value).map_err(|error| {
                ControlPlaneError::InvalidResponse(format!("cursors.{name} is invalid: {error}"))
            })?;
        }

        Ok(())
    }
}

/// Response body for `POST /agent-trace/ingestion/state`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceIngestionStateResponse {
    pub cursors: AgentTraceCursors,
}

/// Request body for `POST /agent-trace/ingestion/batch`, generic over the
/// PR #198 export row type carried by the stream being uploaded.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceIngestionBatchRequest<T> {
    pub repository_id: String,
    pub source_instance_id: String,
    pub stream: IngestionStream,
    pub expected_cursor: i64,
    pub rows: Vec<T>,
}

/// Response body for `POST /agent-trace/ingestion/batch`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceIngestionBatchResponse {
    pub accepted: usize,
    pub cursor: i64,
}

const STATE_PATH: &str = "agent-trace/ingestion/state";
const BATCH_PATH: &str = "agent-trace/ingestion/batch";
const ME_PATH: &str = "me";

const STATE_RETRY_POLICY: RetryPolicy = RetryPolicy::builtin(1, 60_000, 250, 2_000);

/// Typed failure classification for control-plane HTTP interactions, kept
/// separate from `CliError` so the sync engine and CLI wiring can
/// react to each case before deciding how to surface it.
#[derive(Debug)]
pub enum ControlPlaneError {
    MissingCredentials,
    AuthenticationFailed(String),
    Transport(String),
    BadRequest(String),
    Forbidden(String),
    Conflict(String),
    ServerError(String),
    InvalidResponse(String),
    Storage(String),
    /// A terminal protocol/API mismatch during a control-plane request (e.g.
    /// `404`/`405`/`415`/`422`), distinct from `InvalidResponse` which is
    /// reserved for a syntactically successful (`2xx`) but undecodable body.
    Protocol {
        status: StatusCode,
        message: String,
    },
}

impl fmt::Display for ControlPlaneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCredentials => write!(
                f,
                "No stored WorkOS credentials were found. Try: run 'sce auth login' before running 'sce sync'."
            ),
            Self::AuthenticationFailed(reason) => write!(
                f,
                "Control-plane authentication failed: {reason}. Try: run 'sce auth login' to re-authenticate."
            ),
            Self::Transport(reason) => write!(f, "Control-plane request failed: {reason}"),
            Self::BadRequest(reason) => {
                write!(f, "Control-plane rejected the request as invalid: {reason}")
            }
            Self::Forbidden(reason) => write!(f, "Control-plane denied the request: {reason}"),
            Self::Conflict(reason) => {
                write!(f, "Control-plane reported a cursor conflict: {reason}")
            }
            Self::ServerError(reason) => {
                write!(f, "Control-plane request failed with a server error: {reason}")
            }
            Self::InvalidResponse(reason) => {
                write!(f, "Control-plane returned an unexpected response: {reason}")
            }
            Self::Storage(reason) => write!(f, "Local credential storage error: {reason}"),
            Self::Protocol { status, message } => write!(
                f,
                "Control-plane rejected the request ({status}): {message}"
            ),
        }
    }
}

impl std::error::Error for ControlPlaneError {}

impl ControlPlaneError {
    /// True only for the two variants that mean the caller has no usable
    /// `WorkOS` credentials: no stored token, or a token the control plane
    /// rejected as invalid/expired. Every other variant is a different kind
    /// of failure (request shape, ownership, transport, server-side) and
    /// must not be classified as an authentication failure.
    pub fn is_authentication_failure(&self) -> bool {
        matches!(
            self,
            Self::MissingCredentials | Self::AuthenticationFailed(_)
        )
    }

    /// True when the failure came from loading or saving local authentication
    /// credentials, rather than from the control-plane request itself.
    pub fn is_storage_failure(&self) -> bool {
        matches!(self, Self::Storage(_))
    }
}

impl From<TokenStorageError> for ControlPlaneError {
    fn from(value: TokenStorageError) -> Self {
        Self::Storage(value.to_string())
    }
}

impl From<AuthError> for ControlPlaneError {
    fn from(value: AuthError) -> Self {
        match value {
            AuthError::Unauthorized(reason) => Self::AuthenticationFailed(reason),
            AuthError::RequestFailed(error) => Self::Transport(error.to_string()),
            AuthError::Storage(error) => Self::Storage(error.to_string()),
            other => Self::AuthenticationFailed(other.to_string()),
        }
    }
}

fn is_transient(error: &ControlPlaneError) -> bool {
    matches!(
        error,
        ControlPlaneError::Transport(_) | ControlPlaneError::ServerError(_)
    )
}

enum StateAttempt {
    Done(AgentTraceIngestionStateResponse),
    Terminal(ControlPlaneError),
}

pub trait CredentialStore: Send + Sync + 'static {
    fn load(&self) -> impl Future<Output = Result<Option<StoredTokens>, ControlPlaneError>> + Send;
    fn save(
        &self,
        token: &TokenResponse,
    ) -> impl Future<Output = Result<StoredTokens, ControlPlaneError>> + Send;
}

/// Production `CredentialStore` backed by the real encrypted auth database.
pub struct SystemCredentialStore;

impl CredentialStore for SystemCredentialStore {
    async fn load(&self) -> Result<Option<StoredTokens>, ControlPlaneError> {
        Ok(token_storage::load_tokens().await?)
    }

    async fn save(&self, token: &TokenResponse) -> Result<StoredTokens, ControlPlaneError> {
        Ok(token_storage::save_tokens(token).await?)
    }
}

/// Authenticated HTTP client for the control-plane Agent Trace ingestion API.
///
/// Loads/refreshes stored `WorkOS` credentials through the existing
/// `auth`/`token_storage` primitives, injects the `Authorization: Bearer`
/// header, and retries exactly once on an unexpected `401`.
pub struct AuthenticatedControlPlaneClient<S = SystemCredentialStore> {
    http: reqwest::Client,
    base_url: String,
    workos_api_base_url: String,
    workos_client_id: String,
    credential_store: Arc<S>,
    refresh_lock: Arc<tokio::sync::Mutex<()>>,
}

/// Response body for `GET /me`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeResponse {
    pub user: MeUser,
    pub authorization: MeAuthorization,
    pub workspace: Option<MeWorkspace>,
}

/// User profile returned by the Control Plane's `/me` endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeUser {
    pub email: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
}

/// Authorization information returned by the Control Plane's `/me` endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeAuthorization {
    pub permissions: Vec<String>,
    pub role: Option<String>,
}

/// Current workspace returned by the Control Plane's `/me` endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeWorkspace {
    pub name: String,
}

impl AuthenticatedControlPlaneClient {
    pub fn new(
        http: reqwest::Client,
        base_url: impl Into<String>,
        workos_api_base_url: impl Into<String>,
        workos_client_id: impl Into<String>,
    ) -> Self {
        Self::with_credential_store(
            http,
            base_url,
            workos_api_base_url,
            workos_client_id,
            SystemCredentialStore,
        )
    }
}

impl<S: CredentialStore> AuthenticatedControlPlaneClient<S> {
    pub fn with_credential_store(
        http: reqwest::Client,
        base_url: impl Into<String>,
        workos_api_base_url: impl Into<String>,
        workos_client_id: impl Into<String>,
        credential_store: S,
    ) -> Self {
        Self {
            http,
            base_url: base_url.into(),
            workos_api_base_url: workos_api_base_url.into(),
            workos_client_id: workos_client_id.into(),
            credential_store: Arc::new(credential_store),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub(crate) async fn wait_for_pending_refresh(&self) {
        let _guard = self.refresh_lock.lock().await;
    }

    /// Calls `POST /agent-trace/ingestion/state` with a single attempt and a
    /// 60-second timeout. `400`/`403`/post-refresh-`401` are terminal and never
    /// retried.
    pub async fn ingestion_state(
        &self,
        request: &AgentTraceIngestionStateRequest,
    ) -> Result<AgentTraceIngestionStateResponse, ControlPlaneError> {
        let url = self.endpoint(STATE_PATH);
        let policy = STATE_RETRY_POLICY;

        let outcome = run_with_retry(
            policy,
            "agent_trace_sync.ingestion_state",
            "check network connectivity and control-plane availability, then rerun 'sce sync'",
            |_attempt| {
                let url = url.clone();
                async move {
                    match self.send_state_request(&url, request).await {
                        Ok(response) => Ok(StateAttempt::Done(response)),
                        Err(error) if is_transient(&error) => Err(anyhow!(error)),
                        Err(error) => Ok(StateAttempt::Terminal(error)),
                    }
                }
            },
        )
        .await
        .map_err(|error| ControlPlaneError::ServerError(error.to_string()))?;

        match outcome {
            StateAttempt::Done(response) => Ok(response),
            StateAttempt::Terminal(error) => Err(error),
        }
    }

    /// Calls `GET /me` to retrieve the current authenticated user's profile,
    /// authorization, and optional workspace from the Control Plane.
    pub async fn me(&self) -> Result<MeResponse, ControlPlaneError> {
        let url = self.endpoint(ME_PATH);
        let response = self
            .execute_authenticated(|token| self.http.get(&url).bearer_auth(token))
            .await?;
        classify_response(response).await
    }

    pub async fn ingest_messages(
        &self,
        request: &AgentTraceIngestionBatchRequest<AgentTraceMessageExportRow>,
    ) -> Result<AgentTraceIngestionBatchResponse, ControlPlaneError> {
        self.post_batch(request).await
    }

    pub async fn ingest_parts(
        &self,
        request: &AgentTraceIngestionBatchRequest<AgentTracePartExportRow>,
    ) -> Result<AgentTraceIngestionBatchResponse, ControlPlaneError> {
        self.post_batch(request).await
    }

    pub async fn ingest_diff_traces(
        &self,
        request: &AgentTraceIngestionBatchRequest<AgentTraceDiffTraceExportRow>,
    ) -> Result<AgentTraceIngestionBatchResponse, ControlPlaneError> {
        self.post_batch(request).await
    }

    pub async fn ingest_agent_traces(
        &self,
        request: &AgentTraceIngestionBatchRequest<AgentTraceAgentTraceExportRow>,
    ) -> Result<AgentTraceIngestionBatchResponse, ControlPlaneError> {
        self.post_batch(request).await
    }

    async fn post_batch<T>(
        &self,
        request: &AgentTraceIngestionBatchRequest<T>,
    ) -> Result<AgentTraceIngestionBatchResponse, ControlPlaneError>
    where
        T: Serialize,
    {
        let url = self.endpoint(BATCH_PATH);
        let response = self
            .execute_authenticated(|token| self.http.post(&url).bearer_auth(token).json(request))
            .await?;
        classify_response(response).await
    }

    async fn send_state_request(
        &self,
        url: &str,
        request: &AgentTraceIngestionStateRequest,
    ) -> Result<AgentTraceIngestionStateResponse, ControlPlaneError> {
        let response = self
            .execute_authenticated(|token| self.http.post(url).bearer_auth(token).json(request))
            .await?;
        let state: AgentTraceIngestionStateResponse = classify_response(response).await?;
        state.cursors.validate()?;
        Ok(state)
    }

    /// Sends one authenticated request, retrying exactly once on an
    /// unexpected `401` (refresh, save, retry). A `401` after the retry is
    /// terminal.
    async fn execute_authenticated<F>(
        &self,
        build: F,
    ) -> Result<reqwest::Response, ControlPlaneError>
    where
        F: Fn(&str) -> reqwest::RequestBuilder,
    {
        let token = self.resolve_access_token().await?;
        let response = build(&token)
            .send()
            .await
            .map_err(|error| ControlPlaneError::Transport(error.to_string()))?;

        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok(response);
        }

        let refreshed_token = self.force_refresh_access_token(&token).await?;
        let retried = build(&refreshed_token)
            .send()
            .await
            .map_err(|error| ControlPlaneError::Transport(error.to_string()))?;

        if retried.status() == StatusCode::UNAUTHORIZED {
            return Err(ControlPlaneError::AuthenticationFailed(
                "control-plane returned 401 after a refreshed token was retried".to_string(),
            ));
        }

        Ok(retried)
    }

    async fn resolve_access_token(&self) -> Result<String, ControlPlaneError> {
        let stored = self
            .load_credentials()
            .await?
            .ok_or(ControlPlaneError::MissingCredentials)?;

        if !auth::is_stored_token_expired(&stored)? {
            return Ok(stored.access_token);
        }

        let refresh_guard = Arc::clone(&self.refresh_lock).lock_owned().await;
        let stored = self
            .load_credentials()
            .await?
            .ok_or(ControlPlaneError::MissingCredentials)?;

        if !auth::is_stored_token_expired(&stored)? {
            return Ok(stored.access_token);
        }

        self.refresh_and_save(refresh_guard, stored).await
    }

    async fn force_refresh_access_token(
        &self,
        rejected_access_token: &str,
    ) -> Result<String, ControlPlaneError> {
        let refresh_guard = Arc::clone(&self.refresh_lock).lock_owned().await;
        let stored = self
            .load_credentials()
            .await?
            .ok_or(ControlPlaneError::MissingCredentials)?;

        if stored.access_token != rejected_access_token {
            return Ok(stored.access_token);
        }

        self.refresh_and_save(refresh_guard, stored).await
    }

    async fn refresh_and_save(
        &self,
        refresh_guard: tokio::sync::OwnedMutexGuard<()>,
        stored: StoredTokens,
    ) -> Result<String, ControlPlaneError> {
        let http = self.http.clone();
        let workos_api_base_url = self.workos_api_base_url.clone();
        let workos_client_id = self.workos_client_id.clone();
        let credential_store = Arc::clone(&self.credential_store);

        tokio::spawn(async move {
            let _refresh_guard = refresh_guard;
            let token = auth::renew_stored_token_from_refresh_token(
                &http,
                &workos_api_base_url,
                &workos_client_id,
                &stored.refresh_token,
            )
            .await?;
            credential_store.save(&token).await?;
            Ok(token.access_token)
        })
        .await
        .map_err(|error| {
            ControlPlaneError::AuthenticationFailed(format!(
                "credential refresh task did not complete: {error}"
            ))
        })?
    }

    async fn load_credentials(&self) -> Result<Option<StoredTokens>, ControlPlaneError> {
        self.credential_store.load().await
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.base_url.trim_end_matches('/'), path)
    }
}

/// Maximum accepted length, in bytes, of a `message`/`error` field extracted
/// from a control-plane error body. Anything longer is treated as
/// untrustworthy and discarded in favor of a generic fallback.
const MAX_SAFE_ERROR_MESSAGE_LEN: usize = 500;

/// Extracts a narrow, safe error message from a raw HTTP error response body.
///
/// Accepts only a top-level JSON object with a `message` or `error` string
/// field (checked in that order) no longer than
/// [`MAX_SAFE_ERROR_MESSAGE_LEN`]. Returns `None` for malformed JSON, HTML,
/// non-object top-level values, a missing/non-string field, or a field that
/// exceeds the length bound — so an arbitrary server-side implementation
/// detail (a SQL error, a stack trace, an HTML error page) can never reach
/// the CLI's user-visible error text.
fn extract_safe_error_message(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let object = value.as_object()?;
    let field = object
        .get("message")
        .or_else(|| object.get("error"))?
        .as_str()?;

    if field.is_empty() || field.len() > MAX_SAFE_ERROR_MESSAGE_LEN {
        return None;
    }

    Some(field.to_string())
}

fn safe_error_message(body: &str, generic: &str) -> String {
    extract_safe_error_message(body).unwrap_or_else(|| generic.to_string())
}

async fn classify_response<T>(response: reqwest::Response) -> Result<T, ControlPlaneError>
where
    T: serde::de::DeserializeOwned,
{
    let status = response.status();
    if status.is_success() {
        return response
            .json::<T>()
            .await
            .map_err(|error| ControlPlaneError::InvalidResponse(error.to_string()));
    }

    let body = response
        .text()
        .await
        .unwrap_or_else(|error| format!("<failed to read response body: {error}>"));

    match status {
        StatusCode::BAD_REQUEST => Err(ControlPlaneError::BadRequest(safe_error_message(
            &body,
            "control plane rejected the Agent Trace request",
        ))),
        StatusCode::FORBIDDEN => Err(ControlPlaneError::Forbidden(safe_error_message(
            &body,
            "Agent Trace source cannot be synchronized by the current authenticated user",
        ))),
        StatusCode::CONFLICT => Err(ControlPlaneError::Conflict(safe_error_message(
            &body,
            "Agent Trace cursor conflict",
        ))),
        StatusCode::SERVICE_UNAVAILABLE => Err(ControlPlaneError::ServerError(safe_error_message(
            &body,
            "control-plane Agent Trace storage is unavailable",
        ))),
        status if status.is_server_error() => Err(ControlPlaneError::ServerError(
            safe_error_message(&body, "control plane encountered an internal error"),
        )),
        status if status.is_client_error() => Err(ControlPlaneError::Protocol {
            status,
            message: safe_error_message(
                &body,
                "control plane rejected the Agent Trace request as unsupported",
            ),
        }),
        status => Err(ControlPlaneError::InvalidResponse(format!(
            "unexpected status {status}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use tokio::sync::{Notify, Semaphore};

    use super::{AuthenticatedControlPlaneClient, ControlPlaneError, CredentialStore};
    use crate::services::auth::TokenResponse;
    use crate::services::token_storage::StoredTokens;

    const REPLACEMENT_ACCESS_TOKEN: &str = "replacement-access";
    const REPLACEMENT_REFRESH_TOKEN: &str = "replacement-refresh";

    struct HeldSaveStore {
        stored: Mutex<StoredTokens>,
        saves: AtomicUsize,
        save_started: Notify,
        release_save: Semaphore,
    }

    impl CredentialStore for HeldSaveStore {
        async fn load(&self) -> Result<Option<StoredTokens>, ControlPlaneError> {
            Ok(Some(self.stored.lock().expect("stored lock").clone()))
        }

        async fn save(&self, token: &TokenResponse) -> Result<StoredTokens, ControlPlaneError> {
            self.save_started.notify_one();
            self.release_save
                .acquire()
                .await
                .expect("release semaphore open")
                .forget();
            let saved = StoredTokens {
                access_token: token.access_token.clone(),
                token_type: token.token_type.clone(),
                expires_in: token.expires_in,
                refresh_token: token.refresh_token.clone(),
                scope: token.scope.clone(),
                stored_at_unix_seconds: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock after epoch")
                    .as_secs(),
            };
            *self.stored.lock().expect("stored lock") = saved.clone();
            self.saves.fetch_add(1, Ordering::SeqCst);
            Ok(saved)
        }
    }

    fn start_refresh_server() -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind refresh server");
        let base_url = format!("http://{}", listener.local_addr().expect("server addr"));
        let hits = Arc::new(AtomicUsize::new(0));
        let thread_hits = Arc::clone(&hits);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    continue;
                };
                let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    let line = line.trim_end_matches(['\r', '\n']);
                    if line.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            content_length = value.trim().parse().unwrap_or(0);
                        }
                    }
                }
                let mut body = vec![0u8; content_length];
                let _ = reader.read_exact(&mut body);
                thread_hits.fetch_add(1, Ordering::SeqCst);
                let payload = format!(
                    r#"{{"access_token":"{REPLACEMENT_ACCESS_TOKEN}","refresh_token":"{REPLACEMENT_REFRESH_TOKEN}","expires_in":3600}}"#
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (base_url, hits)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_caller_does_not_lose_rotated_refresh_token() {
        let (workos_url, workos_hits) = start_refresh_server();
        let store = HeldSaveStore {
            stored: Mutex::new(StoredTokens {
                access_token: "expired-access".to_string(),
                token_type: "Bearer".to_string(),
                expires_in: 0,
                refresh_token: "original-refresh".to_string(),
                scope: None,
                stored_at_unix_seconds: 0,
            }),
            saves: AtomicUsize::new(0),
            save_started: Notify::new(),
            release_save: Semaphore::new(0),
        };
        let client = Arc::new(AuthenticatedControlPlaneClient::with_credential_store(
            reqwest::Client::builder()
                .tls_certs_only(Vec::new())
                .build()
                .expect("test http client"),
            "http://127.0.0.1:1",
            workos_url,
            "client",
            store,
        ));

        let first = {
            let client = Arc::clone(&client);
            tokio::spawn(async move { client.resolve_access_token().await })
        };
        client.credential_store.save_started.notified().await;
        let second = {
            let client = Arc::clone(&client);
            tokio::spawn(async move { client.resolve_access_token().await })
        };

        first.abort();
        assert!(first
            .await
            .expect_err("first caller cancelled")
            .is_cancelled());
        assert_eq!(client.credential_store.saves.load(Ordering::SeqCst), 0);

        client.credential_store.release_save.add_permits(1);
        let second_token = second
            .await
            .expect("second caller joined")
            .expect("second caller resolved a token");
        let third_token = client
            .resolve_access_token()
            .await
            .expect("third caller resolved a token");

        assert_eq!(second_token, REPLACEMENT_ACCESS_TOKEN);
        assert_eq!(third_token, REPLACEMENT_ACCESS_TOKEN);
        assert_eq!(client.credential_store.saves.load(Ordering::SeqCst), 1);
        assert_eq!(workos_hits.load(Ordering::SeqCst), 1);
        assert_eq!(
            client
                .credential_store
                .stored
                .lock()
                .expect("stored lock")
                .refresh_token,
            REPLACEMENT_REFRESH_TOKEN
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pending_refresh_drain_waits_for_cancelled_callers_save() {
        let (workos_url, workos_hits) = start_refresh_server();
        let store = HeldSaveStore {
            stored: Mutex::new(StoredTokens {
                access_token: "expired-access".to_string(),
                token_type: "Bearer".to_string(),
                expires_in: 0,
                refresh_token: "original-refresh".to_string(),
                scope: None,
                stored_at_unix_seconds: 0,
            }),
            saves: AtomicUsize::new(0),
            save_started: Notify::new(),
            release_save: Semaphore::new(0),
        };
        let client = Arc::new(AuthenticatedControlPlaneClient::with_credential_store(
            reqwest::Client::builder()
                .tls_certs_only(Vec::new())
                .build()
                .expect("test http client"),
            "http://127.0.0.1:1",
            workos_url,
            "client",
            store,
        ));

        let caller = {
            let client = Arc::clone(&client);
            tokio::spawn(async move { client.resolve_access_token().await })
        };
        client.credential_store.save_started.notified().await;
        caller.abort();
        assert!(caller.await.expect_err("caller cancelled").is_cancelled());

        let mut drain = std::pin::pin!(client.wait_for_pending_refresh());
        let probe =
            tokio::time::timeout(std::time::Duration::from_millis(100), drain.as_mut()).await;
        assert!(probe.is_err(), "drain returned before the save finished");
        assert_eq!(client.credential_store.saves.load(Ordering::SeqCst), 0);

        client.credential_store.release_save.add_permits(1);
        tokio::time::timeout(std::time::Duration::from_secs(10), drain)
            .await
            .expect("drain completes after the save is released");

        assert_eq!(client.credential_store.saves.load(Ordering::SeqCst), 1);
        assert_eq!(workos_hits.load(Ordering::SeqCst), 1);
        assert_eq!(
            client
                .credential_store
                .stored
                .lock()
                .expect("stored lock")
                .refresh_token,
            REPLACEMENT_REFRESH_TOKEN
        );
    }
}
