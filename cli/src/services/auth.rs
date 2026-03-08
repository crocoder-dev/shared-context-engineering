use std::fmt;

use serde::{Deserialize, Serialize};

pub const DEVICE_CODE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
pub const REFRESH_TOKEN_GRANT_TYPE: &str = "refresh_token";

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

#[derive(Debug)]
pub enum AuthError {
    MissingClientId,
    InvalidResponse(String),
    Unauthorized(String),
    RequestFailed(reqwest::Error),
    Io(std::io::Error),
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

#[cfg(test)]
mod tests {
    use super::{
        DeviceAuthorizationResponse, DeviceTokenPollRequest, OAuthErrorResponse, TokenResponse,
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
}
