use crate::app::ContextWithRepoRoot;
use crate::services::error::ClassifiedError;
use crate::services::trace::discovery::{
    discover_agent_trace_dbs, discover_legacy_agent_trace_dbs,
};
use crate::services::trace::render_list;
use crate::services::trace::render_status;
use crate::services::trace::render_status_all;
use crate::services::trace::shell::{run_agent_trace_db_shell, ShellTarget};
use crate::services::trace::status::{resolve_current_status, StatusErrorOrRuntime};
use crate::services::trace::status_all::aggregate_current_status_all;
use crate::services::trace::{
    resolve_agent_trace_db_identifier, TraceRequest, TraceSubcommandRequest,
};

pub struct TraceCommand {
    pub request: TraceRequest,
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

fn classify_status_error(err: StatusErrorOrRuntime) -> ClassifiedError {
    match err {
        StatusErrorOrRuntime::Status(status_err) => {
            ClassifiedError::validation(status_err.user_message())
        }
        StatusErrorOrRuntime::Runtime(runtime_err) => {
            ClassifiedError::runtime(format!("{runtime_err:#}"))
        }
    }
}

impl TraceCommand {
    pub fn execute<C>(&self, context: &C) -> Result<String, ClassifiedError>
    where
        C: ContextWithRepoRoot,
    {
        match &self.request.subcommand {
            TraceSubcommandRequest::DbList { format, legacy } => {
                let databases = if *legacy {
                    discover_legacy_agent_trace_dbs()
                } else {
                    discover_agent_trace_dbs()
                }
                .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))?;
                render_list::render(&databases, *format)
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))
            }
            TraceSubcommandRequest::DbShell { identifier, legacy } => {
                let target = if *legacy {
                    let Some(identifier) = identifier else {
                        return Err(ClassifiedError::validation(
                            "sce trace db shell --legacy requires a checkout ID or alias"
                                .to_string(),
                        ));
                    };
                    let databases = discover_legacy_agent_trace_dbs()
                        .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))?;
                    let database = resolve_agent_trace_db_identifier(&databases, identifier)
                        .map_err(|error| ClassifiedError::validation(error.user_message()))?;
                    ShellTarget {
                        alias: database.alias,
                        scope: database.kind.label().to_string(),
                        identifier: database.kind.identifier().to_string(),
                        path: database.path,
                    }
                } else if let Some(identifier) = identifier {
                    let databases = discover_agent_trace_dbs()
                        .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))?;
                    let database = resolve_agent_trace_db_identifier(&databases, identifier)
                        .map_err(|error| ClassifiedError::validation(error.user_message()))?;
                    ShellTarget {
                        alias: database.alias,
                        scope: database.kind.label().to_string(),
                        identifier: database.kind.identifier().to_string(),
                        path: database.path,
                    }
                } else {
                    let repo_root = current_repo_root(context)?;
                    let report =
                        resolve_current_status(&repo_root).map_err(classify_status_error)?;
                    ShellTarget {
                        alias: "current".to_string(),
                        scope: "repository".to_string(),
                        identifier: report
                            .repository_id
                            .unwrap_or_else(|| "unknown".to_string()),
                        path: report.database_path,
                    }
                };

                let stdin = std::io::stdin();
                let stdout = std::io::stdout();
                run_agent_trace_db_shell(&target, stdin.lock(), stdout.lock())
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))?;
                Ok(String::new())
            }
            TraceSubcommandRequest::Status {
                all: true,
                format,
                legacy,
            } => {
                let report = aggregate_current_status_all(*legacy)
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))?;
                render_status_all::render(&report, *format)
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))
            }
            TraceSubcommandRequest::Status {
                all: false,
                format,
                legacy,
            } => {
                let repo_root = current_repo_root(context)?;

                let report = if *legacy {
                    let state_root = crate::services::default_paths::resolve_state_data_root()
                        .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))?;
                    crate::services::trace::status::resolve_current_legacy_status_in(
                        &repo_root,
                        &state_root.join("sce"),
                    )
                } else {
                    resolve_current_status(&repo_root)
                }
                .map_err(classify_status_error)?;

                render_status::render(&report, *format)
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))
            }
        }
    }
}
