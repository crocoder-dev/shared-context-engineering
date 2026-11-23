use std::future::Future;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::services::auth::{self, AuthError, TokenResponse};
use crate::services::config::{
    self, AuthMode, ConfigPathSource, ResolvedAccessPolicy, TelemetryDisabledReason,
    TelemetryPolicyState, ValueSource,
};
use crate::services::db::encryption_key::CredentialAccess;
use crate::services::managed_telemetry::{run_with_deadline, CREDENTIAL_LOOKUP_DEADLINE};
use crate::services::token_storage::{self, StoredTokens, StoredTokensLookup, TokenStorageError};

use crate::services::lifecycle::HealthProblem;

use super::types::{
    DoctorProblem, ProblemCategory, ProblemFixability, ProblemKind, ProblemSeverity,
};

pub(super) const LOGIN_REMEDIATION: &str = "Run 'sce auth login'.";
const NEXT_ACTION_AUTH_LOGIN: &str = "auth_login";
const LOGIN_STATE_NOT_REQUIRED: &str = "not_required";
const LOGIN_STATE_LOGGED_IN: &str = "logged_in";
const LOGIN_STATE_NOT_LOGGED_IN: &str = "not_logged_in";
const SESSION_CONFIRMATION_GRACE: Duration = Duration::from_hours(24);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum LoginState {
    NotRequired,
    LoggedIn,
    NotLoggedIn,
    SessionRejected(String),
    Unavailable(LoginUnavailableReason),
}

impl LoginState {
    fn as_str(&self) -> &'static str {
        match self {
            Self::NotRequired => LOGIN_STATE_NOT_REQUIRED,
            Self::LoggedIn => LOGIN_STATE_LOGGED_IN,
            Self::NotLoggedIn | Self::SessionRejected(_) => LOGIN_STATE_NOT_LOGGED_IN,
            Self::Unavailable(_) => "unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LoginUnavailableReason {
    CredentialInaccessible,
    DatabaseLocked,
    DatabaseError,
    PathResolutionError,
    SchemaNotCurrent,
    LookupTimeout,
    ClockError,
    SessionUnreachable,
    AuthConfigUnavailable,
}

impl LoginUnavailableReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::CredentialInaccessible => "credential_inaccessible",
            Self::DatabaseLocked => "database_locked",
            Self::DatabaseError => "database_error",
            Self::PathResolutionError => "path_resolution_error",
            Self::SchemaNotCurrent => "schema_not_current",
            Self::LookupTimeout => "lookup_timeout",
            Self::ClockError => "clock_error",
            Self::SessionUnreachable => "session_unreachable",
            Self::AuthConfigUnavailable => "auth_config_unavailable",
        }
    }

    fn guidance(self) -> (&'static str, &'static str) {
        match self {
            Self::CredentialInaccessible => (
                "SCE cannot read the saved session because credential store access is unavailable.",
                "Unlock the system credential store; run 'sce auth login' in an interactive terminal to authorize access, then rerun 'sce doctor'.",
            ),
            Self::DatabaseLocked => (
                "SCE cannot read the saved session because authentication storage is locked.",
                "Wait for other SCE processes to exit, then rerun 'sce doctor'.",
            ),
            Self::DatabaseError => (
                "SCE cannot read the saved session because authentication storage is unavailable.",
                "Check authentication storage access and the SCE logs, then rerun 'sce doctor'.",
            ),
            Self::PathResolutionError => (
                "SCE cannot locate authentication storage.",
                "Check HOME and XDG_STATE_HOME, then rerun 'sce doctor'.",
            ),
            Self::SchemaNotCurrent => (
                "SCE cannot inspect the saved session until authentication storage is upgraded.",
                "Run 'sce auth login' to upgrade authentication storage and reuse the saved session, then rerun 'sce doctor'.",
            ),
            Self::LookupTimeout => (
                "The saved session lookup did not finish in time.",
                "Rerun 'sce doctor'; if this persists, check credential store access and other SCE processes.",
            ),
            Self::ClockError => (
                "SCE cannot determine the saved session status because the system clock is invalid.",
                "Correct the system clock, then rerun 'sce doctor'.",
            ),
            Self::SessionUnreachable => (
                "SCE could not reach WorkOS to confirm the saved session, and it has not been confirmed in the last 24 hours.",
                "Check network connectivity, then rerun 'sce doctor'.",
            ),
            Self::AuthConfigUnavailable => (
                "SCE cannot confirm the saved session because the WorkOS client ID is not configured.",
                "Check the SCE auth configuration with 'sce config show', then rerun 'sce doctor'.",
            ),
        }
    }
}

