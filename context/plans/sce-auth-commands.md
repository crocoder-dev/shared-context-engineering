# Plan: SCE Auth Commands

## Change Summary
Add `sce auth` command group with nested `login`, `logout`, and `status` subcommands to the CLI. The underlying WorkOS Device Authorization Flow is already implemented in `cli/src/services/auth.rs`; this plan wires up the command surface.

## Success Criteria
- [ ] `sce auth login` initiates WorkOS Device Authorization Flow and stores tokens
- [ ] `sce auth logout` clears stored credentials
- [ ] `sce auth status` shows current authentication state (authenticated/unauthenticated, expiry info)
- [ ] All auth commands support `--format text|json` output
- [ ] Error messages include actionable "Try:" guidance
- [ ] Help text updated to include auth commands
- [ ] Unit tests cover command parsing and dispatch

## Constraints and Non-Goals

**In Scope:**
- Add `Auth` command with `Login`, `Logout`, `Status` subcommands to clap schema
- Create thin `auth_command.rs` service for command orchestration
- Wire auth commands into app.rs dispatch
- Support `--format` flag for all auth commands
- Update help text and command surface registry

**Out of Scope:**
- Changes to existing `auth.rs` core service (already implemented)
- Changes to `token_storage.rs` (already implemented)
- Authentication guard on `sync` command (separate task)
- WorkOS configuration support (separate task)
- Browser auto-open functionality

**Non-Goals:**
- User info extraction from ID tokens
- Multi-session management
- Token revocation with WorkOS API

## Assumptions
- Existing `auth.rs` service with `start_device_auth_flow()` and `ensure_valid_token()` works correctly
- Existing `token_storage.rs` with `load_tokens()` and `save_tokens()` works correctly
- `reqwest::Client` can be created in command context
- WorkOS client ID is available via `WORKOS_CLIENT_ID` env var or config (validation happens in auth service)
- Command follows existing patterns: clap for parsing, service modules, output format support

## Task Stack

- [x] T01: Add Auth command to clap schema (status:done)
  - Task ID: T01
  - Goal: Add `Auth` command with `Login`, `Logout`, `Status` subcommands to `cli_schema.rs`
  - Boundaries (in/out of scope):
    - IN: Add `Auth` variant to `Commands` enum in `cli_schema.rs`
    - IN: Create `AuthSubcommand` enum with `Login`, `Logout`, `Status` variants
    - IN: `Login` has optional `--format` flag (default text)
    - IN: `Logout` has optional `--format` flag (default text)
    - IN: `Status` has optional `--format` flag (default text)
    - IN: Add unit tests for parsing `sce auth login`, `sce auth logout`, `sce auth status`
    - OUT: Service implementation, dispatch logic, actual auth flow
  - Done when:
    - `AuthSubcommand` enum exists with `Login`, `Logout`, `Status` variants
    - `Commands::Auth` variant exists with `subcommand: AuthSubcommand` field
    - Unit tests pass for all three subcommands with and without `--format json`
    - `cargo test --manifest-path cli/Cargo.toml --lib cli_schema` passes
  - Verification notes:
    - Run `cargo test --manifest-path cli/Cargo.toml --lib cli_schema`
    - Verify `Cli::try_parse_from(["sce", "auth", "login"])` returns `Commands::Auth`

- [ ] T02: Create auth command service module (status:todo)
  - Task ID: T02
  - Goal: Create `cli/src/services/auth_command.rs` for auth command orchestration
  - Boundaries (in/out of scope):
    - IN: Create `auth_command.rs` module with `AuthSubcommand`, `AuthRequest` types
    - IN: Implement `run_auth_subcommand()` function that dispatches to login/logout/status
    - IN: Implement `run_login()` using existing `auth::start_device_auth_flow()`
    - IN: Implement `run_logout()` using existing `token_storage::delete_tokens()` (or equivalent)
    - IN: Implement `run_status()` using existing `token_storage::load_tokens()` and expiry calculation
    - IN: Support text and JSON output formats for all commands
    - IN: Include "Try:" guidance in error messages
    - OUT: Changes to `auth.rs` core service, token encryption, network retry logic
  - Done when:
    - `cli/src/services/auth_command.rs` exists with all three command handlers
    - `run_auth_subcommand()` correctly dispatches based on subcommand type
    - Text output is human-readable with clear status messages
    - JSON output includes structured status fields
    - Error messages include actionable guidance
    - `cargo test --manifest-path cli/Cargo.toml --lib auth_command` passes
  - Verification notes:
    - Run `cargo test --manifest-path cli/Cargo.toml --lib auth_command`
    - Verify login initiates device flow and displays user code/URL
    - Verify logout removes token file gracefully
    - Verify status shows correct state with expiry calculation

