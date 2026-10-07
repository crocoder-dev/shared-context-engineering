pub mod command;

use std::future::Future;
use std::io::Write;

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

pub async fn run_auth_subcommand(request: AuthRequest) -> Result<String, CliError> {
    run_auth_subcommand_with(request, run_login, run_logout, run_whoami).await
}

async fn run_auth_subcommand_with<L, O, S, LF, OF, SF>(
    request: AuthRequest,
    login: L,
    logout: O,
    whoami: S,
) -> Result<String, CliError>
where
    L: FnOnce(AuthFormat) -> LF,
    LF: Future<Output = Result<String, CliError>>,
    O: FnOnce(AuthFormat) -> OF,
    OF: Future<Output = Result<String, CliError>>,
    S: FnOnce(AuthFormat) -> SF,
    SF: Future<Output = Result<String, CliError>>,
{
    match request.subcommand {
        AuthSubcommand::Login { format } => login(format).await,
        AuthSubcommand::Logout { format } => logout(format).await,
        AuthSubcommand::Whoami { format } => whoami(format).await,
    }
}

pub async fn run_login(format: AuthFormat) -> Result<String, CliError> {
    let client = reqwest::Client::new();

    let client_id = resolve_login_client_id().map_err(unexpected_auth_command_error)?;
    let stored_tokens = run_credential_operation(token_storage::load_tokens).await?;

    let client = &client;
    let client_id = client_id.as_str();
    run_login_with_stored_credentials(
        format,
        stored_tokens,
        |stored_tokens| async move {
            maybe_renew_stored_credentials(client, client_id, stored_tokens).await
        },
        |format| async move {
            match format {
                AuthFormat::Text => run_text_login(client, client_id).await,
                AuthFormat::Json => run_login_json(client, client_id, format).await,
            }
        },
    )
    .await
}

pub async fn run_logout(format: AuthFormat) -> Result<String, CliError> {
    let deleted = run_credential_operation(token_storage::delete_tokens).await?;
    render_logout_result(deleted, format).map_err(unexpected_auth_command_error)
}

pub async fn run_whoami(format: AuthFormat) -> Result<String, CliError> {
    if run_credential_operation(token_storage::load_tokens)
        .await?
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
    let profile = client
        .me()
        .await
        .map_err(|error| map_whoami_control_plane_error(&error))?;

    render_whoami_result(&profile, format).map_err(unexpected_auth_command_error)
}

async fn maybe_renew_stored_credentials(
    client: &reqwest::Client,
    client_id: &str,
    stored_tokens: StoredTokens,
) -> Result<Option<StoredTokens>, CliError> {
    match auth::ensure_valid_token_returning_token(
        client,
        auth::WORKOS_DEFAULT_BASE_URL,
        client_id,
        &stored_tokens,
    )
    .await
    {
        Ok(token) => run_credential_operation(move || token_storage::save_tokens(&token))
            .await
            .map(Some),
        Err(_) => Ok(None),
    }
}

async fn run_login_with_stored_credentials<R, D, RF, DF>(
    format: AuthFormat,
    stored_tokens: Option<StoredTokens>,
    renew: R,
    device_login: D,
) -> Result<String, CliError>
where
    R: FnOnce(StoredTokens) -> RF,
    RF: Future<Output = Result<Option<StoredTokens>, CliError>>,
    D: FnOnce(AuthFormat) -> DF,
    DF: Future<Output = Result<String, CliError>>,
{
    if let Some(stored_tokens) = stored_tokens {
        if let Some(renewed_tokens) = renew(stored_tokens).await? {
            return render_login_refresh_result(&renewed_tokens, format)
                .map_err(unexpected_auth_command_error);
        }
    }

    device_login(format).await
}

async fn run_text_login(client: &reqwest::Client, client_id: &str) -> Result<String, CliError> {
    let authorization =
        auth::request_device_authorization(client, auth::WORKOS_DEFAULT_BASE_URL, client_id)
            .await
            .map_err(map_login_error)?;

    write_login_prompt(&authorization).map_err(unexpected_auth_command_error)?;

    let token = auth::complete_device_auth_flow_returning_token(
        client,
        auth::WORKOS_DEFAULT_BASE_URL,
        client_id,
        &authorization,
    )
    .await
    .map_err(map_login_error)?;

    let stored_tokens =
        run_credential_operation(move || token_storage::save_tokens(&token)).await?;

    render_login_result(
        &DeviceAuthFlowResult {
            authorization,
            stored_tokens,
        },
        AuthFormat::Text,
    )
    .map_err(unexpected_auth_command_error)
}

