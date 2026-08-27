use std::io::Write;
use std::process::ExitCode;

use crate::app::{ContextWithRepoRoot, HasLogger};
use crate::services;
use services::command_registry::RuntimeCommand;
use services::error::CliError;
use services::observability::traits::Logger as LoggerTrait;

const INVALID_CONFIG_WARNING_EVENT_ID: &str = "sce.config.invalid_config";

pub(crate) struct RunOutcome<L>
where
    L: LoggerTrait,
{
    pub(crate) result: Result<String, CliError>,
    pub(crate) logger: Option<L>,
    pub(crate) startup_diagnostic: Option<String>,
}

pub(crate) fn render_run_outcome<L, StdoutW, StderrW>(
    outcome: RunOutcome<L>,
    stdout: &mut StdoutW,
    stderr: &mut StderrW,
) -> ExitCode
where
    L: LoggerTrait,
    StdoutW: Write,
    StderrW: Write,
{
    match outcome.result {
        Ok(payload) => {
            if let Some(diagnostic) = outcome.startup_diagnostic {
                write_startup_diagnostic(stderr, &diagnostic);
            }
            let logger = outcome.logger.as_ref();
            write_stdout_payload(stdout, &payload).map_or_else(
                |error| exit_with_error(stderr, logger, &error),
                |()| ExitCode::SUCCESS,
            )
        }
        Err(error) => {
            let logger = outcome.logger.as_ref();
            exit_with_error(stderr, logger, &error)
        }
    }
}

pub(crate) fn classify_observability_configuration_error(error: &anyhow::Error) -> CliError {
    CliError::validation(format!("Invalid observability configuration: {error}"))
}

pub(crate) fn invalid_discovered_config_guidance(
    observability_config: &services::config::ResolvedObservabilityRuntimeConfig,
) -> Option<String> {
    if observability_config.validation_errors.is_empty() {
        return None;
    }

    let has_invalid_local_config =
        observability_config
            .loaded_config_paths
            .iter()
            .any(|loaded_path| {
                loaded_path.source == services::config::ConfigPathSource::DefaultDiscoveredLocal
                    && observability_config
                        .validation_errors
                        .iter()
                        .any(|error| error.contains(loaded_path.path.to_string_lossy().as_ref()))
            });

    Some(if has_invalid_local_config {
        "Local `.sce` config is invalid. Fix `.sce` and run `sce config validate`.".to_string()
    } else {
        "A discovered config file is invalid. Fix it and run `sce config validate`.".to_string()
    })
}

pub(crate) fn log_startup_configuration(
    logger: &services::observability::Logger,
    observability_config: &services::config::ResolvedObservabilityRuntimeConfig,
) {
    for loaded_path in &observability_config.loaded_config_paths {
        logger.debug(
            "sce.config.file_discovered",
            "Config file discovered",
            &[
                ("path", loaded_path.path.to_string_lossy().as_ref()),
                ("source", loaded_path.source.as_str()),
            ],
            None,
        );
    }
    for validation_error in &observability_config.validation_errors {
        logger.warn(
            INVALID_CONFIG_WARNING_EVENT_ID,
            "Invalid discovered config skipped; using degraded defaults",
            &[("error", validation_error.as_str())],
            None,
        );
    }
}

pub(crate) fn execute_command_phase<C, W>(
    command: &RuntimeCommand,
    context: &C,
    stderr: &mut W,
) -> Result<String, CliError>
where
    C: HasLogger + ContextWithRepoRoot,
    W: Write,
{
    let command_name = command.name();
    let logger = context.logger();
    logger.debug(
        "sce.command.dispatch_start",
        "Dispatching command",
        &[("command", command_name.as_ref())],
        None,
    );
    let dispatch_result = command.execute_with_stderr(context, stderr);
    if dispatch_result.is_ok() {
        logger.debug(
            "sce.command.dispatch_end",
            "Command dispatch completed",
            &[("command", command_name.as_ref())],
            None,
        );
    }
    dispatch_result.inspect(|_payload| {
        logger.info(
            "sce.command.completed",
            "Command completed",
            &[("command", command_name.as_ref())],
            None,
        );
    })
}

fn exit_with_error<L, W>(stderr: &mut W, logger: Option<&L>, error: &CliError) -> ExitCode
where
    L: LoggerTrait,
    W: Write,
{
    if let Some(log) = logger {
        log.log_cli_error(error, None);
    }
    write_error_diagnostic(stderr, error);
    ExitCode::from(error.class().exit_code())
}

fn write_stdout_payload<W: Write>(writer: &mut W, payload: &str) -> Result<(), CliError> {
    if payload.is_empty() {
        return Ok(());
    }
    writeln!(writer, "{payload}").map_err(|error| {
        CliError::runtime(anyhow::Error::msg(format!(
            "Failed to write command output to stdout: {error}"
        )))
    })
}

pub(crate) fn write_error_diagnostic<W: Write>(writer: &mut W, error: &CliError) {
    write_error_diagnostic_with_color_policy(
        writer,
        error,
        services::style::supports_color_stderr(),
    );
}

fn write_error_diagnostic_with_color_policy<W: Write>(
    writer: &mut W,
    error: &CliError,
    color_enabled: bool,
) {
    let rendered = match error {
        CliError::Internal { source, .. } => {
            let message = format!("{source:#}");
            if message.contains("Try:") {
                message
            } else {
                format!("{message} Try: {}", error.class().default_try_guidance())
            }
        }
        CliError::User {
            error: user_error, ..
        } => user_error.message(),
    };
    let styled_message = services::style::error_text_with_color_policy(
        &services::security::redact_sensitive_text(&rendered),
        color_enabled,
    );
    writeln!(
        writer,
        "{} [{}]: {}",
        services::style::heading_with_color_policy("Error", color_enabled),
        services::style::error_code_with_color_policy(error.code(), color_enabled),
        styled_message
    )
    .expect("writing error diagnostic to writer should not fail");
}

