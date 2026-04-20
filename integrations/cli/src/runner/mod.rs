use std::path::PathBuf;

use crate::cli::Args;
use crate::error::HarnessError;

mod assertions;
mod catalog;
mod command_execution;
mod selection;
mod validators;

use catalog::CommandCase;
use selection::select_suites;

const SCE_BINARY_ENV: &str = "SCE_CLI_INTEGRATION_SCE_BIN";

pub(crate) struct Runner;

impl Runner {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn run(self, args: Args) -> Result<(), HarnessError> {
        let sce_binary = resolve_sce_binary()?;

        for suite in select_suites(args.command.as_deref())? {
            for case in suite.cases {
                run_case(&sce_binary, *case)?;
            }
        }

        Ok(())
    }
}

fn run_case(sce_binary: &PathBuf, case: CommandCase) -> Result<(), HarnessError> {
    let output = command_execution::run_command(sce_binary, case.argv)?;
    let status = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let command = command_execution::render_command(sce_binary, case.argv);

    assertions::assert_status(case, status, &stdout, &stderr, &command)?;
    assertions::assert_stdout_output(
        case,
        &stdout,
        &stderr,
        status,
        &command,
        case.expectation.stdout,
    )?;

    Ok(())
}

pub(super) fn validate_version_text_output(stream: &str) -> Result<(), String> {
    validators::validate_version_text_output(stream)
}

pub(super) fn validate_version_json_output(stream: &str) -> Result<(), String> {
    validators::validate_version_json_output(stream)
}

pub(super) fn validate_completion_bash_output(stream: &str) -> Result<(), String> {
    validators::validate_completion_bash_output(stream)
}

pub(super) fn validate_completion_zsh_output(stream: &str) -> Result<(), String> {
    validators::validate_completion_zsh_output(stream)
}

pub(super) fn validate_completion_fish_output(stream: &str) -> Result<(), String> {
    validators::validate_completion_fish_output(stream)
}

pub(super) fn validate_config_show_text_output(stream: &str) -> Result<(), String> {
    validators::validate_config_show_text_output(stream)
}

pub(super) fn validate_config_show_json_output(stream: &str) -> Result<(), String> {
    validators::validate_config_show_json_output(stream)
}

pub(super) fn validate_config_validate_text_output(stream: &str) -> Result<(), String> {
    validators::validate_config_validate_text_output(stream)
}

pub(super) fn validate_config_validate_json_output(stream: &str) -> Result<(), String> {
    validators::validate_config_validate_json_output(stream)
}

pub(super) fn validate_doctor_text_output(stream: &str) -> Result<(), String> {
    validators::validate_doctor_text_output(stream)
}

pub(super) fn validate_doctor_json_output(stream: &str) -> Result<(), String> {
    validators::validate_doctor_json_output(stream)
}

fn resolve_sce_binary() -> Result<PathBuf, HarnessError> {
    let binary = std::env::var_os(SCE_BINARY_ENV).ok_or(HarnessError::MissingEnv {
        env: SCE_BINARY_ENV,
    })?;
    Ok(PathBuf::from(binary))
}
