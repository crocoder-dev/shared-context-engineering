use serde_json::{json, Value};

use crate::services::error::{CredentialStorePlatform, CURRENT_CREDENTIAL_STORE_PLATFORM};
use crate::services::managed_telemetry::{
    ManagedTelemetryHealth, TelemetryFallbackReason, CREDENTIAL_LOOKUP_DEADLINE,
};

use super::types::{
    DoctorProblem, ProblemCategory, ProblemFixability, ProblemKind, ProblemSeverity,
};

const STATE_HEALTHY: &str = "healthy";
const STATE_NOT_CONFIGURED: &str = "not_configured";
const STATE_DISABLED: &str = "disabled";
const SEVERITY_WARNING: &str = "warning";
const NEXT_ACTION_AUTH_LOGIN: &str = "auth_login";
const NEXT_ACTION_MANUAL_STEPS: &str = "manual_steps";
pub(super) const HOOK_IMPACT: &str =
    "Claude Code hooks continue working, but telemetry export is disabled.";
const MACOS_INACCESSIBLE_SUMMARY: &str =
    "SCE cannot access its encryption key in macOS Keychain without user interaction.";
const MACOS_INACCESSIBLE_REMEDIATION: &str = "To fix:
  1. Run `sce auth login` in an interactive terminal.
  2. When macOS requests access to the SCE Keychain item,
     select \"Always Allow\".
  3. Run `sce doctor` again to verify access.";
const RELOGIN_REMEDIATION: &str = "To fix:
  1. Run `sce auth login` in an interactive terminal to provision
     a new telemetry credential.
  2. Run `sce doctor` again to verify access.";
const DATABASE_ERROR_REMEDIATION: &str = "To fix:
  1. Make sure no older `sce` executable is running; mixed versions
     cannot share the auth DB.
  2. Run `sce doctor` again.
  3. If the problem persists, check the SCE log files for
     `sce.telemetry.fallback` events with reason `database_error`.";
const PATH_RESOLUTION_REMEDIATION: &str = "To fix:
  1. Make sure HOME (and XDG_STATE_HOME, if set) point to a writable directory.
  2. Run `sce doctor` again.";
const EXPORTER_INIT_REMEDIATION: &str = "To fix:
  1. Run `sce doctor` again.
  2. If the problem persists, check the SCE log files for
     `sce.telemetry.fallback` events with reason `exporter_init_failed`.";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ManagedTelemetryDiagnostic {
    pub(super) reason: &'static str,
    pub(super) kind: &'static str,
    pub(super) summary: String,
    pub(super) remediation: String,
    pub(super) next_action: &'static str,
}

pub(super) fn diagnostic(health: ManagedTelemetryHealth) -> Option<ManagedTelemetryDiagnostic> {
    diagnostic_for_platform(health, CURRENT_CREDENTIAL_STORE_PLATFORM)
}

fn diagnostic_for_platform(
    health: ManagedTelemetryHealth,
    platform: CredentialStorePlatform,
) -> Option<ManagedTelemetryDiagnostic> {
    let ManagedTelemetryHealth::Disabled(reason) = health else {
        return None;
    };
    let (kind, summary, remediation, next_action) = match reason {
        TelemetryFallbackReason::NotConfigured => return None,
        TelemetryFallbackReason::CredentialInaccessible => {
            let (summary, remediation) = inaccessible_guidance(platform);
            (
                "managed_telemetry_credential_inaccessible",
                summary,
                remediation,
                NEXT_ACTION_AUTH_LOGIN,
            )
        }
        TelemetryFallbackReason::CredentialUnavailable => (
            "managed_telemetry_credential_unavailable",
            "Managed telemetry was configured, but its credential is no longer available in the auth DB."
                .to_string(),
            RELOGIN_REMEDIATION.to_string(),
            NEXT_ACTION_AUTH_LOGIN,
        ),
        TelemetryFallbackReason::CredentialExpired => (
            "managed_telemetry_credential_expired",
            "The managed telemetry credential has expired.".to_string(),
            RELOGIN_REMEDIATION.to_string(),
            NEXT_ACTION_AUTH_LOGIN,
        ),
        TelemetryFallbackReason::LookupTimeout => (
            "managed_telemetry_lookup_timeout",
            format!(
                "The managed telemetry credential lookup did not finish within {} seconds.",
                CREDENTIAL_LOOKUP_DEADLINE.as_secs()
            ),
            format!(
                "To fix:\n  1. Run `sce doctor` again; another SCE process may have held the auth DB.\n  2. If the timeout persists, make sure {} is responsive and unlocked.",
                platform.store_name()
            ),
            NEXT_ACTION_MANUAL_STEPS,
        ),
        TelemetryFallbackReason::DatabaseError => (
            "managed_telemetry_database_error",
            "The auth DB could not be read to load the managed telemetry credential.".to_string(),
            DATABASE_ERROR_REMEDIATION.to_string(),
            NEXT_ACTION_MANUAL_STEPS,
        ),
        TelemetryFallbackReason::PathResolutionError => (
            "managed_telemetry_path_resolution_error",
            "The SCE state directory could not be resolved, so the managed telemetry credential could not be located."
                .to_string(),
            PATH_RESOLUTION_REMEDIATION.to_string(),
            NEXT_ACTION_MANUAL_STEPS,
        ),
        TelemetryFallbackReason::ExporterInitFailed => (
            "managed_telemetry_exporter_init_failed",
            "The managed telemetry exporter could not be initialized.".to_string(),
            EXPORTER_INIT_REMEDIATION.to_string(),
            NEXT_ACTION_MANUAL_STEPS,
        ),
    };
    Some(ManagedTelemetryDiagnostic {
        reason: reason.as_str(),
        kind,
        summary,
        remediation,
        next_action,
    })
}

