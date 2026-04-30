#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::app::AppContext;
use crate::services::lifecycle::{
    FixOutcome, FixResultRecord, HealthCategory, HealthFixability, HealthProblem,
    HealthProblemKind, HealthSeverity, RequiredHookInstallStatus, RequiredHooksInstallOutcome,
    ServiceLifecycle, SetupOutcome,
};
use crate::services::setup::{
    install_required_git_hooks, iter_required_hook_assets,
    RequiredHookInstallStatus as SetupRequiredHookInstallStatus,
    RequiredHooksInstallOutcome as SetupRequiredHooksInstallOutcome,
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
                return vec![HealthProblem {
                    kind: HealthProblemKind::NotInsideGitRepository,
                    category: HealthCategory::RepositoryTargeting,
                    severity: HealthSeverity::Error,
                    fixability: HealthFixability::ManualOnly,
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

    fn fix(&self, ctx: &AppContext, problems: &[HealthProblem]) -> Vec<FixResultRecord> {
        let should_fix_hooks = problems.iter().any(|problem| {
            problem.category == HealthCategory::HookRollout
                && problem.fixability == HealthFixability::AutoFixable
        });
        if !should_fix_hooks {
            return Vec::new();
        }

        let repository_root = match ctx.repo_root() {
            Some(path) => path.to_path_buf(),
            None => {
                return vec![FixResultRecord {
                    category: HealthCategory::HookRollout,
                    outcome: FixOutcome::Failed,
                    detail: String::from(
                        "Automatic hook repair could not start because the repository root was not resolved from context",
                    ),
                }];
            }
        };

        match install_required_git_hooks(&repository_root) {
            Ok(outcome) => build_hook_fix_results(&outcome),
            Err(error) => vec![FixResultRecord {
                category: HealthCategory::HookRollout,
                outcome: FixOutcome::Failed,
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
            required_hooks_install: Some(required_hooks_outcome_from_setup(outcome)),
        })
    }
}

pub fn diagnose_repository_hooks(repository_root: &Path) -> Vec<HealthProblem> {
    let mut problems = Vec::new();

    if !is_git_available() {
        problems.push(HealthProblem {
            kind: HealthProblemKind::GitUnavailable,
            category: HealthCategory::RepositoryTargeting,
            severity: HealthSeverity::Error,
            fixability: HealthFixability::ManualOnly,
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
        problems.push(HealthProblem {
            kind: HealthProblemKind::BareRepository,
            category: HealthCategory::RepositoryTargeting,
            severity: HealthSeverity::Error,
            fixability: HealthFixability::ManualOnly,
            summary: String::from(
                "The current repository is bare and does not support local SCE hook rollout.",
            ),
            remediation: String::from("Run 'sce doctor' from a non-bare working tree clone to inspect repo-scoped SCE hook health."),
            next_action: "manual_steps",
        });
        return problems;
    }

    let Some(resolved_root) = detected_repository_root else {
        problems.push(HealthProblem {
            kind: HealthProblemKind::NotInsideGitRepository,
            category: HealthCategory::RepositoryTargeting,
            severity: HealthSeverity::Error,
            fixability: HealthFixability::ManualOnly,
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
        problems.push(HealthProblem {
            kind: HealthProblemKind::UnableToResolveGitHooksDirectory,
            category: HealthCategory::RepositoryTargeting,
            severity: HealthSeverity::Error,
            fixability: HealthFixability::ManualOnly,
            summary: String::from("Unable to resolve git hooks directory."),
            remediation: String::from("Verify that git repository inspection succeeds and rerun 'sce doctor' inside a non-bare git repository."),
            next_action: "manual_steps",
        });
        return problems;
    };

    collect_hook_health_problems(&hooks_directory, &mut problems);
    problems
}

fn collect_hook_health_problems(directory: &Path, problems: &mut Vec<HealthProblem>) {
    if !directory.exists() {
        problems.push(HealthProblem {
            kind: HealthProblemKind::HooksDirectoryMissing,
            category: HealthCategory::HookRollout,
            severity: HealthSeverity::Error,
            fixability: HealthFixability::AutoFixable,
            summary: format!("Hooks directory '{}' does not exist.", directory.display()),
            remediation: format!(
                "Run 'sce doctor --fix' to install the canonical SCE-managed hooks into '{}', or run 'sce setup --hooks' directly.",
                directory.display()
            ),
            next_action: "doctor_fix",
        });
    } else if !directory.is_dir() {
        problems.push(HealthProblem {
            kind: HealthProblemKind::HooksPathNotDirectory,
            category: HealthCategory::HookRollout,
            severity: HealthSeverity::Error,
            fixability: HealthFixability::ManualOnly,
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
            problems.push(HealthProblem {
                kind: HealthProblemKind::RequiredHookMissing,
                category: HealthCategory::HookRollout,
                severity: HealthSeverity::Error,
                fixability: HealthFixability::AutoFixable,
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
            problems.push(HealthProblem {
                kind: HealthProblemKind::HookNotExecutable,
                category: HealthCategory::HookRollout,
                severity: HealthSeverity::Error,
                fixability: HealthFixability::AutoFixable,
                summary: format!("Hook '{hook_name}' exists but is not executable."),
                remediation: format!(
                    "Run 'sce doctor --fix' to restore the canonical executable hook, or run 'sce setup --hooks' / 'chmod +x {}' manually.",
                    hook_path.display()
                ),
                next_action: "doctor_fix",
            });
        }

        if content_state == HookContentState::Stale {
            problems.push(HealthProblem {
                kind: HealthProblemKind::HookContentStale,
                category: HealthCategory::HookRollout,
                severity: HealthSeverity::Error,
                fixability: HealthFixability::AutoFixable,
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
    problems: &mut Vec<HealthProblem>,
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
            problems.push(HealthProblem {
                kind: HealthProblemKind::HookReadFailed,
                category: HealthCategory::FilesystemPermissions,
                severity: HealthSeverity::Error,
                fixability: HealthFixability::ManualOnly,
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

fn build_hook_fix_results(outcome: &SetupRequiredHooksInstallOutcome) -> Vec<FixResultRecord> {
    outcome
        .hook_results
        .iter()
        .map(|hook_result| FixResultRecord {
            category: HealthCategory::HookRollout,
            outcome: match hook_result.status {
                SetupRequiredHookInstallStatus::Installed
                | SetupRequiredHookInstallStatus::Updated => FixOutcome::Fixed,
                SetupRequiredHookInstallStatus::Skipped => FixOutcome::Skipped,
            },
            detail: format!(
                "Hook '{}' {} at '{}'.",
                hook_result.hook_name,
                match hook_result.status {
                    SetupRequiredHookInstallStatus::Installed => "installed",
                    SetupRequiredHookInstallStatus::Updated => "updated",
                    SetupRequiredHookInstallStatus::Skipped => "already matched canonical content",
                },
                hook_result.hook_path.display()
            ),
        })
        .collect()
}

fn required_hooks_outcome_from_setup(
    outcome: SetupRequiredHooksInstallOutcome,
) -> RequiredHooksInstallOutcome {
    RequiredHooksInstallOutcome {
        repository_root: outcome.repository_root,
        hooks_directory: outcome.hooks_directory,
        hook_results: outcome
            .hook_results
            .into_iter()
            .map(
                |result| crate::services::lifecycle::RequiredHookInstallResult {
                    hook_name: result.hook_name,
                    hook_path: result.hook_path,
                    status: match result.status {
                        SetupRequiredHookInstallStatus::Installed => {
                            RequiredHookInstallStatus::Installed
                        }
                        SetupRequiredHookInstallStatus::Updated => {
                            RequiredHookInstallStatus::Updated
                        }
                        SetupRequiredHookInstallStatus::Skipped => {
                            RequiredHookInstallStatus::Skipped
                        }
                    },
                },
            )
            .collect(),
    }
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
