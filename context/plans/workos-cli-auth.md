# Plan: WorkOS CLI Authentication

## Change Summary
Add WorkOS SSO/OIDC authentication to the existing `sce` CLI using the OAuth 2.0 Device Authorization Flow (RFC 8628). Users must authenticate via WorkOS before using the `sync` command. This includes adding a `login` command, token management, and authentication guards.

## Success Criteria
- [ ] `sce login` command initiates WorkOS Device Authorization Flow
- [ ] Users can authenticate by visiting verification URL and entering user code
- [ ] Access tokens and refresh tokens are securely stored locally
- [ ] Token refresh works automatically when access tokens expire
- [ ] `sce sync` command requires authentication and fails gracefully when unauthenticated
- [ ] `sce logout` command clears stored credentials
- [ ] `sce auth status` command shows current authentication state
- [ ] All authentication flows handle errors with actionable user guidance
- [ ] Documentation updated in `cli/README.md`

## Constraints and Non-Goals
**In Scope:**
- Device Authorization Flow (OAuth 2.0 RFC 8628) implementation
- Local secure token storage (file-based with restricted permissions)
- Token refresh logic
- Authentication guards on `sync` command
- Login/logout/status commands
- Configuration of WorkOS client ID and domain via environment or config file

**Out of Scope:**
- Web-based authentication (only CLI device flow)
- Multi-tenant or organization selection (single WorkOS app)
- Token encryption at rest (relying on filesystem permissions for MVP)
- Browser auto-open (user manually visits URL)
- SSO provider selection (WorkOS handles this)
- Migration from any existing auth system (none exists)

**Non-Goals:**
- Replacing WorkOS SDK with custom implementation (using direct HTTP calls is acceptable for MVP)
- Supporting other OAuth flows (Authorization Code, etc.)
- Enterprise SSO configuration UI
- Token revocation on logout (best-effort only)

## Assumptions
- WorkOS application is already configured with Device Authorization Flow enabled
- Client ID and AuthKit domain will be provided via environment variables (`WORKOS_CLIENT_ID`, `WORKOS_DOMAIN`) or config file
- Default WorkOS API base URL: `https://api.workos.com`
- Default AuthKit verification URL pattern: `https://{domain}.authkit.app/device`
- Token storage locations (cross-platform):
  - Linux: `${XDG_STATE_HOME:-~/.local/state}/sce/auth/tokens.json`
  - macOS: `~/Library/Application Support/sce/auth/tokens.json`
  - Windows: `%APPDATA%\sce\auth\tokens.json`
- Access token lifetime: 3600 seconds (1 hour) - will be read from WorkOS response
- Minimum polling interval: 5 seconds (will use WorkOS-provided interval)

## Task Stack

- [x] T01: Add HTTP client dependency and auth service skeleton (status:done)
  - Task ID: T01
  - Goal: Add `reqwest`, `serde`, and `dirs` dependencies, create `auth` service module with type definitions
  - Boundaries (in/out of scope):
    - IN: Add dependencies to `Cargo.toml`, create `cli/src/services/auth.rs` with types for Device Code and Token responses
    - IN: Add `dirs` crate for cross-platform state directory resolution
    - IN: Define error types specific to auth failures
    - OUT: Actual HTTP calls, token storage, command integration
  - Done when:
    - `reqwest` with `json` feature added to `Cargo.toml`
    - `serde` and `serde_json` features configured
    - `dirs` crate added for cross-platform paths
    - `cli/src/services/auth.rs` exists with type definitions matching WorkOS API
    - Module compiles without errors
    - Dependency contract updated in `cli/src/dependency_contract.rs`
  - Verification notes:
    - Run `cargo check --manifest-path cli/Cargo.toml`
    - Verify `reqwest` and `dirs` appear in `Cargo.toml` dependencies
    - Verify auth types serialize/deserialize correctly in unit tests

