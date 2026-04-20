use super::catalog::{CommandCase, ExpectedStatus, OutputExpectation};
use crate::error::HarnessError;

pub(super) fn assert_status(
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

pub(super) fn assert_stdout_output(
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
