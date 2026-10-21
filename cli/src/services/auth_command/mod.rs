pub mod command;

use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde_json::json;

use crate::services::agent_trace_sync::control_plane::{
    AuthenticatedControlPlaneClient, ControlPlaneError, MeResponse,
};
use crate::services::auth::{self, AuthError, DeviceAuthFlowResult};
use crate::services::config::{self, AuthMode, ResolvedAccessPolicy, ResolvedAuthRuntimeConfig};
use crate::services::db::encryption_key::CredentialAccess;
use crate::services::error::{AuthStorageFailure, CliError, UserError};
use crate::services::managed_telemetry::claude_helper::{
    repository_id_for, visible_destination_conflict,
};
use crate::services::managed_telemetry::{self, run_with_deadline, CREDENTIAL_LOOKUP_DEADLINE};
use crate::services::output_format::OutputFormat;
use crate::services::style::{label, prompt_label, prompt_value, value};
use crate::services::telemetry_credential_storage;
use crate::services::token_storage::{self, StoredTokens, TokenStorageError};

pub const NAME: &str = "auth";

pub type AuthFormat = OutputFormat;

const TELEMETRY_PROVISIONING_WARNING: &str =
    "Telemetry credential provisioning was unavailable. Retry with `sce auth login`.";
const TELEMETRY_PROVISIONING_SUCCESS: &str = "Telemetry credential provisioned.";
const TELEMETRY_PROVISIONING_DISABLED: &str = "Telemetry credential provisioning is disabled by SCE config. Set `telemetry.enabled: true` in the global SCE config to enable it.";
const TELEMETRY_HEADERS_UNAVAILABLE: &str = "Telemetry credential is unavailable.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TelemetryProvisioningResult {
    Issued,
    Unavailable,
    Disabled,
}

