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
    let rendered = render_command_result(
        outcome.result,
        outcome.logger.as_ref(),
        outcome.startup_diagnostic.as_deref(),
        stdout,
        stderr,
    );
    exit_code_for(&rendered)
}

pub(crate) fn render_command_result<L, StdoutW, StderrW>(
    result: Result<String, CliError>,
    logger: Option<&L>,
    startup_diagnostic: Option<&str>,
    stdout: &mut StdoutW,
    stderr: &mut StderrW,
) -> Result<(), CliError>
where
    L: LoggerTrait,
    StdoutW: Write,
    StderrW: Write,
{
    let error = match result {
        Ok(payload) => {
            if let Some(diagnostic) = startup_diagnostic {
                write_startup_diagnostic(stderr, diagnostic);
            }
            match write_stdout_payload(stdout, &payload) {
                Ok(()) => return Ok(()),
                Err(error) => error,
            }
        }
        Err(error) => error,
    };
    if let Some(log) = logger {
        log.log_cli_error(&error, None);
    }
    write_error_diagnostic(stderr, &error);
    Err(error)
}

pub(crate) fn exit_code_for<T>(result: &Result<T, CliError>) -> ExitCode {
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => ExitCode::from(error.class().exit_code()),
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

pub(crate) async fn execute_command_phase<C, W>(
    command: &RuntimeCommand,
    context: &C,
    stderr: &mut W,
) -> Result<String, CliError>
where
    C: HasLogger + ContextWithRepoRoot + crate::app::HasGit + crate::app::HasFs,
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
    let dispatch_result = command.execute_with_stderr(context, stderr).await;
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

fn write_stdout_payload<W: Write>(writer: &mut W, payload: &str) -> Result<(), CliError> {
    if payload.is_empty() {
        return Ok(());
    }
    writeln!(writer, "{payload}")
        .and_then(|()| writer.flush())
        .map_err(|error| {
            CliError::runtime(anyhow::Error::msg(format!(
                "Failed to write command output to stdout: {error}"
            )))
        })
}

fn write_error_diagnostic<W: Write>(writer: &mut W, error: &CliError) {
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
        } => {
            let message = services::security::redact_sensitive_text(user_error.message());
            let _ = writeln!(writer, "{message}");
            return;
        }
    };
    let styled_message = services::style::error_text_with_color_policy(
        &services::security::redact_sensitive_text(&rendered),
        color_enabled,
    );
    let _ = writeln!(
        writer,
        "{} [{}]: {}",
        services::style::heading_with_color_policy("Error", color_enabled),
        services::style::error_code_with_color_policy(error.code(), color_enabled),
        styled_message
    );
}

fn write_startup_diagnostic<W: Write>(writer: &mut W, diagnostic: &str) {
    let _ = writeln!(writer, "{}", services::style::error_code(diagnostic));
}
