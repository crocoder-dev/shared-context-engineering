use std::fs;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde_json::json;

use crate::services::auth::{self, DeviceAuthFlowResult};
use crate::services::output_format::OutputFormat;
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
    let client_id = std::env::var("WORKOS_CLIENT_ID").unwrap_or_default();
    let runtime = shared_runtime()?;
    let result = runtime
        .block_on(auth::start_device_auth_flow(
            &client,
            auth::WORKOS_DEFAULT_BASE_URL,
            &client_id,
        ))
        .map_err(|error| {
            anyhow!(with_try_guidance(
                error.to_string(),
                "verify WORKOS_CLIENT_ID, confirm network access, and rerun 'sce auth login'."
            ))
        })?;

    render_login_result(&result, format)
}

pub fn run_logout(format: AuthFormat) -> Result<String> {
    let token_path = token_storage::token_file_path()?;
    let deleted = delete_tokens_at_path(&token_path)?;
    render_logout_result(deleted, format)
}

pub fn run_status(format: AuthFormat) -> Result<String> {
    let report = match token_storage::load_tokens()? {
        Some(tokens) => build_authenticated_status_report(&tokens)?,
        None => AuthStatusReport {
            authentication_state: "unauthenticated",
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
        .enable_time()
        .build()
        .context("failed to create auth command runtime. Try: rerun the command; if the issue persists, verify the local Tokio runtime environment.")?;

    Ok(AUTH_RUNTIME.get_or_init(|| runtime))
}

fn build_authenticated_status_report(tokens: &StoredTokens) -> Result<AuthStatusReport> {
    let now_unix_seconds = current_unix_timestamp_seconds()?;
    let expires_at_unix_seconds = tokens
        .stored_at_unix_seconds
        .saturating_add(tokens.expires_in);
    let seconds_until_expiry = expires_at_unix_seconds as i64 - now_unix_seconds as i64;

    Ok(AuthStatusReport {
        authentication_state: "authenticated",
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
        AuthFormat::Text => Ok(format!(
            "Authentication succeeded. User code: {}\nVerification URL: {}\nVerification URL (complete): {}\nToken type: {}\nExpires at (unix): {}",
            result.authorization.user_code,
            result.authorization.verification_uri,
            result
                .authorization
                .verification_uri_complete
                .as_deref()
                .unwrap_or("(not provided)"),
            result.stored_tokens.token_type,
            expires_at_unix_seconds,
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

fn render_logout_result(deleted: bool, format: AuthFormat) -> Result<String> {
    match format {
        AuthFormat::Text => Ok(if deleted {
            "Removed stored WorkOS credentials.".to_string()
        } else {
            "No stored WorkOS credentials were found.".to_string()
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
                return Ok("Authentication status: unauthenticated\nStored credentials: none".to_string());
            }

            Ok(format!(
                "Authentication status: {}\nStored credentials: present\nToken expired: {}\nSeconds until expiry: {}\nExpires at (unix): {}\nToken type: {}\nScope: {}",
                report.authentication_state,
                report.token_expired.unwrap_or(false),
                report.seconds_until_expiry.unwrap_or_default(),
                report.expires_at_unix_seconds.unwrap_or_default(),
                report.token_type.as_deref().unwrap_or("(unknown)"),
                report.scope.as_deref().unwrap_or("(none)"),
            ))
        }
        AuthFormat::Json => serde_json::to_string_pretty(&json!({
            "status": "ok",
            "command": NAME,
            "subcommand": "status",
            "authentication_state": report.authentication_state,
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

fn delete_tokens_at_path(path: &Path) -> Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(anyhow!(
            "Failed to remove stored authentication tokens at '{}': {}. Try: verify file permissions for the auth state directory and rerun 'sce auth logout'.",
            path.display(),
            error
        )),
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

#[cfg(test)]
mod tests {
    use anyhow::{anyhow, Result};
    use serde_json::Value;

    use crate::test_support::TestTempDir;

    use super::{
        build_authenticated_status_report, delete_tokens_at_path, render_login_result,
        render_logout_result, render_status_result, run_auth_subcommand_with, with_try_guidance,
        AuthFormat, AuthRequest, AuthStatusReport, AuthSubcommand,
    };
    use crate::services::auth::{DeviceAuthFlowResult, DeviceAuthorizationResponse};
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
            |_| Ok("logout".to_string()),
            |_| Ok("status".to_string()),
        )?;

        assert_eq!(output, "status");
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
    fn logout_json_output_reports_removal_state() -> Result<()> {
        let output = render_logout_result(true, AuthFormat::Json)?;
        let parsed: Value = serde_json::from_str(&output)?;

        assert_eq!(parsed["subcommand"], "logout");
        assert_eq!(parsed["credentials_removed"], true);
        Ok(())
    }

    #[test]
    fn status_text_output_reports_unauthenticated_state() -> Result<()> {
        let output = render_status_result(
            &AuthStatusReport {
                authentication_state: "unauthenticated",
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
        Ok(())
    }

    #[test]
    fn status_json_output_reports_expiry_fields() -> Result<()> {
        let report = build_authenticated_status_report(&StoredTokens {
            access_token: "access-token".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: 3600,
            refresh_token: "refresh-token".to_string(),
            scope: Some("openid profile".to_string()),
            stored_at_unix_seconds: super::current_unix_timestamp_seconds()? - 60,
        })?;

        let output = render_status_result(&report, AuthFormat::Json)?;
        let parsed: Value = serde_json::from_str(&output)?;

        assert_eq!(parsed["subcommand"], "status");
        assert_eq!(parsed["authentication_state"], "authenticated");
        assert!(parsed["has_stored_credentials"].as_bool().unwrap_or(false));
        assert!(parsed["seconds_until_expiry"].as_i64().is_some());
        Ok(())
    }

    #[test]
    fn delete_tokens_at_path_removes_existing_file() -> Result<()> {
        let temp_dir = TestTempDir::new("auth-command-delete")?;
        let token_path = temp_dir.path().join("tokens.json");
        std::fs::write(&token_path, "{}")?;

        let deleted = delete_tokens_at_path(&token_path)?;

        assert!(deleted);
        assert!(!token_path.exists());
        Ok(())
    }

    #[test]
    fn delete_tokens_at_path_returns_false_when_missing() -> Result<()> {
        let temp_dir = TestTempDir::new("auth-command-missing")?;
        let token_path = temp_dir.path().join("tokens.json");

        let deleted = delete_tokens_at_path(&token_path)?;

        assert!(!deleted);
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
    fn dispatcher_preserves_actionable_errors() {
        let error = run_auth_subcommand_with(
            AuthRequest {
                subcommand: AuthSubcommand::Login {
                    format: AuthFormat::Text,
                },
            },
            |_| Err(anyhow!("login failed. Try: rerun login.")),
            |_| Ok("logout".to_string()),
            |_| Ok("status".to_string()),
        )
        .expect_err("login should fail");

        assert!(error.to_string().contains("Try:"));
    }
}
