use std::path::PathBuf;
use std::process::{Command, Output};

use crate::cli::Args;
use crate::error::HarnessError;

const SCE_BINARY_ENV: &str = "SCE_CLI_INTEGRATION_SCE_BIN";

pub(crate) struct Runner;

#[derive(Clone, Copy)]
enum ExpectedStatus {
    Success,
}

#[derive(Clone, Copy)]
struct OutputExpectation {
    must_be_empty: bool,
    must_be_non_empty: bool,
    required_substrings: &'static [&'static str],
}

impl OutputExpectation {
    const fn any() -> Self {
        Self {
            must_be_empty: false,
            must_be_non_empty: false,
            required_substrings: &[],
        }
    }

    const fn non_empty() -> Self {
        Self {
            must_be_empty: false,
            must_be_non_empty: true,
            required_substrings: &[],
        }
    }

    const fn with_required_substrings(
        mut self,
        required_substrings: &'static [&'static str],
    ) -> Self {
        self.required_substrings = required_substrings;
        self
    }
}

#[derive(Clone, Copy)]
struct CaseExpectation {
    status: ExpectedStatus,
    stdout: OutputExpectation,
    stderr: OutputExpectation,
}

#[derive(Clone, Copy)]
struct CommandCase {
    name: &'static str,
    argv: &'static [&'static str],
    expectation: CaseExpectation,
}

const CASES: &[CommandCase] = &[
    CommandCase {
        name: "top-level-help",
        argv: &["--help"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty().with_required_substrings(&["Usage:"]),
            stderr: OutputExpectation::any(),
        },
    },
    CommandCase {
        name: "version-default-text",
        argv: &["version"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty().with_required_substrings(&[
                "shared-context-engineering ",
                " (",
                ")",
            ]),
            stderr: OutputExpectation::any(),
        },
    },
    CommandCase {
        name: "version-explicit-text-format",
        argv: &["version", "--format", "text"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty().with_required_substrings(&[
                "shared-context-engineering ",
                " (",
                ")",
            ]),
            stderr: OutputExpectation::any(),
        },
    },
    CommandCase {
        name: "version-json-format",
        argv: &["version", "--format", "json"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty().with_required_substrings(&[
                "\"status\": \"ok\"",
                "\"command\": \"version\"",
                "\"binary\": \"shared-context-engineering\"",
                "\"version\": ",
                "\"git_commit\": ",
            ]),
            stderr: OutputExpectation::any(),
        },
    },
    CommandCase {
        name: "top-level-version-long-flag",
        argv: &["--version"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty().with_required_substrings(&[
                "shared-context-engineering ",
                " (",
                ")",
            ]),
            stderr: OutputExpectation::any(),
        },
    },
    CommandCase {
        name: "top-level-version-short-flag",
        argv: &["-V"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty().with_required_substrings(&[
                "shared-context-engineering ",
                " (",
                ")",
            ]),
            stderr: OutputExpectation::any(),
        },
    },
    CommandCase {
        name: "version-help-long-flag",
        argv: &["version", "--help"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::any(),
            stderr: OutputExpectation::any(),
        },
    },
];

impl Runner {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn run(self, _args: Args) -> Result<(), HarnessError> {
        let sce_binary = resolve_sce_binary()?;
        for case in CASES {
            run_case(&sce_binary, *case)?;
        }
        Ok(())
    }
}

fn run_case(sce_binary: &PathBuf, case: CommandCase) -> Result<(), HarnessError> {
    let output = run_command(sce_binary, case.argv)?;
    let status = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let command = render_command(sce_binary, case.argv);

    assert_status(case, status, &stdout, &stderr, &command)?;
    assert_output(
        "stdout",
        case,
        &stdout,
        &stderr,
        status,
        &command,
        case.expectation.stdout,
    )?;
    assert_output(
        "stderr",
        case,
        &stderr,
        &stdout,
        status,
        &command,
        case.expectation.stderr,
    )?;

    Ok(())
}

fn run_command(sce_binary: &PathBuf, args: &[&str]) -> Result<Output, HarnessError> {
    let mut command = Command::new(sce_binary);
    command.args(args);
    command
        .output()
        .map_err(|error| HarnessError::CommandRunFailed {
            program: render_command(sce_binary, args),
            error: error.to_string(),
        })
}

fn render_command(sce_binary: &PathBuf, args: &[&str]) -> String {
    let mut command = sce_binary.display().to_string();
    for argument in args {
        command.push(' ');
        command.push_str(argument);
    }
    command
}

fn assert_status(
    case: CommandCase,
    status: i32,
    stdout: &str,
    stderr: &str,
    command: &str,
) -> Result<(), HarnessError> {
    let status_matches = match case.expectation.status {
        ExpectedStatus::Success => status == 0,
    };

    if status_matches {
        return Ok(());
    }

    let reason = match case.expectation.status {
        ExpectedStatus::Success => format!("expected success status 0, got {status}"),
    };

    Err(HarnessError::AssertionFailed {
        case: case.name,
        reason,
        command: command.to_string(),
        status,
        stdout: stdout.trim().to_string(),
        stderr: stderr.trim().to_string(),
    })
}

fn assert_output(
    stream_name: &'static str,
    case: CommandCase,
    stream: &str,
    other_stream: &str,
    status: i32,
    command: &str,
    expectation: OutputExpectation,
) -> Result<(), HarnessError> {
    let trimmed = stream.trim();

    if expectation.must_be_empty && !trimmed.is_empty() {
        return Err(HarnessError::AssertionFailed {
            case: case.name,
            reason: format!("expected {stream_name} to be empty"),
            command: command.to_string(),
            status,
            stdout: if stream_name == "stdout" {
                trimmed.to_string()
            } else {
                other_stream.trim().to_string()
            },
            stderr: if stream_name == "stderr" {
                trimmed.to_string()
            } else {
                other_stream.trim().to_string()
            },
        });
    }

    if expectation.must_be_non_empty && trimmed.is_empty() {
        return Err(HarnessError::AssertionFailed {
            case: case.name,
            reason: format!("expected {stream_name} to be non-empty"),
            command: command.to_string(),
            status,
            stdout: if stream_name == "stdout" {
                trimmed.to_string()
            } else {
                other_stream.trim().to_string()
            },
            stderr: if stream_name == "stderr" {
                trimmed.to_string()
            } else {
                other_stream.trim().to_string()
            },
        });
    }

    for required in expectation.required_substrings {
        if !stream.contains(required) {
            return Err(HarnessError::AssertionFailed {
                case: case.name,
                reason: format!("expected {stream_name} to contain '{required}'"),
                command: command.to_string(),
                status,
                stdout: if stream_name == "stdout" {
                    trimmed.to_string()
                } else {
                    other_stream.trim().to_string()
                },
                stderr: if stream_name == "stderr" {
                    trimmed.to_string()
                } else {
                    other_stream.trim().to_string()
                },
            });
        }
    }

    Ok(())
}

fn resolve_sce_binary() -> Result<PathBuf, HarnessError> {
    let binary = std::env::var_os(SCE_BINARY_ENV).ok_or(HarnessError::MissingEnv {
        env: SCE_BINARY_ENV,
    })?;
    Ok(PathBuf::from(binary))
}
