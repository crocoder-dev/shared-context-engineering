use crate::app::ContextWithRepoRoot;
use crate::services::error::ClassifiedError;
use crate::services::trace::discovery::discover_agent_trace_dbs;
use crate::services::trace::render_list;
use crate::services::trace::render_status;
use crate::services::trace::render_status_all;
use crate::services::trace::status::{resolve_current_status, StatusErrorOrRuntime};
use crate::services::trace::status_all::aggregate_current_status_all;
use crate::services::trace::{TraceRequest, TraceSubcommandRequest};

pub struct TraceCommand {
    pub request: TraceRequest,
}

impl TraceCommand {
    pub fn execute<C>(&self, context: &C) -> Result<String, ClassifiedError>
    where
        C: ContextWithRepoRoot,
    {
        match &self.request.subcommand {
            TraceSubcommandRequest::DbList { format } => {
                let databases = discover_agent_trace_dbs()
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))?;
                render_list::render(&databases, *format)
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))
            }
            TraceSubcommandRequest::Status { all: true, format } => {
                let report = aggregate_current_status_all()
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))?;
                render_status_all::render(&report, *format)
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))
            }
            TraceSubcommandRequest::Status { all: false, format } => {
                let repo_root = if let Some(path) = context.repo_root() {
                    path.to_path_buf()
                } else {
                    std::env::current_dir().map_err(|err| {
                        ClassifiedError::runtime(format!(
                            "failed to determine current directory: {err}"
                        ))
                    })?
                };

                let report = resolve_current_status(&repo_root).map_err(|err| match err {
                    StatusErrorOrRuntime::Status(status_err) => {
                        ClassifiedError::validation(status_err.user_message())
                    }
                    StatusErrorOrRuntime::Runtime(runtime_err) => {
                        ClassifiedError::runtime(format!("{runtime_err:#}"))
                    }
                })?;

                render_status::render(&report, *format)
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))
            }
        }
    }
}
