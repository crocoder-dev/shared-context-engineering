//! Wire-contract DTOs for the control-plane Agent Trace ingestion API.
//!
//! These types define the request/response shapes for
//! `POST /agent-trace/ingestion/state` and `POST /agent-trace/ingestion/batch`.
//! They perform no HTTP I/O and hold no cursor state themselves.

use std::fmt;

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

const STATE_RETRY_MAX_ATTEMPTS: u32 = 3;
const STATE_RETRY_TIMEOUT_MS: u64 = 10_000;
const STATE_RETRY_INITIAL_BACKOFF_MS: u64 = 250;
const STATE_RETRY_MAX_BACKOFF_MS: u64 = 2_000;

/// Typed failure classification for control-plane HTTP interactions, kept
/// separate from `ClassifiedError` so the sync engine and CLI wiring can
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
                "No stored WorkOS credentials were found. Try: run 'sce auth login' before running 'sce trace sync'."
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

/// Seam over `token_storage::{load_tokens, save_tokens}` so tests can assert
/// exactly when a token is (or is not) saved without touching the real,
/// process-wide encrypted auth database.
pub trait CredentialStore: Send + Sync {
    fn load(&self) -> Result<Option<StoredTokens>, ControlPlaneError>;
    fn save(&self, token: &TokenResponse) -> Result<StoredTokens, ControlPlaneError>;
}

/// Production `CredentialStore` backed by the real encrypted auth database.
pub struct SystemCredentialStore;

impl CredentialStore for SystemCredentialStore {
    fn load(&self) -> Result<Option<StoredTokens>, ControlPlaneError> {
        Ok(token_storage::load_tokens()?)
    }

    fn save(&self, token: &TokenResponse) -> Result<StoredTokens, ControlPlaneError> {
        Ok(token_storage::save_tokens(token)?)
    }
}

