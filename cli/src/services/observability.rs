use std::{
    cmp::Ordering,
    collections::hash_map::DefaultHasher,
    fmt::Write as FmtWrite,
    fs::{self, OpenOptions},
    hash::{Hash, Hasher},
    io::{self, ErrorKind, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::SystemTime,
};

use anyhow::{bail, Context, Result};
use chrono::{Local, NaiveDate, Utc};
use serde_json::json;
use tracing::Level;

use crate::services::config::{self, LogFormat, LogLevel, ENV_LOG_DIR};
use crate::services::error::CliError;
use crate::services::security::redact_sensitive_text;

pub mod otel_policy;
pub mod tracing_boundary;
pub mod traits;

use tracing_boundary::{classify_event_id, cli_error_tracing_event_id, SCE_TRACING_TARGET};

pub const NAME: &str = "observability";
const LOG_FILE_PREFIX: &str = "sce";
const LOG_FILE_EXTENSION: &str = "log";
const EMPTY_SESSION_ID_TOKEN: &str = "%EMPTY";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservabilityConfig {
    pub level: LogLevel,
    pub format: LogFormat,
    pub log_to_file: bool,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Error,
            format: LogFormat::Text,
            log_to_file: true,
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
                log_to_file: config.log_to_file,
            },
            log_dir: config
                .log_to_file
                .then_some(config.log_dir.as_deref())
                .flatten()
                .map(PathBuf::from),
            log_file_retention_limit: config.log_file_retention_limit,
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

    pub fn error(
        &self,
        event_id: &str,
        message: &str,
        fields: &[(&str, &str)],
        session_id: Option<&str>,
    ) {
        self.log(LogLevel::Error, event_id, message, fields, session_id);
    }

    pub fn log_cli_error(&self, error: &CliError, session_id: Option<&str>) {
        let event_id = format!("sce.error.{}", error.code());
        let message = error.to_string();
        let fields = cli_error_fields(error);
        let field_refs: Vec<(&str, &str)> = fields
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();

        if !self.enabled(LogLevel::Error) {
            return;
        }

        self.log_classified(
            LogLevel::Error,
            &event_id,
            cli_error_tracing_event_id(error),
            &message,
            &field_refs,
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
        self.log_classified(
            level,
            event_id,
            classify_event_id(event_id),
            message,
            fields,
            session_id,
        );
    }

    fn log_classified(
        &self,
        level: LogLevel,
        event_id: &str,
        tracing_event_id: &'static str,
        message: &str,
        fields: &[(&str, &str)],
        session_id: Option<&str>,
    ) {
        emit_tracing_event(level, tracing_event_id);

        let line = self.render_line(level, event_id, message, fields);
        let redacted_line = redact_sensitive_text(&line);
        if should_emit_to_stderr(level, self.config.log_to_file) {
            emit_stderr_line(&redacted_line);
        }

        if let Err(error) = self.write_log_line(&redacted_line, session_id) {
            let diagnostic = redact_sensitive_text(&format!(
                "Failed to write SCE log file: {error}. Logging continues on stderr."
            ));
            emit_stderr_line(&diagnostic);
        }
    }

    fn write_log_line(&self, redacted_line: &str, session_id: Option<&str>) -> Result<()> {
        if !self.config.log_to_file {
            return Ok(());
        }

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

fn cli_error_surface(error: &CliError) -> &'static str {
    match error {
        CliError::User { .. } => "user",
        CliError::Internal { .. } => "internal",
    }
}

fn cli_error_technical_source(error: &CliError) -> Option<&anyhow::Error> {
    match error {
        CliError::User { source, .. } => source.as_ref(),
        CliError::Internal { source, .. } => Some(source),
    }
}

fn cli_error_fields(error: &CliError) -> Vec<(&'static str, String)> {
    let mut fields: Vec<(&str, String)> = vec![
        ("error_code", error.code().to_string()),
        ("error_class", error.class().as_str().to_string()),
        ("error_surface", cli_error_surface(error).to_string()),
    ];

    if let CliError::User {
        error: user_error, ..
    } = error
    {
        fields.push(("user_error", user_error.key().to_string()));
    }

    if let Some(source) = cli_error_technical_source(error) {
        fields.push(("error_source", format!("{source:#}")));
    }

    fields
}

fn should_emit_to_stderr(level: LogLevel, log_to_file: bool) -> bool {
    level == LogLevel::Error && !log_to_file
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

    let primary_error = {
        let _guard = log_lock_stripe(path).lock().map_err(|error| {
            anyhow::anyhow!("failed to lock log file '{}': {error}", path.display())
        })?;

        match persist_log_line(path, redacted_line) {
            Ok(write_target) => {
                run_log_retention_after_creation(path, write_target, cleanup);
                return Ok(());
            }
            Err(primary_error) => primary_error,
        }
    };

    attempt_v2_log_fallback(
        path,
        redacted_line,
        &primary_error,
        |fallback_path, line| append_log_line_once_with_cleanup(fallback_path, line, cleanup),
    )
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

    let _guard = log_lock_stripe(path).lock().map_err(|error| {
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

const LOG_LOCK_STRIPES: usize = 64;

static LOG_LOCKS: [Mutex<()>; LOG_LOCK_STRIPES] = [const { Mutex::new(()) }; LOG_LOCK_STRIPES];

fn log_lock_stripe_index(path: &Path) -> usize {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    usize::try_from(hasher.finish() % LOG_LOCK_STRIPES as u64)
        .expect("log lock stripe index must fit usize")
}

fn log_lock_stripe(path: &Path) -> &'static Mutex<()> {
    &LOG_LOCKS[log_lock_stripe_index(path)]
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

fn emit_tracing_event(level: LogLevel, tracing_event_id: &'static str) {
    if !tracing_event_enabled(level) {
        return;
    }

    let log_level = level.as_str();
    match level {
        LogLevel::Error => tracing::error!(
            target: SCE_TRACING_TARGET,
            event_id = tracing_event_id,
            log_level = log_level,
            "sce log event"
        ),
        LogLevel::Warn => tracing::warn!(
            target: SCE_TRACING_TARGET,
            event_id = tracing_event_id,
            log_level = log_level,
            "sce log event"
        ),
        LogLevel::Info => tracing::info!(
            target: SCE_TRACING_TARGET,
            event_id = tracing_event_id,
            log_level = log_level,
            "sce log event"
        ),
        LogLevel::Debug => tracing::debug!(
            target: SCE_TRACING_TARGET,
            event_id = tracing_event_id,
            log_level = log_level,
            "sce log event"
        ),
    }
}

fn tracing_event_enabled(level: LogLevel) -> bool {
    match level {
        LogLevel::Error => tracing::enabled!(target: "sce", Level::ERROR),
        LogLevel::Warn => tracing::enabled!(target: "sce", Level::WARN),
        LogLevel::Info => tracing::enabled!(target: "sce", Level::INFO),
        LogLevel::Debug => tracing::enabled!(target: "sce", Level::DEBUG),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::error::{FailureClass, UserError};

    fn field_value<'a>(fields: &'a [(&'static str, String)], key: &str) -> Option<&'a str> {
        fields
            .iter()
            .find(|(field_key, _)| *field_key == key)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn observability_fields_for_user_error_carry_surface_and_key_plus_source() {
        let error = CliError::user_with_source(
            UserError::NotAuthenticated,
            anyhow::anyhow!("missing credentials"),
        );

        let fields = cli_error_fields(&error);

        assert_eq!(field_value(&fields, "error_code"), Some("SCE-ERR-RUNTIME"));
        assert_eq!(field_value(&fields, "error_class"), Some("runtime"));
        assert_eq!(field_value(&fields, "error_surface"), Some("user"));
        assert_eq!(
            field_value(&fields, "user_error"),
            Some("auth.not_authenticated")
        );
        assert_eq!(
            field_value(&fields, "error_source"),
            Some("missing credentials")
        );
    }

    #[test]
    fn observability_fields_for_user_error_without_source_omit_error_source() {
        let error = CliError::User {
            error: UserError::NotAuthenticated,
            source: None,
        };

        let fields = cli_error_fields(&error);

        assert_eq!(field_value(&fields, "error_surface"), Some("user"));
        assert_eq!(field_value(&fields, "error_source"), None);
    }

    #[test]
    fn error_records_route_to_stderr_only_when_file_logging_is_disabled() {
        assert!(!should_emit_to_stderr(LogLevel::Error, true));
        assert!(should_emit_to_stderr(LogLevel::Error, false));
    }

    #[test]
    fn non_error_records_do_not_route_to_stderr_when_file_logging_is_enabled() {
        assert!(!should_emit_to_stderr(LogLevel::Warn, true));
        assert!(!should_emit_to_stderr(LogLevel::Info, true));
        assert!(!should_emit_to_stderr(LogLevel::Debug, true));
    }

    #[test]
    fn non_error_records_do_not_route_to_stderr_when_file_logging_is_disabled() {
        assert!(!should_emit_to_stderr(LogLevel::Warn, false));
        assert!(!should_emit_to_stderr(LogLevel::Info, false));
        assert!(!should_emit_to_stderr(LogLevel::Debug, false));
    }

    #[test]
    fn enabled_file_logging_writes_error_records_without_stderr_routing() {
        let log_dir =
            std::env::temp_dir().join(format!("sce-observability-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&log_dir);
        let logger = Logger {
            config: ObservabilityConfig {
                level: LogLevel::Error,
                format: LogFormat::Text,
                log_to_file: true,
            },
            log_dir: Some(log_dir.clone()),
            log_file_retention_limit: 10,
        };

        logger.error("sce.test.error", "test error", &[], None);

        let entries = fs::read_dir(&log_dir)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(entries.len(), 1);
        let contents = fs::read_to_string(entries[0].path()).unwrap();
        assert!(contents.contains("event_id=sce.test.error"));
        let _ = fs::remove_dir_all(log_dir);
    }

    #[test]
    fn disabled_file_logging_keeps_error_records_off_disk() {
        let log_dir = std::env::temp_dir().join(format!(
            "sce-observability-disabled-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&log_dir);
        let logger = Logger {
            config: ObservabilityConfig {
                level: LogLevel::Error,
                format: LogFormat::Text,
                log_to_file: false,
            },
            log_dir: Some(log_dir.clone()),
            log_file_retention_limit: 10,
        };

        logger.error("sce.test.error", "test error", &[], None);

        assert!(!log_dir.exists());
    }

    const CHILD_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);
    const CHILD_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(20);
    const ENV_CHILD_PATH: &str = "SCE_LOG_LOCK_CHILD_PATH";
    const SAME_STRIPE_SENTINEL: &str = "SAME_STRIPE_FALLBACK_OK";
    const POISON_SENTINEL: &str = "POISONED_STRIPE_OK";

    fn text_logger(log_dir: &Path) -> Logger {
        Logger {
            config: ObservabilityConfig {
                level: LogLevel::Error,
                format: LogFormat::Text,
                log_to_file: true,
            },
            log_dir: Some(log_dir.to_path_buf()),
            log_file_retention_limit: 10,
        }
    }

    fn read_all_log_lines(log_dir: &Path) -> Vec<String> {
        let mut lines = Vec::new();
        for entry in fs::read_dir(log_dir).unwrap() {
            let path = entry.unwrap().path();
            if path
                .extension()
                .is_some_and(|ext| ext == LOG_FILE_EXTENSION)
            {
                lines.extend(fs::read_to_string(path).unwrap().lines().map(String::from));
            }
        }
        lines
    }

    fn run_bounded_child(test_name: &str, child_path: &Path) -> std::process::Output {
        use std::io::Read;
        use std::process::{Command, Stdio};

        let sandbox = tempfile::tempdir().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture"])
            .current_dir(sandbox.path())
            .env(ENV_CHILD_PATH, child_path)
            .env("HOME", sandbox.path())
            .env("XDG_CONFIG_HOME", sandbox.path().join("config"))
            .env("XDG_STATE_HOME", sandbox.path().join("state"))
            .env("XDG_CACHE_HOME", sandbox.path().join("cache"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        let mut stdout_pipe = child.stdout.take().unwrap();
        let mut stderr_pipe = child.stderr.take().unwrap();
        let stdout_reader = std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = stdout_pipe.read_to_end(&mut buffer);
            buffer
        });
        let stderr_reader = std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = stderr_pipe.read_to_end(&mut buffer);
            buffer
        });

        let deadline = std::time::Instant::now() + CHILD_DEADLINE;
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break Some(status);
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                break None;
            }
            std::thread::sleep(CHILD_POLL_INTERVAL);
        };

        let stdout = stdout_reader.join().unwrap();
        let stderr = stderr_reader.join().unwrap();
        let Some(status) = status else {
            panic!(
                "child '{test_name}' exceeded {CHILD_DEADLINE:?} and was killed\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr)
            );
        };

        std::process::Output {
            status,
            stdout,
            stderr,
        }
    }

    fn assert_child_succeeded(output: &std::process::Output, sentinel: &str) {
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains(sentinel),
            "child failed: {:?}\nstdout: {stdout}\nstderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn blocked_primary_path(log_dir: &Path, stem: &str) -> PathBuf {
        let primary = log_dir.join(format!("{stem}.log"));
        fs::create_dir_all(&primary).unwrap();
        primary
    }

    #[test]
    fn concurrent_writers_to_one_path_produce_complete_unique_lines() {
        const THREADS: usize = 8;
        const LINES_PER_THREAD: usize = 50;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sce-concurrent.log");
        let padding = "x".repeat(512);

        std::thread::scope(|scope| {
            for thread in 0..THREADS {
                let path = &path;
                let padding = &padding;
                scope.spawn(move || {
                    for index in 0..LINES_PER_THREAD {
                        let line = format!("t{thread}-l{index}-{padding}-end");
                        append_log_line_with_cleanup(path, &line, |_| Ok(())).unwrap();
                    }
                });
            }
        });

        let contents = fs::read_to_string(&path).unwrap();
        let mut seen = std::collections::BTreeSet::new();
        for line in contents.lines() {
            assert!(line.ends_with("-end"), "interleaved line: {line}");
            assert!(seen.insert(line.to_string()), "duplicated line: {line}");
        }
        assert_eq!(seen.len(), THREADS * LINES_PER_THREAD);
    }

    #[test]
    fn independent_logger_instances_serialize_writes_to_one_path() {
        const INSTANCES: usize = 4;
        const RECORDS_PER_INSTANCE: usize = 25;

        let dir = tempfile::tempdir().unwrap();
        let loggers: Vec<Logger> = (0..INSTANCES).map(|_| text_logger(dir.path())).collect();

        std::thread::scope(|scope| {
            for (instance, logger) in loggers.iter().enumerate() {
                scope.spawn(move || {
                    for index in 0..RECORDS_PER_INSTANCE {
                        let event_id = format!("sce.test.i{instance}.r{index}");
                        logger.error(&event_id, "concurrent record", &[], Some("shared"));
                    }
                });
            }
        });

        let lines = read_all_log_lines(dir.path());
        assert_eq!(lines.len(), INSTANCES * RECORDS_PER_INSTANCE);
        for line in &lines {
            assert!(line.starts_with("timestamp="), "malformed line: {line}");
            assert!(
                line.ends_with("message=concurrent record"),
                "malformed line: {line}"
            );
        }
        let unique: std::collections::BTreeSet<&String> = lines.iter().collect();
        assert_eq!(unique.len(), lines.len());
    }

    #[test]
    fn stripe_selection_is_stable_bounded_and_independent_of_path_count() {
        assert_eq!(LOG_LOCKS.len(), LOG_LOCK_STRIPES);

        let mut used = std::collections::BTreeSet::new();
        for index in 0..5000 {
            let path = PathBuf::from(format!("/tmp/sce-distinct/sce-{index}.log"));
            let stripe = log_lock_stripe_index(&path);
            assert!(stripe < LOG_LOCK_STRIPES);
            assert_eq!(stripe, log_lock_stripe_index(&path));
            assert!(std::ptr::eq(
                log_lock_stripe(&path),
                LOG_LOCKS.get(stripe).unwrap()
            ));
            used.insert(stripe);
        }

        assert!(used.len() > 1);
        assert_eq!(LOG_LOCKS.len(), LOG_LOCK_STRIPES);
    }

    #[test]
    fn fallback_on_a_different_stripe_writes_the_v2_file() {
        let dir = tempfile::tempdir().unwrap();
        let primary = (0..1000)
            .map(|index| dir.path().join(format!("sce-diff-{index}.log")))
            .find(|candidate| {
                !std::ptr::eq(
                    log_lock_stripe(candidate),
                    log_lock_stripe(&v2_log_path(candidate)),
                )
            })
            .unwrap();
        fs::create_dir_all(&primary).unwrap();

        append_log_line_with_cleanup(&primary, "fallback-line", |_| Ok(())).unwrap();

        assert_eq!(
            fs::read_to_string(v2_log_path(&primary)).unwrap(),
            "fallback-line\n"
        );
        assert!(primary.is_dir());
    }

    #[test]
    fn fallback_failure_reports_combined_primary_and_v2_error() {
        let dir = tempfile::tempdir().unwrap();
        let primary = blocked_primary_path(dir.path(), "sce-both-fail");
        let fallback = v2_log_path(&primary);
        fs::create_dir_all(&fallback).unwrap();

        let error = append_log_line_with_cleanup(&primary, "line", |_| Ok(()))
            .unwrap_err()
            .to_string();

        assert!(
            error.starts_with(&format!(
                "primary log file persistence failed for '{}': ",
                primary.display()
            )),
            "{error}"
        );
        assert!(
            error.contains(&format!(
                "; v2 fallback log file persistence failed for '{}': ",
                fallback.display()
            )),
            "{error}"
        );
    }

    #[test]
    fn same_stripe_fallback_completes_in_bounded_child_process() {
        let dir = tempfile::tempdir().unwrap();
        let primary = (0..10_000)
            .map(|index| dir.path().join(format!("sce-collide-{index}.log")))
            .find(|candidate| {
                std::ptr::eq(
                    log_lock_stripe(candidate),
                    log_lock_stripe(&v2_log_path(candidate)),
                )
            })
            .expect("a primary path whose v2 path shares its stripe");
        fs::create_dir_all(&primary).unwrap();

        let output = run_bounded_child(
            "services::observability::tests::isolated_same_stripe_fallback",
            &primary,
        );

        assert_child_succeeded(&output, SAME_STRIPE_SENTINEL);
        assert_eq!(
            fs::read_to_string(v2_log_path(&primary)).unwrap(),
            "same-stripe-line\n"
        );
        assert!(primary.is_dir());
    }

    #[test]
    fn isolated_same_stripe_fallback() {
        let Some(primary) = std::env::var_os(ENV_CHILD_PATH).map(PathBuf::from) else {
            return;
        };
        assert!(std::ptr::eq(
            log_lock_stripe(&primary),
            log_lock_stripe(&v2_log_path(&primary))
        ));

        append_log_line_with_cleanup(&primary, "same-stripe-line", |_| Ok(())).unwrap();

        println!("{SAME_STRIPE_SENTINEL}");
    }

    #[test]
    fn poisoned_stripe_fails_lock_without_fallback_in_child_process() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sce-poison.log");

        let output = run_bounded_child(
            "services::observability::tests::isolated_poisoned_stripe",
            &path,
        );

        assert_child_succeeded(&output, POISON_SENTINEL);
        assert!(!v2_log_path(&path).exists());
    }

    #[test]
    fn isolated_poisoned_stripe() {
        let Some(path) = std::env::var_os(ENV_CHILD_PATH).map(PathBuf::from) else {
            return;
        };

        let poisoned = std::panic::catch_unwind(|| {
            let _ = append_log_line_with_cleanup(&path, "first", |_| panic!("poison the stripe"));
        });
        assert!(poisoned.is_err());

        let error = append_log_line_with_cleanup(&path, "second", |_| Ok(()))
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with(&format!("failed to lock log file '{}': ", path.display())),
            "{error}"
        );
        assert!(!v2_log_path(&path).exists());
        assert_eq!(fs::read_to_string(&path).unwrap(), "first\n");

        println!("{POISON_SENTINEL}");
    }

    #[test]
    fn retention_runs_once_for_concurrent_creation_of_one_path() {
        const THREADS: usize = 8;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sce-retention.log");
        let cleanups = std::sync::atomic::AtomicUsize::new(0);

        std::thread::scope(|scope| {
            for thread in 0..THREADS {
                let path = &path;
                let cleanups = &cleanups;
                scope.spawn(move || {
                    append_log_line_with_cleanup(path, &format!("line-{thread}"), |log_dir| {
                        assert_eq!(log_dir, path.parent().unwrap());
                        cleanups.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok(())
                    })
                    .unwrap();
                });
            }
        });

        assert_eq!(cleanups.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), THREADS);
    }

    #[cfg(unix)]
    #[test]
    fn created_log_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sce-perms.log");

        append_log_line_with_cleanup(&path, "line", |_| Ok(())).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn secrets_are_redacted_before_they_reach_the_log_file() {
        let dir = tempfile::tempdir().unwrap();
        let logger = text_logger(dir.path());

        logger.error(
            "sce.test.redaction",
            "request failed",
            &[("password", "hunter2-super-secret")],
            None,
        );

        let lines = read_all_log_lines(dir.path());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("[REDACTED]"));
        assert!(!lines[0].contains("hunter2-super-secret"));
    }

    #[test]
    fn observability_fields_for_internal_error_carry_surface_and_full_source_chain() {
        let source = anyhow::anyhow!("root cause").context("failed to do the thing");
        let error = CliError::internal(FailureClass::Dependency, source);

        let fields = cli_error_fields(&error);

        assert_eq!(
            field_value(&fields, "error_code"),
            Some("SCE-ERR-DEPENDENCY")
        );
        assert_eq!(field_value(&fields, "error_class"), Some("dependency"));
        assert_eq!(field_value(&fields, "error_surface"), Some("internal"));
        assert_eq!(field_value(&fields, "user_error"), None);
        assert_eq!(
            field_value(&fields, "error_source"),
            Some("failed to do the thing: root cause")
        );
    }
}
