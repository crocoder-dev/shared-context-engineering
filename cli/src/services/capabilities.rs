#![allow(dead_code)]

use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};

pub trait FsOps: Send + Sync {
    fn read_file(&self, path: &Path) -> Result<String>;

    fn write_file(&self, path: &Path, content: &str) -> Result<()>;

    fn metadata(&self, path: &Path) -> Result<Metadata>;

    fn exists(&self, path: &Path) -> bool;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StdFsOps;

impl FsOps for StdFsOps {
    fn read_file(&self, path: &Path) -> Result<String> {
        fs::read_to_string(path)
            .with_context(|| format!("Failed to read file '{}'", path.display()))
    }

    fn write_file(&self, path: &Path, content: &str) -> Result<()> {
        fs::write(path, content)
            .with_context(|| format!("Failed to write file '{}'", path.display()))
    }

    fn metadata(&self, path: &Path) -> Result<Metadata> {
        fs::metadata(path)
            .with_context(|| format!("Failed to inspect path '{}'", path.display()))
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

pub trait GitOps: Send + Sync {
    fn run_command(&self, repo: &Path, args: &[&str]) -> Result<String>;

    fn resolve_repository_root(&self, dir: &Path) -> Result<PathBuf>;

    fn resolve_hooks_directory(&self, repo: &Path) -> Result<PathBuf>;

    fn is_available(&self) -> bool;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProcessGitOps;

impl GitOps for ProcessGitOps {
    fn run_command(&self, repo: &Path, args: &[&str]) -> Result<String> {
        run_git_command(repo, args)
    }

    fn resolve_repository_root(&self, dir: &Path) -> Result<PathBuf> {
        let output = run_git_command(dir, &["rev-parse", "--show-toplevel"])?;
        Ok(PathBuf::from(output.trim()))
    }

    fn resolve_hooks_directory(&self, repo: &Path) -> Result<PathBuf> {
        let output = run_git_command(repo, &["rev-parse", "--git-path", "hooks"])?;
        let hooks_path = PathBuf::from(output.trim());

        if hooks_path.is_absolute() {
            Ok(hooks_path)
        } else {
            Ok(repo.join(hooks_path))
        }
    }

    fn is_available(&self) -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
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
        .with_context(|| format!("Git command {:?} emitted invalid UTF-8", args))
}

#[cfg(test)]
pub mod test_stubs {
    use super::*;

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub struct UnimplementedFsOps;

    impl FsOps for UnimplementedFsOps {
        fn read_file(&self, _path: &Path) -> Result<String> {
            unimplemented!("test stub must implement read_file for this test")
        }

        fn write_file(&self, _path: &Path, _content: &str) -> Result<()> {
            unimplemented!("test stub must implement write_file for this test")
        }

        fn metadata(&self, _path: &Path) -> Result<Metadata> {
            unimplemented!("test stub must implement metadata for this test")
        }

        fn exists(&self, _path: &Path) -> bool {
            unimplemented!("test stub must implement exists for this test")
        }
    }

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub struct UnimplementedGitOps;

    impl GitOps for UnimplementedGitOps {
        fn run_command(&self, _repo: &Path, _args: &[&str]) -> Result<String> {
            unimplemented!("test stub must implement run_command for this test")
        }

        fn resolve_repository_root(&self, _dir: &Path) -> Result<PathBuf> {
            unimplemented!("test stub must implement resolve_repository_root for this test")
        }

        fn resolve_hooks_directory(&self, _repo: &Path) -> Result<PathBuf> {
            unimplemented!("test stub must implement resolve_hooks_directory for this test")
        }

        fn is_available(&self) -> bool {
            unimplemented!("test stub must implement is_available for this test")
        }
    }
}
