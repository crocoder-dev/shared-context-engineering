#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::app::AppContext;
use crate::services::doctor::types::{
    DoctorFixResultRecord, DoctorProblem, FixResult, ProblemCategory, ProblemFixability,
    ProblemKind, ProblemSeverity,
};
use crate::services::lifecycle::{HealthProblem, ServiceLifecycle, SetupOutcome};
use crate::services::setup::{
    install_required_git_hooks, iter_required_hook_assets, RequiredHookInstallStatus,
    RequiredHooksInstallOutcome,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HooksLifecycle;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HookContentState {
    Current,
    Stale,
    Missing,
    Unknown,
}

impl ServiceLifecycle for HooksLifecycle {
    fn diagnose(&self, ctx: &AppContext) -> Vec<HealthProblem> {
        let repository_root = match ctx.repo_root() {
            Some(path) => path.to_path_buf(),
            None => {
                return vec![DoctorProblem {
                    kind: ProblemKind::NotInsideGitRepository,
                    category: ProblemCategory::RepositoryTargeting,
                    severity: ProblemSeverity::Error,
                    fixability: ProblemFixability::ManualOnly,
                    summary: String::from("The current directory is not inside a git repository."),
                    remediation: String::from(
                        "Run 'sce doctor' from inside the target repository working tree to inspect repo-scoped SCE hook health.",
                    ),
                    next_action: "manual_steps",
                }];
            }
        };

        diagnose_repository_hooks(&repository_root)
    }

    fn fix(&self, ctx: &AppContext, problems: &[HealthProblem]) -> Vec<DoctorFixResultRecord> {
        let should_fix_hooks = problems.iter().any(|problem| {
            problem.category == ProblemCategory::HookRollout
                && problem.fixability == ProblemFixability::AutoFixable
        });
        if !should_fix_hooks {
            return Vec::new();
        }

        let repository_root = match ctx.repo_root() {
            Some(path) => path.to_path_buf(),
            None => {
                return vec![DoctorFixResultRecord {
                    category: ProblemCategory::HookRollout,
                    outcome: FixResult::Failed,
                    detail: String::from(
                        "Automatic hook repair could not start because the repository root was not resolved from context",
                    ),
                }];
            }
        };

        match install_required_git_hooks(&repository_root) {
            Ok(outcome) => build_hook_fix_results(&outcome),
            Err(error) => vec![DoctorFixResultRecord {
                category: ProblemCategory::HookRollout,
                outcome: FixResult::Failed,
                detail: format!(
                    "Automatic hook repair failed while reusing the canonical setup flow: {error}"
                ),
            }],
        }
    }

    fn setup(&self, ctx: &AppContext) -> Result<SetupOutcome> {
        let repository_root = ctx
            .repo_root()
            .context("Hooks lifecycle setup requires a resolved repository root")?;
        let outcome = install_required_git_hooks(repository_root)
            .context("Hook lifecycle setup failed while installing required git hooks")?;

        Ok(SetupOutcome {
            required_hooks_install: Some(outcome),
            ..SetupOutcome::default()
        })
    }
}

pub fn diagnose_repository_hooks(repository_root: &Path) -> Vec<DoctorProblem> {
    let mut problems = Vec::new();

    if !is_git_available() {
        problems.push(DoctorProblem {
            kind: ProblemKind::GitUnavailable,
            category: ProblemCategory::RepositoryTargeting,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: String::from("Git is not available on this machine."),
            remediation: String::from("Install an accessible 'git' binary and ensure it is on PATH before rerunning 'sce doctor'."),
            next_action: "manual_steps",
        });
        return problems;
    }

    let detected_repository_root =
        run_git_command(repository_root, &["rev-parse", "--show-toplevel"]).map(PathBuf::from);
    let bare_repository = run_git_command(repository_root, &["rev-parse", "--is-bare-repository"])
        .is_some_and(|value| value == "true");

    if bare_repository {
        problems.push(DoctorProblem {
            kind: ProblemKind::BareRepository,
            category: ProblemCategory::RepositoryTargeting,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: String::from(
                "The current repository is bare and does not support local SCE hook rollout.",
            ),
            remediation: String::from("Run 'sce doctor' from a non-bare working tree clone to inspect repo-scoped SCE hook health."),
            next_action: "manual_steps",
        });
        return problems;
    }

    let Some(resolved_root) = detected_repository_root else {
        problems.push(DoctorProblem {
            kind: ProblemKind::NotInsideGitRepository,
            category: ProblemCategory::RepositoryTargeting,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: String::from("The current directory is not inside a git repository."),
            remediation: String::from("Run 'sce doctor' from inside the target repository working tree to inspect repo-scoped SCE hook health."),
            next_action: "manual_steps",
        });
        return problems;
    };

    let hooks_directory = run_git_command(&resolved_root, &["rev-parse", "--git-path", "hooks"])
        .map(|value| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                resolved_root.join(path)
            }
        });

    let Some(hooks_directory) = hooks_directory else {
        problems.push(DoctorProblem {
            kind: ProblemKind::UnableToResolveGitHooksDirectory,
            category: ProblemCategory::RepositoryTargeting,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: String::from("Unable to resolve git hooks directory."),
            remediation: String::from("Verify that git repository inspection succeeds and rerun 'sce doctor' inside a non-bare git repository."),
            next_action: "manual_steps",
        });
        return problems;
    };

    collect_hook_health_problems(&hooks_directory, &mut problems);
    problems
}