impl TelemetryProvisioningResult {
    fn status(self) -> &'static str {
        match self {
            Self::Issued => "issued",
            Self::Unavailable => "unavailable",
            Self::Disabled => "disabled",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Issued => TELEMETRY_PROVISIONING_SUCCESS,
            Self::Unavailable => TELEMETRY_PROVISIONING_WARNING,
            Self::Disabled => TELEMETRY_PROVISIONING_DISABLED,
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
    let working_dir = std::env::current_dir().ok();
    let telemetry_enabled = working_dir.as_deref().is_some_and(telemetry_enabled_for);
    telemetry_headers_with(
        working_dir,
        telemetry_enabled,
        visible_destination_conflict(|key| std::env::var(key).ok()),
        managed_telemetry::auth_db_present,
        repository_id_for,
        telemetry_credential_storage::load_repository_telemetry_credential,
    )
    .await
}

fn telemetry_enabled_for(working_dir: &Path) -> bool {
    config::resolve_auth_runtime_config(working_dir)
        .is_ok_and(|config| config.access_policy.telemetry_enabled())
}

pub(crate) async fn telemetry_headers_with<P, R, L, Fut>(
    working_dir: Option<PathBuf>,
    telemetry_enabled: bool,
    destination_conflict: bool,
    auth_db_probe: P,
    resolve_repository: R,
    lookup: L,
) -> Result<String, CliError>
where
    P: FnOnce() -> bool,
    R: FnOnce(&Path) -> Option<String> + Send + 'static,
    L: FnOnce(String) -> Fut + Send + 'static,
    Fut: Future<
            Output = Result<
                Option<telemetry_credential_storage::TelemetryCredential>,
                TokenStorageError,
            >,
        > + Send
        + 'static,
{
    if !telemetry_enabled || destination_conflict {
        return Err(telemetry_headers_unavailable());
    }
    let Some(working_dir) = working_dir.filter(|_| auth_db_probe()) else {
        return Err(telemetry_headers_unavailable());
    };
    let credential = run_with_deadline(CREDENTIAL_LOOKUP_DEADLINE, async move {
        let repository_id = resolve_repository(&working_dir)?;
        lookup(repository_id).await.ok().flatten()
    })
    .await
    .flatten();
    render_telemetry_headers(Ok(credential))
}

pub(crate) fn render_telemetry_headers(
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

fn resolve_current_auth_config() -> Result<ResolvedAuthRuntimeConfig, CliError> {
    let cwd = std::env::current_dir()
        .context("failed to determine current directory for auth config resolution")
        .map_err(unexpected_auth_command_error)?;
    config::resolve_auth_runtime_config(&cwd).map_err(unexpected_auth_command_error)
}

fn ensure_auth_allowed(policy: &ResolvedAccessPolicy) -> Result<(), CliError> {
    let Some(file) = policy.auth_disabled_config_file() else {
        return Ok(());
    };
    Err(CliError::user_with_source(
        UserError::AuthDisabledByConfig(file),
        anyhow!(
            "auth.mode resolved to disabled (source: {})",
            policy.auth_mode.source.as_str()
        ),
    ))
}

pub async fn run_login(format: AuthFormat) -> Result<String, CliError> {
    let client = reqwest::Client::new();
    let auth_config = resolve_current_auth_config()?;
    let client_id = auth_config.workos_client_id.value.unwrap_or_default();
    let control_plane = AuthenticatedControlPlaneClient::new(
        client.clone(),
        auth_config.control_plane_base_url.value.unwrap_or_default(),
        auth::WORKOS_DEFAULT_BASE_URL,
        client_id.clone(),
        CredentialAccess::Interactive,
    );
    let telemetry_enabled = auth_config.access_policy.telemetry_enabled();

    let client = &client;
    let control_plane = &control_plane;
    let client_id = client_id.as_str();
    run_login_with(
        format,
        &auth_config.access_policy,
        || token_storage::load_tokens(CredentialAccess::Interactive),
        |stored_tokens| async move {
            maybe_renew_stored_credentials(client, client_id, stored_tokens).await
        },
        || async move { provision_telemetry_credential(control_plane).await },
        |format| async move {
            match format {
                AuthFormat::Text => {
                    run_text_login(client, client_id, control_plane, telemetry_enabled).await
                }
                AuthFormat::Json => {
                    run_login_json(client, client_id, format, control_plane, telemetry_enabled)
                        .await
                }
            }
        },
    )
    .await
}

async fn run_login_with<T, TF, R, P, D, RF, PF, DF>(
    format: AuthFormat,
    policy: &ResolvedAccessPolicy,
    load_tokens: T,
    renew: R,
    provision: P,
    device_login: D,
) -> Result<String, CliError>
where
    T: FnOnce() -> TF,
    TF: Future<Output = Result<Option<StoredTokens>, TokenStorageError>>,
    R: FnOnce(StoredTokens) -> RF,
    RF: Future<Output = Result<Option<StoredTokens>, CliError>>,
    P: FnOnce() -> PF,
    PF: Future<Output = TelemetryProvisioningResult>,
    D: FnOnce(AuthFormat) -> DF,
    DF: Future<Output = Result<String, CliError>>,
{
    ensure_auth_allowed(policy)?;
    let stored_tokens = load_tokens().await.map_err(login_storage_error)?;
    let telemetry_enabled = policy.telemetry_enabled();
    run_login_with_stored_credentials(
        format,
        stored_tokens,
        renew,
        || provision_if_enabled(telemetry_enabled, provision),
        device_login,
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
    let auth_config = resolve_current_auth_config()?;
    let client = AuthenticatedControlPlaneClient::new(
        reqwest::Client::new(),
        auth_config.control_plane_base_url.value.unwrap_or_default(),
        auth::WORKOS_DEFAULT_BASE_URL,
        auth_config.workos_client_id.value.unwrap_or_default(),
        CredentialAccess::Interactive,
    );
    let client = &client;
    run_whoami_with(
        format,
        &auth_config.access_policy,
        || token_storage::load_tokens(CredentialAccess::Interactive),
        || client.me(),
    )
    .await
}

async fn run_whoami_with<T, TF, F, FF>(
    format: AuthFormat,
    policy: &ResolvedAccessPolicy,
    load_tokens: T,
    fetch_profile: F,
) -> Result<String, CliError>
where
    T: FnOnce() -> TF,
    TF: Future<Output = Result<Option<StoredTokens>, TokenStorageError>>,
    F: FnOnce() -> FF,
    FF: Future<Output = Result<MeResponse, ControlPlaneError>>,
{
    ensure_auth_allowed(policy)?;
    if load_tokens().await.map_err(auth_storage_error)?.is_none() {
        if policy.auth_mode.value == AuthMode::Mandatory {
            return Err(CliError::user_with_source(
                UserError::NotAuthenticated,
                anyhow!("auth.mode is mandatory and no stored credentials exist"),
            ));
        }
        return render_unauthenticated_whoami(format).map_err(unexpected_auth_command_error);
    }

    let profile = fetch_profile()
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
        Ok(token) => token_storage::save_tokens(&token, CredentialAccess::Interactive)
            .await
            .map_err(login_storage_error)
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

async fn save_new_session_tokens(token: &auth::TokenResponse) -> Result<StoredTokens, CliError> {
    replace_session_with(
        telemetry_credential_storage::delete_telemetry_credential(),
        || token_storage::save_tokens(token, CredentialAccess::Interactive),
    )
    .await
}

async fn replace_session_with<D, S, SF>(
    invalidate_telemetry: D,
    save_session: S,
) -> Result<StoredTokens, CliError>
where
    D: Future<Output = Result<bool, TokenStorageError>>,
    S: FnOnce() -> SF,
    SF: Future<Output = Result<StoredTokens, TokenStorageError>>,
{
    invalidate_telemetry.await.map_err(login_storage_error)?;
    save_session().await.map_err(login_storage_error)
}

async fn provision_if_enabled<P, PF>(
    telemetry_enabled: bool,
    provision: P,
) -> TelemetryProvisioningResult
where
    P: FnOnce() -> PF,
    PF: Future<Output = TelemetryProvisioningResult>,
{
    if !telemetry_enabled {
        return TelemetryProvisioningResult::Disabled;
    }
    provision().await
}

async fn provision_telemetry_credential(
    client: &AuthenticatedControlPlaneClient,
) -> TelemetryProvisioningResult {
    if client.issue_and_save_telemetry_credential().await.is_err() {
        return TelemetryProvisioningResult::Unavailable;
    }
    enroll_current_repository_if_unbound().await;
    TelemetryProvisioningResult::Issued
}

async fn enroll_current_repository_if_unbound() {
    let Some(repository_id) = std::env::current_dir()
        .ok()
        .and_then(|working_dir| repository_id_for(&working_dir))
    else {
        return;
    };
    let _ = telemetry_credential_storage::bind_repository_telemetry(
        repository_id,
        telemetry_credential_storage::RepositoryBindingMode::EnrollIfUnbound,
    )
    .await;
}

async fn run_text_login(
    client: &reqwest::Client,
    client_id: &str,
    control_plane: &AuthenticatedControlPlaneClient,
    telemetry_enabled: bool,
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

    let stored_tokens = save_new_session_tokens(&token).await?;

    let telemetry = provision_if_enabled(telemetry_enabled, || {
        provision_telemetry_credential(control_plane)
    })
    .await;
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
    telemetry_enabled: bool,
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

    let stored_tokens = save_new_session_tokens(&token).await?;

    let telemetry = provision_if_enabled(telemetry_enabled, || {
        provision_telemetry_credential(control_plane)
    })
    .await;
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
        AuthError::Io(_) => UserError::LoginAuthStorageUnavailable(AuthStorageFailure::Database),
        AuthError::Storage(storage) => UserError::LoginAuthStorageUnavailable(storage.failure()),
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
        TelemetryProvisioningResult::Disabled => {
            format!("{}\nNote: {}", headline, telemetry.message())
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
    } else if let Some(failure) = error.storage_failure() {
        UserError::AuthStorageUnavailable(failure)
    } else {
        UserError::UnexpectedFailure
    };

    CliError::user_with_source(
        user_error,
        anyhow!("failed to fetch authenticated user information from the Control Plane: {error}"),
    )
}

fn auth_storage_error(error: TokenStorageError) -> CliError {
    CliError::user_with_source(UserError::AuthStorageUnavailable(error.failure()), error)
}

fn login_storage_error(error: TokenStorageError) -> CliError {
    CliError::user_with_source(
        UserError::LoginAuthStorageUnavailable(error.failure()),
        error,
    )
}

fn unexpected_auth_command_error(error: anyhow::Error) -> CliError {
    CliError::user_with_source(UserError::UnexpectedFailure, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::config::{ConfigPathSource, ValueSource};
    use crate::services::error::AuthPolicyConfigFile;

    fn storage_failure() -> TokenStorageError {
        TokenStorageError::Database("failed".to_string())
    }

    fn telemetry_credential(token: &str) -> telemetry_credential_storage::TelemetryCredential {
        telemetry_credential_storage::TelemetryCredential {
            token: token.to_string(),
            accessor: "fixture-accessor".to_string(),
            expires_at_unix_seconds: 4_000_000_000,
            identity: telemetry_credential_storage::TelemetryIdentity {
                user_id: "user_a".to_string(),
                workspace_id: "workspace_a".to_string(),
            },
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

    #[tokio::test]
    async fn new_session_invalidates_previous_telemetry_credential_before_saving() {
        let order = std::sync::Mutex::new(Vec::new());
        let saved = replace_session_with(
            async {
                order.lock().unwrap().push("invalidate");
                Ok(true)
            },
            || async {
                order.lock().unwrap().push("save");
                Ok(stored_tokens())
            },
        )
        .await
        .expect("replacement should succeed");
        assert_eq!(saved, stored_tokens());
        assert_eq!(*order.lock().unwrap(), ["invalidate", "save"]);

        let save_called = std::sync::atomic::AtomicBool::new(false);
        let failed = replace_session_with(async { Err(storage_failure()) }, || async {
            save_called.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(stored_tokens())
        })
        .await;
        assert!(!save_called.load(std::sync::atomic::Ordering::SeqCst));
        assert_login_storage_guidance(
            &failed.expect_err("invalidation failure fails login"),
            AuthStorageFailure::Database,
        );

        let failed = replace_session_with(async { Ok(true) }, || async {
            Err(TokenStorageError::CredentialAccessUnavailable)
        })
        .await;
        assert_login_storage_guidance(
            &failed.expect_err("save failure fails login"),
            AuthStorageFailure::CredentialAccess,
        );
    }

    fn assert_login_storage_guidance(error: &CliError, failure: AuthStorageFailure) {
        let CliError::User {
            error: user_error, ..
        } = error
        else {
            panic!("expected a user-facing storage error");
        };
        assert_eq!(*user_error, UserError::LoginAuthStorageUnavailable(failure));
        assert_eq!(user_error.key(), "auth.storage_unavailable");
        assert_eq!(error.class().exit_code(), 4);
        let message = error
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(!message.contains("Retry the original command"));
        let keychain_guidance = message.contains("encryption key");
        assert_eq!(
            keychain_guidance,
            failure == AuthStorageFailure::CredentialAccess
        );
        if keychain_guidance {
            assert!(message.contains("Running login again does not help until"));
            assert!(!message.contains("Run `sce auth login` in an interactive terminal."));
        }
    }

    #[test]
    fn login_device_flow_storage_failure_uses_noncircular_guidance() {
        for (storage, failure) in [
            (
                TokenStorageError::CredentialAccessUnavailable,
                AuthStorageFailure::CredentialAccess,
            ),
            (
                TokenStorageError::DatabaseLocked("busy".to_string()),
                AuthStorageFailure::DatabaseLocked,
            ),
            (
                TokenStorageError::Database("corrupt".to_string()),
                AuthStorageFailure::Database,
            ),
        ] {
            assert_login_storage_guidance(&map_login_error(AuthError::Storage(storage)), failure);
        }
        assert_login_storage_guidance(
            &map_login_error(AuthError::Io(std::io::Error::other("io"))),
            AuthStorageFailure::Database,
        );
        let CliError::User { error, .. } = map_login_error(AuthError::MissingClientId) else {
            panic!("expected a user-facing error");
        };
        assert_eq!(error, UserError::UnexpectedFailure);
    }

    #[tokio::test]
    async fn logout_storage_failure_keeps_command_storage_guidance() {
        let error = run_logout_with(
            AuthFormat::Json,
            async { Err(TokenStorageError::CredentialAccessUnavailable) },
            async { Ok(false) },
        )
        .await
        .expect_err("storage failure fails logout");
        let CliError::User {
            error: user_error, ..
        } = &error
        else {
            panic!("expected a user-facing storage error");
        };
        assert_eq!(
            *user_error,
            UserError::AuthStorageUnavailable(AuthStorageFailure::CredentialAccess)
        );
        assert!(error.to_string().contains("sce auth login"));
    }

    #[tokio::test]
    async fn telemetry_headers_fail_closed_before_lookup_without_repository_or_on_conflict() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let working_dir = Some(PathBuf::from("/workspace/repo"));
        let cases = [
            (
                "policy disabled",
                working_dir.clone(),
                false,
                false,
                true,
                Some("repo-a"),
            ),
            (
                "no auth db",
                working_dir.clone(),
                true,
                false,
                false,
                Some("repo-a"),
            ),
            (
                "destination conflict",
                working_dir.clone(),
                true,
                true,
                true,
                Some("repo-a"),
            ),
            ("no working dir", None, true, false, true, Some("repo-a")),
            (
                "no repository",
                working_dir.clone(),
                true,
                false,
                true,
                None,
            ),
        ];
        for (case, dir, telemetry_enabled, conflict, auth_db, repository) in cases {
            let probed = Arc::new(AtomicBool::new(false));
            let resolved = Arc::new(AtomicBool::new(false));
            let looked_up = Arc::new(AtomicBool::new(false));
            let error = telemetry_headers_with(
                dir,
                telemetry_enabled,
                conflict,
                {
                    let probed = Arc::clone(&probed);
                    move || {
                        probed.store(true, Ordering::SeqCst);
                        auth_db
                    }
                },
                {
                    let resolved = Arc::clone(&resolved);
                    move |_: &Path| {
                        resolved.store(true, Ordering::SeqCst);
                        repository.map(str::to_string)
                    }
                },
                {
                    let looked_up = Arc::clone(&looked_up);
                    move |_repository_id: String| async move {
                        looked_up.store(true, Ordering::SeqCst);
                        Ok(Some(telemetry_credential("tok-scoped")))
                    }
                },
            )
            .await
            .expect_err(case);
            assert_eq!(error.to_string(), TELEMETRY_HEADERS_UNAVAILABLE, "{case}");
            assert!(!looked_up.load(Ordering::SeqCst), "{case}: lookup ran");
            if !telemetry_enabled {
                assert!(!probed.load(Ordering::SeqCst), "{case}: auth.db probed");
            }
            if !telemetry_enabled || !auth_db {
                assert!(
                    !resolved.load(Ordering::SeqCst),
                    "{case}: repository resolved"
                );
            }
        }

        let headers = telemetry_headers_with(
            working_dir,
            true,
            false,
            || true,
            |dir: &Path| (dir == Path::new("/workspace/repo")).then(|| "repo-a".to_string()),
            |repository_id: String| async move {
                assert_eq!(repository_id, "repo-a");
                Ok(Some(telemetry_credential("tok-scoped")))
            },
        )
        .await
        .expect("authorized repository should render");
        assert_eq!(headers, r#"{"Authorization":"Bearer tok-scoped"}"#);
    }

    #[test]
    fn telemetry_headers_policy_follows_auth_and_invalid_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let global = dir.path().join("global.json");
        let local_dir = dir.path().join(".sce");
        std::fs::create_dir_all(&local_dir).unwrap();
        let local = local_dir.join("config.json");
        let enabled = |global_json: &str, local_json: Option<&str>| {
            std::fs::write(&global, global_json).unwrap();
            match local_json {
                Some(json) => std::fs::write(&local, json).unwrap(),
                None => {
                    let _ = std::fs::remove_file(&local);
                }
            }
            config::resolver::resolve_auth_runtime_config_with(
                dir.path(),
                |_| None,
                |path| std::fs::read_to_string(path).map_err(anyhow::Error::from),
                Path::exists,
                || Ok(global.clone()),
            )
            .is_ok_and(|config| config.access_policy.telemetry_enabled())
        };

        assert!(enabled(r#"{"telemetry":{"enabled":true}}"#, None));
        assert!(!enabled("{}", None));
        assert!(!enabled(
            r#"{"telemetry":{"enabled":true},"auth":{"mode":"disabled"}}"#,
            None
        ));
        assert!(!enabled(
            r#"{"telemetry":{"enabled":true}}"#,
            Some(r#"{"auth":{"mode":"disabled"}}"#)
        ));
        assert!(!enabled(
            r#"{"telemetry":{"enabled":true}}"#,
            Some(r#"{"telemetry":{"enabled":"yes"}}"#)
        ));
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
    async fn stored_credentials_are_provisioned_only_after_renewal_completes() {
        let original = stored_tokens();
        let renewed = StoredTokens {
            access_token: "renewed-access".to_string(),
            refresh_token: "renewed-refresh".to_string(),
            ..original.clone()
        };
        for returned in [original.clone(), renewed] {
            let events = std::sync::Mutex::new(Vec::new());
            let events = &events;
            let expected_stored = original.clone();
            let output = run_login_with_stored_credentials(
                AuthFormat::Json,
                Some(original.clone()),
                |stored| async move {
                    assert_eq!(stored, expected_stored);
                    tokio::task::yield_now().await;
                    events.lock().unwrap().push("renewal-complete");
                    Ok(Some(returned))
                },
                || async move {
                    events.lock().unwrap().push("provision");
                    TelemetryProvisioningResult::Issued
                },
                |_format| async { panic!("stored credentials must not use device login") },
            )
            .await
            .expect("stored login should succeed");

            assert_eq!(*events.lock().unwrap(), ["renewal-complete", "provision"]);
            assert!(output.contains("\"renewed\": true"));
            assert!(output.contains("\"status\": \"issued\""));
        }
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
    async fn logout_attempts_both_deletions_before_reporting() {
        use AuthStorageFailure::{CredentialAccess, DatabaseLocked};

        fn deletion(result: Result<bool, AuthStorageFailure>) -> Result<bool, TokenStorageError> {
            result.map_err(|failure| match failure {
                CredentialAccess => TokenStorageError::CredentialAccessUnavailable,
                _ => TokenStorageError::DatabaseLocked("busy".to_string()),
            })
        }

        for (workos, telemetry, expected) in [
            (Ok(false), Ok(true), Ok(())),
            (Ok(true), Err(DatabaseLocked), Err(DatabaseLocked)),
            (Err(CredentialAccess), Ok(true), Err(CredentialAccess)),
            (
                Err(CredentialAccess),
                Err(DatabaseLocked),
                Err(CredentialAccess),
            ),
        ] {
            let events = std::sync::Mutex::new(Vec::new());
            let result = run_logout_with(
                AuthFormat::Json,
                async {
                    events.lock().unwrap().push("delete-workos");
                    deletion(workos)
                },
                async {
                    events.lock().unwrap().push("delete-telemetry");
                    deletion(telemetry)
                },
            )
            .await;

            assert_eq!(
                *events.lock().unwrap(),
                ["delete-workos", "delete-telemetry"],
                "{workos:?} {telemetry:?}"
            );
            match (result, expected) {
                (Ok(output), Ok(())) => assert!(output.contains("\"credentials_removed\": true")),
                (Err(CliError::User { error, .. }), Err(failure)) => {
                    assert_eq!(error, UserError::AuthStorageUnavailable(failure));
                }
                (result, expected) => {
                    panic!("{workos:?} {telemetry:?}: {result:?} != {expected:?}")
                }
            }
        }
    }

    fn access_policy(mode: AuthMode, source: ValueSource, telemetry: bool) -> ResolvedAccessPolicy {
        use crate::services::config::{
            ResolvedValue, TelemetryDisabledReason, TelemetryPolicyState,
        };

        ResolvedAccessPolicy {
            auth_mode: ResolvedValue {
                value: mode,
                source,
            },
            telemetry_enabled: ResolvedValue {
                value: telemetry,
                source: ValueSource::Default,
            },
            telemetry: if telemetry {
                TelemetryPolicyState::Enabled
            } else {
                TelemetryPolicyState::Disabled(TelemetryDisabledReason::NotEnabled)
            },
            background_cloud_allowed: mode != AuthMode::Disabled,
        }
    }

    fn global_policy(mode: AuthMode, telemetry: bool) -> ResolvedAccessPolicy {
        access_policy(
            mode,
            ValueSource::ConfigFile(ConfigPathSource::DefaultDiscoveredGlobal),
            telemetry,
        )
    }

    fn me_response() -> MeResponse {
        use crate::services::agent_trace_sync::control_plane::{MeAuthorization, MeUser};

        MeResponse {
            user: MeUser {
                email: "dev@example.test".to_string(),
                first_name: None,
                last_name: None,
            },
            authorization: MeAuthorization {
                permissions: Vec::new(),
                role: None,
            },
            workspace: None,
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum StoredCredentials {
        None,
        Valid,
        Expired,
    }

    fn user_error_key(result: &Result<String, CliError>) -> Option<&'static str> {
        match result {
            Err(CliError::User { error, .. }) => Some(error.key()),
            _ => None,
        }
    }

    #[tokio::test]
    async fn login_follows_auth_mode_across_stored_credentials() {
        for mode in [AuthMode::Disabled, AuthMode::Optional, AuthMode::Mandatory] {
            for stored in [
                StoredCredentials::None,
                StoredCredentials::Valid,
                StoredCredentials::Expired,
            ] {
                let policy = global_policy(mode, true);
                let storage_reads = std::sync::atomic::AtomicUsize::new(0);
                let storage_reads = &storage_reads;
                let result = run_login_with(
                    AuthFormat::Json,
                    &policy,
                    || async move {
                        storage_reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok(match stored {
                            StoredCredentials::None => None,
                            StoredCredentials::Valid | StoredCredentials::Expired => {
                                Some(stored_tokens())
                            }
                        })
                    },
                    |tokens| async move {
                        Ok(match stored {
                            StoredCredentials::Valid => Some(tokens),
                            StoredCredentials::None | StoredCredentials::Expired => None,
                        })
                    },
                    || async { TelemetryProvisioningResult::Issued },
                    |_format| async { Ok("device-login-complete".to_string()) },
                )
                .await;

                let reads = storage_reads.load(std::sync::atomic::Ordering::SeqCst);
                match (mode, stored) {
                    (AuthMode::Disabled, _) => {
                        assert_eq!(user_error_key(&result), Some("auth.disabled_by_config"));
                        assert_eq!(reads, 0, "{mode:?} {stored:?}");
                    }
                    (_, StoredCredentials::Valid) => {
                        let output = result.expect("renewed login should succeed");
                        assert!(output.contains("\"renewed\": true"));
                        assert!(output.contains("\"status\": \"issued\""));
                        assert_eq!(reads, 1);
                    }
                    (_, StoredCredentials::None | StoredCredentials::Expired) => {
                        assert_eq!(result.expect("device login"), "device-login-complete");
                        assert_eq!(reads, 1);
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn login_skips_provisioning_unless_telemetry_is_enabled() {
        for mode in [AuthMode::Optional, AuthMode::Mandatory] {
            let output = run_login_with(
                AuthFormat::Json,
                &global_policy(mode, false),
                || async { Ok(Some(stored_tokens())) },
                |tokens| async move { Ok(Some(tokens)) },
                || async { panic!("provisioning must not run without telemetry consent") },
                |_format| async { panic!("renewed credentials must not use device login") },
            )
            .await
            .expect("renewed login should succeed");

            assert!(output.contains("\"status\": \"disabled\""));
            assert!(output.contains(TELEMETRY_PROVISIONING_DISABLED));
        }

        let disabled = provision_if_enabled(false, || async {
            panic!("provisioning must not run without telemetry consent")
        })
        .await;
        let enabled =
            provision_if_enabled(true, || async { TelemetryProvisioningResult::Unavailable }).await;

        assert_eq!(disabled, TelemetryProvisioningResult::Disabled);
        assert_eq!(enabled, TelemetryProvisioningResult::Unavailable);
    }

    #[tokio::test]
    async fn whoami_follows_auth_mode_across_stored_credentials() {
        let unauthenticated =
            render_unauthenticated_whoami(AuthFormat::Json).expect("unauthenticated render");
        let authenticated =
            render_whoami_result(&me_response(), AuthFormat::Json).expect("profile render");

        for mode in [AuthMode::Disabled, AuthMode::Optional, AuthMode::Mandatory] {
            for stored in [
                StoredCredentials::None,
                StoredCredentials::Valid,
                StoredCredentials::Expired,
            ] {
                let policy = global_policy(mode, false);
                let storage_reads = std::sync::atomic::AtomicUsize::new(0);
                let storage_reads = &storage_reads;
                let result = run_whoami_with(
                    AuthFormat::Json,
                    &policy,
                    || async move {
                        storage_reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok(match stored {
                            StoredCredentials::None => None,
                            StoredCredentials::Valid | StoredCredentials::Expired => {
                                Some(stored_tokens())
                            }
                        })
                    },
                    || async move {
                        match stored {
                            StoredCredentials::Valid => Ok(me_response()),
                            StoredCredentials::Expired => Err(
                                ControlPlaneError::AuthenticationFailed("expired".to_string()),
                            ),
                            StoredCredentials::None => {
                                panic!("no stored credentials must not fetch a profile")
                            }
                        }
                    },
                )
                .await;

                let reads = storage_reads.load(std::sync::atomic::Ordering::SeqCst);
                match (mode, stored) {
                    (AuthMode::Disabled, _) => {
                        assert_eq!(user_error_key(&result), Some("auth.disabled_by_config"));
                        assert_eq!(reads, 0);
                    }
                    (AuthMode::Optional, StoredCredentials::None) => {
                        assert_eq!(result.expect("unauthenticated success"), unauthenticated);
                    }
                    (AuthMode::Mandatory, StoredCredentials::None)
                    | (_, StoredCredentials::Expired) => {
                        assert_eq!(
                            user_error_key(&result),
                            Some("auth.not_authenticated"),
                            "{mode:?} {stored:?}"
                        );
                    }
                    (_, StoredCredentials::Valid) => {
                        assert_eq!(result.expect("profile"), authenticated);
                    }
                }
                if mode != AuthMode::Disabled {
                    assert_eq!(reads, 1, "{mode:?} {stored:?}");
                }
            }
        }
    }

    #[test]
    fn auth_disabled_rejection_names_the_deciding_config_file() {
        for (source, expected) in [
            (
                ValueSource::ConfigFile(ConfigPathSource::DefaultDiscoveredGlobal),
                AuthPolicyConfigFile::Global,
            ),
            (
                ValueSource::ConfigFile(ConfigPathSource::DefaultDiscoveredLocal),
                AuthPolicyConfigFile::Repository,
            ),
            (
                ValueSource::ConfigFile(ConfigPathSource::Flag),
                AuthPolicyConfigFile::ConfigFlag,
            ),
            (
                ValueSource::ConfigFile(ConfigPathSource::Env),
                AuthPolicyConfigFile::ConfigEnv,
            ),
        ] {
            let Err(CliError::User { error, .. }) =
                ensure_auth_allowed(&access_policy(AuthMode::Disabled, source, true))
            else {
                panic!("disabled auth must be rejected for {source:?}");
            };
            assert_eq!(error, UserError::AuthDisabledByConfig(expected));
        }

        for mode in [AuthMode::Optional, AuthMode::Mandatory] {
            assert!(ensure_auth_allowed(&global_policy(mode, false)).is_ok());
        }
    }

    #[test]
    fn login_reports_disabled_telemetry_provisioning_in_text_and_json() {
        let text =
            render_login_text_with_color_policy(TelemetryProvisioningResult::Disabled, false);
        let json = render_login_result_with_telemetry(
            &device_auth_result(),
            AuthFormat::Json,
            TelemetryProvisioningResult::Disabled,
        )
        .expect("disabled JSON login output should render");
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

        assert_eq!(
            text,
            "✓ Authentication succeeded.\nNote: Telemetry credential provisioning is disabled by SCE config. Set `telemetry.enabled: true` in the global SCE config to enable it."
        );
        assert_eq!(
            value["telemetry_provisioning"],
            json!({
                "status": "disabled",
                "message": TELEMETRY_PROVISIONING_DISABLED,
            })
        );
    }

    #[tokio::test]
    async fn logout_deletes_both_credential_rows_when_auth_is_disabled_by_config() {
        let dir = tempfile::tempdir().unwrap();
        let global = dir.path().join("global.json");
        std::fs::write(&global, r#"{"auth":{"mode":"disabled"}}"#).unwrap();
        let policy = config::resolver::resolve_auth_runtime_config_with(
            dir.path(),
            |_| None,
            |path| std::fs::read_to_string(path).map_err(anyhow::Error::from),
            Path::exists,
            || Ok(global.clone()),
        )
        .expect("config resolves")
        .access_policy;
        assert!(!policy.auth_allowed());

        let events = std::sync::Mutex::new(Vec::new());
        let output = run_logout_with(
            AuthFormat::Json,
            async {
                events.lock().unwrap().push("delete-workos");
                Ok(true)
            },
            async {
                events.lock().unwrap().push("delete-telemetry");
                Ok(true)
            },
        )
        .await
        .expect("logout should succeed");

        assert_eq!(
            *events.lock().unwrap(),
            ["delete-workos", "delete-telemetry"]
        );
        assert!(output.contains("\"credentials_removed\": true"));
    }
}
