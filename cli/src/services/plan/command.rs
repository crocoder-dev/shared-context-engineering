use std::fmt::{self, Write};
use std::io;
use std::path::{Path, PathBuf};

use crate::app::HasFs;
use crate::services::capabilities::FsOps;
use crate::services::error::{CliError, FailureClass};
use crate::services::output_format::OutputFormat;
use crate::services::plan::parse::{self, PlanParseError};
use crate::services::plan::{ContextSync, PlanStatus, PlanStatusRequest};

#[derive(Debug)]
pub enum PlanStatusError {
    Read {
        path: PathBuf,
        source: io::Error,
    },
    InvalidUtf8 {
        path: PathBuf,
    },
    Parse {
        path: PathBuf,
        source: PlanParseError,
    },
}

impl fmt::Display for PlanStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => write!(
                f,
                "failed to read plan '{}': {source}. Try: pass the path of an existing SCE plan file.",
                path.display()
            ),
            Self::InvalidUtf8 { path } => write!(
                f,
                "plan '{}' is not valid UTF-8. Try: save the plan as UTF-8 Markdown.",
                path.display()
            ),
            Self::Parse { path, source } => write!(
                f,
                "plan '{}' has uninterpretable lifecycle state: {source}. Try: fix the plan entry at the reported line.",
                path.display()
            ),
        }
    }
}

impl std::error::Error for PlanStatusError {}

impl From<PlanStatusError> for CliError {
    fn from(error: PlanStatusError) -> Self {
        let class = match error {
            PlanStatusError::Read { .. } => FailureClass::Runtime,
            PlanStatusError::InvalidUtf8 { .. } | PlanStatusError::Parse { .. } => {
                FailureClass::Validation
            }
        };
        CliError::internal(class, error)
    }
}

pub struct PlanStatusCommand {
    pub request: PlanStatusRequest,
}

impl PlanStatusCommand {
    pub fn execute<C: HasFs>(&self, context: &C) -> Result<String, CliError> {
        let status = read_plan_status(context.fs(), &self.request.path)?;
        match self.request.format {
            OutputFormat::Text => Ok(render_text(&status)),
            OutputFormat::Json => serde_json::to_string_pretty(&status).map_err(CliError::runtime),
        }
    }
}

fn read_plan_status(fs: &impl FsOps, path: &Path) -> Result<PlanStatus, PlanStatusError> {
    let source = fs.read_to_string(path).map_err(|source| {
        if source.kind() == io::ErrorKind::InvalidData {
            PlanStatusError::InvalidUtf8 {
                path: path.to_path_buf(),
            }
        } else {
            PlanStatusError::Read {
                path: path.to_path_buf(),
                source,
            }
        }
    })?;
    parse::parse(&source).map_err(|source| PlanStatusError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

fn render_text(status: &PlanStatus) -> String {
    let mut out = String::new();
    for task in &status.tasks {
        let sync = task.context_sync.map_or("none", ContextSync::as_str);
        let _ = writeln!(
            out,
            "{} [{}] {} (context sync: {sync})",
            task.id,
            task.status.as_str(),
            task.title
        );
    }
    match &status.validation {
        Some(validation) => {
            let _ = write!(
                out,
                "Validation: {} (report sha256 {})",
                validation.status.as_str(),
                validation.report_sha256
            );
        }
        None => out.push_str("Validation: none"),
    }
    out
}

#[cfg(test)]
mod tests {
    use std::process::ExitCode;

    use serde_json::json;

    use crate::app::HasFs;
    use crate::services::app_support::{self, RunOutcome};
    use crate::services::capabilities::StdFsOps;
    use crate::services::command_registry::RuntimeCommand;
    use crate::services::observability::traits::NoopLogger;
    use crate::services::parse::command_runtime::parse_runtime_command;

    struct FsContext(StdFsOps);

    impl HasFs for FsContext {
        type Fs = StdFsOps;

        fn fs(&self) -> &StdFsOps {
            &self.0
        }
    }

    fn run(plan: &str) -> (ExitCode, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.md");
        std::fs::write(&path, plan).unwrap();
        let args = [
            "sce",
            "plan",
            "status",
            path.to_str().unwrap(),
            "--format",
            "json",
        ];
        let Ok(RuntimeCommand::PlanStatus(command)) =
            parse_runtime_command(args.map(String::from), None)
        else {
            panic!("expected plan status command");
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = app_support::render_run_outcome(
            RunOutcome::<NoopLogger> {
                result: command.execute(&FsContext(StdFsOps)),
                logger: None,
                startup_diagnostic: None,
            },
            &mut stdout,
            &mut stderr,
        );
        (
            code,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    #[test]
    fn prints_json_contract() {
        let (code, stdout, stderr) = run("## Task stack\n\n\
            - [x] T01: `Create database schema` (status:done)\n  - Context synchronization: synced\n\
            - [ ] T02: `Implement repository` (status:todo)\n");
        assert_eq!(code, ExitCode::SUCCESS, "{stderr}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&stdout).unwrap(),
            json!({
                "tasks": [
                    {"id": "T01", "title": "Create database schema", "status": "done", "context_sync": "synced"},
                    {"id": "T02", "title": "Implement repository", "status": "todo", "context_sync": null},
                ],
                "validation": null,
            })
        );
    }

    #[test]
    fn malformed_plan_fails_without_stdout() {
        let (code, stdout, stderr) = run("## Task stack\n\n- [x] T01: `A` (status:todo)\n");
        assert_eq!(code, ExitCode::from(3));
        assert_eq!(stdout, "");
        assert!(stderr.contains("SCE-ERR-VALIDATION"), "{stderr}");
        assert!(stderr.contains("line 3: task T01 is checked"), "{stderr}");
    }
}
