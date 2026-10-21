use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use super::atomic_write::write_atomically;
use super::config_merge::{
    merge_claude_telemetry_settings, ClaudeTelemetryMergeOutcome, ClaudeTelemetrySkipReason,
};

const CLAUDE_CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";
const CLAUDE_SETTINGS_FILE_NAME: &str = "settings.json";
const CLAUDE_USER_DIR_NAME: &str = ".claude";
const LINUX_MANAGED_SETTINGS_PATH: &str = "/etc/claude-code/managed-settings.json";
const MACOS_MANAGED_SETTINGS_PATH: &str =
    "/Library/Application Support/ClaudeCode/managed-settings.json";

const CONFIGURED_LINE: &str =
    "Claude telemetry: configured in user-level Claude settings for native OpenTelemetry export.";
const AUTH_PENDING_LINE: &str =
    "Claude telemetry: authentication pending for this repository; run 'sce auth login' in this repository to start exporting.";
const SKIPPED_HELPER_LINE: &str =
    "Claude telemetry: skipped; existing Claude settings define a different otelHeadersHelper.";
const SKIPPED_DISABLED_LINE: &str =
    "Claude telemetry: skipped; existing Claude settings explicitly disable telemetry or tracing.";
const SKIPPED_PRIVACY_LINE: &str =
    "Claude telemetry: skipped; existing Claude settings explicitly opt out of repository attributes.";
const SKIPPED_ENDPOINT_LINE: &str =
    "Claude telemetry: skipped; existing Claude settings define a different OTLP endpoint.";
const SKIPPED_HEADERS_LINE: &str =
    "Claude telemetry: skipped; existing Claude settings define OTLP headers.";
const SKIPPED_PROTOCOL_LINE: &str =
    "Claude telemetry: skipped; existing Claude settings define a different OTLP protocol.";
const SKIPPED_EXPORTER_LINE: &str =
    "Claude telemetry: skipped; existing Claude settings define a different telemetry exporter.";
const SKIPPED_HELPER_SETTING_LINE: &str =
    "Claude telemetry: skipped; existing Claude settings define a different headers helper refresh interval.";
const SKIPPED_MANAGED_LINE: &str =
    "Claude telemetry: skipped; managed Claude settings own the telemetry configuration.";
const SKIPPED_INVALID_LINE: &str =
    "Claude telemetry: skipped; existing Claude settings could not be parsed and were left unchanged.";

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ClaudeTelemetryInstallOutcome {
    Configured,
    SkippedConflict(ClaudeTelemetrySkipReason),
    SkippedInvalid,
}

pub(crate) fn resolve_claude_settings_path() -> Option<PathBuf> {
    if let Some(dir) = env::var_os(CLAUDE_CONFIG_DIR_ENV).filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(dir).join(CLAUDE_SETTINGS_FILE_NAME));
    }
    dirs::home_dir().map(|home| {
        home.join(CLAUDE_USER_DIR_NAME)
            .join(CLAUDE_SETTINGS_FILE_NAME)
    })
}

pub(crate) fn managed_claude_settings_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from(MACOS_MANAGED_SETTINGS_PATH)
    } else {
        PathBuf::from(LINUX_MANAGED_SETTINGS_PATH)
    }
}

pub(crate) fn install_claude_telemetry(
    settings_path: &Path,
    managed_settings_path: &Path,
) -> Result<ClaudeTelemetryInstallOutcome> {
    let existing = read_optional(settings_path)?;
    let managed = read_optional(managed_settings_path).unwrap_or(None);
    let source_path = settings_path.display().to_string();

    match merge_claude_telemetry_settings(existing.as_deref(), managed.as_deref(), &source_path) {
        Err(_) => Ok(ClaudeTelemetryInstallOutcome::SkippedInvalid),
        Ok(ClaudeTelemetryMergeOutcome::AlreadyCurrent) => {
            Ok(ClaudeTelemetryInstallOutcome::Configured)
        }
        Ok(ClaudeTelemetryMergeOutcome::Skipped(reason)) => {
            Ok(ClaudeTelemetryInstallOutcome::SkippedConflict(reason))
        }
        Ok(ClaudeTelemetryMergeOutcome::Installed(bytes)) => {
            write_atomically(settings_path, &bytes)?;
            Ok(ClaudeTelemetryInstallOutcome::Configured)
        }
    }
}

pub(crate) fn format_claude_telemetry_report(
    outcome: &ClaudeTelemetryInstallOutcome,
    authentication_pending: bool,
) -> String {
    match outcome {
        ClaudeTelemetryInstallOutcome::Configured if authentication_pending => {
            format!("{CONFIGURED_LINE}\n{AUTH_PENDING_LINE}")
        }
        ClaudeTelemetryInstallOutcome::Configured => CONFIGURED_LINE.to_string(),
        ClaudeTelemetryInstallOutcome::SkippedConflict(reason) => {
            skipped_conflict_line(*reason).to_string()
        }
        ClaudeTelemetryInstallOutcome::SkippedInvalid => SKIPPED_INVALID_LINE.to_string(),
    }
}