#[derive(Debug)]
enum SessionRenewalError {
    Auth(AuthError),
    Storage(TokenStorageError),
}

pub(super) async fn inspect_login(repository_root: &Path) -> LoginState {
    let client_id = config::resolve_auth_runtime_config(repository_root)
        .ok()
        .and_then(|config| config.workos_client_id.value)
        .unwrap_or_default();
    inspect_login_with(
        CREDENTIAL_LOOKUP_DEADLINE,
        token_storage::load_current_tokens,
        |refresh_token| async move {
            auth::renew_stored_token_from_refresh_token(
                &reqwest::Client::new(),
                auth::WORKOS_DEFAULT_BASE_URL,
                &client_id,
                &refresh_token,
            )
            .await
        },
        |token| async move {
            token_storage::save_tokens(&token, CredentialAccess::NonInteractive).await
        },
    )
    .await
}

async fn inspect_login_with<L, LF, R, RF, S, SF>(
    deadline: Duration,
    lookup: L,
    refresh: R,
    save: S,
) -> LoginState
where
    L: Fn() -> LF,
    LF: Future<Output = Result<StoredTokensLookup, TokenStorageError>> + Send + 'static,
    R: FnOnce(String) -> RF,
    RF: Future<Output = Result<TokenResponse, AuthError>> + Send + 'static,
    S: FnOnce(TokenResponse) -> SF + Send + 'static,
    SF: Future<Output = Result<StoredTokens, TokenStorageError>> + Send + 'static,
{
    let tokens = match load_session(deadline, lookup()).await {
        Ok(tokens) => tokens,
        Err(state) => return state,
    };
    if !tokens.access_token.trim().is_empty() {
        match auth::is_stored_token_expired(&tokens) {
            Ok(false) => return LoginState::LoggedIn,
            Ok(true) => {}
            Err(_) => return LoginState::Unavailable(LoginUnavailableReason::ClockError),
        }
    }
    if tokens.refresh_token.trim().is_empty() {
        return LoginState::NotLoggedIn;
    }
    // Refresh and save run as one unit: WorkOS rotates the refresh token, so a
    // refresh whose result is not saved invalidates the stored session.
    let refreshed = refresh(tokens.refresh_token.clone());
    let renewal = async move {
        let token = refreshed.await.map_err(SessionRenewalError::Auth)?;
        save(token).await.map_err(SessionRenewalError::Storage)
    };
    match run_with_deadline(deadline, renewal).await {
        Some(Ok(_)) => LoginState::LoggedIn,
        Some(Err(SessionRenewalError::Auth(AuthError::Unauthorized(reason)))) => {
            if session_rotated_elsewhere(deadline, lookup(), &tokens.refresh_token).await {
                LoginState::LoggedIn
            } else {
                LoginState::SessionRejected(rejection_reason(&reason))
            }
        }
        Some(Err(SessionRenewalError::Auth(AuthError::MissingClientId))) => {
            LoginState::Unavailable(LoginUnavailableReason::AuthConfigUnavailable)
        }
        Some(Err(SessionRenewalError::Storage(error))) => {
            LoginState::Unavailable(storage_unavailable_reason(&error))
        }
        None | Some(Err(SessionRenewalError::Auth(_))) => unreachable_session_state(&tokens),
    }
}

