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
use crate::services::style::{label, prompt_label, prompt_value, value};
use crate::services::telemetry_credential_storage;
use crate::services::token_storage::{self, StoredTokens, TokenStorageError};

pub const NAME: &str = "auth";

pub type AuthFormat = OutputFormat;

const TELEMETRY_PROVISIONING_WARNING: &str =
    "Telemetry credential provisioning was unavailable. Retry with `sce auth login`.";
const TELEMETRY_PROVISIONING_SUCCESS: &str = "Telemetry credential provisioned.";
const TELEMETRY_HEADERS_UNAVAILABLE: &str = "Telemetry credential is unavailable.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TelemetryProvisioningResult {
    Issued,
    Unavailable,
}

impl TelemetryProvisioningResult {
    fn status(self) -> &'static str {
        match self {
            Self::Issued => "issued",
            Self::Unavailable => "unavailable",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Issued => TELEMETRY_PROVISIONING_SUCCESS,
            Self::Unavailable => TELEMETRY_PROVISIONING_WARNING,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthSubcommand {
    Login { format: AuthFormat },
    Logout { format: AuthFormat },
    Whoami { format: AuthFormat },
    TelemetryHeaders,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthRequest {
    pub subcommand: AuthSubcommand,
}

pub async fn run_auth_subcommand(request: AuthRequest) -> Result<String, CliError> {
    run_auth_subcommand_with(
        request,
        run_login,
        run_logout,
        run_whoami,
        run_telemetry_headers,
    )
    .await
}

async fn run_auth_subcommand_with<L, O, S, H, LF, OF, SF, HF>(
    request: AuthRequest,
    login: L,
    logout: O,
    whoami: S,
    telemetry_headers: H,
) -> Result<String, CliError>
where
    H: FnOnce() -> HF,
    HF: Future<Output = Result<String, CliError>>,
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
        AuthSubcommand::TelemetryHeaders => telemetry_headers().await,
    }
}

async fn run_telemetry_headers() -> Result<String, CliError> {
    render_telemetry_headers(telemetry_credential_storage::load_telemetry_credential().await)
}

fn render_telemetry_headers(
    loaded: Result<Option<telemetry_credential_storage::TelemetryCredential>, TokenStorageError>,
) -> Result<String, CliError> {
    let Ok(Some(credential)) = loaded else {
        return Err(telemetry_headers_unavailable());
    };

    serde_json::to_string(&json!({
        "Authorization": format!("Bearer {}", credential.token),
    }))
    .map_err(|_| telemetry_headers_unavailable())
}

fn telemetry_headers_unavailable() -> CliError {
    CliError::runtime(anyhow!(TELEMETRY_HEADERS_UNAVAILABLE))
}

pub async fn run_login(format: AuthFormat) -> Result<String, CliError> {
    let client = reqwest::Client::new();
    let cwd = std::env::current_dir()
        .context("failed to determine current directory for auth config resolution")
        .map_err(unexpected_auth_command_error)?;
    let auth_config =
        config::resolve_auth_runtime_config(&cwd).map_err(unexpected_auth_command_error)?;
    let client_id = auth_config.workos_client_id.value.unwrap_or_default();
    let control_plane = AuthenticatedControlPlaneClient::new(
        client.clone(),
        auth_config.control_plane_base_url.value.unwrap_or_default(),
        auth::WORKOS_DEFAULT_BASE_URL,
        client_id.clone(),
    );
    let stored_tokens = token_storage::load_tokens()
        .await
        .map_err(auth_storage_error)?;

    let client = &client;
    let control_plane = &control_plane;
    let client_id = client_id.as_str();
    run_login_with_stored_credentials(
        format,
        stored_tokens,
        |stored_tokens| async move {
            maybe_renew_stored_credentials(client, client_id, stored_tokens).await
        },
        || async move { provision_telemetry_credential(control_plane).await },
        |format| async move {
            match format {
                AuthFormat::Text => run_text_login(client, client_id, control_plane).await,
                AuthFormat::Json => run_login_json(client, client_id, format, control_plane).await,
            }
        },
    )
    .await
}

pub async fn run_logout(format: AuthFormat) -> Result<String, CliError> {
    run_logout_with(
        format,
        token_storage::delete_tokens(),
        telemetry_credential_storage::delete_telemetry_credential(),
    )
    .await
}

async fn run_logout_with<W, T>(
    format: AuthFormat,
    delete_workos: W,
    delete_telemetry: T,
) -> Result<String, CliError>
where
    W: Future<Output = Result<bool, TokenStorageError>>,
    T: Future<Output = Result<bool, TokenStorageError>>,
{
    let workos_result = delete_workos.await;
    let telemetry_result = delete_telemetry.await;
    let workos_deleted = workos_result.map_err(auth_storage_error)?;
    let telemetry_deleted = telemetry_result.map_err(auth_storage_error)?;
    render_logout_result(workos_deleted || telemetry_deleted, format)
        .map_err(unexpected_auth_command_error)
}

pub async fn run_whoami(format: AuthFormat) -> Result<String, CliError> {
    if token_storage::load_tokens()
        .await
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
        Ok(token) => token_storage::save_tokens(&token)
            .await
            .map_err(auth_storage_error)
            .map(Some),
        Err(_) => Ok(None),
    }
}

