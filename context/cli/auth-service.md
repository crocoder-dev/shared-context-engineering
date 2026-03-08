# WorkOS Authentication Service

The `cli/src/services/auth.rs` module provides type definitions for WorkOS SSO/OIDC authentication via OAuth 2.0 Device Authorization Flow (RFC 8628).

## Current implementation status

**T01 skeleton complete:** Type definitions and error handling for Device Authorization Flow.

**Planned (T02-T10):**
- Cross-platform token storage (T02)
- Device Authorization Flow HTTP implementation (T03)
- Token refresh logic (T04)
- `login` command (T05)
- `logout` command (T06)
- `auth status` command (T07)
- Authentication guard on `sync` command (T08)
- WorkOS configuration support (T09)
- Documentation updates (T10)

## Type definitions

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
- Constants: `WORKOS_API_BASE_URL`, `WORKOS_AUTHKIT_DEVICE_URL_TEMPLATE`

### Error Handling

`AuthError` enum covers all authentication failure modes with actionable user guidance:
- `AuthorizationPending`: User has not completed authentication
- `SlowDown`: Polling too frequently
- `AccessDenied`: User denied authorization
- `ExpiredToken`: Device code expired
- `InvalidGrant`, `InvalidClient`, `InvalidRequest`: Configuration/request errors
- `NetworkError`, `StorageError`, `ConfigurationError`, `Unexpected`: Runtime errors

## Token storage paths (planned)

- Linux: `${XDG_STATE_HOME:-~/.local/state}/sce/auth/tokens.json`
- macOS: `~/Library/Application Support/sce/auth/tokens.json`
- Windows: `%APPDATA%\sce\auth\tokens.json`

## Dependencies

- `reqwest`: Async HTTP client for WorkOS API calls
- `serde`/`serde_json`: JSON serialization for API requests/responses
- `dirs`: Cross-platform state directory resolution

## Related context

- Plan: `context/plans/workos-cli-auth.md`
- CLI foundation: `context/cli/placeholder-foundation.md`
