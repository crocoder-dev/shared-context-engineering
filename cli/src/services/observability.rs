use std::fs::{File, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Result};
use chrono::Utc;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::SdkTracerProvider;
use serde_json::json;
use tracing_subscriber::prelude::*;

use crate::services::config;
use crate::services::default_paths::{repo_dir, repo_file};
use crate::services::error::ClassifiedError;
use crate::services::security::redact_sensitive_text;
use crate::services::style::{error_text, heading};

pub const NAME: &str = "observability";

const ENV_LOG_LEVEL: &str = "SCE_LOG_LEVEL";
const ENV_LOG_FORMAT: &str = "SCE_LOG_FORMAT";
const ENV_LOG_FILE: &str = "SCE_LOG_FILE";
const ENV_LOG_FILE_MODE: &str = "SCE_LOG_FILE_MODE";
const ENV_OTEL_ENABLED: &str = "SCE_OTEL_ENABLED";
const ENV_OTEL_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
const ENV_OTEL_PROTOCOL: &str = "OTEL_EXPORTER_OTLP_PROTOCOL";

const DEFAULT_OTEL_ENDPOINT: &str = "http://127.0.0.1:4317";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OtlpProtocol {
    Grpc,
    HttpProtobuf,
}

impl OtlpProtocol {
    #[cfg_attr(not(test), allow(dead_code))]
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "grpc" => Ok(Self::Grpc),
            "http/protobuf" => Ok(Self::HttpProtobuf),
            _ => bail!("Invalid {ENV_OTEL_PROTOCOL} '{raw}'. Valid values: grpc, http/protobuf."),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TelemetryConfig {
    enabled: bool,
    endpoint: String,
    protocol: OtlpProtocol,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: DEFAULT_OTEL_ENDPOINT.to_string(),
            protocol: OtlpProtocol::Grpc,
        }
    }
}

impl TelemetryConfig {
    #[cfg_attr(not(test), allow(dead_code))]
    fn from_env_lookup<F>(lookup: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut config = Self::default();

        if let Some(raw) = lookup(ENV_OTEL_ENABLED) {
            config.enabled = parse_bool_env(ENV_OTEL_ENABLED, &raw)?;
        }

        if !config.enabled {
            return Ok(config);
        }

        if let Some(raw) = lookup(ENV_OTEL_PROTOCOL) {
            config.protocol = OtlpProtocol::parse(&raw)?;
        }

        if let Some(raw) = lookup(ENV_OTEL_ENDPOINT) {
            config.endpoint = raw;
        }

        validate_otlp_endpoint(&config.endpoint)?;

        Ok(config)
    }
}

pub struct TelemetryRuntime {
    provider: Option<SdkTracerProvider>,
}

