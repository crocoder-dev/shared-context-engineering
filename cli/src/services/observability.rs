use std::{
    cmp::Ordering,
    collections::BTreeMap,
    fmt::Write as FmtWrite,
    fs::{self, OpenOptions},
    io::{self, ErrorKind, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
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
    log_file_retention_limit: usize,
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
            log_file_retention_limit: config.log_file_retention_limit,
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

        Ok(Self {
            config,
            log_dir,
            log_file_retention_limit: config::DEFAULT_LOG_FILE_RETENTION_LIMIT,
        })
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
        append_log_line(&path, redacted_line, self.log_file_retention_limit)
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

fn append_log_line(path: &Path, redacted_line: &str, retention_limit: usize) -> Result<()> {
    append_log_line_with_cleanup(path, redacted_line, |log_dir| {
        enforce_log_retention(log_dir, retention_limit)
    })
}

fn append_log_line_with_cleanup<F>(path: &Path, redacted_line: &str, cleanup: F) -> Result<()>
where
    F: FnOnce(&Path) -> Result<()>,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create log directory '{}'", parent.display()))?;
    }

    let lock = file_log_lock(path);
    let _guard = lock.lock().map_err(|error| {
        anyhow::anyhow!("failed to lock log file '{}': {error}", path.display())
    })?;

    match persist_log_line(path, redacted_line) {
        Ok(write_target) => {
            run_log_retention_after_creation(path, write_target, cleanup);
            Ok(())
        }
        Err(primary_error) => attempt_v2_log_fallback(
            path,
            redacted_line,
            &primary_error,
            |fallback_path, line| append_log_line_once_with_cleanup(fallback_path, line, cleanup),
        ),
    }
}

fn attempt_v2_log_fallback<F>(
    primary_path: &Path,
    redacted_line: &str,
    primary_error: &anyhow::Error,
    persist_fallback: F,
) -> Result<()>
where
    F: FnOnce(&Path, &str) -> Result<()>,
{
    let fallback_path = v2_log_path(primary_path);
    persist_fallback(&fallback_path, redacted_line).map_err(|fallback_error| {
        anyhow::anyhow!(
            "primary log file persistence failed for '{}': {primary_error:#}; v2 fallback log file persistence failed for '{}': {fallback_error:#}",
            primary_path.display(),
            fallback_path.display(),
        )
    })
}

fn append_log_line_once_with_cleanup<F>(path: &Path, redacted_line: &str, cleanup: F) -> Result<()>
where
    F: FnOnce(&Path) -> Result<()>,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create log directory '{}'", parent.display()))?;
    }

    let lock = file_log_lock(path);
    let _guard = lock.lock().map_err(|error| {
        anyhow::anyhow!("failed to lock log file '{}': {error}", path.display())
    })?;

    let write_target = persist_log_line(path, redacted_line)?;
    run_log_retention_after_creation(path, write_target, cleanup);

    Ok(())
}

fn persist_log_line(path: &Path, redacted_line: &str) -> Result<LogWriteTarget> {
    let (mut file, write_target) = open_log_file_for_append(path)?;
    writeln!(file, "{redacted_line}")
        .with_context(|| format!("failed to append log line to '{}'", path.display()))?;
    file.flush()
        .with_context(|| format!("failed to flush log file '{}'", path.display()))?;

    Ok(write_target)
}

fn run_log_retention_after_creation<F>(path: &Path, write_target: LogWriteTarget, cleanup: F)
where
    F: FnOnce(&Path) -> Result<()>,
{
    if write_target == LogWriteTarget::Created {
        if let Some(parent) = path.parent() {
            if let Err(error) = cleanup(parent) {
                let diagnostic = redact_sensitive_text(&format!(
                    "Failed to clean up SCE log files: {error}. Logging continues on stderr."
                ));
                emit_stderr_line(&diagnostic);
            }
        }
    }
}

fn v2_log_path(path: &Path) -> PathBuf {
    let mut file_name = path.file_stem().unwrap_or(path.as_os_str()).to_os_string();
    file_name.push("-v2.");
    file_name.push(LOG_FILE_EXTENSION);
    path.with_file_name(file_name)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LogWriteTarget {
    Created,
    Existing,
}

fn open_log_file_for_append(path: &Path) -> Result<(fs::File, LogWriteTarget)> {
    loop {
        let mut create_options = OpenOptions::new();
        create_options.create_new(true).append(true);
        configure_owner_only_file_permissions(&mut create_options);

        match create_options.open(path) {
            Ok(file) => return Ok((file, LogWriteTarget::Created)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                let mut append_options = OpenOptions::new();
                append_options.append(true);
                match append_options.open(path) {
                    Ok(file) => return Ok((file, LogWriteTarget::Existing)),
                    Err(error) if error.kind() == ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!("failed to open log file '{}' for append", path.display())
                        });
                    }
                }
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to open log file '{}' for append", path.display())
                });
            }
        }
    }
}

#[derive(Debug)]
struct ManagedLogFile {
    path: PathBuf,
    modified: SystemTime,
}

fn enforce_log_retention(log_dir: &Path, retention_limit: usize) -> Result<()> {
    enforce_log_retention_with(log_dir, retention_limit, |path| fs::remove_file(path))
}

fn enforce_log_retention_with<F>(
    log_dir: &Path,
    retention_limit: usize,
    mut remove_file: F,
) -> Result<()>
where
    F: FnMut(&Path) -> io::Result<()>,
{
    let (mut managed_files, mut errors) = collect_managed_log_files(log_dir)?;

    managed_files.sort_by(|left, right| match right.modified.cmp(&left.modified) {
        Ordering::Equal => left.path.cmp(&right.path),
        ordering => ordering,
    });

    for managed_file in managed_files.into_iter().skip(retention_limit) {
        if let Err(error) = remove_file(&managed_file.path) {
            errors.push(format!(
                "failed to remove old log file '{}': {error}",
                managed_file.path.display()
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("log retention cleanup incomplete: {}", errors.join("; "));
    }
}

fn collect_managed_log_files(log_dir: &Path) -> Result<(Vec<ManagedLogFile>, Vec<String>)> {
    let entries = fs::read_dir(log_dir)
        .with_context(|| format!("failed to scan log directory '{}'", log_dir.display()))?;
    let mut managed_files = Vec::new();
    let mut errors = Vec::new();

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                errors.push(format!(
                    "failed to inspect log directory entry in '{}': {error}",
                    log_dir.display()
                ));
                continue;
            }
        };
        let path = entry.path();
        if path
            .extension()
            .is_none_or(|extension| extension != LOG_FILE_EXTENSION)
        {
            continue;
        }

        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                errors.push(format!(
                    "failed to inspect log file type '{}': {error}",
                    path.display()
                ));
                continue;
            }
        };
        if !file_type.is_file() {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                errors.push(format!(
                    "failed to inspect log file metadata '{}': {error}",
                    path.display()
                ));
                continue;
            }
        };
        let modified = match metadata.modified() {
            Ok(modified) => modified,
            Err(error) => {
                errors.push(format!(
                    "failed to inspect log file modified time '{}': {error}",
                    path.display()
                ));
                continue;
            }
        };

        managed_files.push(ManagedLogFile { path, modified });
    }

    Ok((managed_files, errors))
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