/// Authenticated HTTP client for the control-plane Agent Trace ingestion API.
///
/// Loads/refreshes stored `WorkOS` credentials through the existing
/// `auth`/`token_storage` primitives, injects the `Authorization: Bearer`
/// header, and retries exactly once on an unexpected `401`.
pub struct AuthenticatedControlPlaneClient {
    http: reqwest::Client,
    base_url: String,
    workos_api_base_url: String,
    workos_client_id: String,
    credential_store: Box<dyn CredentialStore>,
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
            Box::new(SystemCredentialStore),
        )
    }

    pub fn with_credential_store(
        http: reqwest::Client,
        base_url: impl Into<String>,
        workos_api_base_url: impl Into<String>,
        workos_client_id: impl Into<String>,
        credential_store: Box<dyn CredentialStore>,
    ) -> Self {
        Self {
            http,
            base_url: base_url.into(),
            workos_api_base_url: workos_api_base_url.into(),
            workos_client_id: workos_client_id.into(),
            credential_store,
        }
    }

    /// Calls `POST /agent-trace/ingestion/state`, retrying transient
    /// `500`/`503`/transport failures via the existing sync resilience retry
    /// policy. `400`/`403`/post-refresh-`401` are terminal and never retried.
    pub async fn ingestion_state(
        &self,
        request: &AgentTraceIngestionStateRequest,
    ) -> Result<AgentTraceIngestionStateResponse, ControlPlaneError> {
        let url = self.endpoint(STATE_PATH);
        let policy = RetryPolicy {
            max_attempts: STATE_RETRY_MAX_ATTEMPTS,
            timeout_ms: STATE_RETRY_TIMEOUT_MS,
            initial_backoff_ms: STATE_RETRY_INITIAL_BACKOFF_MS,
            max_backoff_ms: STATE_RETRY_MAX_BACKOFF_MS,
        };

        let outcome = run_with_retry(
            policy,
            "agent_trace_sync.ingestion_state",
            "check network connectivity and control-plane availability, then rerun 'sce trace sync'",
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

        let refreshed_token = self.force_refresh_access_token().await?;
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

    /// Loads the stored token, reusing it as-is when still valid and
    /// refreshing (and saving) it only when expired. Makes exactly one
    /// expiry decision, so a token cannot be refreshed without also being
    /// persisted.
    async fn resolve_access_token(&self) -> Result<String, ControlPlaneError> {
        let stored = self
            .credential_store
            .load()?
            .ok_or(ControlPlaneError::MissingCredentials)?;

        if !auth::is_stored_token_expired(&stored)? {
            return Ok(stored.access_token);
        }

        let token = auth::renew_stored_token_from_refresh_token(
            &self.http,
            &self.workos_api_base_url,
            &self.workos_client_id,
            &stored.refresh_token,
        )
        .await?;
        self.credential_store.save(&token)?;

        Ok(token.access_token)
    }

    /// Unconditionally refreshes the stored token via its refresh token and
    /// saves the result, used for the unexpected-`401` retry path.
    async fn force_refresh_access_token(&self) -> Result<String, ControlPlaneError> {
        let stored = self
            .credential_store
            .load()?
            .ok_or(ControlPlaneError::MissingCredentials)?;
        let token = auth::renew_stored_token_from_refresh_token(
            &self.http,
            &self.workos_api_base_url,
            &self.workos_client_id,
            &stored.refresh_token,
        )
        .await?;
        self.credential_store.save(&token)?;
        Ok(token.access_token)
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
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::services::agent_trace_db::MessageRole;
    use crate::services::agent_trace_export::{
        AgentTraceAgentTraceExportRow, AgentTraceDiffTraceExportRow, AgentTraceMessageExportRow,
        AgentTracePartExportRow,
    };
    use crate::services::agent_trace_sync::test_http_server::{CannedResponse, TestHttpServer};
    use serde_json::json;

    #[test]
    fn ingestion_stream_serializes_to_exact_snake_case_wire_values() {
        assert_eq!(
            serde_json::to_value(IngestionStream::Messages).unwrap(),
            json!("messages")
        );
        assert_eq!(
            serde_json::to_value(IngestionStream::Parts).unwrap(),
            json!("parts")
        );
        assert_eq!(
            serde_json::to_value(IngestionStream::DiffTraces).unwrap(),
            json!("diff_traces")
        );
        assert_eq!(
            serde_json::to_value(IngestionStream::AgentTraces).unwrap(),
            json!("agent_traces")
        );
    }

    #[test]
    fn state_request_serializes_to_camel_case_fields() {
        let request = AgentTraceIngestionStateRequest {
            repository_id: "acme-monorepo".to_string(),
            source_instance_id: "src-123".to_string(),
        };

        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({
                "repositoryId": "acme-monorepo",
                "sourceInstanceId": "src-123",
            })
        );
    }

    #[test]
    fn state_response_deserializes_camel_case_cursor_fields() {
        let response: AgentTraceIngestionStateResponse = serde_json::from_value(json!({
            "cursors": {
                "messages": 10,
                "parts": 20,
                "diffTraces": 30,
                "agentTraces": 40,
            }
        }))
        .unwrap();

        assert_eq!(
            response.cursors,
            AgentTraceCursors {
                messages: 10,
                parts: 20,
                diff_traces: 30,
                agent_traces: 40,
            }
        );
    }

    #[test]
    fn agent_trace_cursors_accept_boundary_values() {
        for boundary in [0_i64, 1_i64, agent_trace_export::JS_MAX_SAFE_INTEGER] {
            let cursors = AgentTraceCursors {
                messages: boundary,
                parts: boundary,
                diff_traces: boundary,
                agent_traces: boundary,
            };
            assert!(
                cursors.validate().is_ok(),
                "boundary {boundary} should be valid"
            );
        }
    }

    #[test]
    fn agent_trace_cursors_reject_out_of_range_value_in_each_field() {
        let base = AgentTraceCursors {
            messages: 0,
            parts: 0,
            diff_traces: 0,
            agent_traces: 0,
        };

        for invalid in [-1_i64, agent_trace_export::JS_MAX_SAFE_INTEGER + 1] {
            let cases = [
                AgentTraceCursors {
                    messages: invalid,
                    ..base
                },
                AgentTraceCursors {
                    parts: invalid,
                    ..base
                },
                AgentTraceCursors {
                    diff_traces: invalid,
                    ..base
                },
                AgentTraceCursors {
                    agent_traces: invalid,
                    ..base
                },
            ];

            for cursors in cases {
                let error = cursors
                    .validate()
                    .expect_err(&format!("{invalid} should be rejected in {cursors:?}"));
                assert!(matches!(error, ControlPlaneError::InvalidResponse(_)));
            }
        }
    }

    #[test]
    fn batch_response_deserializes_accepted_and_cursor() {
        let response: AgentTraceIngestionBatchResponse = serde_json::from_value(json!({
            "accepted": 3,
            "cursor": 13,
        }))
        .unwrap();

        assert_eq!(
            response,
            AgentTraceIngestionBatchResponse {
                accepted: 3,
                cursor: 13,
            }
        );
    }

    #[test]
    fn batch_request_composes_with_message_export_rows() {
        let request = AgentTraceIngestionBatchRequest {
            repository_id: "acme-monorepo".to_string(),
            source_instance_id: "src-123".to_string(),
            stream: IngestionStream::Messages,
            expected_cursor: 10,
            rows: vec![AgentTraceMessageExportRow {
                source_row_id: 11,
                session_id: "session-1".to_string(),
                message_id: "message-1".to_string(),
                role: MessageRole::User,
                generated_at_unix_ms: 1_700_000_000_000,
            }],
        };

        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["repositoryId"], json!("acme-monorepo"));
        assert_eq!(value["sourceInstanceId"], json!("src-123"));
        assert_eq!(value["stream"], json!("messages"));
        assert_eq!(value["expectedCursor"], json!(10));
        assert_eq!(value["rows"][0]["sourceRowId"], json!(11));
    }

    #[test]
    fn batch_request_composes_with_part_export_rows() {
        let request = AgentTraceIngestionBatchRequest {
            repository_id: "acme-monorepo".to_string(),
            source_instance_id: "src-123".to_string(),
            stream: IngestionStream::Parts,
            expected_cursor: 0,
            rows: vec![AgentTracePartExportRow {
                source_row_id: 1,
                session_id: "session-1".to_string(),
                message_id: "message-1".to_string(),
                part_type: "text".to_string(),
                text: "hello".to_string(),
                generated_at_unix_ms: 1_700_000_000_000,
            }],
        };

        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["stream"], json!("parts"));
        assert_eq!(value["rows"][0]["type"], json!("text"));
    }

    #[test]
    fn batch_request_composes_with_diff_trace_export_rows() {
        let request = AgentTraceIngestionBatchRequest {
            repository_id: "acme-monorepo".to_string(),
            source_instance_id: "src-123".to_string(),
            stream: IngestionStream::DiffTraces,
            expected_cursor: 0,
            rows: vec![AgentTraceDiffTraceExportRow {
                source_row_id: 1,
                session_id: "session-1".to_string(),
                time_ms: 1_700_000_000_000,
                patch: "diff --git a b".to_string(),
                model_id: None,
                tool_name: None,
                tool_version: None,
                payload_type: "patch".to_string(),
            }],
        };

        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["stream"], json!("diff_traces"));
        assert_eq!(value["rows"][0]["payloadType"], json!("patch"));
    }

    #[test]
    fn batch_request_composes_with_agent_trace_export_rows() {
        let request = AgentTraceIngestionBatchRequest {
            repository_id: "acme-monorepo".to_string(),
            source_instance_id: "src-123".to_string(),
            stream: IngestionStream::AgentTraces,
            expected_cursor: 0,
            rows: vec![AgentTraceAgentTraceExportRow {
                source_row_id: 1,
                agent_trace_id: "agent-trace-1".to_string(),
                commit_id: "abc123".to_string(),
                commit_time_ms: 1_700_000_000_000,
                trace_json: "{}".to_string(),
                url: "https://example.com/trace".to_string(),
                remote_url: None,
            }],
        };

        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["stream"], json!("agent_traces"));
        assert_eq!(value["rows"][0]["agentTraceId"], json!("agent-trace-1"));
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .expect("build test tokio runtime")
            .block_on(future)
    }

    fn now_unix_seconds() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_secs()
    }

    fn valid_stored_tokens(access_token: &str) -> StoredTokens {
        StoredTokens {
            access_token: access_token.to_string(),
            token_type: "Bearer".to_string(),
            expires_in: 3_600,
            refresh_token: "refresh-token-1".to_string(),
            scope: None,
            stored_at_unix_seconds: now_unix_seconds(),
        }
    }

    fn expired_stored_tokens(access_token: &str) -> StoredTokens {
        StoredTokens {
            access_token: access_token.to_string(),
            token_type: "Bearer".to_string(),
            expires_in: 1,
            refresh_token: "refresh-token-1".to_string(),
            scope: None,
            stored_at_unix_seconds: 0,
        }
    }

    fn token_response_json(access_token: &str) -> serde_json::Value {
        json!({
            "access_token": access_token,
            "token_type": "Bearer",
            "expires_in": 3_600,
            "refresh_token": "refresh-token-2",
        })
    }

    fn state_response_json() -> serde_json::Value {
        json!({
            "cursors": {
                "messages": 0,
                "parts": 0,
                "diffTraces": 0,
                "agentTraces": 0,
            }
        })
    }

    fn sample_state_request() -> AgentTraceIngestionStateRequest {
        AgentTraceIngestionStateRequest {
            repository_id: "acme-monorepo".to_string(),
            source_instance_id: "src-123".to_string(),
        }
    }

    fn sample_batch_request() -> AgentTraceIngestionBatchRequest<AgentTraceMessageExportRow> {
        AgentTraceIngestionBatchRequest {
            repository_id: "acme-monorepo".to_string(),
            source_instance_id: "src-123".to_string(),
            stream: IngestionStream::Messages,
            expected_cursor: 0,
            rows: vec![AgentTraceMessageExportRow {
                source_row_id: 1,
                session_id: "session-1".to_string(),
                message_id: "message-1".to_string(),
                role: MessageRole::User,
                generated_at_unix_ms: 1_700_000_000_000,
            }],
        }
    }

    #[derive(Clone, Default)]
    struct FakeCredentialStore {
        tokens: Arc<Mutex<Option<StoredTokens>>>,
        save_calls: Arc<Mutex<Vec<TokenResponse>>>,
    }

    impl FakeCredentialStore {
        fn empty() -> Self {
            Self::default()
        }

        fn with_tokens(tokens: StoredTokens) -> Self {
            let store = Self::default();
            *store.tokens.lock().unwrap() = Some(tokens);
            store
        }

        fn save_call_count(&self) -> usize {
            self.save_calls.lock().unwrap().len()
        }
    }

    impl CredentialStore for FakeCredentialStore {
        fn load(&self) -> Result<Option<StoredTokens>, ControlPlaneError> {
            Ok(self.tokens.lock().unwrap().clone())
        }

        fn save(&self, token: &TokenResponse) -> Result<StoredTokens, ControlPlaneError> {
            self.save_calls.lock().unwrap().push(token.clone());
            let stored = StoredTokens {
                access_token: token.access_token.clone(),
                token_type: token.token_type.clone(),
                expires_in: token.expires_in,
                refresh_token: token.refresh_token.clone(),
                scope: token.scope.clone(),
                stored_at_unix_seconds: now_unix_seconds(),
            };
            *self.tokens.lock().unwrap() = Some(stored.clone());
            Ok(stored)
        }
    }

    /// A plain-`http://` loopback test double never needs TLS root
    /// certificates. Skip platform certificate verification so the test
    /// suite also passes in sandboxes with no system CA store.
    fn test_http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .expect("build test reqwest client")
    }

    fn client_with(
        server: &TestHttpServer,
        credential_store: FakeCredentialStore,
    ) -> AuthenticatedControlPlaneClient {
        AuthenticatedControlPlaneClient::with_credential_store(
            test_http_client(),
            server.base_url.clone(),
            server.base_url.clone(),
            "test-client-id",
            Box::new(credential_store),
        )
    }

    #[test]
    fn missing_credentials_fail_before_any_http_call() {
        let server = TestHttpServer::start();
        let client = client_with(&server, FakeCredentialStore::empty());

        let error = block_on(client.ingestion_state(&sample_state_request())).unwrap_err();

        assert!(matches!(error, ControlPlaneError::MissingCredentials));
        assert!(error.to_string().contains("sce auth login"));
        assert_eq!(server.call_count(), 0);
    }

    #[test]
    fn valid_token_is_reused_without_resave() {
        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(200, &state_response_json()));
        let store = FakeCredentialStore::with_tokens(valid_stored_tokens("valid-access-token"));
        let client = client_with(&server, store);

        let response = block_on(client.ingestion_state(&sample_state_request())).unwrap();

        assert_eq!(response.cursors.messages, 0);
        assert_eq!(server.call_count(), 1);
        let requests = server.captured_requests();
        assert_eq!(
            requests[0].headers.get("authorization").map(String::as_str),
            Some("Bearer valid-access-token")
        );
    }

    #[test]
    fn valid_token_is_not_resaved() {
        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(200, &state_response_json()));
        let store = FakeCredentialStore::with_tokens(valid_stored_tokens("valid-access-token"));
        let store_handle = store.clone();
        let client = client_with(&server, store);

        block_on(client.ingestion_state(&sample_state_request())).unwrap();

        assert_eq!(store_handle.save_call_count(), 0);
    }

    #[test]
    fn expired_token_is_refreshed_and_saved() {
        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(
            200,
            &token_response_json("refreshed-token"),
        ));
        server.queue_response(CannedResponse::json(200, &state_response_json()));
        let store = FakeCredentialStore::with_tokens(expired_stored_tokens("stale-access-token"));
        let store_handle = store.clone();
        let client = client_with(&server, store);

        let response = block_on(client.ingestion_state(&sample_state_request())).unwrap();

        assert_eq!(response.cursors.messages, 0);
        assert_eq!(server.call_count(), 2);
        assert_eq!(store_handle.save_call_count(), 1);
        let requests = server.captured_requests();
        assert_eq!(
            requests[1].headers.get("authorization").map(String::as_str),
            Some("Bearer refreshed-token")
        );
    }

    #[test]
    fn unexpected_401_refreshes_once_and_retries_once_on_success() {
        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(401, &json!({"error": "unauthorized"})));
        server.queue_response(CannedResponse::json(
            200,
            &token_response_json("refreshed-token"),
        ));
        server.queue_response(CannedResponse::json(200, &state_response_json()));
        let store = FakeCredentialStore::with_tokens(valid_stored_tokens("stale-but-unexpired"));
        let store_handle = store.clone();
        let client = client_with(&server, store);

        let response = block_on(client.ingestion_state(&sample_state_request())).unwrap();

        assert_eq!(response.cursors.messages, 0);
        assert_eq!(server.call_count(), 3);
        assert_eq!(store_handle.save_call_count(), 1);
        let requests = server.captured_requests();
        assert_eq!(
            requests[2].headers.get("authorization").map(String::as_str),
            Some("Bearer refreshed-token")
        );
    }

    #[test]
    fn unexpected_401_twice_fails_without_a_third_attempt() {
        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(401, &json!({"error": "unauthorized"})));
        server.queue_response(CannedResponse::json(
            200,
            &token_response_json("refreshed-token"),
        ));
        server.queue_response(CannedResponse::json(401, &json!({"error": "unauthorized"})));
        let store = FakeCredentialStore::with_tokens(valid_stored_tokens("stale-but-unexpired"));
        let client = client_with(&server, store);

        let error = block_on(client.ingestion_state(&sample_state_request())).unwrap_err();

        assert!(matches!(error, ControlPlaneError::AuthenticationFailed(_)));
        assert_eq!(server.call_count(), 3);
    }

    #[test]
    fn state_request_body_has_exact_shape_and_call_count() {
        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(200, &state_response_json()));
        let store = FakeCredentialStore::with_tokens(valid_stored_tokens("valid-access-token"));
        let client = client_with(&server, store);

        block_on(client.ingestion_state(&sample_state_request())).unwrap();

        assert_eq!(server.call_count(), 1);
        let requests = server.captured_requests();
        assert_eq!(requests[0].path, "/agent-trace/ingestion/state");
        let body: serde_json::Value = serde_json::from_str(&requests[0].body).unwrap();
        assert_eq!(
            body,
            json!({
                "repositoryId": "acme-monorepo",
                "sourceInstanceId": "src-123",
            })
        );
    }

    #[test]
    fn batch_classifies_400_as_bad_request() {
        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(400, &json!({"error": "invalid"})));
        let store = FakeCredentialStore::with_tokens(valid_stored_tokens("valid-access-token"));
        let client = client_with(&server, store);

        let error = block_on(client.ingest_messages(&sample_batch_request())).unwrap_err();

        assert!(matches!(error, ControlPlaneError::BadRequest(_)));
    }

    #[test]
    fn batch_classifies_403_as_forbidden() {
        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(403, &json!({"error": "forbidden"})));
        let store = FakeCredentialStore::with_tokens(valid_stored_tokens("valid-access-token"));
        let client = client_with(&server, store);

        let error = block_on(client.ingest_messages(&sample_batch_request())).unwrap_err();

        assert!(matches!(error, ControlPlaneError::Forbidden(_)));
    }

    #[test]
    fn batch_classifies_409_as_conflict() {
        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(409, &json!({"error": "conflict"})));
        let store = FakeCredentialStore::with_tokens(valid_stored_tokens("valid-access-token"));
        let client = client_with(&server, store);

        let error = block_on(client.ingest_messages(&sample_batch_request())).unwrap_err();

        assert!(matches!(error, ControlPlaneError::Conflict(_)));
    }

    #[test]
    fn batch_classifies_5xx_as_server_error() {
        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(500, &json!({"error": "boom"})));
        let store = FakeCredentialStore::with_tokens(valid_stored_tokens("valid-access-token"));
        let client = client_with(&server, store);

        let error = block_on(client.ingest_messages(&sample_batch_request())).unwrap_err();

        assert!(matches!(error, ControlPlaneError::ServerError(_)));
    }

    #[test]
    fn server_error_body_is_not_leaked() {
        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::text(
            500,
            "SQLITE_ERROR: no such table: agent_trace_messages at /var/lib/sce/db.sqlite3",
        ));
        let store = FakeCredentialStore::with_tokens(valid_stored_tokens("valid-access-token"));
        let client = client_with(&server, store);

        let error = block_on(client.ingest_messages(&sample_batch_request())).unwrap_err();

        let rendered = error.to_string();
        assert!(matches!(error, ControlPlaneError::ServerError(_)));
        assert!(!rendered.contains("SQLITE_ERROR"));
        assert!(!rendered.contains("agent_trace_messages"));
        assert!(!rendered.contains("/var/lib/sce"));
        assert!(rendered.contains("control plane encountered an internal error"));
    }

    #[test]
    fn known_safe_error_payload_is_surfaced() {
        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(
            400,
            &json!({"message": "repositoryId is required"}),
        ));
        let store = FakeCredentialStore::with_tokens(valid_stored_tokens("valid-access-token"));
        let client = client_with(&server, store);

        let error = block_on(client.ingest_messages(&sample_batch_request())).unwrap_err();

        match error {
            ControlPlaneError::BadRequest(message) => {
                assert_eq!(message, "repositoryId is required");
            }
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn html_error_body_falls_back_to_generic_message() {
        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::text(
            500,
            "<html><body><h1>500 Internal Server Error</h1></body></html>",
        ));
        let store = FakeCredentialStore::with_tokens(valid_stored_tokens("valid-access-token"));
        let client = client_with(&server, store);

        let error = block_on(client.ingest_messages(&sample_batch_request())).unwrap_err();

        let rendered = error.to_string();
        assert!(!rendered.contains("<html>"));
        assert!(rendered.contains("control plane encountered an internal error"));
    }

    #[test]
    fn batch_classifies_404_as_protocol_error() {
        let server = TestHttpServer::start();
        server.queue_response(CannedResponse::json(404, &json!({"error": "not found"})));
        let store = FakeCredentialStore::with_tokens(valid_stored_tokens("valid-access-token"));
        let client = client_with(&server, store);

        let error = block_on(client.ingest_messages(&sample_batch_request())).unwrap_err();

        match error {
            ControlPlaneError::Protocol { status, message } => {
                assert_eq!(status, StatusCode::NOT_FOUND);
                assert_eq!(message, "not found");
            }
            other => panic!("expected Protocol, got {other:?}"),
        }
    }
}
