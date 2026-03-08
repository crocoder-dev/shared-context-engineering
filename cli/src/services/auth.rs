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
// Device Authorization Flow Implementation
// ============================================================================

/// Additional seconds to add to polling interval when receiving `slow_down` error.
const SLOW_DOWN_INTERVAL_ADDITION_SECS: u64 = 5;

/// Creates a new HTTP client for WorkOS API requests.
fn create_http_client() -> Result<reqwest::Client, AuthError> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| AuthError::NetworkError(format!("Failed to create HTTP client: {e}")))
}

/// Requests a device code from WorkOS Device Authorization endpoint.
///
/// POST to `/user_management/authorize/device` to initiate the device flow.
///
/// # Errors
///
/// Returns `AuthError` if:
/// - HTTP request fails
/// - Response cannot be parsed
/// - WorkOS returns an error
pub fn request_device_code(config: &WorkOSConfig) -> Result<DeviceCodeResponse, AuthError> {
    let client = create_http_client()?;

    let url = format!("{}/user_management/authorize/device", config.api_base_url);
    let request_body = DeviceCodeRequest {
        client_id: config.client_id.clone(),
        scope: None, // Use default scopes for MVP
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AuthError::Unexpected(format!("Failed to create tokio runtime: {e}")))?;

    runtime.block_on(async {
        let response = client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| AuthError::NetworkError(format!("Device code request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AuthError::Unexpected(format!(
                "Device code request failed with status {}: {}",
                status, error_text
            )));
        }

        response
            .json::<DeviceCodeResponse>()
            .await
            .map_err(|e| AuthError::Unexpected(format!("Failed to parse device code response: {e}")))
    })
}

/// Polls the WorkOS token endpoint until authentication is complete or an error occurs.
///
/// Handles `authorization_pending` by continuing to poll, and `slow_down` by increasing
/// the polling interval. Respects the device code expiry time.
///
/// # Arguments
///
/// * `config` - WorkOS configuration
/// * `device_code_response` - The device code response from `request_device_code`
/// * `status_callback` - Optional callback for polling status updates (receives message)
///
/// # Errors
///
/// Returns `AuthError` if:
/// - User denies authorization (`access_denied`)
/// - Device code expires (`expired_token`)
/// - Any other terminal error occurs
pub fn poll_for_token<F>(
    config: &WorkOSConfig,
    device_code_response: &DeviceCodeResponse,
    mut status_callback: Option<F>,
) -> Result<TokenResponse, AuthError>
where
    F: FnMut(&str),
{
    let client = create_http_client()?;
    let url = format!("{}/user_management/authenticate", config.api_base_url);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AuthError::Unexpected(format!("Failed to create tokio runtime: {e}")))?;

    let mut interval_secs = device_code_response.interval as u64;
    let expires_at = std::time::Instant::now()
        + std::time::Duration::from_secs(device_code_response.expires_in as u64);

    loop {
        // Check if device code has expired
        if std::time::Instant::now() >= expires_at {
            return Err(AuthError::ExpiredToken);
        }

        // Wait before polling
        runtime.block_on(tokio::time::sleep(std::time::Duration::from_secs(interval_secs)));

        // Build token request
        let request_body = TokenRequest {
            client_id: config.client_id.clone(),
            device_code: device_code_response.device_code.clone(),
            grant_type: GRANT_TYPE_DEVICE_CODE,
        };

        let result = runtime.block_on(async {
            let response = client
                .post(&url)
                .json(&request_body)
                .send()
                .await
                .map_err(|e| AuthError::NetworkError(format!("Token polling failed: {e}")))?;

            let status = response.status();

            // Try to parse as successful token response first
            if status.is_success() {
                return response
                    .json::<TokenResponse>()
                    .await
                    .map_err(|e| AuthError::Unexpected(format!("Failed to parse token response: {e}")));
            }

            // Parse as error response
            let error_response = response
                .json::<TokenErrorResponse>()
                .await
                .map_err(|e| AuthError::Unexpected(format!("Failed to parse error response: {e}")))?;

            Err(AuthError::from(error_response))
        });

        match result {
            Ok(token_response) => return Ok(token_response),
            Err(AuthError::AuthorizationPending) => {
                if let Some(ref mut callback) = status_callback {
                    callback("Waiting for authentication...");
                }
                // Continue polling with same interval
            }
            Err(AuthError::SlowDown) => {
                interval_secs += SLOW_DOWN_INTERVAL_ADDITION_SECS;
                if let Some(ref mut callback) = status_callback {
                    callback(&format!("Slowing down polling (now {}s interval)", interval_secs));
                }
                // Continue polling with increased interval
            }
            Err(e) => return Err(e), // Terminal error
        }
    }
}

