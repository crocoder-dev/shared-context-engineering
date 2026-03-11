use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::services::output_format::OutputFormat;

pub const NAME: &str = "config";

const DEFAULT_TIMEOUT_MS: u64 = 30000;
const WORKOS_CLIENT_ID_ENV: &str = "WORKOS_CLIENT_ID";
const WORKOS_CLIENT_ID_BAKED_DEFAULT: &str = "client_sce_default";
const WORKOS_CLIENT_ID_KEY: AuthConfigKeySpec = AuthConfigKeySpec {
    config_key: "workos_client_id",
    env_key: WORKOS_CLIENT_ID_ENV,
    baked_default: Some(WORKOS_CLIENT_ID_BAKED_DEFAULT),
};

pub type ReportFormat = OutputFormat;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    fn parse(raw: &str, source: &str) -> Result<Self> {
        match raw {
            "error" => Ok(Self::Error),
            "warn" => Ok(Self::Warn),
            "info" => Ok(Self::Info),
            "debug" => Ok(Self::Debug),
            _ => bail!(
                "Invalid log level '{raw}' from {source}. Valid values: error, warn, info, debug."
            ),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ValueSource {
    Flag,
    Env,
    ConfigFile(ConfigPathSource),
    Default,
}

impl ValueSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Env => "env",
            Self::ConfigFile(_) => "config_file",
            Self::Default => "default",
        }
    }

    fn config_source(self) -> Option<ConfigPathSource> {
        match self {
            Self::ConfigFile(source) => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigSubcommand {
    Show(ConfigRequest),
    Validate(ConfigRequest),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigRequest {
    pub report_format: ReportFormat,
    pub config_path: Option<PathBuf>,
    pub log_level: Option<LogLevel>,
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConfigPathSource {
    Flag,
    Env,
    DefaultDiscoveredGlobal,
    DefaultDiscoveredLocal,
}

impl ConfigPathSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Env => "env",
            Self::DefaultDiscoveredGlobal => "default_discovered_global",
            Self::DefaultDiscoveredLocal => "default_discovered_local",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResolvedValue<T> {
    value: T,
    source: ValueSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LoadedConfigPath {
    path: PathBuf,
    source: ConfigPathSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AuthConfigKeySpec {
    config_key: &'static str,
    env_key: &'static str,
    baked_default: Option<&'static str>,
}

impl AuthConfigKeySpec {
    fn precedence_description(self) -> String {
        let mut layers = vec![
            format!("env ({})", self.env_key),
            format!("config file ({})", self.config_key),
        ];

        if let Some(default) = self.baked_default {
            layers.push(format!("baked default ({default})"));
        }

        layers.join(" > ")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeConfig {
    loaded_config_paths: Vec<LoadedConfigPath>,
    log_level: ResolvedValue<LogLevel>,
    timeout_ms: ResolvedValue<u64>,
    workos_client_id: ResolvedOptionalValue<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedOptionalValue<T> {
    pub(crate) value: Option<T>,
    source: Option<ValueSource>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedAuthRuntimeConfig {
    pub(crate) workos_client_id: ResolvedOptionalValue<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileConfig {
    log_level: Option<FileConfigValue<LogLevel>>,
    timeout_ms: Option<FileConfigValue<u64>>,
    workos_client_id: Option<FileConfigValue<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileConfigValue<T> {
    value: T,
    source: ConfigPathSource,
}

pub fn run_config_subcommand(subcommand: ConfigSubcommand) -> Result<String> {
    match subcommand {
        ConfigSubcommand::Show(request) => {
            let cwd = std::env::current_dir().context("Failed to determine current directory")?;
            let runtime = resolve_runtime_config(&request, &cwd)?;
            Ok(format_show_output(&runtime, request.report_format))
        }
        ConfigSubcommand::Validate(request) => {
            let cwd = std::env::current_dir().context("Failed to determine current directory")?;
            let runtime = resolve_runtime_config(&request, &cwd)?;
            Ok(format_validate_output(&runtime, request.report_format))
        }
    }
}

pub(crate) fn resolve_auth_runtime_config(cwd: &Path) -> Result<ResolvedAuthRuntimeConfig> {
    resolve_auth_runtime_config_with(
        cwd,
        |key| std::env::var(key).ok(),
        |path| {
            std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read config file '{}'.", path.display()))
        },
        Path::exists,
        resolve_default_global_config_path,
    )
}

pub(crate) fn resolve_auth_runtime_config_with<FEnv, FRead, FGlobalPath>(
    cwd: &Path,
    env_lookup: FEnv,
    read_file: FRead,
    path_exists: fn(&Path) -> bool,
    resolve_global_config_path: FGlobalPath,
) -> Result<ResolvedAuthRuntimeConfig>
where
    FEnv: Fn(&str) -> Option<String>,
    FRead: Fn(&Path) -> Result<String>,
    FGlobalPath: Fn() -> Result<PathBuf>,
{
    let runtime = resolve_runtime_config_with(
        &ConfigRequest {
            report_format: ReportFormat::Text,
            config_path: None,
            log_level: None,
            timeout_ms: None,
        },
        cwd,
        env_lookup,
        read_file,
        path_exists,
        resolve_global_config_path,
    )?;

    Ok(ResolvedAuthRuntimeConfig {
        workos_client_id: runtime.workos_client_id,
    })
}

fn resolve_runtime_config(request: &ConfigRequest, cwd: &Path) -> Result<RuntimeConfig> {
    resolve_runtime_config_with(
        request,
        cwd,
        |key| std::env::var(key).ok(),
        |path| {
            std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read config file '{}'.", path.display()))
        },
        Path::exists,
        resolve_default_global_config_path,
    )
}

fn resolve_runtime_config_with<FEnv, FRead, FGlobalPath>(
    request: &ConfigRequest,
    cwd: &Path,
    env_lookup: FEnv,
    read_file: FRead,
    path_exists: fn(&Path) -> bool,
    resolve_global_config_path: FGlobalPath,
) -> Result<RuntimeConfig>
where
    FEnv: Fn(&str) -> Option<String>,
    FRead: Fn(&Path) -> Result<String>,
    FGlobalPath: Fn() -> Result<PathBuf>,
{
    let loaded_config_paths = resolve_config_paths(
        request,
        cwd,
        &env_lookup,
        path_exists,
        resolve_global_config_path,
    )?;

    let mut file_config = FileConfig {
        log_level: None,
        timeout_ms: None,
        workos_client_id: None,
    };
    for loaded_path in &loaded_config_paths {
        let raw = read_file(&loaded_path.path)?;
        let layer = parse_file_config(&raw, &loaded_path.path, loaded_path.source)?;
        if let Some(log_level) = layer.log_level {
            file_config.log_level = Some(log_level);
        }
        if let Some(timeout_ms) = layer.timeout_ms {
            file_config.timeout_ms = Some(timeout_ms);
        }
        if let Some(workos_client_id) = layer.workos_client_id {
            file_config.workos_client_id = Some(workos_client_id);
        }
    }

    let mut resolved_log_level = ResolvedValue {
        value: LogLevel::Info,
        source: ValueSource::Default,
    };
    if let Some(value) = file_config.log_level {
        resolved_log_level = ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        };
    }
    if let Some(raw) = env_lookup("SCE_LOG_LEVEL") {
        resolved_log_level = ResolvedValue {
            value: LogLevel::parse(&raw, "SCE_LOG_LEVEL")?,
            source: ValueSource::Env,
        };
    }
    if let Some(value) = request.log_level {
        resolved_log_level = ResolvedValue {
            value,
            source: ValueSource::Flag,
        };
    }

    let mut resolved_timeout_ms = ResolvedValue {
        value: DEFAULT_TIMEOUT_MS,
        source: ValueSource::Default,
    };
    if let Some(value) = file_config.timeout_ms {
        resolved_timeout_ms = ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        };
    }
    if let Some(raw) = env_lookup("SCE_TIMEOUT_MS") {
        let value = raw
            .parse::<u64>()
            .map_err(|_| anyhow!("Invalid timeout '{raw}' from SCE_TIMEOUT_MS."))?;
        resolved_timeout_ms = ResolvedValue {
            value,
            source: ValueSource::Env,
        };
    }
    if let Some(value) = request.timeout_ms {
        resolved_timeout_ms = ResolvedValue {
            value,
            source: ValueSource::Flag,
        };
    }

    let resolved_workos_client_id = resolve_optional_auth_config_value(
        WORKOS_CLIENT_ID_KEY,
        file_config.workos_client_id,
        &env_lookup,
    );

    Ok(RuntimeConfig {
        loaded_config_paths,
        log_level: resolved_log_level,
        timeout_ms: resolved_timeout_ms,
        workos_client_id: resolved_workos_client_id,
    })
}

fn resolve_optional_auth_config_value<FEnv>(
    key: AuthConfigKeySpec,
    file_value: Option<FileConfigValue<String>>,
    env_lookup: &FEnv,
) -> ResolvedOptionalValue<String>
where
    FEnv: Fn(&str) -> Option<String>,
{
    if let Some(raw) = env_lookup(key.env_key) {
        return ResolvedOptionalValue {
            value: Some(raw),
            source: Some(ValueSource::Env),
        };
    }

    if let Some(value) = file_value {
        return ResolvedOptionalValue {
            value: Some(value.value),
            source: Some(ValueSource::ConfigFile(value.source)),
        };
    }

    if let Some(value) = key.baked_default {
        return ResolvedOptionalValue {
            value: Some(value.to_string()),
            source: Some(ValueSource::Default),
        };
    }

    ResolvedOptionalValue {
        value: None,
        source: None,
    }
}

fn resolve_config_paths<FEnv, FGlobalPath>(
    request: &ConfigRequest,
    cwd: &Path,
    env_lookup: &FEnv,
    path_exists: fn(&Path) -> bool,
    resolve_global_config_path: FGlobalPath,
) -> Result<Vec<LoadedConfigPath>>
where
    FEnv: Fn(&str) -> Option<String>,
    FGlobalPath: Fn() -> Result<PathBuf>,
{
    if let Some(path) = request.config_path.as_ref() {
        if !path_exists(path) {
            bail!(
                "Config file '{}' was provided via --config but does not exist.",
                path.display()
            );
        }
        return Ok(vec![LoadedConfigPath {
            path: path.clone(),
            source: ConfigPathSource::Flag,
        }]);
    }

    if let Some(raw) = env_lookup("SCE_CONFIG_FILE") {
        let path = PathBuf::from(raw);
        if !path_exists(&path) {
            bail!(
                "Config file '{}' was provided via SCE_CONFIG_FILE but does not exist.",
                path.display()
            );
        }
        return Ok(vec![LoadedConfigPath {
            path,
            source: ConfigPathSource::Env,
        }]);
    }

    let mut discovered_paths = Vec::new();

    let global_path = resolve_global_config_path()?
        .join("sce")
        .join("config.json");
    if path_exists(&global_path) {
        discovered_paths.push(LoadedConfigPath {
            path: global_path,
            source: ConfigPathSource::DefaultDiscoveredGlobal,
        });
    }

    let local_path = cwd.join(".sce").join("config.json");
    if path_exists(&local_path) {
        discovered_paths.push(LoadedConfigPath {
            path: local_path,
            source: ConfigPathSource::DefaultDiscoveredLocal,
        });
    }

    Ok(discovered_paths)
}

fn resolve_default_global_config_path() -> Result<PathBuf> {
    crate::services::local_db::resolve_state_data_root()
}

fn parse_file_config(raw: &str, path: &Path, source: ConfigPathSource) -> Result<FileConfig> {
    let parsed: Value = serde_json::from_str(raw)
        .with_context(|| format!("Config file '{}' must contain valid JSON.", path.display()))?;

    let object = parsed.as_object().with_context(|| {
        format!(
            "Config file '{}' must contain a top-level JSON object.",
            path.display()
        )
    })?;

    for key in object.keys() {
        if key != "log_level" && key != "timeout_ms" && key != WORKOS_CLIENT_ID_KEY.config_key {
            bail!(
                "Config file '{}' contains unknown key '{}'. Allowed keys: log_level, timeout_ms, {}.",
                path.display(),
                key,
                WORKOS_CLIENT_ID_KEY.config_key
            );
        }
    }

    let log_level = match object.get("log_level") {
        Some(value) => {
            let raw = value.as_str().with_context(|| {
                format!(
                    "Config key 'log_level' in '{}' must be a string.",
                    path.display()
                )
            })?;
            Some(FileConfigValue {
                value: LogLevel::parse(raw, &format!("config file '{}'", path.display()))?,
                source,
            })
        }
        None => None,
    };

    let timeout_ms = match object.get("timeout_ms") {
        Some(value) => {
            let parsed = value.as_u64().with_context(|| {
                format!(
                    "Config key 'timeout_ms' in '{}' must be an unsigned integer.",
                    path.display()
                )
            })?;
            Some(FileConfigValue {
                value: parsed,
                source,
            })
        }
        None => None,
    };

    let workos_client_id = parse_optional_string_key(object, path, source, WORKOS_CLIENT_ID_KEY)?;

    Ok(FileConfig {
        log_level,
        timeout_ms,
        workos_client_id,
    })
}

fn parse_optional_string_key(
    object: &serde_json::Map<String, Value>,
    path: &Path,
    source: ConfigPathSource,
    key: AuthConfigKeySpec,
) -> Result<Option<FileConfigValue<String>>> {
    let Some(value) = object.get(key.config_key) else {
        return Ok(None);
    };

    let raw = value.as_str().with_context(|| {
        format!(
            "Config key '{}' in '{}' must be a string.",
            key.config_key,
            path.display()
        )
    })?;

    Ok(Some(FileConfigValue {
        value: raw.to_string(),
        source,
    }))
}

fn format_show_output(runtime: &RuntimeConfig, report_format: ReportFormat) -> String {
    match report_format {
        ReportFormat::Text => {
            let lines = [
                "SCE config: resolved".to_string(),
                "Precedence: flags > env > config file > defaults".to_string(),
                format_config_paths_text(runtime),
                format_resolved_value_text(
                    "log_level",
                    runtime.log_level.value.as_str(),
                    runtime.log_level.source,
                ),
                format_resolved_value_text(
                    "timeout_ms",
                    &runtime.timeout_ms.value.to_string(),
                    runtime.timeout_ms.source,
                ),
                format_optional_auth_resolved_value_text(
                    WORKOS_CLIENT_ID_KEY,
                    &runtime.workos_client_id,
                ),
            ];
            lines.join("\n")
        }
        ReportFormat::Json => {
            let payload = json!({
                "status": "ok",
                "result": {
                    "command": "config_show",
                    "precedence": "flags > env > config file > defaults",
                    "config_paths": format_config_paths_json(runtime),
                    "resolved": {
                        "log_level": {
                            "value": runtime.log_level.value.as_str(),
                            "source": runtime.log_level.source.as_str(),
                            "config_source": runtime.log_level.source.config_source().map(ConfigPathSource::as_str),
                        },
                        "timeout_ms": {
                            "value": runtime.timeout_ms.value,
                            "source": runtime.timeout_ms.source.as_str(),
                            "config_source": runtime.timeout_ms.source.config_source().map(ConfigPathSource::as_str),
                        },
                        "workos_client_id": format_optional_auth_resolved_value_json(WORKOS_CLIENT_ID_KEY, &runtime.workos_client_id)
                    }
                }
            });
            serde_json::to_string_pretty(&payload).expect("config show payload should serialize")
        }
    }
}

fn format_validate_output(runtime: &RuntimeConfig, report_format: ReportFormat) -> String {
    match report_format {
        ReportFormat::Text => {
            let lines = [
                "SCE config validation: valid".to_string(),
                "Precedence: flags > env > config file > defaults".to_string(),
                format_config_paths_text(runtime),
                "Validation issues: none".to_string(),
                format!(
                    "Resolved auth precedence: {}",
                    WORKOS_CLIENT_ID_KEY.precedence_description()
                ),
                format_optional_auth_resolved_value_text(
                    WORKOS_CLIENT_ID_KEY,
                    &runtime.workos_client_id,
                ),
            ];
            lines.join("\n")
        }
        ReportFormat::Json => {
            let payload = json!({
                "status": "ok",
                "result": {
                    "command": "config_validate",
                    "valid": true,
                    "precedence": "flags > env > config file > defaults",
                    "config_paths": format_config_paths_json(runtime),
                    "issues": [],
                    "resolved_auth": {
                        "workos_client_id": format_optional_auth_resolved_value_json(WORKOS_CLIENT_ID_KEY, &runtime.workos_client_id)
                    }
                }
            });
            serde_json::to_string_pretty(&payload)
                .expect("config validate payload should serialize")
        }
    }
}

fn format_config_paths_text(runtime: &RuntimeConfig) -> String {
    if runtime.loaded_config_paths.is_empty() {
        return "Config files: (none discovered)".to_string();
    }

    let mut lines = vec!["Config files:".to_string()];
    for path in &runtime.loaded_config_paths {
        lines.push(format!(
            "- {} (source: {})",
            path.path.display(),
            path.source.as_str()
        ));
    }
    lines.join("\n")
}

fn format_config_paths_json(runtime: &RuntimeConfig) -> Value {
    Value::Array(
        runtime
            .loaded_config_paths
            .iter()
            .map(|path| {
                json!({
                "path": path.path.display().to_string(),
                "source": path.source.as_str(),
                    })
            })
            .collect(),
    )
}

fn format_resolved_value_text(key: &str, value: &str, source: ValueSource) -> String {
    match source.config_source() {
        Some(config_source) => format!(
            "- {}: {} (source: {}, config_source: {})",
            key,
            value,
            source.as_str(),
            config_source.as_str()
        ),
        None => format!("- {}: {} (source: {})", key, value, source.as_str()),
    }
}

fn format_optional_auth_resolved_value_text(
    key: AuthConfigKeySpec,
    value: &ResolvedOptionalValue<String>,
) -> String {
    match (value.value.as_deref(), value.source) {
        (Some(raw_value), Some(source)) => {
            let display_value = format_text_display_value(key.config_key, raw_value);
            match source.config_source() {
                Some(config_source) => format!(
                    "- {}: {} (source: {}, config_source: {}, auth_precedence: {})",
                    key.config_key,
                    display_value,
                    source.as_str(),
                    config_source.as_str(),
                    key.precedence_description()
                ),
                None => format!(
                    "- {}: {} (source: {}, auth_precedence: {})",
                    key.config_key,
                    display_value,
                    source.as_str(),
                    key.precedence_description()
                ),
            }
        }
        _ => format!(
            "- {}: (unset) (source: none, auth_precedence: {})",
            key.config_key,
            key.precedence_description()
        ),
    }
}

fn format_optional_auth_resolved_value_json(
    key: AuthConfigKeySpec,
    value: &ResolvedOptionalValue<String>,
) -> Value {
    json!({
        "value": value.value,
        "display_value": value.value.as_deref().map(|raw| format_text_display_value(key.config_key, raw)),
        "source": value.source.map(ValueSource::as_str),
        "config_source": value.source.and_then(ValueSource::config_source).map(ConfigPathSource::as_str),
        "precedence": key.precedence_description(),
    })
}

fn format_text_display_value(key: &str, value: &str) -> String {
    if should_fully_redact_text_value(key) {
        return "[REDACTED]".to_string();
    }

    if looks_credential_like(value) {
        return abbreviate_text_value(value);
    }

    value.to_string()
}

fn should_fully_redact_text_value(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    ["password", "passwd", "secret", "token", "api_key", "apikey"]
        .iter()
        .any(|needle| key.contains(needle))
}

fn looks_credential_like(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.len() >= 16
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/'))
}

fn abbreviate_text_value(value: &str) -> String {
    let total = value.chars().count();
    if total <= 8 {
        return value.to_string();
    }

    let prefix: String = value.chars().take(4).collect();
    let suffix: String = value
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{prefix}...{suffix}")
}

#[cfg(test)]
mod tests {
    use super::{
        format_show_output, format_validate_output, resolve_optional_auth_config_value,
        resolve_runtime_config_with, AuthConfigKeySpec, ConfigPathSource, ConfigRequest,
        FileConfigValue, LoadedConfigPath, LogLevel, ReportFormat, ResolvedOptionalValue,
        ResolvedValue, RuntimeConfig, ValueSource, WORKOS_CLIENT_ID_BAKED_DEFAULT,
        WORKOS_CLIENT_ID_KEY,
    };
    use anyhow::Result;
    use serde_json::Value;
    use std::path::{Path, PathBuf};

    fn request() -> ConfigRequest {
        ConfigRequest {
            report_format: ReportFormat::Text,
            config_path: None,
            log_level: None,
            timeout_ms: None,
        }
    }

    #[test]
    fn resolver_applies_precedence_flag_then_env_then_config_then_default() -> Result<()> {
        let req = ConfigRequest {
            report_format: ReportFormat::Text,
            config_path: Some(PathBuf::from("/tmp/config.json")),
            log_level: Some(LogLevel::Warn),
            timeout_ms: Some(900),
        };
        let resolved = resolve_runtime_config_with(
            &req,
            Path::new("/workspace"),
            |key| match key {
                "SCE_LOG_LEVEL" => Some("debug".to_string()),
                "SCE_TIMEOUT_MS" => Some("700".to_string()),
                "WORKOS_CLIENT_ID" => Some("from-env".to_string()),
                _ => None,
            },
            |_| {
                Ok("{\"log_level\":\"error\",\"timeout_ms\":500,\"workos_client_id\":\"from-config\"}".to_string())
            },
            |_| true,
            || Ok(PathBuf::from("/state")),
        )?;

        assert_eq!(resolved.log_level.value, LogLevel::Warn);
        assert_eq!(resolved.log_level.source.as_str(), "flag");
        assert_eq!(resolved.timeout_ms.value, 900);
        assert_eq!(resolved.timeout_ms.source.as_str(), "flag");
        assert_eq!(resolved.workos_client_id.value.as_deref(), Some("from-env"));
        assert_eq!(
            resolved
                .workos_client_id
                .source
                .map(super::ValueSource::as_str),
            Some("env")
        );
        Ok(())
    }

    #[test]
    fn resolver_uses_env_when_flags_absent() -> Result<()> {
        let req = ConfigRequest {
            report_format: ReportFormat::Text,
            config_path: Some(PathBuf::from("/tmp/config.json")),
            log_level: None,
            timeout_ms: None,
        };
        let resolved = resolve_runtime_config_with(
            &req,
            Path::new("/workspace"),
            |key| match key {
                "SCE_LOG_LEVEL" => Some("warn".to_string()),
                "SCE_TIMEOUT_MS" => Some("1200".to_string()),
                "WORKOS_CLIENT_ID" => Some("from-env".to_string()),
                _ => None,
            },
            |_| {
                Ok("{\"log_level\":\"error\",\"timeout_ms\":500,\"workos_client_id\":\"from-config\"}".to_string())
            },
            |_| true,
            || Ok(PathBuf::from("/state")),
        )?;

        assert_eq!(resolved.log_level.value, LogLevel::Warn);
        assert_eq!(resolved.log_level.source.as_str(), "env");
        assert_eq!(resolved.timeout_ms.value, 1200);
        assert_eq!(resolved.timeout_ms.source.as_str(), "env");
        assert_eq!(resolved.workos_client_id.value.as_deref(), Some("from-env"));
        assert_eq!(
            resolved
                .workos_client_id
                .source
                .map(super::ValueSource::as_str),
            Some("env")
        );
        Ok(())
    }

    #[test]
    fn resolver_uses_defaults_when_no_inputs_present() -> Result<()> {
        let req = request();
        let resolved = resolve_runtime_config_with(
            &req,
            Path::new("/workspace"),
            |_| None,
            |_| Ok("{}".to_string()),
            |_| false,
            || Ok(PathBuf::from("/state")),
        )?;

        assert_eq!(resolved.log_level.value, LogLevel::Info);
        assert_eq!(resolved.log_level.source.as_str(), "default");
        assert_eq!(resolved.timeout_ms.value, 30000);
        assert_eq!(resolved.timeout_ms.source.as_str(), "default");
        assert_eq!(
            resolved.workos_client_id.value.as_deref(),
            Some(WORKOS_CLIENT_ID_BAKED_DEFAULT)
        );
        assert_eq!(
            resolved
                .workos_client_id
                .source
                .map(super::ValueSource::as_str),
            Some("default")
        );
        Ok(())
    }

    #[test]
    fn auth_resolver_uses_baked_default_when_env_and_config_are_absent() {
        let resolved = resolve_optional_auth_config_value(WORKOS_CLIENT_ID_KEY, None, &|_| None);

        assert_eq!(
            resolved.value.as_deref(),
            Some(WORKOS_CLIENT_ID_BAKED_DEFAULT)
        );
        assert_eq!(resolved.source, Some(ValueSource::Default));
    }

    #[test]
    fn auth_resolver_uses_config_when_env_is_absent() {
        let resolved = resolve_optional_auth_config_value(
            WORKOS_CLIENT_ID_KEY,
            Some(FileConfigValue {
                value: "from-config".to_string(),
                source: ConfigPathSource::DefaultDiscoveredLocal,
            }),
            &|_| None,
        );

        assert_eq!(resolved.value.as_deref(), Some("from-config"));
        assert_eq!(
            resolved.source,
            Some(ValueSource::ConfigFile(
                ConfigPathSource::DefaultDiscoveredLocal,
            ))
        );
    }

    #[test]
    fn auth_resolver_uses_env_over_config_and_baked_default() {
        let resolved = resolve_optional_auth_config_value(
            WORKOS_CLIENT_ID_KEY,
            Some(FileConfigValue {
                value: "from-config".to_string(),
                source: ConfigPathSource::DefaultDiscoveredGlobal,
            }),
            &|key| match key {
                "WORKOS_CLIENT_ID" => Some("from-env".to_string()),
                _ => None,
            },
        );

        assert_eq!(resolved.value.as_deref(), Some("from-env"));
        assert_eq!(resolved.source, Some(ValueSource::Env));
    }

    #[test]
    fn auth_resolver_supports_keys_without_baked_defaults() {
        let key = AuthConfigKeySpec {
            config_key: "other_auth_key",
            env_key: "OTHER_AUTH_KEY",
            baked_default: None,
        };

        let resolved = resolve_optional_auth_config_value(key, None, &|_| None);

        assert_eq!(resolved.value, None);
        assert_eq!(resolved.source, None);
    }

    #[test]
    fn resolver_rejects_unknown_config_keys() {
        let req = ConfigRequest {
            report_format: ReportFormat::Text,
            config_path: Some(PathBuf::from("/tmp/config.json")),
            log_level: None,
            timeout_ms: None,
        };
        let error = resolve_runtime_config_with(
            &req,
            Path::new("/workspace"),
            |_| None,
            |_| Ok("{\"unknown\":true}".to_string()),
            |_| true,
            || Ok(PathBuf::from("/state")),
        )
        .expect_err("unknown config keys should fail");
        assert!(error.to_string().contains("contains unknown key 'unknown'"));
        assert!(error.to_string().contains("workos_client_id"));
    }

    #[test]
    fn resolver_merges_discovered_global_and_local_configs() -> Result<()> {
        let req = request();
        let resolved = resolve_runtime_config_with(
            &req,
            Path::new("/workspace"),
            |_| None,
            |path| {
                if path == Path::new("/state/sce/config.json") {
                    return Ok("{\"log_level\":\"error\",\"timeout_ms\":500,\"workos_client_id\":\"global-client\"}".to_string());
                }
                if path == Path::new("/workspace/.sce/config.json") {
                    return Ok(
                        "{\"timeout_ms\":700,\"workos_client_id\":\"local-client\"}".to_string()
                    );
                }
                Err(anyhow::anyhow!(
                    "unexpected config path: {}",
                    path.display()
                ))
            },
            |path| {
                path == Path::new("/state/sce/config.json")
                    || path == Path::new("/workspace/.sce/config.json")
            },
            || Ok(PathBuf::from("/state")),
        )?;

        assert_eq!(resolved.loaded_config_paths.len(), 2);
        assert_eq!(
            resolved.loaded_config_paths[0].source.as_str(),
            "default_discovered_global"
        );
        assert_eq!(
            resolved.loaded_config_paths[1].source.as_str(),
            "default_discovered_local"
        );

        assert_eq!(resolved.log_level.value, LogLevel::Error);
        assert_eq!(resolved.log_level.source.as_str(), "config_file");
        assert_eq!(
            resolved
                .log_level
                .source
                .config_source()
                .map(super::ConfigPathSource::as_str),
            Some("default_discovered_global")
        );

        assert_eq!(resolved.timeout_ms.value, 700);
        assert_eq!(resolved.timeout_ms.source.as_str(), "config_file");
        assert_eq!(
            resolved
                .timeout_ms
                .source
                .config_source()
                .map(super::ConfigPathSource::as_str),
            Some("default_discovered_local")
        );

        assert_eq!(
            resolved.workos_client_id.value.as_deref(),
            Some("local-client")
        );
        assert_eq!(
            resolved
                .workos_client_id
                .source
                .map(super::ValueSource::as_str),
            Some("config_file")
        );
        assert_eq!(
            resolved
                .workos_client_id
                .source
                .and_then(super::ValueSource::config_source)
                .map(super::ConfigPathSource::as_str),
            Some("default_discovered_local")
        );
        Ok(())
    }

    #[test]
    fn resolver_uses_global_workos_client_id_when_local_omits_key() -> Result<()> {
        let req = request();
        let resolved = resolve_runtime_config_with(
            &req,
            Path::new("/workspace"),
            |_| None,
            |path| {
                if path == Path::new("/state/sce/config.json") {
                    return Ok("{\"workos_client_id\":\"global-client\"}".to_string());
                }
                if path == Path::new("/workspace/.sce/config.json") {
                    return Ok("{}".to_string());
                }
                Err(anyhow::anyhow!(
                    "unexpected config path: {}",
                    path.display()
                ))
            },
            |path| {
                path == Path::new("/state/sce/config.json")
                    || path == Path::new("/workspace/.sce/config.json")
            },
            || Ok(PathBuf::from("/state")),
        )?;

        assert_eq!(
            resolved.workos_client_id.value.as_deref(),
            Some("global-client")
        );
        assert_eq!(
            resolved
                .workos_client_id
                .source
                .map(super::ValueSource::as_str),
            Some("config_file")
        );
        Ok(())
    }

    fn sample_runtime() -> RuntimeConfig {
        RuntimeConfig {
            loaded_config_paths: vec![
                LoadedConfigPath {
                    path: PathBuf::from("/state/sce/config.json"),
                    source: ConfigPathSource::DefaultDiscoveredGlobal,
                },
                LoadedConfigPath {
                    path: PathBuf::from("/workspace/.sce/config.json"),
                    source: ConfigPathSource::DefaultDiscoveredLocal,
                },
            ],
            log_level: ResolvedValue {
                value: LogLevel::Warn,
                source: ValueSource::Env,
            },
            timeout_ms: ResolvedValue {
                value: 1200,
                source: ValueSource::Flag,
            },
            workos_client_id: ResolvedOptionalValue {
                value: None,
                source: None,
            },
        }
    }

    #[test]
    fn show_json_output_is_deterministic_for_same_runtime() -> Result<()> {
        let runtime = sample_runtime();
        let first = format_show_output(&runtime, ReportFormat::Json);
        let second = format_show_output(&runtime, ReportFormat::Json);
        assert_eq!(first, second);

        let parsed: Value = serde_json::from_str(&first)?;
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["result"]["command"], "config_show");
        assert_eq!(
            parsed["result"]["precedence"],
            "flags > env > config file > defaults"
        );
        assert_eq!(parsed["result"]["resolved"]["log_level"]["source"], "env");
        assert_eq!(parsed["result"]["resolved"]["timeout_ms"]["source"], "flag");
        assert_eq!(
            parsed["result"]["resolved"]["workos_client_id"]["value"],
            Value::Null
        );
        assert_eq!(
            parsed["result"]["resolved"]["workos_client_id"]["source"],
            Value::Null
        );
        Ok(())
    }

    #[test]
    fn show_json_output_reports_workos_client_id_source_metadata() -> Result<()> {
        let runtime = RuntimeConfig {
            loaded_config_paths: vec![LoadedConfigPath {
                path: PathBuf::from("/workspace/.sce/config.json"),
                source: ConfigPathSource::DefaultDiscoveredLocal,
            }],
            log_level: ResolvedValue {
                value: LogLevel::Info,
                source: ValueSource::Default,
            },
            timeout_ms: ResolvedValue {
                value: 30000,
                source: ValueSource::Default,
            },
            workos_client_id: ResolvedOptionalValue {
                value: Some("local-client".to_string()),
                source: Some(ValueSource::ConfigFile(
                    ConfigPathSource::DefaultDiscoveredLocal,
                )),
            },
        };

        let parsed: Value =
            serde_json::from_str(&format_show_output(&runtime, ReportFormat::Json))?;
        assert_eq!(
            parsed["result"]["resolved"]["workos_client_id"]["value"],
            "local-client"
        );
        assert_eq!(
            parsed["result"]["resolved"]["workos_client_id"]["source"],
            "config_file"
        );
        assert_eq!(
            parsed["result"]["resolved"]["workos_client_id"]["config_source"],
            "default_discovered_local"
        );
        assert_eq!(
            parsed["result"]["resolved"]["workos_client_id"]["precedence"],
            "env (WORKOS_CLIENT_ID) > config file (workos_client_id) > baked default (client_sce_default)"
        );
        Ok(())
    }

    #[test]
    fn show_text_output_abbreviates_credential_like_auth_values() {
        let runtime = RuntimeConfig {
            loaded_config_paths: vec![],
            log_level: ResolvedValue {
                value: LogLevel::Info,
                source: ValueSource::Default,
            },
            timeout_ms: ResolvedValue {
                value: 30000,
                source: ValueSource::Default,
            },
            workos_client_id: ResolvedOptionalValue {
                value: Some("client_1234567890abcdef".to_string()),
                source: Some(ValueSource::Env),
            },
        };

        let output = format_show_output(&runtime, ReportFormat::Text);
        assert!(output.contains("workos_client_id: clie...cdef"));
        assert!(output.contains("auth_precedence: env (WORKOS_CLIENT_ID) > config file (workos_client_id) > baked default (client_sce_default)"));
    }

    #[test]
    fn validate_json_output_is_deterministic_for_same_runtime() -> Result<()> {
        let runtime = sample_runtime();
        let first = format_validate_output(&runtime, ReportFormat::Json);
        let second = format_validate_output(&runtime, ReportFormat::Json);
        assert_eq!(first, second);

        let parsed: Value = serde_json::from_str(&first)?;
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["result"]["command"], "config_validate");
        assert_eq!(parsed["result"]["valid"], true);
        assert!(parsed["result"]["issues"].as_array().is_some());
        assert!(parsed["result"]["resolved_auth"]["workos_client_id"].is_object());
        Ok(())
    }

    #[test]
    fn validate_text_output_reports_auth_precedence_and_source() {
        let runtime = RuntimeConfig {
            loaded_config_paths: vec![LoadedConfigPath {
                path: PathBuf::from("/workspace/.sce/config.json"),
                source: ConfigPathSource::DefaultDiscoveredLocal,
            }],
            log_level: ResolvedValue {
                value: LogLevel::Info,
                source: ValueSource::Default,
            },
            timeout_ms: ResolvedValue {
                value: 30000,
                source: ValueSource::Default,
            },
            workos_client_id: ResolvedOptionalValue {
                value: Some("local-client".to_string()),
                source: Some(ValueSource::ConfigFile(
                    ConfigPathSource::DefaultDiscoveredLocal,
                )),
            },
        };

        let output = format_validate_output(&runtime, ReportFormat::Text);
        assert!(output.contains("Resolved auth precedence: env (WORKOS_CLIENT_ID) > config file (workos_client_id) > baked default (client_sce_default)"));
        assert!(output.contains("workos_client_id: local-client (source: config_file, config_source: default_discovered_local"));
    }
}
