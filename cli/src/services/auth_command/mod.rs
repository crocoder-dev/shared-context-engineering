pub mod command;

use std::io::Write;
use std::sync::OnceLock;

use anyhow::{anyhow, Context, Result};
use serde_json::json;

use crate::services::agent_trace_sync::control_plane::{
    AuthenticatedControlPlaneClient, ControlPlaneError, MeResponse,
};
use crate::services::auth::{self, AuthError, DeviceAuthFlowResult};
use crate::services::config;
use crate::services::error::{CliError, UserError};
use crate::services::output_format::OutputFormat;
use crate::services::style::{label, prompt_label, prompt_value, success, value};
use crate::services::token_storage::{self, StoredTokens};

pub const NAME: &str = "auth";

pub type AuthFormat = OutputFormat;

static AUTH_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthSubcommand {
    Login { format: AuthFormat },
    Logout { format: AuthFormat },
    Whoami { format: AuthFormat },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthRequest {
    pub subcommand: AuthSubcommand,
}

pub fn run_auth_subcommand(request: AuthRequest) -> Result<String, CliError> {
    run_auth_subcommand_with(request, run_login, run_logout, run_whoami)
}

fn run_auth_subcommand_with<L, O, S>(
    request: AuthRequest,
    login: L,
    logout: O,
    whoami: S,
) -> Result<String, CliError>
where
    L: FnOnce(AuthFormat) -> Result<String, CliError>,
    O: FnOnce(AuthFormat) -> Result<String, CliError>,
    S: FnOnce(AuthFormat) -> Result<String, CliError>,
{
    match request.subcommand {
        AuthSubcommand::Login { format } => login(format),
        AuthSubcommand::Logout { format } => logout(format),
        AuthSubcommand::Whoami { format } => whoami(format),
    }
}

pub fn run_login(format: AuthFormat) -> Result<String, CliError> {
    let client = reqwest::Client::new();
    let runtime = shared_runtime().map_err(unexpected_auth_command_error)?;

    let client_id = resolve_login_client_id().map_err(unexpected_auth_command_error)?;
    let stored_tokens = token_storage::load_tokens().map_err(auth_storage_error)?;

    run_login_with_stored_credentials(
        format,
        stored_tokens,
        |stored_tokens| maybe_renew_stored_credentials(runtime, &client, &client_id, stored_tokens),
        |format| match format {
            AuthFormat::Text => run_text_login_with_runtime(runtime, &client, &client_id),
            AuthFormat::Json => run_login_json(runtime, &client, &client_id, format),
        },
    )
}

pub fn run_logout(format: AuthFormat) -> Result<String, CliError> {
    let deleted = token_storage::delete_tokens().map_err(auth_storage_error)?;
    render_logout_result(deleted, format).map_err(unexpected_auth_command_error)
}

pub fn run_whoami(format: AuthFormat) -> Result<String, CliError> {
    if token_storage::load_tokens()
        .map_err(auth_storage_error)?
        .is_none()
    {
        return render_unauthenticated_whoami(format).map_err(unexpected_auth_command_error);
    }

    let cwd = std::env::current_dir()
        .context("failed to determine current directory for auth config resolution")
        .map_err(unexpected_auth_command_error)?;
    let auth_config =
        config::resolve_auth_runtime_config(&cwd).map_err(unexpected_auth_command_error)?;
    let client = AuthenticatedControlPlaneClient::new(
        reqwest::Client::new(),
        auth_config.control_plane_base_url.value.unwrap_or_default(),
        auth::WORKOS_DEFAULT_BASE_URL,
        auth_config.workos_client_id.value.unwrap_or_default(),
    );
    let profile = shared_runtime()
        .map_err(unexpected_auth_command_error)?
        .block_on(client.me())
        .map_err(|error| map_whoami_control_plane_error(&error))?;

    render_whoami_result(&profile, format).map_err(unexpected_auth_command_error)
}

fn shared_runtime() -> Result<&'static tokio::runtime::Runtime> {
    if let Some(runtime) = AUTH_RUNTIME.get() {
        return Ok(runtime);
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .context("failed to create auth command runtime. Try: rerun the command; if the issue persists, verify the local Tokio runtime environment.")?;

    Ok(AUTH_RUNTIME.get_or_init(|| runtime))
}

fn maybe_renew_stored_credentials(
    runtime: &tokio::runtime::Runtime,
    client: &reqwest::Client,
    client_id: &str,
    stored_tokens: &StoredTokens,
) -> Result<Option<StoredTokens>, CliError> {
    match runtime.block_on(auth::ensure_valid_token_returning_token(
        client,
        auth::WORKOS_DEFAULT_BASE_URL,
        client_id,
        stored_tokens,
    )) {
        Ok(token) => token_storage::save_tokens(&token)
            .map(Some)
            .map_err(auth_storage_error),
        Err(_) => Ok(None),
    }
}

fn run_login_with_stored_credentials<R, D>(
    format: AuthFormat,
    stored_tokens: Option<StoredTokens>,
    renew: R,
    device_login: D,
) -> Result<String, CliError>
where
    R: FnOnce(&StoredTokens) -> Result<Option<StoredTokens>, CliError>,
    D: FnOnce(AuthFormat) -> Result<String, CliError>,
{
    if let Some(stored_tokens) = stored_tokens {
        if let Some(renewed_tokens) = renew(&stored_tokens)? {
            return render_login_refresh_result(&renewed_tokens, format)
                .map_err(unexpected_auth_command_error);
        }
    }

    device_login(format)
}

fn run_text_login_with_runtime(
    runtime: &tokio::runtime::Runtime,
    client: &reqwest::Client,
    client_id: &str,
) -> Result<String, CliError> {
    let authorization = runtime
        .block_on(auth::request_device_authorization(
            client,
            auth::WORKOS_DEFAULT_BASE_URL,
            client_id,
        ))
        .map_err(map_login_error)?;

    write_login_prompt(&authorization).map_err(unexpected_auth_command_error)?;

    let token = runtime
        .block_on(auth::complete_device_auth_flow_returning_token(
            client,
            auth::WORKOS_DEFAULT_BASE_URL,
            client_id,
            &authorization,
        ))
        .map_err(map_login_error)?;

    let stored_tokens = token_storage::save_tokens(&token).map_err(auth_storage_error)?;

    render_login_result(
        &DeviceAuthFlowResult {
            authorization,
            stored_tokens,
        },
        AuthFormat::Text,
    )
    .map_err(unexpected_auth_command_error)
}

fn run_login_json(
    runtime: &tokio::runtime::Runtime,
    client: &reqwest::Client,
    client_id: &str,
    format: AuthFormat,
) -> Result<String, CliError> {
    let authorization = runtime
        .block_on(auth::request_device_authorization(
            client,
            auth::WORKOS_DEFAULT_BASE_URL,
            client_id,
        ))
        .map_err(map_login_error)?;

    let token = runtime
        .block_on(auth::complete_device_auth_flow_returning_token(
            client,
            auth::WORKOS_DEFAULT_BASE_URL,
            client_id,
            &authorization,
        ))
        .map_err(map_login_error)?;

    let stored_tokens = token_storage::save_tokens(&token).map_err(auth_storage_error)?;

    render_login_result(
        &DeviceAuthFlowResult {
            authorization,
            stored_tokens,
        },
        format,
    )
    .map_err(unexpected_auth_command_error)
}

fn resolve_login_client_id() -> Result<String> {
    let cwd = std::env::current_dir()
        .context("failed to determine current directory for auth config resolution")?;

    Ok(config::resolve_auth_runtime_config(&cwd)?
        .workos_client_id
        .value
        .unwrap_or_default())
}

fn write_login_prompt(authorization: &auth::DeviceAuthorizationResponse) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    let browser_url = authorization
        .verification_uri_complete
        .as_deref()
        .unwrap_or(&authorization.verification_uri);
    writeln!(
        stdout,
        "{} {}",
        prompt_label("Open in browser:"),
        prompt_value(browser_url)
    )
    .context("failed to write auth verification URL to stdout")?;
    writeln!(
        stdout,
        "{} {}",
        prompt_label("Code:"),
        prompt_value(&authorization.user_code)
    )
    .context("failed to write auth user code to stdout")?;
    writeln!(stdout, "{}", value("Waiting for browser confirmation..."))
        .context("failed to write auth progress message to stdout")?;
    stdout
        .flush()
        .context("failed to flush auth prompt to stdout")?;
    Ok(())
}

