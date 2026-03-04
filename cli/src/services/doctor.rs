use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;

pub const NAME: &str = "doctor";

const REQUIRED_HOOKS: [&str; 3] = ["pre-commit", "commit-msg", "post-commit"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Readiness {
    Ready,
    NotReady,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HookPathSource {
    Default,
    LocalConfig,
    GlobalConfig,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HookFileHealth {
    name: &'static str,
    path: PathBuf,
    exists: bool,
    executable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HookDoctorReport {
    readiness: Readiness,
    repository_root: Option<PathBuf>,
    hook_path_source: HookPathSource,
    hooks_directory: Option<PathBuf>,
    hooks: Vec<HookFileHealth>,
    diagnostics: Vec<String>,
}

pub fn run_doctor() -> Result<String> {
    let report = build_report();
    Ok(format_report(&report))
}

fn build_report() -> HookDoctorReport {
    let repository_root = run_git_command(&["rev-parse", "--show-toplevel"]).map(PathBuf::from);
    let hooks_directory = run_git_command(&["rev-parse", "--git-path", "hooks"]).map(PathBuf::from);

    let local_hooks_path = run_git_command(&["config", "--local", "--get", "core.hooksPath"]);
    let global_hooks_path = run_git_command(&["config", "--global", "--get", "core.hooksPath"]);

    let hook_path_source = if local_hooks_path.is_some() {
        HookPathSource::LocalConfig
    } else if global_hooks_path.is_some() {
        HookPathSource::GlobalConfig
    } else {
        HookPathSource::Default
    };

    let mut diagnostics = Vec::new();
    let hooks = match hooks_directory.as_deref() {
        Some(directory) => collect_hook_health(directory, &mut diagnostics),
        None => {
            diagnostics.push(
                "Unable to resolve git hooks directory. Run this command inside a git repository."
                    .to_string(),
            );
            Vec::new()
        }
    };

    let readiness = if diagnostics.is_empty() {
        Readiness::Ready
    } else {
        Readiness::NotReady
    };

    HookDoctorReport {
        readiness,
        repository_root,
        hook_path_source,
        hooks_directory,
        hooks,
        diagnostics,
    }
}

fn collect_hook_health(directory: &Path, diagnostics: &mut Vec<String>) -> Vec<HookFileHealth> {
    if !directory.exists() {
        diagnostics.push(format!(
            "Hooks directory '{}' does not exist.",
            directory.display()
        ));
    }

    REQUIRED_HOOKS
        .iter()
        .map(|hook_name| {
            let hook_path = directory.join(hook_name);
            let metadata = fs::metadata(&hook_path).ok();
            let exists = metadata.is_some();
            let executable = metadata
                .as_ref()
                .is_some_and(|entry| entry.is_file() && is_executable(entry));

            if !exists {
                diagnostics.push(format!(
                    "Missing required hook '{}' at '{}'.",
                    hook_name,
                    hook_path.display()
                ));
            } else if !executable {
                diagnostics.push(format!(
                    "Hook '{}' exists but is not executable. Run 'chmod +x {}' to fix it.",
                    hook_name,
                    hook_path.display()
                ));
            }

            HookFileHealth {
                name: hook_name,
                path: hook_path,
                exists,
                executable,
            }
        })
        .collect()
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(metadata: &fs::Metadata) -> bool {
    metadata.is_file()
}

fn run_git_command(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn format_report(report: &HookDoctorReport) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "SCE doctor: {}",
        match report.readiness {
            Readiness::Ready => "ready",
            Readiness::NotReady => "not ready",
        }
    ));

    lines.push(format!(
        "Hooks path source: {}",
        match report.hook_path_source {
            HookPathSource::Default => "default (.git/hooks)",
            HookPathSource::LocalConfig => "per-repo core.hooksPath",
            HookPathSource::GlobalConfig => "global core.hooksPath",
        }
    ));

    lines.push(format!(
        "Repository root: {}",
        report
            .repository_root
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "(not detected)".to_string())
    ));

    lines.push(format!(
        "Effective hooks directory: {}",
        report
            .hooks_directory
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "(not detected)".to_string())
    ));

    lines.push("Required hooks:".to_string());
    for hook in &report.hooks {
        let state = if hook.exists && hook.executable {
            "ok"
        } else if !hook.exists {
            "missing"
        } else {
            "misconfigured"
        };
        lines.push(format!(
            "- {}: {} ({})",
            hook.name,
            state,
            hook.path.display()
        ));
    }

    if report.diagnostics.is_empty() {
        lines.push("Diagnostics: none".to_string());
    } else {
        lines.push("Diagnostics:".to_string());
        for diagnostic in &report.diagnostics {
            lines.push(format!("- {diagnostic}"));
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use anyhow::Result;

    use crate::test_support::TestTempDir;

    use super::{collect_hook_health, format_report, HookDoctorReport, HookPathSource, Readiness};

    #[test]
    fn doctor_output_reports_healthy_state_when_all_required_hooks_exist() -> Result<()> {
        let temp_dir = TestTempDir::new("doctor-healthy")?;
        let hooks_dir = temp_dir.path().join("hooks");
        fs::create_dir_all(&hooks_dir)?;

        for hook in ["pre-commit", "commit-msg", "post-commit"] {
            let hook_path = hooks_dir.join(hook);
            fs::write(&hook_path, "#!/bin/sh\n")?;
            fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755))?;
        }

        let mut diagnostics = Vec::new();
        let hooks = collect_hook_health(&hooks_dir, &mut diagnostics);
        let report = HookDoctorReport {
            readiness: if diagnostics.is_empty() {
                Readiness::Ready
            } else {
                Readiness::NotReady
            },
            repository_root: Some(temp_dir.path().to_path_buf()),
            hook_path_source: HookPathSource::LocalConfig,
            hooks_directory: Some(hooks_dir),
            hooks,
            diagnostics,
        };

        let output = format_report(&report);
        assert!(output.contains("SCE doctor: ready"));
        assert!(output.contains("pre-commit: ok"));
        assert!(output.contains("commit-msg: ok"));
        assert!(output.contains("post-commit: ok"));
        assert!(output.contains("Diagnostics: none"));
        Ok(())
    }

    #[test]
    fn doctor_output_reports_missing_hook_state() -> Result<()> {
        let temp_dir = TestTempDir::new("doctor-missing")?;
        let hooks_dir = temp_dir.path().join("hooks");
        fs::create_dir_all(&hooks_dir)?;

        for hook in ["pre-commit", "post-commit"] {
            let hook_path = hooks_dir.join(hook);
            fs::write(&hook_path, "#!/bin/sh\n")?;
            fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755))?;
        }

        let mut diagnostics = Vec::new();
        let hooks = collect_hook_health(&hooks_dir, &mut diagnostics);
        let report = HookDoctorReport {
            readiness: if diagnostics.is_empty() {
                Readiness::Ready
            } else {
                Readiness::NotReady
            },
            repository_root: Some(temp_dir.path().to_path_buf()),
            hook_path_source: HookPathSource::GlobalConfig,
            hooks_directory: Some(hooks_dir),
            hooks,
            diagnostics,
        };

        let output = format_report(&report);
        assert!(output.contains("SCE doctor: not ready"));
        assert!(output.contains("commit-msg: missing"));
        assert!(output.contains("Missing required hook 'commit-msg'"));
        Ok(())
    }

    #[test]
    fn doctor_output_reports_misconfigured_hook_permissions() -> Result<()> {
        let temp_dir = TestTempDir::new("doctor-misconfigured")?;
        let hooks_dir = temp_dir.path().join("hooks");
        fs::create_dir_all(&hooks_dir)?;

        for hook in ["pre-commit", "commit-msg", "post-commit"] {
            let hook_path = hooks_dir.join(hook);
            fs::write(&hook_path, "#!/bin/sh\n")?;
            let mode = if hook == "post-commit" { 0o644 } else { 0o755 };
            fs::set_permissions(&hook_path, fs::Permissions::from_mode(mode))?;
        }

        let mut diagnostics = Vec::new();
        let hooks = collect_hook_health(&hooks_dir, &mut diagnostics);
        let report = HookDoctorReport {
            readiness: if diagnostics.is_empty() {
                Readiness::Ready
            } else {
                Readiness::NotReady
            },
            repository_root: Some(temp_dir.path().to_path_buf()),
            hook_path_source: HookPathSource::Default,
            hooks_directory: Some(hooks_dir),
            hooks,
            diagnostics,
        };

        let output = format_report(&report);
        assert!(output.contains("SCE doctor: not ready"));
        assert!(output.contains("post-commit: misconfigured"));
        assert!(output.contains("Hook 'post-commit' exists but is not executable"));
        Ok(())
    }
}
