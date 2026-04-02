use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use crate::services::resilience::{run_with_retry, RetryPolicy};
use crate::services::token_storage::{load_tokens, save_tokens, StoredTokens, TokenStorageError};

pub const DEVICE_CODE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
#[allow(dead_code)]
pub const REFRESH_TOKEN_GRANT_TYPE: &str = "refresh_token";
pub const WORKOS_DEFAULT_BASE_URL: &str = "https://api.workos.com";
pub const DEFAULT_DEVICE_POLL_INTERVAL_SECONDS: u64 = 5;
#[allow(dead_code)]
const TOKEN_EXPIRY_SKEW_SECONDS: u64 = 30;
#[allow(dead_code)]
const TOKEN_REFRESH_MAX_ATTEMPTS: u32 = 3;
#[allow(dead_code)]
const TOKEN_REFRESH_TIMEOUT_MS: u64 = 10_000;
#[allow(dead_code)]
const TOKEN_REFRESH_INITIAL_BACKOFF_MS: u64 = 250;
#[allow(dead_code)]
const TOKEN_REFRESH_MAX_BACKOFF_MS: u64 = 2_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeviceAuthorizationRequest {
    pub client_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeviceAuthorizationResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeviceTokenPollRequest {
    pub grant_type: String,
    pub device_code: String,
    pub client_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct RefreshTokenRequest {
    pub grant_type: String,
    pub refresh_token: String,
    pub client_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default = "default_token_type")]
    pub token_type: String,
    #[serde(default = "default_access_token_expires_in")]
    pub expires_in: u64,
    pub refresh_token: String,
    #[serde(default)]
    pub scope: Option<String>,
}

fn default_token_type() -> String {
    String::from("Bearer")
}

