use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{io::Write, path::Path};

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
    Renew { format: AuthFormat, force: bool },
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
    run_auth_subcommand_with(request, run_login, run_renew, run_logout, run_status)
}

fn run_auth_subcommand_with<L, R, O, S>(
    request: AuthRequest,
    login: L,
    renew: R,
    logout: O,
    status: S,
) -> Result<String>
where
    L: FnOnce(AuthFormat) -> Result<String>,
    R: FnOnce(AuthFormat, bool) -> Result<String>,
    O: FnOnce(AuthFormat) -> Result<String>,
    S: FnOnce(AuthFormat) -> Result<String>,
{
    match request.subcommand {
        AuthSubcommand::Login { format } => login(format),
        AuthSubcommand::Renew { format, force } => renew(format, force),
        AuthSubcommand::Logout { format } => logout(format),
        AuthSubcommand::Status { format } => status(format),
    }
}

pub fn run_login(format: AuthFormat) -> Result<String> {
    let client = reqwest::Client::new();
    let runtime = shared_runtime()?;

    let client_id = resolve_login_client_id()?;

    if let Some(stored_tokens) = maybe_renew_expired_credentials(runtime, &client, &client_id)? {
        return render_login_refresh_result(&stored_tokens, format);
    }

    match format {
        AuthFormat::Text => run_text_login_with_runtime(runtime, &client, &client_id),
        AuthFormat::Json => run_login_with(
            format,
            || Ok(client_id),
            |client_id| {
                runtime
                    .block_on(auth::start_device_auth_flow(
                        &client,
                        auth::WORKOS_DEFAULT_BASE_URL,
                        client_id,
                    ))
                    .map_err(|e| map_login_error(&e))
            },
        ),
    }
}

pub fn run_logout(format: AuthFormat) -> Result<String> {
    let deleted = token_storage::delete_tokens().map_err(|error| {
        let guidance = auth_state_path_guidance(
            "verify file permissions for the auth state directory and rerun 'sce auth logout'",
        );
        anyhow!(with_try_guidance(error.to_string(), &guidance,))
    })?;
    render_logout_result(deleted, format)
}

