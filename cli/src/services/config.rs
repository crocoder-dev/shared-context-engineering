use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use anyhow::{anyhow, bail, Context, Result};
use jsonschema::{validator_for, Validator};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::services::default_paths::{resolve_sce_default_locations, schema, RepoPaths};
use crate::services::output_format::OutputFormat;
use crate::services::style::{self};

pub const NAME: &str = "config";
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const SCE_CONFIG_SCHEMA_JSON: &str =
    include_str!("../../assets/generated/config/schema/sce-config.schema.json");

const DEFAULT_TIMEOUT_MS: u64 = 30000;
const PRECEDENCE_DESCRIPTION: &str = "flags > env > config file > defaults";
const CONFIG_SCHEMA_DECLARATION_KEY: &str = "$schema";
const TOP_LEVEL_CONFIG_KEYS: &[&str] = &[
    CONFIG_SCHEMA_DECLARATION_KEY,
    "log_level",
    "log_format",
    "log_file",
    "log_file_mode",
    "otel",
    "timeout_ms",
    WORKOS_CLIENT_ID_KEY.config_key,
    "policies",
];
const TOP_LEVEL_CONFIG_KEYS_DESCRIPTION: &str =
    "$schema, log_level, log_format, log_file, log_file_mode, otel, timeout_ms, workos_client_id, policies";