fn default_access_token_expires_in() -> u64 {
    3600
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OAuthErrorResponse {
    pub error: String,
    pub error_description: Option<String>,
    pub error_uri: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceAuthFlowResult {
    pub authorization: DeviceAuthorizationResponse,
    pub stored_tokens: StoredTokens,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PollDecision {
    Continue,
    SlowDown,
    Stop,
}

#[derive(Debug)]
pub enum AuthError {
    MissingClientId,
    InvalidResponse(String),
    Unauthorized(String),
    RequestFailed(reqwest::Error),
    Io(std::io::Error),
    Storage(TokenStorageError),
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingClientId => write!(
                f,
                "WorkOS client ID is not configured. Try: set WORKOS_CLIENT_ID, add workos_client_id to an SCE config file, or remove an invalid higher-precedence override so the baked default can apply."
            ),
            Self::InvalidResponse(reason) => write!(
                f,
                "WorkOS auth response was invalid: {reason}. Try: retry the command and verify WorkOS app settings."
            ),
            Self::Unauthorized(reason) => write!(
                f,
                "WorkOS authentication request was rejected: {reason}. Try: verify the client ID and rerun login."
            ),
            Self::RequestFailed(error) => {
                write!(f, "WorkOS authentication request failed: {error}")
            }
            Self::Io(error) => write!(f, "Authentication storage operation failed: {error}"),
            Self::Storage(error) => write!(f, "Authentication storage operation failed: {error}"),
        }
    }
}

impl std::error::Error for AuthError {}

impl From<reqwest::Error> for AuthError {
    fn from(value: reqwest::Error) -> Self {
        Self::RequestFailed(value)
    }
}

impl From<std::io::Error> for AuthError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<TokenStorageError> for AuthError {
    fn from(value: TokenStorageError) -> Self {
        Self::Storage(value)
    }
}

pub async fn start_device_auth_flow(
    client: &reqwest::Client,
    api_base_url: &str,
    client_id: &str,
) -> Result<DeviceAuthFlowResult, AuthError> {
    if client_id.trim().is_empty() {
        return Err(AuthError::MissingClientId);
    }

    let authorization = request_device_authorization(client, api_base_url, client_id).await?;
    let stored_tokens =
        complete_device_auth_flow(client, api_base_url, client_id, &authorization).await?;

    Ok(DeviceAuthFlowResult {
        authorization,
        stored_tokens,
    })
}

pub async fn complete_device_auth_flow(
    client: &reqwest::Client,
    api_base_url: &str,
    client_id: &str,
    authorization: &DeviceAuthorizationResponse,
) -> Result<StoredTokens, AuthError> {
    if client_id.trim().is_empty() {
        return Err(AuthError::MissingClientId);
    }

    let token = poll_for_device_token(client, api_base_url, client_id, authorization).await?;
    let stored_tokens = save_tokens(&token)?;
    Ok(stored_tokens)
}

#[allow(dead_code)]
pub async fn ensure_valid_token(
    client: &reqwest::Client,
    api_base_url: &str,
    client_id: &str,
) -> Result<StoredTokens, AuthError> {
    if client_id.trim().is_empty() {
        return Err(AuthError::MissingClientId);
    }

    let Some(stored) = load_tokens()? else {
        return Err(AuthError::Unauthorized(
            String::from("No stored WorkOS credentials were found. Try: run 'sce login' before running authenticated commands."),
        ));
    };

    let now_unix_seconds = current_unix_timestamp_seconds()?;
    if !is_token_expired(&stored, now_unix_seconds) {
        return Ok(stored);
    }

    let refreshed = refresh_access_token(client, api_base_url, client_id, &stored.refresh_token)
        .await
        .map_err(map_refresh_failure_for_public_cli)?;
    let updated = save_tokens(&refreshed)?;
    Ok(updated)
}

pub async fn renew_stored_token(
    client: &reqwest::Client,
    api_base_url: &str,
    client_id: &str,
) -> Result<StoredTokens, AuthError> {
    if client_id.trim().is_empty() {
        return Err(AuthError::MissingClientId);
    }

    let Some(stored) = load_tokens()? else {
        return Err(AuthError::Unauthorized(
            String::from("No stored WorkOS credentials were found. Try: run 'sce auth login' before running authenticated commands."),
        ));
    };

    let refreshed = refresh_access_token(client, api_base_url, client_id, &stored.refresh_token)
        .await
        .map_err(map_refresh_failure_for_public_cli)?;
    let updated = save_tokens(&refreshed)?;
    Ok(updated)
}

pub fn is_stored_token_expired(stored: &StoredTokens) -> Result<bool, AuthError> {
    let now_unix_seconds = current_unix_timestamp_seconds()?;
    Ok(is_token_expired(stored, now_unix_seconds))
}

pub async fn request_device_authorization(
    client: &reqwest::Client,
    api_base_url: &str,
    client_id: &str,
) -> Result<DeviceAuthorizationResponse, AuthError> {
    let endpoint = device_authorization_endpoint(api_base_url);
    let request = DeviceAuthorizationRequest {
        client_id: client_id.to_string(),
    };

    let response = client.post(endpoint).form(&request).send().await?;

    if response.status().is_success() {
        let parsed = response
            .json::<DeviceAuthorizationResponse>()
            .await
            .map_err(AuthError::RequestFailed)?;
        if parsed.device_code.trim().is_empty()
            || parsed.user_code.trim().is_empty()
            || parsed.verification_uri.trim().is_empty()
        {
            return Err(AuthError::InvalidResponse(String::from(
                "device authorization response is missing required fields",
            )));
        }
        return Ok(parsed);
    }

    let oauth_error = parse_oauth_error_response(response).await?;
    Err(map_oauth_terminal_error(
        &oauth_error.error,
        oauth_error.error_description.as_deref(),
    ))
}

async fn poll_for_device_token(
    client: &reqwest::Client,
    api_base_url: &str,
    client_id: &str,
    authorization: &DeviceAuthorizationResponse,
) -> Result<TokenResponse, AuthError> {
    let endpoint = token_endpoint(api_base_url);
    let request = DeviceTokenPollRequest {
        grant_type: DEVICE_CODE_GRANT_TYPE.to_string(),
        device_code: authorization.device_code.clone(),
        client_id: client_id.to_string(),
    };

    let mut poll_interval_seconds = authorization
        .interval
        .unwrap_or(DEFAULT_DEVICE_POLL_INTERVAL_SECONDS)
        .max(1);
    let max_polls = authorization
        .expires_in
        .saturating_div(poll_interval_seconds)
        .max(1)
        + 1;
    let mut attempts = 0_u64;

    loop {
        attempts = attempts.saturating_add(1);
        if attempts > max_polls {
            return Err(AuthError::Unauthorized(
                String::from("WorkOS device authorization expired before approval completed. Try: run 'sce login' again and complete verification before the code expires."),
            ));
        }

        let response = client.post(&endpoint).form(&request).send().await?;
        if response.status().is_success() {
            let token = response
                .json::<TokenResponse>()
                .await
                .map_err(AuthError::RequestFailed)?;
            return Ok(token);
        }

        let oauth_error = parse_oauth_error_response(response).await?;
        match poll_decision_for_error_code(&oauth_error.error) {
            PollDecision::Continue => {
                tokio::time::sleep(Duration::from_secs(poll_interval_seconds)).await;
            }
            PollDecision::SlowDown => {
                poll_interval_seconds = poll_interval_seconds.saturating_add(5);
                tokio::time::sleep(Duration::from_secs(poll_interval_seconds)).await;
            }
            PollDecision::Stop => {
                return Err(map_oauth_terminal_error(
                    &oauth_error.error,
                    oauth_error.error_description.as_deref(),
                ));
            }
        }
    }
}

fn poll_decision_for_error_code(code: &str) -> PollDecision {
    match code {
        "authorization_pending" => PollDecision::Continue,
        "slow_down" => PollDecision::SlowDown,
        _ => PollDecision::Stop,
    }
}

#[allow(dead_code)]
fn is_token_expired(stored: &StoredTokens, now_unix_seconds: u64) -> bool {
    let lifetime_seconds = stored.expires_in.saturating_sub(TOKEN_EXPIRY_SKEW_SECONDS);
    let expires_at = stored
        .stored_at_unix_seconds
        .saturating_add(lifetime_seconds);
    now_unix_seconds >= expires_at
}

#[allow(dead_code)]
fn current_unix_timestamp_seconds() -> Result<u64, AuthError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| {
            AuthError::InvalidResponse(format!(
                "system clock is invalid for token expiry checks: {error}"
            ))
        })
}