- [x] T02: Implement cross-platform token storage service (status:done)
  - Task ID: T02
  - Goal: Create secure file-based token storage with proper permissions across Linux, macOS, and Windows
  - Boundaries (in/out of scope):
    - IN: Create `cli/src/services/token_storage.rs` module
    - IN: Implement token save/load with platform-appropriate security:
      - Linux/macOS: 600 file permissions (owner read/write only)
      - Windows: Remove inherited permissions, grant only to current user
    - IN: Use `dirs` crate to resolve platform-appropriate state directory:
      - Linux: `dirs::state_dir()` or fallback to `~/.local/state`
      - macOS: `dirs::data_dir()` (resolves to `~/Library/Application Support`)
      - Windows: `dirs::data_dir()` (resolves to `%APPDATA%`)
    - IN: Handle missing/invalid/corrupted token files gracefully
    - IN: Ensure parent directory creation with appropriate permissions
    - OUT: Token encryption, keychain/credential manager integration, cross-machine sync
  - Done when:
    - `token_storage.rs` exists with `save_tokens()` and `load_tokens()` functions
    - Tokens stored as JSON with platform-appropriate restricted permissions
    - Works correctly on Linux, macOS, and Windows
    - Unit tests cover save/load/error scenarios
    - Module compiles and integrates with auth service
  - Verification notes:
    - Run `cargo test --manifest-path cli/Cargo.toml --lib token_storage`
    - Linux/macOS: Manually inspect created token file permissions (should be 0600)
    - Windows: Verify file ACL restricts access to current user only
    - Test with missing/invalid token files on all platforms
    - Verify correct state directory resolution on each platform

- [x] T03: Implement Device Authorization Flow (status:done)
  - Task ID: T03
  - Goal: Implement complete OAuth 2.0 Device Authorization Flow with polling
  - Boundaries (in/out of scope):
    - IN: POST to `/user_management/authorize/device` to get device code
    - IN: Display user code and verification URL to user
    - IN: Poll `/user_management/authenticate` with exponential backoff
    - IN: Handle `authorization_pending`, `slow_down`, `access_denied`, `expired_token` errors
    - IN: Store tokens on successful authentication
    - OUT: Browser auto-open, QR code display, WebSocket-based callbacks
  - Done when:
    - `auth.rs` has `start_device_auth_flow()` function
    - Device code request returns proper user_code and verification URLs
    - Token polling works with WorkOS-specified interval
    - All error cases handled with actionable messages
    - Integration test can complete flow (requires manual WorkOS app setup)
  - Verification notes:
    - Run `cargo test --manifest-path cli/Cargo.toml --lib auth`
    - Manual test with real WorkOS credentials (document in test plan)
    - Verify error messages include "Try:" guidance

- [x] T04: Implement token refresh logic (status:done)
  - Task ID: T04
  - Goal: Automatically refresh expired access tokens using refresh tokens
  - Boundaries (in/out of scope):
    - IN: Check token expiry before use
    - IN: Use refresh token to get new access token
    - IN: Update stored tokens after refresh
    - IN: Handle refresh token expiration (require re-login)
    - OUT: Proactive background refresh, token rotation callbacks
  - Done when:
    - `auth.rs` has `ensure_valid_token()` function
    - Expired access tokens are automatically refreshed
    - Refresh failures require re-authentication
    - Unit tests cover expiry checking and refresh scenarios
  - Verification notes:
    - Run `cargo test --manifest-path cli/Cargo.toml --lib auth::refresh`
    - Test with manually expired tokens
    - Verify new tokens are persisted

- [ ] T05: Add `login` command to CLI (status:todo)
  - Task ID: T05
  - Goal: Add `sce login` command that initiates authentication flow
  - Boundaries (in/out of scope):
    - IN: Add `login` to command surface in `cli/src/command_surface.rs`
    - IN: Add `cli/src/services/login.rs` with command parsing and dispatch
    - IN: Wire login command to auth service device flow
    - IN: Display user-friendly instructions with verification URL and code
    - IN: Show success message with user info from ID token
    - OUT: Non-interactive login, session selection, organization switching
  - Done when:
    - `sce login` command registered in command surface
    - Command displays device code and verification URL
    - User can complete authentication in browser
    - Success message shows authenticated user email/name
    - Handles errors with actionable guidance
    - Help text updated
  - Verification notes:
    - Run `sce login --help` shows usage
    - Run `sce login` and complete flow manually
    - Verify token file created after successful login
    - Test error scenarios (network failure, user denial)