async fn run_login_json(
    client: &reqwest::Client,
    client_id: &str,
    format: AuthFormat,
) -> Result<String, CliError> {
    let authorization =
        auth::request_device_authorization(client, auth::WORKOS_DEFAULT_BASE_URL, client_id)
            .await
            .map_err(map_login_error)?;

    let token = auth::complete_device_auth_flow_returning_token(
        client,
        auth::WORKOS_DEFAULT_BASE_URL,
        client_id,
        &authorization,
    )
    .await
    .map_err(map_login_error)?;

    let stored_tokens =
        run_credential_operation(move || token_storage::save_tokens(&token)).await?;

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
    render_logout_result_with_color_policy(
        deleted,
        format,
        crate::services::style::supports_color(),
    )
}

fn render_logout_result_with_color_policy(
    deleted: bool,
    format: AuthFormat,
    color_enabled: bool,
) -> Result<String> {
    match format {
        AuthFormat::Text => Ok(if deleted {
            crate::services::style::success_with_color_policy("Logged out", color_enabled)
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
    render_unauthenticated_whoami_with_color_policy(
        format,
        crate::services::style::supports_color(),
    )
}

fn render_unauthenticated_whoami_with_color_policy(
    format: AuthFormat,
    color_enabled: bool,
) -> Result<String> {
    match format {
        AuthFormat::Text => Ok(format!(
            "You are not logged in. Please log in using the {} command.",
            crate::services::style::success_with_color_policy("sce auth login", color_enabled)
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

async fn run_credential_operation<T, F>(operation: F) -> Result<T, CliError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, token_storage::TokenStorageError> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| unexpected_auth_command_error(error.into()))?
        .map_err(auth_storage_error)
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

    fn stored_tokens() -> StoredTokens {
        StoredTokens {
            access_token: "secret-access".into(),
            refresh_token: "secret-refresh".into(),
            token_type: "Bearer".into(),
            scope: None,
            expires_in: 3600,
            stored_at_unix_seconds: 100,
        }
    }

    async fn completed_action(format: AuthFormat, name: &str) -> Result<String, CliError> {
        assert_eq!(format, AuthFormat::Json);
        assert_eq!(
            tokio::runtime::Handle::current().runtime_flavor(),
            tokio::runtime::RuntimeFlavor::MultiThread
        );
        tokio::task::yield_now().await;
        Ok(name.to_string())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn auth_dispatch_awaits_only_the_selected_action() {
        for (subcommand, expected) in [
            (
                AuthSubcommand::Login {
                    format: AuthFormat::Json,
                },
                "login",
            ),
            (
                AuthSubcommand::Logout {
                    format: AuthFormat::Json,
                },
                "logout",
            ),
            (
                AuthSubcommand::Whoami {
                    format: AuthFormat::Json,
                },
                "whoami",
            ),
        ] {
            let result = run_auth_subcommand_with(
                AuthRequest { subcommand },
                |format| async move {
                    assert_eq!(expected, "login");
                    completed_action(format, "login").await
                },
                |format| async move {
                    assert_eq!(expected, "logout");
                    completed_action(format, "logout").await
                },
                |format| async move {
                    assert_eq!(expected, "whoami");
                    completed_action(format, "whoami").await
                },
            )
            .await
            .expect("selected action should complete");
            assert_eq!(result, expected);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn login_helper_awaits_owned_renewal_and_redacts_credentials() {
        let report = run_login_with_stored_credentials(
            AuthFormat::Json,
            Some(stored_tokens()),
            |tokens| async move {
                completed_action(AuthFormat::Json, "renew").await?;
                Ok(Some(tokens))
            },
            |_| async { panic!("renewed credentials must skip device login") },
        )
        .await
        .expect("renewal should complete");
        assert_eq!(
            report,
            render_login_refresh_result(&stored_tokens(), AuthFormat::Json).unwrap()
        );
        assert!(!report.contains("secret-access"));
        assert!(!report.contains("secret-refresh"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn login_helper_awaits_device_fallback_after_unsuccessful_renewal() {
        let renewal_completed = std::cell::Cell::new(false);
        let renewed = &renewal_completed;
        let result = run_login_with_stored_credentials(
            AuthFormat::Json,
            Some(stored_tokens()),
            |_| async {
                completed_action(AuthFormat::Json, "renew").await?;
                renewed.set(true);
                Ok(None)
            },
            |format| async move {
                assert!(renewed.get());
                completed_action(format, "device login completed").await
            },
        )
        .await
        .expect("fallback should complete");
        assert_eq!(result, "device login completed");
    }

    fn check_blocking_credential_context() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("build credential fixture runtime");
        runtime.block_on(async {});
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn auth_credential_load_save_delete_are_isolated_and_awaited() {
        let credentials = std::sync::Arc::new(std::sync::Mutex::new(Some(stored_tokens())));
        let load_store = credentials.clone();
        let loaded = run_credential_operation(move || {
            check_blocking_credential_context();
            Ok(load_store.lock().unwrap().clone())
        })
        .await
        .expect("load should complete")
        .unwrap();
        let save_store = credentials.clone();
        let saved = run_credential_operation(move || {
            check_blocking_credential_context();
            let mut tokens = loaded;
            tokens.access_token = "saved-access".into();
            *save_store.lock().unwrap() = Some(tokens.clone());
            Ok(tokens)
        })
        .await
        .expect("save should complete");
        assert_eq!(saved.access_token, "saved-access");
        assert_eq!(
            credentials.lock().unwrap().as_ref().unwrap().access_token,
            "saved-access"
        );
        let delete_store = credentials.clone();
        let deleted = run_credential_operation(move || {
            check_blocking_credential_context();
            Ok(delete_store.lock().unwrap().take().is_some())
        })
        .await
        .expect("delete should complete");
        assert!(deleted);
        assert!(credentials.lock().unwrap().is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn auth_credential_storage_and_join_failures_keep_typed_sources() {
        let storage_error = run_credential_operation::<(), _>(|| {
            Err(token_storage::TokenStorageError::Database(
                "fixture failure".into(),
            ))
        })
        .await
        .unwrap_err();
        let join_error =
            run_credential_operation::<(), _>(|| panic!("credential worker fixture failure"))
                .await
                .unwrap_err();
        for (mapped, expected) in [
            (storage_error, UserError::AuthStorageUnavailable),
            (join_error, UserError::UnexpectedFailure),
        ] {
            match mapped {
                CliError::User {
                    error,
                    source: Some(source),
                } => {
                    assert_eq!(error, expected);
                    assert!(source.to_string().contains("fixture failure"));
                }
                _ => panic!("credential failure lost its typed source"),
            }
        }
    }

    #[test]
    fn logout_text_reports_whether_credentials_were_removed() {
        assert_eq!(
            render_logout_result_with_color_policy(false, AuthFormat::Text, false)
                .expect("logout should render"),
            "No user logged in"
        );
        assert_eq!(
            render_logout_result_with_color_policy(true, AuthFormat::Text, false)
                .expect("logout should render"),
            "Logged out"
        );
    }

    #[test]
    fn logout_json_reports_whether_credentials_were_removed() {
        let absent: serde_json::Value = serde_json::from_str(
            &render_logout_result_with_color_policy(false, AuthFormat::Json, false)
                .expect("logout should render"),
        )
        .expect("logout JSON should be valid");
        let present: serde_json::Value = serde_json::from_str(
            &render_logout_result_with_color_policy(true, AuthFormat::Json, false)
                .expect("logout should render"),
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
            render_unauthenticated_whoami_with_color_policy(AuthFormat::Text, false)
                .expect("unauthenticated whoami should render"),
            "You are not logged in. Please log in using the sce auth login command."
        );
    }

    #[test]
    fn unauthenticated_whoami_json_reports_state() {
        let report: serde_json::Value = serde_json::from_str(
            &render_unauthenticated_whoami_with_color_policy(AuthFormat::Json, false)
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
