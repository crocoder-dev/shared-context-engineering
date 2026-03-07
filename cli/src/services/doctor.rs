use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use lexopt::Arg;
use lexopt::ValueExt;
use serde_json::json;

use crate::services::output_format::OutputFormat;

pub const NAME: &str = "doctor";

const REQUIRED_HOOKS: [&str; 3] = ["pre-commit", "commit-msg", "post-commit"];

pub type DoctorFormat = OutputFormat;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DoctorRequest {
    pub format: DoctorFormat,
}

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

pub fn run_doctor(request: DoctorRequest) -> Result<String> {
    let repository_root =
        std::env::current_dir().context("Failed to determine current directory")?;
    let report = build_report(&repository_root);
    render_report(request, &report)
}

pub fn doctor_usage_text() -> &'static str {
    "Usage:\n  sce doctor [--format <text|json>]\n\nExamples:\n  sce doctor\n  sce doctor --format json\n  sce doctor | rg 'not ready'"
}

pub fn parse_doctor_request(args: Vec<String>) -> Result<DoctorRequest> {
    let mut parser = lexopt::Parser::from_args(args);
    let mut format = DoctorFormat::Text;

    while let Some(arg) = parser.next()? {
        match arg {
            Arg::Long("format") => {
                let value = parser
                    .value()
                    .context("Option '--format' requires a value")?;
                let raw = value.string()?;
                format = DoctorFormat::parse(&raw, "sce doctor --help")?;
            }
            Arg::Long("help") | Arg::Short('h') => {
                bail!("Use 'sce doctor --help' for doctor usage.");
            }
            Arg::Long(option) => {
                bail!(
                    "Unknown doctor option '--{}'. Run 'sce doctor --help' to see valid usage.",
                    option
                );
            }
            Arg::Short(option) => {
                bail!(
                    "Unknown doctor option '-{}'. Run 'sce doctor --help' to see valid usage.",
                    option
                );
            }
            Arg::Value(value) => {
                bail!(
                    "Unexpected doctor argument '{}'. Run 'sce doctor --help' to see valid usage.",
                    value.string()?
                );
            }
        }
    }

    Ok(DoctorRequest { format })
}

fn build_report(repository_root: &Path) -> HookDoctorReport {
    let detected_repository_root =
        run_git_command(repository_root, &["rev-parse", "--show-toplevel"]).map(PathBuf::from);
    let hooks_directory = detected_repository_root.as_ref().and_then(|resolved_root| {
        run_git_command(resolved_root, &["rev-parse", "--git-path", "hooks"]).map(|value| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                resolved_root.join(path)
            }
        })
    });

    let local_hooks_path = run_git_command(
        repository_root,
        &["config", "--local", "--get", "core.hooksPath"],
    );
    let global_hooks_path = run_git_command(
        repository_root,
        &["config", "--global", "--get", "core.hooksPath"],
    );

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
        repository_root: detected_repository_root,
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

fn run_git_command(repository_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository_root)
        .output()
        .ok()?;
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

fn render_report(request: DoctorRequest, report: &HookDoctorReport) -> Result<String> {
    match request.format {
        DoctorFormat::Text => Ok(format_report(report)),
        DoctorFormat::Json => render_report_json(report),
    }
}

fn render_report_json(report: &HookDoctorReport) -> Result<String> {
    let hooks = report
        .hooks
        .iter()
        .map(|hook| {
            json!({
                "name": hook.name,
                "path": hook.path.display().to_string(),
                "exists": hook.exists,
                "executable": hook.executable,
                "state": hook_state(hook),
            })
        })
        .collect::<Vec<_>>();

    let payload = json!({
        "status": "ok",
        "command": NAME,
        "readiness": match report.readiness {
            Readiness::Ready => "ready",
            Readiness::NotReady => "not_ready",
        },
        "hook_path_source": match report.hook_path_source {
            HookPathSource::Default => "default",
            HookPathSource::LocalConfig => "local_config",
            HookPathSource::GlobalConfig => "global_config",
        },
        "repository_root": report
            .repository_root
            .as_ref()
            .map(|path| path.display().to_string()),
        "hooks_directory": report
            .hooks_directory
            .as_ref()
            .map(|path| path.display().to_string()),
        "hooks": hooks,
        "diagnostics": report.diagnostics,
    });

    serde_json::to_string_pretty(&payload).context("failed to serialize doctor report to JSON")
}