#[allow(dead_code)]
async fn refresh_access_token(
    client: &reqwest::Client,
    api_base_url: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<TokenResponse, AuthError> {
    if refresh_token.trim().is_empty() {
        return Err(AuthError::Unauthorized(
            "Stored WorkOS refresh token is missing. Try: run 'sce login' to authenticate again."
                .to_string(),
        ));
    }

    let endpoint = token_endpoint(api_base_url);
    let request = RefreshTokenRequest {
        grant_type: REFRESH_TOKEN_GRANT_TYPE.to_string(),
        refresh_token: refresh_token.to_string(),
        client_id: client_id.to_string(),
    };
    let retry_policy = RetryPolicy {
        max_attempts: TOKEN_REFRESH_MAX_ATTEMPTS,
        timeout_ms: TOKEN_REFRESH_TIMEOUT_MS,
        initial_backoff_ms: TOKEN_REFRESH_INITIAL_BACKOFF_MS,
        max_backoff_ms: TOKEN_REFRESH_MAX_BACKOFF_MS,
    };

    let response = run_with_retry(
        retry_policy,
        "auth.refresh_token",
        "check network connectivity and rerun the command",
        |_| {
            let endpoint = endpoint.clone();
            let request = request.clone();
            async move {
                client
                    .post(&endpoint)
                    .form(&request)
                    .send()
                    .await
                    .map_err(|error| anyhow!(error))
            }
        },
    )
    .await
    .map_err(|error| {
        AuthError::Unauthorized(format!(
            "WorkOS token refresh failed due to repeated transient errors: {error}. Try: rerun the command; if this persists, run 'sce login' to re-authenticate."
        ))
    })?;

    if response.status().is_success() {
        let token = response
            .json::<TokenResponse>()
            .await
            .map_err(AuthError::RequestFailed)?;
        return Ok(token);
    }

    let oauth_error = parse_oauth_error_response(response).await?;
    Err(map_refresh_terminal_error(
        &oauth_error.error,
        oauth_error.error_description.as_deref(),
    ))
}

#[allow(dead_code)]
fn map_refresh_terminal_error(code: &str, description: Option<&str>) -> AuthError {
    let detail = description
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!(" ({value})"))
        .unwrap_or_default();

    match code {
        "invalid_grant" | "expired_token" => AuthError::Unauthorized(format!(
            "Stored WorkOS refresh token is no longer valid{detail}. Try: run 'sce auth login' again to authenticate again."
        )),
        "invalid_client" => AuthError::Unauthorized(format!(
            "WorkOS rejected automatic token refresh for this public CLI client{detail}. Try: run 'sce auth login' again to re-authenticate."
        )),
        "invalid_request" => AuthError::Unauthorized(format!(
            "WorkOS rejected the refresh token request as invalid{detail}. Try: run 'sce auth login' again to reset local credentials."
        )),
        "unsupported_grant_type" => AuthError::Unauthorized(format!(
            "WorkOS rejected the refresh OAuth grant type{detail}. Try: update the CLI and rerun 'sce auth login'."
        )),
        "access_denied" => AuthError::Unauthorized(format!(
            "WorkOS denied the refresh token request{detail}. Try: run 'sce auth login' again to re-authenticate."
        )),
        other => AuthError::Unauthorized(format!(
            "WorkOS returned OAuth error '{other}' while refreshing credentials{detail}. Try: run 'sce auth login' again to restore authentication."
        )),
    }
}

