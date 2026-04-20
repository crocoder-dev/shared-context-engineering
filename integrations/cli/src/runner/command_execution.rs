use std::path::Path;
use std::process::{Command, Output};

use crate::error::HarnessError;

pub(super) fn run_command(sce_binary: &Path, args: &[&str]) -> Result<Output, HarnessError> {
    let mut command = Command::new(sce_binary);
    command.args(args);
    command
        .output()
        .map_err(|error| HarnessError::CommandRunFailed {
            program: render_command(sce_binary, args),
            error: error.to_string(),
        })
}

pub(super) fn render_command(sce_binary: &Path, args: &[&str]) -> String {
    let mut command = sce_binary.display().to_string();
    for argument in args {
        command.push(' ');
        command.push_str(argument);
    }
    command
}
