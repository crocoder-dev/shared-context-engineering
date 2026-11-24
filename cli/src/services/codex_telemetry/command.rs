use std::io::Write;

use anyhow::anyhow;

use super::{
    plan_launch_with, read_config_file, repository_root_for, LaunchContext, LaunchPlan,
    CODEX_PROGRAM,
};
use crate::services::auth_command::{repository_telemetry_authorized, telemetry_enabled_for};
use crate::services::error::CliError;

pub struct CodexCommand {
    pub args: Vec<String>,
}

impl CodexCommand {
    pub async fn execute_with_stderr<C, W>(
        &self,
        _context: &C,
        stderr: &mut W,
    ) -> Result<String, CliError>
    where
        W: Write,
    {
        let context = LaunchContext::from_process().map_err(|error| {
            CliError::runtime(anyhow!(
                "Failed to determine the Codex launch directory: {error}"
            ))
        })?;
        let plan = plan_launch_with(
            self.args.clone(),
            &context,
            read_config_file,
            |key| std::env::var(key).ok(),
            repository_root_for,
            telemetry_enabled_for,
            async |repository_root| repository_telemetry_authorized(&repository_root).await,
        )
        .await;
        if let Some(notice) = &plan.notice {
            let _ = writeln!(stderr, "{notice}");
        }
        let _ = stderr.flush();
        launch(plan)
    }
}

#[cfg(unix)]
fn launch(plan: LaunchPlan) -> Result<String, CliError> {
    use std::os::unix::process::CommandExt;

    let error = std::process::Command::new(CODEX_PROGRAM)
        .args(plan.argv)
        .exec();
    Err(launch_failure(&error))
}

#[cfg(not(unix))]
fn launch(plan: LaunchPlan) -> Result<String, CliError> {
    let status = std::process::Command::new(CODEX_PROGRAM)
        .args(plan.argv)
        .status()
        .map_err(|error| launch_failure(&error))?;
    std::process::exit(status.code().unwrap_or(1));
}

fn launch_failure(error: &std::io::Error) -> CliError {
    CliError::runtime(anyhow!("Failed to launch `{CODEX_PROGRAM}`: {error}"))
}