fn skipped_conflict_line(reason: ClaudeTelemetrySkipReason) -> &'static str {
    match reason {
        ClaudeTelemetrySkipReason::UserHelperPresent => SKIPPED_HELPER_LINE,
        ClaudeTelemetrySkipReason::TelemetryDisabled => SKIPPED_DISABLED_LINE,
        ClaudeTelemetrySkipReason::PrivacySettingConflict => SKIPPED_PRIVACY_LINE,
        ClaudeTelemetrySkipReason::ConflictingEndpoint => SKIPPED_ENDPOINT_LINE,
        ClaudeTelemetrySkipReason::ConflictingHeaders => SKIPPED_HEADERS_LINE,
        ClaudeTelemetrySkipReason::ConflictingProtocol => SKIPPED_PROTOCOL_LINE,
        ClaudeTelemetrySkipReason::ConflictingExporter => SKIPPED_EXPORTER_LINE,
        ClaudeTelemetrySkipReason::ConflictingHelperSetting => SKIPPED_HELPER_SETTING_LINE,
        ClaudeTelemetrySkipReason::ManagedSettingsConflict => SKIPPED_MANAGED_LINE,
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Failed to read '{}'", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    const FIXTURE_TOKEN: &str = "fixture-secret-token";

    fn read_json(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn claude_telemetry_install_writes_env_and_helper_idempotently() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("claude").join("settings.json");
        let managed = dir.path().join("managed.json");

        let outcome = install_claude_telemetry(&settings, &managed).unwrap();
        assert_eq!(outcome, ClaudeTelemetryInstallOutcome::Configured);
        let first = fs::read(&settings).unwrap();
        let parsed: Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(parsed["otelHeadersHelper"], "sce auth telemetry-headers");
        assert_eq!(parsed["env"]["CLAUDE_CODE_ENABLE_TELEMETRY"], "1");
        assert!(!String::from_utf8(first.clone())
            .unwrap()
            .contains(FIXTURE_TOKEN));

        install_claude_telemetry(&settings, &managed).unwrap();
        assert_eq!(fs::read(&settings).unwrap(), first);
    }

    #[cfg(unix)]
    #[test]
    fn claude_telemetry_install_preserves_unrelated_settings_and_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        let managed = dir.path().join("managed.json");
        fs::write(
            &settings,
            serde_json::to_vec(&json!({
                "permissions": {"allow": ["Bash(ls)"]},
                "env": {"KEEP": "me"}
            }))
            .unwrap(),
        )
        .unwrap();
        fs::set_permissions(&settings, fs::Permissions::from_mode(0o640)).unwrap();

        install_claude_telemetry(&settings, &managed).unwrap();

        let parsed = read_json(&settings);
        assert_eq!(parsed["permissions"]["allow"][0], "Bash(ls)");
        assert_eq!(parsed["env"]["KEEP"], "me");
        assert_eq!(
            fs::metadata(&settings).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[cfg(unix)]
    #[test]
    fn claude_telemetry_install_creates_new_file_with_private_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        install_claude_telemetry(&settings, &dir.path().join("managed.json")).unwrap();
        assert_eq!(
            fs::metadata(&settings).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn claude_telemetry_install_skips_user_owned_helper_and_leaves_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        let original = serde_json::to_vec(&json!({"otelHeadersHelper": "my-helper"})).unwrap();
        fs::write(&settings, &original).unwrap();

        let outcome =
            install_claude_telemetry(&settings, &dir.path().join("managed.json")).unwrap();

        assert!(matches!(
            outcome,
            ClaudeTelemetryInstallOutcome::SkippedConflict(_)
        ));
        assert_eq!(fs::read(&settings).unwrap(), original);
    }

    #[test]
    fn claude_telemetry_install_refuses_malformed_json_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        fs::write(&settings, b"{ not json").unwrap();

        let outcome =
            install_claude_telemetry(&settings, &dir.path().join("managed.json")).unwrap();

        assert_eq!(outcome, ClaudeTelemetryInstallOutcome::SkippedInvalid);
        assert_eq!(fs::read(&settings).unwrap(), b"{ not json");
    }

    #[test]
    fn claude_telemetry_install_skips_on_managed_settings_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        let managed = dir.path().join("managed.json");
        fs::write(
            &managed,
            serde_json::to_vec(&json!({"otelHeadersHelper": "corp-helper"})).unwrap(),
        )
        .unwrap();

        let outcome = install_claude_telemetry(&settings, &managed).unwrap();

        assert!(matches!(
            outcome,
            ClaudeTelemetryInstallOutcome::SkippedConflict(_)
        ));
        assert!(!settings.exists());
    }

    #[test]
    fn claude_telemetry_install_keeps_explicit_opt_out_and_reports_reason() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        let original =
            serde_json::to_vec(&json!({"env": {"CLAUDE_CODE_ENABLE_TELEMETRY": "0"}})).unwrap();
        fs::write(&settings, &original).unwrap();

        let outcome =
            install_claude_telemetry(&settings, &dir.path().join("managed.json")).unwrap();

        assert_eq!(
            outcome,
            ClaudeTelemetryInstallOutcome::SkippedConflict(
                ClaudeTelemetrySkipReason::TelemetryDisabled
            )
        );
        assert_eq!(fs::read(&settings).unwrap(), original);
        let report = format_claude_telemetry_report(&outcome, false);
        assert!(report.contains("explicitly disable telemetry"));
        assert!(!report.contains("configured"));
    }

    #[test]
    fn claude_telemetry_report_marks_authentication_pending_without_helper_name() {
        let pending =
            format_claude_telemetry_report(&ClaudeTelemetryInstallOutcome::Configured, true);
        let ready =
            format_claude_telemetry_report(&ClaudeTelemetryInstallOutcome::Configured, false);

        assert!(pending.contains("authentication pending"));
        assert!(!ready.contains("authentication pending"));
        assert!(!pending.contains("telemetry-headers"));
    }
}
