pub mod command;

use std::io::Write;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde_json::json;

use crate::services::auth::{self, AuthError, DeviceAuthFlowResult};
use crate::services::config;
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
    Status { format: AuthFormat },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthRequest {
    pub subcommand: AuthSubcommand,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AuthStatusReport {
    authentication_state: &'static str,
    stored_credentials_path: String,
    has_stored_credentials: bool,
    token_expired: Option<bool>,
    token_type: Option<String>,
    scope: Option<String>,
    stored_at_unix_seconds: Option<u64>,
    expires_at_unix_seconds: Option<u64>,
    seconds_until_expiry: Option<i64>,
}

pub fn run_auth_subcommand(request: AuthRequest) -> Result<String> {
    run_auth_subcommand_with(request, run_login, run_logout, run_status)
}

fn run_auth_subcommand_with<L, O, S>(
    request: AuthRequest,
    login: L,
    logout: O,
    status: S,
) -> Result<String>
where
    L: FnOnce(AuthFormat) -> Result<String>,
    O: FnOnce(AuthFormat) -> Result<String>,
    S: FnOnce(AuthFormat) -> Result<String>,
{
    match request.subcommand {
        AuthSubcommand::Login { format } => login(format),
        AuthSubcommand::Logout { format } => logout(format),
        AuthSubcommand::Status { format } => status(format),
    }
}

pub fn run_login(format: AuthFormat) -> Result<String> {
    let client = reqwest::Client::new();
    let runtime = shared_runtime()?;

    let client_id = resolve_login_client_id()?;

    run_login_with_stored_credentials(
        format,
        token_storage::load_tokens()?,
        |stored_tokens| maybe_renew_stored_credentials(runtime, &client, &client_id, stored_tokens),
        |format| match format {
            AuthFormat::Text => run_text_login_with_runtime(runtime, &client, &client_id),
            AuthFormat::Json => run_login_json(runtime, &client, &client_id, format),
        },
    )
}

pub fn run_logout(format: AuthFormat) -> Result<String> {
    let deleted = token_storage::delete_tokens().map_err(|error| {
        let guidance = auth_state_path_guidance(
            "verify file permissions for the auth state directory and rerun 'sce auth logout'",
        );
        anyhow!(format!("{error} Try: {guidance}"))
    })?;
    render_logout_result(deleted, format)
}

pub fn run_status(format: AuthFormat) -> Result<String> {
    let stored_credentials_path = token_storage::token_file_path()?.display().to_string();
    let report = match token_storage::load_tokens()? {
        Some(tokens) => {
            let tokens = maybe_refresh_tokens_for_status(&tokens)?.unwrap_or(tokens);
            build_authenticated_status_report(&tokens, stored_credentials_path)?
        }
        None => AuthStatusReport {
            authentication_state: "unauthenticated",
            stored_credentials_path,
            has_stored_credentials: false,
            token_expired: None,
            token_type: None,
            scope: None,
            stored_at_unix_seconds: None,
            expires_at_unix_seconds: None,
            seconds_until_expiry: None,
        },
    };

    render_status_result(&report, format)
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
) -> Result<Option<StoredTokens>> {
    match runtime.block_on(auth::ensure_valid_token_returning_token(
        client,
        auth::WORKOS_DEFAULT_BASE_URL,
        client_id,
        stored_tokens,
    )) {
        Ok(token) => Ok(Some(token_storage::save_tokens(&token)?)),
        Err(_) => Ok(None),
    }
}

fn run_login_with_stored_credentials<R, D>(
    format: AuthFormat,
    stored_tokens: Option<StoredTokens>,
    renew: R,
    device_login: D,
) -> Result<String>
where
    R: FnOnce(&StoredTokens) -> Result<Option<StoredTokens>>,
    D: FnOnce(AuthFormat) -> Result<String>,
{
    if let Some(stored_tokens) = stored_tokens {
        if let Some(renewed_tokens) = renew(&stored_tokens)? {
            return render_login_refresh_result(&renewed_tokens, format);
        }
    }

    device_login(format)
}

fn maybe_refresh_tokens_for_status(stored_tokens: &StoredTokens) -> Result<Option<StoredTokens>> {
    if !auth::is_stored_token_expired(stored_tokens)? {
        return Ok(None);
    }

    let client_id = resolve_login_client_id()?;
    let runtime = shared_runtime()?;
    let client = reqwest::Client::new();

    match runtime.block_on(auth::ensure_valid_token_returning_token(
        &client,
        auth::WORKOS_DEFAULT_BASE_URL,
        &client_id,
        stored_tokens,
    )) {
        Ok(token) => Ok(Some(token_storage::save_tokens(&token)?)),
        Err(_) => Ok(None),
    }
}

fn run_text_login_with_runtime(
    runtime: &tokio::runtime::Runtime,
    client: &reqwest::Client,
    client_id: &str,
) -> Result<String> {
    let authorization = runtime
        .block_on(auth::request_device_authorization(
            client,
            auth::WORKOS_DEFAULT_BASE_URL,
            client_id,
        ))
        .map_err(|e| map_login_error(&e))?;

    write_login_prompt(&authorization)?;

    let token = runtime
        .block_on(auth::complete_device_auth_flow_returning_token(
            client,
            auth::WORKOS_DEFAULT_BASE_URL,
            client_id,
            &authorization,
        ))
        .map_err(|e| map_login_error(&e))?;

    let stored_tokens = token_storage::save_tokens(&token)?;

    render_login_result(
        &DeviceAuthFlowResult {
            authorization,
            stored_tokens,
        },
        AuthFormat::Text,
    )
}

fn run_login_json(
    runtime: &tokio::runtime::Runtime,
    client: &reqwest::Client,
    client_id: &str,
    format: AuthFormat,
) -> Result<String> {
    let authorization = runtime
        .block_on(auth::request_device_authorization(
            client,
            auth::WORKOS_DEFAULT_BASE_URL,
            client_id,
        ))
        .map_err(|e| map_login_error(&e))?;

    let token = runtime
        .block_on(auth::complete_device_auth_flow_returning_token(
            client,
            auth::WORKOS_DEFAULT_BASE_URL,
            client_id,
            &authorization,
        ))
        .map_err(|e| map_login_error(&e))?;

    let stored_tokens = token_storage::save_tokens(&token)?;

    render_login_result(
        &DeviceAuthFlowResult {
            authorization,
            stored_tokens,
        },
        format,
    )
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

fn map_login_error(error: &AuthError) -> anyhow::Error {
    anyhow!(with_try_guidance(
        error.to_string(),
        "verify the resolved WorkOS client ID source (WORKOS_CLIENT_ID, config file, or baked default), confirm network access, and rerun 'sce auth login'."
    ))
}

fn build_authenticated_status_report(
    tokens: &StoredTokens,
    stored_credentials_path: String,
) -> Result<AuthStatusReport> {
    let now_unix_seconds = current_unix_timestamp_seconds()?;
    let expires_at_unix_seconds = tokens
        .stored_at_unix_seconds
        .saturating_add(tokens.expires_in);
    let seconds_until_expiry = i64::try_from(expires_at_unix_seconds).unwrap_or(i64::MAX)
        - i64::try_from(now_unix_seconds).unwrap_or(0);

    Ok(AuthStatusReport {
        authentication_state: "authenticated",
        stored_credentials_path,
        has_stored_credentials: true,
        token_expired: Some(seconds_until_expiry <= 0),
        token_type: Some(tokens.token_type.clone()),
        scope: tokens.scope.clone(),
        stored_at_unix_seconds: Some(tokens.stored_at_unix_seconds),
        expires_at_unix_seconds: Some(expires_at_unix_seconds),
        seconds_until_expiry: Some(seconds_until_expiry),
    })
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

fn render_status_result(report: &AuthStatusReport, format: AuthFormat) -> Result<String> {
    match format {
        AuthFormat::Text => {
            if !report.has_stored_credentials {
                return Ok(format!(
                    "{} {}",
                    label("Authentication status:"),
                    value("unauthenticated")
                ) + &format!(
                    "\n{} {}\n{} {}",
                    label("Stored credentials:"),
                    value("none"),
                    label("Credentials file:"),
                    value(&report.stored_credentials_path),
                ));
            }

            Ok(format!(
                "{} {}\n{} {}\n{} {}\n{} {}\n{} {}\n{} {}\n{} {}\n{} {}",
                label("Authentication status:"),
                value(report.authentication_state),
                label("Stored credentials:"),
                value("present"),
                label("Credentials file:"),
                value(&report.stored_credentials_path),
                label("Token expired:"),
                value(&report.token_expired.unwrap_or(false).to_string()),
                label("Seconds until expiry:"),
                value(&report.seconds_until_expiry.unwrap_or_default().to_string()),
                label("Expires at (unix):"),
                value(&report.expires_at_unix_seconds.unwrap_or_default().to_string()),
                label("Token type:"),
                value(report.token_type.as_deref().unwrap_or("(unknown)")),
                label("Scope:"),
                value(report.scope.as_deref().unwrap_or("(none)")),
            ))
        }
        AuthFormat::Json => serde_json::to_string_pretty(&json!({
            "status": "ok",
            "command": NAME,
            "subcommand": "status",
            "authentication_state": report.authentication_state,
            "stored_credentials_path": report.stored_credentials_path,
            "has_stored_credentials": report.has_stored_credentials,
            "token_expired": report.token_expired,
            "token_type": report.token_type,
            "scope": report.scope,
            "stored_at_unix_seconds": report.stored_at_unix_seconds,
            "expires_at_unix_seconds": report.expires_at_unix_seconds,
            "seconds_until_expiry": report.seconds_until_expiry,
        }))
        .context("failed to serialize auth status report to JSON. Try: rerun 'sce auth status --format json'."),
    }
}

fn current_unix_timestamp_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| anyhow!("system clock is invalid for auth status checks: {error}. Try: verify local system time and rerun 'sce auth status'."))?
        .as_secs())
}

fn with_try_guidance(message: String, guidance: &str) -> String {
    if message.contains("Try:") {
        message
    } else {
        format!("{message} Try: {guidance}")
    }
}

fn auth_state_path_guidance(action: &str) -> String {
    match token_storage::token_file_path() {
        Ok(path) => format!("{action}; expected path: '{}'", path.display()),
        Err(_) => action.to_string(),
    }
}