fn map_login_error(error: AuthError) -> CliError {
    let user_error = match &error {
        AuthError::Io(_) | AuthError::Storage(_) => UserError::AuthStorageUnavailable,
        _ => UserError::UnexpectedFailure,
    };
    CliError::user_with_source(user_error, error)
}

fn render_login_result(result: &DeviceAuthFlowResult, format: AuthFormat) -> Result<String> {
    let expires_at_unix_seconds = result
        .stored_tokens
        .stored_at_unix_seconds
        .saturating_add(result.stored_tokens.expires_in);

    match format {
        AuthFormat::Text => Ok(success("✓ Authentication succeeded.")),
        AuthFormat::Json => serde_json::to_string_pretty(&json!({
            "status": "ok",
            "command": NAME,
            "subcommand": "login",
            "authenticated": true,
            "user_code": result.authorization.user_code,
            "verification_uri": result.authorization.verification_uri,
            "verification_uri_complete": result.authorization.verification_uri_complete,
            "token_type": result.stored_tokens.token_type,
            "scope": result.stored_tokens.scope,
            "stored_at_unix_seconds": result.stored_tokens.stored_at_unix_seconds,
            "expires_in_seconds": result.stored_tokens.expires_in,
            "expires_at_unix_seconds": expires_at_unix_seconds,
        }))
        .context("failed to serialize auth login report to JSON. Try: rerun 'sce auth login --format json'."),
    }
}

