use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde_json::{Map, Value};

use crate::services::agent_trace::sce_config_schema_url;
use crate::services::config::schema::{parse_file_config, CONFIG_SCHEMA_DECLARATION_KEY};
use crate::services::config::{AuthMode, ConfigPathSource};

use super::atomic_write::write_atomically;

const AUTH_KEY: &str = "auth";
const AUTH_MODE_KEY: &str = "mode";
const TELEMETRY_KEY: &str = "telemetry";
const TELEMETRY_ENABLED_KEY: &str = "enabled";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccessPreferences {
    Missing,
    Valid {
        auth_mode: Option<AuthMode>,
        telemetry_enabled: Option<bool>,
    },
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccessWriteOutcome {
    Unchanged,
    Written,
}

#[derive(Debug)]
pub(crate) enum RepositoryAccessPreferencesError {
    InvalidRepositoryConfig { path: PathBuf, reason: String },
    ConcurrentModification { path: PathBuf },
    SymlinkManaged { path: PathBuf },
    Io(anyhow::Error),
}

impl fmt::Display for RepositoryAccessPreferencesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRepositoryConfig { path, reason } => write!(
                f,
                "Repository SCE config '{}' is invalid and was left unchanged: {reason}",
                path.display()
            ),
            Self::ConcurrentModification { path } => write!(
                f,
                "Repository SCE config '{}' changed while setup was updating it and was left unchanged.",
                path.display()
            ),
            Self::SymlinkManaged { path } => write!(
                f,
                "Repository SCE config '{}' is a symlink and was left unchanged; SCE cannot modify a symlink-managed repository config. Update the file's source directly.",
                path.display()
            ),
            Self::Io(error) => write!(f, "{error:#}"),
        }
    }
}

impl std::error::Error for RepositoryAccessPreferencesError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConfigLayer {
    Global,
    Repository,
}

impl ConfigLayer {
    fn path_source(self) -> ConfigPathSource {
        match self {
            Self::Global => ConfigPathSource::DefaultDiscoveredGlobal,
            Self::Repository => ConfigPathSource::DefaultDiscoveredLocal,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Global => "global SCE config",
            Self::Repository => "repository SCE config",
        }
    }

    fn bootstrap_value(self) -> Value {
        match self {
            Self::Global => Value::Object(Map::from_iter([(
                CONFIG_SCHEMA_DECLARATION_KEY.to_string(),
                Value::String(sce_config_schema_url()),
            )])),
            Self::Repository => serde_json::from_str(&super::repo_local_config_bootstrap_payload())
                .expect("repo-local config bootstrap payload must be valid JSON"),
        }
    }
}

#[derive(Debug)]
enum WriteFailure {
    Invalid(String),
    ConcurrentModification,
    SymlinkManaged,
    Io(anyhow::Error),
}

pub(crate) fn read_global_access_preferences(path: &Path) -> anyhow::Result<AccessPreferences> {
    read_access_preferences(path, ConfigLayer::Global)
}

pub(crate) fn read_repository_access_preferences(path: &Path) -> anyhow::Result<AccessPreferences> {
    read_access_preferences(path, ConfigLayer::Repository)
}

fn read_access_preferences(path: &Path, layer: ConfigLayer) -> anyhow::Result<AccessPreferences> {
    let Some(bytes) = read_optional(path, layer)? else {
        return Ok(AccessPreferences::Missing);
    };
    let Ok(raw) = String::from_utf8(bytes) else {
        return Ok(AccessPreferences::Invalid);
    };
    let Ok(config) = parse_file_config(&raw, path, layer.path_source()) else {
        return Ok(AccessPreferences::Invalid);
    };

    Ok(AccessPreferences::Valid {
        auth_mode: config.auth_mode.map(|setting| setting.value),
        telemetry_enabled: config.telemetry_enabled.map(|setting| setting.value),
    })
}

pub(crate) fn persist_repository_access_preferences(
    path: &Path,
    auth_mode: Option<AuthMode>,
    telemetry_enabled: Option<bool>,
) -> Result<AccessWriteOutcome, RepositoryAccessPreferencesError> {
    persist_repository_access_preferences_with(path, auth_mode, telemetry_enabled, |path| {
        read_optional(path, ConfigLayer::Repository)
    })
}

fn persist_repository_access_preferences_with<R>(
    path: &Path,
    auth_mode: Option<AuthMode>,
    telemetry_enabled: Option<bool>,
    reread: R,
) -> Result<AccessWriteOutcome, RepositoryAccessPreferencesError>
where
    R: FnOnce(&Path) -> anyhow::Result<Option<Vec<u8>>>,
{
    persist_access_preferences_with(
        path,
        ConfigLayer::Repository,
        auth_mode,
        telemetry_enabled,
        reread,
    )
    .map_err(|failure| match failure {
        WriteFailure::Invalid(reason) => {
            RepositoryAccessPreferencesError::InvalidRepositoryConfig {
                path: path.to_path_buf(),
                reason,
            }
        }
        WriteFailure::ConcurrentModification => {
            RepositoryAccessPreferencesError::ConcurrentModification {
                path: path.to_path_buf(),
            }
        }
        WriteFailure::SymlinkManaged => RepositoryAccessPreferencesError::SymlinkManaged {
            path: path.to_path_buf(),
        },
        WriteFailure::Io(error) => RepositoryAccessPreferencesError::Io(error),
    })
}

