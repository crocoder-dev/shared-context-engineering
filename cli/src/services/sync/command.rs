use std::io::Write;

use crate::app::ContextWithRepoRoot;
use crate::services::error::{AutomaticSyncFailureKind, CliError, UserError};
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

fn current_repo_root<C>(context: &C) -> Result<std::path::PathBuf, CliError>
where
    C: ContextWithRepoRoot,
{
    if let Some(path) = context.repo_root() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir().map_err(|err| {
            CliError::runtime(anyhow::Error::msg(format!(
                "failed to determine current directory: {err}"
            )))
        })
    }
}

#[allow(clippy::needless_pass_by_value)]
fn classify_sync_error(
    err: TraceSyncError,
    invocation: crate::services::sync::SyncInvocation,
) -> CliError {
    if invocation == crate::services::sync::SyncInvocation::Automatic {
        let failure_kind = if err.is_authentication_failure() {
            AutomaticSyncFailureKind::Authentication
        } else {
            match &err {
                TraceSyncError::ControlPlane(_) => AutomaticSyncFailureKind::ControlPlane,
                TraceSyncError::Stream { .. } => AutomaticSyncFailureKind::Stream,
                TraceSyncError::Runtime(_) => AutomaticSyncFailureKind::Runtime,
            }
        };
        let reason = err.to_string();
        return CliError::user_with_source(
            UserError::AutomaticSyncFailed {
                failure_kind,
                reason,
            },
            err,
        );
    }

    if err.is_authentication_failure() {
        CliError::user_with_source(UserError::NotAuthenticated, err)
    } else {
        CliError::runtime(err)
    }
}

impl SyncCommand {
    #[allow(dead_code)]
    pub fn execute<C>(&self, context: &C) -> Result<String, CliError>
    where
        C: ContextWithRepoRoot,
    {
        let mut stderr = std::io::sink();
        self.execute_with_stderr(context, &mut stderr)
    }

    pub fn execute_with_stderr<C, W>(&self, context: &C, stderr: &mut W) -> Result<String, CliError>
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
    ) -> Result<String, CliError>
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
        .map_err(|error| classify_sync_error(error, self.request.invocation))?;

        render_sync::render(&report, self.request.format)
            .map_err(|error| CliError::runtime(anyhow::Error::msg(format!("{error:#}"))))
    }
}

#[cfg(test)]
mod tests {
    use super::classify_sync_error;
    use crate::services::agent_trace_sync::control_plane::ControlPlaneError;
    use crate::services::agent_trace_sync::StreamSyncError;
    use crate::services::error::{AutomaticSyncFailureKind, CliError};
    use crate::services::sync::sync::TraceSyncError;
    use crate::services::sync::SyncInvocation;

    fn assert_user_not_authenticated(err: TraceSyncError) {
        match classify_sync_error(err, SyncInvocation::Manual) {
            CliError::User { error, source } => {
                assert_eq!(error.key(), "auth.not_authenticated");
                assert!(source.is_some());
            }
            other @ CliError::Internal { .. } => panic!("expected CliError::User, got {other:?}"),
        }
    }

    fn assert_internal(err: TraceSyncError) {
        match classify_sync_error(err, SyncInvocation::Manual) {
            CliError::Internal { .. } => {}
            other @ CliError::User { .. } => panic!("expected CliError::Internal, got {other:?}"),
        }
    }

    #[test]
    fn initial_state_missing_credentials_classifies_as_not_authenticated() {
        assert_user_not_authenticated(TraceSyncError::ControlPlane(
            ControlPlaneError::MissingCredentials,
        ));
    }

    #[test]
    fn initial_state_authentication_failed_classifies_as_not_authenticated() {
        assert_user_not_authenticated(TraceSyncError::ControlPlane(
            ControlPlaneError::AuthenticationFailed("token expired".to_string()),
        ));
    }

    #[test]
    fn stream_batch_authentication_failed_classifies_as_not_authenticated() {
        assert_user_not_authenticated(TraceSyncError::Stream {
            stream: "prompts",
            source: StreamSyncError::Terminal(ControlPlaneError::AuthenticationFailed(
                "token expired".to_string(),
            )),
        });
    }

    #[test]
    fn stream_refresh_authentication_failed_classifies_as_not_authenticated() {
        assert_user_not_authenticated(TraceSyncError::Stream {
            stream: "prompts",
            source: StreamSyncError::Refresh(ControlPlaneError::MissingCredentials),
        });
    }

    #[test]
    fn other_control_plane_errors_classify_as_internal() {
        for error in [
            ControlPlaneError::Forbidden("nope".to_string()),
            ControlPlaneError::BadRequest("bad".to_string()),
            ControlPlaneError::Transport("down".to_string()),
            ControlPlaneError::ServerError("500".to_string()),
            ControlPlaneError::InvalidResponse("garbage".to_string()),
            ControlPlaneError::Storage("disk".to_string()),
            ControlPlaneError::Protocol {
                status: reqwest::StatusCode::NOT_FOUND,
                message: "route removed".to_string(),
            },
        ] {
            assert_internal(TraceSyncError::ControlPlane(error));
        }
    }

    #[test]
    fn runtime_failure_classifies_as_internal() {
        assert_internal(TraceSyncError::Runtime("local failure".to_string()));
    }
}
