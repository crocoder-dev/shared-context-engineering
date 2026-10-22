use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use super::atomic_write::write_atomically;
use super::config_merge::{
    merge_claude_telemetry_settings, remove_claude_telemetry_settings,
    settings_configure_claude_telemetry, ClaudeTelemetryMergeOutcome, ClaudeTelemetryRemovalMerge,
    ClaudeTelemetrySkipReason,
};

const CLAUDE_CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";
const CLAUDE_SETTINGS_FILE_NAME: &str = "settings.json";
const CLAUDE_USER_DIR_NAME: &str = ".claude";
const LINUX_MANAGED_SETTINGS_PATH: &str = "/etc/claude-code/managed-settings.json";
const MACOS_MANAGED_SETTINGS_PATH: &str =
    "/Library/Application Support/ClaudeCode/managed-settings.json";

const CONFIGURED_LINE: &str =
    "Claude telemetry: configured in user-level Claude settings for native OpenTelemetry export.";
const NOT_READY_LINE: &str =
    "Claude telemetry: not ready for this repository; no valid SCE telemetry credential is authorized for it, so Claude will not export. Run 'sce auth login' in this repository with the SCE account it is enrolled under.";
const NOT_CHECKED_LINE: &str =
    "Claude telemetry: configured; repository authorization was not checked during non-interactive setup.";
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
pub(crate) const CLAUDE_TELEMETRY_DISABLED_BY_CONFIG_LINE: &str =
    "Claude telemetry: skipped; SCE telemetry is disabled by config.";

const REMOVAL_LINE_PREFIX: &str = "Claude telemetry:";
const REMOVED_TEXT: &str = "removed SCE-managed telemetry settings from user-level Claude settings";
const REMOVED_REASON_TEXT: &str = "because the global SCE config disables telemetry.";
const PRESERVED_TEXT: &str = "left user-level Claude settings";
const PRESERVED_MODIFIED_TEXT: &str = "unchanged; SCE-managed telemetry settings were modified";
const PRESERVED_HELPER_MISSING_TEXT: &str =
    "unchanged; they set the SCE OTLP endpoint without the SCE headers helper.";
const PRESERVED_INVALID_TEXT: &str =
    "unchanged; they could not be parsed. Fix the file, then rerun 'sce setup'.";
const PRESERVED_SYMLINK_TEXT: &str =
    "unchanged; the file is a symlink. Remove the SCE telemetry entries from its source directly (for example Nix or Home Manager).";
const PRESERVED_READ_ONLY_TEXT: &str =
    "unchanged; the file is read-only. Remove the SCE telemetry entries manually.";
const MANUAL_REMOVAL_TEXT: &str = "Remove the SCE telemetry entries manually.";
const STAYS_DISABLED_TEXT: &str = "SCE telemetry stays disabled by config.";
const FAILED_TEXT: &str =
    "did not remove SCE-managed telemetry settings from user-level Claude settings";
const CONSENT_WITHDRAWN_TEXT: &str = "SCE telemetry consent remains withdrawn.";
const CONCURRENT_MODIFICATION_TEXT: &str =
    "the file changed while setup was updating it; rerun 'sce setup'";
const READ_FAILED_TEXT: &str = "the file could not be read";
const WRITE_FAILED_TEXT: &str = "the file could not be replaced";
const MANAGED_NOTE_LINE: &str =
    "Claude telemetry: managed Claude settings also configure telemetry; SCE does not change them.";

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RepositoryAuthorization {
    Authorized,
    Unauthorized,
    NotChecked,
}

