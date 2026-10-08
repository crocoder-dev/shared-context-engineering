use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Context, Result};

pub trait FsOps: Send + Sync {}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StdFsOps;

impl FsOps for StdFsOps {}

pub trait GitOps: Send + Sync {
    fn run_command(&self, repo: &Path, args: &[&str]) -> Result<String>;

    fn is_available(&self) -> bool;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProcessGitOps;

impl GitOps for ProcessGitOps {
    fn run_command(&self, repo: &Path, args: &[&str]) -> Result<String> {
        run_git_command(repo, args)
    }

    fn is_available(&self) -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    }
}

fn run_git_command(current_dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(current_dir)
        .output()
        .with_context(|| {
            format!(
                "Failed to run git command in '{}' with args {:?}",
                current_dir.display(),
                args
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(anyhow!(
            "Git command {:?} failed in '{}': {}",
            args,
            current_dir.display(),
            detail
        ));
    }

    String::from_utf8(output.stdout)
        .with_context(|| format!("Git command {args:?} emitted invalid UTF-8"))
}