- [ ] T06: Add `logout` command to CLI (status:todo)
  - Task ID: T06
  - Goal: Add `sce logout` command that clears stored credentials
  - Boundaries (in/out of scope):
    - IN: Add `logout` to command surface
    - IN: Create `cli/src/services/logout.rs` module
    - IN: Delete token file from storage
    - IN: Show success message
    - OUT: Token revocation with WorkOS API, multi-session management
  - Done when:
    - `sce logout` command registered and working
    - Token file deleted on logout
    - Success message displayed
    - Handles already-logged-out state gracefully
    - Help text updated
  - Verification notes:
    - Run `sce logout` after login, verify token file removed
    - Run `sce logout` when already logged out, verify clean exit
    - Run `sce login --help`

- [ ] T07: Add `auth status` command to CLI (status:todo)
  - Task ID: T07
  - Goal: Add `sce auth status` command that shows authentication state
  - Boundaries (in/out of scope):
    - IN: Add `auth` subcommand with `status` sub-subcommand (or `sce auth-status` as top-level)
    - IN: Check if tokens exist and are valid
    - IN: Display user info from ID token (email, name, org)
    - IN: Show token expiry time
    - IN: Support `--format json` output
    - OUT: Session switching, multi-account support
  - Done when:
    - `sce auth status` command works (or equivalent)
    - Shows authenticated/unauthenticated state
    - Displays user email and name when authenticated
    - Shows token expiry in human-readable format
    - JSON output includes all fields
    - Help text updated
  - Verification notes:
    - Run `sce auth status` when unauthenticated
    - Run `sce auth status` when authenticated
    - Run `sce auth status --format json` and verify JSON schema
    - Run `sce auth status --help`

- [ ] T08: Add authentication guard to `sync` command (status:todo)
  - Task ID: T08
  - Goal: Require valid authentication before allowing `sync` command execution
  - Boundaries (in/out of scope):
    - IN: Check authentication status in `sync` command before execution
    - IN: Attempt token refresh if expired
    - IN: Fail with actionable error if unauthenticated
    - IN: Include "Run `sce login` first" guidance
    - OUT: Fine-grained permission checks, role-based access
  - Done when:
    - `sce sync` fails gracefully when not logged in
    - Error message includes "Run `sce login`" guidance
    - `sce sync` works after successful login
    - Expired tokens are auto-refreshed
    - Sync placeholder still returns placeholder message when authenticated
  - Verification notes:
    - Run `sce sync` without login, verify error
    - Run `sce login`, then `sce sync`, verify success
    - Wait for token expiry, run `sce sync`, verify auto-refresh

- [ ] T09: Add WorkOS configuration support (status:todo)
  - Task ID: T09
  - Goal: Support WorkOS client ID and domain configuration via environment and config file
  - Boundaries (in/out of scope):
    - IN: Add `workos_client_id` and `workos_domain` to config schema
    - IN: Support `WORKOS_CLIENT_ID` and `WORKOS_DOMAIN` environment variables
    - IN: Add to config precedence: flags > env > config file > defaults
    - IN: Update `sce config show` to display WorkOS settings
    - IN: Validate WorkOS config is present when auth commands run
    - OUT: Interactive WorkOS setup wizard, multi-environment config
  - Done when:
    - Config service supports `workos_client_id` and `workos_domain`
    - Environment variables override config file values
    - `sce config show` displays WorkOS settings (redacted)
    - Auth commands fail with actionable error if config missing
    - Config validation checks WorkOS settings
  - Verification notes:
    - Run `sce config show` with WorkOS env vars set
    - Run `sce login` without WorkOS config, verify error
    - Test config precedence (env overrides file)