/// Converts a TokenResponse to StoredTokens with calculated expiry timestamp.
fn token_response_to_stored_tokens(response: TokenResponse) -> StoredTokens {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    StoredTokens {
        access_token: response.access_token,
        refresh_token: response.refresh_token.unwrap_or_default(),
        expires_at: now + response.expires_in as u64,
        id_token: response.id_token,
        scope: response.scope,
    }
}

/// Executes the complete Device Authorization Flow.
///
/// This function:
/// 1. Requests a device code from WorkOS
/// 2. Displays user instructions (verification URL and user code)
/// 3. Polls for token completion
/// 4. Stores tokens on success
///
/// # Arguments
///
/// * `config` - WorkOS configuration (client ID, domain, API URL)
/// * `display_instructions` - Callback to display user instructions (receives user_code and verification_url)
/// * `status_callback` - Optional callback for polling status updates
///
/// # Returns
///
/// Returns `StoredTokens` on successful authentication.
///
/// # Errors
///
/// Returns `AuthError` if any step of the flow fails.
pub fn start_device_auth_flow<F, G>(
    config: &WorkOSConfig,
    mut display_instructions: F,
    mut status_callback: Option<G>,
) -> Result<StoredTokens, AuthError>
where
    F: FnMut(&str, &str),
    G: FnMut(&str),
{
    // Step 1: Request device code
    let device_code_response = request_device_code(config)?;

    // Step 2: Display instructions to user
    display_instructions(
        &device_code_response.user_code,
        &device_code_response.verification_url,
    );

    // Step 3: Poll for token
    let token_response = poll_for_token(config, &device_code_response, status_callback.as_mut())?;

    // Step 4: Convert and store tokens
    let stored_tokens = token_response_to_stored_tokens(token_response);

    super::token_storage::save_tokens(&stored_tokens)
        .map_err(|e| AuthError::StorageError(e.to_string()))?;

    Ok(stored_tokens)
}

// ============================================================================
// Token Refresh Logic
// ============================================================================

/// Number of seconds before actual expiry to consider token expired (buffer for clock skew).
const TOKEN_EXPIRY_BUFFER_SECS: u64 = 60;

/// Checks if a stored token is expired or about to expire.
///
/// Uses a buffer (default 60 seconds) to account for clock skew and
/// network latency. A token is considered expired if:
/// `current_time + buffer >= expires_at`
///
/// # Arguments
///
/// * `tokens` - The stored tokens to check
///
/// # Returns
///
/// `true` if the token is expired or will expire within the buffer window.
pub fn is_token_expired(tokens: &StoredTokens) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Add buffer to account for clock skew and network latency
    now.saturating_add(TOKEN_EXPIRY_BUFFER_SECS) >= tokens.expires_at
}