fn map_refresh_failure_for_public_cli(error: AuthError) -> AuthError {
    match error {
        AuthError::Unauthorized(reason) => AuthError::Unauthorized(format!(
            "Stored WorkOS access token expired and automatic refresh did not succeed for this public CLI. {reason}"
        )),
        other => other,
    }
}

async fn parse_oauth_error_response(
    response: reqwest::Response,
) -> Result<OAuthErrorResponse, AuthError> {
    response
        .json::<OAuthErrorResponse>()
        .await
        .map_err(|error| {
            AuthError::InvalidResponse(format!("unable to parse OAuth error payload: {error}"))
        })
}

fn map_oauth_terminal_error(code: &str, description: Option<&str>) -> AuthError {
    let detail = description
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!(" ({value})"))
        .unwrap_or_default();

    match code {
        "access_denied" => AuthError::Unauthorized(format!(
            "WorkOS login was declined by the user{detail}. Try: rerun 'sce login' and approve the request in the browser."
        )),
        "expired_token" => AuthError::Unauthorized(format!(
            "WorkOS device code expired{detail}. Try: rerun 'sce login' to request a fresh device code."
        )),
        "invalid_request" => AuthError::Unauthorized(format!(
            "WorkOS rejected the device auth request as invalid{detail}. Try: verify CLI auth parameters and rerun 'sce login'."
        )),
        "invalid_client" => AuthError::Unauthorized(format!(
            "WorkOS rejected the client configuration{detail}. Try: verify WORKOS_CLIENT_ID (or config value) and rerun 'sce login'."
        )),
        "invalid_grant" => AuthError::Unauthorized(format!(
            "WorkOS reported an invalid or already-used device code{detail}. Try: rerun 'sce login' to restart the device flow."
        )),
        "unsupported_grant_type" => AuthError::Unauthorized(format!(
            "WorkOS rejected the OAuth grant type{detail}. Try: update the CLI and rerun 'sce login'."
        )),
        other => AuthError::Unauthorized(format!(
            "WorkOS returned OAuth error '{other}'{detail}. Try: rerun 'sce login'; if the issue persists, check WorkOS auth configuration."
        )),
    }
}

fn device_authorization_endpoint(api_base_url: &str) -> String {
    let base_url = api_base_url.trim_end_matches('/');
    if uses_connect_oauth_endpoints(api_base_url) {
        format!("{base_url}/oauth2/device_authorization")
    } else {
        format!("{base_url}/user_management/authorize/device")
    }
}

fn token_endpoint(api_base_url: &str) -> String {
    let base_url = api_base_url.trim_end_matches('/');
    if uses_connect_oauth_endpoints(api_base_url) {
        format!("{base_url}/oauth2/token")
    } else {
        format!("{base_url}/user_management/authenticate")
    }
}

fn uses_connect_oauth_endpoints(api_base_url: &str) -> bool {
    match reqwest::Url::parse(api_base_url) {
        Ok(url) => url.host_str() != Some("api.workos.com"),
        Err(_) => api_base_url.trim_end_matches('/') != WORKOS_DEFAULT_BASE_URL,
    }
}