async fn run_login_with_stored_credentials<R, P, D, RF, PF, DF>(
    format: AuthFormat,
    stored_tokens: Option<StoredTokens>,
    renew: R,
    provision: P,
    device_login: D,
) -> Result<String, CliError>
where
    R: FnOnce(StoredTokens) -> RF,
    RF: Future<Output = Result<Option<StoredTokens>, CliError>>,
    P: FnOnce() -> PF,
    PF: Future<Output = TelemetryProvisioningResult>,
    D: FnOnce(AuthFormat) -> DF,
    DF: Future<Output = Result<String, CliError>>,
{
    if let Some(stored_tokens) = stored_tokens {
        if let Some(renewed_tokens) = renew(stored_tokens).await? {
            let telemetry = provision().await;
            return render_login_refresh_result_with_telemetry(&renewed_tokens, format, telemetry)
                .map_err(unexpected_auth_command_error);
        }
    }

    device_login(format).await
}

async fn provision_telemetry_credential(
    client: &AuthenticatedControlPlaneClient,
) -> TelemetryProvisioningResult {
    match client.issue_and_save_telemetry_credential().await {
        Ok(_) => TelemetryProvisioningResult::Issued,
        Err(_) => TelemetryProvisioningResult::Unavailable,
    }
}

async fn run_text_login(
    client: &reqwest::Client,
    client_id: &str,
    control_plane: &AuthenticatedControlPlaneClient,
) -> Result<String, CliError> {
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

    let stored_tokens = token_storage::save_tokens(&token)
        .await
        .map_err(auth_storage_error)?;

    let telemetry = provision_telemetry_credential(control_plane).await;
    render_login_result_with_telemetry(
        &DeviceAuthFlowResult {
            authorization,
            stored_tokens,
        },
        AuthFormat::Text,
        telemetry,
    )
    .map_err(unexpected_auth_command_error)
}

async fn run_login_json(
    client: &reqwest::Client,
    client_id: &str,
    format: AuthFormat,
    control_plane: &AuthenticatedControlPlaneClient,
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

    let stored_tokens = token_storage::save_tokens(&token)
        .await
        .map_err(auth_storage_error)?;

    let telemetry = provision_telemetry_credential(control_plane).await;
    render_login_result_with_telemetry(
        &DeviceAuthFlowResult {
            authorization,
            stored_tokens,
        },
        format,
        telemetry,
    )
    .map_err(unexpected_auth_command_error)
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

pub(crate) fn render_login_result_with_telemetry(
    result: &DeviceAuthFlowResult,
    format: AuthFormat,
    telemetry: TelemetryProvisioningResult,
) -> Result<String> {
    let expires_at_unix_seconds = result
        .stored_tokens
        .stored_at_unix_seconds
        .saturating_add(result.stored_tokens.expires_in);

    match format {
        AuthFormat::Text => Ok(render_login_text(telemetry)),
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
            "telemetry_provisioning": telemetry_json(telemetry),
        }))
        .context("failed to serialize auth login report to JSON. Try: rerun 'sce auth login --format json'."),
    }
}

pub(crate) fn render_login_refresh_result_with_telemetry(
    tokens: &StoredTokens,
    format: AuthFormat,
    telemetry: TelemetryProvisioningResult,
) -> Result<String> {
    let expires_at_unix_seconds = tokens
        .stored_at_unix_seconds
        .saturating_add(tokens.expires_in);

    match format {
        AuthFormat::Text => Ok(render_login_text(telemetry)),
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
            "telemetry_provisioning": telemetry_json(telemetry),
        }))
        .context("failed to serialize auth login renewal report to JSON. Try: rerun 'sce auth login --format json'."),
    }
}

