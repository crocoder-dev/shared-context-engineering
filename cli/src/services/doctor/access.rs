use std::future::Future;
use std::time::Duration;

use serde_json::{json, Value};

use crate::services::config::{
    AuthMode, ConfigPathSource, ResolvedAccessPolicy, TelemetryDisabledReason,
    TelemetryPolicyState, ValueSource,
};
use crate::services::managed_telemetry::{run_with_deadline, CREDENTIAL_LOOKUP_DEADLINE};
use crate::services::token_storage::{self, StoredTokensLookup, TokenStorageError};

use super::types::{
    DoctorProblem, ProblemCategory, ProblemFixability, ProblemKind, ProblemSeverity,
};

pub(super) const LOGIN_REMEDIATION: &str = "Run 'sce auth login'.";
const NEXT_ACTION_AUTH_LOGIN: &str = "auth_login";
const LOGIN_STATE_NOT_REQUIRED: &str = "not_required";
const LOGIN_STATE_LOGGED_IN: &str = "logged_in";
const LOGIN_STATE_NOT_LOGGED_IN: &str = "not_logged_in";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LoginState {
    NotRequired,
    LoggedIn,
    NotLoggedIn,
    Unavailable(LoginUnavailableReason),
}

impl LoginState {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotRequired => LOGIN_STATE_NOT_REQUIRED,
            Self::LoggedIn => LOGIN_STATE_LOGGED_IN,
            Self::NotLoggedIn => LOGIN_STATE_NOT_LOGGED_IN,
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
        }
    }
}

pub(super) async fn inspect_login() -> LoginState {
    inspect_login_with(
        CREDENTIAL_LOOKUP_DEADLINE,
        token_storage::load_current_tokens(),
    )
    .await
}

async fn inspect_login_with<Fut>(deadline: Duration, lookup: Fut) -> LoginState
where
    Fut: Future<Output = Result<StoredTokensLookup, TokenStorageError>> + Send + 'static,
{
    match run_with_deadline(deadline, lookup).await {
        None => LoginState::Unavailable(LoginUnavailableReason::LookupTimeout),
        Some(Err(error)) => LoginState::Unavailable(match error {
            TokenStorageError::CredentialAccessUnavailable => {
                LoginUnavailableReason::CredentialInaccessible
            }
            TokenStorageError::DatabaseLocked(_) => LoginUnavailableReason::DatabaseLocked,
            TokenStorageError::Database(_) => LoginUnavailableReason::DatabaseError,
            TokenStorageError::PathResolution(_) => LoginUnavailableReason::PathResolutionError,
        }),
        Some(Ok(StoredTokensLookup::Missing)) => LoginState::NotLoggedIn,
        Some(Ok(StoredTokensLookup::SchemaNotCurrent)) => {
            LoginState::Unavailable(LoginUnavailableReason::SchemaNotCurrent)
        }
        Some(Ok(StoredTokensLookup::Found(tokens))) => {
            if !tokens.refresh_token.trim().is_empty() {
                return LoginState::LoggedIn;
            }
            if tokens.access_token.trim().is_empty() {
                return LoginState::NotLoggedIn;
            }
            match crate::services::auth::is_stored_token_expired(&tokens) {
                Ok(false) => LoginState::LoggedIn,
                Ok(true) => LoginState::NotLoggedIn,
                Err(_) => LoginState::Unavailable(LoginUnavailableReason::ClockError),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

    pub(super) fn telemetry_label(self) -> String {
        match self.telemetry {
            TelemetryPolicyState::Enabled => {
                "Managed telemetry: enabled for this repository".to_string()
            }
            TelemetryPolicyState::Disabled(reason) => {
                format!("Managed telemetry: disabled (reason: {})", reason.as_str())
            }
        }
    }

    pub(super) fn telemetry_source_label(self) -> Option<&'static str> {
        self.telemetry_source.map(source_label)
    }

    pub(super) fn login_problem(self) -> Option<DoctorProblem> {
        let (kind, severity, summary, remediation, next_action) = match self.login {
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
            kind,
            category: ProblemCategory::GlobalState,
            severity,
            fixability: ProblemFixability::ManualOnly,
            summary,
            remediation: remediation.to_string(),
            next_action,
            scope: None,
            mutation_scope_target: None,
        })
    }

    pub(super) fn login_json(self) -> Value {
        let mut result = json!({
            "state": self.login.as_str(),
            "auth_mode": self.auth_mode.as_str(),
            "source": self.auth_mode_source.as_str(),
            "config_source": self.auth_mode_source.config_source().map(ConfigPathSource::as_str),
        });
        if let LoginState::Unavailable(reason) = self.login {
            result["reason"] = json!(reason.as_str());
        }
        result
    }

    pub(super) fn telemetry_policy_json(self) -> Value {
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
