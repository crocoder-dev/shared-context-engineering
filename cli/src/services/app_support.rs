use std::io::Write;
use std::process::ExitCode;
use std::sync::Arc;

use crate::app::AppContext;
use crate::services;
use services::command_registry::RuntimeCommand;
use services::error::ClassifiedError;
use services::observability::traits::Logger as LoggerTrait;

const INVALID_CONFIG_WARNING_EVENT_ID: &str = "sce.config.invalid_config";

pub(crate) struct RunOutcome {
    pub(crate) result: Result<String, ClassifiedError>,
    pub(crate) logger: Option<Arc<dyn LoggerTrait>>,
    pub(crate) startup_diagnostic: Option<String>,
}

pub(crate) fn render_run_outcome<StdoutW, StderrW>(
    outcome: RunOutcome,
    stdout: &mut StdoutW,
    stderr: &mut StderrW,
) -> ExitCode
where
    StdoutW: Write,
    StderrW: Write,
{
    match outcome.result {
        Ok(payload) => {
            if let Some(diagnostic) = outcome.startup_diagnostic {
                write_startup_diagnostic(stderr, &diagnostic);
            }
            write_stdout_payload(stdout, &payload).map_or_else(
                |error| exit_with_error(stderr, outcome.logger.as_deref(), &error),
                |()| ExitCode::SUCCESS,
            )
        }
        Err(error) => exit_with_error(stderr, outcome.logger.as_deref(), &error),
    }
}

pub(crate) fn classify_observability_configuration_error(error: &anyhow::Error) -> ClassifiedError {
    ClassifiedError::validation(format!("Invalid observability configuration: {error}"))
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
        );
    }
    for validation_error in &observability_config.validation_errors {
        logger.warn(
            INVALID_CONFIG_WARNING_EVENT_ID,
            "Invalid discovered config skipped; using degraded defaults",
            &[("error", validation_error.as_str())],
        );
    }
}

pub(crate) fn execute_command_phase(
    command: &dyn RuntimeCommand,
    context: &AppContext,
) -> Result<String, ClassifiedError> {
    let command_name = command.name();
    let logger = context.logger();
    logger.debug(
        "sce.command.dispatch_start",
        "Dispatching command",
        &[("command", command_name.as_ref())],
    );
    let dispatch_result = command.execute(context);
    if dispatch_result.is_ok() {
        logger.debug(
            "sce.command.dispatch_end",
            "Command dispatch completed",
            &[("command", command_name.as_ref())],
        );
    }
    dispatch_result.inspect(|_payload| {
        logger.info(
            "sce.command.completed",
            "Command completed",
            &[("command", command_name.as_ref())],
        );
    })
}

fn exit_with_error<W>(
    stderr: &mut W,
    logger: Option<&dyn LoggerTrait>,
    error: &ClassifiedError,
) -> ExitCode
where
    W: Write,
{
    if let Some(log) = logger {
        log.log_classified_error(error);
    }
    write_error_diagnostic(stderr, error);
    ExitCode::from(error.class().exit_code())
}

fn write_stdout_payload<W: Write>(writer: &mut W, payload: &str) -> Result<(), ClassifiedError> {
    if payload.is_empty() {
        return Ok(());
    }
    writeln!(writer, "{payload}").map_err(|error| {
        ClassifiedError::runtime(format!("Failed to write command output to stdout: {error}"))
    })
}

fn write_error_diagnostic<W: Write>(writer: &mut W, error: &ClassifiedError) {
    let rendered = if error.message().contains("Try:") {
        error.message().to_string()
    } else {
        format!(
            "{} Try: {}",
            error.message(),
            error.class().default_try_guidance()
        )
    };
    let styled_message =
        services::style::error_text(&services::security::redact_sensitive_text(&rendered));
    writeln!(
        writer,
        "{} [{}]: {}",
        services::style::heading("Error"),
        services::style::error_code(error.code()),
        styled_message
    )
    .expect("writing error diagnostic to writer should not fail");
}

fn write_startup_diagnostic<W: Write>(writer: &mut W, diagnostic: &str) {
    writeln!(writer, "{}", services::style::error_code(diagnostic))
        .expect("writing startup diagnostic to writer should not fail");
}