fn render_login_refresh_result(tokens: &StoredTokens, format: AuthFormat) -> Result<String> {
    let expires_at_unix_seconds = tokens
        .stored_at_unix_seconds
        .saturating_add(tokens.expires_in);

    match format {
        AuthFormat::Text => Ok(success("✓ Authentication succeeded.")),
        AuthFormat::Json => serde_json::to_string_pretty(&json!({
            "status": "ok",
            "command": NAME,
            "subcommand": "login",
            "authenticated": true,
            "renewed": true,
            "token_type": tokens.token_type,
            "scope": tokens.scope,
            "stored_at_unix_seconds": tokens.stored_at_unix_seconds,
            "expires_in_seconds": tokens.expires_in,
            "expires_at_unix_seconds": expires_at_unix_seconds,
        }))
        .context("failed to serialize auth login renewal report to JSON. Try: rerun 'sce auth login --format json'."),
    }
}

fn render_logout_result(deleted: bool, format: AuthFormat) -> Result<String> {
    match format {
        AuthFormat::Text => Ok(if deleted {
            success("Logged out")
        } else {
            value("No user logged in")
        }),
        AuthFormat::Json => serde_json::to_string_pretty(&json!({
            "status": "ok",
            "command": NAME,
            "subcommand": "logout",
            "authenticated": false,
            "credentials_removed": deleted,
        }))
        .context("failed to serialize auth logout report to JSON. Try: rerun 'sce auth logout --format json'."),
    }
}

fn render_unauthenticated_whoami(format: AuthFormat) -> Result<String> {
    match format {
        AuthFormat::Text => Ok(format!(
            "You are not logged in. Please log in using the {} command.",
            success("sce auth login")
        )),
        AuthFormat::Json => serde_json::to_string_pretty(&json!({
            "status": "ok",
            "command": NAME,
            "subcommand": "whoami",
            "authentication_state": "unauthenticated",
            "has_stored_credentials": false,
        }))
        .context("failed to serialize auth whoami report to JSON. Try: rerun 'sce auth whoami --format json'."),
    }
}

fn render_whoami_result(profile: &MeResponse, format: AuthFormat) -> Result<String> {
    match format {
        AuthFormat::Text => {
            let permissions = if profile.authorization.permissions.is_empty() {
                String::from("none")
            } else {
                profile.authorization.permissions.join(", ")
            };

            Ok(format!(
            "{} {}\n{} {}\n{} {}\n{} {}\n{} {}\n{} {}",
            label("Email:"),
            value(&profile.user.email),
            label("First Name:"),
            value(profile.user.first_name.as_deref().unwrap_or("")),
            label("Last Name:"),
            value(profile.user.last_name.as_deref().unwrap_or("")),
            label("Role:"),
            value(profile.authorization.role.as_deref().unwrap_or("none")),
            label("Permissions:"),
            value(&permissions),
            label("Organization Name:"),
            value(
                profile
                    .workspace
                    .as_ref()
                    .map_or("none", |workspace| workspace.name.as_str()),
            ),
            ))
        }
        AuthFormat::Json => serde_json::to_string_pretty(&json!({
            "status": "ok",
            "command": NAME,
            "subcommand": "whoami",
            "user": {
                "email": profile.user.email,
                "first_name": profile.user.first_name,
                "last_name": profile.user.last_name,
            },
            "authorization": {
                "role": profile.authorization.role,
                "permissions": profile.authorization.permissions,
            },
            "workspace": profile.workspace.as_ref().map(|workspace| json!({
                "name": workspace.name,
            })),
        }))
        .context("failed to serialize auth whoami report to JSON. Try: rerun 'sce auth whoami --format json'."),
    }
}

