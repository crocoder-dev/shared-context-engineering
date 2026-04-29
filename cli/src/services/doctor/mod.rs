use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::services::default_paths::{resolve_sce_default_locations, resolve_state_data_root};
use crate::services::output_format::OutputFormat;
use crate::services::setup::{install_required_git_hooks, RequiredHooksInstallOutcome};

mod fixes;
mod inspect;
mod render;
pub(crate) mod types;

pub mod command;

use fixes::{build_manual_fix_results, run_auto_fixes};
use inspect::build_report_with_dependencies;
use render::render_report;
use types::{DoctorFixResultRecord, HookDoctorReport};

pub const NAME: &str = "doctor";

pub(super) const REQUIRED_HOOKS: [&str; 3] = ["pre-commit", "commit-msg", "post-commit"];

pub type DoctorFormat = OutputFormat;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DoctorMode {
    Diagnose,
    Fix,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DoctorRequest {
    pub mode: DoctorMode,
    pub format: DoctorFormat,
}

struct DoctorDependencies<'a> {
    run_git_command: &'a dyn Fn(&Path, &[&str]) -> Option<String>,
    check_git_available: &'a dyn Fn() -> bool,
    resolve_state_root: &'a dyn Fn() -> Result<PathBuf>,
    resolve_global_config_path: &'a dyn Fn() -> Result<PathBuf>,
    validate_config_file: &'a dyn Fn(&Path) -> Result<()>,
    install_required_git_hooks: &'a dyn Fn(&Path) -> Result<RequiredHooksInstallOutcome>,
}

struct DoctorExecution {
    report: HookDoctorReport,
    fix_results: Vec<DoctorFixResultRecord>,
}

pub fn run_doctor(request: DoctorRequest) -> Result<String> {
    let repository_root =
        std::env::current_dir().context("Failed to determine current directory")?;
    let execution = execute_doctor(request, &repository_root);
    render_report(request, &execution)
}

fn execute_doctor(request: DoctorRequest, repository_root: &Path) -> DoctorExecution {
    execute_doctor_with_dependencies(
        request,
        repository_root,
        &DoctorDependencies {
            run_git_command: &run_git_command,
            check_git_available: &is_git_available,
            resolve_state_root: &resolve_state_data_root,
            resolve_global_config_path: &|| {
                Ok(resolve_sce_default_locations()?.global_config_file())
            },
            validate_config_file: &crate::services::config::validate_config_file,
            install_required_git_hooks: &install_required_git_hooks,
        },
    )
}

fn execute_doctor_with_dependencies(
    request: DoctorRequest,
    repository_root: &Path,
    dependencies: &DoctorDependencies<'_>,
) -> DoctorExecution {
    let initial_report =
        build_report_with_dependencies(request.mode, repository_root, dependencies);

    if request.mode != DoctorMode::Fix {
        return DoctorExecution {
            report: initial_report,
            fix_results: Vec::new(),
        };
    }

    let mut fix_results = run_auto_fixes(&initial_report, dependencies);
    let final_report = build_report_with_dependencies(request.mode, repository_root, dependencies);
    fix_results.extend(build_manual_fix_results(&final_report));

    DoctorExecution {
        report: final_report,
        fix_results,
    }
}

fn is_git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
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
