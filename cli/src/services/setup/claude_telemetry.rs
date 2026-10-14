use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use super::config_merge::{
    merge_claude_telemetry_settings, ClaudeTelemetryMergeOutcome, ClaudeTelemetrySkipReason,
};

const CLAUDE_CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";
const CLAUDE_SETTINGS_FILE_NAME: &str = "settings.json";
const CLAUDE_USER_DIR_NAME: &str = ".claude";
const LINUX_MANAGED_SETTINGS_PATH: &str = "/etc/claude-code/managed-settings.json";
const MACOS_MANAGED_SETTINGS_PATH: &str =
    "/Library/Application Support/ClaudeCode/managed-settings.json";
const NEW_SETTINGS_FILE_MODE: u32 = 0o600;
const TEMP_SUFFIX: &str = ".sce-tmp";

const CONFIGURED_LINE: &str =
    "Claude telemetry: configured in user-level Claude settings for native OpenTelemetry export.";
const AUTH_PENDING_LINE: &str =
    "Claude telemetry: authentication pending; run 'sce auth login' to start exporting.";
const SKIPPED_CONFLICT_LINE: &str =
    "Claude telemetry: skipped; existing Claude settings already define telemetry configuration.";
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
        ClaudeTelemetryInstallOutcome::SkippedConflict(_) => SKIPPED_CONFLICT_LINE.to_string(),
        ClaudeTelemetryInstallOutcome::SkippedInvalid => SKIPPED_INVALID_LINE.to_string(),
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Failed to read '{}'", path.display())),
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("Settings path '{}' has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create '{}'", parent.display()))?;

    let mut temp_name = path.as_os_str().to_os_string();
    temp_name.push(TEMP_SUFFIX);
    let temp_path = PathBuf::from(temp_name);

    fs::write(&temp_path, bytes)
        .with_context(|| format!("Failed to write '{}'", temp_path.display()))?;
    apply_mode(path, &temp_path)?;
    fs::rename(&temp_path, path).with_context(|| {
        let _ = fs::remove_file(&temp_path);
        format!("Failed to replace '{}'", path.display())
    })
}

#[cfg(unix)]
fn apply_mode(target: &Path, temp_path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = match fs::metadata(target) {
        Ok(metadata) => metadata.permissions().mode() & 0o7777,
        Err(_) => NEW_SETTINGS_FILE_MODE,
    };
    fs::set_permissions(temp_path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("Failed to set permissions on '{}'", temp_path.display()))
}

#[cfg(not(unix))]
fn apply_mode(_target: &Path, _temp_path: &Path) -> Result<()> {
    Ok(())
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
