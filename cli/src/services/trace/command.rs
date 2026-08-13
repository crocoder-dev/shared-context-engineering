use std::io::Write;

use crate::app::ContextWithRepoRoot;
use crate::services::error::ClassifiedError;
use crate::services::trace::discovery::discover_agent_trace_dbs;
use crate::services::trace::render_list;
use crate::services::trace::render_status;
use crate::services::trace::render_status_all;
use crate::services::trace::render_sync;
use crate::services::trace::shell::{run_agent_trace_db_shell, ShellTarget};
use crate::services::trace::status::{resolve_current_status, StatusErrorOrRuntime};
use crate::services::trace::status_all::aggregate_current_status_all;
use crate::services::trace::sync::{
    run_current_sync_with_progress_and_clock, NoopSyncProgressSink, SyncProgressClock,
    SyncProgressEvent, SyncProgressSink, SystemSyncProgressClock, TraceSyncError,
};
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
        StatusErrorOrRuntime::Runtime(runtime_err) => {
            ClassifiedError::runtime(format!("{runtime_err:#}"))
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn classify_sync_error(err: TraceSyncError) -> ClassifiedError {
    ClassifiedError::runtime(format!("{err}"))
}

struct StderrSyncProgressReporter<'a, W> {
    writer: &'a mut W,
}

impl<'a, W> StderrSyncProgressReporter<'a, W> {
    fn new(writer: &'a mut W) -> Self {
        Self { writer }
    }
}

impl<W> SyncProgressSink for StderrSyncProgressReporter<'_, W>
where
    W: Write,
{
    fn report(&mut self, event: SyncProgressEvent) {
        let _ = writeln!(self.writer, "{}", format_progress_event(&event));
        let _ = self.writer.flush();
    }
}

fn format_progress_event(event: &SyncProgressEvent) -> String {
    match event {
        SyncProgressEvent::Started { timestamp } => {
            format!("Starting Agent Trace sync at {timestamp}...")
        }
        SyncProgressEvent::BatchAccepted {
            stream,
            batch_rows,
            uploaded,
            cursor,
        } => format!(
            "{stream}: uploaded batch of {batch_rows} rows ({uploaded} total, cursor {cursor})"
        ),
        SyncProgressEvent::StreamCompleted {
            stream,
            uploaded,
            cursor,
            batches,
        } if *batches == 0 => {
            format!("{stream}: complete - no new rows uploaded (cursor {cursor})")
        }
        SyncProgressEvent::StreamCompleted {
            stream,
            uploaded,
            cursor,
            batches,
        } => format!(
            "{stream}: complete - {uploaded} rows uploaded in {batches} batches (cursor {cursor})"
        ),
        SyncProgressEvent::Finished { timestamp } => {
            format!("Agent Trace sync finished at {timestamp}.")
        }
    }
}

impl TraceCommand {
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
        match &self.request.subcommand {
            TraceSubcommandRequest::DbList { format } => {
                let databases = discover_agent_trace_dbs()
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))?;
                render_list::render(&databases, *format)
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))
            }
            TraceSubcommandRequest::DbShell { identifier } => {
                let target = if let Some(identifier) = identifier {
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
            TraceSubcommandRequest::Status { all: true, format } => {
                let report = aggregate_current_status_all()
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))?;
                render_status_all::render(&report, *format)
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))
            }
            TraceSubcommandRequest::Status { all: false, format } => {
                let repo_root = current_repo_root(context)?;

                let report = resolve_current_status(&repo_root).map_err(classify_status_error)?;

                render_status::render(&report, *format)
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))
            }
            TraceSubcommandRequest::Sync { format } => {
                let repo_root = current_repo_root(context)?;

                let report = match format {
                    crate::services::output_format::OutputFormat::Text => {
                        let mut progress = StderrSyncProgressReporter::new(stderr);
                        run_current_sync_with_progress_and_clock(&repo_root, &mut progress, clock)
                    }
                    crate::services::output_format::OutputFormat::Json => {
                        let mut progress = NoopSyncProgressSink;
                        run_current_sync_with_progress_and_clock(&repo_root, &mut progress, clock)
                    }
                }
                .map_err(classify_sync_error)?;

                render_sync::render(&report, *format)
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_reporter_writes_deterministic_text_lines_and_flushes_each_event() {
        let mut output = Vec::new();
        let mut reporter = StderrSyncProgressReporter::new(&mut output);

        reporter.report(SyncProgressEvent::Started {
            timestamp: "2026-01-02T03:04:05Z".to_string(),
        });
        reporter.report(SyncProgressEvent::BatchAccepted {
            stream: "messages",
            batch_rows: 500,
            uploaded: 500,
            cursor: 500,
        });
        reporter.report(SyncProgressEvent::StreamCompleted {
            stream: "parts",
            uploaded: 0,
            cursor: 12,
            batches: 0,
        });
        reporter.report(SyncProgressEvent::Finished {
            timestamp: "2026-01-02T03:04:06Z".to_string(),
        });

        assert_eq!(
            String::from_utf8(output).expect("progress output should be UTF-8"),
            "Starting Agent Trace sync at 2026-01-02T03:04:05Z...\nmessages: uploaded batch of 500 rows (500 total, cursor 500)\nparts: complete - no new rows uploaded (cursor 12)\nAgent Trace sync finished at 2026-01-02T03:04:06Z.\n"
        );
    }
}
