//! WorkOS authentication service types for OAuth 2.0 Device Authorization Flow (RFC 8628).
//!
//! This module provides type definitions for WorkOS authentication flows.
//! Actual HTTP calls and token storage are handled in separate modules.

use serde::{Deserialize, Serialize};

/// Default WorkOS API base URL.
pub const WORKOS_API_BASE_URL: &str = "https://api.workos.com";

/// Default AuthKit verification URL pattern (domain placeholder replaced at runtime).
pub const WORKOS_AUTHKIT_DEVICE_URL_TEMPLATE: &str = "https://{domain}.authkit.app/device";

/// OAuth 2.0 grant type for Device Authorization Flow.
pub const GRANT_TYPE_DEVICE_CODE: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// OAuth 2.0 grant type for refresh token exchange.
pub const GRANT_TYPE_REFRESH_TOKEN: &str = "refresh_token";

// ============================================================================
// Device Code Flow Types
// ============================================================================

/// Request to initiate Device Authorization Flow.
/// POST to `/user_management/authorize/device`
#[derive(Clone, Debug, Serialize)]
pub struct DeviceCodeRequest {
    /// WorkOS client ID.
    pub client_id: String,
    /// OAuth scopes requested (space-separated). Optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// Response from Device Authorization endpoint.
#[derive(Clone, Debug, Deserialize)]
pub struct DeviceCodeResponse {
    /// The device verification code.
    pub device_code: String,
    /// The end-user verification code (displayed to user).
    pub user_code: String,
    /// The verification URL where user enters the user code.
    pub verification_url: String,
    /// Verification URL with user_code pre-filled (optional).
    #[serde(default)]
    pub verification_url_complete: Option<String>,
    /// Lifetime of device code in seconds.
    pub expires_in: u32,
    /// Minimum polling interval in seconds.
    pub interval: u32,
}

// ============================================================================
// Token Request/Response Types
// ============================================================================

/// Request to poll for token during Device Authorization Flow.
/// POST to `/user_management/authenticate`
#[derive(Clone, Debug, Serialize)]
pub struct TokenRequest {
    /// WorkOS client ID.
    pub client_id: String,
    /// The device code from DeviceCodeResponse.
    pub device_code: String,
    /// Must be `urn:ietf:params:oauth:grant-type:device_code`.
    pub grant_type: &'static str,
}

/// Request to refresh an access token.
/// POST to `/user_management/authenticate`
#[derive(Clone, Debug, Serialize)]
pub struct RefreshTokenRequest {
    /// WorkOS client ID.
    pub client_id: String,
    /// The refresh token.
    pub refresh_token: String,
    /// Must be `refresh_token`.
    pub grant_type: &'static str,
}

/// Successful token response.
#[derive(Clone, Debug, Deserialize)]
pub struct TokenResponse {
    /// The access token.
    pub access_token: String,
    /// The refresh token (for obtaining new access tokens).
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Token type (typically "Bearer").
    pub token_type: String,
    /// Access token lifetime in seconds.
    pub expires_in: u32,
    /// ID token containing user information (JWT).
    #[serde(default)]
    pub id_token: Option<String>,
    /// OAuth scopes granted (space-separated).
    #[serde(default)]
    pub scope: Option<String>,
}

/// Error response from token endpoint.
#[derive(Clone, Debug, Deserialize)]
pub struct TokenErrorResponse {
    /// Error code (e.g., "authorization_pending", "access_denied").
    pub error: String,
    /// Human-readable error description.
    #[serde(default)]
    pub error_description: Option<String>,
}

// ============================================================================
// Stored Token Types
// ============================================================================

/// Tokens stored locally for authentication persistence.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredTokens {
    /// The access token.
    pub access_token: String,
    /// The refresh token.
    pub refresh_token: String,
    /// Unix timestamp when the access token expires.
    pub expires_at: u64,
    /// ID token containing user information (JWT).
    #[serde(default)]
    pub id_token: Option<String>,
    /// OAuth scopes granted.
    #[serde(default)]
    pub scope: Option<String>,
}

// ============================================================================
// WorkOS Configuration Types
// ============================================================================

/// WorkOS client configuration.
#[derive(Clone, Debug)]
pub struct WorkOSConfig {
    /// WorkOS client ID.
    pub client_id: String,
    /// AuthKit domain (e.g., "your-app" for your-app.authkit.app).
    pub domain: String,
    /// WorkOS API base URL (defaults to WORKOS_API_BASE_URL).
    pub api_base_url: String,
}

impl WorkOSConfig {
    /// Creates a new WorkOS configuration.
    pub fn new(client_id: String, domain: String) -> Self {
        Self {
            client_id,
            domain,
            api_base_url: WORKOS_API_BASE_URL.to_string(),
        }
    }