fn map_whoami_control_plane_error(error: &ControlPlaneError) -> CliError {
    let user_error = if error.is_authentication_failure() {
        UserError::NotAuthenticated
    } else if error.is_storage_failure() {
        UserError::AuthStorageUnavailable
    } else {
        UserError::UnexpectedFailure
    };

    CliError::user_with_source(
        user_error,
        anyhow!("failed to fetch authenticated user information from the Control Plane: {error}"),
    )
}

fn auth_storage_error(error: crate::services::token_storage::TokenStorageError) -> CliError {
    CliError::user_with_source(UserError::AuthStorageUnavailable, error)
}

fn unexpected_auth_command_error(error: anyhow::Error) -> CliError {
    CliError::user_with_source(UserError::UnexpectedFailure, error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logout_text_reports_whether_credentials_were_removed() {
        assert_eq!(
            render_logout_result(false, AuthFormat::Text).expect("logout should render"),
            "No user logged in"
        );
        assert_eq!(
            render_logout_result(true, AuthFormat::Text).expect("logout should render"),
            "Logged out"
        );
    }

    #[test]
    fn logout_json_reports_whether_credentials_were_removed() {
        let absent: serde_json::Value = serde_json::from_str(
            &render_logout_result(false, AuthFormat::Json).expect("logout should render"),
        )
        .expect("logout JSON should be valid");
        let present: serde_json::Value = serde_json::from_str(
            &render_logout_result(true, AuthFormat::Json).expect("logout should render"),
        )
        .expect("logout JSON should be valid");

        assert_eq!(absent["status"], "ok");
        assert_eq!(absent["authenticated"], false);
        assert_eq!(absent["credentials_removed"], false);
        assert_eq!(present["credentials_removed"], true);
    }

    #[test]
    fn unauthenticated_whoami_renders_text_guidance() {
        assert_eq!(
            render_unauthenticated_whoami(AuthFormat::Text)
                .expect("unauthenticated whoami should render"),
            "You are not logged in. Please log in using the sce auth login command."
        );
    }

    #[test]
    fn unauthenticated_whoami_json_reports_state() {
        let report: serde_json::Value = serde_json::from_str(
            &render_unauthenticated_whoami(AuthFormat::Json)
                .expect("unauthenticated whoami should render"),
        )
        .expect("whoami JSON should be valid");

        assert_eq!(report["status"], "ok");
        assert_eq!(report["command"], "auth");
        assert_eq!(report["subcommand"], "whoami");
        assert_eq!(report["authentication_state"], "unauthenticated");
        assert_eq!(report["has_stored_credentials"], false);
    }

    #[test]
    fn authenticated_whoami_failures_keep_typed_errors_and_sources() {
        let cases = [
            (
                ControlPlaneError::AuthenticationFailed("expired".to_string()),
                UserError::NotAuthenticated,
            ),
            (
                ControlPlaneError::Storage("database unavailable".to_string()),
                UserError::AuthStorageUnavailable,
            ),
            (
                ControlPlaneError::Transport("connection refused".to_string()),
                UserError::UnexpectedFailure,
            ),
        ];

        for (control_plane_error, expected_user_error) in cases {
            let mapped = map_whoami_control_plane_error(&control_plane_error);
            match mapped {
                CliError::User {
                    error,
                    source: Some(source),
                } => {
                    assert_eq!(error, expected_user_error);
                    assert!(!source.to_string().is_empty());
                }
                _ => panic!("authenticated whoami failure lost its typed source"),
            }
        }
    }
}
