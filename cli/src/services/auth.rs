use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::services::token_storage::{save_tokens, StoredTokens, TokenStorageError};

pub const DEVICE_CODE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
pub const REFRESH_TOKEN_GRANT_TYPE: &str = "refresh_token";
pub const WORKOS_DEFAULT_BASE_URL: &str = "https://api.workos.com";
pub const DEFAULT_DEVICE_POLL_INTERVAL_SECONDS: u64 = 5;

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
pub struct RefreshTokenRequest {
    pub grant_type: String,
    pub refresh_token: String,
    pub client_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub refresh_token: String,
    pub scope: Option<String>,
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
                "WorkOS client ID is not configured. Try: set WORKOS_CLIENT_ID or configure the CLI auth client id."
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
    let token = poll_for_device_token(client, api_base_url, client_id, &authorization).await?;
    let stored_tokens = save_tokens(&token)?;

    Ok(DeviceAuthFlowResult {
        authorization,
        stored_tokens,
    })
}

async fn request_device_authorization(
    client: &reqwest::Client,
    api_base_url: &str,
    client_id: &str,
) -> Result<DeviceAuthorizationResponse, AuthError> {
    let endpoint = format!(
        "{}/oauth/device/authorize",
        api_base_url.trim_end_matches('/')
    );
    let request = DeviceAuthorizationRequest {
        client_id: client_id.to_string(),
    };

    let response = client.post(endpoint).json(&request).send().await?;

    if response.status().is_success() {
        let parsed = response
            .json::<DeviceAuthorizationResponse>()
            .await
            .map_err(AuthError::RequestFailed)?;
        if parsed.device_code.trim().is_empty()
            || parsed.user_code.trim().is_empty()
            || parsed.verification_uri.trim().is_empty()
        {
            return Err(AuthError::InvalidResponse(
                "device authorization response is missing required fields".to_string(),
            ));
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
    let endpoint = format!("{}/oauth/device/token", api_base_url.trim_end_matches('/'));
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
                "WorkOS device authorization expired before approval completed. Try: run 'sce login' again and complete verification before the code expires.".to_string(),
            ));
        }

        let response = client.post(&endpoint).json(&request).send().await?;
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

#[cfg(test)]
mod tests {
    use super::{
        map_oauth_terminal_error, poll_decision_for_error_code, DeviceAuthorizationResponse,
        DeviceTokenPollRequest, OAuthErrorResponse, PollDecision, TokenResponse,
        DEVICE_CODE_GRANT_TYPE,
    };

    #[test]
    fn device_authorization_response_deserializes_from_workos_shape() {
        let payload = r#"{
            "device_code": "dev_123",
            "user_code": "ABCD-EFGH",
            "verification_uri": "https://workos.com/device",
            "verification_uri_complete": "https://workos.com/device?user_code=ABCD-EFGH",
            "expires_in": 900,
            "interval": 5
        }"#;

        let parsed: DeviceAuthorizationResponse =
            serde_json::from_str(payload).expect("device auth response should parse");

        assert_eq!(parsed.device_code, "dev_123");
        assert_eq!(parsed.user_code, "ABCD-EFGH");
        assert_eq!(parsed.expires_in, 900);
        assert_eq!(parsed.interval, Some(5));
        assert_eq!(
            parsed.verification_uri_complete.as_deref(),
            Some("https://workos.com/device?user_code=ABCD-EFGH")
        );
    }

    #[test]
    fn token_response_serializes_and_deserializes() {
        let token = TokenResponse {
            access_token: "access_123".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: 3600,
            refresh_token: "refresh_123".to_string(),
            scope: Some("openid profile".to_string()),
        };

        let encoded = serde_json::to_string(&token).expect("token response should serialize");
        let decoded: TokenResponse =
            serde_json::from_str(&encoded).expect("token response should deserialize");

        assert_eq!(decoded, token);
    }

    #[test]
    fn device_token_poll_request_uses_rfc8628_grant_type_constant() {
        let request = DeviceTokenPollRequest {
            grant_type: DEVICE_CODE_GRANT_TYPE.to_string(),
            device_code: "device_abc".to_string(),
            client_id: "client_abc".to_string(),
        };

        let encoded = serde_json::to_string(&request).expect("poll request should serialize");
        assert!(encoded.contains(DEVICE_CODE_GRANT_TYPE));
    }

    #[test]
    fn oauth_error_response_deserializes_optional_fields() {
        let payload = r#"{
            "error": "authorization_pending",
            "error_description": "Authorization pending"
        }"#;

        let parsed: OAuthErrorResponse =
            serde_json::from_str(payload).expect("oauth error payload should parse");

        assert_eq!(parsed.error, "authorization_pending");
        assert_eq!(
            parsed.error_description.as_deref(),
            Some("Authorization pending")
        );
        assert_eq!(parsed.error_uri, None);
    }

    #[test]
    fn oauth_error_mapping_for_all_required_terminal_codes_has_try_guidance() {
        let codes = [
            "access_denied",
            "expired_token",
            "invalid_request",
            "invalid_client",
            "invalid_grant",
            "unsupported_grant_type",
        ];

        for code in codes {
            let message = map_oauth_terminal_error(code, Some("detail")).to_string();
            assert!(message.contains("Try:"), "missing Try guidance for {code}");
        }
    }

    #[test]
    fn oauth_error_mapping_includes_original_code_for_unknown_errors() {
        let message = map_oauth_terminal_error("unexpected_error", None).to_string();
        assert!(message.contains("unexpected_error"));
        assert!(message.contains("Try:"));
    }

    #[test]
    fn poll_decision_uses_fixed_interval_and_slow_down_increment_path() {
        assert_eq!(
            poll_decision_for_error_code("authorization_pending"),
            PollDecision::Continue
        );
        assert_eq!(
            poll_decision_for_error_code("slow_down"),
            PollDecision::SlowDown
        );
    }

    #[test]
    fn poll_decision_stops_for_terminal_oauth_errors() {
        assert_eq!(
            poll_decision_for_error_code("access_denied"),
            PollDecision::Stop
        );
        assert_eq!(
            poll_decision_for_error_code("invalid_client"),
            PollDecision::Stop
        );
    }
}