    /// Returns the AuthKit device verification URL.
    pub fn verification_url(&self) -> String {
        WORKOS_AUTHKIT_DEVICE_URL_TEMPLATE.replace("{domain}", &self.domain)
    }
}

// ============================================================================
// Error Types
// ============================================================================

/// Authentication-specific errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthError {
    /// The user has not yet completed authentication.
    AuthorizationPending,
    /// Polling too frequently; increase interval.
    SlowDown,
    /// The user denied the authorization request.
    AccessDenied,
    /// The device code has expired.
    ExpiredToken,
    /// The grant is invalid or expired.
    InvalidGrant,
    /// The client is not authorized.
    InvalidClient,
    /// Invalid request parameters.
    InvalidRequest,
    /// Network error during authentication.
    NetworkError(String),
    /// Token storage error.
    StorageError(String),
    /// Configuration error (missing client ID, domain, etc.).
    ConfigurationError(String),
    /// Unexpected error.
    Unexpected(String),
}

impl AuthError {
    /// Returns the error code string for this error.
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::AuthorizationPending => "authorization_pending",
            Self::SlowDown => "slow_down",
            Self::AccessDenied => "access_denied",
            Self::ExpiredToken => "expired_token",
            Self::InvalidGrant => "invalid_grant",
            Self::InvalidClient => "invalid_client",
            Self::InvalidRequest => "invalid_request",
            Self::NetworkError(_) => "network_error",
            Self::StorageError(_) => "storage_error",
            Self::ConfigurationError(_) => "configuration_error",
            Self::Unexpected(_) => "unexpected_error",
        }
    }

    /// Returns actionable user guidance for this error.
    pub fn user_guidance(&self) -> String {
        match self {
            Self::AuthorizationPending => {
                "Waiting for you to complete authentication in your browser.".to_string()
            }
            Self::SlowDown => {
                "Polling too frequently. Will retry with longer interval.".to_string()
            }
            Self::AccessDenied => {
                "Authentication was denied. Try: Run `sce login` again and approve the request.".to_string()
            }
            Self::ExpiredToken => {
                "The login code expired. Try: Run `sce login` again.".to_string()
            }
            Self::InvalidGrant => {
                "The authentication grant is invalid. Try: Run `sce login` again.".to_string()
            }
            Self::InvalidClient => {
                "WorkOS client configuration is invalid. Try: Check WORKOS_CLIENT_ID and WORKOS_DOMAIN.".to_string()
            }
            Self::InvalidRequest => {
                "Invalid authentication request. Try: Check WorkOS configuration.".to_string()
            }
            Self::NetworkError(msg) => {
                format!("Network error: {msg}. Try: Check your internet connection.")
            }
            Self::StorageError(msg) => {
                format!("Token storage error: {msg}. Try: Check file permissions.")
            }
            Self::ConfigurationError(msg) => {
                format!("Configuration error: {msg}. Try: Set WORKOS_CLIENT_ID and WORKOS_DOMAIN.")
            }
            Self::Unexpected(msg) => {
                format!("Unexpected error: {msg}. Try: Run `sce login` again or check logs.")
            }
        }
    }
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.user_guidance())
    }
}

impl std::error::Error for AuthError {}

impl From<TokenErrorResponse> for AuthError {
    fn from(response: TokenErrorResponse) -> Self {
        match response.error.as_str() {
            "authorization_pending" => Self::AuthorizationPending,
            "slow_down" => Self::SlowDown,
            "access_denied" => Self::AccessDenied,
            "expired_token" => Self::ExpiredToken,
            "invalid_grant" => Self::InvalidGrant,
            "invalid_client" => Self::InvalidClient,
            "invalid_request" => Self::InvalidRequest,
            _ => Self::Unexpected(format!(
                "{}: {}",
                response.error,
                response.error_description.unwrap_or_default()
            )),
        }
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_code_request_serializes_correctly() {
        let req = DeviceCodeRequest {
            client_id: "client_123".to_string(),
            scope: Some("openid profile".to_string()),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"client_id\":\"client_123\""));
        assert!(json.contains("\"scope\":\"openid profile\""));
    }

