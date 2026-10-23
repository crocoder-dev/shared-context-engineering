use std::future::Future;

use serde_json::{json, Value};

use crate::services::config::{
    AuthMode, ConfigPathSource, ResolvedAccessPolicy, TelemetryDisabledReason,
    TelemetryPolicyState, ValueSource,
};

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
}

impl LoginState {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotRequired => LOGIN_STATE_NOT_REQUIRED,
            Self::LoggedIn => LOGIN_STATE_LOGGED_IN,
            Self::NotLoggedIn => LOGIN_STATE_NOT_LOGGED_IN,
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
        authorized: F,
    ) -> Self
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = bool>,
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
        let login = if policy.auth_mode.value != AuthMode::Mandatory {
            LoginState::NotRequired
        } else if authorized().await {
            LoginState::LoggedIn
        } else {
            LoginState::NotLoggedIn
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
        (self.login == LoginState::NotLoggedIn).then(|| DoctorProblem {
            kind: ProblemKind::SceLoginRequired,
            category: ProblemCategory::GlobalState,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!(
                "auth.mode: mandatory is set in the {}, but no valid SCE login was found.",
                source_label(self.auth_mode_source)
            ),
            remediation: LOGIN_REMEDIATION.to_string(),
            next_action: NEXT_ACTION_AUTH_LOGIN,
            scope: None,
            mutation_scope_target: None,
        })
    }

    pub(super) fn login_json(self) -> Value {
        json!({
            "state": self.login.as_str(),
            "auth_mode": self.auth_mode.as_str(),
            "source": self.auth_mode_source.as_str(),
            "config_source": self.auth_mode_source.config_source().map(ConfigPathSource::as_str),
        })
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
