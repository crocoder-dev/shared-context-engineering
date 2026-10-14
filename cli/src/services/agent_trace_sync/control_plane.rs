use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::services::agent_trace_export::{
    self, AgentTraceAgentTraceExportRow, AgentTraceDiffTraceExportRow, AgentTraceMessageExportRow,
    AgentTracePartExportRow,
};
use crate::services::auth::{self, AuthError, TokenResponse};
use crate::services::resilience::{run_with_retry, RetryPolicy};
use crate::services::telemetry_credential_storage::{self, TelemetryCredential};
use crate::services::token_storage::{self, StoredTokens, TokenStorageError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestionStream {
    Messages,
    Parts,
    DiffTraces,
    AgentTraces,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceIngestionStateRequest {
    pub repository_id: String,
    pub source_instance_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceCursors {
    pub messages: i64,
    pub parts: i64,
    pub diff_traces: i64,
    pub agent_traces: i64,
}

impl AgentTraceCursors {
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

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceIngestionStateResponse {
    pub cursors: AgentTraceCursors,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceIngestionBatchRequest<T> {
    pub repository_id: String,
    pub source_instance_id: String,
    pub stream: IngestionStream,
    pub expected_cursor: i64,
    pub rows: Vec<T>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceIngestionBatchResponse {
    pub accepted: usize,
    pub cursor: i64,
}

const STATE_PATH: &str = "agent-trace/ingestion/state";
const BATCH_PATH: &str = "agent-trace/ingestion/batch";
const ME_PATH: &str = "me";
const TELEMETRY_CREDENTIAL_PATH: &str = "auth/telemetry/credential";

const TELEMETRY_FORBIDDEN_MESSAGE: &str = "telemetry credential issuance was denied";
const TELEMETRY_UNAVAILABLE_MESSAGE: &str = "telemetry credential issuance is unavailable";
const TELEMETRY_SERVER_MESSAGE: &str = "telemetry credential issuance failed on the server";
const TELEMETRY_PROTOCOL_MESSAGE: &str = "telemetry credential request was rejected";
const TELEMETRY_INVALID_RESPONSE_MESSAGE: &str =
    "telemetry credential response was malformed or invalid";
const TELEMETRY_TRANSPORT_MESSAGE: &str = "telemetry credential request failed to complete";
const TELEMETRY_STORAGE_MESSAGE: &str = "failed to save telemetry credential";
const TELEMETRY_REDACTED: &str = "<redacted>";

const BATCH_REQUEST_TIMEOUT: Duration = Duration::from_mins(1);
const TELEMETRY_CREDENTIAL_TIMEOUT: Duration = Duration::from_mins(1);

const STATE_RETRY_POLICY: RetryPolicy = RetryPolicy::builtin(1, 60_000, 250, 2_000);

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
    Protocol { status: StatusCode, message: String },
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
    pub fn is_authentication_failure(&self) -> bool {
        matches!(
            self,
            Self::MissingCredentials | Self::AuthenticationFailed(_)
        )
    }

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

pub struct SystemCredentialStore;

impl CredentialStore for SystemCredentialStore {
    async fn load(&self) -> Result<Option<StoredTokens>, ControlPlaneError> {
        Ok(token_storage::load_tokens().await?)
    }

    async fn save(&self, token: &TokenResponse) -> Result<StoredTokens, ControlPlaneError> {
        Ok(token_storage::save_tokens(token).await?)
    }
}

pub struct AuthenticatedControlPlaneClient<S = SystemCredentialStore> {
    http: reqwest::Client,
    base_url: String,
    workos_api_base_url: String,
    workos_client_id: String,
    credential_store: Arc<S>,
    refresh_lock: Arc<tokio::sync::Mutex<()>>,
    batch_timeout: Duration,
    telemetry_timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeResponse {
    pub user: MeUser,
    pub authorization: MeAuthorization,
    pub workspace: Option<MeWorkspace>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeUser {
    pub email: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeAuthorization {
    pub permissions: Vec<String>,
    pub role: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeWorkspace {
    pub name: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryCredentialResponse {
    token: String,
    accessor: String,
    expires_at: String,
}

impl fmt::Debug for TelemetryCredentialResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelemetryCredentialResponse")
            .field("token", &TELEMETRY_REDACTED)
            .field("accessor", &TELEMETRY_REDACTED)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl TelemetryCredentialResponse {
    fn into_credential(
        self,
        now_unix_seconds: u64,
    ) -> Result<TelemetryCredential, ControlPlaneError> {
        let invalid =
            || ControlPlaneError::InvalidResponse(TELEMETRY_INVALID_RESPONSE_MESSAGE.to_string());
        if self.token.trim().is_empty() || self.accessor.trim().is_empty() {
            return Err(invalid());
        }
        let expires_at = chrono::DateTime::parse_from_rfc3339(&self.expires_at)
            .map_err(|_| invalid())?
            .timestamp();
        let expires_at_unix_seconds = u64::try_from(expires_at).map_err(|_| invalid())?;
        if expires_at_unix_seconds <= now_unix_seconds {
            return Err(invalid());
        }
        Ok(TelemetryCredential {
            token: self.token,
            accessor: self.accessor,
            expires_at_unix_seconds,
        })
    }
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
            batch_timeout: BATCH_REQUEST_TIMEOUT,
            telemetry_timeout: TELEMETRY_CREDENTIAL_TIMEOUT,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_telemetry_timeout(mut self, telemetry_timeout: Duration) -> Self {
        self.telemetry_timeout = telemetry_timeout;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_batch_timeout(mut self, batch_timeout: Duration) -> Self {
        self.batch_timeout = batch_timeout;
        self
    }

    pub(crate) async fn wait_for_pending_refresh(&self) {
        let _guard = self.refresh_lock.lock().await;
    }

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

    pub async fn me(&self) -> Result<MeResponse, ControlPlaneError> {
        let url = self.endpoint(ME_PATH);
        let response = self
            .execute_authenticated(|token| self.http.get(&url).bearer_auth(token))
            .await?;
        classify_response(response).await
    }

    pub async fn issue_telemetry_credential(
        &self,
    ) -> Result<TelemetryCredential, ControlPlaneError> {
        tokio::time::timeout(self.telemetry_timeout, self.request_telemetry_credential())
            .await
            .unwrap_or_else(|_| {
                Err(ControlPlaneError::Transport(
                    TELEMETRY_TRANSPORT_MESSAGE.to_string(),
                ))
            })
    }

    async fn request_telemetry_credential(&self) -> Result<TelemetryCredential, ControlPlaneError> {
        let url = self.endpoint(TELEMETRY_CREDENTIAL_PATH);
        let response = self
            .execute_authenticated(|token| self.http.post(&url).bearer_auth(token))
            .await
            .map_err(sanitize_telemetry_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(classify_telemetry_status(status));
        }
        let body: TelemetryCredentialResponse = response.json().await.map_err(|_| {
            ControlPlaneError::InvalidResponse(TELEMETRY_INVALID_RESPONSE_MESSAGE.to_string())
        })?;
        body.into_credential(current_unix_seconds())
    }

    pub async fn issue_and_save_telemetry_credential(
        &self,
    ) -> Result<TelemetryCredential, ControlPlaneError> {
        self.issue_and_save_with(|credential| async move {
            telemetry_credential_storage::save_telemetry_credential(&credential).await
        })
        .await
    }

    async fn issue_and_save_with<F, Fut>(
        &self,
        save: F,
    ) -> Result<TelemetryCredential, ControlPlaneError>
    where
        F: FnOnce(TelemetryCredential) -> Fut,
        Fut: Future<Output = Result<(), TokenStorageError>>,
    {
        let credential = self.issue_telemetry_credential().await?;
        save(credential.clone())
            .await
            .map_err(|_| ControlPlaneError::Storage(TELEMETRY_STORAGE_MESSAGE.to_string()))?;
        Ok(credential)
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

    #[allow(
        dead_code,
        reason = "diff_traces compatibility surface retained per context/decisions/2026-10-01-retire-diff-traces-remote-sync-compatibility-gate.md"
    )]
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
            .execute_authenticated(|token| {
                self.http
                    .post(&url)
                    .bearer_auth(token)
                    .timeout(self.batch_timeout)
                    .json(request)
            })
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

fn current_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn sanitize_telemetry_error(error: ControlPlaneError) -> ControlPlaneError {
    match error {
        ControlPlaneError::Transport(_) => {
            ControlPlaneError::Transport(TELEMETRY_TRANSPORT_MESSAGE.to_string())
        }
        other => other,
    }
}

fn classify_telemetry_status(status: StatusCode) -> ControlPlaneError {
    match status {
        StatusCode::FORBIDDEN => {
            ControlPlaneError::Forbidden(TELEMETRY_FORBIDDEN_MESSAGE.to_string())
        }
        StatusCode::SERVICE_UNAVAILABLE => {
            ControlPlaneError::ServerError(TELEMETRY_UNAVAILABLE_MESSAGE.to_string())
        }
        status if status.is_server_error() => {
            ControlPlaneError::ServerError(TELEMETRY_SERVER_MESSAGE.to_string())
        }
        status if status.is_client_error() => ControlPlaneError::Protocol {
            status,
            message: TELEMETRY_PROTOCOL_MESSAGE.to_string(),
        },
        status => ControlPlaneError::InvalidResponse(format!("unexpected status {status}")),
    }
}

const MAX_SAFE_ERROR_MESSAGE_LEN: usize = 500;

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
        return response.json::<T>().await.map_err(|error| {
            if error.is_timeout() {
                ControlPlaneError::Transport(error.to_string())
            } else {
                ControlPlaneError::InvalidResponse(error.to_string())
            }
        });
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

    use super::{
        AgentTraceIngestionStateRequest, AgentTraceIngestionStateResponse,
        AuthenticatedControlPlaneClient, ControlPlaneError, CredentialStore,
    };
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

    struct FixedStore;

    impl CredentialStore for FixedStore {
        async fn load(&self) -> Result<Option<StoredTokens>, ControlPlaneError> {
            Ok(Some(StoredTokens {
                access_token: "access".to_string(),
                token_type: "Bearer".to_string(),
                expires_in: 3600,
                refresh_token: "refresh".to_string(),
                scope: None,
                stored_at_unix_seconds: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock after epoch")
                    .as_secs(),
            }))
        }

        async fn save(&self, _token: &TokenResponse) -> Result<StoredTokens, ControlPlaneError> {
            unreachable!("fixed credentials never refresh")
        }
    }

    fn start_state_server(payload: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind state server");
        let base_url = format!("http://{}", listener.local_addr().expect("server addr"));
        let payload = payload.to_string();
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
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        base_url
    }

    fn start_capture_server(status_line: &str, payload: &str) -> (String, Arc<Mutex<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind capture server");
        let base_url = format!("http://{}", listener.local_addr().expect("server addr"));
        let captured = Arc::new(Mutex::new(String::new()));
        let thread_captured = Arc::clone(&captured);
        let status_line = status_line.to_string();
        let payload = payload.to_string();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    continue;
                };
                let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
                let mut head = String::new();
                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    head.push_str(&line);
                    let trimmed = line.trim_end_matches(['\r', '\n']);
                    if trimmed.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = trimmed.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            content_length = value.trim().parse().unwrap_or(0);
                        }
                    }
                }
                let mut body = vec![0u8; content_length];
                let _ = reader.read_exact(&mut body);
                *thread_captured.lock().expect("capture lock") = head;
                let response = format!(
                    "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (base_url, captured)
    }

    async fn request_telemetry_credential(
        status_line: &str,
        payload: &str,
    ) -> (
        Result<
            crate::services::telemetry_credential_storage::TelemetryCredential,
            ControlPlaneError,
        >,
        Arc<Mutex<String>>,
    ) {
        let (base_url, captured) = start_capture_server(status_line, payload);
        let client = AuthenticatedControlPlaneClient::with_credential_store(
            reqwest::Client::builder()
                .tls_certs_only(Vec::new())
                .build()
                .expect("test http client"),
            base_url,
            "http://127.0.0.1:1",
            "client",
            FixedStore,
        );
        (client.issue_telemetry_credential().await, captured)
    }

    const TELEMETRY_SECRET_TOKEN: &str = "hvs.secret-token-value";
    const TELEMETRY_SECRET_ACCESSOR: &str = "secret-accessor-value";

    fn telemetry_success_payload(expires_at: &str) -> String {
        format!(
            r#"{{"token":"{TELEMETRY_SECRET_TOKEN}","accessor":"{TELEMETRY_SECRET_ACCESSOR}","expiresAt":"{expires_at}"}}"#
        )
    }

    fn assert_no_telemetry_secret(error: &ControlPlaneError) {
        let rendered = format!("{error} {error:?}");
        assert!(!rendered.contains(TELEMETRY_SECRET_TOKEN), "{rendered}");
        assert!(!rendered.contains(TELEMETRY_SECRET_ACCESSOR), "{rendered}");
        assert!(!rendered.contains("Bearer"), "{rendered}");
    }

    #[tokio::test]
    async fn telemetry_credential_client_posts_with_bearer_and_no_body() {
        let (result, captured) = request_telemetry_credential(
            "200 OK",
            &telemetry_success_payload("2999-01-01T00:00:00Z"),
        )
        .await;

        let credential = result.expect("valid credential");
        assert_eq!(credential.token, TELEMETRY_SECRET_TOKEN);
        assert_eq!(credential.accessor, TELEMETRY_SECRET_ACCESSOR);
        assert_eq!(credential.expires_at_unix_seconds, 32_472_144_000);

        let head = captured.lock().expect("capture lock").to_lowercase();
        assert!(
            head.starts_with("post /auth/telemetry/credential "),
            "{head}"
        );
        assert!(head.contains("authorization: bearer access"), "{head}");
        assert!(
            !head.contains("content-type: application/json"),
            "request must carry no JSON body: {head}"
        );
        let rendered = format!("{credential:?}");
        assert!(!rendered.contains(TELEMETRY_SECRET_TOKEN));
        assert!(!rendered.contains(TELEMETRY_SECRET_ACCESSOR));
    }

    #[tokio::test]
    async fn telemetry_credential_issue_and_save_replaces_cached_credential() {
        let (base_url, _) =
            start_capture_server("200 OK", &telemetry_success_payload("2999-01-01T00:00:00Z"));
        let client = AuthenticatedControlPlaneClient::with_credential_store(
            reqwest::Client::builder()
                .tls_certs_only(Vec::new())
                .build()
                .expect("test http client"),
            base_url,
            "http://127.0.0.1:1",
            "client",
            FixedStore,
        );
        let saved = Arc::new(Mutex::new(None));
        let saved_credential = Arc::clone(&saved);

        let result = client
            .issue_and_save_with(|credential| async move {
                *saved_credential.lock().expect("saved credential lock") = Some(credential);
                Ok(())
            })
            .await
            .expect("issuance and save should succeed");

        assert_eq!(
            saved.lock().expect("saved credential lock").as_ref(),
            Some(&result)
        );
    }

    #[tokio::test]
    async fn telemetry_credential_client_maps_failures_to_sanitized_errors() {
        let leaky = format!(
            r#"{{"message":"{TELEMETRY_SECRET_TOKEN} {TELEMETRY_SECRET_ACCESSOR} Bearer x"}}"#
        );

        let (forbidden, _) = request_telemetry_credential("403 Forbidden", &leaky).await;
        let forbidden = forbidden.expect_err("403");
        assert!(
            matches!(&forbidden, ControlPlaneError::Forbidden(_)),
            "{forbidden:?}"
        );
        assert_no_telemetry_secret(&forbidden);

        let (unavailable, _) =
            request_telemetry_credential("503 Service Unavailable", &leaky).await;
        let unavailable = unavailable.expect_err("503");
        assert!(
            matches!(&unavailable, ControlPlaneError::ServerError(_)),
            "{unavailable:?}"
        );
        assert_no_telemetry_secret(&unavailable);

        let (server, _) = request_telemetry_credential("500 Internal Server Error", &leaky).await;
        let server = server.expect_err("500");
        assert!(
            matches!(&server, ControlPlaneError::ServerError(_)),
            "{server:?}"
        );
        assert_no_telemetry_secret(&server);

        let (unauthorized, _) = request_telemetry_credential("401 Unauthorized", &leaky).await;
        let unauthorized = unauthorized.expect_err("401");
        assert!(
            matches!(
                &unauthorized,
                ControlPlaneError::AuthenticationFailed(_)
                    | ControlPlaneError::Transport(_)
                    | ControlPlaneError::Storage(_)
            ),
            "{unauthorized:?}"
        );
        assert_no_telemetry_secret(&unauthorized);

        for malformed in [
            leaky.as_str(),
            r#"{"token":"","accessor":"a","expiresAt":"2999-01-01T00:00:00Z"}"#,
            r#"{"token":"t","accessor":"","expiresAt":"2999-01-01T00:00:00Z"}"#,
            r#"{"token":"t","accessor":"a","expiresAt":"not-a-date"}"#,
            r#"{"token":"t","accessor":"a","expiresAt":"2000-01-01T00:00:00Z"}"#,
        ] {
            let (result, _) = request_telemetry_credential("200 OK", malformed).await;
            let error = result.expect_err("malformed");
            assert!(
                matches!(&error, ControlPlaneError::InvalidResponse(_)),
                "{error:?}"
            );
            assert_no_telemetry_secret(&error);
        }
    }

    #[tokio::test]
    async fn telemetry_credential_client_transport_failure_is_sanitized() {
        let client = AuthenticatedControlPlaneClient::with_credential_store(
            reqwest::Client::builder()
                .tls_certs_only(Vec::new())
                .build()
                .expect("test http client"),
            "http://127.0.0.1:1",
            "http://127.0.0.1:1",
            "client",
            FixedStore,
        );
        let error = client
            .issue_telemetry_credential()
            .await
            .expect_err("transport failure");
        assert!(
            matches!(&error, ControlPlaneError::Transport(_)),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            "Control-plane request failed: telemetry credential request failed to complete"
        );
    }

    fn start_stall_server(
        status_line: &'static str,
        headers_then_stall: bool,
        per_request_unauthorized_first: bool,
    ) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stall server");
        let base_url = format!("http://{}", listener.local_addr().expect("server addr"));
        let hits = Arc::new(AtomicUsize::new(0));
        let thread_hits = Arc::clone(&hits);
        thread::spawn(move || {
            let mut open_connections = Vec::new();
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    continue;
                };
                let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    if line.trim_end_matches(['\r', '\n']).is_empty() {
                        break;
                    }
                }
                let attempt = thread_hits.fetch_add(1, Ordering::SeqCst);
                if per_request_unauthorized_first && attempt == 0 {
                    let _ = stream.write_all(
                        b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    continue;
                }
                if headers_then_stall {
                    let partial_response = format!(
                        "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: 4096\r\n\r\n{{\"token\":\"{TELEMETRY_SECRET_TOKEN}"
                    );
                    let _ = stream.write_all(partial_response.as_bytes());
                    let _ = stream.flush();
                }
                open_connections.push((stream, reader));
            }
        });
        (base_url, hits)
    }

    fn telemetry_client_with_timeout<S: CredentialStore>(
        base_url: String,
        workos_url: &str,
        store: S,
        timeout: std::time::Duration,
    ) -> AuthenticatedControlPlaneClient<S> {
        AuthenticatedControlPlaneClient::with_credential_store(
            reqwest::Client::builder()
                .tls_certs_only(Vec::new())
                .build()
                .expect("test http client"),
            base_url,
            workos_url,
            "client",
            store,
        )
        .with_telemetry_timeout(timeout)
    }

    const SHORT_TELEMETRY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(300);
    const TELEMETRY_TEST_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

    fn assert_sanitized_transport_timeout(error: &ControlPlaneError) {
        assert!(
            matches!(error, ControlPlaneError::Transport(_)),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            "Control-plane request failed: telemetry credential request failed to complete"
        );
        assert_no_telemetry_secret(error);
    }

    #[tokio::test]
    async fn telemetry_credential_times_out_when_server_never_responds() {
        let (base_url, hits) = start_stall_server("200 OK", false, false);
        let client = telemetry_client_with_timeout(
            base_url,
            "http://127.0.0.1:1",
            FixedStore,
            SHORT_TELEMETRY_TIMEOUT,
        );
        let error =
            tokio::time::timeout(TELEMETRY_TEST_DEADLINE, client.issue_telemetry_credential())
                .await
                .expect("issuance bounded by its own timeout")
                .expect_err("stalled server");
        assert_sanitized_transport_timeout(&error);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn telemetry_credential_times_out_when_body_stalls() {
        let (base_url, _) = start_stall_server("200 OK", true, false);
        let client = telemetry_client_with_timeout(
            base_url,
            "http://127.0.0.1:1",
            FixedStore,
            SHORT_TELEMETRY_TIMEOUT,
        );
        let error =
            tokio::time::timeout(TELEMETRY_TEST_DEADLINE, client.issue_telemetry_credential())
                .await
                .expect("issuance bounded by its own timeout")
                .expect_err("stalled body");
        assert_sanitized_transport_timeout(&error);
    }

    #[tokio::test]
    async fn telemetry_credential_times_out_when_retry_after_401_stalls() {
        let (workos_url, workos_hits) = start_refresh_server();
        let (base_url, hits) = start_stall_server("200 OK", false, true);
        let client = telemetry_client_with_timeout(
            base_url,
            &workos_url,
            RefreshableStore::new(),
            SHORT_TELEMETRY_TIMEOUT,
        );
        let error =
            tokio::time::timeout(TELEMETRY_TEST_DEADLINE, client.issue_telemetry_credential())
                .await
                .expect("issuance bounded by its own timeout")
                .expect_err("stalled retry");
        assert_sanitized_transport_timeout(&error);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert_eq!(workos_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn telemetry_credential_succeeds_within_deadline() {
        let (base_url, _) =
            start_capture_server("200 OK", &telemetry_success_payload("2999-01-01T00:00:00Z"));
        let client = telemetry_client_with_timeout(
            base_url,
            "http://127.0.0.1:1",
            FixedStore,
            TELEMETRY_TEST_DEADLINE,
        );
        let credential = client
            .issue_telemetry_credential()
            .await
            .expect("credential within deadline");
        assert_eq!(credential.token, TELEMETRY_SECRET_TOKEN);
    }

    #[tokio::test]
    async fn telemetry_credential_timeout_does_not_invoke_storage_save() {
        let (base_url, _) = start_stall_server("200 OK", false, false);
        let client = telemetry_client_with_timeout(
            base_url,
            "http://127.0.0.1:1",
            FixedStore,
            SHORT_TELEMETRY_TIMEOUT,
        );
        let saves = Arc::new(AtomicUsize::new(0));
        let save_counter = Arc::clone(&saves);
        let error = client
            .issue_and_save_with(|_| async move {
                save_counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await
            .expect_err("timeout");
        assert_sanitized_transport_timeout(&error);
        assert_eq!(saves.load(Ordering::SeqCst), 0);
    }

    struct RefreshableStore {
        stored: Mutex<StoredTokens>,
    }

    impl RefreshableStore {
        fn new() -> Self {
            Self {
                stored: Mutex::new(StoredTokens {
                    access_token: "access".to_string(),
                    token_type: "Bearer".to_string(),
                    expires_in: 3600,
                    refresh_token: "refresh".to_string(),
                    scope: None,
                    stored_at_unix_seconds: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("clock after epoch")
                        .as_secs(),
                }),
            }
        }
    }

    impl CredentialStore for RefreshableStore {
        async fn load(&self) -> Result<Option<StoredTokens>, ControlPlaneError> {
            Ok(Some(self.stored.lock().expect("stored lock").clone()))
        }

        async fn save(&self, token: &TokenResponse) -> Result<StoredTokens, ControlPlaneError> {
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
            Ok(saved)
        }
    }

    async fn request_state(
        payload: &str,
    ) -> Result<AgentTraceIngestionStateResponse, ControlPlaneError> {
        let client = AuthenticatedControlPlaneClient::with_credential_store(
            reqwest::Client::builder()
                .tls_certs_only(Vec::new())
                .build()
                .expect("test http client"),
            start_state_server(payload),
            "http://127.0.0.1:1",
            "client",
            FixedStore,
        );
        client
            .ingestion_state(&AgentTraceIngestionStateRequest {
                repository_id: "repo".to_string(),
                source_instance_id: "source".to_string(),
            })
            .await
    }

    #[tokio::test]
    async fn state_response_decodes_required_diff_traces_cursor() {
        let state =
            request_state(r#"{"cursors":{"messages":1,"parts":2,"diffTraces":7,"agentTraces":4}}"#)
                .await
                .expect("valid state");

        assert_eq!(state.cursors.diff_traces, 7);
    }

    #[tokio::test]
    async fn state_response_rejects_missing_diff_traces_cursor() {
        let error = request_state(r#"{"cursors":{"messages":1,"parts":2,"agentTraces":4}}"#)
            .await
            .expect_err("missing diffTraces");

        assert!(
            matches!(&error, ControlPlaneError::InvalidResponse(_)),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn state_response_rejects_out_of_range_diff_traces_cursor() {
        let error = request_state(
            r#"{"cursors":{"messages":1,"parts":2,"diffTraces":-1,"agentTraces":4}}"#,
        )
        .await
        .expect_err("negative diffTraces");

        assert!(
            matches!(&error, ControlPlaneError::InvalidResponse(reason) if reason.contains("cursors.diffTraces")),
            "{error:?}"
        );
    }
}
