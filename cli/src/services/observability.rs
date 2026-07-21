use std::{
    collections::BTreeMap,
    fmt::Write as FmtWrite,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use anyhow::{bail, Context, Result};
use chrono::{Local, NaiveDate, Utc};
use serde_json::json;
use tracing::Level;

use crate::services::config::{
    self, LogFormat, LogLevel, ENV_LOG_DIR, ENV_LOG_FORMAT, ENV_LOG_LEVEL,
};
use crate::services::error::ClassifiedError;
use crate::services::security::redact_sensitive_text;

pub mod traits;

pub const NAME: &str = "observability";
const LOG_FILE_PREFIX: &str = "sce";
const LOG_FILE_EXTENSION: &str = "log";
const EMPTY_SESSION_ID_TOKEN: &str = "%EMPTY";

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
    log_dir: Option<PathBuf>,
}

impl Logger {
    pub fn from_resolved_config(
        config: &config::ResolvedObservabilityRuntimeConfig,
    ) -> Result<Self> {
        if let Some(log_dir) = config.log_dir.as_deref() {
            validate_log_dir(log_dir)?;
        }

        Ok(Self {
            config: ObservabilityConfig {
                level: config.log_level,
                format: config.log_format,
            },
            log_dir: config.log_dir.as_deref().map(PathBuf::from),
        })
    }

    #[allow(dead_code)]
    fn from_env_lookup<F>(lookup: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut config = ObservabilityConfig::default();

        if let Some(raw) = lookup(ENV_LOG_LEVEL) {
            config.level = LogLevel::parse_env(&raw, ENV_LOG_LEVEL)?;
        }

        if let Some(raw) = lookup(ENV_LOG_FORMAT) {
            config.format = LogFormat::parse_env(&raw, ENV_LOG_FORMAT)?;
        }

        let mut log_dir = None;
        if let Some(raw) = lookup(ENV_LOG_DIR) {
            validate_log_dir(&raw)?;
            log_dir = Some(PathBuf::from(raw));
        }

        Ok(Self { config, log_dir })
    }

    pub fn info(
        &self,
        event_id: &str,
        message: &str,
        fields: &[(&str, &str)],
        session_id: Option<&str>,
    ) {
        self.log(LogLevel::Info, event_id, message, fields, session_id);
    }

    pub fn debug(
        &self,
        event_id: &str,
        message: &str,
        fields: &[(&str, &str)],
        session_id: Option<&str>,
    ) {
        self.log(LogLevel::Debug, event_id, message, fields, session_id);
    }

    pub fn warn(
        &self,
        event_id: &str,
        message: &str,
        fields: &[(&str, &str)],
        session_id: Option<&str>,
    ) {
        self.log_forced(LogLevel::Warn, event_id, message, fields, session_id);
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn error(
        &self,
        event_id: &str,
        message: &str,
        fields: &[(&str, &str)],
        session_id: Option<&str>,
    ) {
        self.log(LogLevel::Error, event_id, message, fields, session_id);
    }

    pub fn log_classified_error(&self, error: &ClassifiedError, session_id: Option<&str>) {
        let event_id = format!("sce.error.{}", error.code());
        self.log(
            LogLevel::Error,
            &event_id,
            error.message(),
            &[
                ("error_code", error.code()),
                ("error_class", error.class().as_str()),
            ],
            session_id,
        );
    }

    fn log(
        &self,
        level: LogLevel,
        event_id: &str,
        message: &str,
        fields: &[(&str, &str)],
        session_id: Option<&str>,
    ) {
        if !self.enabled(level) {
            return;
        }

        self.log_forced(level, event_id, message, fields, session_id);
    }

    fn log_forced(
        &self,
        level: LogLevel,
        event_id: &str,
        message: &str,
        fields: &[(&str, &str)],
        session_id: Option<&str>,
    ) {
        emit_tracing_event(level, event_id, message, fields);

        let line = self.render_line(level, event_id, message, fields);
        let redacted_line = redact_sensitive_text(&line);
        emit_stderr_line(&redacted_line);

        if let Err(error) = self.write_log_line(&redacted_line, session_id) {
            let diagnostic = redact_sensitive_text(&format!(
                "Failed to write SCE log file: {error}. Logging continues on stderr."
            ));
            emit_stderr_line(&diagnostic);
        }
    }

    fn write_log_line(&self, redacted_line: &str, session_id: Option<&str>) -> Result<()> {
        let Some(log_dir) = self.log_dir.as_deref() else {
            return Ok(());
        };

        let path = current_log_path(log_dir, session_id);
        append_log_line(&path, redacted_line)
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

fn validate_log_dir(value: &str) -> Result<()> {
    if value.is_empty() {
        bail!("Invalid {ENV_LOG_DIR} ''. Try: set it to a directory path or unset {ENV_LOG_DIR}.");
    }

    Ok(())
}

fn current_log_path(log_dir: &Path, session_id: Option<&str>) -> PathBuf {
    log_path_for_date(log_dir, Local::now().date_naive(), session_id)
}

fn log_path_for_date(log_dir: &Path, date: NaiveDate, session_id: Option<&str>) -> PathBuf {
    log_dir.join(log_name_for_date(date, session_id))
}

fn log_name_for_date(date: NaiveDate, session_id: Option<&str>) -> String {
    let date = date.format("%d_%m_%Y");
    match session_id {
        Some(session_id) => format!(
            "{LOG_FILE_PREFIX}-{date}-{}.{LOG_FILE_EXTENSION}",
            sanitize_session_id_for_filename(session_id)
        ),
        None => format!("{LOG_FILE_PREFIX}-{date}.{LOG_FILE_EXTENSION}"),
    }
}

fn sanitize_session_id_for_filename(session_id: &str) -> String {
    if session_id.is_empty() {
        return EMPTY_SESSION_ID_TOKEN.to_string();
    }

    let mut sanitized = String::with_capacity(session_id.len());
    for byte in session_id.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' => {
                sanitized.push(char::from(*byte));
            }
            _ => {
                let _ = write!(&mut sanitized, "%{byte:02X}");
            }
        }
    }
    sanitized
}