- [ ] T10: Update CLI documentation and help text (status:todo)
  - Task ID: T10
  - Goal: Document WorkOS authentication in `cli/README.md` and update all help text
  - Boundaries (in/out of scope):
    - IN: Add "Authentication" section to `cli/README.md`
    - IN: Document `login`, `logout`, `auth status` commands
    - IN: Document required WorkOS configuration
    - IN: Update main help text to mention auth commands
    - IN: Add authentication troubleshooting section
    - OUT: WorkOS setup guide (assumes WorkOS app already configured)
  - Done when:
    - `cli/README.md` has complete auth documentation
    - All auth commands documented with examples
    - Configuration instructions clear
    - Main help text lists auth commands
    - Common issues and solutions documented
  - Verification notes:
    - Read `cli/README.md` for completeness
    - Run `sce --help` and verify auth commands mentioned
    - Run `sce login --help` and verify useful guidance

- [ ] T11: Validation, testing, and context sync (status:todo)
  - Task ID: T11
  - Goal: Final validation, comprehensive testing, and context synchronization
  - Boundaries (in/out of scope):
    - IN: Run full `nix flake check` to verify no regressions
    - IN: Run `nix run .#pkl-check-generated` for generated output parity
    - IN: Run all cargo tests: `cargo test --manifest-path cli/Cargo.toml`
    - IN: Manual end-to-end test of complete auth flow
    - IN: Update `context/cli/placeholder-foundation.md` with auth status
    - IN: Update `context/overview.md` with auth feature summary
    - IN: Update `context/glossary.md` with auth-related terms
    - OUT: Performance testing, load testing, security audit
  - Done when:
    - All automated checks pass (`nix flake check`, `pkl-check-generated`, cargo tests)
    - Manual auth flow test completed successfully
    - Context files updated to reflect current auth state
    - No regressions in existing commands
    - Documentation is accurate and complete
  - Verification notes:
    - Run `nix flake check` and verify success
    - Run `nix run .#pkl-check-generated`
    - Run `cargo test --manifest-path cli/Cargo.toml --all`
    - Complete manual auth flow: `sce login` → `sce auth status` → `sce sync` → `sce logout`
    - Verify context files updated correctly

## Open Questions
None - all requirements clarified with user.

## Dependencies
- **External**: WorkOS API (`https://api.workos.com`)
- **Runtime**: Requires WorkOS client ID and domain configuration
- **Build**: `reqwest` crate with `json` feature, `serde` derive macros, `dirs` crate for cross-platform paths

## Risk Mitigation
- **Risk**: Token storage security
  - **Mitigation**: Use restrictive file permissions (0600 on Unix, user-only ACL on Windows), document security assumptions, recommend OS keychain for production
- **Risk**: Cross-platform path resolution differences
  - **Mitigation**: Use well-tested `dirs` crate, add platform-specific tests, document expected paths per platform
- **Risk**: Network failures during auth flow
  - **Mitigation**: Use existing resilience wrapper from `cli/src/services/resilience.rs` for retries
- **Risk**: Token expiry during long-running operations
  - **Mitigation**: Implement `ensure_valid_token()` check before each authenticated operation
- **Risk**: WorkOS API changes
  - **Mitigation**: Use stable OAuth 2.0 standard endpoints, version pin API in docs

## Implementation Notes
- Follow existing CLI patterns: lexopt for parsing, anyhow for errors, services/ module structure
- Reuse `cli/src/services/resilience.rs` for HTTP retry logic
- Follow `cli/src/services/output_format.rs` for dual text/JSON output
- Maintain exit code contract from `cli/src/app.rs`
- Keep auth service focused on WorkOS only (no abstraction for other providers)
- Use `dirs` crate for cross-platform state directory resolution (Linux, macOS, Windows)
- Platform-specific file security: Unix permissions (0600) vs Windows ACLs
