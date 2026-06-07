pub mod command;
pub mod lifecycle;
pub mod policy;
pub mod schema;
pub mod types;

pub use types::*;

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::services::default_paths::{resolve_sce_default_locations, RepoPaths};
use crate::services::style;

use policy::{
    build_validation_warnings, format_bash_policies_json, format_bash_policies_text,
    resolve_bash_policy_config, BashPolicyConfig,
};

pub(crate) use schema::validate_config_file;

const DEFAULT_TIMEOUT_MS: u64 = 30000;
const PRECEDENCE_DESCRIPTION: &str = "flags > env > config file > defaults";
const WORKOS_CLIENT_ID_ENV: &str = "WORKOS_CLIENT_ID";
const WORKOS_CLIENT_ID_BAKED_DEFAULT: &str = "client_sce_default";
pub(crate) const WORKOS_CLIENT_ID_KEY: AuthConfigKeySpec = AuthConfigKeySpec {
    config_key: "workos_client_id",
    env_key: WORKOS_CLIENT_ID_ENV,
    baked_default: Some(WORKOS_CLIENT_ID_BAKED_DEFAULT),
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AuthConfigKeySpec {
    pub(crate) config_key: &'static str,
    pub(crate) env_key: &'static str,
    pub(crate) baked_default: Option<&'static str>,
}

impl AuthConfigKeySpec {
    pub(crate) fn precedence_description(self) -> String {
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
    log_format: ResolvedValue<LogFormat>,
    log_file: ResolvedOptionalValue<String>,
    log_file_mode: ResolvedValue<LogFileMode>,
    timeout_ms: ResolvedValue<u64>,
    attribution_hooks_enabled: ResolvedValue<bool>,
    workos_client_id: ResolvedOptionalValue<String>,
    bash_policies: ResolvedOptionalValue<BashPolicyConfig>,
    validation_errors: Vec<String>,
    validation_warnings: Vec<String>,
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

pub(crate) fn resolve_observability_runtime_config(
    cwd: &Path,
) -> Result<ResolvedObservabilityRuntimeConfig> {
    resolve_observability_runtime_config_with(
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

pub(crate) fn resolve_hook_runtime_config(cwd: &Path) -> Result<ResolvedHookRuntimeConfig> {
    resolve_hook_runtime_config_with(
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

pub(crate) fn resolve_observability_runtime_config_with<FEnv, FRead, FGlobalPath>(
    cwd: &Path,
    env_lookup: FEnv,
    read_file: FRead,
    path_exists: fn(&Path) -> bool,
    resolve_global_config_path: FGlobalPath,
) -> Result<ResolvedObservabilityRuntimeConfig>
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

    Ok(ResolvedObservabilityRuntimeConfig {
        log_level: runtime.log_level.value,
        log_format: runtime.log_format.value,
        log_file: runtime.log_file.value,
        log_file_mode: runtime.log_file_mode.value,
        loaded_config_paths: runtime.loaded_config_paths,
        validation_errors: runtime.validation_errors,
    })
}

pub(crate) fn resolve_hook_runtime_config_with<FEnv, FRead, FGlobalPath>(
    cwd: &Path,
    env_lookup: FEnv,
    read_file: FRead,
    path_exists: fn(&Path) -> bool,
    resolve_global_config_path: FGlobalPath,
) -> Result<ResolvedHookRuntimeConfig>
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

    Ok(ResolvedHookRuntimeConfig {
        attribution_hooks_enabled: runtime.attribution_hooks_enabled.value,
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

#[allow(clippy::too_many_lines)]
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

    let mut file_config = schema::FileConfig {
        log_level: None,
        log_format: None,
        log_file: None,
        log_file_mode: None,
        timeout_ms: None,
        attribution_hooks_enabled: None,
        workos_client_id: None,
        bash_policy_presets: None,
        bash_policy_custom: None,
    };
    let mut validation_errors = Vec::new();
    for loaded_path in &loaded_config_paths {
        let raw = read_file(&loaded_path.path)?;
        let layer = match schema::parse_file_config(&raw, &loaded_path.path, loaded_path.source) {
            Ok(layer) => layer,
            Err(error) if loaded_path.source.is_default_discovered() => {
                validation_errors.push(error.to_string());
                continue;
            }
            Err(error) => return Err(error),
        };
        if let Some(log_level) = layer.log_level {
            file_config.log_level = Some(log_level);
        }
        if let Some(log_format) = layer.log_format {
            file_config.log_format = Some(log_format);
        }
        if let Some(log_file) = layer.log_file {
            file_config.log_file = Some(log_file);
        }
        if let Some(log_file_mode) = layer.log_file_mode {
            file_config.log_file_mode = Some(log_file_mode);
        }
        if let Some(timeout_ms) = layer.timeout_ms {
            file_config.timeout_ms = Some(timeout_ms);
        }
        if let Some(attribution_hooks_enabled) = layer.attribution_hooks_enabled {
            file_config.attribution_hooks_enabled = Some(attribution_hooks_enabled);
        }
        if let Some(workos_client_id) = layer.workos_client_id {
            file_config.workos_client_id = Some(workos_client_id);
        }
        if let Some(bash_policy_presets) = layer.bash_policy_presets {
            file_config.bash_policy_presets = Some(bash_policy_presets);
        }
        if let Some(bash_policy_custom) = layer.bash_policy_custom {
            file_config.bash_policy_custom = Some(bash_policy_custom);
        }
    }

    let mut resolved_log_level = ResolvedValue {
        value: LogLevel::Error,
        source: ValueSource::Default,
    };
    if let Some(value) = file_config.log_level {
        resolved_log_level = ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        };
    }
    if let Some(raw) = env_lookup(ENV_LOG_LEVEL) {
        resolved_log_level = ResolvedValue {
            value: LogLevel::parse(&raw, ENV_LOG_LEVEL)?,
            source: ValueSource::Env,
        };
    }
    if let Some(value) = request.log_level {
        resolved_log_level = ResolvedValue {
            value,
            source: ValueSource::Flag,
        };
    }

    let mut resolved_log_format = ResolvedValue {
        value: LogFormat::Text,
        source: ValueSource::Default,
    };
    if let Some(value) = file_config.log_format {
        resolved_log_format = ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        };
    }
    if let Some(raw) = env_lookup(ENV_LOG_FORMAT) {
        resolved_log_format = ResolvedValue {
            value: LogFormat::parse(&raw, ENV_LOG_FORMAT)?,
            source: ValueSource::Env,
        };
    }

    let mut resolved_log_file = ResolvedOptionalValue {
        value: file_config
            .log_file
            .as_ref()
            .map(|value| value.value.clone()),
        source: file_config
            .log_file
            .as_ref()
            .map(|value| ValueSource::ConfigFile(value.source)),
    };
    if let Some(raw) = env_lookup(ENV_LOG_FILE) {
        resolved_log_file = ResolvedOptionalValue {
            value: Some(raw),
            source: Some(ValueSource::Env),
        };
    }

    let mut resolved_log_file_mode = ResolvedValue {
        value: LogFileMode::Truncate,
        source: ValueSource::Default,
    };
    if let Some(value) = file_config.log_file_mode {
        resolved_log_file_mode = ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        };
    }
    if let Some(raw) = env_lookup(ENV_LOG_FILE_MODE) {
        resolved_log_file_mode = ResolvedValue {
            value: LogFileMode::parse(&raw, ENV_LOG_FILE_MODE)?,
            source: ValueSource::Env,
        };
    }
    if resolved_log_file.value.is_none() && resolved_log_file_mode.source != ValueSource::Default {
        bail!(
            "{ENV_LOG_FILE_MODE} requires {ENV_LOG_FILE}. Try: set {ENV_LOG_FILE} to a file path or unset {ENV_LOG_FILE_MODE}."
        );
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

    let mut resolved_attribution_hooks_enabled = ResolvedValue {
        value: false,
        source: ValueSource::Default,
    };
    if let Some(value) = file_config.attribution_hooks_enabled {
        resolved_attribution_hooks_enabled = ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        };
    }
    if let Some(raw) = env_lookup(ENV_ATTRIBUTION_HOOKS_ENABLED) {
        resolved_attribution_hooks_enabled = ResolvedValue {
            value: parse_bool_value_from(
                ENV_ATTRIBUTION_HOOKS_ENABLED,
                &raw,
                ENV_ATTRIBUTION_HOOKS_ENABLED,
            )?,
            source: ValueSource::Env,
        };
    }

    let resolved_workos_client_id = resolve_optional_auth_config_value(
        WORKOS_CLIENT_ID_KEY,
        file_config.workos_client_id,
        &env_lookup,
    );

    let resolved_bash_policies = resolve_bash_policy_config(
        file_config.bash_policy_presets.as_ref(),
        file_config.bash_policy_custom.as_ref(),
    );
    let validation_warnings = build_validation_warnings(&resolved_bash_policies);

    Ok(RuntimeConfig {
        loaded_config_paths,
        log_level: resolved_log_level,
        log_format: resolved_log_format,
        log_file: resolved_log_file,
        log_file_mode: resolved_log_file_mode,
        timeout_ms: resolved_timeout_ms,
        attribution_hooks_enabled: resolved_attribution_hooks_enabled,
        workos_client_id: resolved_workos_client_id,
        bash_policies: resolved_bash_policies,
        validation_errors,
        validation_warnings,
    })
}

fn resolve_optional_auth_config_value<FEnv>(
    key: AuthConfigKeySpec,
    file_value: Option<schema::FileConfigValue<String>>,
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

    let global_path = resolve_global_config_path()?;
    if path_exists(&global_path) {
        discovered_paths.push(LoadedConfigPath {
            path: global_path,
            source: ConfigPathSource::DefaultDiscoveredGlobal,
        });
    }

    let local_path = RepoPaths::new(cwd).sce_config_file();
    if path_exists(&local_path) {
        discovered_paths.push(LoadedConfigPath {
            path: local_path,
            source: ConfigPathSource::DefaultDiscoveredLocal,
        });
    }

    Ok(discovered_paths)
}

fn resolve_default_global_config_path() -> Result<PathBuf> {
    Ok(resolve_sce_default_locations()?.global_config_file())
}

fn format_show_output(runtime: &RuntimeConfig, report_format: ReportFormat) -> String {
    let warnings = build_show_warnings(runtime);
    match report_format {
        ReportFormat::Text => {
            let mut lines = vec![
                format!(
                    "{}: {}",
                    style::success("SCE config"),
                    style::value("resolved")
                ),
                format!(
                    "{}: {}",
                    style::label("Precedence"),
                    style::value(PRECEDENCE_DESCRIPTION)
                ),
                format_config_paths_text(runtime),
                format_resolved_value_text(
                    "timeout_ms",
                    &runtime.timeout_ms.value.to_string(),
                    runtime.timeout_ms.source,
                ),
                format_optional_auth_resolved_value_text(
                    WORKOS_CLIENT_ID_KEY,
                    &runtime.workos_client_id,
                ),
                format_bash_policies_text(&runtime.bash_policies),
                format_validation_warnings_text(&warnings),
            ];
            lines.splice(3..3, format_observability_text_lines(runtime));
            lines.join("\n")
        }
        ReportFormat::Json => {
            let payload = json!({
                "status": "ok",
                "result": {
                    "command": "config_show",
                    "precedence": PRECEDENCE_DESCRIPTION,
                    "config_paths": format_config_paths_json(runtime),
                    "resolved": {
                        "log_level": format_resolved_value_json(
                            runtime.log_level.value.as_str(),
                            runtime.log_level.source,
                        ),
                        "log_format": format_resolved_value_json(
                            runtime.log_format.value.as_str(),
                            runtime.log_format.source,
                        ),
                        "log_file": format_optional_resolved_value_json(&runtime.log_file),
                        "log_file_mode": format_resolved_value_json(
                            runtime.log_file_mode.value.as_str(),
                            runtime.log_file_mode.source,
                        ),
                        "timeout_ms": {
                            "value": runtime.timeout_ms.value,
                            "source": runtime.timeout_ms.source.as_str(),
                            "config_source": runtime.timeout_ms.source.config_source().map(ConfigPathSource::as_str),
                        },
                        "workos_client_id": format_optional_auth_resolved_value_json(WORKOS_CLIENT_ID_KEY, &runtime.workos_client_id),
                        "policies": {
                            "bash": format_bash_policies_json(&runtime.bash_policies),
                        }
                    },
                    "warnings": warnings,
                }
            });
            serde_json::to_string_pretty(&payload).expect("config show payload should serialize")
        }
    }
}

fn format_validate_output(runtime: &RuntimeConfig, report_format: ReportFormat) -> String {
    let valid = runtime.validation_errors.is_empty();
    match report_format {
        ReportFormat::Text => {
            let lines = [
                format!(
                    "{}: {}",
                    style::success("SCE config validation"),
                    style::value(if valid { "valid" } else { "invalid" })
                ),
                format_validation_issues_text(&runtime.validation_errors),
                format_validation_warnings_text(&runtime.validation_warnings),
            ];
            lines.join("\n")
        }
        ReportFormat::Json => {
            let payload = json!({
                "status": "ok",
                "result": {
                    "command": "config_validate",
                    "valid": valid,
                    "issues": runtime.validation_errors,
                    "warnings": runtime.validation_warnings,
                }
            });
            serde_json::to_string_pretty(&payload)
                .expect("config validate payload should serialize")
        }
    }
}

fn format_config_paths_text(runtime: &RuntimeConfig) -> String {
    if runtime.loaded_config_paths.is_empty() {
        return format!(
            "{}: {}",
            style::label("Config files"),
            style::value("(none discovered)")
        );
    }

    let mut lines = vec![format!("{}:", style::label("Config files"))];
    for path in &runtime.loaded_config_paths {
        lines.push(format!(
            "  - {} (source: {})",
            style::value(&path.path.display().to_string()),
            style::label(path.source.as_str())
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

fn build_show_warnings(runtime: &RuntimeConfig) -> Vec<String> {
    let mut warnings = runtime
        .validation_errors
        .iter()
        .map(|error| format!("Skipped invalid config: {error}"))
        .collect::<Vec<_>>();
    warnings.extend(runtime.validation_warnings.iter().cloned());
    warnings
}

fn format_validation_issues_text(issues: &[String]) -> String {
    if issues.is_empty() {
        return format!(
            "{}: {}",
            style::label("Validation issues"),
            style::value("none")
        );
    }

    format!(
        "{}: {}",
        style::label("Validation issues"),
        style::value(&issues.join(" | "))
    )
}

fn format_validation_warnings_text(warnings: &[String]) -> String {
    if warnings.is_empty() {
        return format!(
            "{}: {}",
            style::label("Validation warnings"),
            style::value("none")
        );
    }

    format!(
        "{}: {}",
        style::label("Validation warnings"),
        style::value(&warnings.join(" | "))
    )
}

fn format_observability_text_lines(runtime: &RuntimeConfig) -> Vec<String> {
    vec![
        format_resolved_value_text(
            "log_level",
            runtime.log_level.value.as_str(),
            runtime.log_level.source,
        ),
        format_resolved_value_text(
            "log_format",
            runtime.log_format.value.as_str(),
            runtime.log_format.source,
        ),
        format_optional_resolved_value_text("log_file", &runtime.log_file),
        format_resolved_value_text(
            "log_file_mode",
            runtime.log_file_mode.value.as_str(),
            runtime.log_file_mode.source,
        ),
    ]
}

fn format_resolved_value_json<T>(value: T, source: ValueSource) -> Value
where
    T: serde::Serialize,
{
    json!({
        "value": value,
        "source": source.as_str(),
        "config_source": source.config_source().map(ConfigPathSource::as_str),
    })
}

fn format_resolved_value_text(key: &str, value_text: &str, source: ValueSource) -> String {
    match source.config_source() {
        Some(config_source) => format!(
            "- {}: {} (source: {}, config_source: {})",
            style::label(key),
            style::value(value_text),
            style::label(source.as_str()),
            style::label(config_source.as_str())
        ),
        None => format!(
            "- {}: {} (source: {})",
            style::label(key),
            style::value(value_text),
            style::label(source.as_str())
        ),
    }
}

fn format_optional_resolved_value_text(key: &str, value: &ResolvedOptionalValue<String>) -> String {
    match (value.value.as_deref(), value.source) {
        (Some(raw_value), Some(source)) => match source.config_source() {
            Some(config_source) => format!(
                "- {}: {} (source: {}, config_source: {})",
                style::label(key),
                style::value(raw_value),
                style::label(source.as_str()),
                style::label(config_source.as_str())
            ),
            None => format!(
                "- {}: {} (source: {})",
                style::label(key),
                style::value(raw_value),
                style::label(source.as_str())
            ),
        },
        _ => format!(
            "- {}: {} (source: {})",
            style::label(key),
            style::value("(unset)"),
            style::label("none")
        ),
    }
}

fn format_optional_resolved_value_json(value: &ResolvedOptionalValue<String>) -> Value {
    json!({
        "value": value.value,
        "source": value.source.map(ValueSource::as_str),
        "config_source": value.source.and_then(ValueSource::config_source).map(ConfigPathSource::as_str),
    })
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
                    style::label(key.config_key),
                    style::value(&display_value),
                    style::label(source.as_str()),
                    style::label(config_source.as_str()),
                    style::value(&key.precedence_description())
                ),
                None => format!(
                    "- {}: {} (source: {}, auth_precedence: {})",
                    style::label(key.config_key),
                    style::value(&display_value),
                    style::label(source.as_str()),
                    style::value(&key.precedence_description())
                ),
            }
        }
        _ => format!(
            "- {}: {} (source: {}, auth_precedence: {})",
            style::label(key.config_key),
            style::value("(unset)"),
            style::label("none"),
            style::value(&key.precedence_description())
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
        return String::from("[REDACTED]");
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