    #[test]
    fn device_code_request_serializes_without_optional_scope() {
        let req = DeviceCodeRequest {
            client_id: "client_123".to_string(),
            scope: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"client_id\":\"client_123\""));
        assert!(!json.contains("scope"));
    }

    #[test]
    fn device_code_response_deserializes_correctly() {
        let json = r#"{
            "device_code": "device_abc",
            "user_code": "ABCD-EFGH",
            "verification_url": "https://example.authkit.app/device",
            "verification_url_complete": "https://example.authkit.app/device?code=ABCD-EFGH",
            "expires_in": 600,
            "interval": 5
        }"#;
        let resp: DeviceCodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.device_code, "device_abc");
        assert_eq!(resp.user_code, "ABCD-EFGH");
        assert_eq!(resp.expires_in, 600);
        assert_eq!(resp.interval, 5);
        assert!(resp.verification_url_complete.is_some());
    }

    #[test]
    fn token_request_serializes_correctly() {
        let req = TokenRequest {
            client_id: "client_123".to_string(),
            device_code: "device_abc".to_string(),
            grant_type: GRANT_TYPE_DEVICE_CODE,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"grant_type\":\"urn:ietf:params:oauth:grant-type:device_code\""));
    }

    #[test]
    fn refresh_token_request_serializes_correctly() {
        let req = RefreshTokenRequest {
            client_id: "client_123".to_string(),
            refresh_token: "refresh_xyz".to_string(),
            grant_type: GRANT_TYPE_REFRESH_TOKEN,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"grant_type\":\"refresh_token\""));
        assert!(json.contains("\"refresh_token\":\"refresh_xyz\""));
    }

    #[test]
    fn token_response_deserializes_correctly() {
        let json = r#"{
            "access_token": "access_123",
            "refresh_token": "refresh_456",
            "token_type": "Bearer",
            "expires_in": 3600,
            "id_token": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.test",
            "scope": "openid profile"
        }"#;
        let resp: TokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.access_token, "access_123");
        assert_eq!(resp.refresh_token, Some("refresh_456".to_string()));
        assert_eq!(resp.token_type, "Bearer");
        assert_eq!(resp.expires_in, 3600);
        assert!(resp.id_token.is_some());
    }

    #[test]
    fn token_error_response_deserializes_correctly() {
        let json = r#"{
            "error": "authorization_pending",
            "error_description": "User has not completed authentication"
        }"#;
        let resp: TokenErrorResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.error, "authorization_pending");
        assert_eq!(
            resp.error_description,
            Some("User has not completed authentication".to_string())
        );
    }

    #[test]
    fn stored_tokens_serializes_and_deserializes() {
        let tokens = StoredTokens {
            access_token: "access_123".to_string(),
            refresh_token: "refresh_456".to_string(),
            expires_at: 1234567890,
            id_token: Some("id_token_here".to_string()),
            scope: Some("openid".to_string()),
        };
        let json = serde_json::to_string(&tokens).unwrap();
        let deserialized: StoredTokens = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.access_token, tokens.access_token);
        assert_eq!(deserialized.refresh_token, tokens.refresh_token);
        assert_eq!(deserialized.expires_at, tokens.expires_at);
    }

    #[test]
    fn auth_error_from_token_error_response() {
        let pending = TokenErrorResponse {
            error: "authorization_pending".to_string(),
            error_description: None,
        };
        assert_eq!(AuthError::from(pending), AuthError::AuthorizationPending);

        let slow_down = TokenErrorResponse {
            error: "slow_down".to_string(),
            error_description: None,
        };
        assert_eq!(AuthError::from(slow_down), AuthError::SlowDown);

        let unknown = TokenErrorResponse {
            error: "unknown_error".to_string(),
            error_description: Some("Something went wrong".to_string()),
        };
        let auth_err = AuthError::from(unknown);
        assert!(matches!(auth_err, AuthError::Unexpected(_)));
    }

    #[test]
    fn workos_config_verification_url() {
        let config = WorkOSConfig::new("client_123".to_string(), "my-app".to_string());
        assert_eq!(
            config.verification_url(),
            "https://my-app.authkit.app/device"
        );
    }

    #[test]
    fn auth_error_user_guidance_includes_try() {
        let errors = [
            AuthError::AccessDenied,
            AuthError::ExpiredToken,
            AuthError::InvalidGrant,
            AuthError::InvalidClient,
            AuthError::InvalidRequest,
            AuthError::NetworkError("timeout".to_string()),
            AuthError::StorageError("permission denied".to_string()),
            AuthError::ConfigurationError("missing client_id".to_string()),
            AuthError::Unexpected("unknown".to_string()),
        ];

        for error in errors {
            let guidance = error.user_guidance();
            assert!(
                guidance.contains("Try:"),
                "Error {:?} guidance should contain 'Try:'",
                error
            );
        }
    }

    #[test]
    fn auth_error_authorization_pending_guidance_is_informative() {
        let error = AuthError::AuthorizationPending;
        let guidance = error.user_guidance();
        // Should NOT contain "Try:" since this is expected behavior during polling
        assert!(!guidance.contains("Try:"));
        assert!(guidance.contains("Waiting"));
    }

    #[test]
    fn auth_error_slow_down_guidance_is_informative() {
        let error = AuthError::SlowDown;
        let guidance = error.user_guidance();
        // Should NOT contain "Try:" since this is auto-handled
        assert!(!guidance.contains("Try:"));
        assert!(guidance.contains("interval"));
    }
}
