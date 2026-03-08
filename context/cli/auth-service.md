# WorkOS Authentication Service

The `cli/src/services/auth.rs` module provides type definitions and OAuth 2.0 Device Authorization Flow (RFC 8628) implementation for WorkOS SSO/OIDC authentication.

The `cli/src/services/token_storage.rs` module provides secure cross-platform token storage.

## Current implementation status

**T01 skeleton complete:** Type definitions and error handling for Device Authorization Flow.

**T02 token storage complete:** Cross-platform secure token storage with proper file permissions.

**T03 device flow complete:** HTTP-based Device Authorization Flow with polling.

**T04 token refresh complete:** Automatic token refresh with expiry checking.

**Planned (T05-T10):**
- `login` command (T05)
- `logout` command (T06)
- `auth status` command (T07)
- Authentication guard on `sync` command (T08)
- WorkOS configuration support (T09)
- Documentation updates (T10)

## Device Authorization Flow (T03 implemented)

The `auth` module provides blocking HTTP-based device authorization:

### Public API

- `request_device_code(config: &WorkOSConfig) -> Result<DeviceCodeResponse, AuthError>`: Requests device code from WorkOS
- `poll_for_token<F>(config: &WorkOSConfig, device_code: &DeviceCodeResponse, status_callback: Option<F>) -> Result<TokenResponse, AuthError>`: Polls until authentication completes
- `start_device_auth_flow<F, G>(config: &WorkOSConfig, display_instructions: F, status_callback: Option<G>) -> Result<StoredTokens, AuthError>`: Orchestrates complete flow

### Flow behavior

1. **Request device code**: POST to `/user_management/authorize/device`
2. **Display instructions**: Callback receives `user_code` and `verification_url`
3. **Poll for token**: Polls `/user_management/authenticate` with WorkOS-specified interval
4. **Handle errors**:
   - `authorization_pending`: Continue polling
   - `slow_down`: Increase interval by 5 seconds
   - Terminal errors: Return immediately
5. **Store tokens**: Converts `TokenResponse` to `StoredTokens` and persists

### Polling behavior

- Respects `interval` from device code response
- Adds 5 seconds on `slow_down` error
- Times out based on `expires_in` from device code response
- Uses `tokio::runtime::Builder::new_current_thread()` for async HTTP

## Token Refresh (T04 implemented)

The `auth` module provides automatic token refresh with expiry checking:

### Public API

- `is_token_expired(tokens: &StoredTokens) -> bool`: Checks if access token is expired or about to expire (60-second buffer)
- `refresh_access_token(config: &WorkOSConfig, tokens: &StoredTokens) -> Result<StoredTokens, AuthError>`: Refreshes expired access token using refresh token
- `ensure_valid_token(config: &WorkOSConfig) -> Result<StoredTokens, AuthError>`: High-level API that loads tokens, checks expiry, and refreshes if needed

### Refresh behavior

1. **Load stored tokens**: Returns `ConfigurationError` if not logged in
2. **Check expiry**: Uses 60-second buffer to account for clock skew
3. **Refresh if expired**: POSTs to `/user_management/authenticate` with refresh token
4. **Update storage**: Saves new tokens after successful refresh
5. **Handle failures**: Maps `invalid_grant` errors to require re-authentication

### Expiry checking

- Uses 60-second buffer before actual expiry (defined in `TOKEN_EXPIRY_BUFFER_SECS`)
- Accounts for clock skew and network latency
- Conservative approach: refreshes early rather than late

### Error handling

- Empty refresh token: Returns `AuthError::InvalidGrant`
- Invalid/expired refresh token: Returns `AuthError::InvalidGrant` (requires re-login)
- Network failures: Returns `AuthError::NetworkError` with actionable guidance
- Storage failures: Returns `AuthError::StorageError` with actionable guidance

## Token Storage (T02 implemented)

The `token_storage` module provides secure file-based token storage:

### Public API

- `resolve_token_storage_path() -> Result<PathBuf>`: Resolves platform-appropriate token file path
- `save_tokens(tokens: &StoredTokens) -> Result<()>`: Saves tokens with secure permissions
- `load_tokens() -> Result<Option<StoredTokens>>`: Loads tokens (returns `None` if missing)
- `delete_tokens() -> Result<()>`: Deletes stored tokens (idempotent)

### Platform paths

- Linux: `${XDG_STATE_HOME:-~/.local/state}/sce/auth/tokens.json`
- macOS: `~/Library/Application Support/sce/auth/tokens.json`
- Windows: `%APPDATA%\sce\auth\tokens.json`

### File security

- Unix (Linux/macOS): 0600 file permissions (owner read/write only)
- Windows: Relies on directory-level security in user's AppData directory

### Error handling

All functions return `anyhow::Result` with actionable error messages including "Try:" guidance for user-facing errors.

## Type definitions (auth.rs)

### Device Code Flow

- `DeviceCodeRequest`: Request to initiate Device Authorization Flow
- `DeviceCodeResponse`: Response with device code, user code, verification URL
- `TokenRequest`: Polling request during device flow
- `RefreshTokenRequest`: Token refresh request

### Token Types

- `TokenResponse`: Successful token response (access token, refresh token, ID token)
- `TokenErrorResponse`: Error response with standardized error codes
- `StoredTokens`: Persistent token storage structure

### Configuration

- `WorkOSConfig`: Client configuration (client ID, domain, API base URL)
- Constants: `WORKOS_API_BASE_URL`, `WORKOS_AUTHKIT_DEVICE_URL_TEMPLATE`, `TOKEN_EXPIRY_BUFFER_SECS`

### Internal constants

- `SLOW_DOWN_INTERVAL_ADDITION_SECS`: 5 seconds added to polling interval on `slow_down` error
- `TOKEN_EXPIRY_BUFFER_SECS`: 60 seconds buffer for token expiry checking
- `GRANT_TYPE_DEVICE_CODE`: OAuth 2.0 grant type for device flow
- `GRANT_TYPE_REFRESH_TOKEN`: OAuth 2.0 grant type for token refresh

### Error Handling

`AuthError` enum covers all authentication failure modes with actionable user guidance:
- `AuthorizationPending`: User has not completed authentication
- `SlowDown`: Polling too frequently
- `AccessDenied`: User denied authorization
- `ExpiredToken`: Device code expired
- `InvalidGrant`, `InvalidClient`, `InvalidRequest`: Configuration/request errors
- `NetworkError`, `StorageError`, `ConfigurationError`, `Unexpected`: Runtime errors

## Dependencies

- `reqwest`: Async HTTP client for WorkOS API calls
- `serde`/`serde_json`: JSON serialization for API requests/responses and token storage
- `dirs`: Cross-platform state directory resolution

## Related context

- Plan: `context/plans/workos-cli-auth.md`
- CLI foundation: `context/cli/placeholder-foundation.md`
