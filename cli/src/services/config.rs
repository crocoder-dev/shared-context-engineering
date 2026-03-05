use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use lexopt::{Arg, ValueExt};
use serde_json::{json, Value};

pub const NAME: &str = "config";

const DEFAULT_TIMEOUT_MS: u64 = 30000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportFormat {
    Text,
    Json,
}

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
    ConfigFile,
    Default,
}

impl ValueSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Env => "env",
            Self::ConfigFile => "config_file",
            Self::Default => "default",
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
    DefaultDiscovered,
}

impl ConfigPathSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Env => "env",
            Self::DefaultDiscovered => "default_discovered",
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
    loaded_config_path: Option<LoadedConfigPath>,
    log_level: ResolvedValue<LogLevel>,
    timeout_ms: ResolvedValue<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileConfig {
    log_level: Option<LogLevel>,
    timeout_ms: Option<u64>,
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
                request.report_format = parse_report_format(&raw)?;
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

fn parse_report_format(raw: &str) -> Result<ReportFormat> {
    match raw {
        "text" => Ok(ReportFormat::Text),
        "json" => Ok(ReportFormat::Json),
        _ => bail!(
            "Invalid format '{}'. Valid values: text, json. Run 'sce config --help' to see valid usage.",
            raw
        ),
    }
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
    "Usage:\n  sce config show [--config <path>] [--log-level <error|warn|info|debug>] [--timeout-ms <value>] [--format <text|json>]\n  sce config validate [--config <path>] [--log-level <error|warn|info|debug>] [--timeout-ms <value>] [--format <text|json>]\n\nResolution precedence: flags > env > config file > defaults\nEnvironment keys: SCE_CONFIG_FILE, SCE_LOG_LEVEL, SCE_TIMEOUT_MS"
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
    )
}

fn resolve_runtime_config_with<FEnv, FRead>(
    request: &ConfigRequest,
    cwd: &Path,
    env_lookup: FEnv,
    read_file: FRead,
    path_exists: fn(&Path) -> bool,
) -> Result<RuntimeConfig>
where
    FEnv: Fn(&str) -> Option<String>,
    FRead: Fn(&Path) -> Result<String>,
{
    let loaded_config_path = resolve_config_path(request, cwd, &env_lookup, path_exists)?;
    let file_config = match loaded_config_path.as_ref() {
        Some(path) => {
            let raw = read_file(&path.path)?;
            parse_file_config(&raw, &path.path)?
        }
        None => FileConfig {
            log_level: None,
            timeout_ms: None,
        },
    };

    let mut resolved_log_level = ResolvedValue {
        value: LogLevel::Info,
        source: ValueSource::Default,
    };
    if let Some(value) = file_config.log_level {
        resolved_log_level = ResolvedValue {
            value,
            source: ValueSource::ConfigFile,
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
            value,
            source: ValueSource::ConfigFile,
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
        loaded_config_path,
        log_level: resolved_log_level,
        timeout_ms: resolved_timeout_ms,
    })
}

fn resolve_config_path<FEnv>(
    request: &ConfigRequest,
    cwd: &Path,
    env_lookup: &FEnv,
    path_exists: fn(&Path) -> bool,
) -> Result<Option<LoadedConfigPath>>
where
    FEnv: Fn(&str) -> Option<String>,
{
    if let Some(path) = request.config_path.as_ref() {
        if !path_exists(path) {
            bail!(
                "Config file '{}' was provided via --config but does not exist.",
                path.display()
            );
        }
        return Ok(Some(LoadedConfigPath {
            path: path.clone(),
            source: ConfigPathSource::Flag,
        }));
    }

    if let Some(raw) = env_lookup("SCE_CONFIG_FILE") {
        let path = PathBuf::from(raw);
        if !path_exists(&path) {
            bail!(
                "Config file '{}' was provided via SCE_CONFIG_FILE but does not exist.",
                path.display()
            );
        }
        return Ok(Some(LoadedConfigPath {
            path,
            source: ConfigPathSource::Env,
        }));
    }

    let default_path = cwd.join(".sce").join("config.json");
    if path_exists(&default_path) {
        return Ok(Some(LoadedConfigPath {
            path: default_path,
            source: ConfigPathSource::DefaultDiscovered,
        }));
    }

    Ok(None)
}

fn parse_file_config(raw: &str, path: &Path) -> Result<FileConfig> {
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
            Some(LogLevel::parse(
                raw,
                &format!("config file '{}'", path.display()),
            )?)
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
            Some(parsed)
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
                format_config_path_text(runtime),
                format!(
                    "- log_level: {} (source: {})",
                    runtime.log_level.value.as_str(),
                    runtime.log_level.source.as_str()
                ),
                format!(
                    "- timeout_ms: {} (source: {})",
                    runtime.timeout_ms.value,
                    runtime.timeout_ms.source.as_str()
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
                    "config_path": format_config_path_json(runtime),
                    "resolved": {
                        "log_level": {
                            "value": runtime.log_level.value.as_str(),
                            "source": runtime.log_level.source.as_str(),
                        },
                        "timeout_ms": {
                            "value": runtime.timeout_ms.value,
                            "source": runtime.timeout_ms.source.as_str(),
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
                format_config_path_text(runtime),
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
                    "config_path": format_config_path_json(runtime),
                    "issues": []
                }
            });
            serde_json::to_string_pretty(&payload)
                .expect("config validate payload should serialize")
        }
    }
}

fn format_config_path_text(runtime: &RuntimeConfig) -> String {
    match runtime.loaded_config_path.as_ref() {
        Some(path) => format!(
            "Config file: {} (source: {})",
            path.path.display(),
            path.source.as_str()
        ),
        None => "Config file: (none discovered)".to_string(),
    }
}

fn format_config_path_json(runtime: &RuntimeConfig) -> Value {
    match runtime.loaded_config_path.as_ref() {
        Some(path) => json!({
            "path": path.path.display().to_string(),
            "source": path.source.as_str(),
        }),
        None => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_config_subcommand, resolve_runtime_config_with, ConfigRequest, ConfigSubcommand,
        LogLevel, ReportFormat,
    };
    use anyhow::Result;
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
        )
        .expect_err("unknown config keys should fail");
        assert!(error.to_string().contains("contains unknown key 'unknown'"));
    }
}
