use std::path::PathBuf;
use std::process::Command;

use crate::cli::Args;
use crate::error::HarnessError;

const SCE_BINARY_ENV: &str = "SCE_CLI_INTEGRATION_SCE_BIN";

pub(crate) struct Runner;

impl Runner {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn run(self, _args: Args) -> Result<(), HarnessError> {
        let sce_binary = resolve_sce_binary()?;
        let output = Command::new(&sce_binary)
            .arg("--help")
            .output()
            .map_err(|error| HarnessError::CommandRunFailed {
                program: sce_binary.display().to_string(),
                error: error.to_string(),
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if !output.status.success() {
            return Err(HarnessError::HelpCommandNonZero {
                status: output.status.code().unwrap_or(-1),
                stdout,
                stderr,
            });
        }

        Ok(())
    }
}

fn resolve_sce_binary() -> Result<PathBuf, HarnessError> {
    let binary = std::env::var_os(SCE_BINARY_ENV).ok_or(HarnessError::MissingEnv {
        env: SCE_BINARY_ENV,
    })?;
    Ok(PathBuf::from(binary))
}
