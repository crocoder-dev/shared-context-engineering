use std::path::PathBuf;
use std::process::{Command, Output};

use crate::cli::Args;
use crate::error::HarnessError;

const SCE_BINARY_ENV: &str = "SCE_CLI_INTEGRATION_SCE_BIN";

pub(crate) struct Runner;

#[derive(Clone, Copy)]
struct CommandSuite {
    name: &'static str,
    cases: &'static [CommandCase],
}

#[derive(Clone, Copy)]
enum ExpectedStatus {
    Success,
}

#[derive(Clone, Copy)]
struct OutputExpectation {
    must_be_empty: bool,
    must_be_non_empty: bool,
    required_substrings: &'static [&'static str],
    validator: Option<OutputValidator>,
}

type OutputValidator = fn(&str) -> Result<(), String>;

impl OutputExpectation {
    const fn non_empty() -> Self {
        Self {
            must_be_empty: false,
            must_be_non_empty: true,
            required_substrings: &[],
            validator: None,
        }
    }

    const fn with_required_substrings(
        mut self,
        required_substrings: &'static [&'static str],
    ) -> Self {
        self.required_substrings = required_substrings;
        self
    }

    const fn with_validator(mut self, validator: OutputValidator) -> Self {
        self.validator = Some(validator);
        self
    }
}

#[derive(Clone, Copy)]
struct CaseExpectation {
    status: ExpectedStatus,
    stdout: OutputExpectation,
}

#[derive(Clone, Copy)]
struct CommandCase {
    name: &'static str,
    argv: &'static [&'static str],
    expectation: CaseExpectation,
}

const HELP_CASES: &[CommandCase] = &[CommandCase {
    name: "top-level-help",
    argv: &["--help"],
    expectation: CaseExpectation {
        status: ExpectedStatus::Success,
        stdout: OutputExpectation::non_empty().with_required_substrings(&["Usage:"]),
    },
}];

const VERSION_CASES: &[CommandCase] = &[
    CommandCase {
        name: "version-default-text",
        argv: &["version"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty().with_validator(validate_version_text_output),
        },
    },
    CommandCase {
        name: "version-explicit-text-format",
        argv: &["version", "--format", "text"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty().with_validator(validate_version_text_output),
        },
    },
    CommandCase {
        name: "version-json-format",
        argv: &["version", "--format", "json"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty().with_validator(validate_version_json_output),
        },
    },
    CommandCase {
        name: "top-level-version-long-flag",
        argv: &["--version"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty().with_validator(validate_version_text_output),
        },
    },
    CommandCase {
        name: "top-level-version-short-flag",
        argv: &["-V"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty().with_validator(validate_version_text_output),
        },
    },
];

const COMMAND_SUITES: &[CommandSuite] = &[
    CommandSuite {
        name: "help",
        cases: HELP_CASES,
    },
    CommandSuite {
        name: "version",
        cases: VERSION_CASES,
    },
];

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

fn select_suites(command: Option<&str>) -> Result<Vec<&'static CommandSuite>, HarnessError> {
    match command {
        Some(name) => {
            let suite = COMMAND_SUITES
                .iter()
                .find(|suite| suite.name == name)
                .ok_or_else(|| HarnessError::UnknownCommandSelector {
                    selected: name.to_string(),
                    available: render_available_command_suites(),
                })?;
            Ok(vec![suite])
        }
        None => Ok(COMMAND_SUITES.iter().collect()),
    }
}

fn render_available_command_suites() -> String {
    let mut rendered = String::new();
    for (index, suite) in COMMAND_SUITES.iter().enumerate() {
        if index > 0 {
            rendered.push_str(", ");
        }
        rendered.push_str(suite.name);
    }
    rendered
}

