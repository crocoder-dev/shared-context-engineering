use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use lexopt::{Arg, ValueExt};
use serde_json::{json, Value};

use crate::services::output_format::OutputFormat;

pub const NAME: &str = "config";

const DEFAULT_TIMEOUT_MS: u64 = 30000;

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
                "Invalid log level '{}' from {}. Valid values: error, warn, info, debug.",
                raw,
                source
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
    Help,
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeConfig {
    loaded_config_paths: Vec<LoadedConfigPath>,
    log_level: ResolvedValue<LogLevel>,
    timeout_ms: ResolvedValue<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileConfig {
    log_level: Option<FileConfigValue<LogLevel>>,
    timeout_ms: Option<FileConfigValue<u64>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileConfigValue<T> {
    value: T,
    source: ConfigPathSource,
}

pub fn parse_config_subcommand(mut args: Vec<String>) -> Result<ConfigSubcommand> {
    if args.is_empty() {
        bail!("Missing config subcommand. Run 'sce config --help' to see valid usage.");
    }

    if let [only] = args.as_slice() {
        if only == "--help" || only == "-h" {
            return Ok(ConfigSubcommand::Help);
        }
    }

    let subcommand = args.remove(0);
    let tail = args;
    match subcommand.as_str() {
        "show" => Ok(ConfigSubcommand::Show(parse_config_request(tail)?)),
        "validate" => Ok(ConfigSubcommand::Validate(parse_config_request(tail)?)),
        _ => bail!(
            "Unknown config subcommand '{}'. Run 'sce config --help' to see valid usage.",
            subcommand
        ),
    }
}

fn parse_config_request(args: Vec<String>) -> Result<ConfigRequest> {
    let mut parser = lexopt::Parser::from_args(args);
    let mut request = ConfigRequest {
        report_format: ReportFormat::Text,
        config_path: None,
        log_level: None,
        timeout_ms: None,
    };

    while let Some(arg) = parser.next()? {
        match arg {
            Arg::Long("format") => {
                let value = parser
                    .value()
                    .context("Option '--format' requires a value")?;
                let raw = value.string()?;
                request.report_format = ReportFormat::parse(&raw, "sce config --help")?;
            }
            Arg::Long("config") => {
                let value = parser
                    .value()
                    .context("Option '--config' requires a path value")?;
                if request.config_path.is_some() {
                    bail!(
                        "Option '--config' may only be provided once. Run 'sce config --help' to see valid usage."
                    );
                }
                request.config_path = Some(PathBuf::from(value.string()?));
            }
            Arg::Long("log-level") => {
                let value = parser
                    .value()
                    .context("Option '--log-level' requires a value")?;
                let raw = value.string()?;
                request.log_level = Some(LogLevel::parse(&raw, "--log-level")?);
            }
            Arg::Long("timeout-ms") => {
                let value = parser
                    .value()
                    .context("Option '--timeout-ms' requires a numeric value")?;
                let raw = value.string()?;
                let timeout = raw
                    .parse::<u64>()
                    .map_err(|_| anyhow!("Invalid timeout '{}' from --timeout-ms.", raw))?;
                request.timeout_ms = Some(timeout);
            }
            Arg::Long("help") | Arg::Short('h') => {
                bail!(
                    "Use 'sce config --help' for config usage. Command-local help does not accept additional arguments."
                );
            }
            Arg::Long(option) => {
                bail!(
                    "Unknown config option '--{}'. Run 'sce config --help' to see valid usage.",
                    option
                );
            }
            Arg::Short(option) => {
                bail!(
                    "Unknown config option '-{}'. Run 'sce config --help' to see valid usage.",
                    option
                );
            }
            Arg::Value(value) => {
                let raw = value.string()?;
                bail!(
                    "Unexpected config argument '{}'. Run 'sce config --help' to see valid usage.",
                    raw
                );
            }
        }
    }

    Ok(request)
}

pub fn run_config_subcommand(subcommand: ConfigSubcommand) -> Result<String> {
    match subcommand {
        ConfigSubcommand::Help => Ok(config_usage_text().to_string()),
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

pub fn config_usage_text() -> &'static str {
    "Usage:\n  sce config show [--config <path>] [--log-level <error|warn|info|debug>] [--timeout-ms <value>] [--format <text|json>]\n  sce config validate [--config <path>] [--log-level <error|warn|info|debug>] [--timeout-ms <value>] [--format <text|json>]\n\nResolution precedence: flags > env > config file > defaults\nConfig discovery order: --config, SCE_CONFIG_FILE, then discovered global+local defaults (global merged first, local overrides per key)\nEnvironment keys: SCE_CONFIG_FILE, SCE_LOG_LEVEL, SCE_TIMEOUT_MS"
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
            .map_err(|_| anyhow!("Invalid timeout '{}' from SCE_TIMEOUT_MS.", raw))?;
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

    Ok(RuntimeConfig {
        loaded_config_paths,
        log_level: resolved_log_level,
        timeout_ms: resolved_timeout_ms,
    })
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
        if key != "log_level" && key != "timeout_ms" {
            bail!(
                "Config file '{}' contains unknown key '{}'. Allowed keys: log_level, timeout_ms.",
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

    Ok(FileConfig {
        log_level,
        timeout_ms,
    })
}

fn format_show_output(runtime: &RuntimeConfig, report_format: ReportFormat) -> String {
    match report_format {
        ReportFormat::Text => {
            let lines = vec![
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
                        }
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
            let lines = vec![
                "SCE config validation: valid".to_string(),
                "Precedence: flags > env > config file > defaults".to_string(),
                format_config_paths_text(runtime),
                "Validation issues: none".to_string(),
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
                    "issues": []
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

#[cfg(test)]
mod tests {
    use super::{
        format_show_output, format_validate_output, parse_config_subcommand,
        resolve_runtime_config_with, ConfigPathSource, ConfigRequest, ConfigSubcommand,
        LoadedConfigPath, LogLevel, ReportFormat, ResolvedValue, RuntimeConfig, ValueSource,
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
    fn parser_routes_show_subcommand() -> Result<()> {
        let parsed = parse_config_subcommand(vec!["show".to_string()])?;
        assert_eq!(parsed, ConfigSubcommand::Show(request()));
        Ok(())
    }

    #[test]
    fn parser_routes_validate_subcommand_with_options() -> Result<()> {
        let parsed = parse_config_subcommand(vec![
            "validate".to_string(),
            "--format".to_string(),
            "json".to_string(),
            "--log-level".to_string(),
            "debug".to_string(),
            "--timeout-ms".to_string(),
            "100".to_string(),
            "--config".to_string(),
            "./demo.json".to_string(),
        ])?;
        assert_eq!(
            parsed,
            ConfigSubcommand::Validate(ConfigRequest {
                report_format: ReportFormat::Json,
                config_path: Some(PathBuf::from("./demo.json")),
                log_level: Some(LogLevel::Debug),
                timeout_ms: Some(100),
            })
        );
        Ok(())
    }

    #[test]
    fn parser_rejects_invalid_format_with_help_guidance() {
        let error = parse_config_subcommand(vec![
            "show".to_string(),
            "--format".to_string(),
            "yaml".to_string(),
        ])
        .expect_err("invalid format should fail");
        assert_eq!(
            error.to_string(),
            "Invalid --format value 'yaml'. Valid values: text, json. Run 'sce config --help' to see valid usage."
        );
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
                _ => None,
            },
            |_| Ok("{\"log_level\":\"error\",\"timeout_ms\":500}".to_string()),
            |_| true,
            || Ok(PathBuf::from("/state")),
        )?;

        assert_eq!(resolved.log_level.value, LogLevel::Warn);
        assert_eq!(resolved.log_level.source.as_str(), "flag");
        assert_eq!(resolved.timeout_ms.value, 900);
        assert_eq!(resolved.timeout_ms.source.as_str(), "flag");
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
                _ => None,
            },
            |_| Ok("{\"log_level\":\"error\",\"timeout_ms\":500}".to_string()),
            |_| true,
            || Ok(PathBuf::from("/state")),
        )?;

        assert_eq!(resolved.log_level.value, LogLevel::Warn);
        assert_eq!(resolved.log_level.source.as_str(), "env");
        assert_eq!(resolved.timeout_ms.value, 1200);
        assert_eq!(resolved.timeout_ms.source.as_str(), "env");
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
        Ok(())
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
                    return Ok("{\"log_level\":\"error\",\"timeout_ms\":500}".to_string());
                }
                if path == Path::new("/workspace/.sce/config.json") {
                    return Ok("{\"timeout_ms\":700}".to_string());
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
                .map(|source| source.as_str()),
            Some("default_discovered_global")
        );

        assert_eq!(resolved.timeout_ms.value, 700);
        assert_eq!(resolved.timeout_ms.source.as_str(), "config_file");
        assert_eq!(
            resolved
                .timeout_ms
                .source
                .config_source()
                .map(|source| source.as_str()),
            Some("default_discovered_local")
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
        Ok(())
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
        Ok(())
    }
}
