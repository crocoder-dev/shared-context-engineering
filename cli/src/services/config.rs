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
const DEFAULT_OTEL_ENDPOINT: &str = "http://127.0.0.1:4317";
const ENV_LOG_LEVEL: &str = "SCE_LOG_LEVEL";
const ENV_LOG_FORMAT: &str = "SCE_LOG_FORMAT";
const ENV_LOG_FILE: &str = "SCE_LOG_FILE";
const ENV_LOG_FILE_MODE: &str = "SCE_LOG_FILE_MODE";
const ENV_OTEL_ENABLED: &str = "SCE_OTEL_ENABLED";
const ENV_OTEL_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
const ENV_OTEL_PROTOCOL: &str = "OTEL_EXPORTER_OTLP_PROTOCOL";
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
pub(crate) enum LogFormat {
    Text,
    Json,
}

impl LogFormat {
    fn parse(raw: &str, source: &str) -> Result<Self> {
        match raw {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => bail!("Invalid log format '{raw}' from {source}. Valid values: text, json."),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LogFileMode {
    Truncate,
    Append,
}

impl LogFileMode {
    fn parse(raw: &str, source: &str) -> Result<Self> {
        match raw {
            "truncate" => Ok(Self::Truncate),
            "append" => Ok(Self::Append),
            _ => bail!(
                "Invalid log file mode '{raw}' from {source}. Valid values: truncate, append."
            ),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Truncate => "truncate",
            Self::Append => "append",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OtlpProtocol {
    Grpc,
    HttpProtobuf,
}

impl OtlpProtocol {
    fn parse(raw: &str, source: &str) -> Result<Self> {
        match raw {
            "grpc" => Ok(Self::Grpc),
            "http/protobuf" => Ok(Self::HttpProtobuf),
            _ => bail!(
                "Invalid OTLP protocol '{raw}' from {source}. Valid values: grpc, http/protobuf."
            ),
        }
    }

    fn as_str(self) -> &'static str {
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
    workos_client_id: ResolvedOptionalValue<String>,
    bash_policies: ResolvedOptionalValue<BashPolicyConfig>,
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
    workos_client_id: Option<FileConfigValue<String>>,
    bash_policy_presets: Option<FileConfigValue<Vec<String>>>,
    bash_policy_custom: Option<FileConfigValue<Vec<CustomBashPolicyEntry>>>,
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
        workos_client_id: None,
        bash_policy_presets: None,
        bash_policy_custom: None,
    };
    for loaded_path in &loaded_config_paths {
        let raw = read_file(&loaded_path.path)?;
        let layer = parse_file_config(&raw, &loaded_path.path, loaded_path.source)?;
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
            value: parse_bool_value(ENV_OTEL_ENABLED, &raw, ENV_OTEL_ENABLED)?,
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
        workos_client_id: resolved_workos_client_id,
        bash_policies: resolved_bash_policies,
        validation_warnings,
    })
}

fn parse_bool_value(key: &str, raw: &str, source: &str) -> Result<bool> {
    match raw {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => bail!("Invalid {key} '{raw}' from {source}. Valid values: true, false, 1, 0."),
    }
}

fn validate_otlp_endpoint(endpoint: &str) -> Result<()> {
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

#[allow(clippy::too_many_lines)]
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

    for key in object.keys() {
        if !TOP_LEVEL_CONFIG_KEYS.contains(&key.as_str()) {
            bail!(
                "Config file '{}' contains unknown key '{}'. Allowed keys: {TOP_LEVEL_CONFIG_KEYS_DESCRIPTION}.",
                path.display(),
                key
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

    let log_format = match object.get("log_format") {
        Some(value) => {
            let raw = value.as_str().with_context(|| {
                format!(
                    "Config key 'log_format' in '{}' must be a string.",
                    path.display()
                )
            })?;
            Some(FileConfigValue {
                value: LogFormat::parse(raw, &format!("config file '{}'", path.display()))?,
                source,
            })
        }
        None => None,
    };

    let log_file = match object.get("log_file") {
        Some(value) => {
            let raw = value.as_str().with_context(|| {
                format!(
                    "Config key 'log_file' in '{}' must be a string.",
                    path.display()
                )
            })?;
            Some(FileConfigValue {
                value: raw.to_string(),
                source,
            })
        }
        None => None,
    };

    let log_file_mode = match object.get("log_file_mode") {
        Some(value) => {
            let raw = value.as_str().with_context(|| {
                format!(
                    "Config key 'log_file_mode' in '{}' must be a string.",
                    path.display()
                )
            })?;
            Some(FileConfigValue {
                value: LogFileMode::parse(raw, &format!("config file '{}'", path.display()))?,
                source,
            })
        }
        None => None,
    };

    let (otel_enabled, otel_endpoint, otel_protocol) = parse_otel_config(object, path, source)?;

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
    let (bash_policy_presets, bash_policy_custom) = parse_bash_policy_config(object, path, source)?;

    Ok(FileConfig {
        log_level,
        log_format,
        log_file,
        log_file_mode,
        otel_enabled,
        otel_endpoint,
        otel_protocol,
        timeout_ms,
        workos_client_id,
        bash_policy_presets,
        bash_policy_custom,
    })
}

fn parse_otel_config(
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

    for key in otel_object.keys() {
        if key != "enabled" && key != "exporter_otlp_endpoint" && key != "exporter_otlp_protocol" {
            bail!(
                "Config key 'otel' in '{}' contains unknown key '{}'. Allowed keys: enabled, exporter_otlp_endpoint, exporter_otlp_protocol.",
                path.display(),
                key
            );
        }
    }

    let enabled = match otel_object.get("enabled") {
        Some(value) => {
            let raw = value.as_bool().with_context(|| {
                format!(
                    "Config key 'otel.enabled' in '{}' must be a boolean.",
                    path.display()
                )
            })?;
            Some(FileConfigValue { value: raw, source })
        }
        None => None,
    };

    let endpoint = match otel_object.get("exporter_otlp_endpoint") {
        Some(value) => {
            let raw = value.as_str().with_context(|| {
                format!(
                    "Config key 'otel.exporter_otlp_endpoint' in '{}' must be a string.",
                    path.display()
                )
            })?;
            Some(FileConfigValue {
                value: raw.to_string(),
                source,
            })
        }
        None => None,
    };

    let protocol = match otel_object.get("exporter_otlp_protocol") {
        Some(value) => {
            let raw = value.as_str().with_context(|| {
                format!(
                    "Config key 'otel.exporter_otlp_protocol' in '{}' must be a string.",
                    path.display()
                )
            })?;
            Some(FileConfigValue {
                value: OtlpProtocol::parse(raw, &format!("config file '{}'", path.display()))?,
                source,
            })
        }
        None => None,
    };

    Ok((enabled, endpoint, protocol))
}

fn parse_bash_policy_config(
    object: &serde_json::Map<String, Value>,
    path: &Path,
    source: ConfigPathSource,
) -> Result<ParsedBashPolicyConfig> {
    let Some(policies_value) = object.get("policies") else {
        return Ok((None, None));
    };

    let policies_object = policies_value.as_object().with_context(|| {
        format!(
            "Config key 'policies' in '{}' must be an object.",
            path.display()
        )
    })?;

    for key in policies_object.keys() {
        if key != "bash" {
            bail!(
                "Config key 'policies' in '{}' contains unknown key '{}'. Allowed keys: bash.",
                path.display(),
                key
            );
        }
    }

    let Some(bash_value) = policies_object.get("bash") else {
        return Ok((None, None));
    };

    let bash_object = bash_value.as_object().with_context(|| {
        format!(
            "Config key 'policies.bash' in '{}' must be an object.",
            path.display()
        )
    })?;

    for key in bash_object.keys() {
        if key != "presets" && key != "custom" {
            bail!(
                "Config key 'policies.bash' in '{}' contains unknown key '{}'. Allowed keys: presets, custom.",
                path.display(),
                key
            );
        }
    }

    let presets = match bash_object.get("presets") {
        Some(value) => Some(FileConfigValue {
            value: parse_bash_policy_presets(value, path)?,
            source,
        }),
        None => None,
    };

    let custom = match bash_object.get("custom") {
        Some(value) => Some(FileConfigValue {
            value: parse_custom_bash_policies(value, path)?,
            source,
        }),
        None => None,
    };

    Ok((presets, custom))
}

fn parse_bash_policy_presets(value: &Value, path: &Path) -> Result<Vec<String>> {
    let items = value.as_array().with_context(|| {
        format!(
            "Config key 'policies.bash.presets' in '{}' must be an array.",
            path.display()
        )
    })?;

    let mut presets = Vec::with_capacity(items.len());
    let builtin_preset_ids = builtin_bash_policy_preset_ids();
    for item in items {
        let preset = item.as_str().with_context(|| {
            format!(
                "Config key 'policies.bash.presets' in '{}' must contain only strings.",
                path.display()
            )
        })?;
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

fn parse_custom_bash_policies(value: &Value, path: &Path) -> Result<Vec<CustomBashPolicyEntry>> {
    let items = value.as_array().with_context(|| {
        format!(
            "Config key 'policies.bash.custom' in '{}' must be an array.",
            path.display()
        )
    })?;

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

fn parse_custom_bash_policy_entry(item: &Value, path: &Path) -> Result<CustomBashPolicyEntry> {
    let object = item.as_object().with_context(|| {
        format!(
            "Config key 'policies.bash.custom' in '{}' must contain only objects.",
            path.display()
        )
    })?;

    validate_custom_bash_policy_fields(object, path)?;

    let id = object
        .get("id")
        .and_then(Value::as_str)
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

    let message = object
        .get("message")
        .and_then(Value::as_str)
        .with_context(|| {
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

    let argv_prefix = parse_custom_bash_policy_match(&id, object, path)?;

    Ok(CustomBashPolicyEntry {
        id,
        argv_prefix,
        message: message.to_string(),
    })
}

fn validate_custom_bash_policy_fields(
    object: &serde_json::Map<String, Value>,
    path: &Path,
) -> Result<()> {
    for key in object.keys() {
        if key != "id" && key != "match" && key != "message" {
            bail!(
                "Config key 'policies.bash.custom' in '{}' contains unknown field '{}'. Allowed fields: id, match, message.",
                path.display(),
                key
            );
        }
    }

    Ok(())
}

fn parse_custom_bash_policy_match(
    id: &str,
    object: &serde_json::Map<String, Value>,
    path: &Path,
) -> Result<Vec<String>> {
    let match_object = object
        .get("match")
        .and_then(Value::as_object)
        .with_context(|| {
            format!(
                "Custom bash policy '{}' in '{}' must include object field 'match'.",
                id,
                path.display()
            )
        })?;
    for key in match_object.keys() {
        if key != "argv_prefix" {
            bail!(
                "Custom bash policy '{}' in '{}' contains unknown 'match' field '{}'. Allowed fields: argv_prefix.",
                id,
                path.display(),
                key
            );
        }
    }

    let argv_prefix_values = match_object
        .get("argv_prefix")
        .and_then(Value::as_array)
        .with_context(|| {
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
    argv_prefix_values: &[Value],
    path: &Path,
) -> Result<Vec<String>> {
    let mut argv_prefix = Vec::with_capacity(argv_prefix_values.len());
    for token in argv_prefix_values {
        let token = token.as_str().with_context(|| {
            format!(
                "Custom bash policy '{}' in '{}' must use only string argv_prefix tokens.",
                id,
                path.display()
            )
        })?;
        if token.is_empty() {
            bail!(
                "Custom bash policy '{}' in '{}' cannot use empty argv_prefix tokens.",
                id,
                path.display()
            );
        }
        argv_prefix.push(token.to_string());
    }

    Ok(argv_prefix)
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
                format_validation_warnings_text(&runtime.validation_warnings),
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
                    "warnings": runtime.validation_warnings,
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
                format!(
                    "{}: {}",
                    style::success("SCE config validation"),
                    style::value("valid")
                ),
                format!(
                    "{}: {}",
                    style::label("Validation issues"),
                    style::value("none")
                ),
                format_validation_warnings_text(&runtime.validation_warnings),
            ];
            lines.join("\n")
        }
        ReportFormat::Json => {
            let payload = json!({
                "status": "ok",
                "result": {
                    "command": "config_validate",
                    "valid": true,
                    "issues": [],
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

#[cfg(test)]
mod tests {
    use super::{
        format_show_output, format_validate_output, generated_config_schema_path,
        resolve_observability_runtime_config_with, resolve_optional_auth_config_value,
        resolve_runtime_config_with, AuthConfigKeySpec, BashPolicyConfig, ConfigPathSource,
        ConfigRequest, CustomBashPolicyEntry, FileConfigValue, LoadedConfigPath, LogFileMode,
        LogFormat, LogLevel, OtlpProtocol, ReportFormat, ResolvedObservabilityRuntimeConfig,
        ResolvedOptionalValue, ResolvedValue, RuntimeConfig, ValueSource, DEFAULT_OTEL_ENDPOINT,
        SCE_CONFIG_SCHEMA_JSON, WORKOS_CLIENT_ID_BAKED_DEFAULT, WORKOS_CLIENT_ID_KEY,
    };
    use anyhow::Result;
    use serde_json::{json, Value};
    use std::path::{Path, PathBuf};

    fn schema() -> Value {
        serde_json::from_str(SCE_CONFIG_SCHEMA_JSON).expect("config schema JSON should parse")
    }

    fn schema_error_strings(instance: &Value) -> Vec<String> {
        super::config_schema_validator()
            .iter_errors(instance)
            .map(|error| error.to_string())
            .collect()
    }

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
            || Ok(PathBuf::from("/config/sce/config.json")),
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
            || Ok(PathBuf::from("/config/sce/config.json")),
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
            || Ok(PathBuf::from("/config/sce/config.json")),
        )?;

        assert_eq!(resolved.log_level.value, LogLevel::Error);
        assert_eq!(resolved.log_level.source.as_str(), "default");
        assert_eq!(resolved.log_format.value, LogFormat::Text);
        assert_eq!(resolved.log_format.source.as_str(), "default");
        assert_eq!(resolved.log_file.value, None);
        assert_eq!(resolved.log_file.source, None);
        assert_eq!(resolved.log_file_mode.value, LogFileMode::Truncate);
        assert_eq!(resolved.log_file_mode.source.as_str(), "default");
        assert!(!resolved.otel_enabled.value);
        assert_eq!(resolved.otel_enabled.source.as_str(), "default");
        assert_eq!(resolved.otel_endpoint.value, DEFAULT_OTEL_ENDPOINT);
        assert_eq!(resolved.otel_endpoint.source.as_str(), "default");
        assert_eq!(resolved.otel_protocol.value, OtlpProtocol::Grpc);
        assert_eq!(resolved.otel_protocol.source.as_str(), "default");
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
    fn resolver_uses_config_backed_observability_values() -> Result<()> {
        let req = request();
        let resolved = resolve_runtime_config_with(
            &req,
            Path::new("/workspace"),
            |_| None,
            |_| {
                Ok("{\"log_format\":\"json\",\"log_file\":\".sce/sce.log\",\"log_file_mode\":\"append\",\"otel\":{\"enabled\":true,\"exporter_otlp_endpoint\":\"https://collector.example/v1/traces\",\"exporter_otlp_protocol\":\"http/protobuf\"}}".to_string())
            },
            |_| true,
            || Ok(PathBuf::from("/config/sce/config.json")),
        )?;

        assert_eq!(resolved.log_format.value, LogFormat::Json);
        assert_eq!(resolved.log_format.source.as_str(), "config_file");
        assert_eq!(resolved.log_file.value.as_deref(), Some(".sce/sce.log"));
        assert_eq!(
            resolved.log_file.source.map(ValueSource::as_str),
            Some("config_file")
        );
        assert_eq!(resolved.log_file_mode.value, LogFileMode::Append);
        assert_eq!(resolved.log_file_mode.source.as_str(), "config_file");
        assert!(resolved.otel_enabled.value);
        assert_eq!(resolved.otel_enabled.source.as_str(), "config_file");
        assert_eq!(
            resolved.otel_endpoint.value,
            "https://collector.example/v1/traces"
        );
        assert_eq!(resolved.otel_endpoint.source.as_str(), "config_file");
        assert_eq!(resolved.otel_protocol.value, OtlpProtocol::HttpProtobuf);
        assert_eq!(resolved.otel_protocol.source.as_str(), "config_file");
        Ok(())
    }

    #[test]
    fn resolver_uses_env_over_config_for_observability_values() -> Result<()> {
        let req = request();
        let resolved = resolve_runtime_config_with(
            &req,
            Path::new("/workspace"),
            |key| match key {
                "SCE_LOG_FORMAT" => Some("text".to_string()),
                "SCE_LOG_FILE" => Some("env.log".to_string()),
                "SCE_LOG_FILE_MODE" => Some("truncate".to_string()),
                "SCE_OTEL_ENABLED" => Some("true".to_string()),
                "OTEL_EXPORTER_OTLP_ENDPOINT" => Some("https://env.example/v1/traces".to_string()),
                "OTEL_EXPORTER_OTLP_PROTOCOL" => Some("grpc".to_string()),
                _ => None,
            },
            |_| {
                Ok("{\"log_format\":\"json\",\"log_file\":\"config.log\",\"log_file_mode\":\"append\",\"otel\":{\"enabled\":false,\"exporter_otlp_endpoint\":\"https://config.example/v1/traces\",\"exporter_otlp_protocol\":\"http/protobuf\"}}".to_string())
            },
            |_| true,
            || Ok(PathBuf::from("/config/sce/config.json")),
        )?;

        assert_eq!(resolved.log_format.value, LogFormat::Text);
        assert_eq!(resolved.log_format.source.as_str(), "env");
        assert_eq!(resolved.log_file.value.as_deref(), Some("env.log"));
        assert_eq!(
            resolved.log_file.source.map(ValueSource::as_str),
            Some("env")
        );
        assert_eq!(resolved.log_file_mode.value, LogFileMode::Truncate);
        assert_eq!(resolved.log_file_mode.source.as_str(), "env");
        assert!(resolved.otel_enabled.value);
        assert_eq!(resolved.otel_enabled.source.as_str(), "env");
        assert_eq!(
            resolved.otel_endpoint.value,
            "https://env.example/v1/traces"
        );
        assert_eq!(resolved.otel_endpoint.source.as_str(), "env");
        assert_eq!(resolved.otel_protocol.value, OtlpProtocol::Grpc);
        assert_eq!(resolved.otel_protocol.source.as_str(), "env");
        Ok(())
    }

    #[test]
    fn observability_resolver_returns_runtime_ready_values() -> Result<()> {
        let resolved = resolve_observability_runtime_config_with(
            Path::new("/workspace"),
            |key| match key {
                "SCE_LOG_LEVEL" => Some("debug".to_string()),
                "SCE_OTEL_ENABLED" => Some("true".to_string()),
                _ => None,
            },
            |_| {
                Ok("{\"log_format\":\"json\",\"log_file\":\"config.log\",\"log_file_mode\":\"append\",\"otel\":{\"enabled\":false,\"exporter_otlp_endpoint\":\"https://config.example/v1/traces\",\"exporter_otlp_protocol\":\"http/protobuf\"}}".to_string())
            },
            |_| true,
            || Ok(PathBuf::from("/config/sce/config.json")),
        )?;

        assert_eq!(
            resolved,
            ResolvedObservabilityRuntimeConfig {
                log_level: LogLevel::Debug,
                log_format: LogFormat::Json,
                log_file: Some("config.log".to_string()),
                log_file_mode: LogFileMode::Append,
                otel_enabled: true,
                otel_endpoint: "https://config.example/v1/traces".to_string(),
                otel_protocol: OtlpProtocol::HttpProtobuf,
                loaded_config_paths: vec![
                    LoadedConfigPath {
                        path: PathBuf::from("/config/sce/config.json"),
                        source: ConfigPathSource::DefaultDiscoveredGlobal,
                    },
                    LoadedConfigPath {
                        path: PathBuf::from("/workspace/.sce/config.json"),
                        source: ConfigPathSource::DefaultDiscoveredLocal,
                    },
                ],
            }
        );
        Ok(())
    }

    #[test]
    fn observability_resolver_rejects_invalid_env_overrides() {
        let error = resolve_observability_runtime_config_with(
            Path::new("/workspace"),
            |key| match key {
                "OTEL_EXPORTER_OTLP_PROTOCOL" => Some("udp".to_string()),
                _ => None,
            },
            |_| Ok("{}".to_string()),
            |_| false,
            || Ok(PathBuf::from("/config/sce/config.json")),
        )
        .expect_err("invalid env OTLP protocol should fail");

        assert_eq!(
            error.to_string(),
            "Invalid OTLP protocol 'udp' from OTEL_EXPORTER_OTLP_PROTOCOL. Valid values: grpc, http/protobuf."
        );
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
            || Ok(PathBuf::from("/config/sce/config.json")),
        )
        .expect_err("unknown config keys should fail");
        assert!(error
            .to_string()
            .contains("failed schema validation against generated schema"));
        assert!(error.to_string().contains(&generated_config_schema_path()));
        assert!(error.to_string().contains("unknown"));
    }

    #[test]
    fn resolver_accepts_canonical_schema_key_in_config_file() -> Result<()> {
        let req = ConfigRequest {
            report_format: ReportFormat::Text,
            config_path: Some(PathBuf::from("/tmp/config.json")),
            log_level: None,
            timeout_ms: None,
        };
        let resolved = resolve_runtime_config_with(
            &req,
            Path::new("/workspace"),
            |_| None,
            |_| {
                Ok(
                    "{\"$schema\":\"https://sce.crocoder.dev/config.json\",\"log_level\":\"warn\",\"timeout_ms\":500}".to_string()
                )
            },
            |_| true,
            || Ok(PathBuf::from("/config/sce/config.json")),
        )?;

        assert_eq!(resolved.log_level.value, LogLevel::Warn);
        assert_eq!(resolved.log_level.source.as_str(), "config_file");
        assert_eq!(resolved.timeout_ms.value, 500);
        assert_eq!(resolved.timeout_ms.source.as_str(), "config_file");
        Ok(())
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn resolver_merges_discovered_global_and_local_configs() -> Result<()> {
        let req = request();
        let resolved = resolve_runtime_config_with(
            &req,
            Path::new("/workspace"),
            |_| None,
            |path| {
                if path == Path::new("/config/sce/config.json") {
                    return Ok("{\"log_level\":\"error\",\"log_format\":\"json\",\"otel\":{\"enabled\":true},\"timeout_ms\":500,\"workos_client_id\":\"global-client\"}".to_string());
                }
                if path == Path::new("/workspace/.sce/config.json") {
                    return Ok(
                        "{\"log_file\":\"local.log\",\"log_file_mode\":\"append\",\"otel\":{\"exporter_otlp_endpoint\":\"https://local.example/v1/traces\",\"exporter_otlp_protocol\":\"http/protobuf\"},\"timeout_ms\":700,\"workos_client_id\":\"local-client\"}".to_string()
                    );
                }
                Err(anyhow::anyhow!(
                    "unexpected config path: {}",
                    path.display()
                ))
            },
            |path| {
                path == Path::new("/config/sce/config.json")
                    || path == Path::new("/workspace/.sce/config.json")
            },
            || Ok(PathBuf::from("/config/sce/config.json")),
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

        assert_eq!(resolved.log_format.value, LogFormat::Json);
        assert_eq!(resolved.log_format.source.as_str(), "config_file");
        assert_eq!(
            resolved
                .log_format
                .source
                .config_source()
                .map(super::ConfigPathSource::as_str),
            Some("default_discovered_global")
        );

        assert_eq!(resolved.log_file.value.as_deref(), Some("local.log"));
        assert_eq!(
            resolved
                .log_file
                .source
                .and_then(super::ValueSource::config_source)
                .map(super::ConfigPathSource::as_str),
            Some("default_discovered_local")
        );

        assert_eq!(resolved.log_file_mode.value, LogFileMode::Append);
        assert_eq!(
            resolved
                .log_file_mode
                .source
                .config_source()
                .map(super::ConfigPathSource::as_str),
            Some("default_discovered_local")
        );

        assert!(resolved.otel_enabled.value);
        assert_eq!(
            resolved
                .otel_enabled
                .source
                .config_source()
                .map(super::ConfigPathSource::as_str),
            Some("default_discovered_global")
        );

        assert_eq!(
            resolved.otel_endpoint.value,
            "https://local.example/v1/traces"
        );
        assert_eq!(
            resolved
                .otel_endpoint
                .source
                .config_source()
                .map(super::ConfigPathSource::as_str),
            Some("default_discovered_local")
        );

        assert_eq!(resolved.otel_protocol.value, OtlpProtocol::HttpProtobuf);
        assert_eq!(
            resolved
                .otel_protocol
                .source
                .config_source()
                .map(super::ConfigPathSource::as_str),
            Some("default_discovered_local")
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
    fn resolver_rejects_log_file_mode_without_log_file() {
        let req = request();
        let error = resolve_runtime_config_with(
            &req,
            Path::new("/workspace"),
            |_| None,
            |_| Ok("{\"log_file_mode\":\"append\"}".to_string()),
            |_| true,
            || Ok(PathBuf::from("/config/sce/config.json")),
        )
        .expect_err("log file mode without file should fail");

        assert!(error
            .to_string()
            .contains("failed schema validation against generated schema"));
        assert!(error.to_string().contains("log_file"));
    }

    #[test]
    fn resolver_rejects_invalid_otel_endpoint_when_enabled() {
        let req = request();
        let error = resolve_runtime_config_with(
            &req,
            Path::new("/workspace"),
            |_| None,
            |_| {
                Ok(
                    "{\"otel\":{\"enabled\":true,\"exporter_otlp_endpoint\":\"collector:4317\"}}"
                        .to_string(),
                )
            },
            |_| true,
            || Ok(PathBuf::from("/config/sce/config.json")),
        )
        .expect_err("invalid otel endpoint should fail");

        assert!(error
            .to_string()
            .contains("failed schema validation against generated schema"));
        assert!(error.to_string().contains("collector:4317"));
    }

    #[test]
    fn resolver_uses_global_workos_client_id_when_local_omits_key() -> Result<()> {
        let req = request();
        let resolved = resolve_runtime_config_with(
            &req,
            Path::new("/workspace"),
            |_| None,
            |path| {
                if path == Path::new("/config/sce/config.json") {
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
                path == Path::new("/config/sce/config.json")
                    || path == Path::new("/workspace/.sce/config.json")
            },
            || Ok(PathBuf::from("/config/sce/config.json")),
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
                    path: PathBuf::from("/config/sce/config.json"),
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
            log_format: ResolvedValue {
                value: LogFormat::Json,
                source: ValueSource::ConfigFile(ConfigPathSource::DefaultDiscoveredLocal),
            },
            log_file: ResolvedOptionalValue {
                value: Some(".sce/sce.log".to_string()),
                source: Some(ValueSource::ConfigFile(
                    ConfigPathSource::DefaultDiscoveredLocal,
                )),
            },
            log_file_mode: ResolvedValue {
                value: LogFileMode::Append,
                source: ValueSource::ConfigFile(ConfigPathSource::DefaultDiscoveredLocal),
            },
            otel_enabled: ResolvedValue {
                value: true,
                source: ValueSource::Env,
            },
            otel_endpoint: ResolvedValue {
                value: "https://collector.example/v1/traces".to_string(),
                source: ValueSource::ConfigFile(ConfigPathSource::DefaultDiscoveredGlobal),
            },
            otel_protocol: ResolvedValue {
                value: OtlpProtocol::HttpProtobuf,
                source: ValueSource::Default,
            },
            timeout_ms: ResolvedValue {
                value: 1200,
                source: ValueSource::Flag,
            },
            workos_client_id: ResolvedOptionalValue {
                value: None,
                source: None,
            },
            bash_policies: ResolvedOptionalValue {
                value: Some(BashPolicyConfig {
                    presets: vec!["forbid-git-commit".to_string()],
                    custom: vec![CustomBashPolicyEntry {
                        id: "prefer-jj-status".to_string(),
                        argv_prefix: vec!["git".to_string(), "status".to_string()],
                        message: "Use `jj status` instead.".to_string(),
                    }],
                }),
                source: Some(ValueSource::ConfigFile(
                    ConfigPathSource::DefaultDiscoveredLocal,
                )),
            },
            validation_warnings: Vec::new(),
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
        assert_eq!(parsed["result"]["resolved"]["log_format"]["value"], "json");
        assert_eq!(
            parsed["result"]["resolved"]["log_file"]["config_source"],
            "default_discovered_local"
        );
        assert_eq!(
            parsed["result"]["resolved"]["log_file_mode"]["value"],
            "append"
        );
        assert_eq!(
            parsed["result"]["resolved"]["otel"]["enabled"]["value"],
            true
        );
        assert_eq!(
            parsed["result"]["resolved"]["otel"]["exporter_otlp_endpoint"]["config_source"],
            "default_discovered_global"
        );
        assert_eq!(
            parsed["result"]["resolved"]["otel"]["exporter_otlp_protocol"]["value"],
            "http/protobuf"
        );
        assert_eq!(parsed["result"]["resolved"]["timeout_ms"]["source"], "flag");
        assert_eq!(
            parsed["result"]["resolved"]["workos_client_id"]["value"],
            Value::Null
        );
        assert_eq!(
            parsed["result"]["resolved"]["workos_client_id"]["source"],
            Value::Null
        );
        assert_eq!(
            parsed["result"]["resolved"]["policies"]["bash"]["source"],
            "config_file"
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
                value: LogLevel::Error,
                source: ValueSource::Default,
            },
            log_format: ResolvedValue {
                value: LogFormat::Text,
                source: ValueSource::Default,
            },
            log_file: ResolvedOptionalValue {
                value: None,
                source: None,
            },
            log_file_mode: ResolvedValue {
                value: LogFileMode::Truncate,
                source: ValueSource::Default,
            },
            otel_enabled: ResolvedValue {
                value: false,
                source: ValueSource::Default,
            },
            otel_endpoint: ResolvedValue {
                value: DEFAULT_OTEL_ENDPOINT.to_string(),
                source: ValueSource::Default,
            },
            otel_protocol: ResolvedValue {
                value: OtlpProtocol::Grpc,
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
            bash_policies: ResolvedOptionalValue {
                value: None,
                source: None,
            },
            validation_warnings: Vec::new(),
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
                value: LogLevel::Error,
                source: ValueSource::Default,
            },
            log_format: ResolvedValue {
                value: LogFormat::Text,
                source: ValueSource::Default,
            },
            log_file: ResolvedOptionalValue {
                value: None,
                source: None,
            },
            log_file_mode: ResolvedValue {
                value: LogFileMode::Truncate,
                source: ValueSource::Default,
            },
            otel_enabled: ResolvedValue {
                value: false,
                source: ValueSource::Default,
            },
            otel_endpoint: ResolvedValue {
                value: DEFAULT_OTEL_ENDPOINT.to_string(),
                source: ValueSource::Default,
            },
            otel_protocol: ResolvedValue {
                value: OtlpProtocol::Grpc,
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
            bash_policies: ResolvedOptionalValue {
                value: None,
                source: None,
            },
            validation_warnings: Vec::new(),
        };

        let output = format_show_output(&runtime, ReportFormat::Text);
        assert!(output.contains("workos_client_id: clie...cdef"));
        assert!(output.contains("auth_precedence: env (WORKOS_CLIENT_ID) > config file (workos_client_id) > baked default (client_sce_default)"));
    }

    #[test]
    fn show_text_output_includes_observability_values_and_sources() {
        let output = format_show_output(&sample_runtime(), ReportFormat::Text);

        assert!(output.contains(
            "log_format: json (source: config_file, config_source: default_discovered_local)"
        ));
        assert!(output.contains(
            "log_file: .sce/sce.log (source: config_file, config_source: default_discovered_local)"
        ));
        assert!(output.contains(
            "log_file_mode: append (source: config_file, config_source: default_discovered_local)"
        ));
        assert!(output.contains("otel.enabled: true (source: env)"));
        assert!(output.contains("otel.exporter_otlp_endpoint: https://collector.example/v1/traces (source: config_file, config_source: default_discovered_global)"));
        assert!(output.contains("otel.exporter_otlp_protocol: http/protobuf (source: default)"));
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
        assert!(parsed["result"]["warnings"].as_array().is_some());
        assert!(parsed["result"].get("precedence").is_none());
        assert!(parsed["result"].get("config_paths").is_none());
        assert!(parsed["result"].get("resolved_observability").is_none());
        assert!(parsed["result"].get("resolved_auth").is_none());
        assert!(parsed["result"].get("resolved_policies").is_none());
        Ok(())
    }

    #[test]
    fn validate_text_output_is_trimmed_to_status_and_issues() {
        let runtime = RuntimeConfig {
            loaded_config_paths: vec![LoadedConfigPath {
                path: PathBuf::from("/workspace/.sce/config.json"),
                source: ConfigPathSource::DefaultDiscoveredLocal,
            }],
            log_level: ResolvedValue {
                value: LogLevel::Error,
                source: ValueSource::Default,
            },
            log_format: ResolvedValue {
                value: LogFormat::Text,
                source: ValueSource::Default,
            },
            log_file: ResolvedOptionalValue {
                value: None,
                source: None,
            },
            log_file_mode: ResolvedValue {
                value: LogFileMode::Truncate,
                source: ValueSource::Default,
            },
            otel_enabled: ResolvedValue {
                value: false,
                source: ValueSource::Default,
            },
            otel_endpoint: ResolvedValue {
                value: DEFAULT_OTEL_ENDPOINT.to_string(),
                source: ValueSource::Default,
            },
            otel_protocol: ResolvedValue {
                value: OtlpProtocol::Grpc,
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
            bash_policies: ResolvedOptionalValue {
                value: None,
                source: None,
            },
            validation_warnings: Vec::new(),
        };

        let output = format_validate_output(&runtime, ReportFormat::Text);
        assert!(output.contains("SCE config validation: valid"));
        assert!(output.contains("Validation issues: none"));
        assert!(output.contains("Validation warnings: none"));
        assert!(!output.contains("Precedence:"));
        assert!(!output.contains("Config files:"));
        assert!(!output.contains("Resolved auth precedence:"));
        assert!(!output.contains("workos_client_id:"));
        assert!(!output.contains("policies.bash:"));
        assert!(!output.contains("otel."));
    }

    #[test]
    fn validate_json_output_only_reports_status_and_issues() -> Result<()> {
        let parsed: Value = serde_json::from_str(&format_validate_output(
            &sample_runtime(),
            ReportFormat::Json,
        ))?;

        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["result"]["command"], "config_validate");
        assert_eq!(parsed["result"]["valid"], true);
        assert!(parsed["result"]["issues"].is_array());
        assert!(parsed["result"]["warnings"].is_array());
        assert_eq!(
            parsed["result"].as_object().map(serde_json::Map::len),
            Some(4)
        );
        Ok(())
    }

    #[test]
    fn resolver_parses_bash_policy_config_and_reports_local_override() -> Result<()> {
        let req = request();
        let resolved = resolve_runtime_config_with(
            &req,
            Path::new("/workspace"),
            |_| None,
            |path| {
                if path == Path::new("/config/sce/config.json") {
                    return Ok(
                        "{\"policies\":{\"bash\":{\"presets\":[\"forbid-git-all\"]}}}".to_string(),
                    );
                }
                if path == Path::new("/workspace/.sce/config.json") {
                    return Ok("{\"policies\":{\"bash\":{\"presets\":[\"forbid-git-commit\"],\"custom\":[{\"id\":\"prefer-jj-status\",\"match\":{\"argv_prefix\":[\"git\",\"status\"]},\"message\":\"Use jj.\"}]}}}".to_string());
                }
                Err(anyhow::anyhow!(
                    "unexpected config path: {}",
                    path.display()
                ))
            },
            |path| {
                path == Path::new("/config/sce/config.json")
                    || path == Path::new("/workspace/.sce/config.json")
            },
            || Ok(PathBuf::from("/config/sce/config.json")),
        )?;

        let policies = resolved
            .bash_policies
            .value
            .expect("bash policies should resolve");
        assert_eq!(policies.presets, vec!["forbid-git-commit".to_string()]);
        assert_eq!(policies.custom[0].id, "prefer-jj-status");
        assert_eq!(
            resolved
                .bash_policies
                .source
                .and_then(super::ValueSource::config_source)
                .map(super::ConfigPathSource::as_str),
            Some("default_discovered_local")
        );
        Ok(())
    }

    #[test]
    fn resolver_rejects_unknown_bash_policy_preset() {
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
            |_| Ok("{\"policies\":{\"bash\":{\"presets\":[\"forbid-hg\"]}}}".to_string()),
            |_| true,
            || Ok(PathBuf::from("/config/sce/config.json")),
        )
        .expect_err("unknown preset should fail");

        assert!(error
            .to_string()
            .contains("failed schema validation against generated schema"));
        assert!(error.to_string().contains("forbid-hg"));
    }

    #[test]
    fn resolver_rejects_conflicting_npm_presets() {
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
            |_| Ok("{\"policies\":{\"bash\":{\"presets\":[\"use-pnpm-over-npm\",\"use-bun-over-npm\"]}}}".to_string()),
            |_| true,
            || Ok(PathBuf::from("/config/sce/config.json")),
        )
        .expect_err("conflicting presets should fail");

        assert!(error
            .to_string()
            .contains("failed schema validation against generated schema"));
    }

    #[test]
    fn validate_config_file_reports_generated_schema_path_for_invalid_shape() {
        let temp_dir =
            std::env::temp_dir().join(format!("sce-config-schema-test-{}", std::process::id()));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should be creatable");
        let path = temp_dir.join("config.json");
        std::fs::write(&path, "{\"unknown\":true}").expect("config fixture should write");

        let error = super::validate_config_file(&path).expect_err("invalid config should fail");
        assert!(error
            .to_string()
            .contains("failed schema validation against generated schema"));
        assert!(error.to_string().contains(&generated_config_schema_path()));

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&temp_dir).ok();
    }

    #[test]
    fn resolver_rejects_duplicate_custom_bash_prefixes() {
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
            |_| Ok("{\"policies\":{\"bash\":{\"custom\":[{\"id\":\"one\",\"match\":{\"argv_prefix\":[\"git\",\"status\"]},\"message\":\"one\"},{\"id\":\"two\",\"match\":{\"argv_prefix\":[\"git\",\"status\"]},\"message\":\"two\"}]}}}".to_string()),
            |_| true,
            || Ok(PathBuf::from("/config/sce/config.json")),
        )
        .expect_err("duplicate custom prefix should fail");

        assert!(error
            .to_string()
            .contains("duplicate argv_prefix [git status]"));
    }

    #[test]
    fn validate_reports_redundant_git_presets_as_warning() -> Result<()> {
        let req = ConfigRequest {
            report_format: ReportFormat::Text,
            config_path: Some(PathBuf::from("/tmp/config.json")),
            log_level: None,
            timeout_ms: None,
        };
        let resolved = resolve_runtime_config_with(
            &req,
            Path::new("/workspace"),
            |_| None,
            |_| {
                Ok("{\"policies\":{\"bash\":{\"presets\":[\"forbid-git-all\",\"forbid-git-commit\"]}}}".to_string())
            },
            |_| true,
            || Ok(PathBuf::from("/config/sce/config.json")),
        )?;

        let output = format_validate_output(&resolved, ReportFormat::Text);
        assert!(output.contains("Validation warnings: Preset 'forbid-git-commit' is redundant when 'forbid-git-all' is also enabled."));
        Ok(())
    }

    #[test]
    fn config_schema_keeps_builtin_preset_enum_in_sync_with_catalog() {
        let schema = schema();
        let preset_enum = schema["properties"]["policies"]["properties"]["bash"]["properties"]
            ["presets"]["items"]["enum"]
            .as_array()
            .expect("schema should expose preset enum");
        let preset_ids = super::builtin_bash_policy_preset_ids();

        assert_eq!(preset_enum.len(), preset_ids.len());
        for preset in preset_ids {
            assert!(preset_enum
                .iter()
                .any(|value| value.as_str() == Some(preset)));
        }
    }

    #[test]
    fn config_schema_accepts_supported_config_shape() {
        let instance = json!({
            "log_level": "warn",
            "timeout_ms": 1200,
            "workos_client_id": "client_local",
            "policies": {
                "bash": {
                    "presets": ["forbid-git-commit"],
                    "custom": [
                        {
                            "id": "prefer-jj-status",
                            "match": {
                                "argv_prefix": ["git", "status"]
                            },
                            "message": "Use jj status instead."
                        }
                    ]
                }
            }
        });

        assert!(schema_error_strings(&instance).is_empty());
    }

    #[test]
    fn config_schema_rejects_unknown_top_level_key() {
        let errors = schema_error_strings(&json!({
            "log_level": "error",
            "unknown": true
        }));

        assert!(!errors.is_empty());
    }

    #[test]
    fn config_schema_rejects_invalid_log_level() {
        let errors = schema_error_strings(&json!({
            "log_level": "trace"
        }));

        assert!(!errors.is_empty());
    }

    #[test]
    fn config_schema_rejects_conflicting_npm_presets() {
        let errors = schema_error_strings(&json!({
            "policies": {
                "bash": {
                    "presets": ["use-pnpm-over-npm", "use-bun-over-npm"]
                }
            }
        }));

        assert!(!errors.is_empty());
    }

    #[test]
    fn config_schema_rejects_builtin_custom_policy_ids() {
        let errors = schema_error_strings(&json!({
            "policies": {
                "bash": {
                    "custom": [
                        {
                            "id": "forbid-git-all",
                            "match": {
                                "argv_prefix": ["git"]
                            },
                            "message": "blocked"
                        }
                    ]
                }
            }
        }));

        assert!(!errors.is_empty());
    }

    #[test]
    fn config_schema_rejects_empty_argv_prefix_tokens() {
        let errors = schema_error_strings(&json!({
            "policies": {
                "bash": {
                    "custom": [
                        {
                            "id": "prefer-jj-status",
                            "match": {
                                "argv_prefix": [""]
                            },
                            "message": "blocked"
                        }
                    ]
                }
            }
        }));

        assert!(!errors.is_empty());
    }
}
