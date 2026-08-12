use std::io::Write;
use std::process::ExitCode;

use crate::app::{ContextWithRepoRoot, HasLogger};
use crate::services;
use services::command_registry::RuntimeCommand;
use services::error::ClassifiedError;
use services::observability::traits::Logger as LoggerTrait;

const INVALID_CONFIG_WARNING_EVENT_ID: &str = "sce.config.invalid_config";

pub(crate) struct RunOutcome<L>
where
    L: LoggerTrait,
{
    pub(crate) result: Result<String, ClassifiedError>,
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

pub(crate) fn execute_command_phase<C>(
    command: &RuntimeCommand,
    context: &C,
) -> Result<String, ClassifiedError>
where
    C: HasLogger + ContextWithRepoRoot,
{
    let command_name = command.name();
    let logger = context.logger();
    logger.debug(
        "sce.command.dispatch_start",
        "Dispatching command",
        &[("command", command_name.as_ref())],
        None,
    );
    let dispatch_result = command.execute(context);
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

fn exit_with_error<L, W>(stderr: &mut W, logger: Option<&L>, error: &ClassifiedError) -> ExitCode
where
    L: LoggerTrait,
    W: Write,
{
    if let Some(log) = logger {
        log.log_classified_error(error, None);
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
    let rendered = if let Some(hint) = error.hint() {
        format!("{} Try: {}", error.message(), hint)
    } else if error.message().contains("Try:") {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(error: &ClassifiedError) -> String {
        let mut buffer = Vec::new();
        write_error_diagnostic(&mut buffer, error);
        String::from_utf8(buffer).expect("diagnostic output should be valid utf-8")
    }

    #[test]
    fn explicit_hint_is_rendered() {
        let error = ClassifiedError::parse("x").with_hint("y");

        assert_eq!(rendered(&error), "Error [SCE-ERR-PARSE]: x Try: y\n");
    }

    #[test]
    fn absent_hint_falls_back_to_class_default_guidance() {
        let error = ClassifiedError::parse("x");

        assert_eq!(
            rendered(&error),
            format!(
                "Error [SCE-ERR-PARSE]: x Try: {}\n",
                error.class().default_try_guidance()
            )
        );
    }

    #[test]
    fn message_already_containing_try_is_left_unchanged_without_a_hint() {
        let error = ClassifiedError::parse("x Try: existing guidance.");

        assert_eq!(
            rendered(&error),
            "Error [SCE-ERR-PARSE]: x Try: existing guidance.\n"
        );
    }

    #[test]
    fn explicit_hint_is_not_doubled_with_class_default_guidance() {
        let error = ClassifiedError::parse("x").with_hint("y");

        let output = rendered(&error);
        assert_eq!(output.matches("Try:").count(), 1);
    }

    #[test]
    fn hinted_message_is_still_redacted() {
        let error = ClassifiedError::parse("token: Bearer abcdef123456")
            .with_hint("retry with a new token");

        let output = rendered(&error);
        assert!(!output.contains("abcdef123456"));
    }
}