async fn load_session<Fut>(deadline: Duration, lookup: Fut) -> Result<StoredTokens, LoginState>
where
    Fut: Future<Output = Result<StoredTokensLookup, TokenStorageError>> + Send + 'static,
{
    match run_with_deadline(deadline, lookup).await {
        None => Err(LoginState::Unavailable(
            LoginUnavailableReason::LookupTimeout,
        )),
        Some(Err(error)) => Err(LoginState::Unavailable(storage_unavailable_reason(&error))),
        Some(Ok(StoredTokensLookup::Missing)) => Err(LoginState::NotLoggedIn),
        Some(Ok(StoredTokensLookup::SchemaNotCurrent)) => Err(LoginState::Unavailable(
            LoginUnavailableReason::SchemaNotCurrent,
        )),
        Some(Ok(StoredTokensLookup::Found(tokens))) => Ok(tokens),
    }
}

async fn session_rotated_elsewhere<Fut>(
    deadline: Duration,
    lookup: Fut,
    rejected_refresh_token: &str,
) -> bool
where
    Fut: Future<Output = Result<StoredTokensLookup, TokenStorageError>> + Send + 'static,
{
    load_session(deadline, lookup)
        .await
        .is_ok_and(|tokens| tokens.refresh_token != rejected_refresh_token)
}

/// Drops the trailing `Try: ...` advice from an `AuthError::Unauthorized`
/// message, because the login problem carries its own remediation.
fn rejection_reason(message: &str) -> String {
    message
        .split_once(" Try:")
        .map_or(message, |(reason, _)| reason)
        .trim()
        .trim_end_matches('.')
        .to_string()
}

fn unreachable_session_state(tokens: &StoredTokens) -> LoginState {
    let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return LoginState::Unavailable(LoginUnavailableReason::ClockError);
    };
    let confirmed_ago = now.as_secs().saturating_sub(tokens.stored_at_unix_seconds);
    if confirmed_ago < SESSION_CONFIRMATION_GRACE.as_secs() {
        LoginState::LoggedIn
    } else {
        LoginState::Unavailable(LoginUnavailableReason::SessionUnreachable)
    }
}