impl TelemetryRuntime {
    pub fn from_resolved_config(
        config: &config::ResolvedObservabilityRuntimeConfig,
    ) -> Result<Self> {
        Self::from_config(&TelemetryConfig {
            enabled: config.otel_enabled,
            // Clone required: TelemetryConfig owns the endpoint String
            endpoint: config.otel_endpoint.clone(),
            protocol: match config.otel_protocol {
                config::OtlpProtocol::Grpc => OtlpProtocol::Grpc,
                config::OtlpProtocol::HttpProtobuf => OtlpProtocol::HttpProtobuf,
            },
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn from_env_lookup<F>(lookup: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let config = TelemetryConfig::from_env_lookup(lookup)?;
        Self::from_config(&config)
    }

    fn from_config(config: &TelemetryConfig) -> Result<Self> {
        if !config.enabled {
            return Ok(Self { provider: None });
        }

        let exporter = match config.protocol {
            OtlpProtocol::Grpc => opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                // Clone required: with_endpoint takes ownership of the endpoint String
                .with_endpoint(config.endpoint.clone())
                .build()
                .map_err(|error| anyhow!("Failed to initialize OTLP gRPC exporter: {error}"))?,
            OtlpProtocol::HttpProtobuf => opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                // Clone required: with_endpoint takes ownership of the endpoint String
                .with_endpoint(config.endpoint.clone())
                .build()
                .map_err(|error| anyhow!("Failed to initialize OTLP HTTP exporter: {error}"))?,
        };

        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter)
            .build();

        Ok(Self {
            provider: Some(provider),
        })
    }

    pub fn with_default_subscriber<T, F>(&self, action: F) -> T
    where
        F: FnOnce() -> T,
    {
        if let Some(provider) = &self.provider {
            let tracer = provider.tracer("sce-cli");
            let subscriber = tracing_subscriber::registry()
                .with(tracing_opentelemetry::layer().with_tracer(tracer));
            return tracing::subscriber::with_default(subscriber, action);
        }

        action()
    }
}

impl Drop for TelemetryRuntime {
    fn drop(&mut self) {
        if let Some(provider) = self.provider.take() {
            // Best-effort shutdown during drop; errors are logged but not propagated
            if let Err(e) = provider.shutdown() {
                eprintln!("Warning: Failed to shutdown telemetry provider: {e:?}");
            }
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn parse_bool_env(key: &str, raw: &str) -> Result<bool> {
    match raw {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => bail!("Invalid {key} '{raw}'. Valid values: true, false, 1, 0."),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogFormat {
    Text,
    Json,
}

impl LogFormat {
    #[cfg_attr(not(test), allow(dead_code))]
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => bail!("Invalid {ENV_LOG_FORMAT} '{raw}'. Valid values: text, json."),
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
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    #[cfg_attr(not(test), allow(dead_code))]
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "error" => Ok(Self::Error),
            "warn" => Ok(Self::Warn),
            "info" => Ok(Self::Info),
            "debug" => Ok(Self::Debug),
            _ => bail!("Invalid {ENV_LOG_LEVEL} '{raw}'. Valid values: error, warn, info, debug."),
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

    fn severity(self) -> u8 {
        match self {
            Self::Error => 1,
            Self::Warn => 2,
            Self::Info => 3,
            Self::Debug => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservabilityConfig {
    pub level: LogLevel,
    pub format: LogFormat,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Error,
            format: LogFormat::Text,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Logger {
    config: ObservabilityConfig,
    file_sink: Option<LogFileSink>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LogFileMode {
    Truncate,
    Append,
}

impl LogFileMode {
    #[cfg_attr(not(test), allow(dead_code))]
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "truncate" => Ok(Self::Truncate),
            "append" => Ok(Self::Append),
            _ => bail!("Invalid {ENV_LOG_FILE_MODE} '{raw}'. Valid values: truncate, append."),
        }
    }
}

#[derive(Clone, Debug)]
struct LogFileSink {
    path: PathBuf,
    writer: Arc<Mutex<File>>,
}

impl LogFileSink {
    fn open(path: PathBuf, mode: LogFileMode) -> Result<Self> {
        if path.as_os_str().is_empty() {
            bail!(
                "Invalid {ENV_LOG_FILE} ''. Try: set it to an absolute or relative file path, for example {}.",
                default_repo_log_file_example()
            );
        }

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    anyhow!(
                        "Failed to prepare log directory '{}': {}",
                        parent.display(),
                        error
                    )
                })?;
            }
        }

        let mut options = OpenOptions::new();
        options.create(true).write(true);
        match mode {
            LogFileMode::Truncate => {
                options.truncate(true);
            }
            LogFileMode::Append => {
                options.append(true);
            }
        }

        #[cfg(unix)]
        {
            options.mode(0o600);
        }

        let file = options.open(&path).map_err(|error| {
            anyhow!(
                "Failed to open {} '{}': {}. Try: verify the path is writable or unset {}.",
                ENV_LOG_FILE,
                path.display(),
                error,
                ENV_LOG_FILE
            )
        })?;

        #[cfg(unix)]
        enforce_unix_log_file_permissions(&path)?;

        Ok(Self {
            path,
            writer: Arc::new(Mutex::new(file)),
        })
    }

    fn write_line(&self, line: &str) -> Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| anyhow!("Log file writer lock poisoned"))?;
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }
}

fn default_repo_log_file_example() -> String {
    format!("{}/{}", repo_dir::SCE, repo_file::SCE_LOG)
}

impl Logger {
    pub fn from_resolved_config(
        config: &config::ResolvedObservabilityRuntimeConfig,
    ) -> Result<Self> {
        let file_sink = match config.log_file.as_deref() {
            Some(path) => Some(LogFileSink::open(
                PathBuf::from(path),
                match config.log_file_mode {
                    config::LogFileMode::Truncate => LogFileMode::Truncate,
                    config::LogFileMode::Append => LogFileMode::Append,
                },
            )?),
            None => None,
        };

        Ok(Self {
            config: ObservabilityConfig {
                level: match config.log_level {
                    config::LogLevel::Error => LogLevel::Error,
                    config::LogLevel::Warn => LogLevel::Warn,
                    config::LogLevel::Info => LogLevel::Info,
                    config::LogLevel::Debug => LogLevel::Debug,
                },
                format: match config.log_format {
                    config::LogFormat::Text => LogFormat::Text,
                    config::LogFormat::Json => LogFormat::Json,
                },
            },
            file_sink,
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn from_env_lookup<F>(lookup: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut config = ObservabilityConfig::default();
        let mut file_path = None;
        let mut file_mode_raw_seen = false;
        let mut file_mode = LogFileMode::Truncate;

        if let Some(raw) = lookup(ENV_LOG_LEVEL) {
            config.level = LogLevel::parse(&raw)?;
        }

        if let Some(raw) = lookup(ENV_LOG_FORMAT) {
            config.format = LogFormat::parse(&raw)?;
        }

        if let Some(raw) = lookup(ENV_LOG_FILE) {
            file_path = Some(PathBuf::from(raw));
        }

        if let Some(raw) = lookup(ENV_LOG_FILE_MODE) {
            file_mode_raw_seen = true;
            file_mode = LogFileMode::parse(&raw)?;
        }

        if file_path.is_none() && file_mode_raw_seen {
            bail!(
                "{ENV_LOG_FILE_MODE} requires {ENV_LOG_FILE}. Try: set {ENV_LOG_FILE} to a file path or unset {ENV_LOG_FILE_MODE}."
            );
        }

        let file_sink = match file_path {
            Some(path) => Some(LogFileSink::open(path, file_mode)?),
            None => None,
        };

        Ok(Self { config, file_sink })
    }

    pub fn info(&self, event_id: &str, message: &str, fields: &[(&str, &str)]) {
        self.log(LogLevel::Info, event_id, message, fields);
    }

    pub fn debug(&self, event_id: &str, message: &str, fields: &[(&str, &str)]) {
        self.log(LogLevel::Debug, event_id, message, fields);
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn error(&self, event_id: &str, message: &str, fields: &[(&str, &str)]) {
        self.log(LogLevel::Error, event_id, message, fields);
    }

    pub fn log_classified_error(&self, error: &ClassifiedError) {
        let event_id = format!("sce.error.{}", error.code());
        self.log(
            LogLevel::Error,
            &event_id,
            error.message(),
            &[
                ("error_code", error.code()),
                ("error_class", error.class().as_str()),
            ],
        );
    }

    fn log(&self, level: LogLevel, event_id: &str, message: &str, fields: &[(&str, &str)]) {
        if !self.enabled(level) {
            return;
        }

        emit_tracing_event(level, event_id, message, fields);

        let line = self.render_line(level, event_id, message, fields);
        let redacted_line = redact_sensitive_text(&line);
        eprintln!("{redacted_line}");

        if let Some(file_sink) = &self.file_sink {
            if let Err(error) = file_sink.write_line(&redacted_line) {
                let diagnostic = redact_sensitive_text(&format!(
                    "Failed to write log file '{}': {}. Try: verify the file is writable or unset {}.",
                    file_sink.path.display(),
                    error,
                    ENV_LOG_FILE
                ));
                eprintln!("{}: {}", heading("Error"), error_text(&diagnostic));
            }
        }
    }

    fn enabled(&self, level: LogLevel) -> bool {
        level.severity() <= self.config.level.severity()
    }

    fn render_line(
        &self,
        level: LogLevel,
        event_id: &str,
        message: &str,
        fields: &[(&str, &str)],
    ) -> String {
        let timestamp = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

        match self.config.format {
            LogFormat::Text => {
                let mut line = format!(
                    "timestamp={} log_format={} level={} event_id={} message={}",
                    timestamp,
                    self.config.format.as_str(),
                    level.as_str(),
                    event_id,
                    message
                );

                for (key, value) in fields {
                    line.push(' ');
                    line.push_str(key);
                    line.push('=');
                    line.push_str(value);
                }

                line
            }
            LogFormat::Json => {
                let details = fields
                    .iter()
                    .map(|(key, value)| {
                        (
                            (*key).to_string(),
                            serde_json::Value::String((*value).to_string()),
                        )
                    })
                    .collect::<serde_json::Map<String, serde_json::Value>>();
                json!({
                    "timestamp": timestamp,
                    "log_format": self.config.format.as_str(),
                    "level": level.as_str(),
                    "event_id": event_id,
                    "message": message,
                    "fields": details,
                })
                .to_string()
            }
        }
    }
}

fn emit_tracing_event(level: LogLevel, event_id: &str, message: &str, fields: &[(&str, &str)]) {
    let detail_fields = fields
        .iter()
        .map(|(key, value)| {
            (
                (*key).to_string(),
                serde_json::Value::String((*value).to_string()),
            )
        })
        .collect::<serde_json::Map<String, serde_json::Value>>();
    let fields_json = serde_json::Value::Object(detail_fields).to_string();

    match level {
        LogLevel::Error => tracing::error!(
            target: "sce",
            event_id = event_id,
            event_message = message,
            fields = %fields_json
        ),
        LogLevel::Warn => tracing::warn!(
            target: "sce",
            event_id = event_id,
            event_message = message,
            fields = %fields_json
        ),
        LogLevel::Info => tracing::info!(
            target: "sce",
            event_id = event_id,
            event_message = message,
            fields = %fields_json
        ),
        LogLevel::Debug => tracing::debug!(
            target: "sce",
            event_id = event_id,
            event_message = message,
            fields = %fields_json
        ),
    }
}

#[cfg(unix)]
fn enforce_unix_log_file_permissions(path: &Path) -> Result<()> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        anyhow!(
            "Failed to inspect permissions for log file '{}': {}",
            path.display(),
            error
        )
    })?;

    let mode = metadata.mode() & 0o777;
    if mode & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
            |error| {
                anyhow!(
                    "Failed to secure permissions for {} '{}': {}. Try: run 'chmod 600 {}' and retry.",
                    ENV_LOG_FILE,
                    path.display(),
                    error,
                    path.display()
                )
            },
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use super::{
        validate_otlp_endpoint, LogFormat, LogLevel, Logger, TelemetryConfig, TelemetryRuntime,
        DEFAULT_OTEL_ENDPOINT,
    };
    use crate::services::config::{
        LogFileMode as ConfigLogFileMode, LogFormat as ConfigLogFormat, LogLevel as ConfigLogLevel,
        OtlpProtocol as ConfigOtlpProtocol, ResolvedObservabilityRuntimeConfig,
    };
    use crate::services::error::ClassifiedError;

    fn unique_temp_log_path(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos();
        std::env::temp_dir().join(format!("sce-observability-{label}-{nanos}.log"))
    }

    #[test]
    fn logger_defaults_to_error_text() {
        let logger = Logger::from_env_lookup(|_| None).expect("logger should parse defaults");
        let line = logger.render_line(LogLevel::Error, "sce.test.event", "hello", &[]);
        assert!(
            line.starts_with("timestamp="),
            "line should start with timestamp"
        );
        assert!(line.contains(" log_format=text level=error event_id=sce.test.event message=hello"));
    }

    #[test]
    fn logger_parses_env_level_and_format() {
        let logger = Logger::from_env_lookup(|key| match key {
            "SCE_LOG_LEVEL" => Some("debug".to_string()),
            "SCE_LOG_FORMAT" => Some("json".to_string()),
            _ => None,
        })
        .expect("logger should parse env");

        let line = logger.render_line(
            LogLevel::Info,
            "sce.test.event",
            "hello",
            &[("command", "setup")],
        );
        // Parse as JSON to verify structure
        let parsed: serde_json::Value =
            serde_json::from_str(&line).expect("line should be valid JSON");
        assert!(
            parsed.get("timestamp").is_some(),
            "JSON should contain timestamp field"
        );
        assert_eq!(parsed["log_format"], "json");
        assert_eq!(parsed["level"], "info");
        assert_eq!(parsed["event_id"], "sce.test.event");
        assert_eq!(parsed["message"], "hello");
        assert_eq!(parsed["fields"]["command"], "setup");
    }

    #[test]
    fn logger_rejects_invalid_level() {
        let error = Logger::from_env_lookup(|key| {
            if key == "SCE_LOG_LEVEL" {
                return Some("trace".to_string());
            }
            None
        })
        .expect_err("invalid level should fail");

        assert_eq!(
            error.to_string(),
            "Invalid SCE_LOG_LEVEL 'trace'. Valid values: error, warn, info, debug."
        );
    }

    #[test]
    fn logger_rejects_log_file_mode_without_path() {
        let error = Logger::from_env_lookup(|key| {
            if key == "SCE_LOG_FILE_MODE" {
                return Some("append".to_string());
            }
            None
        })
        .expect_err("log file mode without path should fail");

        assert_eq!(
            error.to_string(),
            "SCE_LOG_FILE_MODE requires SCE_LOG_FILE. Try: set SCE_LOG_FILE to a file path or unset SCE_LOG_FILE_MODE."
        );
    }

    #[test]
    fn logger_rejects_invalid_log_file_mode() {
        let error = Logger::from_env_lookup(|key| match key {
            "SCE_LOG_FILE" => Some(".sce/sce.log".to_string()),
            "SCE_LOG_FILE_MODE" => Some("rotate".to_string()),
            _ => None,
        })
        .expect_err("invalid log file mode should fail");

        assert_eq!(
            error.to_string(),
            "Invalid SCE_LOG_FILE_MODE 'rotate'. Valid values: truncate, append."
        );
    }

    #[test]
    fn logger_file_sink_truncates_by_default() {
        let log_path = unique_temp_log_path("truncate-default");
        std::fs::write(&log_path, "old-data\n").expect("should write prior content");

        let logger = Logger::from_env_lookup(|key| {
            if key == "SCE_LOG_FILE" {
                return Some(log_path.display().to_string());
            }
            None
        })
        .expect("logger should initialize with file sink");

        logger.error("sce.test.event", "hello", &[("command", "setup")]);

        let content = std::fs::read_to_string(&log_path).expect("should read log file");
        assert!(content.contains("event_id=sce.test.event"));
        assert!(!content.contains("old-data"));

        let _ = std::fs::remove_file(log_path);
    }

    #[test]
    fn logger_file_sink_appends_when_requested() {
        let log_path = unique_temp_log_path("append");
        std::fs::write(&log_path, "first\n").expect("should write prior content");

        let logger = Logger::from_env_lookup(|key| match key {
            "SCE_LOG_FILE" => Some(log_path.display().to_string()),
            "SCE_LOG_FILE_MODE" => Some("append".to_string()),
            _ => None,
        })
        .expect("logger should initialize with append sink");

        logger.error("sce.test.event", "hello", &[]);

        let content = std::fs::read_to_string(&log_path).expect("should read log file");
        assert!(content.starts_with("first\n"));
        assert!(content.contains("event_id=sce.test.event"));

        let _ = std::fs::remove_file(log_path);
    }

    #[cfg(unix)]
    #[test]
    fn logger_tightens_world_readable_log_file_permissions() {
        let log_path = unique_temp_log_path("permissions");
        std::fs::write(&log_path, "seed\n").expect("should write seed file");
        std::fs::set_permissions(&log_path, std::fs::Permissions::from_mode(0o644))
            .expect("should set loose mode");

        let logger = Logger::from_env_lookup(|key| {
            if key == "SCE_LOG_FILE" {
                return Some(log_path.display().to_string());
            }
            None
        })
        .expect("logger should repair loose permissions");

        logger.error("sce.test.event", "hello", &[]);

        let mode = std::fs::metadata(&log_path)
            .expect("metadata should be readable")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);

        let _ = std::fs::remove_file(log_path);
    }

    #[test]
    fn logger_level_filtering_is_threshold_based() {
        let logger = Logger::from_env_lookup(|key| {
            if key == "SCE_LOG_LEVEL" {
                return Some("warn".to_string());
            }
            None
        })
        .expect("logger should parse warn level");

        assert!(logger.enabled(LogLevel::Error));
        assert!(logger.enabled(LogLevel::Warn));
        assert!(!logger.enabled(LogLevel::Info));
        assert!(!logger.enabled(LogLevel::Debug));
    }

    #[test]
    fn logger_uses_config_backed_file_sink() {
        let log_path = unique_temp_log_path("config-backed");
        std::fs::write(&log_path, "old-data\n").expect("should write prior content");

        let logger = Logger::from_resolved_config(&ResolvedObservabilityRuntimeConfig {
            log_level: ConfigLogLevel::Error,
            log_format: ConfigLogFormat::Text,
            log_file: Some(log_path.display().to_string()),
            log_file_mode: ConfigLogFileMode::Append,
            otel_enabled: false,
            otel_endpoint: DEFAULT_OTEL_ENDPOINT.to_string(),
            otel_protocol: ConfigOtlpProtocol::Grpc,
            loaded_config_paths: vec![],
        })
        .expect("logger should initialize from resolved config");

        logger.error("sce.test.event", "hello", &[]);

        let content = std::fs::read_to_string(&log_path).expect("should read log file");
        assert!(content.starts_with("old-data\n"));
        assert!(content.contains("event_id=sce.test.event"));

        let _ = std::fs::remove_file(log_path);
    }

    #[test]
    fn log_format_parser_accepts_documented_values() {
        assert_eq!(
            LogFormat::parse("text").expect("text should parse"),
            LogFormat::Text
        );
        assert_eq!(
            LogFormat::parse("json").expect("json should parse"),
            LogFormat::Json
        );
    }

    #[test]
    fn telemetry_defaults_to_disabled() {
        let runtime = TelemetryRuntime::from_env_lookup(|_| None)
            .expect("telemetry runtime should parse default config");
        assert!(runtime.provider.is_none());
    }

    #[test]
    fn telemetry_rejects_invalid_enabled_value() {
        let error = TelemetryConfig::from_env_lookup(|key| {
            if key == "SCE_OTEL_ENABLED" {
                return Some("maybe".to_string());
            }
            None
        })
        .expect_err("invalid enabled value should fail");

        assert_eq!(
            error.to_string(),
            "Invalid SCE_OTEL_ENABLED 'maybe'. Valid values: true, false, 1, 0."
        );
    }

    #[test]
    fn telemetry_rejects_invalid_protocol_when_enabled() {
        let error = TelemetryConfig::from_env_lookup(|key| match key {
            "SCE_OTEL_ENABLED" => Some("true".to_string()),
            "OTEL_EXPORTER_OTLP_PROTOCOL" => Some("udp".to_string()),
            _ => None,
        })
        .expect_err("invalid protocol should fail");

        assert_eq!(
            error.to_string(),
            "Invalid OTEL_EXPORTER_OTLP_PROTOCOL 'udp'. Valid values: grpc, http/protobuf."
        );
    }

    #[test]
    fn telemetry_rejects_invalid_endpoint_when_enabled() {
        let error = validate_otlp_endpoint("collector:4317")
            .expect_err("non-URL endpoint should fail validation");
        assert_eq!(
            error.to_string(),
            "Invalid OTEL_EXPORTER_OTLP_ENDPOINT 'collector:4317'. Try: set it to an absolute http(s) URL, for example http://127.0.0.1:4317."
        );
    }

    #[test]
    fn telemetry_uses_config_backed_enablement() {
        let tokio_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime should build");
        let runtime = tokio_runtime
            .block_on(async {
                TelemetryRuntime::from_resolved_config(&ResolvedObservabilityRuntimeConfig {
                    log_level: ConfigLogLevel::Error,
                    log_format: ConfigLogFormat::Text,
                    log_file: None,
                    log_file_mode: ConfigLogFileMode::Truncate,
                    otel_enabled: true,
                    otel_endpoint: "http://127.0.0.1:4317".to_string(),
                    otel_protocol: ConfigOtlpProtocol::Grpc,
                    loaded_config_paths: vec![],
                })
            })
            .expect("telemetry runtime should initialize from resolved config");

        assert!(runtime.provider.is_some());
    }

    #[test]
    fn log_classified_error_includes_error_code_in_event_id() {
        let logger = Logger::from_env_lookup(|_| None).expect("logger should parse defaults");
        let error = ClassifiedError::runtime("Test runtime error");
        let line = logger.render_line(
            LogLevel::Error,
            &format!("sce.error.{}", error.code()),
            error.message(),
            &[
                ("error_code", error.code()),
                ("error_class", error.class().as_str()),
            ],
        );

        assert!(line.starts_with("timestamp="));
        assert!(line.contains("level=error"));
        assert!(line.contains("event_id=sce.error.SCE-ERR-RUNTIME"));
        assert!(line.contains("error_code=SCE-ERR-RUNTIME"));
        assert!(line.contains("error_class=runtime"));
    }

    #[test]
    fn log_classified_error_json_format_includes_all_fields() {
        let logger = Logger::from_env_lookup(|key| {
            if key == "SCE_LOG_FORMAT" {
                return Some("json".to_string());
            }
            None
        })
        .expect("logger should parse json format");

        let error = ClassifiedError::validation("Validation failed");
        let line = logger.render_line(
            LogLevel::Error,
            &format!("sce.error.{}", error.code()),
            error.message(),
            &[
                ("error_code", error.code()),
                ("error_class", error.class().as_str()),
            ],
        );

        let parsed: serde_json::Value =
            serde_json::from_str(&line).expect("line should be valid JSON");
        assert!(
            parsed.get("timestamp").is_some(),
            "JSON should contain timestamp field"
        );
        assert_eq!(parsed["level"], "error");
        assert_eq!(parsed["event_id"], "sce.error.SCE-ERR-VALIDATION");
        assert_eq!(parsed["fields"]["error_code"], "SCE-ERR-VALIDATION");
        assert_eq!(parsed["fields"]["error_class"], "validation");
    }

    #[test]
    fn log_classified_error_supports_all_error_classes() {
        let logger = Logger::from_env_lookup(|_| None).expect("logger should parse defaults");

        // Test all error classes
        let parse_error = ClassifiedError::parse("Parse error");
        let validation_error = ClassifiedError::validation("Validation error");
        let runtime_error = ClassifiedError::runtime("Runtime error");
        let dependency_error = ClassifiedError::dependency("Dependency error");

        // Verify each has correct code and class
        assert_eq!(parse_error.code(), "SCE-ERR-PARSE");
        assert_eq!(parse_error.class().as_str(), "parse");

        assert_eq!(validation_error.code(), "SCE-ERR-VALIDATION");
        assert_eq!(validation_error.class().as_str(), "validation");

        assert_eq!(runtime_error.code(), "SCE-ERR-RUNTIME");
        assert_eq!(runtime_error.class().as_str(), "runtime");

        assert_eq!(dependency_error.code(), "SCE-ERR-DEPENDENCY");
        assert_eq!(dependency_error.class().as_str(), "dependency");

        // Verify log lines can be rendered for each
        for (error, expected_code) in [
            (&parse_error, "SCE-ERR-PARSE"),
            (&validation_error, "SCE-ERR-VALIDATION"),
            (&runtime_error, "SCE-ERR-RUNTIME"),
            (&dependency_error, "SCE-ERR-DEPENDENCY"),
        ] {
            let line = logger.render_line(
                LogLevel::Error,
                &format!("sce.error.{}", error.code()),
                error.message(),
                &[
                    ("error_code", error.code()),
                    ("error_class", error.class().as_str()),
                ],
            );
            assert!(line.contains(&format!("error_code={expected_code}")));
        }
    }
}