pub(crate) const DEFAULT_OTEL_ENDPOINT: &str = "http://127.0.0.1:4317";
pub(crate) const ENV_LOG_LEVEL: &str = "SCE_LOG_LEVEL";
pub(crate) const ENV_LOG_FORMAT: &str = "SCE_LOG_FORMAT";
pub(crate) const ENV_LOG_FILE: &str = "SCE_LOG_FILE";
pub(crate) const ENV_LOG_FILE_MODE: &str = "SCE_LOG_FILE_MODE";
pub(crate) const ENV_OTEL_ENABLED: &str = "SCE_OTEL_ENABLED";
pub(crate) const ENV_OTEL_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
pub(crate) const ENV_OTEL_PROTOCOL: &str = "OTEL_EXPORTER_OTLP_PROTOCOL";
pub(crate) const ENV_ATTRIBUTION_HOOKS_ENABLED: &str = "SCE_ATTRIBUTION_HOOKS_ENABLED";
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
    pub(crate) fn parse(raw: &str, source: &str) -> Result<Self> {
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

    pub(crate) fn parse_env(raw: &str, key: &str) -> Result<Self> {
        match raw {
            "error" => Ok(Self::Error),
            "warn" => Ok(Self::Warn),
            "info" => Ok(Self::Info),
            "debug" => Ok(Self::Debug),
            _ => bail!("Invalid {key} '{raw}'. Valid values: error, warn, info, debug."),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
        }
    }

    pub(crate) fn severity(self) -> u8 {
        match self {
            Self::Error => 1,
            Self::Warn => 2,
            Self::Info => 3,
            Self::Debug => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogFormat {
    Text,
    Json,
}

impl LogFormat {
    pub(crate) fn parse(raw: &str, source: &str) -> Result<Self> {
        match raw {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => bail!("Invalid log format '{raw}' from {source}. Valid values: text, json."),
        }
    }

    pub(crate) fn parse_env(raw: &str, key: &str) -> Result<Self> {
        match raw {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => bail!("Invalid {key} '{raw}'. Valid values: text, json."),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogFileMode {
    Truncate,
    Append,
}

impl LogFileMode {
    pub(crate) fn parse(raw: &str, source: &str) -> Result<Self> {
        match raw {
            "truncate" => Ok(Self::Truncate),
            "append" => Ok(Self::Append),
            _ => bail!(
                "Invalid log file mode '{raw}' from {source}. Valid values: truncate, append."
            ),
        }
    }

    pub(crate) fn parse_env(raw: &str, key: &str) -> Result<Self> {
        match raw {
            "truncate" => Ok(Self::Truncate),
            "append" => Ok(Self::Append),
            _ => bail!("Invalid {key} '{raw}'. Valid values: truncate, append."),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Truncate => "truncate",
            Self::Append => "append",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OtlpProtocol {
    Grpc,
    HttpProtobuf,
}

impl OtlpProtocol {
    pub(crate) fn parse(raw: &str, source: &str) -> Result<Self> {
        match raw {
            "grpc" => Ok(Self::Grpc),
            "http/protobuf" => Ok(Self::HttpProtobuf),
            _ => bail!(
                "Invalid OTLP protocol '{raw}' from {source}. Valid values: grpc, http/protobuf."
            ),
        }
    }

    pub(crate) fn parse_env(raw: &str, key: &str) -> Result<Self> {
        match raw {
            "grpc" => Ok(Self::Grpc),
            "http/protobuf" => Ok(Self::HttpProtobuf),
            _ => bail!("Invalid {key} '{raw}'. Valid values: grpc, http/protobuf."),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Grpc => "grpc",
            Self::HttpProtobuf => "http/protobuf",
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
pub(crate) enum ConfigPathSource {
    Flag,
    Env,
    DefaultDiscoveredGlobal,
    DefaultDiscoveredLocal,
}

impl ConfigPathSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Env => "env",
            Self::DefaultDiscoveredGlobal => "default_discovered_global",
            Self::DefaultDiscoveredLocal => "default_discovered_local",
        }
    }

    const fn is_default_discovered(self) -> bool {
        matches!(
            self,
            Self::DefaultDiscoveredGlobal | Self::DefaultDiscoveredLocal
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResolvedValue<T> {
    value: T,
    source: ValueSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LoadedConfigPath {
    pub(crate) path: PathBuf,
    pub(crate) source: ConfigPathSource,
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
    log_format: ResolvedValue<LogFormat>,
    log_file: ResolvedOptionalValue<String>,
    log_file_mode: ResolvedValue<LogFileMode>,
    otel_enabled: ResolvedValue<bool>,
    otel_endpoint: ResolvedValue<String>,
    otel_protocol: ResolvedValue<OtlpProtocol>,
    timeout_ms: ResolvedValue<u64>,
    attribution_hooks_enabled: ResolvedValue<bool>,
    workos_client_id: ResolvedOptionalValue<String>,
    bash_policies: ResolvedOptionalValue<BashPolicyConfig>,
    validation_errors: Vec<String>,
    validation_warnings: Vec<String>,
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
pub(crate) struct ResolvedObservabilityRuntimeConfig {
    pub(crate) log_level: LogLevel,
    pub(crate) log_format: LogFormat,
    pub(crate) log_file: Option<String>,
    pub(crate) log_file_mode: LogFileMode,
    pub(crate) otel_enabled: bool,
    pub(crate) otel_endpoint: String,
    pub(crate) otel_protocol: OtlpProtocol,
    pub(crate) loaded_config_paths: Vec<LoadedConfigPath>,
    pub(crate) validation_errors: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedHookRuntimeConfig {
    pub(crate) attribution_hooks_enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileConfig {
    log_level: Option<FileConfigValue<LogLevel>>,
    log_format: Option<FileConfigValue<LogFormat>>,
    log_file: Option<FileConfigValue<String>>,
    log_file_mode: Option<FileConfigValue<LogFileMode>>,
    otel_enabled: Option<FileConfigValue<bool>>,
    otel_endpoint: Option<FileConfigValue<String>>,
    otel_protocol: Option<FileConfigValue<OtlpProtocol>>,
    timeout_ms: Option<FileConfigValue<u64>>,
    attribution_hooks_enabled: Option<FileConfigValue<bool>>,
    workos_client_id: Option<FileConfigValue<String>>,
    bash_policy_presets: Option<FileConfigValue<Vec<String>>>,
    bash_policy_custom: Option<FileConfigValue<Vec<CustomBashPolicyEntry>>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct ParsedFileConfigDocument {
    #[serde(rename = "$schema")]
    _schema: Option<String>,
    log_level: Option<String>,
    log_format: Option<String>,
    log_file: Option<String>,
    log_file_mode: Option<String>,
    otel: Option<ParsedOtelConfigDocument>,
    timeout_ms: Option<u64>,
    workos_client_id: Option<String>,
    policies: Option<ParsedPoliciesConfigDocument>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct ParsedOtelConfigDocument {
    enabled: Option<bool>,
    exporter_otlp_endpoint: Option<String>,
    exporter_otlp_protocol: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct ParsedPoliciesConfigDocument {
    bash: Option<ParsedBashPolicyConfigDocument>,
    attribution_hooks: Option<ParsedAttributionHooksConfigDocument>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct ParsedBashPolicyConfigDocument {
    presets: Option<Vec<String>>,
    custom: Option<Vec<ParsedCustomBashPolicyEntryDocument>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct ParsedAttributionHooksConfigDocument {
    enabled: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct ParsedCustomBashPolicyEntryDocument {
    id: Option<String>,
    #[serde(rename = "match")]
    matcher: Option<ParsedCustomBashPolicyMatchDocument>,
    message: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct ParsedCustomBashPolicyMatchDocument {
    argv_prefix: Option<Vec<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileConfigValue<T> {
    value: T,
    source: ConfigPathSource,
}

type ParsedBashPolicyConfig = (
    Option<FileConfigValue<Vec<String>>>,
    Option<FileConfigValue<Vec<CustomBashPolicyEntry>>>,
);
type ParsedFilePolicies = (
    Option<FileConfigValue<bool>>,
    Option<FileConfigValue<Vec<String>>>,
    Option<FileConfigValue<Vec<CustomBashPolicyEntry>>>,
);
type OtelFileConfig = (
    Option<FileConfigValue<bool>>,
    Option<FileConfigValue<String>>,
    Option<FileConfigValue<OtlpProtocol>>,
);

static BUILTIN_BASH_POLICY_CATALOG: OnceLock<BuiltinBashPolicyCatalog> = OnceLock::new();
static CONFIG_SCHEMA_VALIDATOR: OnceLock<Validator> = OnceLock::new();

const BASH_POLICY_PRESET_CATALOG_JSON: &str =
    include_str!("../../assets/generated/config/opencode/lib/bash-policy-presets.json");

#[derive(Clone, Debug, Eq, PartialEq)]
struct BashPolicyConfig {
    presets: Vec<String>,
    custom: Vec<CustomBashPolicyEntry>,
}

#[derive(Debug, Deserialize)]
struct BuiltinBashPolicyCatalog {
    presets: Vec<BuiltinBashPolicyPreset>,
    mutually_exclusive: Vec<Vec<String>>,
    redundancy_warnings: Vec<BuiltinBashPolicyRedundancyWarning>,
}

#[derive(Debug, Deserialize)]
struct BuiltinBashPolicyPreset {
    id: String,
    #[serde(rename = "match")]
    matcher: BuiltinBashPolicyMatcher,
    message: String,
}

#[derive(Debug, Deserialize)]
struct BuiltinBashPolicyMatcher {
    argv_prefixes: Vec<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct BuiltinBashPolicyRedundancyWarning {
    if_enabled: Vec<String>,
    warning: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CustomBashPolicyEntry {
    id: String,
    argv_prefix: Vec<String>,
    message: String,
}

impl CustomBashPolicyEntry {
    fn json_value(&self) -> Value {
        json!({
            "id": self.id,
            "match": {
                "argv_prefix": self.argv_prefix,
            },
            "message": self.message,
        })
    }

    fn text_summary(&self) -> String {
        format!(
            "{} => [{}] :: {}",
            self.id,
            self.argv_prefix.join(" "),
            self.message
        )
    }
}

fn builtin_bash_policy_catalog() -> &'static BuiltinBashPolicyCatalog {
    BUILTIN_BASH_POLICY_CATALOG.get_or_init(|| {
        let catalog: BuiltinBashPolicyCatalog =
            serde_json::from_str(BASH_POLICY_PRESET_CATALOG_JSON)
                .expect("bash policy preset catalog JSON must remain valid");
        debug_assert!(catalog.presets.iter().all(|preset| !preset.id.is_empty()
            && !preset.message.is_empty()
            && !preset.matcher.argv_prefixes.is_empty()));
        catalog
    })
}

fn builtin_bash_policy_preset_ids() -> Vec<&'static str> {
    builtin_bash_policy_catalog()
        .presets
        .iter()
        .map(|preset| preset.id.as_str())
        .collect()
}

fn is_builtin_bash_policy_preset_id(id: &str) -> bool {
    builtin_bash_policy_catalog()
        .presets
        .iter()
        .any(|preset| preset.id == id)
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
        otel_enabled: runtime.otel_enabled.value,
        otel_endpoint: runtime.otel_endpoint.value,
        otel_protocol: runtime.otel_protocol.value,
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

    let mut file_config = FileConfig {
        log_level: None,
        log_format: None,
        log_file: None,
        log_file_mode: None,
        otel_enabled: None,
        otel_endpoint: None,
        otel_protocol: None,
        timeout_ms: None,
        attribution_hooks_enabled: None,
        workos_client_id: None,
        bash_policy_presets: None,
        bash_policy_custom: None,
    };
    let mut validation_errors = Vec::new();
    for loaded_path in &loaded_config_paths {
        let raw = read_file(&loaded_path.path)?;
        let layer = match parse_file_config(&raw, &loaded_path.path, loaded_path.source) {
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
        if let Some(otel_enabled) = layer.otel_enabled {
            file_config.otel_enabled = Some(otel_enabled);
        }
        if let Some(otel_endpoint) = layer.otel_endpoint {
            file_config.otel_endpoint = Some(otel_endpoint);
        }
        if let Some(otel_protocol) = layer.otel_protocol {
            file_config.otel_protocol = Some(otel_protocol);
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

    let mut resolved_otel_enabled = ResolvedValue {
        value: false,
        source: ValueSource::Default,
    };
    if let Some(value) = file_config.otel_enabled {
        resolved_otel_enabled = ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        };
    }
    if let Some(raw) = env_lookup(ENV_OTEL_ENABLED) {
        resolved_otel_enabled = ResolvedValue {
            value: parse_bool_value_from(ENV_OTEL_ENABLED, &raw, ENV_OTEL_ENABLED)?,
            source: ValueSource::Env,
        };
    }

    let mut resolved_otel_endpoint = ResolvedValue {
        value: DEFAULT_OTEL_ENDPOINT.to_string(),
        source: ValueSource::Default,
    };
    if let Some(value) = file_config.otel_endpoint {
        resolved_otel_endpoint = ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        };
    }
    if let Some(raw) = env_lookup(ENV_OTEL_ENDPOINT) {
        resolved_otel_endpoint = ResolvedValue {
            value: raw,
            source: ValueSource::Env,
        };
    }

    let mut resolved_otel_protocol = ResolvedValue {
        value: OtlpProtocol::Grpc,
        source: ValueSource::Default,
    };
    if let Some(value) = file_config.otel_protocol {
        resolved_otel_protocol = ResolvedValue {
            value: value.value,
            source: ValueSource::ConfigFile(value.source),
        };
    }
    if let Some(raw) = env_lookup(ENV_OTEL_PROTOCOL) {
        resolved_otel_protocol = ResolvedValue {
            value: OtlpProtocol::parse(&raw, ENV_OTEL_PROTOCOL)?,
            source: ValueSource::Env,
        };
    }
    if resolved_otel_enabled.value {
        validate_otlp_endpoint(&resolved_otel_endpoint.value)?;
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
        otel_enabled: resolved_otel_enabled,
        otel_endpoint: resolved_otel_endpoint,
        otel_protocol: resolved_otel_protocol,
        timeout_ms: resolved_timeout_ms,
        attribution_hooks_enabled: resolved_attribution_hooks_enabled,
        workos_client_id: resolved_workos_client_id,
        bash_policies: resolved_bash_policies,
        validation_errors,
        validation_warnings,
    })
}

pub(crate) fn parse_bool_value_from(key: &str, raw: &str, source: &str) -> Result<bool> {
    match raw {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => bail!("Invalid {key} '{raw}' from {source}. Valid values: true, false, 1, 0."),
    }
}

pub(crate) fn parse_bool_env_value(key: &str, raw: &str) -> Result<bool> {
    match raw {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => bail!("Invalid {key} '{raw}'. Valid values: true, false, 1, 0."),
    }
}

pub(crate) fn validate_otlp_endpoint(endpoint: &str) -> Result<()> {
    if endpoint.is_empty() {
        bail!(
            "Invalid {ENV_OTEL_ENDPOINT} ''. Try: set it to an absolute http(s) URL, for example {DEFAULT_OTEL_ENDPOINT}."
        );
    }

    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        return Ok(());
    }

    bail!(
        "Invalid {ENV_OTEL_ENDPOINT} '{endpoint}'. Try: set it to an absolute http(s) URL, for example {DEFAULT_OTEL_ENDPOINT}."
    )
}

fn resolve_bash_policy_config(
    presets: Option<&FileConfigValue<Vec<String>>>,
    custom: Option<&FileConfigValue<Vec<CustomBashPolicyEntry>>>,
) -> ResolvedOptionalValue<BashPolicyConfig> {
    let resolved_presets = presets.map(|value| value.value.clone());
    let resolved_custom = custom.map(|value| value.value.clone());
    let source = custom
        .map(|value| value.source)
        .or_else(|| presets.map(|value| value.source));

    if resolved_presets.as_ref().is_none_or(Vec::is_empty)
        && resolved_custom.as_ref().is_none_or(Vec::is_empty)
    {
        return ResolvedOptionalValue {
            value: None,
            source: None,
        };
    }

    ResolvedOptionalValue {
        value: Some(BashPolicyConfig {
            presets: resolved_presets.unwrap_or_default(),
            custom: resolved_custom.unwrap_or_default(),
        }),
        source: source.map(ValueSource::ConfigFile),
    }
}

fn build_validation_warnings(value: &ResolvedOptionalValue<BashPolicyConfig>) -> Vec<String> {
    let Some(config) = value.value.as_ref() else {
        return Vec::new();
    };

    builtin_bash_policy_catalog()
        .redundancy_warnings
        .iter()
        .filter(|warning| {
            warning
                .if_enabled
                .iter()
                .all(|preset| config.presets.iter().any(|enabled| enabled == preset))
        })
        .map(|warning| warning.warning.clone())
        .collect()
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

pub(crate) fn validate_config_file(path: &Path) -> Result<()> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file '{}'.", path.display()))?;
    parse_file_config(&raw, path, ConfigPathSource::Flag)?;
    Ok(())
}

fn config_schema_validator() -> &'static Validator {
    CONFIG_SCHEMA_VALIDATOR.get_or_init(|| {
        let schema: Value =
            serde_json::from_str(SCE_CONFIG_SCHEMA_JSON).expect("config schema JSON should parse");
        validator_for(&schema).expect("config schema JSON should compile")
    })
}

fn generated_config_schema_path() -> String {
    format!("{}/{}", schema::SCHEMA_DIR, schema::SCE_CONFIG_SCHEMA)
}

fn validate_config_value_against_schema(value: &Value, path: &Path) -> Result<()> {
    let mut errors = config_schema_validator()
        .iter_errors(value)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();

    if errors.is_empty() {
        return Ok(());
    }

    errors.sort();
    let generated_schema_path = generated_config_schema_path();
    bail!(
        "Config file '{}' failed schema validation against generated schema '{}': {}",
        path.display(),
        generated_schema_path,
        errors.join(" | ")
    );
}

fn validate_object_keys(
    object: &serde_json::Map<String, Value>,
    path: &Path,
    context: Option<&str>,
    allowed_keys: &[&str],
    allowed_keys_description: &str,
) -> Result<()> {
    for key in object.keys() {
        if !allowed_keys.contains(&key.as_str()) {
            match context {
                Some(context) => bail!(
                    "Config key '{context}' in '{}' contains unknown key '{}'. Allowed keys: {allowed_keys_description}.",
                    path.display(),
                    key
                ),
                None => bail!(
                    "Config file '{}' contains unknown key '{}'. Allowed keys: {allowed_keys_description}.",
                    path.display(),
                    key
                ),
            }
        }
    }

    Ok(())
}

fn deserialize_typed_config(parsed: Value, path: &Path) -> Result<ParsedFileConfigDocument> {
    serde_json::from_value(parsed).with_context(|| {
        format!(
            "Config file '{}' could not be mapped into the typed runtime config model.",
            path.display()
        )
    })
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

    validate_config_value_against_schema(&parsed, path)?;
    validate_object_keys(
        object,
        path,
        None,
        TOP_LEVEL_CONFIG_KEYS,
        TOP_LEVEL_CONFIG_KEYS_DESCRIPTION,
    )?;

    let typed = deserialize_typed_config(parsed.clone(), path)?;
    let log_level = typed
        .log_level
        .map(|raw| -> Result<FileConfigValue<LogLevel>> {
            Ok(FileConfigValue {
                value: LogLevel::parse(&raw, &format!("config file '{}'", path.display()))?,
                source,
            })
        })
        .transpose()?;
    let log_format = typed
        .log_format
        .map(|raw| -> Result<FileConfigValue<LogFormat>> {
            Ok(FileConfigValue {
                value: LogFormat::parse(&raw, &format!("config file '{}'", path.display()))?,
                source,
            })
        })
        .transpose()?;
    let log_file = typed
        .log_file
        .map(|value| FileConfigValue { value, source });
    let log_file_mode = typed
        .log_file_mode
        .map(|raw| -> Result<FileConfigValue<LogFileMode>> {
            Ok(FileConfigValue {
                value: LogFileMode::parse(&raw, &format!("config file '{}'", path.display()))?,
                source,
            })
        })
        .transpose()?;
    let (otel_enabled, otel_endpoint, otel_protocol) =
        map_otel_config(typed.otel.as_ref(), object, path, source)?;
    let timeout_ms = typed
        .timeout_ms
        .map(|value| FileConfigValue { value, source });
    let workos_client_id = typed
        .workos_client_id
        .map(|value| FileConfigValue { value, source });
    let (attribution_hooks_enabled, bash_policy_presets, bash_policy_custom) =
        map_policies_config(typed.policies.as_ref(), object, path, source)?;

    Ok(FileConfig {
        log_level,
        log_format,
        log_file,
        log_file_mode,
        otel_enabled,
        otel_endpoint,
        otel_protocol,
        timeout_ms,
        attribution_hooks_enabled,
        workos_client_id,
        bash_policy_presets,
        bash_policy_custom,
    })
}

fn map_otel_config(
    typed: Option<&ParsedOtelConfigDocument>,
    object: &serde_json::Map<String, Value>,
    path: &Path,
    source: ConfigPathSource,
) -> Result<OtelFileConfig> {
    let Some(otel_value) = object.get("otel") else {
        return Ok((None, None, None));
    };

    let otel_object = otel_value.as_object().with_context(|| {
        format!(
            "Config key 'otel' in '{}' must be an object.",
            path.display()
        )
    })?;
    validate_object_keys(
        otel_object,
        path,
        Some("otel"),
        &[
            "enabled",
            "exporter_otlp_endpoint",
            "exporter_otlp_protocol",
        ],
        "enabled, exporter_otlp_endpoint, exporter_otlp_protocol",
    )?;

    let enabled = typed
        .and_then(|config| config.enabled)
        .map(|value| FileConfigValue { value, source });
    let endpoint = typed
        .and_then(|config| config.exporter_otlp_endpoint.clone())
        .map(|value| FileConfigValue { value, source });
    let protocol = typed
        .and_then(|config| config.exporter_otlp_protocol.as_deref())
        .map(|raw| -> Result<FileConfigValue<OtlpProtocol>> {
            Ok(FileConfigValue {
                value: OtlpProtocol::parse(raw, &format!("config file '{}'", path.display()))?,
                source,
            })
        })
        .transpose()?;

    Ok((enabled, endpoint, protocol))
}

fn map_policies_config(
    typed: Option<&ParsedPoliciesConfigDocument>,
    object: &serde_json::Map<String, Value>,
    path: &Path,
    source: ConfigPathSource,
) -> Result<ParsedFilePolicies> {
    let Some(policies_value) = object.get("policies") else {
        return Ok((None, None, None));
    };

    let policies_object = policies_value.as_object().with_context(|| {
        format!(
            "Config key 'policies' in '{}' must be an object.",
            path.display()
        )
    })?;

    validate_object_keys(
        policies_object,
        path,
        Some("policies"),
        &["bash", "attribution_hooks"],
        "bash, attribution_hooks",
    )?;

    let bash = typed.and_then(|config| config.bash.as_ref());
    let attribution_hooks_enabled = map_attribution_hooks_config(
        typed.and_then(|config| config.attribution_hooks.as_ref()),
        policies_object,
        path,
        source,
    )?;
    let (bash_policy_presets, bash_policy_custom) =
        map_bash_policy_config(bash, policies_object, path, source)?;

    Ok((
        attribution_hooks_enabled,
        bash_policy_presets,
        bash_policy_custom,
    ))
}

fn map_attribution_hooks_config(
    typed: Option<&ParsedAttributionHooksConfigDocument>,
    policies_object: &serde_json::Map<String, Value>,
    path: &Path,
    source: ConfigPathSource,
) -> Result<Option<FileConfigValue<bool>>> {
    let Some(attribution_hooks_value) = policies_object.get("attribution_hooks") else {
        return Ok(None);
    };

    let attribution_hooks_object = attribution_hooks_value.as_object().with_context(|| {
        format!(
            "Config key 'policies.attribution_hooks' in '{}' must be an object.",
            path.display()
        )
    })?;

    validate_object_keys(
        attribution_hooks_object,
        path,
        Some("policies.attribution_hooks"),
        &["enabled"],
        "enabled",
    )?;

    Ok(typed
        .and_then(|config| config.enabled)
        .map(|value| FileConfigValue { value, source }))
}

fn map_bash_policy_config(
    typed: Option<&ParsedBashPolicyConfigDocument>,
    policies_object: &serde_json::Map<String, Value>,
    path: &Path,
    source: ConfigPathSource,
) -> Result<ParsedBashPolicyConfig> {
    let Some(bash_value) = policies_object.get("bash") else {
        return Ok((None, None));
    };

    let bash_object = bash_value.as_object().with_context(|| {
        format!(
            "Config key 'policies.bash' in '{}' must be an object.",
            path.display()
        )
    })?;

    validate_object_keys(
        bash_object,
        path,
        Some("policies.bash"),
        &["presets", "custom"],
        "presets, custom",
    )?;

    let presets = typed
        .and_then(|config| config.presets.as_ref())
        .map(|presets| parse_bash_policy_presets(presets, path))
        .transpose()?
        .map(|value| FileConfigValue { value, source });
    let custom = typed
        .and_then(|config| config.custom.as_ref())
        .map(|custom| parse_custom_bash_policies(custom, path))
        .transpose()?
        .map(|value| FileConfigValue { value, source });

    Ok((presets, custom))
}

fn parse_bash_policy_presets(items: &[String], path: &Path) -> Result<Vec<String>> {
    let mut presets = Vec::with_capacity(items.len());
    let builtin_preset_ids = builtin_bash_policy_preset_ids();
    for item in items {
        let preset = item.as_str();
        if !builtin_preset_ids.contains(&preset) {
            bail!(
                "Config key 'policies.bash.presets' in '{}' contains unknown preset '{}'. Allowed presets: {}.",
                path.display(),
                preset,
                builtin_preset_ids.join(", ")
            );
        }
        if presets.iter().any(|existing| existing == preset) {
            bail!(
                "Config key 'policies.bash.presets' in '{}' contains duplicate preset '{}'.",
                path.display(),
                preset
            );
        }
        presets.push(preset.to_string());
    }

    for conflict_group in &builtin_bash_policy_catalog().mutually_exclusive {
        if conflict_group
            .iter()
            .all(|preset| presets.iter().any(|enabled| enabled == preset))
        {
            let joined = conflict_group
                .iter()
                .map(|preset| format!("'{preset}'"))
                .collect::<Vec<_>>()
                .join(" and ");
            bail!(
                "Config key 'policies.bash.presets' in '{}' cannot enable both {}.",
                path.display(),
                joined
            );
        }
    }

    Ok(presets)
}

fn parse_custom_bash_policies(
    items: &[ParsedCustomBashPolicyEntryDocument],
    path: &Path,
) -> Result<Vec<CustomBashPolicyEntry>> {
    let mut policies = Vec::with_capacity(items.len());
    let mut argv_prefixes: Vec<Vec<String>> = Vec::new();
    for item in items {
        let policy = parse_custom_bash_policy_entry(item, path)?;
        if policies
            .iter()
            .any(|existing: &CustomBashPolicyEntry| existing.id == policy.id)
        {
            bail!(
                "Config key 'policies.bash.custom' in '{}' contains duplicate id '{}'.",
                path.display(),
                policy.id
            );
        }

        if argv_prefixes
            .iter()
            .any(|existing| existing == &policy.argv_prefix)
        {
            bail!(
                "Config key 'policies.bash.custom' in '{}' contains duplicate argv_prefix [{}].",
                path.display(),
                policy.argv_prefix.join(" ")
            );
        }
        argv_prefixes.push(policy.argv_prefix.clone());
        policies.push(policy);
    }

    Ok(policies)
}

fn parse_custom_bash_policy_entry(
    item: &ParsedCustomBashPolicyEntryDocument,
    path: &Path,
) -> Result<CustomBashPolicyEntry> {
    let id = item
        .id
        .as_deref()
        .with_context(|| {
            format!(
                "Each 'policies.bash.custom' entry in '{}' must include string field 'id'.",
                path.display()
            )
        })?
        .to_string();
    if is_builtin_bash_policy_preset_id(&id) {
        bail!(
            "Custom bash policy id '{}' in '{}' collides with a built-in preset id.",
            id,
            path.display()
        );
    }

    let message = item.message.as_deref().with_context(|| {
        format!(
            "Custom bash policy '{}' in '{}' must include string field 'message'.",
            id,
            path.display()
        )
    })?;
    if message.is_empty() {
        bail!(
            "Custom bash policy '{}' in '{}' must use a non-empty 'message'.",
            id,
            path.display()
        );
    }

    let argv_prefix = parse_custom_bash_policy_match(&id, item.matcher.as_ref(), path)?;

    Ok(CustomBashPolicyEntry {
        id,
        argv_prefix,
        message: message.to_string(),
    })
}

fn parse_custom_bash_policy_match(
    id: &str,
    matcher: Option<&ParsedCustomBashPolicyMatchDocument>,
    path: &Path,
) -> Result<Vec<String>> {
    let matcher = matcher.with_context(|| {
        format!(
            "Custom bash policy '{}' in '{}' must include object field 'match'.",
            id,
            path.display()
        )
    })?;
    let argv_prefix_values = matcher.argv_prefix.as_deref().with_context(|| {
        format!(
            "Custom bash policy '{}' in '{}' must include array field 'match.argv_prefix'.",
            id,
            path.display()
        )
    })?;
    if argv_prefix_values.is_empty() {
        bail!(
            "Custom bash policy '{}' in '{}' must use a non-empty 'match.argv_prefix'.",
            id,
            path.display()
        );
    }

    parse_custom_bash_policy_argv_prefix(id, argv_prefix_values, path)
}

fn parse_custom_bash_policy_argv_prefix(
    id: &str,
    argv_prefix_values: &[String],
    path: &Path,
) -> Result<Vec<String>> {
    let mut argv_prefix = Vec::with_capacity(argv_prefix_values.len());
    for token in argv_prefix_values {
        if token.is_empty() {
            bail!(
                "Custom bash policy '{}' in '{}' cannot use empty argv_prefix tokens.",
                id,
                path.display()
            );
        }
        argv_prefix.push(token.clone());
    }

    Ok(argv_prefix)
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
                        "otel": format_otel_resolved_json(runtime),
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

fn format_bash_policies_text(value: &ResolvedOptionalValue<BashPolicyConfig>) -> String {
    match (value.value.as_ref(), value.source) {
        (Some(config), Some(source)) => {
            let presets = if config.presets.is_empty() {
                String::from("(none)")
            } else {
                config.presets.join(", ")
            };
            let custom = if config.custom.is_empty() {
                String::from("(none)")
            } else {
                config
                    .custom
                    .iter()
                    .map(CustomBashPolicyEntry::text_summary)
                    .collect::<Vec<_>>()
                    .join(" | ")
            };
            match source.config_source() {
                Some(config_source) => format!(
                    "- {}: presets=[{}]; custom=[{}] (source: {}, config_source: {})",
                    style::label("policies.bash"),
                    style::value(&presets),
                    style::value(&custom),
                    style::label(source.as_str()),
                    style::label(config_source.as_str())
                ),
                None => format!(
                    "- {}: presets=[{}]; custom=[{}] (source: {})",
                    style::label("policies.bash"),
                    style::value(&presets),
                    style::value(&custom),
                    style::label(source.as_str())
                ),
            }
        }
        _ => format!(
            "- {}: {} (source: {})",
            style::label("policies.bash"),
            style::value("(unset)"),
            style::label("none")
        ),
    }
}

fn format_bash_policies_json(value: &ResolvedOptionalValue<BashPolicyConfig>) -> Value {
    let config = value.value.as_ref();
    json!({
        "presets": config.map(|bash| bash.presets.clone()),
        "custom": config.map(|bash| bash.custom.iter().map(CustomBashPolicyEntry::json_value).collect::<Vec<_>>()),
        "source": value.source.map(ValueSource::as_str),
        "config_source": value.source.and_then(ValueSource::config_source).map(ConfigPathSource::as_str),
    })
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
        format_resolved_value_text(
            "otel.enabled",
            bool_text(runtime.otel_enabled.value),
            runtime.otel_enabled.source,
        ),
        format_resolved_value_text(
            "otel.exporter_otlp_endpoint",
            runtime.otel_endpoint.value.as_str(),
            runtime.otel_endpoint.source,
        ),
        format_resolved_value_text(
            "otel.exporter_otlp_protocol",
            runtime.otel_protocol.value.as_str(),
            runtime.otel_protocol.source,
        ),
    ]
}

fn format_otel_resolved_json(runtime: &RuntimeConfig) -> Value {
    json!({
        "enabled": format_resolved_value_json(
            runtime.otel_enabled.value,
            runtime.otel_enabled.source,
        ),
        "exporter_otlp_endpoint": format_resolved_value_json(
            runtime.otel_endpoint.value.as_str(),
            runtime.otel_endpoint.source,
        ),
        "exporter_otlp_protocol": format_resolved_value_json(
            runtime.otel_protocol.value.as_str(),
            runtime.otel_protocol.source,
        ),
    })
}

const fn bool_text(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
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
