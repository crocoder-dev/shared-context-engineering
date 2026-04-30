#![allow(dead_code)]

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

use crate::services::config::{
    self, parse_bool_env_value, validate_otlp_endpoint, LogFileMode, LogFormat, LogLevel,
    OtlpProtocol, DEFAULT_OTEL_ENDPOINT, ENV_LOG_FILE, ENV_LOG_FILE_MODE, ENV_LOG_FORMAT,
    ENV_LOG_LEVEL, ENV_OTEL_ENABLED, ENV_OTEL_ENDPOINT, ENV_OTEL_PROTOCOL,
};
use crate::services::default_paths::{repo_dir, repo_file};
use crate::services::error::ClassifiedError;
use crate::services::security::redact_sensitive_text;
use crate::services::style::{error_text, heading};

pub mod traits;

pub const NAME: &str = "observability";

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
            config.enabled = parse_bool_env_value(ENV_OTEL_ENABLED, &raw)?;
        }

        if !config.enabled {
            return Ok(config);
        }

        if let Some(raw) = lookup(ENV_OTEL_PROTOCOL) {
            config.protocol = OtlpProtocol::parse_env(&raw, ENV_OTEL_PROTOCOL)?;
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
            protocol: config.otel_protocol,
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
                config.log_file_mode,
            )?),
            None => None,
        };

        Ok(Self {
            config: ObservabilityConfig {
                level: config.log_level,
                format: config.log_format,
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
            config.level = LogLevel::parse_env(&raw, ENV_LOG_LEVEL)?;
        }

        if let Some(raw) = lookup(ENV_LOG_FORMAT) {
            config.format = LogFormat::parse_env(&raw, ENV_LOG_FORMAT)?;
        }

        if let Some(raw) = lookup(ENV_LOG_FILE) {
            file_path = Some(PathBuf::from(raw));
        }

        if let Some(raw) = lookup(ENV_LOG_FILE_MODE) {
            file_mode_raw_seen = true;
            file_mode = LogFileMode::parse_env(&raw, ENV_LOG_FILE_MODE)?;
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

    pub fn warn(&self, event_id: &str, message: &str, fields: &[(&str, &str)]) {
        self.log_forced(LogLevel::Warn, event_id, message, fields);
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

        self.log_forced(level, event_id, message, fields);
    }

    fn log_forced(&self, level: LogLevel, event_id: &str, message: &str, fields: &[(&str, &str)]) {
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