fn write_startup_diagnostic<W: Write>(writer: &mut W, diagnostic: &str) {
    writeln!(writer, "{}", services::style::error_code(diagnostic))
        .expect("writing startup diagnostic to writer should not fail");
}

#[cfg(test)]
mod tests {
    use super::*;
    use services::error::UserError;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct RecordingLogger {
        log_cli_error_calls: Arc<Mutex<Vec<(&'static str, bool)>>>,
    }

    impl LoggerTrait for RecordingLogger {
        fn info(&self, _: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {}
        fn debug(&self, _: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {}
        fn warn(&self, _: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {}
        fn error(&self, _: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {}

        fn log_cli_error(&self, error: &CliError, _session_id: Option<&str>) {
            let has_source = match error {
                CliError::User { source, .. } => source.is_some(),
                CliError::Internal { .. } => true,
            };
            self.log_cli_error_calls
                .lock()
                .expect("recording logger mutex must not be poisoned")
                .push((error.code(), has_source));
        }
    }

    fn diagnostic_lines(stderr: &str) -> Vec<&str> {
        stderr
            .lines()
            .filter(|line| line.contains("Error") && line.contains("SCE-ERR-"))
            .collect()
    }

    #[test]
    fn user_error_routes_to_friendly_diagnostic_with_empty_stdout_and_exit_four() {
        let error = CliError::user_with_source(
            UserError::NotAuthenticated,
            anyhow::anyhow!("missing credentials"),
        );
        let logger = RecordingLogger::default();
        let outcome = RunOutcome {
            result: Err(error),
            logger: Some(logger),
            startup_diagnostic: None,
        };

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = render_run_outcome(outcome, &mut stdout, &mut stderr);

        assert!(stdout.is_empty(), "stdout must stay empty on failure");
        assert_eq!(exit_code, ExitCode::from(4));

        let stderr_text = String::from_utf8(stderr).expect("stderr is valid utf8");
        assert_eq!(
            diagnostic_lines(&stderr_text).len(),
            1,
            "exactly one terminal diagnostic must be written"
        );
        assert!(stderr_text.contains("You are not logged in"));
        assert!(!stderr_text.contains("missing credentials"));
        assert!(!stderr_text.to_lowercase().contains("control-plane"));
    }

    #[test]
    fn user_error_preserves_technical_source_for_observability() {
        let error = CliError::user_with_source(
            UserError::NotAuthenticated,
            anyhow::anyhow!("missing credentials"),
        );
        let logger = RecordingLogger::default();
        let calls = logger.log_cli_error_calls.clone();
        let outcome = RunOutcome {
            result: Err(error),
            logger: Some(logger),
            startup_diagnostic: None,
        };

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        render_run_outcome(outcome, &mut stdout, &mut stderr);

        let recorded = calls.lock().expect("mutex must not be poisoned");
        assert_eq!(
            recorded.as_slice(),
            [("SCE-ERR-RUNTIME", true)],
            "observability must log the CliError with its technical source preserved"
        );
    }

    #[test]
    fn internal_error_still_renders_full_chain_and_exit_four() {
        let source = anyhow::anyhow!("root cause").context("failed to sync");
        let error = CliError::runtime(source);
        let outcome: RunOutcome<RecordingLogger> = RunOutcome {
            result: Err(error),
            logger: None,
            startup_diagnostic: None,
        };

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = render_run_outcome(outcome, &mut stdout, &mut stderr);

        assert!(stdout.is_empty());
        assert_eq!(exit_code, ExitCode::from(4));
        let stderr_text = String::from_utf8(stderr).expect("stderr is valid utf8");
        assert_eq!(diagnostic_lines(&stderr_text).len(), 1);
        assert!(stderr_text.contains("failed to sync"));
        assert!(stderr_text.contains("root cause"));
    }

    #[test]
    fn redaction_still_applies_to_the_rendered_message() {
        let mut stderr = Vec::new();
        let error = CliError::user(UserError::NotAuthenticated);
        write_error_diagnostic_with_color_policy(&mut stderr, &error, false);

        let rendered = String::from_utf8(stderr).expect("stderr is valid utf8");
        let redacted_message =
            services::security::redact_sensitive_text(&UserError::NotAuthenticated.message());
        assert!(rendered.contains(&redacted_message));
    }

    #[test]
    fn user_error_diagnostic_is_styled_only_when_color_is_enabled() {
        let error = CliError::user(UserError::NotAuthenticated);

        let mut colored = Vec::new();
        write_error_diagnostic_with_color_policy(&mut colored, &error, true);
        let colored_text = String::from_utf8(colored).expect("stderr is valid utf8");

        let mut plain = Vec::new();
        write_error_diagnostic_with_color_policy(&mut plain, &error, false);
        let plain_text = String::from_utf8(plain).expect("stderr is valid utf8");

        // TTY-following (color_enabled: true) and redirected/NO_COLOR
        // (color_enabled: false) diverge: only the enabled case carries ANSI.
        assert_ne!(colored_text, plain_text);
        assert!(!plain_text.contains('\u{1b}'));
        assert!(colored_text.contains('\u{1b}'));
        assert!(plain_text.contains("You are not logged in"));
        assert!(colored_text.contains("You are not logged in"));
    }
}
