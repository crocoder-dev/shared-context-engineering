use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde_json::json;

use crate::services::default_paths::{
    hook_dir, opencode_asset, resolve_sce_default_locations, InstallTargetPaths, RepoPaths,
};
use crate::services::output_format::OutputFormat;
use crate::services::setup::{
    install_required_git_hooks, iter_required_hook_assets, RequiredHookInstallStatus,
    RequiredHooksInstallOutcome,
};
use crate::services::style::{heading, label, success, value};

pub const NAME: &str = "doctor";

const REQUIRED_HOOKS: [&str; 3] = [
    hook_dir::PRE_COMMIT,
    hook_dir::COMMIT_MSG,
    hook_dir::POST_COMMIT,
];

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
    content_state: HookContentState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HookContentState {
    Current,
    Stale,
    Missing,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileLocationHealth {
    label: &'static str,
    path: PathBuf,
    state: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GlobalStateHealth {
    state_root: Option<FileLocationHealth>,
    config_locations: Vec<FileLocationHealth>,
    agent_trace_local_db: Option<FileLocationHealth>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HookDoctorReport {
    mode: DoctorMode,
    readiness: Readiness,
    state_root: Option<FileLocationHealth>,
    repository_root: Option<PathBuf>,
    hook_path_source: HookPathSource,
    hooks_directory: Option<PathBuf>,
    config_locations: Vec<FileLocationHealth>,
    agent_trace_local_db: Option<FileLocationHealth>,
    hooks: Vec<HookFileHealth>,
    problems: Vec<DoctorProblem>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProblemCategory {
    GlobalState,
    RepositoryTargeting,
    HookRollout,
    RepoAssets,
    FilesystemPermissions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProblemSeverity {
    Error,
    Warning,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProblemFixability {
    AutoFixable,
    ManualOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FixResult {
    Fixed,
    Skipped,
    Manual,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DoctorProblem {
    category: ProblemCategory,
    severity: ProblemSeverity,
    fixability: ProblemFixability,
    summary: String,
    remediation: String,
    next_action: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DoctorFixResultRecord {
    category: ProblemCategory,
    outcome: FixResult,
    detail: String,
}

struct DoctorDependencies<'a> {
    run_git_command: &'a dyn Fn(&Path, &[&str]) -> Option<String>,
    check_git_available: &'a dyn Fn() -> bool,
    resolve_state_root: &'a dyn Fn() -> Result<PathBuf>,
    resolve_global_config_path: &'a dyn Fn() -> Result<PathBuf>,
    resolve_agent_trace_local_db_path: &'a dyn Fn() -> Result<PathBuf>,
    validate_config_file: &'a dyn Fn(&Path) -> Result<()>,
    check_agent_trace_local_db_health: &'a dyn Fn(&Path) -> Result<()>,
    install_required_git_hooks: &'a dyn Fn(&Path) -> Result<RequiredHooksInstallOutcome>,
    create_directory_all: &'a dyn Fn(&Path) -> Result<()>,
}

struct DoctorExecution {
    report: HookDoctorReport,
    fix_results: Vec<DoctorFixResultRecord>,
}

enum DirectoryWriteReadiness {
    Ready,
    Missing,
    NotDirectory,
    ReadOnly,
    Unknown(std::io::Error),
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
            resolve_state_root: &crate::services::local_db::resolve_state_data_root,
            resolve_global_config_path: &|| {
                Ok(resolve_sce_default_locations()?.global_config_file())
            },
            resolve_agent_trace_local_db_path:
                &crate::services::local_db::resolve_agent_trace_local_db_path,
            validate_config_file: &crate::services::config::validate_config_file,
            check_agent_trace_local_db_health:
                &crate::services::local_db::check_agent_trace_local_db_health_blocking,
            install_required_git_hooks: &install_required_git_hooks,
            create_directory_all: &create_directory_all,
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

#[allow(clippy::too_many_lines)]
fn build_report_with_dependencies(
    mode: DoctorMode,
    repository_root: &Path,
    dependencies: &DoctorDependencies<'_>,
) -> HookDoctorReport {
    let mut problems = Vec::new();
    let global_state = collect_global_state_health(repository_root, &mut problems, dependencies);
    let git_available = (dependencies.check_git_available)();

    let detected_repository_root = if git_available {
        (dependencies.run_git_command)(repository_root, &["rev-parse", "--show-toplevel"])
            .map(PathBuf::from)
    } else {
        None
    };

    let bare_repository = if git_available {
        (dependencies.run_git_command)(repository_root, &["rev-parse", "--is-bare-repository"])
            .is_some_and(|value| value == "true")
    } else {
        false
    };

    let local_hooks_path = if git_available {
        (dependencies.run_git_command)(
            repository_root,
            &["config", "--local", "--get", "core.hooksPath"],
        )
    } else {
        None
    };
    let global_hooks_path = if git_available {
        (dependencies.run_git_command)(
            repository_root,
            &["config", "--global", "--get", "core.hooksPath"],
        )
    } else {
        None
    };

    let hook_path_source = if local_hooks_path.is_some() {
        HookPathSource::LocalConfig
    } else if global_hooks_path.is_some() {
        HookPathSource::GlobalConfig
    } else {
        HookPathSource::Default
    };

    let hooks_directory = detected_repository_root.as_ref().and_then(|resolved_root| {
        (dependencies.run_git_command)(resolved_root, &["rev-parse", "--git-path", "hooks"]).map(
            |value| {
                let path = PathBuf::from(value);
                if path.is_absolute() {
                    path
                } else {
                    resolved_root.join(path)
                }
            },
        )
    });

    let hooks = if !git_available {
        problems.push(DoctorProblem {
            category: ProblemCategory::RepositoryTargeting,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: String::from("Git is not available on this machine."),
            remediation: String::from("Install an accessible 'git' binary and ensure it is on PATH before rerunning 'sce doctor'."),
            next_action: "manual_steps",
        });
        Vec::new()
    } else if bare_repository {
        problems.push(DoctorProblem {
            category: ProblemCategory::RepositoryTargeting,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: String::from("The current repository is bare and does not support local SCE hook rollout."),
            remediation: String::from("Run 'sce doctor' from a non-bare working tree clone to inspect repo-scoped SCE hook health."),
            next_action: "manual_steps",
        });
        Vec::new()
    } else if detected_repository_root.is_none() {
        problems.push(DoctorProblem {
            category: ProblemCategory::RepositoryTargeting,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: String::from("The current directory is not inside a git repository."),
            remediation: String::from("Run 'sce doctor' from inside the target repository working tree to inspect repo-scoped SCE hook health."),
            next_action: "manual_steps",
        });
        Vec::new()
    } else if let Some(directory) = hooks_directory.as_deref() {
        collect_hook_health(directory, &mut problems)
    } else {
        problems.push(DoctorProblem {
            category: ProblemCategory::RepositoryTargeting,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: String::from("Unable to resolve git hooks directory."),
            remediation: String::from("Verify that git repository inspection succeeds and rerun 'sce doctor' inside a non-bare git repository."),
            next_action: "manual_steps",
        });
        Vec::new()
    };

    if git_available && !bare_repository {
        if let Some(resolved_root) = detected_repository_root.as_deref() {
            inspect_opencode_plugin_health(resolved_root, &mut problems);
        }
    }

    let readiness = if problems
        .iter()
        .any(|problem| problem.severity == ProblemSeverity::Error)
    {
        Readiness::NotReady
    } else {
        Readiness::Ready
    };

    HookDoctorReport {
        mode,
        readiness,
        state_root: global_state.state_root,
        repository_root: detected_repository_root,
        hook_path_source,
        hooks_directory,
        config_locations: global_state.config_locations,
        agent_trace_local_db: global_state.agent_trace_local_db,
        hooks,
        problems,
    }
}

#[allow(clippy::too_many_lines)]
fn collect_global_state_health(
    repository_root: &Path,
    problems: &mut Vec<DoctorProblem>,
    dependencies: &DoctorDependencies<'_>,
) -> GlobalStateHealth {
    let mut state_root_health = None;
    let mut config_locations = Vec::new();

    match (dependencies.resolve_state_root)() {
        Ok(state_root) => {
            state_root_health = Some(FileLocationHealth {
                label: "State root",
                state: if state_root.exists() { "present" } else { "expected" },
                path: state_root.clone(),
            });
        }
        Err(error) => problems.push(DoctorProblem {
            category: ProblemCategory::GlobalState,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!("Unable to resolve expected state root: {error}"),
            remediation: String::from("Verify that the current platform exposes a writable SCE state directory before rerunning 'sce doctor'."),
            next_action: "manual_steps",
        }),
    }

    match (dependencies.resolve_global_config_path)() {
        Ok(global_path) => {
            if global_path.exists() {
                if let Err(error) = (dependencies.validate_config_file)(&global_path) {
                    problems.push(DoctorProblem {
                        category: ProblemCategory::GlobalState,
                        severity: ProblemSeverity::Error,
                        fixability: ProblemFixability::ManualOnly,
                        summary: format!(
                            "Global config file '{}' failed validation: {error}",
                            global_path.display()
                        ),
                        remediation: format!(
                            "Repair or remove the invalid global config file at '{}' and rerun 'sce doctor'.",
                            global_path.display()
                        ),
                        next_action: "manual_steps",
                    });
                }
            }
            config_locations.push(FileLocationHealth {
                label: "Global config",
                state: if global_path.exists() { "present" } else { "expected" },
                path: global_path,
            });
        }
        Err(error) => problems.push(DoctorProblem {
            category: ProblemCategory::GlobalState,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!("Unable to resolve expected global config path: {error}"),
            remediation: String::from("Verify that the current platform exposes a writable SCE config directory before rerunning 'sce doctor'."),
            next_action: "manual_steps",
        }),
    }

    let local_path = RepoPaths::new(repository_root).sce_config_file();
    if local_path.exists() {
        if let Err(error) = (dependencies.validate_config_file)(&local_path) {
            problems.push(DoctorProblem {
                category: ProblemCategory::GlobalState,
                severity: ProblemSeverity::Error,
                fixability: ProblemFixability::ManualOnly,
                summary: format!(
                    "Local config file '{}' failed validation: {error}",
                    local_path.display()
                ),
                remediation: format!(
                    "Repair or remove the invalid local config file at '{}' and rerun 'sce doctor'.",
                    local_path.display()
                ),
                next_action: "manual_steps",
            });
        }
    }
    config_locations.push(FileLocationHealth {
        label: "Local config",
        state: if local_path.exists() {
            "present"
        } else {
            "expected"
        },
        path: local_path,
    });

    let agent_trace_local_db = match (dependencies.resolve_agent_trace_local_db_path)() {
        Ok(path) => {
            let health = FileLocationHealth {
                label: "Agent Trace local DB",
                state: if path.exists() { "present" } else { "expected" },
                path,
            };
            inspect_agent_trace_db_health(&health, problems, dependencies);
            Some(health)
        }
        Err(error) => {
            problems.push(DoctorProblem {
                category: ProblemCategory::GlobalState,
                severity: ProblemSeverity::Error,
                fixability: ProblemFixability::ManualOnly,
                summary: format!("Unable to resolve expected Agent Trace local DB path: {error}"),
                remediation: String::from("Verify that the SCE state root can be resolved on this machine before rerunning 'sce doctor'."),
                next_action: "manual_steps",
            });
            None
        }
    };

    GlobalStateHealth {
        state_root: state_root_health,
        config_locations,
        agent_trace_local_db,
    }
}

fn inspect_agent_trace_db_health(
    db_health: &FileLocationHealth,
    problems: &mut Vec<DoctorProblem>,
    dependencies: &DoctorDependencies<'_>,
) {
    let Some(parent) = db_health.path.parent() else {
        problems.push(DoctorProblem {
            category: ProblemCategory::GlobalState,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!(
                "Agent Trace local DB path '{}' has no parent directory.",
                db_health.path.display()
            ),
            remediation: String::from("Verify that the SCE state root resolves to a normal filesystem path before rerunning 'sce doctor'."),
            next_action: "manual_steps",
        });
        return;
    };

    match inspect_directory_write_readiness(parent) {
        DirectoryWriteReadiness::Ready => {}
        DirectoryWriteReadiness::Missing => problems.push(DoctorProblem {
            category: ProblemCategory::FilesystemPermissions,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::AutoFixable,
            summary: format!(
                "Agent Trace local DB parent directory '{}' does not exist.",
                parent.display()
            ),
            remediation: format!(
                "Run 'sce doctor --fix' to create the SCE-owned Agent Trace state directory '{}', or create it manually with write access and rerun 'sce doctor'.",
                parent.display()
            ),
            next_action: "doctor_fix",
        }),
        DirectoryWriteReadiness::NotDirectory => problems.push(DoctorProblem {
            category: ProblemCategory::FilesystemPermissions,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!(
                "Agent Trace local DB parent path '{}' is not a directory.",
                parent.display()
            ),
            remediation: format!(
                "Replace '{}' with a writable directory before rerunning 'sce doctor'.",
                parent.display()
            ),
            next_action: "manual_steps",
        }),
        DirectoryWriteReadiness::ReadOnly => problems.push(DoctorProblem {
            category: ProblemCategory::FilesystemPermissions,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!(
                "Agent Trace local DB parent directory '{}' is not writable.",
                parent.display()
            ),
            remediation: format!(
                "Grant write access to '{}' before rerunning 'sce doctor'.",
                parent.display()
            ),
            next_action: "manual_steps",
        }),
        DirectoryWriteReadiness::Unknown(error) => problems.push(DoctorProblem {
            category: ProblemCategory::FilesystemPermissions,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!(
                "Unable to inspect Agent Trace local DB parent directory '{}': {error}",
                parent.display()
            ),
            remediation: format!(
                "Verify that '{}' is accessible and writable before rerunning 'sce doctor'.",
                parent.display()
            ),
            next_action: "manual_steps",
        }),
    }

    if db_health.path.exists() {
        if let Err(error) = (dependencies.check_agent_trace_local_db_health)(&db_health.path) {
            problems.push(DoctorProblem {
                category: ProblemCategory::GlobalState,
                severity: ProblemSeverity::Error,
                fixability: ProblemFixability::ManualOnly,
                summary: format!(
                    "Agent Trace local DB '{}' failed health checks: {error}",
                    db_health.path.display()
                ),
                remediation: format!(
                    "Repair or replace the Agent Trace local DB at '{}' and rerun 'sce doctor'.",
                    db_health.path.display()
                ),
                next_action: "manual_steps",
            });
        }
    }
}

fn collect_hook_health(directory: &Path, problems: &mut Vec<DoctorProblem>) -> Vec<HookFileHealth> {
    if !directory.exists() {
        problems.push(DoctorProblem {
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

    REQUIRED_HOOKS
        .iter()
        .map(|hook_name| {
            let hook_path = directory.join(hook_name);
            let metadata = fs::metadata(&hook_path).ok();
            let exists = metadata.is_some();
            let executable = metadata
                .as_ref()
                .is_some_and(|entry| entry.is_file() && is_executable(entry));
            let content_state = inspect_hook_content_state(hook_name, &hook_path, exists, problems);

            if !exists {
                problems.push(DoctorProblem {
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

            HookFileHealth {
                name: hook_name,
                path: hook_path,
                exists,
                executable,
                content_state,
            }
        })
        .collect()
}

fn inspect_opencode_plugin_health(repository_root: &Path, problems: &mut Vec<DoctorProblem>) {
    let repo_paths = RepoPaths::new(repository_root);
    let install_targets = InstallTargetPaths::new(repository_root);
    let opencode_root = repo_paths.opencode_dir();
    if !opencode_root.exists() {
        return;
    }

    let manifest_path = repo_paths.opencode_manifest_file();
    if let Some(summary) = opencode_plugin_registry_issue(&manifest_path) {
        problems.push(DoctorProblem {
            category: ProblemCategory::RepoAssets,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary,
            remediation: format!(
                "Reinstall OpenCode assets so '{}' registers '{}', then rerun 'sce doctor'.",
                manifest_path.display(),
                opencode_asset::PLUGIN_MANIFEST_ENTRY
            ),
            next_action: "manual_steps",
        });
    }

    inspect_opencode_plugin_dependency_health(&install_targets, problems);

    let plugin_path = install_targets.opencode_plugin_target();
    let plugin_metadata = fs::metadata(&plugin_path).ok();
    let plugin_is_file = plugin_metadata
        .as_ref()
        .is_some_and(std::fs::Metadata::is_file);

    if !plugin_is_file {
        let summary = if plugin_metadata.is_some() {
            format!(
                "OpenCode plugin path '{}' is not a file.",
                plugin_path.display()
            )
        } else {
            format!(
                "OpenCode plugin file '{}' is missing.",
                plugin_path.display()
            )
        };
        problems.push(DoctorProblem {
            category: ProblemCategory::RepoAssets,
            severity: ProblemSeverity::Warning,
            fixability: ProblemFixability::ManualOnly,
            summary,
            remediation: format!(
                "Reinstall OpenCode assets to restore the canonical plugin at '{}', then rerun 'sce doctor'.",
                plugin_path.display()
            ),
            next_action: "manual_steps",
        });
    }
}

fn opencode_plugin_registry_issue(manifest_path: &Path) -> Option<String> {
    if !manifest_path.exists() {
        return Some(format!(
            "OpenCode plugin registry file '{}' is missing.",
            manifest_path.display()
        ));
    }

    let Ok(bytes) = fs::read(manifest_path) else {
        return Some(format!(
            "OpenCode plugin registry file '{}' is not readable.",
            manifest_path.display()
        ));
    };

    let payload: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => {
            return Some(format!(
                "OpenCode plugin registry file '{}' is not valid JSON.",
                manifest_path.display()
            ));
        }
    };

    let Some(plugins) = payload.get("plugin").and_then(|value| value.as_array()) else {
        return Some(format!(
            "OpenCode plugin registry file '{}' does not define a 'plugin' array.",
            manifest_path.display()
        ));
    };

    if plugins
        .iter()
        .any(|entry| entry.as_str() == Some(opencode_asset::PLUGIN_MANIFEST_ENTRY))
    {
        None
    } else {
        Some(format!(
            "OpenCode plugin registry file '{}' does not register '{}'.",
            manifest_path.display(),
            opencode_asset::PLUGIN_MANIFEST_ENTRY
        ))
    }
}

fn inspect_opencode_plugin_dependency_health(
    install_targets: &InstallTargetPaths,
    problems: &mut Vec<DoctorProblem>,
) {
    inspect_opencode_asset_presence(
        &install_targets.opencode_runtime_target(),
        "OpenCode bash-policy runtime",
        "bash-policy runtime",
        problems,
    );
    inspect_opencode_asset_presence(
        &install_targets.opencode_preset_catalog_target(),
        "OpenCode bash-policy preset catalog",
        "bash-policy preset catalog",
        problems,
    );
}

fn inspect_opencode_asset_presence(
    asset_path: &Path,
    summary_label: &str,
    remediation_label: &str,
    problems: &mut Vec<DoctorProblem>,
) {
    let metadata = fs::metadata(asset_path).ok();
    let is_file = metadata.as_ref().is_some_and(std::fs::Metadata::is_file);

    if is_file {
        return;
    }

    let summary = if metadata.is_some() {
        format!(
            "{summary_label} path '{}' is not a file.",
            asset_path.display()
        )
    } else {
        format!(
            "{summary_label} file '{}' is missing.",
            asset_path.display()
        )
    };
    problems.push(DoctorProblem {
        category: ProblemCategory::RepoAssets,
        severity: ProblemSeverity::Warning,
        fixability: ProblemFixability::ManualOnly,
        summary,
        remediation: format!(
            "Reinstall OpenCode assets to restore the canonical {remediation_label} at '{}', then rerun 'sce doctor'.",
            asset_path.display()
        ),
        next_action: "manual_steps",
    });
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

fn inspect_directory_write_readiness(path: &Path) -> DirectoryWriteReadiness {
    match fs::metadata(path) {
        Ok(metadata) => {
            if !metadata.is_dir() {
                DirectoryWriteReadiness::NotDirectory
            } else if metadata.permissions().readonly() {
                DirectoryWriteReadiness::ReadOnly
            } else {
                DirectoryWriteReadiness::Ready
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            DirectoryWriteReadiness::Missing
        }
        Err(error) => DirectoryWriteReadiness::Unknown(error),
    }
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

#[allow(clippy::too_many_lines)]
fn format_report(report: &HookDoctorReport) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "{}: {}",
        label("SCE doctor"),
        match report.readiness {
            Readiness::Ready => success("ready"),
            Readiness::NotReady => value("not ready"),
        }
    ));
    lines.push(format!(
        "{}: {}",
        label("Mode"),
        match report.mode {
            DoctorMode::Diagnose => value("diagnose"),
            DoctorMode::Fix => value("fix"),
        }
    ));

    lines.push(format!(
        "{}: {}",
        label("Hooks path source"),
        value(match report.hook_path_source {
            HookPathSource::Default => "default (.git/hooks)",
            HookPathSource::LocalConfig => "per-repo core.hooksPath",
            HookPathSource::GlobalConfig => "global core.hooksPath",
        })
    ));

    lines.push(format!(
        "{}: {}",
        label("State root"),
        report.state_root.as_ref().map_or_else(
            || value("(not detected)"),
            |location| format!(
                "{} ({})",
                value(location.state),
                value(&location.path.display().to_string())
            )
        )
    ));

    lines.push(format!(
        "{}: {}",
        label("Repository root"),
        report.repository_root.as_ref().map_or_else(
            || value("(not detected)"),
            |path| value(&path.display().to_string())
        )
    ));

    lines.push(format!(
        "{}: {}",
        label("Effective hooks directory"),
        report.hooks_directory.as_ref().map_or_else(
            || value("(not detected)"),
            |path| value(&path.display().to_string())
        )
    ));

    lines.push(format!("\n{}:", heading("Config files")));
    for location in &report.config_locations {
        lines.push(format!(
            "  {}: {} ({})",
            label(location.label),
            value(location.state),
            value(&location.path.display().to_string())
        ));
    }

    lines.push(format!(
        "\n{}: {}",
        label("Agent Trace local DB"),
        report.agent_trace_local_db.as_ref().map_or_else(
            || value("(not detected)"),
            |location| format!(
                "{} ({})",
                value(location.state),
                value(&location.path.display().to_string())
            )
        )
    ));

    // Required hooks
    lines.push(format!("\n{}:", heading("Required hooks")));
    for hook in &report.hooks {
        lines.push(format!(
            "  {}: {} (content={}, executable={}) {}",
            value(hook.name),
            value(hook_state(hook)),
            value(hook_content_state(hook.content_state)),
            value(if hook.executable { "yes" } else { "no" }),
            value(&hook.path.display().to_string())
        ));
    }

    // Problems
    if report.problems.is_empty() {
        lines.push(format!("\n{}: {}", label("Problems"), success("none")));
    } else {
        lines.push(format!("\n{}:", heading("Problems")));
        for problem in &report.problems {
            lines.push(format!(
                "  [{}|{}|{}] {}",
                value(problem_category(problem.category)),
                value(problem_severity(problem.severity)),
                value(problem_fixability(problem.fixability)),
                value(&problem.summary)
            ));
        }
    }

    lines.join("\n")
}

fn format_execution(execution: &DoctorExecution) -> String {
    let report = &execution.report;
    let base_report = format_report(report);
    let mut lines = base_report
        .lines()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if report.mode == DoctorMode::Fix {
        if execution.fix_results.is_empty() {
            lines.push(format!("\n{}: {}", label("Fix results"), value("none")));
        } else {
            lines.push(format!("\n{}:", heading("Fix results")));
            for fix_result in &execution.fix_results {
                lines.push(format!(
                    "  [{}] {}",
                    value(fix_result_outcome(fix_result.outcome)),
                    value(&fix_result.detail)
                ));
            }
        }
    }

    lines.join("\n")
}

fn render_report(request: DoctorRequest, execution: &DoctorExecution) -> Result<String> {
    match request.format {
        DoctorFormat::Text => Ok(format_execution(execution)),
        DoctorFormat::Json => render_report_json(execution),
    }
}

fn render_report_json(execution: &DoctorExecution) -> Result<String> {
    let report = &execution.report;
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
                "content_state": hook_content_state(hook.content_state),
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
                "state": location.state,
            })
        })
        .collect::<Vec<_>>();

    let payload = json!({
        "status": "ok",
        "command": NAME,
        "mode": match report.mode {
            DoctorMode::Diagnose => "diagnose",
            DoctorMode::Fix => "fix",
        },
        "readiness": match report.readiness {
            Readiness::Ready => "ready",
            Readiness::NotReady => "not_ready",
        },
        "state_root": report.state_root.as_ref().map(|location| json!({
            "label": location.label,
            "path": location.path.display().to_string(),
            "state": location.state,
        })),
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
            "state": location.state,
        })),
        "hooks": hooks,
        "problems": report.problems.iter().map(|problem| json!({
            "category": problem_category(problem.category),
            "severity": problem_severity(problem.severity),
            "fixability": problem_fixability(problem.fixability),
            "summary": problem.summary,
            "remediation": {
                "next_action": problem.next_action,
                "text": problem.remediation,
            },
        })).collect::<Vec<_>>(),
        "fix_results": if report.mode == DoctorMode::Fix {
            execution.fix_results.iter()
                .map(|result| json!({
                    "category": problem_category(result.category),
                    "outcome": fix_result_outcome(result.outcome),
                    "detail": result.detail,
                }))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        },
    });

    serde_json::to_string_pretty(&payload).context("failed to serialize doctor report to JSON")
}

fn hook_state(hook: &HookFileHealth) -> &'static str {
    if !hook.exists {
        "missing"
    } else if hook.content_state == HookContentState::Stale {
        "stale"
    } else if !hook.executable {
        "not_executable"
    } else {
        "ok"
    }
}

fn hook_content_state(state: HookContentState) -> &'static str {
    match state {
        HookContentState::Current => "current",
        HookContentState::Stale => "stale",
        HookContentState::Missing => "missing",
        HookContentState::Unknown => "unknown",
    }
}

fn run_auto_fixes(
    report: &HookDoctorReport,
    dependencies: &DoctorDependencies<'_>,
) -> Vec<DoctorFixResultRecord> {
    let auto_fixable_problems = report
        .problems
        .iter()
        .filter(|problem| problem.fixability == ProblemFixability::AutoFixable)
        .collect::<Vec<_>>();

    if auto_fixable_problems.is_empty() {
        return Vec::new();
    }

    let mut fix_results = Vec::new();

    if auto_fixable_problems
        .iter()
        .any(|problem| problem.category == ProblemCategory::FilesystemPermissions)
    {
        fix_results.extend(run_filesystem_auto_fixes(report, dependencies));
    }

    if auto_fixable_problems
        .iter()
        .any(|problem| problem.category == ProblemCategory::HookRollout)
    {
        let Some(repository_root) = report.repository_root.as_deref() else {
            fix_results.push(DoctorFixResultRecord {
                category: ProblemCategory::HookRollout,
                outcome: FixResult::Failed,
                detail: String::from("Automatic hook repair could not start because the repository root was not resolved during diagnosis."),
            });
            return fix_results;
        };

        match (dependencies.install_required_git_hooks)(repository_root) {
            Ok(outcome) => fix_results.extend(build_hook_fix_results(&outcome)),
            Err(error) => fix_results.push(DoctorFixResultRecord {
                category: ProblemCategory::HookRollout,
                outcome: FixResult::Failed,
                detail: format!(
                    "Automatic hook repair failed while reusing the canonical setup flow: {error}"
                ),
            }),
        }
    }

    fix_results
}

fn run_filesystem_auto_fixes(
    report: &HookDoctorReport,
    dependencies: &DoctorDependencies<'_>,
) -> Vec<DoctorFixResultRecord> {
    let Some(db_path) = report
        .agent_trace_local_db
        .as_ref()
        .map(|location| &location.path)
    else {
        return vec![DoctorFixResultRecord {
            category: ProblemCategory::FilesystemPermissions,
            outcome: FixResult::Failed,
            detail: String::from("Automatic Agent Trace directory repair could not start because the expected local DB path was not resolved during diagnosis."),
        }];
    };

    let Some(parent) = db_path.parent() else {
        return vec![DoctorFixResultRecord {
            category: ProblemCategory::FilesystemPermissions,
            outcome: FixResult::Failed,
            detail: format!(
                "Automatic Agent Trace directory repair could not start because '{}' has no parent directory.",
                db_path.display()
            ),
        }];
    };

    let expected_parent = match (dependencies.resolve_agent_trace_local_db_path)() {
        Ok(path) => path.parent().map(Path::to_path_buf),
        Err(error) => {
            return vec![DoctorFixResultRecord {
                category: ProblemCategory::FilesystemPermissions,
                outcome: FixResult::Failed,
                detail: format!(
                    "Automatic Agent Trace directory repair could not confirm the canonical SCE-owned path: {error}"
                ),
            }];
        }
    };

    if expected_parent.as_deref() != Some(parent) {
        return vec![DoctorFixResultRecord {
            category: ProblemCategory::FilesystemPermissions,
            outcome: FixResult::Failed,
            detail: format!(
                "Automatic Agent Trace directory repair refused to modify '{}' because it does not match the canonical SCE-owned path.",
                parent.display()
            ),
        }];
    }

    if parent.exists() {
        return vec![DoctorFixResultRecord {
            category: ProblemCategory::FilesystemPermissions,
            outcome: FixResult::Skipped,
            detail: format!(
                "Agent Trace directory '{}' already exists; no directory bootstrap was needed.",
                parent.display()
            ),
        }];
    }

    match (dependencies.create_directory_all)(parent) {
        Ok(()) => vec![DoctorFixResultRecord {
            category: ProblemCategory::FilesystemPermissions,
            outcome: FixResult::Fixed,
            detail: format!(
                "Created the SCE-owned Agent Trace directory '{}'.",
                parent.display()
            ),
        }],
        Err(error) => vec![DoctorFixResultRecord {
            category: ProblemCategory::FilesystemPermissions,
            outcome: FixResult::Failed,
            detail: format!(
                "Automatic Agent Trace directory repair failed for '{}': {error}",
                parent.display()
            ),
        }],
    }
}

fn create_directory_all(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .with_context(|| format!("Failed to create directory '{}'.", path.display()))
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

fn build_manual_fix_results(report: &HookDoctorReport) -> Vec<DoctorFixResultRecord> {
    report
        .problems
        .iter()
        .filter(|problem| problem.fixability != ProblemFixability::AutoFixable)
        .map(|problem| DoctorFixResultRecord {
            category: problem.category,
            outcome: FixResult::Manual,
            detail: match problem.fixability {
                ProblemFixability::AutoFixable => {
                    unreachable!("auto-fixable problems should not be rendered as manual results")
                }
                ProblemFixability::ManualOnly => {
                    format!("{} Manual remediation is still required.", problem.summary)
                }
            },
        })
        .collect()
}

fn problem_category(category: ProblemCategory) -> &'static str {
    match category {
        ProblemCategory::GlobalState => "global_state",
        ProblemCategory::RepositoryTargeting => "repository_targeting",
        ProblemCategory::HookRollout => "hook_rollout",
        ProblemCategory::RepoAssets => "repo_assets",
        ProblemCategory::FilesystemPermissions => "filesystem_permissions",
    }
}

fn problem_severity(severity: ProblemSeverity) -> &'static str {
    match severity {
        ProblemSeverity::Error => "error",
        ProblemSeverity::Warning => "warning",
    }
}

fn problem_fixability(fixability: ProblemFixability) -> &'static str {
    match fixability {
        ProblemFixability::AutoFixable => "auto_fixable",
        ProblemFixability::ManualOnly => "manual_only",
    }
}

fn fix_result_outcome(outcome: FixResult) -> &'static str {
    match outcome {
        FixResult::Fixed => "fixed",
        FixResult::Skipped => "skipped",
        FixResult::Manual => "manual",
        FixResult::Failed => "failed",
    }
}