pub fn run_renew(format: AuthFormat, force: bool) -> Result<String> {
    let client = reqwest::Client::new();
    let runtime = shared_runtime()?;
    let client_id = resolve_login_client_id()?;

    let Some(stored_tokens) = token_storage::load_tokens()? else {
        return Err(anyhow!(AuthError::Unauthorized(
            "No stored WorkOS credentials were found. Try: run 'sce auth login' before running 'sce auth renew'.".to_string(),
        )));
    };

    let was_expired = auth::is_stored_token_expired(&stored_tokens)?;
    let updated = if force {
        runtime
            .block_on(auth::renew_stored_token(
                &client,
                auth::WORKOS_DEFAULT_BASE_URL,
                &client_id,
            ))
            .map_err(|e| map_login_error(&e))?
    } else {
        runtime
            .block_on(auth::ensure_valid_token(
                &client,
                auth::WORKOS_DEFAULT_BASE_URL,
                &client_id,
            ))
            .map_err(|e| map_login_error(&e))?
    };

    render_renew_result(&updated, force || was_expired, format)
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

fn run_login_with<R, S>(format: AuthFormat, resolve_client_id: R, start_flow: S) -> Result<String>
where
    R: FnOnce() -> Result<String>,
    S: FnOnce(&str) -> Result<DeviceAuthFlowResult>,
{
    let client_id = resolve_client_id()?;
    let result = start_flow(&client_id)?;
    render_login_result(&result, format)
}

fn maybe_renew_expired_credentials(
    runtime: &tokio::runtime::Runtime,
    client: &reqwest::Client,
    client_id: &str,
) -> Result<Option<StoredTokens>> {
    let Some(stored_tokens) = token_storage::load_tokens()? else {
        return Ok(None);
    };

    if !auth::is_stored_token_expired(&stored_tokens)? {
        return Ok(None);
    }

    match runtime.block_on(auth::ensure_valid_token(
        client,
        auth::WORKOS_DEFAULT_BASE_URL,
        client_id,
    )) {
        Ok(updated) => Ok(Some(updated)),
        Err(_) => Ok(None),
    }
}

fn maybe_refresh_tokens_for_status(stored_tokens: &StoredTokens) -> Result<Option<StoredTokens>> {
    if !auth::is_stored_token_expired(stored_tokens)? {
        return Ok(None);
    }

    let client_id = resolve_login_client_id()?;
    let runtime = shared_runtime()?;
    let client = reqwest::Client::new();

    match runtime.block_on(auth::ensure_valid_token(
        &client,
        auth::WORKOS_DEFAULT_BASE_URL,
        &client_id,
    )) {
        Ok(updated) => Ok(Some(updated)),
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

    let stored_tokens = runtime
        .block_on(auth::complete_device_auth_flow(
            client,
            auth::WORKOS_DEFAULT_BASE_URL,
            client_id,
            &authorization,
        ))
        .map_err(|e| map_login_error(&e))?;

    render_login_result(
        &DeviceAuthFlowResult {
            authorization,
            stored_tokens,
        },
        AuthFormat::Text,
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

#[allow(dead_code)]
fn resolve_login_client_id_with<FEnv, FRead, FGlobalPath>(
    cwd: &Path,
    env_lookup: FEnv,
    read_file: FRead,
    path_exists: fn(&Path) -> bool,
    resolve_global_config_path: FGlobalPath,
) -> Result<String>
where
    FEnv: Fn(&str) -> Option<String>,
    FRead: Fn(&Path) -> Result<String>,
    FGlobalPath: Fn() -> Result<std::path::PathBuf>,
{
    Ok(config::resolve_auth_runtime_config_with(
        cwd,
        env_lookup,
        read_file,
        path_exists,
        resolve_global_config_path,
    )?
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
    let browser_url = result
        .authorization
        .verification_uri_complete
        .as_deref()
        .unwrap_or(&result.authorization.verification_uri);

    match format {
        AuthFormat::Text => Ok(format!(
            "{}\n{} {}\n{} {}\n{} {}\n{} {}",
            success("Authentication succeeded."),
            prompt_label("Open in browser:"),
            prompt_value(browser_url),
            prompt_label("Code:"),
            prompt_value(&result.authorization.user_code),
            label("Token type:"),
            value(&result.stored_tokens.token_type),
            label("Expires at (unix):"),
            value(&expires_at_unix_seconds.to_string()),
        )),
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
        AuthFormat::Text => Ok(format!(
            "{}\n{} {}\n{} {}",
            success("Authentication renewed."),
            label("Token type:"),
            value(&tokens.token_type),
            label("Expires at (unix):"),
            value(&expires_at_unix_seconds.to_string()),
        )),
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

fn render_renew_result(tokens: &StoredTokens, renewed: bool, format: AuthFormat) -> Result<String> {
    let expires_at_unix_seconds = tokens
        .stored_at_unix_seconds
        .saturating_add(tokens.expires_in);

    let status_text = if renewed {
        "renewed"
    } else {
        "is already valid"
    };

    match format {
        AuthFormat::Text => Ok(format!(
            "{}\n{} {}\n{} {}",
            success(&format!("Authentication {status_text}.")),
            label("Token type:"),
            value(&tokens.token_type),
            label("Expires at (unix):"),
            value(&expires_at_unix_seconds.to_string()),
        )),
        AuthFormat::Json => serde_json::to_string_pretty(&json!({
            "status": "ok",
            "command": NAME,
            "subcommand": "renew",
            "authenticated": true,
            "renewed": renewed,
            "token_type": tokens.token_type,
            "scope": tokens.scope,
            "stored_at_unix_seconds": tokens.stored_at_unix_seconds,
            "expires_in_seconds": tokens.expires_in,
            "expires_at_unix_seconds": expires_at_unix_seconds,
        }))
        .context("failed to serialize auth renew report to JSON. Try: rerun 'sce auth renew --format json'."),
    }
}

fn render_logout_result(deleted: bool, format: AuthFormat) -> Result<String> {
    match format {
        AuthFormat::Text => Ok(if deleted {
            success("Removed stored WorkOS credentials.")
        } else {
            value("No stored WorkOS credentials were found.")
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

#[cfg(test)]
mod tests {
    use anyhow::{anyhow, Result};
    use serde_json::Value;
    use std::path::{Path, PathBuf};
    use tokio::net::TcpListener;

    use super::{
        build_authenticated_status_report, render_login_result, render_logout_result,
        render_renew_result, render_status_result, resolve_login_client_id_with,
        run_auth_subcommand_with, run_login_with, with_try_guidance, AuthFormat, AuthRequest,
        AuthStatusReport, AuthSubcommand,
    };
    use crate::services::auth::{AuthError, DeviceAuthFlowResult, DeviceAuthorizationResponse};
    use crate::services::token_storage::StoredTokens;

    fn fixture_login_result() -> DeviceAuthFlowResult {
        DeviceAuthFlowResult {
            authorization: DeviceAuthorizationResponse {
                device_code: "device-code".to_string(),
                user_code: "ABCD-EFGH".to_string(),
                verification_uri: "https://workos.com/device".to_string(),
                verification_uri_complete: Some(
                    "https://workos.com/device?user_code=ABCD-EFGH".to_string(),
                ),
                expires_in: 900,
                interval: Some(5),
            },
            stored_tokens: StoredTokens {
                access_token: "access-token".to_string(),
                token_type: "Bearer".to_string(),
                expires_in: 3600,
                refresh_token: "refresh-token".to_string(),
                scope: Some("openid profile".to_string()),
                stored_at_unix_seconds: 1_700_000_000,
            },
        }
    }

    #[test]
    fn dispatcher_routes_login_to_login_handler() -> Result<()> {
        let output = run_auth_subcommand_with(
            AuthRequest {
                subcommand: AuthSubcommand::Login {
                    format: AuthFormat::Text,
                },
            },
            |_| Ok("login".to_string()),
            |_, _| Ok("renew".to_string()),
            |_| Ok("logout".to_string()),
            |_| Ok("status".to_string()),
        )?;

        assert_eq!(output, "login");
        Ok(())
    }

    #[test]
    fn dispatcher_routes_logout_to_logout_handler() -> Result<()> {
        let output = run_auth_subcommand_with(
            AuthRequest {
                subcommand: AuthSubcommand::Logout {
                    format: AuthFormat::Json,
                },
            },
            |_| Ok("login".to_string()),
            |_, _| Ok("renew".to_string()),
            |_| Ok("logout".to_string()),
            |_| Ok("status".to_string()),
        )?;

        assert_eq!(output, "logout");
        Ok(())
    }

    #[test]
    fn dispatcher_routes_status_to_status_handler() -> Result<()> {
        let output = run_auth_subcommand_with(
            AuthRequest {
                subcommand: AuthSubcommand::Status {
                    format: AuthFormat::Text,
                },
            },
            |_| Ok("login".to_string()),
            |_, _| Ok("renew".to_string()),
            |_| Ok("logout".to_string()),
            |_| Ok("status".to_string()),
        )?;

        assert_eq!(output, "status");
        Ok(())
    }

    #[test]
    fn dispatcher_routes_renew_to_renew_handler() -> Result<()> {
        let output = run_auth_subcommand_with(
            AuthRequest {
                subcommand: AuthSubcommand::Renew {
                    format: AuthFormat::Text,
                    force: true,
                },
            },
            |_| Ok("login".to_string()),
            |format, force| {
                assert_eq!(format, AuthFormat::Text);
                assert!(force);
                Ok("renew".to_string())
            },
            |_| Ok("logout".to_string()),
            |_| Ok("status".to_string()),
        )?;

        assert_eq!(output, "renew");
        Ok(())
    }

    #[test]
    fn login_json_output_includes_stable_fields() -> Result<()> {
        let output = render_login_result(&fixture_login_result(), AuthFormat::Json)?;
        let parsed: Value = serde_json::from_str(&output)?;

        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["command"], "auth");
        assert_eq!(parsed["subcommand"], "login");
        assert_eq!(parsed["authenticated"], true);
        assert_eq!(parsed["user_code"], "ABCD-EFGH");
        Ok(())
    }

    #[test]
    fn login_text_output_is_readable_and_uses_browser_url() -> Result<()> {
        let output = render_login_result(&fixture_login_result(), AuthFormat::Text)?;

        assert!(output.contains("Authentication succeeded."));
        assert!(output.contains("Open in browser: https://workos.com/device?user_code=ABCD-EFGH"));
        assert!(output.contains("Code: ABCD-EFGH"));
        assert!(!output.contains("Verification URL (complete):"));
        Ok(())
    }

    #[test]
    fn login_uses_env_workos_client_id_over_config_sources() -> Result<()> {
        let output = run_login_with(
            AuthFormat::Json,
            || {
                resolve_login_client_id_with(
                    Path::new("/workspace"),
                    |key| match key {
                        "WORKOS_CLIENT_ID" => Some("env-client".to_string()),
                        _ => None,
                    },
                    |_| Ok("{\"workos_client_id\":\"config-client\"}".to_string()),
                    |_| true,
                    || Ok(PathBuf::from("/config/sce/config.json")),
                )
            },
            |client_id| {
                assert_eq!(client_id, "env-client");
                Ok(fixture_login_result())
            },
        )?;

        let parsed: Value = serde_json::from_str(&output)?;
        assert_eq!(parsed["authenticated"], true);
        Ok(())
    }

    #[test]
    fn login_uses_local_config_workos_client_id_when_env_is_absent() -> Result<()> {
        run_login_with(
            AuthFormat::Text,
            || {
                resolve_login_client_id_with(
                    Path::new("/workspace"),
                    |_| None,
                    |path| {
                        if path == Path::new("/config/sce/config.json") {
                            return Ok("{\"workos_client_id\":\"global-client\"}".to_string());
                        }
                        if path == Path::new("/workspace/.sce/config.json") {
                            return Ok("{\"workos_client_id\":\"local-client\"}".to_string());
                        }
                        Err(anyhow!("unexpected config path: {}", path.display()))
                    },
                    |path| {
                        path == Path::new("/config/sce/config.json")
                            || path == Path::new("/workspace/.sce/config.json")
                    },
                    || Ok(PathBuf::from("/config/sce/config.json")),
                )
            },
            |client_id| {
                assert_eq!(client_id, "local-client");
                Ok(fixture_login_result())
            },
        )?;

        Ok(())
    }

    #[test]
    fn login_uses_global_config_workos_client_id_when_local_omits_key() -> Result<()> {
        run_login_with(
            AuthFormat::Text,
            || {
                resolve_login_client_id_with(
                    Path::new("/workspace"),
                    |_| None,
                    |path| {
                        if path == Path::new("/config/sce/config.json") {
                            return Ok("{\"workos_client_id\":\"global-client\"}".to_string());
                        }
                        if path == Path::new("/workspace/.sce/config.json") {
                            return Ok("{}".to_string());
                        }
                        Err(anyhow!("unexpected config path: {}", path.display()))
                    },
                    |path| {
                        path == Path::new("/config/sce/config.json")
                            || path == Path::new("/workspace/.sce/config.json")
                    },
                    || Ok(PathBuf::from("/config/sce/config.json")),
                )
            },
            |client_id| {
                assert_eq!(client_id, "global-client");
                Ok(fixture_login_result())
            },
        )?;

        Ok(())
    }

    #[test]
    fn login_uses_baked_default_workos_client_id_when_env_and_config_are_absent() -> Result<()> {
        run_login_with(
            AuthFormat::Text,
            || {
                resolve_login_client_id_with(
                    Path::new("/workspace"),
                    |_| None,
                    |_| Ok("{}".to_string()),
                    |_| false,
                    || Ok(PathBuf::from("/config/sce/config.json")),
                )
            },
            |client_id| {
                assert_eq!(client_id, "client_sce_default");
                Ok(fixture_login_result())
            },
        )?;

        Ok(())
    }

    #[test]
    fn login_preserves_missing_client_id_error_when_highest_precedence_value_is_blank() {
        let error = run_login_with(
            AuthFormat::Text,
            || {
                resolve_login_client_id_with(
                    Path::new("/workspace"),
                    |key| match key {
                        "WORKOS_CLIENT_ID" => Some("   ".to_string()),
                        _ => None,
                    },
                    |_| Ok("{\"workos_client_id\":\"config-client\"}".to_string()),
                    |_| true,
                    || Ok(PathBuf::from("/config/sce/config.json")),
                )
            },
            |_| Err(anyhow!(AuthError::MissingClientId.to_string())),
        )
        .expect_err("blank env client id should fail");

        assert!(error
            .to_string()
            .contains("WorkOS client ID is not configured"));
    }

    #[test]
    fn logout_json_output_reports_removal_state() -> Result<()> {
        let output = render_logout_result(true, AuthFormat::Json)?;
        let parsed: Value = serde_json::from_str(&output)?;

        assert_eq!(parsed["subcommand"], "logout");
        assert_eq!(parsed["credentials_removed"], true);
        Ok(())
    }

    #[test]
    fn renew_json_output_reports_renewal_state() -> Result<()> {
        let output = render_renew_result(
            &fixture_login_result().stored_tokens,
            true,
            AuthFormat::Json,
        )?;
        let parsed: Value = serde_json::from_str(&output)?;

        assert_eq!(parsed["subcommand"], "renew");
        assert_eq!(parsed["renewed"], true);
        Ok(())
    }

    #[test]
    fn status_text_output_reports_unauthenticated_state() -> Result<()> {
        let output = render_status_result(
            &AuthStatusReport {
                authentication_state: "unauthenticated",
                stored_credentials_path: "/tmp/sce/auth/tokens.json".to_string(),
                has_stored_credentials: false,
                token_expired: None,
                token_type: None,
                scope: None,
                stored_at_unix_seconds: None,
                expires_at_unix_seconds: None,
                seconds_until_expiry: None,
            },
            AuthFormat::Text,
        )?;

        assert!(output.contains("unauthenticated"));
        assert!(output.contains("Stored credentials: none"));
        assert!(output.contains("Credentials file: /tmp/sce/auth/tokens.json"));
        Ok(())
    }

    #[test]
    fn status_json_output_reports_expiry_fields() -> Result<()> {
        let report = build_authenticated_status_report(
            &StoredTokens {
                access_token: "access-token".to_string(),
                token_type: "Bearer".to_string(),
                expires_in: 3600,
                refresh_token: "refresh-token".to_string(),
                scope: Some("openid profile".to_string()),
                stored_at_unix_seconds: super::current_unix_timestamp_seconds()? - 60,
            },
            "/tmp/sce/auth/tokens.json".to_string(),
        )?;

        let output = render_status_result(&report, AuthFormat::Json)?;
        let parsed: Value = serde_json::from_str(&output)?;

        assert_eq!(parsed["subcommand"], "status");
        assert_eq!(parsed["authentication_state"], "authenticated");
        assert_eq!(
            parsed["stored_credentials_path"],
            "/tmp/sce/auth/tokens.json"
        );
        assert!(parsed["has_stored_credentials"].as_bool().unwrap_or(false));
        assert!(parsed["seconds_until_expiry"].as_i64().is_some());
        Ok(())
    }

    #[test]
    fn try_guidance_is_added_only_when_missing() {
        let added = with_try_guidance("runtime failed".to_string(), "rerun the command.");
        assert_eq!(added, "runtime failed Try: rerun the command.");

        let preserved = with_try_guidance(
            "runtime failed. Try: rerun the command.".to_string(),
            "something else.",
        );
        assert_eq!(preserved, "runtime failed. Try: rerun the command.");
    }

    #[test]
    fn shared_runtime_enables_tokio_io_for_auth_login_flow() -> Result<()> {
        let runtime = super::shared_runtime()?;
        let listener = runtime.block_on(async { TcpListener::bind("127.0.0.1:0").await })?;

        let local_addr = listener.local_addr()?;
        assert!(local_addr.port() > 0);

        Ok(())
    }

    #[test]
    fn dispatcher_preserves_actionable_errors() {
        let error = run_auth_subcommand_with(
            AuthRequest {
                subcommand: AuthSubcommand::Login {
                    format: AuthFormat::Text,
                },
            },
            |_| Err(anyhow!("login failed. Try: rerun login.")),
            |_, _| Ok("renew".to_string()),
            |_| Ok("logout".to_string()),
            |_| Ok("status".to_string()),
        )
        .expect_err("login should fail");

        assert!(error.to_string().contains("Try:"));
    }
}
