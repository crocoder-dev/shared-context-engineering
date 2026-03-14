use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
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
struct FileLocationHealth {
    label: &'static str,
    path: PathBuf,
    exists: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HookDoctorReport {
    readiness: Readiness,
    repository_root: Option<PathBuf>,
    hook_path_source: HookPathSource,
    hooks_directory: Option<PathBuf>,
    config_locations: Vec<FileLocationHealth>,
    agent_trace_local_db: Option<FileLocationHealth>,
    hooks: Vec<HookFileHealth>,
    diagnostics: Vec<String>,
}

pub fn run_doctor(request: DoctorRequest) -> Result<String> {
    let repository_root =
        std::env::current_dir().context("Failed to determine current directory")?;
    let report = build_report(&repository_root);
    render_report(request, &report)
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
    let config_locations = collect_config_locations(repository_root, &mut diagnostics);
    let agent_trace_local_db = collect_agent_trace_local_db_location(&mut diagnostics);
    let hooks = if let Some(directory) = hooks_directory.as_deref() {
        collect_hook_health(directory, &mut diagnostics)
    } else {
        diagnostics.push(
            "Unable to resolve git hooks directory. Run this command inside a git repository."
                .to_string(),
        );
        Vec::new()
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
        config_locations,
        agent_trace_local_db,
        hooks,
        diagnostics,
    }
}

fn collect_config_locations(
    repository_root: &Path,
    diagnostics: &mut Vec<String>,
) -> Vec<FileLocationHealth> {
    let mut locations = Vec::new();

    match crate::services::local_db::resolve_state_data_root() {
        Ok(state_root) => {
            let global_path = state_root.join("sce").join("config.json");
            locations.push(FileLocationHealth {
                label: "Global config",
                exists: global_path.exists(),
                path: global_path,
            });
        }
        Err(error) => diagnostics.push(format!(
            "Unable to resolve expected global config path: {error}"
        )),
    }

    let local_path = repository_root.join(".sce").join("config.json");
    locations.push(FileLocationHealth {
        label: "Local config",
        exists: local_path.exists(),
        path: local_path,
    });

    locations
}

fn collect_agent_trace_local_db_location(
    diagnostics: &mut Vec<String>,
) -> Option<FileLocationHealth> {
    match crate::services::local_db::resolve_agent_trace_local_db_path() {
        Ok(path) => Some(FileLocationHealth {
            label: "Agent Trace local DB",
            exists: path.exists(),
            path,
        }),
        Err(error) => {
            diagnostics.push(format!(
                "Unable to resolve expected Agent Trace local DB path: {error}"
            ));
            None
        }
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
        report.repository_root.as_ref().map_or_else(
            || "(not detected)".to_string(),
            |path| path.display().to_string()
        )
    ));

    lines.push(format!(
        "Effective hooks directory: {}",
        report.hooks_directory.as_ref().map_or_else(
            || "(not detected)".to_string(),
            |path| path.display().to_string()
        )
    ));

    lines.push("Config files:".to_string());
    for location in &report.config_locations {
        lines.push(format!(
            "- {}: {} ({})",
            location.label,
            if location.exists {
                "present"
            } else {
                "expected"
            },
            location.path.display()
        ));
    }

    lines.push(format!(
        "Agent Trace local DB: {}",
        report.agent_trace_local_db.as_ref().map_or_else(
            || "(not detected)".to_string(),
            |location| format!(
                "{} ({})",
                if location.exists {
                    "present"
                } else {
                    "expected"
                },
                location.path.display()
            )
        )
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

    let config_paths = report
        .config_locations
        .iter()
        .map(|location| {
            json!({
                "label": location.label,
                "path": location.path.display().to_string(),
                "exists": location.exists,
                "state": if location.exists { "present" } else { "expected" },
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
        "config_paths": config_paths,
        "agent_trace_local_db": report.agent_trace_local_db.as_ref().map(|location| json!({
            "label": location.label,
            "path": location.path.display().to_string(),
            "exists": location.exists,
            "state": if location.exists { "present" } else { "expected" },
        })),
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
    use anyhow::Result;
    use serde_json::Value;

    use super::{render_report, DoctorFormat, DoctorRequest, NAME};

    #[test]
    fn render_json_includes_stable_fields_without_filesystem() -> Result<()> {
        let output = render_report(
            DoctorRequest {
                format: DoctorFormat::Json,
            },
            &super::build_report(std::path::Path::new("/nonexistent")),
        )?;

        let parsed: Value = serde_json::from_str(&output)?;
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["command"], NAME);
        assert!(parsed["readiness"].as_str().is_some());
        assert!(parsed["hook_path_source"].as_str().is_some());
        assert!(parsed["config_paths"].is_array());
        assert!(parsed["hooks"].is_array());
        assert!(parsed["diagnostics"].is_array());
        Ok(())
    }
}