fn hook_state(hook: &HookFileHealth) -> &'static str {
    if hook.exists && hook.executable {
        "ok"
    } else if !hook.exists {
        "missing"
    } else {
        "misconfigured"
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::Command;

    use anyhow::Result;
    use serde_json::Value;

    use crate::services::setup::install_required_git_hooks;
    use crate::test_support::TestTempDir;

    use super::{
        build_report, collect_hook_health, format_report, parse_doctor_request, render_report,
        DoctorFormat, DoctorRequest, HookDoctorReport, HookPathSource, Readiness, NAME,
    };

    #[test]
    fn parse_defaults_to_text_format() {
        let request = parse_doctor_request(vec![]).expect("doctor request should parse");
        assert_eq!(request.format, DoctorFormat::Text);
    }

    #[test]
    fn parse_accepts_json_format() {
        let request = parse_doctor_request(vec!["--format".to_string(), "json".to_string()])
            .expect("doctor request should parse");
        assert_eq!(request.format, DoctorFormat::Json);
    }

    #[test]
    fn parse_rejects_invalid_format_with_help_guidance() {
        let error = parse_doctor_request(vec!["--format".to_string(), "yaml".to_string()])
            .expect_err("invalid doctor format should fail");
        assert_eq!(
            error.to_string(),
            "Invalid --format value 'yaml'. Valid values: text, json. Run 'sce doctor --help' to see valid usage."
        );
    }

    #[test]
    #[cfg(unix)]
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
    #[cfg(unix)]
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
    #[cfg(unix)]
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

    #[test]
    fn doctor_reports_ready_after_setup_hook_install() -> Result<()> {
        let temp_dir = TestTempDir::new("doctor-ready-after-setup")?;
        init_git_repo(temp_dir.path())?;

        install_required_git_hooks(temp_dir.path())?;

        let output = format_report(&build_report(temp_dir.path()));
        assert!(output.contains("SCE doctor: ready"));
        assert!(output.contains("pre-commit: ok"));
        assert!(output.contains("commit-msg: ok"));
        assert!(output.contains("post-commit: ok"));
        Ok(())
    }

    #[test]
    fn doctor_reports_ready_for_custom_repo_hooks_path_after_setup() -> Result<()> {
        let temp_dir = TestTempDir::new("doctor-ready-custom-hooks-path")?;
        init_git_repo(temp_dir.path())?;
        run_git_in_repo(temp_dir.path(), &["config", "core.hooksPath", ".githooks"])?;

        install_required_git_hooks(temp_dir.path())?;

        let output = format_report(&build_report(temp_dir.path()));
        assert!(output.contains("SCE doctor: ready"));
        assert!(output.contains("Hooks path source: per-repo core.hooksPath"));
        assert!(output.contains(".githooks"));
        Ok(())
    }

    #[test]
    fn render_json_includes_stable_fields() -> Result<()> {
        let temp_dir = TestTempDir::new("doctor-json-shape")?;
        let report = build_report(temp_dir.path());

        let output = render_report(
            DoctorRequest {
                format: DoctorFormat::Json,
            },
            &report,
        )?;

        let parsed: Value = serde_json::from_str(&output)?;
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["command"], NAME);
        assert!(parsed["readiness"].as_str().is_some());
        assert!(parsed["hook_path_source"].as_str().is_some());
        assert!(parsed["hooks"].is_array());
        assert!(parsed["diagnostics"].is_array());
        Ok(())
    }

    #[test]
    fn render_json_is_deterministic_for_same_report() -> Result<()> {
        let temp_dir = TestTempDir::new("doctor-json-determinism")?;
        let report = build_report(temp_dir.path());

        let first = render_report(
            DoctorRequest {
                format: DoctorFormat::Json,
            },
            &report,
        )?;
        let second = render_report(
            DoctorRequest {
                format: DoctorFormat::Json,
            },
            &report,
        )?;

        assert_eq!(first, second);
        Ok(())
    }

    fn init_git_repo(repository_root: &Path) -> Result<()> {
        run_git_in_repo(repository_root, &["init", "-q"])
    }

    fn run_git_in_repo(repository_root: &Path, args: &[&str]) -> Result<()> {
        let status = Command::new("git")
            .args(args)
            .current_dir(repository_root)
            .status()?;
        if !status.success() {
            anyhow::bail!("git command failed for test repository");
        }

        Ok(())
    }
}