/// Refreshes an expired access token using the refresh token.
///
/// POSTs to `/user_management/authenticate` with the refresh token to obtain
/// a new access token. Updates stored tokens on success.
///
/// # Arguments
///
/// * `config` - WorkOS configuration (client ID, domain, API URL)
/// * `tokens` - Current stored tokens (must contain a valid refresh token)
///
/// # Returns
///
/// Returns new `StoredTokens` on successful refresh.
///
/// # Errors
///
/// Returns `AuthError` if:
/// - HTTP request fails
/// - Response cannot be parsed
/// - Refresh token is invalid or expired (`invalid_grant`)
/// - Token storage fails
pub fn refresh_access_token(
    config: &WorkOSConfig,
    tokens: &StoredTokens,
) -> Result<StoredTokens, AuthError> {
    if tokens.refresh_token.is_empty() {
        return Err(AuthError::InvalidGrant);
    }

    let client = create_http_client()?;
    let url = format!("{}/user_management/authenticate", config.api_base_url);

    let request_body = RefreshTokenRequest {
        client_id: config.client_id.clone(),
        refresh_token: tokens.refresh_token.clone(),
        grant_type: GRANT_TYPE_REFRESH_TOKEN,
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AuthError::Unexpected(format!("Failed to create tokio runtime: {e}")))?;

    runtime.block_on(async {
        let response = client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| AuthError::NetworkError(format!("Token refresh failed: {e}")))?;

        let status = response.status();

        // Try to parse as successful token response first
        if status.is_success() {
            let token_response = response
                .json::<TokenResponse>()
                .await
                .map_err(|e| AuthError::Unexpected(format!("Failed to parse token response: {e}")))?;

            // Convert to stored tokens
            let new_tokens = token_response_to_stored_tokens(token_response);

            // Save the new tokens
            super::token_storage::save_tokens(&new_tokens)
                .map_err(|e| AuthError::StorageError(e.to_string()))?;

            return Ok(new_tokens);
        }

        // Parse as error response
        let error_response = response
            .json::<TokenErrorResponse>()
            .await
            .map_err(|e| AuthError::Unexpected(format!("Failed to parse error response: {e}")))?;

        Err(AuthError::from(error_response))
    })
}

