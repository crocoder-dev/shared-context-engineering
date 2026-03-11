#![allow(dead_code)]

use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

pub type TestResult<T> = Result<T, Box<dyn Error>>;

#[derive(Debug)]
pub struct IntegrationTempDir {
    path: PathBuf,
}

impl IntegrationTempDir {
    pub fn new(prefix: &str) -> TestResult<Self> {
        let epoch_nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path =
            std::env::temp_dir().join(format!("{prefix}-{}-{epoch_nanos}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for IntegrationTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Debug)]
pub struct CommandResult {
    pub status: std::process::ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl CommandResult {
    pub fn success(&self) -> bool {
        self.status.success()
    }
}

#[derive(Debug)]
pub struct BinaryIntegrationHarness {
    temp: IntegrationTempDir,
    repo_root: PathBuf,
    state_home: PathBuf,
    home_dir: PathBuf,
}

impl BinaryIntegrationHarness {
    pub fn new(prefix: &str) -> TestResult<Self> {
        let temp = IntegrationTempDir::new(prefix)?;
        let repo_root = temp.path().join("repo");
        let state_home = temp.path().join("xdg-state");
        let home_dir = temp.path().join("home");

        fs::create_dir_all(&repo_root)?;
        fs::create_dir_all(&state_home)?;
        fs::create_dir_all(&home_dir)?;

        Ok(Self {
            temp,
            repo_root,
            state_home,
            home_dir,
        })
    }

    pub fn temp_path(&self) -> &Path {
        self.temp.path()
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    pub fn state_home(&self) -> &Path {
        &self.state_home
    }

    pub fn home_dir(&self) -> &Path {
        &self.home_dir
    }

    pub fn init_git_repo(&self) -> TestResult<()> {
        let result = self.run_git(["init", "-q"])?;
        if !result.success() {
            return Err(format!(
                "git init failed:\nstdout:\n{}\nstderr:\n{}",
                result.stdout, result.stderr
            )
            .into());
        }
        Ok(())
    }

    pub fn configure_local_hooks_path(&self, relative_hooks_path: &str) -> TestResult<()> {
        let result = self.run_git(["config", "core.hooksPath", relative_hooks_path])?;
        if !result.success() {
            return Err(format!(
                "git config core.hooksPath failed:\nstdout:\n{}\nstderr:\n{}",
                result.stdout, result.stderr
            )
            .into());
        }
        Ok(())
    }

    pub fn run_sce<I, S>(&self, args: I) -> TestResult<CommandResult>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.base_command(sce_binary_path()).args(args).output()?;
        Ok(render_command_result(&output))
    }

    pub fn run_git<I, S>(&self, args: I) -> TestResult<CommandResult>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.base_command("git").args(args).output()?;
        Ok(render_command_result(&output))
    }

    pub fn base_command<P: AsRef<OsStr>>(&self, program: P) -> Command {
        let mut command = Command::new(program);
        command
            .current_dir(&self.repo_root)
            .env("XDG_STATE_HOME", &self.state_home)
            .env("HOME", &self.home_dir)
            .env("LOCALAPPDATA", &self.state_home)
            .env("APPDATA", &self.state_home)
            .env("GIT_CONFIG_GLOBAL", null_device_path())
            .env("GIT_CONFIG_NOSYSTEM", "1");
        command
    }
}

pub fn sce_binary_path() -> PathBuf {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_sce") {
        return PathBuf::from(path);
    }

    let test_executable = std::env::current_exe()
        .expect("integration test should resolve current executable path for binary fallback");
    let debug_root = test_executable
        .parent()
        .and_then(Path::parent)
        .expect("integration test executable should run from target/{profile}/deps");

    let candidate = debug_root.join(binary_filename("sce"));
    assert!(
        candidate.exists(),
        "integration test could not resolve compiled sce binary at '{}'",
        candidate.display()
    );

    candidate
}

pub fn null_device_path() -> &'static str {
    #[cfg(windows)]
    {
        "NUL"
    }

    #[cfg(not(windows))]
    {
        "/dev/null"
    }
}

fn binary_filename(base: &str) -> String {
    #[cfg(windows)]
    {
        format!("{base}.exe")
    }

    #[cfg(not(windows))]
    {
        base.to_string()
    }
}

pub fn render_command_result(output: &Output) -> CommandResult {
    CommandResult {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}
