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
pub(crate) enum GlobalAccessPreferences {
    Missing,
    Valid {
        auth_mode: Option<AuthMode>,
        telemetry_enabled: Option<bool>,
    },
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GlobalWriteOutcome {
    Unchanged,
    Written,
}

#[derive(Debug)]
pub(crate) enum GlobalAccessPreferencesError {
    InvalidGlobalConfig { path: PathBuf, reason: String },
    ConcurrentModification { path: PathBuf },
    SymlinkManaged { path: PathBuf },
    Io(anyhow::Error),
}

impl fmt::Display for GlobalAccessPreferencesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGlobalConfig { path, reason } => write!(
                f,
                "Global SCE config '{}' is invalid and was left unchanged: {reason}",
                path.display()
            ),
            Self::ConcurrentModification { path } => write!(
                f,
                "Global SCE config '{}' changed while setup was updating it and was left unchanged.",
                path.display()
            ),
            Self::SymlinkManaged { path } => write!(
                f,
                "Global SCE config '{}' is a symlink and was left unchanged; SCE cannot modify a symlink-managed global config. Update the file's source directly (for example, your Nix or Home Manager configuration).",
                path.display()
            ),
            Self::Io(error) => write!(f, "{error:#}"),
        }
    }
}

impl std::error::Error for GlobalAccessPreferencesError {}

pub(crate) fn read_global_access_preferences(
    path: &Path,
) -> anyhow::Result<GlobalAccessPreferences> {
    let Some(bytes) = read_optional(path)? else {
        return Ok(GlobalAccessPreferences::Missing);
    };
    let Ok(raw) = String::from_utf8(bytes) else {
        return Ok(GlobalAccessPreferences::Invalid);
    };
    let Ok(config) = parse_file_config(&raw, path, ConfigPathSource::DefaultDiscoveredGlobal)
    else {
        return Ok(GlobalAccessPreferences::Invalid);
    };

    Ok(GlobalAccessPreferences::Valid {
        auth_mode: config.auth_mode.map(|setting| setting.value),
        telemetry_enabled: config.telemetry_enabled.map(|setting| setting.value),
    })
}

pub(crate) fn persist_global_access_preferences(
    path: &Path,
    auth_mode: Option<AuthMode>,
    telemetry_enabled: Option<bool>,
) -> Result<GlobalWriteOutcome, GlobalAccessPreferencesError> {
    persist_global_access_preferences_with(path, auth_mode, telemetry_enabled, read_optional)
}

fn persist_global_access_preferences_with<R>(
    path: &Path,
    auth_mode: Option<AuthMode>,
    telemetry_enabled: Option<bool>,
    reread: R,
) -> Result<GlobalWriteOutcome, GlobalAccessPreferencesError>
where
    R: FnOnce(&Path) -> anyhow::Result<Option<Vec<u8>>>,
{
    if auth_mode.is_none() && telemetry_enabled.is_none() {
        return Ok(GlobalWriteOutcome::Unchanged);
    }

    let original = read_optional(path).map_err(GlobalAccessPreferencesError::Io)?;
    let original_value = match &original {
        Some(bytes) => parse_global_config(path, bytes)?,
        None => Value::Object(Map::from_iter([(
            CONFIG_SCHEMA_DECLARATION_KEY.to_string(),
            Value::String(sce_config_schema_url()),
        )])),
    };

    let mut updated_value = original_value.clone();
    let object = updated_value
        .as_object_mut()
        .ok_or_else(|| invalid_global_config(path, "expected a top-level JSON object"))?;
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
        return Ok(GlobalWriteOutcome::Unchanged);
    }

    let mut serialized = serde_json::to_string_pretty(&updated_value)
        .context("Failed to serialize global SCE config")
        .map_err(GlobalAccessPreferencesError::Io)?;
    serialized.push('\n');
    parse_file_config(&serialized, path, ConfigPathSource::DefaultDiscoveredGlobal)
        .map_err(|error| invalid_global_config(path, &format!("{error:#}")))?;

    let current = reread(path).map_err(GlobalAccessPreferencesError::Io)?;
    if current != original {
        return Err(GlobalAccessPreferencesError::ConcurrentModification {
            path: path.to_path_buf(),
        });
    }
    if is_symlink(path).map_err(GlobalAccessPreferencesError::Io)? {
        return Err(GlobalAccessPreferencesError::SymlinkManaged {
            path: path.to_path_buf(),
        });
    }

    write_atomically(path, serialized.as_bytes()).map_err(GlobalAccessPreferencesError::Io)?;
    Ok(GlobalWriteOutcome::Written)
}

fn parse_global_config(path: &Path, bytes: &[u8]) -> Result<Value, GlobalAccessPreferencesError> {
    let raw = std::str::from_utf8(bytes)
        .map_err(|_| invalid_global_config(path, "file is not valid UTF-8"))?;
    parse_file_config(raw, path, ConfigPathSource::DefaultDiscoveredGlobal)
        .map_err(|error| invalid_global_config(path, &format!("{error:#}")))?;
    serde_json::from_str(raw).map_err(|error| invalid_global_config(path, &error.to_string()))
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

fn invalid_global_config(path: &Path, reason: &str) -> GlobalAccessPreferencesError {
    GlobalAccessPreferencesError::InvalidGlobalConfig {
        path: path.to_path_buf(),
        reason: reason.to_string(),
    }
}

fn is_symlink(path: &Path) -> anyhow::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.file_type().is_symlink()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error)
            .with_context(|| format!("Failed to inspect global SCE config '{}'", path.display())),
    }
}

fn read_optional(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error)
            .with_context(|| format!("Failed to read global SCE config '{}'", path.display())),
    }
}