fn collect_hook_health_problems(directory: &Path, problems: &mut Vec<DoctorProblem>) {
    if !directory.exists() {
        problems.push(DoctorProblem {
            kind: ProblemKind::HooksDirectoryMissing,
            category: ProblemCategory::HookRollout,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::AutoFixable,
            summary: format!("Hooks directory '{}' does not exist.", directory.display()),
            remediation: format!(
                "Run 'sce doctor --fix' to install the canonical SCE-managed hooks into '{}', or run 'sce setup --hooks' directly.",
                directory.display()
            ),
            next_action: "doctor_fix",
        });
    } else if !directory.is_dir() {
        problems.push(DoctorProblem {
            kind: ProblemKind::HooksPathNotDirectory,
            category: ProblemCategory::HookRollout,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!("Hooks path '{}' is not a directory.", directory.display()),
            remediation: format!(
                "Replace '{}' with a writable hooks directory, then rerun 'sce doctor' or 'sce setup --hooks'.",
                directory.display()
            ),
            next_action: "manual_steps",
        });
    }

    for hook_asset in iter_required_hook_assets() {
        let hook_name = hook_asset.relative_path;
        let hook_path = directory.join(hook_name);
        let metadata = fs::metadata(&hook_path).ok();
        let exists = metadata.is_some();
        let executable = metadata
            .as_ref()
            .is_some_and(|entry| entry.is_file() && is_executable(entry));
        let content_state = inspect_hook_content_state(hook_name, &hook_path, exists, problems);

        if !exists {
            problems.push(DoctorProblem {
                kind: ProblemKind::RequiredHookMissing,
                category: ProblemCategory::HookRollout,
                severity: ProblemSeverity::Error,
                fixability: ProblemFixability::AutoFixable,
                summary: format!(
                    "Missing required hook '{}' at '{}'.",
                    hook_name,
                    hook_path.display()
                ),
                remediation: format!(
                    "Run 'sce doctor --fix' to install the canonical '{hook_name}' hook, or run 'sce setup --hooks' directly."
                ),
                next_action: "doctor_fix",
            });
        } else if !executable {
            problems.push(DoctorProblem {
                kind: ProblemKind::HookNotExecutable,
                category: ProblemCategory::HookRollout,
                severity: ProblemSeverity::Error,
                fixability: ProblemFixability::AutoFixable,
                summary: format!("Hook '{hook_name}' exists but is not executable."),
                remediation: format!(
                    "Run 'sce doctor --fix' to restore the canonical executable hook, or run 'sce setup --hooks' / 'chmod +x {}' manually.",
                    hook_path.display()
                ),
                next_action: "doctor_fix",
            });
        }

        if content_state == HookContentState::Stale {
            problems.push(DoctorProblem {
                kind: ProblemKind::HookContentStale,
                category: ProblemCategory::HookRollout,
                severity: ProblemSeverity::Error,
                fixability: ProblemFixability::AutoFixable,
                summary: format!(
                    "Hook '{}' at '{}' differs from the canonical SCE-managed content.",
                    hook_name,
                    hook_path.display()
                ),
                remediation: format!(
                    "Run 'sce doctor --fix' to reinstall the canonical '{hook_name}' hook content, or run 'sce setup --hooks' directly."
                ),
                next_action: "doctor_fix",
            });
        }
    }
}

fn inspect_hook_content_state(
    hook_name: &str,
    hook_path: &Path,
    exists: bool,
    problems: &mut Vec<DoctorProblem>,
) -> HookContentState {
    if !exists {
        return HookContentState::Missing;
    }

    let Some(expected_hook) =
        iter_required_hook_assets().find(|asset| asset.relative_path == hook_name)
    else {
        return HookContentState::Unknown;
    };

    match fs::read(hook_path) {
        Ok(bytes) => {
            if bytes == expected_hook.bytes {
                HookContentState::Current
            } else {
                HookContentState::Stale
            }
        }
        Err(error) => {
            problems.push(DoctorProblem {
                kind: ProblemKind::HookReadFailed,
                category: ProblemCategory::FilesystemPermissions,
                severity: ProblemSeverity::Error,
                fixability: ProblemFixability::ManualOnly,
                summary: format!(
                    "Unable to read hook '{}' at '{}': {error}",
                    hook_name,
                    hook_path.display()
                ),
                remediation: format!(
                    "Verify that '{}' is readable before rerunning 'sce doctor'.",
                    hook_path.display()
                ),
                next_action: "manual_steps",
            });
            HookContentState::Unknown
        }
    }
}

fn build_hook_fix_results(outcome: &RequiredHooksInstallOutcome) -> Vec<DoctorFixResultRecord> {
    outcome
        .hook_results
        .iter()
        .map(|hook_result| DoctorFixResultRecord {
            category: ProblemCategory::HookRollout,
            outcome: match hook_result.status {
                RequiredHookInstallStatus::Installed | RequiredHookInstallStatus::Updated => {
                    FixResult::Fixed
                }
                RequiredHookInstallStatus::Skipped => FixResult::Skipped,
            },
            detail: format!(
                "Hook '{}' {} at '{}'.",
                hook_result.hook_name,
                match hook_result.status {
                    RequiredHookInstallStatus::Installed => "installed",
                    RequiredHookInstallStatus::Updated => "updated",
                    RequiredHookInstallStatus::Skipped => "already matched canonical content",
                },
                hook_result.hook_path.display()
            ),
        })
        .collect()
}

fn is_git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
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

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(metadata: &fs::Metadata) -> bool {
    metadata.is_file()
}