pub(crate) fn format_claude_telemetry_report(
    outcome: &ClaudeTelemetryInstallOutcome,
    authorization: RepositoryAuthorization,
) -> String {
    match outcome {
        ClaudeTelemetryInstallOutcome::Configured => match authorization {
            RepositoryAuthorization::Authorized => CONFIGURED_LINE.to_string(),
            RepositoryAuthorization::Unauthorized => {
                format!("{CONFIGURED_LINE}\n{NOT_READY_LINE}")
            }
            RepositoryAuthorization::NotChecked => NOT_CHECKED_LINE.to_string(),
        },
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

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ClaudeTelemetryRemovalOutcome {
    Removed,
    NotInstalled,
    PreservedModified(ClaudeTelemetrySkipReason),
    PreservedHelperMissing,
    PreservedInvalid,
    PreservedSymlink,
    PreservedReadOnly,
    Failed(String),
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ClaudeTelemetryRemovalReport {
    pub(crate) settings_path: PathBuf,
    pub(crate) outcome: ClaudeTelemetryRemovalOutcome,
    pub(crate) managed_configures_telemetry: bool,
}

pub(crate) fn remove_claude_telemetry(
    settings_path: &Path,
    managed_settings_path: &Path,
) -> ClaudeTelemetryRemovalReport {
    remove_claude_telemetry_with(settings_path, managed_settings_path, |path| fs::read(path))
}

fn remove_claude_telemetry_with<R>(
    settings_path: &Path,
    managed_settings_path: &Path,
    reread: R,
) -> ClaudeTelemetryRemovalReport
where
    R: FnOnce(&Path) -> std::io::Result<Vec<u8>>,
{
    let managed_configures_telemetry = read_optional(managed_settings_path)
        .ok()
        .flatten()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .is_some_and(|managed| settings_configure_claude_telemetry(&managed));
    ClaudeTelemetryRemovalReport {
        settings_path: settings_path.to_path_buf(),
        outcome: removal_outcome(settings_path, reread),
        managed_configures_telemetry,
    }
}

fn removal_outcome<R>(settings_path: &Path, reread: R) -> ClaudeTelemetryRemovalOutcome
where
    R: FnOnce(&Path) -> std::io::Result<Vec<u8>>,
{
    let original = match fs::read(settings_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ClaudeTelemetryRemovalOutcome::NotInstalled;
        }
        Err(error) => {
            return ClaudeTelemetryRemovalOutcome::Failed(format!("{READ_FAILED_TEXT}: {error}"));
        }
    };
    let source_path = settings_path.display().to_string();

    let bytes = match remove_claude_telemetry_settings(Some(&original), &source_path) {
        Err(_) => return ClaudeTelemetryRemovalOutcome::PreservedInvalid,
        Ok(ClaudeTelemetryRemovalMerge::NotInstalled) => {
            return ClaudeTelemetryRemovalOutcome::NotInstalled;
        }
        Ok(ClaudeTelemetryRemovalMerge::Modified(reason)) => {
            return ClaudeTelemetryRemovalOutcome::PreservedModified(reason);
        }
        Ok(ClaudeTelemetryRemovalMerge::HelperMissing) => {
            return ClaudeTelemetryRemovalOutcome::PreservedHelperMissing;
        }
        Ok(ClaudeTelemetryRemovalMerge::Removed(bytes)) => bytes,
    };

    let metadata = match fs::symlink_metadata(settings_path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return ClaudeTelemetryRemovalOutcome::Failed(format!("{READ_FAILED_TEXT}: {error}"));
        }
    };
    if metadata.file_type().is_symlink() {
        return ClaudeTelemetryRemovalOutcome::PreservedSymlink;
    }
    if is_read_only(&metadata) {
        return ClaudeTelemetryRemovalOutcome::PreservedReadOnly;
    }
    match reread(settings_path) {
        Ok(current) if current == original => {}
        Ok(_) => {
            return ClaudeTelemetryRemovalOutcome::Failed(CONCURRENT_MODIFICATION_TEXT.to_string());
        }
        Err(error) => {
            return ClaudeTelemetryRemovalOutcome::Failed(format!("{READ_FAILED_TEXT}: {error}"));
        }
    }

    match write_atomically(settings_path, &bytes) {
        Ok(()) => ClaudeTelemetryRemovalOutcome::Removed,
        Err(error) => {
            ClaudeTelemetryRemovalOutcome::Failed(format!("{WRITE_FAILED_TEXT}: {error:#}"))
        }
    }
}

#[cfg(unix)]
fn is_read_only(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o200 == 0
}

#[cfg(not(unix))]
fn is_read_only(metadata: &fs::Metadata) -> bool {
    metadata.permissions().readonly()
}

pub(crate) fn format_claude_telemetry_removal_report(
    report: &ClaudeTelemetryRemovalReport,
    claude_selected: bool,
) -> Option<String> {
    let path = report.settings_path.display();
    let line = match &report.outcome {
        ClaudeTelemetryRemovalOutcome::Removed => {
            format!("{REMOVAL_LINE_PREFIX} {REMOVED_TEXT} '{path}' {REMOVED_REASON_TEXT}")
        }
        ClaudeTelemetryRemovalOutcome::NotInstalled if claude_selected => {
            CLAUDE_TELEMETRY_DISABLED_BY_CONFIG_LINE.to_string()
        }
        ClaudeTelemetryRemovalOutcome::NotInstalled => return None,
        ClaudeTelemetryRemovalOutcome::PreservedModified(reason) => format!(
            "{REMOVAL_LINE_PREFIX} {PRESERVED_TEXT} '{path}' {PRESERVED_MODIFIED_TEXT} ({}). {MANUAL_REMOVAL_TEXT} {STAYS_DISABLED_TEXT}",
            modified_reason_text(*reason)
        ),
        ClaudeTelemetryRemovalOutcome::PreservedHelperMissing => format!(
            "{REMOVAL_LINE_PREFIX} {PRESERVED_TEXT} '{path}' {PRESERVED_HELPER_MISSING_TEXT} {MANUAL_REMOVAL_TEXT} {STAYS_DISABLED_TEXT}"
        ),
        ClaudeTelemetryRemovalOutcome::PreservedInvalid => format!(
            "{REMOVAL_LINE_PREFIX} {PRESERVED_TEXT} '{path}' {PRESERVED_INVALID_TEXT} {STAYS_DISABLED_TEXT}"
        ),
        ClaudeTelemetryRemovalOutcome::PreservedSymlink => format!(
            "{REMOVAL_LINE_PREFIX} {PRESERVED_TEXT} '{path}' {PRESERVED_SYMLINK_TEXT} {STAYS_DISABLED_TEXT}"
        ),
        ClaudeTelemetryRemovalOutcome::PreservedReadOnly => format!(
            "{REMOVAL_LINE_PREFIX} {PRESERVED_TEXT} '{path}' {PRESERVED_READ_ONLY_TEXT} {STAYS_DISABLED_TEXT}"
        ),
        ClaudeTelemetryRemovalOutcome::Failed(detail) => format!(
            "{REMOVAL_LINE_PREFIX} {FAILED_TEXT} '{path}'; {detail}. {MANUAL_REMOVAL_TEXT} {CONSENT_WITHDRAWN_TEXT}"
        ),
    };
    if report.managed_configures_telemetry {
        Some(format!("{line}\n{MANAGED_NOTE_LINE}"))
    } else {
        Some(line)
    }
}

fn modified_reason_text(reason: ClaudeTelemetrySkipReason) -> &'static str {
    match reason {
        ClaudeTelemetrySkipReason::UserHelperPresent => "different otelHeadersHelper",
        ClaudeTelemetrySkipReason::TelemetryDisabled => "telemetry or tracing disabled",
        ClaudeTelemetrySkipReason::PrivacySettingConflict => "repository attributes opted out",
        ClaudeTelemetrySkipReason::ConflictingEndpoint => "different OTLP endpoint",
        ClaudeTelemetrySkipReason::ConflictingHeaders => "OTLP headers defined",
        ClaudeTelemetrySkipReason::ConflictingProtocol => "different OTLP protocol",
        ClaudeTelemetrySkipReason::ConflictingExporter => "different telemetry exporter",
        ClaudeTelemetrySkipReason::ConflictingHelperSetting => {
            "different headers helper refresh interval"
        }
        ClaudeTelemetrySkipReason::ManagedSettingsConflict => "managed settings conflict",
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
        let report =
            format_claude_telemetry_report(&outcome, RepositoryAuthorization::Unauthorized);
        assert!(report.contains("explicitly disable telemetry"));
        assert!(!report.contains("configured"));
    }
}
