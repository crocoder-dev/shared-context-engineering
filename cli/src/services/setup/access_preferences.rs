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

fn read_optional(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error)
            .with_context(|| format!("Failed to read global SCE config '{}'", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn read_json(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    fn write_json(path: &Path, value: &Value) {
        fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    #[test]
    fn access_preferences_writer_sets_only_requested_keys() {
        struct Case {
            name: &'static str,
            initial: Option<Value>,
            auth_mode: Option<AuthMode>,
            telemetry_enabled: Option<bool>,
            expected: Value,
        }

        let schema = sce_config_schema_url();
        let cases = [
            Case {
                name: "missing file",
                initial: None,
                auth_mode: Some(AuthMode::Optional),
                telemetry_enabled: Some(false),
                expected: json!({
                    "$schema": schema,
                    "auth": {"mode": "optional"},
                    "telemetry": {"enabled": false}
                }),
            },
            Case {
                name: "empty object",
                initial: Some(json!({})),
                auth_mode: None,
                telemetry_enabled: Some(true),
                expected: json!({"telemetry": {"enabled": true}}),
            },
            Case {
                name: "unrelated keys preserved",
                initial: Some(json!({
                    "log_level": "debug",
                    "agent_trace": {"auto_sync": true},
                    "policies": {"attribution_hooks": {"enabled": false}}
                })),
                auth_mode: Some(AuthMode::Mandatory),
                telemetry_enabled: None,
                expected: json!({
                    "log_level": "debug",
                    "agent_trace": {"auto_sync": true},
                    "policies": {"attribution_hooks": {"enabled": false}},
                    "auth": {"mode": "mandatory"}
                }),
            },
            Case {
                name: "explicit values replaced",
                initial: Some(json!({
                    "auth": {"mode": "disabled"},
                    "telemetry": {"enabled": true}
                })),
                auth_mode: Some(AuthMode::Optional),
                telemetry_enabled: Some(false),
                expected: json!({
                    "auth": {"mode": "optional"},
                    "telemetry": {"enabled": false}
                }),
            },
        ];

        for case in cases {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("sce").join("config.json");
            if let Some(initial) = &case.initial {
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                write_json(&path, initial);
            }

            let outcome =
                persist_global_access_preferences(&path, case.auth_mode, case.telemetry_enabled)
                    .unwrap();

            assert_eq!(outcome, GlobalWriteOutcome::Written, "{}", case.name);
            assert_eq!(read_json(&path), case.expected, "{}", case.name);
            assert!(
                fs::read_to_string(&path).unwrap().ends_with("}\n"),
                "{}",
                case.name
            );
            assert_eq!(
                read_global_access_preferences(&path).unwrap(),
                GlobalAccessPreferences::Valid {
                    auth_mode: case.auth_mode,
                    telemetry_enabled: case.telemetry_enabled,
                },
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn access_preferences_writer_is_idempotent_on_rerun() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        persist_global_access_preferences(&path, Some(AuthMode::Optional), Some(true)).unwrap();
        let first = fs::read(&path).unwrap();

        let outcome =
            persist_global_access_preferences(&path, Some(AuthMode::Optional), Some(true)).unwrap();

        assert_eq!(outcome, GlobalWriteOutcome::Unchanged);
        assert_eq!(fs::read(&path).unwrap(), first);
    }

    #[test]
    fn access_preferences_writer_without_answers_never_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        let outcome = persist_global_access_preferences(&path, None, None).unwrap();

        assert_eq!(outcome, GlobalWriteOutcome::Unchanged);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn access_preferences_writer_preserves_existing_mode_and_creates_private_file() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("existing.json");
        write_json(&existing, &json!({}));
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o644)).unwrap();
        let created = dir.path().join("created.json");

        persist_global_access_preferences(&existing, Some(AuthMode::Optional), None).unwrap();
        persist_global_access_preferences(&created, Some(AuthMode::Optional), None).unwrap();

        assert_eq!(
            fs::metadata(&existing).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert_eq!(
            fs::metadata(&created).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn access_preferences_writer_refuses_invalid_files_without_touching_them() {
        let cases: [(&str, &[u8]); 4] = [
            ("invalid json", b"{not json"),
            ("schema invalid mode", br#"{"auth":{"mode":"required"}}"#),
            ("unknown top-level key", br#"{"surprise":true}"#),
            ("not utf8", &[0xff, 0xfe, 0x00]),
        ];

        for (name, bytes) in cases {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            fs::write(&path, bytes).unwrap();

            let error =
                persist_global_access_preferences(&path, Some(AuthMode::Optional), Some(true))
                    .unwrap_err();

            assert!(
                matches!(
                    error,
                    GlobalAccessPreferencesError::InvalidGlobalConfig { .. }
                ),
                "{name}: {error}"
            );
            assert_eq!(fs::read(&path).unwrap(), bytes, "{name}");
            assert_eq!(
                read_global_access_preferences(&path).unwrap(),
                GlobalAccessPreferences::Invalid,
                "{name}"
            );
        }
    }

    #[test]
    fn access_preferences_writer_detects_concurrent_modification_before_rename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_json(&path, &json!({"log_level": "info"}));
        let concurrent = br#"{"log_level":"debug"}"#.to_vec();

        let error = persist_global_access_preferences_with(
            &path,
            Some(AuthMode::Optional),
            Some(true),
            |reread_path| {
                fs::write(reread_path, &concurrent).unwrap();
                read_optional(reread_path)
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            GlobalAccessPreferencesError::ConcurrentModification { .. }
        ));
        assert_eq!(fs::read(&path).unwrap(), concurrent);
    }

    #[cfg(unix)]
    #[test]
    fn access_preferences_writer_reports_io_error_for_read_only_directory() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path().join("sce");
        fs::create_dir_all(&config_dir).unwrap();
        let path = config_dir.join("config.json");
        let original = br#"{"log_level":"info"}"#.to_vec();
        fs::write(&path, &original).unwrap();
        fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o555)).unwrap();

        let result = persist_global_access_preferences(&path, Some(AuthMode::Optional), None);

        fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(result, Err(GlobalAccessPreferencesError::Io(_))));
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn access_preferences_reader_distinguishes_missing_and_absent_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        assert_eq!(
            read_global_access_preferences(&path).unwrap(),
            GlobalAccessPreferences::Missing
        );

        write_json(&path, &json!({"log_level": "info"}));
        assert_eq!(
            read_global_access_preferences(&path).unwrap(),
            GlobalAccessPreferences::Valid {
                auth_mode: None,
                telemetry_enabled: None,
            }
        );
    }
}