fn storage_unavailable_reason(error: &TokenStorageError) -> LoginUnavailableReason {
    match error {
        TokenStorageError::CredentialAccessUnavailable => {
            LoginUnavailableReason::CredentialInaccessible
        }
        TokenStorageError::DatabaseLocked(_) => LoginUnavailableReason::DatabaseLocked,
        TokenStorageError::Database(_) => LoginUnavailableReason::DatabaseError,
        TokenStorageError::PathResolution(_) => LoginUnavailableReason::PathResolutionError,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DoctorAccess {
    pub(super) auth_mode: AuthMode,
    pub(super) auth_mode_source: ValueSource,
    pub(super) telemetry: TelemetryPolicyState,
    pub(super) telemetry_source: Option<ValueSource>,
    pub(super) login: LoginState,
}

impl DoctorAccess {
    pub(super) async fn inspect<F, Fut>(
        policy: Option<&ResolvedAccessPolicy>,
        inspect_login: F,
    ) -> Self
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = LoginState>,
    {
        let Some(policy) = policy else {
            return Self {
                auth_mode: AuthMode::Optional,
                auth_mode_source: ValueSource::Default,
                telemetry: TelemetryPolicyState::Disabled(TelemetryDisabledReason::InvalidConfig),
                telemetry_source: None,
                login: LoginState::NotRequired,
            };
        };
        let login = if policy.auth_mode.value == AuthMode::Mandatory {
            inspect_login().await
        } else {
            LoginState::NotRequired
        };
        let telemetry_source = match policy.telemetry {
            TelemetryPolicyState::Disabled(TelemetryDisabledReason::InvalidConfig) => None,
            TelemetryPolicyState::Disabled(TelemetryDisabledReason::AuthDisabled) => {
                Some(policy.auth_mode.source)
            }
            _ => Some(policy.telemetry_enabled.source),
        };
        Self {
            auth_mode: policy.auth_mode.value,
            auth_mode_source: policy.auth_mode.source,
            telemetry: policy.telemetry,
            telemetry_source,
            login,
        }
    }

    pub(super) fn telemetry_label(&self) -> String {
        match self.telemetry {
            TelemetryPolicyState::Enabled => {
                "Managed telemetry: enabled for this repository".to_string()
            }
            TelemetryPolicyState::Disabled(reason) => {
                format!("Managed telemetry: disabled (reason: {})", reason.as_str())
            }
        }
    }

    pub(super) fn telemetry_source_label(&self) -> Option<&'static str> {
        self.telemetry_source.map(source_label)
    }

    pub(super) fn login_problem(&self) -> Option<DoctorProblem> {
        let (kind, severity, summary, remediation, next_action) = match &self.login {
            LoginState::NotRequired | LoginState::LoggedIn => return None,
            LoginState::NotLoggedIn => (
                ProblemKind::SceLoginRequired,
                ProblemSeverity::Error,
                format!(
                    "auth.mode: mandatory is set in the {}, but no saved SCE session is available.",
                    source_label(self.auth_mode_source)
                ),
                LOGIN_REMEDIATION,
                NEXT_ACTION_AUTH_LOGIN,
            ),
            LoginState::SessionRejected(reason) => (
                ProblemKind::SceLoginRequired,
                ProblemSeverity::Error,
                format!(
                    "auth.mode: mandatory is set in the {}, but WorkOS rejected the saved SCE session: {reason}.",
                    source_label(self.auth_mode_source)
                ),
                LOGIN_REMEDIATION,
                NEXT_ACTION_AUTH_LOGIN,
            ),
            LoginState::Unavailable(reason) => {
                let (summary, remediation) = reason.guidance();
                (
                    ProblemKind::SceLoginUnavailable,
                    ProblemSeverity::Warning,
                    summary.to_string(),
                    remediation,
                    "manual_steps",
                )
            }
        };
        Some(DoctorProblem {
            health: HealthProblem {
                kind,
                category: ProblemCategory::GlobalState,
                severity,
                fixability: ProblemFixability::ManualOnly,
                summary,
                remediation: remediation.to_string(),
                next_action,
            },
            scope: None,
            mutation_scope_target: None,
        })
    }

    pub(super) fn login_json(&self) -> Value {
        let mut result = json!({
            "state": self.login.as_str(),
            "auth_mode": self.auth_mode.as_str(),
            "source": self.auth_mode_source.as_str(),
            "config_source": self.auth_mode_source.config_source().map(ConfigPathSource::as_str),
        });
        if let LoginState::Unavailable(reason) = &self.login {
            result["reason"] = json!(reason.as_str());
        }
        result
    }

    pub(super) fn telemetry_policy_json(&self) -> Value {
        json!({
            "state": self.telemetry.as_str(),
            "reason": self.telemetry.disabled_reason().map(TelemetryDisabledReason::as_str),
            "source": self.telemetry_source.map(ValueSource::as_str),
            "config_source": self
                .telemetry_source
                .and_then(ValueSource::config_source)
                .map(ConfigPathSource::as_str),
        })
    }
}

fn source_label(source: ValueSource) -> &'static str {
    match source {
        ValueSource::ConfigFile(ConfigPathSource::DefaultDiscoveredLocal) => {
            "repository SCE config (`.sce/config.json`)"
        }
        ValueSource::ConfigFile(ConfigPathSource::DefaultDiscoveredGlobal) => "global SCE config",
        ValueSource::ConfigFile(ConfigPathSource::Flag) => "SCE config file passed with `--config`",
        ValueSource::ConfigFile(ConfigPathSource::Env) => {
            "SCE config file named by `SCE_CONFIG_FILE`"
        }
        ValueSource::Flag => "command-line flag",
        ValueSource::Env => "environment",
        ValueSource::Default => "default",
    }
}

#[cfg(test)]
mod tests;
