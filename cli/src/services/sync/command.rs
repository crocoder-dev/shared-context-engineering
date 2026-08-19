use std::io::Write;

use crate::app::ContextWithRepoRoot;
use crate::services::agent_trace_sync::control_plane::ControlPlaneError;
use crate::services::error::{ClassifiedError, UserFacingPresentation};
use crate::services::sync::progress::{
    IndicatifProgressReporter, NoopProgressReporter, ProgressReporter,
};
use crate::services::sync::render_sync;
use crate::services::sync::sync::{
    run_current_sync_with_progress_and_clock, SyncProgressClock, SystemSyncProgressClock,
    TraceSyncError,
};
use crate::services::sync::SyncRequest;

pub struct SyncCommand {
    pub request: SyncRequest,
}

fn current_repo_root<C>(context: &C) -> Result<std::path::PathBuf, ClassifiedError>
where
    C: ContextWithRepoRoot,
{
    if let Some(path) = context.repo_root() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir().map_err(|err| {
            ClassifiedError::runtime(format!("failed to determine current directory: {err}"))
        })
    }
}

#[allow(clippy::needless_pass_by_value)]
fn classify_sync_error(err: TraceSyncError) -> ClassifiedError {
    let is_unauthenticated = matches!(
        &err,
        TraceSyncError::ControlPlane(
            ControlPlaneError::MissingCredentials | ControlPlaneError::AuthenticationFailed(_)
        )
    );
    let classified = ClassifiedError::runtime(format!("{err}"));

    if is_unauthenticated {
        classified.with_user_facing_presentation(UserFacingPresentation::new(format!(
            "You are not logged in. Please log in using the {} command.",
            crate::services::style::success("sce auth login")
        )))
    } else {
        classified
    }
}

impl SyncCommand {
    #[allow(dead_code)]
    pub fn execute<C>(&self, context: &C) -> Result<String, ClassifiedError>
    where
        C: ContextWithRepoRoot,
    {
        let mut stderr = std::io::sink();
        self.execute_with_stderr(context, &mut stderr)
    }

    pub fn execute_with_stderr<C, W>(
        &self,
        context: &C,
        stderr: &mut W,
    ) -> Result<String, ClassifiedError>
    where
        C: ContextWithRepoRoot,
        W: Write,
    {
        let clock = SystemSyncProgressClock;
        self.execute_with_stderr_and_clock(context, stderr, &clock)
    }

    fn execute_with_stderr_and_clock<C, W, Clock>(
        &self,
        context: &C,
        stderr: &mut W,
        clock: &Clock,
    ) -> Result<String, ClassifiedError>
    where
        C: ContextWithRepoRoot,
        W: Write,
        Clock: SyncProgressClock,
    {
        let repo_root = current_repo_root(context)?;

        let report = match self.request.format {
            crate::services::output_format::OutputFormat::Text => {
                let mut progress = IndicatifProgressReporter::new(stderr);
                let result =
                    run_current_sync_with_progress_and_clock(&repo_root, &mut progress, clock);
                if result.is_ok() {
                    progress.finish_successfully();
                }
                result
            }
            crate::services::output_format::OutputFormat::Json => {
                let mut progress = NoopProgressReporter;
                run_current_sync_with_progress_and_clock(&repo_root, &mut progress, clock)
            }
        }
        .map_err(classify_sync_error)?;

        render_sync::render(&report, self.request.format)
            .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))
    }
}