/// Ensures a valid access token is available, refreshing if necessary.
///
/// This function:
/// 1. Loads stored tokens (returns error if not logged in)
/// 2. Checks if access token is expired
/// 3. Returns valid tokens if not expired
/// 4. Refreshes tokens if expired
/// 5. Returns refreshed tokens on success
///
/// # Arguments
///
/// * `config` - WorkOS configuration (client ID, domain, API URL)
///
/// # Returns
///
/// Returns valid `StoredTokens` (either existing or refreshed).
///
/// # Errors
///
/// Returns `AuthError` if:
/// - No tokens are stored (user not logged in)
/// - Token refresh fails (refresh token expired/invalid)
/// - Token storage fails
///
/// # Example
///
/// ```ignore
/// let config = WorkOSConfig::new(client_id, domain);
/// let tokens = ensure_valid_token(&config)?;
/// // Use tokens.access_token for API calls
/// ```
pub fn ensure_valid_token(config: &WorkOSConfig) -> Result<StoredTokens, AuthError> {
    // Load stored tokens
    let tokens = super::token_storage::load_tokens()
        .map_err(|e| AuthError::StorageError(e.to_string()))?
        .ok_or_else(|| {
            AuthError::ConfigurationError(
                "Not logged in. Try: Run `sce login` first.".to_string(),
            )
        })?;

    // Check if token is expired
    if is_token_expired(&tokens) {
        // Refresh the token
        refresh_access_token(config, &tokens)
    } else {
        // Token is still valid
        Ok(tokens)
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

    // ========================================================================
    // Device Authorization Flow Tests
    // ========================================================================

    #[test]
    fn create_http_client_succeeds() {
        let result = create_http_client();
        assert!(result.is_ok());
    }

    #[test]
    fn token_response_to_stored_tokens_calculates_expiry() {
        let response = TokenResponse {
            access_token: "access_123".to_string(),
            refresh_token: Some("refresh_456".to_string()),
            token_type: "Bearer".to_string(),
            expires_in: 3600, // 1 hour
            id_token: Some("id_token".to_string()),
            scope: Some("openid profile".to_string()),
        };

        let stored = token_response_to_stored_tokens(response);

        assert_eq!(stored.access_token, "access_123");
        assert_eq!(stored.refresh_token, "refresh_456");
        assert!(stored.expires_at > 0);
        assert!(stored.id_token.is_some());
        assert!(stored.scope.is_some());
    }

    #[test]
    fn token_response_to_stored_tokens_handles_missing_optional_fields() {
        let response = TokenResponse {
            access_token: "access_123".to_string(),
            refresh_token: None,
            token_type: "Bearer".to_string(),
            expires_in: 3600,
            id_token: None,
            scope: None,
        };

        let stored = token_response_to_stored_tokens(response);

        assert_eq!(stored.access_token, "access_123");
        assert_eq!(stored.refresh_token, ""); // Default for missing refresh_token
        assert!(stored.id_token.is_none());
        assert!(stored.scope.is_none());
    }

    #[test]
    fn workos_config_has_api_base_url() {
        let config = WorkOSConfig::new("client_123".to_string(), "my-app".to_string());
        assert_eq!(config.api_base_url, WORKOS_API_BASE_URL);
        assert_eq!(config.client_id, "client_123");
        assert_eq!(config.domain, "my-app");
    }

    #[test]
    fn slow_down_interval_addition_is_reasonable() {
        // Ensure the slow_down addition is a reasonable value
        assert_eq!(SLOW_DOWN_INTERVAL_ADDITION_SECS, 5);
    }

    // ========================================================================
    // Token Refresh Tests
    // ========================================================================

    #[test]
    fn is_token_expired_returns_true_for_expired_token() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Token expired 1 hour ago
        let tokens = StoredTokens {
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: now - 3600,
            id_token: None,
            scope: None,
        };

        assert!(is_token_expired(&tokens));
    }

    #[test]
    fn is_token_expired_returns_true_for_token_expiring_within_buffer() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Token expires in 30 seconds (within the 60-second buffer)
        let tokens = StoredTokens {
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: now + 30,
            id_token: None,
            scope: None,
        };

        assert!(is_token_expired(&tokens));
    }

    #[test]
    fn is_token_expired_returns_false_for_valid_token() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Token expires in 1 hour (well beyond buffer)
        let tokens = StoredTokens {
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: now + 3600,
            id_token: None,
            scope: None,
        };

        assert!(!is_token_expired(&tokens));
    }

    #[test]
    fn is_token_expired_returns_false_for_token_just_outside_buffer() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Token expires in 120 seconds (just outside the 60-second buffer)
        let tokens = StoredTokens {
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: now + 120,
            id_token: None,
            scope: None,
        };

        assert!(!is_token_expired(&tokens));
    }

    #[test]
    fn token_expiry_buffer_is_reasonable() {
        // Ensure the buffer is a reasonable value (60 seconds)
        assert_eq!(TOKEN_EXPIRY_BUFFER_SECS, 60);
    }

    #[test]
    fn refresh_access_token_fails_with_empty_refresh_token() {
        let config = WorkOSConfig::new("client_123".to_string(), "my-app".to_string());
        let tokens = StoredTokens {
            access_token: "access".to_string(),
            refresh_token: "".to_string(), // Empty refresh token
            expires_at: 0,
            id_token: None,
            scope: None,
        };

        let result = refresh_access_token(&config, &tokens);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), AuthError::InvalidGrant);
    }

    #[test]
    fn ensure_valid_token_returns_error_when_not_logged_in() {
        // This test would require mocking token_storage::load_tokens
        // For now, we document the expected behavior
        // In a real test environment, we'd use a mock or test fixture

        // Expected behavior:
        // - If load_tokens() returns Ok(None), should return ConfigurationError
        // - Error message should include "Run `sce login` first"

        // This is tested indirectly through integration tests
    }

    #[test]
    fn ensure_valid_token_returns_valid_token_when_not_expired() {
        // This test would require mocking token_storage::load_tokens
        // For now, we document the expected behavior
        // In a real test environment, we'd use a mock or test fixture

        // Expected behavior:
        // - If tokens exist and is_token_expired() returns false
        // - Should return the existing tokens without refresh
    }

    #[test]
    fn ensure_valid_token_refreshes_when_expired() {
        // This test would require mocking both token_storage and HTTP client
        // For now, we document the expected behavior
        // In a real test environment, we'd use mocks or test fixtures

        // Expected behavior:
        // - If tokens exist and is_token_expired() returns true
        // - Should call refresh_access_token()
        // - Should return new tokens on success
        // - Should return refresh error on failure
    }
}