fn render_login_text(telemetry: TelemetryProvisioningResult) -> String {
    render_login_text_with_color_policy(telemetry, crate::services::style::supports_color())
}

fn render_login_text_with_color_policy(
    telemetry: TelemetryProvisioningResult,
    color_enabled: bool,
) -> String {
    let headline = crate::services::style::success_with_color_policy(
        "✓ Authentication succeeded.",
        color_enabled,
    );
    match telemetry {
        TelemetryProvisioningResult::Issued => headline,
        TelemetryProvisioningResult::Unavailable => {
            format!("{}\nWarning: {}", headline, telemetry.message())
        }
    }
}

fn telemetry_json(telemetry: TelemetryProvisioningResult) -> serde_json::Value {
    json!({
        "status": telemetry.status(),
        "message": telemetry.message(),
    })
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

fn auth_storage_error(error: TokenStorageError) -> CliError {
    CliError::user_with_source(UserError::AuthStorageUnavailable, error)
}

fn unexpected_auth_command_error(error: anyhow::Error) -> CliError {
    CliError::user_with_source(UserError::UnexpectedFailure, error)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    fn storage_failure() -> TokenStorageError {
        TokenStorageError::Database("failed".to_string())
    }

    fn telemetry_credential(token: &str) -> telemetry_credential_storage::TelemetryCredential {
        telemetry_credential_storage::TelemetryCredential {
            token: token.to_string(),
            accessor: "fixture-accessor".to_string(),
            expires_at_unix_seconds: 4_000_000_000,
        }
    }

    #[test]
    fn telemetry_headers_render_only_authorization_bearer_and_follow_replacement() {
        let first = render_telemetry_headers(Ok(Some(telemetry_credential("tok-one"))))
            .expect("valid credential should render");
        assert_eq!(first, r#"{"Authorization":"Bearer tok-one"}"#);
        assert!(!first.contains("fixture-accessor"));

        let second = render_telemetry_headers(Ok(Some(telemetry_credential("tok-two"))))
            .expect("replacement should render");
        assert_eq!(second, r#"{"Authorization":"Bearer tok-two"}"#);
    }

    #[test]
    fn telemetry_headers_failures_use_fixed_secret_free_diagnostic() {
        let missing = render_telemetry_headers(Ok(None)).expect_err("missing should fail");
        let storage =
            render_telemetry_headers(Err(TokenStorageError::Database("tok-secret".to_string())))
                .expect_err("storage failure should fail");

        for error in [missing, storage] {
            assert_eq!(error.class(), crate::services::error::FailureClass::Runtime);
            assert_eq!(error.to_string(), TELEMETRY_HEADERS_UNAVAILABLE);
        }
    }

    fn device_auth_result() -> DeviceAuthFlowResult {
        DeviceAuthFlowResult {
            authorization: auth::DeviceAuthorizationResponse {
                device_code: "device-code".to_string(),
                user_code: "ABCD-EFGH".to_string(),
                verification_uri: "https://example.test/verify".to_string(),
                verification_uri_complete: None,
                expires_in: 600,
                interval: Some(1),
            },
            stored_tokens: StoredTokens {
                access_token: "workos-access".to_string(),
                token_type: "Bearer".to_string(),
                expires_in: 3600,
                refresh_token: "workos-refresh".to_string(),
                scope: None,
                stored_at_unix_seconds: 100,
            },
        }
    }

    fn stored_tokens() -> StoredTokens {
        device_auth_result().stored_tokens
    }

    #[tokio::test]
    async fn valid_stored_credentials_are_provisioned_after_workos_persistence() {
        let provision_calls = AtomicBool::new(false);
        let tokens = stored_tokens();
        let output = run_login_with_stored_credentials(
            AuthFormat::Json,
            Some(tokens.clone()),
            |stored| async move {
                assert_eq!(stored, tokens);
                Ok(Some(stored))
            },
            || async {
                provision_calls.store(true, Ordering::SeqCst);
                TelemetryProvisioningResult::Issued
            },
            |_format| async { panic!("valid stored credentials must not use device login") },
        )
        .await
        .expect("valid stored login should succeed");

        assert!(provision_calls.load(Ordering::SeqCst));
        assert!(output.contains("\"renewed\": true"));
        assert!(output.contains("\"status\": \"issued\""));
    }

    #[tokio::test]
    async fn renewed_stored_credentials_are_provisioned_after_workos_persistence() {
        let provision_calls = AtomicBool::new(false);
        let original = stored_tokens();
        let renewed = StoredTokens {
            access_token: "renewed-access".to_string(),
            refresh_token: "renewed-refresh".to_string(),
            ..original.clone()
        };
        let output = run_login_with_stored_credentials(
            AuthFormat::Json,
            Some(original),
            |_stored| async move { Ok(Some(renewed)) },
            || async {
                provision_calls.store(true, Ordering::SeqCst);
                TelemetryProvisioningResult::Issued
            },
            |_format| async { panic!("renewed stored credentials must not use device login") },
        )
        .await
        .expect("renewed stored login should succeed");

        assert!(provision_calls.load(Ordering::SeqCst));
        assert!(output.contains("\"renewed\": true"));
        assert!(output.contains("\"status\": \"issued\""));
    }

    #[tokio::test]
    async fn missing_stored_credentials_use_device_login_result() {
        let output = run_login_with_stored_credentials(
            AuthFormat::Text,
            None,
            |_stored| async { unreachable!("there are no stored credentials") },
            || async { unreachable!("device login owns provisioning") },
            |_format| async { Ok("device-login-complete".to_string()) },
        )
        .await
        .expect("device login should succeed");

        assert_eq!(output, "device-login-complete");
    }

    #[test]
    fn login_text_reports_telemetry_provisioning_success() {
        let output =
            render_login_text_with_color_policy(TelemetryProvisioningResult::Issued, false);

        assert_eq!(output, "✓ Authentication succeeded.");
        assert!(!output.contains(TELEMETRY_PROVISIONING_WARNING));
    }

    #[test]
    fn login_text_reports_secret_free_telemetry_warning() {
        let output = render_login_result_with_telemetry(
            &device_auth_result(),
            AuthFormat::Text,
            TelemetryProvisioningResult::Unavailable,
        )
        .expect("text login output should render");

        assert!(output.contains("Authentication succeeded."));
        assert!(output.contains(TELEMETRY_PROVISIONING_WARNING));
        assert!(!output.contains("workos-access"));
        assert!(!output.contains("workos-refresh"));
    }

    #[test]
    fn login_json_reports_telemetry_provisioning_status_and_message() {
        let issued = render_login_result_with_telemetry(
            &device_auth_result(),
            AuthFormat::Json,
            TelemetryProvisioningResult::Issued,
        )
        .expect("issued JSON login output should render");
        let unavailable = render_login_refresh_result_with_telemetry(
            &stored_tokens(),
            AuthFormat::Json,
            TelemetryProvisioningResult::Unavailable,
        )
        .expect("unavailable JSON login output should render");

        assert!(issued.contains("\"status\": \"issued\""));
        assert!(issued.contains(TELEMETRY_PROVISIONING_SUCCESS));
        assert!(unavailable.contains("\"status\": \"unavailable\""));
        assert!(unavailable.contains(TELEMETRY_PROVISIONING_WARNING));
        assert!(!unavailable.contains("workos-access"));
        assert!(!unavailable.contains("workos-refresh"));
    }

    #[tokio::test]
    async fn telemetry_credential_logout_reports_removal_when_only_telemetry_deleted() {
        let output = run_logout_with(AuthFormat::Json, async { Ok(false) }, async { Ok(true) })
            .await
            .expect("logout should succeed");
        assert!(output.contains("\"credentials_removed\": true"));
    }

    #[tokio::test]
    async fn telemetry_credential_logout_attempts_both_deletions_on_telemetry_failure() {
        let workos_called = AtomicBool::new(false);
        let result = run_logout_with(
            AuthFormat::Json,
            async {
                workos_called.store(true, Ordering::SeqCst);
                Ok(true)
            },
            async { Err(storage_failure()) },
        )
        .await;
        assert!(result.is_err());
        assert!(workos_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn telemetry_credential_logout_deletes_telemetry_on_workos_failure() {
        let telemetry_called = AtomicBool::new(false);
        let result = run_logout_with(AuthFormat::Json, async { Err(storage_failure()) }, async {
            telemetry_called.store(true, Ordering::SeqCst);
            Ok(true)
        })
        .await;
        assert!(result.is_err());
        assert!(telemetry_called.load(Ordering::SeqCst));
    }
}