fn persist_access_preferences_with<R>(
    path: &Path,
    layer: ConfigLayer,
    auth_mode: Option<AuthMode>,
    telemetry_enabled: Option<bool>,
    reread: R,
) -> Result<AccessWriteOutcome, WriteFailure>
where
    R: FnOnce(&Path) -> anyhow::Result<Option<Vec<u8>>>,
{
    if auth_mode.is_none() && telemetry_enabled.is_none() {
        return Ok(AccessWriteOutcome::Unchanged);
    }

    let original = read_optional(path, layer).map_err(WriteFailure::Io)?;
    let original_value = match &original {
        Some(bytes) => parse_config(path, layer, bytes)?,
        None => layer.bootstrap_value(),
    };

    let mut updated_value = original_value.clone();
    let object = updated_value
        .as_object_mut()
        .ok_or_else(|| WriteFailure::Invalid("expected a top-level JSON object".to_string()))?;
    if let Some(mode) = auth_mode {
        set_nested_value(
            object,
            AUTH_KEY,
            AUTH_MODE_KEY,
            Value::String(mode.as_str().to_string()),
        );
    }
    if let Some(enabled) = telemetry_enabled {
        set_nested_value(
            object,
            TELEMETRY_KEY,
            TELEMETRY_ENABLED_KEY,
            Value::Bool(enabled),
        );
    }

    if original.is_some() && updated_value == original_value {
        return Ok(AccessWriteOutcome::Unchanged);
    }

    let mut serialized = serde_json::to_string_pretty(&updated_value)
        .with_context(|| format!("Failed to serialize {}", layer.label()))
        .map_err(WriteFailure::Io)?;
    serialized.push('\n');
    parse_file_config(&serialized, path, layer.path_source())
        .map_err(|error| WriteFailure::Invalid(format!("{error:#}")))?;

    let current = reread(path).map_err(WriteFailure::Io)?;
    if current != original {
        return Err(WriteFailure::ConcurrentModification);
    }
    if is_symlink(path, layer).map_err(WriteFailure::Io)? {
        return Err(WriteFailure::SymlinkManaged);
    }

    write_atomically(path, serialized.as_bytes()).map_err(WriteFailure::Io)?;
    Ok(AccessWriteOutcome::Written)
}

fn parse_config(path: &Path, layer: ConfigLayer, bytes: &[u8]) -> Result<Value, WriteFailure> {
    let raw = std::str::from_utf8(bytes)
        .map_err(|_| WriteFailure::Invalid("file is not valid UTF-8".to_string()))?;
    parse_file_config(raw, path, layer.path_source())
        .map_err(|error| WriteFailure::Invalid(format!("{error:#}")))?;
    serde_json::from_str(raw).map_err(|error| WriteFailure::Invalid(error.to_string()))
}

fn set_nested_value(object: &mut Map<String, Value>, section: &str, key: &str, value: Value) {
    let entry = object
        .entry(section.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    if let Some(section_object) = entry.as_object_mut() {
        section_object.insert(key.to_string(), value);
    }
}

fn is_symlink(path: &Path, layer: ConfigLayer) -> anyhow::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.file_type().is_symlink()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error)
            .with_context(|| format!("Failed to inspect {} '{}'", layer.label(), path.display())),
    }
}