fn inaccessible_guidance(platform: CredentialStorePlatform) -> (String, String) {
    if platform == CredentialStorePlatform::Macos {
        return (
            MACOS_INACCESSIBLE_SUMMARY.to_string(),
            MACOS_INACCESSIBLE_REMEDIATION.to_string(),
        );
    }
    let store = platform.store_name();
    (
        format!("SCE cannot access its encryption key in {store} without user interaction."),
        format!(
            "To fix:\n  1. Make sure {store} is available and unlocked.\n  2. Run `sce auth login` in an interactive terminal.\n  3. Run `sce doctor` again to verify access."
        ),
    )
}

pub(super) fn problem(health: ManagedTelemetryHealth) -> Option<DoctorProblem> {
    diagnostic(health).map(|diagnostic| DoctorProblem {
        kind: ProblemKind::ManagedTelemetryUnavailable,
        category: ProblemCategory::GlobalState,
        severity: ProblemSeverity::Warning,
        fixability: ProblemFixability::ManualOnly,
        summary: format!("{} {HOOK_IMPACT}", diagnostic.summary),
        remediation: diagnostic.remediation,
        next_action: diagnostic.next_action,
        scope: None,
        mutation_scope_target: None,
    })
}

pub(super) fn json(health: ManagedTelemetryHealth) -> Value {
    match (health, diagnostic(health)) {
        (_, Some(diagnostic)) => json!({
            "state": STATE_DISABLED,
            "reason": diagnostic.reason,
            "kind": diagnostic.kind,
            "severity": SEVERITY_WARNING,
            "next_action": diagnostic.next_action,
        }),
        (ManagedTelemetryHealth::Healthy, None) => json!({
            "state": STATE_HEALTHY,
            "reason": null,
            "kind": null,
            "severity": null,
            "next_action": null,
        }),
        (ManagedTelemetryHealth::Disabled(reason), None) => json!({
            "state": STATE_NOT_CONFIGURED,
            "reason": reason.as_str(),
            "kind": null,
            "severity": null,
            "next_action": null,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_REASONS: [TelemetryFallbackReason; 8] = [
        TelemetryFallbackReason::NotConfigured,
        TelemetryFallbackReason::CredentialInaccessible,
        TelemetryFallbackReason::CredentialUnavailable,
        TelemetryFallbackReason::CredentialExpired,
        TelemetryFallbackReason::LookupTimeout,
        TelemetryFallbackReason::DatabaseError,
        TelemetryFallbackReason::PathResolutionError,
        TelemetryFallbackReason::ExporterInitFailed,
    ];

    fn disabled(reason: TelemetryFallbackReason) -> ManagedTelemetryHealth {
        ManagedTelemetryHealth::Disabled(reason)
    }

    #[test]
    fn not_configured_and_healthy_telemetry_report_no_problem() {
        assert_eq!(problem(ManagedTelemetryHealth::Healthy), None);
        assert_eq!(
            problem(disabled(TelemetryFallbackReason::NotConfigured)),
            None
        );
        assert_eq!(
            json(ManagedTelemetryHealth::Healthy),
            json!({
                "state": "healthy",
                "reason": null,
                "kind": null,
                "severity": null,
                "next_action": null,
            })
        );
        assert_eq!(
            json(disabled(TelemetryFallbackReason::NotConfigured)),
            json!({
                "state": "not_configured",
                "reason": "not_configured",
                "kind": null,
                "severity": null,
                "next_action": null,
            })
        );
    }

    #[test]
    fn macos_inaccessible_credential_is_an_actionable_always_allow_warning() {
        let diagnostic = diagnostic_for_platform(
            disabled(TelemetryFallbackReason::CredentialInaccessible),
            CredentialStorePlatform::Macos,
        )
        .expect("inaccessible credentials are diagnosed");
        assert_eq!(diagnostic.reason, "credential_inaccessible");
        assert_eq!(diagnostic.kind, "managed_telemetry_credential_inaccessible");
        assert_eq!(diagnostic.next_action, "auth_login");
        assert_eq!(
            diagnostic.summary,
            "SCE cannot access its encryption key in macOS Keychain without user interaction."
        );
        assert_eq!(diagnostic.remediation, MACOS_INACCESSIBLE_REMEDIATION);
        assert!(diagnostic.remediation.contains("select \"Always Allow\"."));
    }

    #[test]
    fn always_allow_is_only_recommended_for_macos_keychain_authorization() {
        for platform in [
            CredentialStorePlatform::Macos,
            CredentialStorePlatform::Linux,
            CredentialStorePlatform::Windows,
            CredentialStorePlatform::Other,
        ] {
            for reason in ALL_REASONS {
                let Some(diagnostic) = diagnostic_for_platform(disabled(reason), platform) else {
                    continue;
                };
                let text = format!("{} {}", diagnostic.summary, diagnostic.remediation);
                let expects_always_allow = platform == CredentialStorePlatform::Macos
                    && reason == TelemetryFallbackReason::CredentialInaccessible;
                assert_eq!(
                    text.contains("Always Allow"),
                    expects_always_allow,
                    "{platform:?}/{reason:?}: {text}"
                );
                if platform != CredentialStorePlatform::Macos {
                    assert!(!text.contains("macOS"), "{platform:?}/{reason:?}");
                }
            }
        }
    }

    #[test]
    fn every_disabled_reason_has_a_stable_kind_and_safe_remediation() {
        let expected = [
            (
                TelemetryFallbackReason::CredentialInaccessible,
                "managed_telemetry_credential_inaccessible",
                "auth_login",
            ),
            (
                TelemetryFallbackReason::CredentialUnavailable,
                "managed_telemetry_credential_unavailable",
                "auth_login",
            ),
            (
                TelemetryFallbackReason::CredentialExpired,
                "managed_telemetry_credential_expired",
                "auth_login",
            ),
            (
                TelemetryFallbackReason::LookupTimeout,
                "managed_telemetry_lookup_timeout",
                "manual_steps",
            ),
            (
                TelemetryFallbackReason::DatabaseError,
                "managed_telemetry_database_error",
                "manual_steps",
            ),
            (
                TelemetryFallbackReason::PathResolutionError,
                "managed_telemetry_path_resolution_error",
                "manual_steps",
            ),
            (
                TelemetryFallbackReason::ExporterInitFailed,
                "managed_telemetry_exporter_init_failed",
                "manual_steps",
            ),
        ];
        for (reason, kind, next_action) in expected {
            let health = disabled(reason);
            let problem = problem(health).expect("disabled telemetry is a problem");
            assert_eq!(problem.kind, ProblemKind::ManagedTelemetryUnavailable);
            assert_eq!(problem.severity, ProblemSeverity::Warning);
            assert_eq!(problem.fixability, ProblemFixability::ManualOnly);
            assert_eq!(problem.next_action, next_action);
            assert!(problem.summary.ends_with(HOOK_IMPACT));
            let lowered = format!("{} {}", problem.summary, problem.remediation).to_lowercase();
            for forbidden in ["delete", "regenerat", "reset", "bearer", "token:"] {
                assert!(
                    !lowered.contains(forbidden),
                    "{reason:?} mentions {forbidden}"
                );
            }
            assert_eq!(
                json(health),
                json!({
                    "state": "disabled",
                    "reason": reason.as_str(),
                    "kind": kind,
                    "severity": "warning",
                    "next_action": next_action,
                })
            );
        }
    }
}