fn append_log_line(path: &Path, redacted_line: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create log directory '{}'", parent.display()))?;
    }

    let lock = file_log_lock(path);
    let _guard = lock.lock().map_err(|error| {
        anyhow::anyhow!("failed to lock log file '{}': {error}", path.display())
    })?;

    let mut options = OpenOptions::new();
    options.create(true).append(true);
    configure_owner_only_file_permissions(&mut options);
    let mut file = options
        .open(path)
        .with_context(|| format!("failed to open log file '{}' for append", path.display()))?;
    writeln!(file, "{redacted_line}")
        .with_context(|| format!("failed to append log line to '{}'", path.display()))?;
    file.flush()
        .with_context(|| format!("failed to flush log file '{}'", path.display()))
}

fn file_log_lock(path: &Path) -> Arc<Mutex<()>> {
    static FILE_LOG_LOCKS: OnceLock<Mutex<BTreeMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let locks = FILE_LOG_LOCKS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut locks = locks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Arc::clone(
        locks
            .entry(path.to_path_buf())
            .or_insert_with(|| Arc::new(Mutex::new(()))),
    )
}

#[cfg(unix)]
fn configure_owner_only_file_permissions(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.mode(0o600);
}

#[cfg(not(unix))]
fn configure_owner_only_file_permissions(_options: &mut OpenOptions) {}

fn emit_stderr_line(line: &str) {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "{line}");
    let _ = stderr.flush();
}

fn emit_tracing_event(level: LogLevel, event_id: &str, message: &str, fields: &[(&str, &str)]) {
    emit_tracing_event_with_fields_json(level, event_id, message, || tracing_fields_json(fields));
}

fn tracing_event_enabled(level: LogLevel) -> bool {
    match level {
        LogLevel::Error => tracing::enabled!(target: "sce", Level::ERROR),
        LogLevel::Warn => tracing::enabled!(target: "sce", Level::WARN),
        LogLevel::Info => tracing::enabled!(target: "sce", Level::INFO),
        LogLevel::Debug => tracing::enabled!(target: "sce", Level::DEBUG),
    }
}

fn tracing_fields_json(fields: &[(&str, &str)]) -> String {
    let detail_fields = fields
        .iter()
        .map(|(key, value)| {
            (
                (*key).to_string(),
                serde_json::Value::String((*value).to_string()),
            )
        })
        .collect::<serde_json::Map<String, serde_json::Value>>();
    serde_json::Value::Object(detail_fields).to_string()
}

fn emit_tracing_event_with_fields_json<F>(
    level: LogLevel,
    event_id: &str,
    message: &str,
    fields_json: F,
) where
    F: FnOnce() -> String,
{
    if !tracing_event_enabled(level) {
        return;
    }

    let fields_json = fields_json();

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