fn read_optional(path: &Path, layer: ConfigLayer) -> anyhow::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error)
            .with_context(|| format!("Failed to read {} '{}'", layer.label(), path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn repository_config_path(dir: &Path) -> PathBuf {
        dir.join(".sce").join("config.json")
    }

    fn write_repository_config(path: &Path, value: &Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_string_pretty(value).unwrap() + "\n").unwrap();
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn repository_writer_bootstraps_missing_file_with_only_explicit_answers() {
        let dir = tempfile::tempdir().unwrap();
        let path = repository_config_path(dir.path());

        let outcome =
            persist_repository_access_preferences(&path, Some(AuthMode::Mandatory), None).unwrap();

        assert_eq!(outcome, AccessWriteOutcome::Written);
        let written = read_json(&path);
        assert_eq!(written["auth"], json!({ "mode": "mandatory" }));
        assert!(written.get("telemetry").is_none());
        assert_eq!(written["agent_trace"], json!({ "auto_sync": true }));
        assert_eq!(
            written["policies"],
            json!({ "attribution_hooks": { "enabled": true } })
        );
        assert_eq!(
            written[CONFIG_SCHEMA_DECLARATION_KEY],
            json!(sce_config_schema_url())
        );
        assert_eq!(
            read_repository_access_preferences(&path).unwrap(),
            AccessPreferences::Valid {
                auth_mode: Some(AuthMode::Mandatory),
                telemetry_enabled: None,
            }
        );
    }

    #[test]
    fn repository_writer_preserves_unrelated_keys_and_skips_no_op_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = repository_config_path(dir.path());
        write_repository_config(
            &path,
            &json!({
                "$schema": sce_config_schema_url(),
                "agent_trace": { "auto_sync": false },
                "auth": { "mode": "optional" },
            }),
        );

        let outcome =
            persist_repository_access_preferences(&path, Some(AuthMode::Disabled), Some(true))
                .unwrap();

        assert_eq!(outcome, AccessWriteOutcome::Written);
        let written = read_json(&path);
        assert_eq!(written["agent_trace"], json!({ "auto_sync": false }));
        assert_eq!(written["auth"], json!({ "mode": "disabled" }));
        assert_eq!(written["telemetry"], json!({ "enabled": true }));

        let before = fs::read(&path).unwrap();
        for (auth_mode, telemetry_enabled) in [
            (Some(AuthMode::Disabled), Some(true)),
            (Some(AuthMode::Disabled), None),
            (None, Some(true)),
            (None, None),
        ] {
            assert_eq!(
                persist_repository_access_preferences(&path, auth_mode, telemetry_enabled).unwrap(),
                AccessWriteOutcome::Unchanged
            );
            assert_eq!(fs::read(&path).unwrap(), before);
        }
    }

    #[test]
    fn repository_writer_skips_missing_file_without_answers() {
        let dir = tempfile::tempdir().unwrap();
        let path = repository_config_path(dir.path());

        assert_eq!(
            persist_repository_access_preferences(&path, None, None).unwrap(),
            AccessWriteOutcome::Unchanged
        );
        assert!(!path.exists());
        assert_eq!(
            read_repository_access_preferences(&path).unwrap(),
            AccessPreferences::Missing
        );
    }

    #[test]
    fn repository_writer_refuses_invalid_file_and_leaves_it_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = repository_config_path(dir.path());
        for invalid in [
            b"{ not json".to_vec(),
            b"{\"unknown_key\": true}\n".to_vec(),
            vec![0xff, 0xfe],
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, &invalid).unwrap();

            let error =
                persist_repository_access_preferences(&path, Some(AuthMode::Mandatory), Some(true))
                    .unwrap_err();

            assert!(matches!(
                error,
                RepositoryAccessPreferencesError::InvalidRepositoryConfig { .. }
            ));
            assert!(error.to_string().starts_with(&format!(
                "Repository SCE config '{}' is invalid and was left unchanged: ",
                path.display()
            )));
            assert_eq!(fs::read(&path).unwrap(), invalid);
            assert_eq!(
                read_repository_access_preferences(&path).unwrap(),
                AccessPreferences::Invalid
            );
        }
    }

    #[test]
    fn repository_writer_refuses_concurrently_modified_file_and_leaves_it_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = repository_config_path(dir.path());
        write_repository_config(&path, &json!({ "agent_trace": { "auto_sync": true } }));
        let concurrent = b"{\"agent_trace\": {\"auto_sync\": false}}\n".to_vec();

        let error = persist_repository_access_preferences_with(
            &path,
            Some(AuthMode::Mandatory),
            None,
            |path| {
                fs::write(path, &concurrent).unwrap();
                Ok(Some(fs::read(path).unwrap()))
            },
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            format!(
                "Repository SCE config '{}' changed while setup was updating it and was left unchanged.",
                path.display()
            )
        );
        assert_eq!(fs::read(&path).unwrap(), concurrent);
    }

    #[cfg(unix)]
    #[test]
    fn repository_writer_refuses_symlinked_file_and_leaves_it_unchanged() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let path = repository_config_path(dir.path());
        let target = dir.path().join("shared-config.json");
        write_repository_config(&target, &json!({ "agent_trace": { "auto_sync": true } }));
        let target_before = fs::read(&target).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        symlink(&target, &path).unwrap();

        let error =
            persist_repository_access_preferences(&path, Some(AuthMode::Mandatory), Some(false))
                .unwrap_err();

        assert!(matches!(
            error,
            RepositoryAccessPreferencesError::SymlinkManaged { .. }
        ));
        assert!(fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_link(&path).unwrap(), target);
        assert_eq!(fs::read(&target).unwrap(), target_before);
    }

    #[cfg(unix)]
    #[test]
    fn repository_writer_replaces_file_atomically_and_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = repository_config_path(dir.path());
        write_repository_config(&path, &json!({ "agent_trace": { "auto_sync": true } }));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

        persist_repository_access_preferences(&path, None, Some(true)).unwrap();

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o640
        );
        let entries: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("config.json")]);
        assert_eq!(read_json(&path)["telemetry"], json!({ "enabled": true }));
    }
}