fn run_case(sce_binary: &PathBuf, case: CommandCase) -> Result<(), HarnessError> {
    let output = run_command(sce_binary, case.argv)?;
    let status = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let command = render_command(sce_binary, case.argv);

    assert_status(case, status, &stdout, &stderr, &command)?;
    assert_stdout_output(
        case,
        &stdout,
        &stderr,
        status,
        &command,
        case.expectation.stdout,
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

fn assert_stdout_output(
    case: CommandCase,
    stdout: &str,
    stderr: &str,
    status: i32,
    command: &str,
    expectation: OutputExpectation,
) -> Result<(), HarnessError> {
    let trimmed_stdout = stdout.trim();
    let trimmed_stderr = stderr.trim();

    if expectation.must_be_empty && !trimmed_stdout.is_empty() {
        return Err(HarnessError::AssertionFailed {
            case: case.name,
            reason: "expected stdout to be empty".to_string(),
            command: command.to_string(),
            status,
            stdout: trimmed_stdout.to_string(),
            stderr: trimmed_stderr.to_string(),
        });
    }

    if expectation.must_be_non_empty && trimmed_stdout.is_empty() {
        return Err(HarnessError::AssertionFailed {
            case: case.name,
            reason: "expected stdout to be non-empty".to_string(),
            command: command.to_string(),
            status,
            stdout: trimmed_stdout.to_string(),
            stderr: trimmed_stderr.to_string(),
        });
    }

    for required in expectation.required_substrings {
        if !stdout.contains(required) {
            return Err(HarnessError::AssertionFailed {
                case: case.name,
                reason: format!("expected stdout to contain '{required}'"),
                command: command.to_string(),
                status,
                stdout: trimmed_stdout.to_string(),
                stderr: trimmed_stderr.to_string(),
            });
        }
    }

    if let Some(validator) = expectation.validator {
        if let Err(reason) = validator(stdout) {
            return Err(HarnessError::AssertionFailed {
                case: case.name,
                reason: format!("invalid stdout contract: {reason}"),
                command: command.to_string(),
                status,
                stdout: trimmed_stdout.to_string(),
                stderr: trimmed_stderr.to_string(),
            });
        }
    }

    Ok(())
}

fn validate_version_text_output(stream: &str) -> Result<(), String> {
    let payload = stream.trim();
    if payload.is_empty() {
        return Err("expected non-empty text payload".to_string());
    }

    let mut parts = payload.splitn(3, ' ');
    let binary = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    let profile = parts.next().unwrap_or_default();

    if binary.is_empty() {
        return Err("expected non-empty binary segment".to_string());
    }
    if binary.chars().any(char::is_whitespace) {
        return Err("expected binary segment without whitespace".to_string());
    }

    if version.is_empty() {
        return Err("expected non-empty version segment".to_string());
    }
    if version.chars().any(char::is_whitespace) {
        return Err("expected version segment without whitespace".to_string());
    }

    if !profile.starts_with('(') || !profile.ends_with(')') || profile.len() <= 2 {
        return Err("expected profile segment formatted as '(...)'".to_string());
    }

    Ok(())
}

fn validate_version_json_output(stream: &str) -> Result<(), String> {
    const MAX_DYNAMIC_FIELD_LENGTH: usize = 64;

    let payload = stream.trim();
    if payload.is_empty() {
        return Err("expected non-empty JSON payload".to_string());
    }

    assert_json_field_equals(payload, "status", "ok")?;
    assert_json_field_equals(payload, "command", "version")?;
    assert_json_field_equals(payload, "binary", "shared-context-engineering")?;

    let version = extract_json_string_field(payload, "version")?;
    assert_non_empty_bounded_field("version", &version, MAX_DYNAMIC_FIELD_LENGTH)?;
    if !version
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || ".-+".contains(character))
    {
        return Err(
            "expected 'version' to contain only ASCII alphanumeric characters or one of: '.', '-', '+'"
                .to_string(),
        );
    }
    if !version.chars().any(|character| character.is_ascii_digit()) {
        return Err("expected 'version' to contain at least one digit".to_string());
    }

    let git_commit = extract_json_string_field(payload, "git_commit")?;
    assert_non_empty_bounded_field("git_commit", &git_commit, MAX_DYNAMIC_FIELD_LENGTH)?;
    if !git_commit
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
    {
        return Err(
            "expected 'git_commit' to contain only ASCII alphanumeric characters or one of: '.', '_', '-'"
                .to_string(),
        );
    }

    Ok(())
}

fn assert_json_field_equals(payload: &str, field: &str, expected: &str) -> Result<(), String> {
    let actual = extract_json_string_field(payload, field)?;
    if actual == expected {
        return Ok(());
    }

    Err(format!(
        "expected '{field}' to equal '{expected}', got '{actual}'"
    ))
}

fn assert_non_empty_bounded_field(
    field: &str,
    value: &str,
    max_length: usize,
) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("expected '{field}' to be non-empty"));
    }

    if value.len() > max_length {
        return Err(format!(
            "expected '{field}' length <= {max_length}, got {}",
            value.len()
        ));
    }

    Ok(())
}

fn extract_json_string_field(payload: &str, field: &str) -> Result<String, String> {
    let field_token = format!("\"{field}\"");
    let field_start = payload
        .find(&field_token)
        .ok_or_else(|| format!("missing JSON string field '{field}'"))?;
    let after_field = &payload[field_start + field_token.len()..];
    let colon_offset = after_field
        .find(':')
        .ok_or_else(|| format!("missing ':' after JSON field '{field}'"))?;
    let after_colon = after_field[colon_offset + 1..].trim_start();

    if !after_colon.starts_with('"') {
        return Err(format!("expected JSON string value for field '{field}'"));
    }

    let mut value = String::new();
    let mut escaped = false;

    for character in after_colon[1..].chars() {
        if escaped {
            value.push(character);
            escaped = false;
            continue;
        }

        if character == '\\' {
            escaped = true;
            continue;
        }

        if character == '"' {
            return Ok(value);
        }

        value.push(character);
    }

    Err(format!(
        "unterminated JSON string value for field '{field}'"
    ))
}

fn resolve_sce_binary() -> Result<PathBuf, HarnessError> {
    let binary = std::env::var_os(SCE_BINARY_ENV).ok_or(HarnessError::MissingEnv {
        env: SCE_BINARY_ENV,
    })?;
    Ok(PathBuf::from(binary))
}
