use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

use crate::error::HarnessError;

pub(super) fn run_command(sce_binary: &Path, args: &[&str]) -> Result<Output, HarnessError> {
    let execution =
        HermeticCommandExecution::new().map_err(|error| HarnessError::CommandRunFailed {
            program: render_command(sce_binary, args),
            error,
        })?;

    let mut command = Command::new(sce_binary);
    command
        .args(args)
        .current_dir(execution.workdir())
        .env_clear()
        .env("HOME", execution.home_dir())
        .env("PATH", execution.path())
        .env("LANG", execution.lang())
        .env("XDG_CONFIG_HOME", execution.xdg_config_home())
        .env("XDG_STATE_HOME", execution.xdg_state_home())
        .env("XDG_CACHE_HOME", execution.xdg_cache_home());

    command
        .output()
        .map_err(|error| HarnessError::CommandRunFailed {
            program: render_command(sce_binary, args),
            error: error.to_string(),
        })
}

struct HermeticCommandExecution {
    _root: TempDir,
    workdir: PathBuf,
    home_dir: PathBuf,
    xdg_config_home: PathBuf,
    xdg_state_home: PathBuf,
    xdg_cache_home: PathBuf,
    path: OsString,
    lang: &'static str,
}

impl HermeticCommandExecution {
    fn new() -> Result<Self, String> {
        let root = tempfile::Builder::new()
            .prefix("sce-cli-integration-runner.")
            .tempdir()
            .map_err(|error| format!("failed to create temporary execution directory: {error}"))?;

        let root_path = root.path();
        let workdir = root_path.join("workdir");
        let home_dir = root_path.join("home");
        let xdg_config_home = root_path.join("xdg/config");
        let xdg_state_home = root_path.join("xdg/state");
        let xdg_cache_home = root_path.join("xdg/cache");

        for path in [
            workdir.clone(),
            home_dir.clone(),
            xdg_config_home.clone(),
            xdg_state_home.clone(),
            xdg_cache_home.clone(),
        ] {
            fs::create_dir_all(&path).map_err(|error| {
                format!(
                    "failed to create isolated path '{}': {error}",
                    path.display()
                )
            })?;
        }

        Ok(Self {
            _root: root,
            workdir,
            home_dir,
            xdg_config_home,
            xdg_state_home,
            xdg_cache_home,
            path: sanitized_path(),
            lang: "C.UTF-8",
        })
    }

    fn workdir(&self) -> &Path {
        &self.workdir
    }

    fn home_dir(&self) -> &Path {
        &self.home_dir
    }

    fn xdg_config_home(&self) -> &Path {
        &self.xdg_config_home
    }

    fn xdg_state_home(&self) -> &Path {
        &self.xdg_state_home
    }

    fn xdg_cache_home(&self) -> &Path {
        &self.xdg_cache_home
    }

    fn path(&self) -> &OsString {
        &self.path
    }

    fn lang(&self) -> &'static str {
        self.lang
    }
}

fn sanitized_path() -> OsString {
    env::var_os("PATH").unwrap_or_else(|| OsString::from("/usr/local/bin:/usr/bin:/bin"))
}

pub(super) fn render_command(sce_binary: &Path, args: &[&str]) -> String {
    let mut command = sce_binary.display().to_string();
    for argument in args {
        command.push(' ');
        command.push_str(argument);
    }
    command
}