- [ ] T03: Wire auth commands into app dispatch (status:todo)
  - Task ID: T03
  - Goal: Add auth command conversion and dispatch to `app.rs`
  - Boundaries (in/out of scope):
    - IN: Add `Auth` variant to internal `Command` enum in `app.rs`
    - IN: Implement `convert_auth_subcommand()` function
    - IN: Add `Command::Auth` case to `dispatch()` function
    - IN: Add `Command::Auth` case to `Command::name()` function
    - IN: Add unit tests for parsing and routing auth commands
    - OUT: Changes to clap schema, auth service logic, output format handling
  - Done when:
    - `parse_command(["sce", "auth", "login"])` returns `Command::Auth`
    - `dispatch(&Command::Auth(...))` calls `auth_command::run_auth_subcommand()`
    - Exit code contract maintained (success=0, runtime=4, etc.)
    - All existing tests still pass
    - `cargo test --manifest-path cli/Cargo.toml --lib app` passes
  - Verification notes:
    - Run `cargo test --manifest-path cli/Cargo.toml --lib app`
    - Run `sce auth status` and verify correct output
    - Run `sce auth login --help` and verify usage displayed

- [ ] T04: Add logout token deletion function (status:todo)
  - Task ID: T04
  - Goal: Add `delete_tokens()` function to `token_storage.rs` for logout
  - Boundaries (in/out of scope):
    - IN: Add `delete_tokens()` function to `token_storage.rs`
    - IN: Handle case where token file doesn't exist (graceful no-op)
    - IN: Return appropriate result type indicating deleted vs not_found
    - IN: Add unit tests for delete_tokens with existing and missing files
    - OUT: Token revocation via WorkOS API, encryption, cross-platform differences
  - Done when:
    - `delete_tokens()` function exists and removes token file
    - Returns `Ok(true)` if file was deleted, `Ok(false)` if not found
    - Returns `Err` only on actual I/O errors
    - Unit tests cover all scenarios
    - `cargo test --manifest-path cli/Cargo.toml --lib token_storage` passes
  - Verification notes:
    - Run `cargo test --manifest-path cli/Cargo.toml --lib token_storage`
    - Manual test: `sce auth login` then `sce auth logout` then verify file removed

- [ ] T05: Update command surface and help text (status:todo)
  - Task ID: T05
  - Goal: Add auth commands to command surface registry and help text
  - Boundaries (in/out of scope):
    - IN: Add `auth` entry to `COMMANDS` array in `command_surface.rs`
    - IN: Update `help_text()` to include auth command usage examples
    - IN: Add `auth` to `is_known_command()` validation
    - IN: Set status as `Implemented` (not Placeholder)
    - OUT: Changes to auth service logic, error handling, output formats
  - Done when:
    - `sce --help` shows auth command in command list
    - `command_surface::is_known_command("auth")` returns true
    - Help text includes auth command examples
    - `cargo test --manifest-path cli/Cargo.toml --lib command_surface` passes
  - Verification notes:
    - Run `sce --help` and verify auth is listed
    - Run `cargo test --manifest-path cli/Cargo.toml --lib command_surface`

- [ ] T06: Validation, testing, and context sync (status:todo)
  - Task ID: T06
  - Goal: Final validation, comprehensive testing, and context synchronization
  - Boundaries (in/out of scope):
    - IN: Run `nix flake check` to verify no regressions
    - IN: Run `nix run .#pkl-check-generated` for generated output parity
    - IN: Run all cargo tests: `cargo test --manifest-path cli/Cargo.toml`
    - IN: Manual end-to-end test: `sce auth status` (unauthenticated) -> `sce auth login` -> `sce auth status` (authenticated) -> `sce auth logout` -> `sce auth status` (unauthenticated)
    - IN: Update `context/cli/placeholder-foundation.md` with auth command status
    - IN: Update `context/overview.md` with auth feature mention
    - OUT: Performance testing, load testing, security audit, sync command auth guard
  - Done when:
    - All automated checks pass
    - Manual auth flow test completed successfully
    - Context files updated to reflect auth commands
    - No regressions in existing commands
  - Verification notes:
    - Run `nix flake check`
    - Run `nix run .#pkl-check-generated`
    - Run `cargo test --manifest-path cli/Cargo.toml --all`
    - Complete manual flow: `sce auth status` -> `sce auth login` -> `sce auth status --format json` -> `sce auth logout` -> `sce auth status`

## Open Questions
None - requirements clarified with user (nested auth command structure confirmed).

## Dependencies
- **Internal**: Existing `cli/src/services/auth.rs`, `cli/src/services/token_storage.rs`
- **External**: WorkOS API (`https://api.workos.com`)
- **Runtime**: WorkOS client ID via `WORKOS_CLIENT_ID` env var or config

## Risk Mitigation
- **Risk**: Token file permissions on different platforms
  - **Mitigation**: Reuse existing `token_storage.rs` which already handles cross-platform paths
- **Risk**: Async runtime for auth polling
  - **Mitigation**: Use existing tokio runtime pattern from `sync.rs` and `local_db.rs`
- **Risk**: User confusion with device flow
  - **Mitigation**: Clear terminal output with user code and verification URL prominently displayed

## Implementation Notes
- Follow existing clap subcommand pattern from `ConfigSubcommand` and `HooksSubcommand`
- Reuse `output_format.rs` for text/JSON formatting
- Maintain exit code contract from `app.rs` (runtime failures use code 4)
- Keep auth_command.rs thin - delegate to existing auth.rs service
- Use `reqwest::Client::new()` for HTTP client (no custom configuration needed)
- Match error message style from existing services (include "Try:" guidance)
