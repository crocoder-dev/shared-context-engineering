use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde_json::json;

use crate::services::default_paths::resolve_sce_default_locations;
use crate::services::output_format::OutputFormat;
use crate::services::setup::{
    install_required_git_hooks, iter_required_hook_assets, RequiredHookInstallStatus,
    RequiredHooksInstallOutcome,
};
#[cfg(test)]
use crate::services::setup::{iter_embedded_assets_for_setup_target, SetupTarget};
use crate::services::style::{heading, label, success, value, OwoColorize};

pub const NAME: &str = "doctor";

const REQUIRED_HOOKS: [&str; 3] = ["pre-commit", "commit-msg", "post-commit"];
const OPENCODE_ROOT_DIR: &str = ".opencode";
const OPENCODE_MANIFEST_FILE: &str = "opencode.json";
const OPENCODE_PLUGIN_RELATIVE_PATH: &str = "plugins/sce-bash-policy.ts";
const OPENCODE_PLUGIN_RUNTIME_RELATIVE_PATH: &str = "plugins/bash-policy/runtime.ts";
const OPENCODE_PLUGIN_PRESET_CATALOG_RELATIVE_PATH: &str = "lib/bash-policy-presets.json";
const OPENCODE_PLUGIN_MANIFEST_ENTRY: &str = "./plugins/sce-bash-policy.ts";
const OPENCODE_REQUIRED_DIRECTORIES: [&str; 3] = ["agent", "command", "skills"];

pub type DoctorFormat = OutputFormat;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DoctorDatabaseInventory {
    Repo,
    All,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DoctorMode {
    Diagnose,
    Fix,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DoctorRequest {
    pub mode: DoctorMode,
    pub database_inventory: DoctorDatabaseInventory,
    pub format: DoctorFormat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DatabaseFamily {
    AgentTraceLocal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DatabaseScope {
    Global,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DatabaseOwnershipStatus {
    Canonical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DatabaseStatus {
    Present,
    Missing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DatabaseHealth {
    family: DatabaseFamily,
    scope: DatabaseScope,
    canonical_path: PathBuf,
    ownership_status: DatabaseOwnershipStatus,
    status: DatabaseStatus,
    repository_root: Option<PathBuf>,
    repository_hash: Option<String>,
    belongs_to_active_repository: bool,
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
    database_inventory: DoctorDatabaseInventory,
    readiness: Readiness,
    state_root: Option<FileLocationHealth>,
    repository_root: Option<PathBuf>,
    hook_path_source: HookPathSource,
    hooks_directory: Option<PathBuf>,
    config_locations: Vec<FileLocationHealth>,
    agent_trace_local_db: Option<FileLocationHealth>,
    repo_databases: Vec<DatabaseHealth>,
    all_databases: Vec<DatabaseHealth>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StatusTag {
    Pass,
    Fail,
    Miss,
    Warn,
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct TaggedLine {
    tag: StatusTag,
    text: String,
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
    let initial_report = build_report_with_dependencies(
        request.mode,
        request.database_inventory,
        repository_root,
        dependencies,
    );

    if request.mode != DoctorMode::Fix {
        return DoctorExecution {
            report: initial_report,
            fix_results: Vec::new(),
        };
    }

    let mut fix_results = run_auto_fixes(&initial_report, dependencies);
    let final_report = build_report_with_dependencies(
        request.mode,
        request.database_inventory,
        repository_root,
        dependencies,
    );
    fix_results.extend(build_manual_fix_results(&final_report));

    DoctorExecution {
        report: final_report,
        fix_results,
    }
}

#[allow(clippy::too_many_lines)]
fn build_report_with_dependencies(
    mode: DoctorMode,
    database_inventory: DoctorDatabaseInventory,
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
            summary: "Git is not available on this machine.".to_string(),
            remediation: "Install an accessible 'git' binary and ensure it is on PATH before rerunning 'sce doctor'.".to_string(),
            next_action: "manual_steps",
        });
        Vec::new()
    } else if bare_repository {
        problems.push(DoctorProblem {
            category: ProblemCategory::RepositoryTargeting,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: "The current repository is bare and does not support local SCE hook rollout.".to_string(),
            remediation: "Run 'sce doctor' from a non-bare working tree clone to inspect repo-scoped SCE hook health.".to_string(),
            next_action: "manual_steps",
        });
        Vec::new()
    } else if detected_repository_root.is_none() {
        problems.push(DoctorProblem {
            category: ProblemCategory::RepositoryTargeting,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: "The current directory is not inside a git repository.".to_string(),
            remediation: "Run 'sce doctor' from inside the target repository working tree to inspect repo-scoped SCE hook health.".to_string(),
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
            summary: "Unable to resolve git hooks directory.".to_string(),
            remediation: "Verify that git repository inspection succeeds and rerun 'sce doctor' inside a non-bare git repository.".to_string(),
            next_action: "manual_steps",
        });
        Vec::new()
    };

    if git_available && !bare_repository {
        if let Some(resolved_root) = detected_repository_root.as_deref() {
            inspect_opencode_plugin_health(resolved_root, &mut problems);
        }
    }

    let repo_databases = Vec::new();
    let all_databases = if database_inventory == DoctorDatabaseInventory::All {
        collect_all_database_health(global_state.agent_trace_local_db.as_ref())
    } else {
        Vec::new()
    };

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
        database_inventory,
        readiness,
        state_root: global_state.state_root,
        repository_root: detected_repository_root,
        hook_path_source,
        hooks_directory,
        config_locations: global_state.config_locations,
        agent_trace_local_db: global_state.agent_trace_local_db,
        repo_databases,
        all_databases,
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
            remediation: "Verify that the current platform exposes a writable SCE state directory before rerunning 'sce doctor'.".to_string(),
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
            remediation: "Verify that the current platform exposes a writable SCE config directory before rerunning 'sce doctor'.".to_string(),
            next_action: "manual_steps",
        }),
    }

    let local_path = repository_root.join(".sce").join("config.json");
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
                remediation: "Verify that the SCE state root can be resolved on this machine before rerunning 'sce doctor'.".to_string(),
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

fn collect_all_database_health(
    agent_trace_local_db: Option<&FileLocationHealth>,
) -> Vec<DatabaseHealth> {
    let mut databases = Vec::new();

    if let Some(agent_trace_local_db) = agent_trace_local_db {
        databases.push(DatabaseHealth {
            family: DatabaseFamily::AgentTraceLocal,
            scope: DatabaseScope::Global,
            canonical_path: agent_trace_local_db.path.clone(),
            ownership_status: DatabaseOwnershipStatus::Canonical,
            status: if agent_trace_local_db.path.exists() {
                DatabaseStatus::Present
            } else {
                DatabaseStatus::Missing
            },
            repository_root: None,
            repository_hash: None,
            belongs_to_active_repository: false,
        });
    }

    databases.sort_by(|left, right| {
        database_scope(left.scope)
            .cmp(database_scope(right.scope))
            .then_with(|| database_family(left.family).cmp(database_family(right.family)))
            .then_with(|| left.canonical_path.cmp(&right.canonical_path))
    });
    databases
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
            remediation: "Verify that the SCE state root resolves to a normal filesystem path before rerunning 'sce doctor'.".to_string(),
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
    let opencode_root = repository_root.join(OPENCODE_ROOT_DIR);
    if !opencode_root.exists() {
        return;
    }

    inspect_opencode_required_directories(&opencode_root, problems);

    let manifest_path = opencode_root.join(OPENCODE_MANIFEST_FILE);
    if let Some(summary) = opencode_plugin_registry_issue(&manifest_path) {
        problems.push(DoctorProblem {
            category: ProblemCategory::RepoAssets,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary,
            remediation: format!(
                "Reinstall OpenCode assets so '{}' registers '{}', then rerun 'sce doctor'.",
                manifest_path.display(),
                OPENCODE_PLUGIN_MANIFEST_ENTRY
            ),
            next_action: "manual_steps",
        });
    }

    inspect_opencode_plugin_dependency_health(&opencode_root, problems);

    let plugin_path = opencode_root.join(OPENCODE_PLUGIN_RELATIVE_PATH);
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

fn inspect_opencode_required_directories(opencode_root: &Path, problems: &mut Vec<DoctorProblem>) {
    for directory in OPENCODE_REQUIRED_DIRECTORIES {
        let required_path = opencode_root.join(directory);
        match fs::metadata(&required_path) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    problems.push(DoctorProblem {
                        category: ProblemCategory::RepoAssets,
                        severity: ProblemSeverity::Error,
                        fixability: ProblemFixability::ManualOnly,
                        summary: format!(
                            "OpenCode required directory '{}' is not a directory.",
                            required_path.display()
                        ),
                        remediation: format!(
                            "Reinstall OpenCode assets so '{}' includes the required '{}' directory, then rerun 'sce doctor'.",
                            opencode_root.display(),
                            directory
                        ),
                        next_action: "manual_steps",
                    });
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                problems.push(DoctorProblem {
                    category: ProblemCategory::RepoAssets,
                    severity: ProblemSeverity::Error,
                    fixability: ProblemFixability::ManualOnly,
                    summary: format!(
                        "OpenCode required directory '{}' is missing.",
                        required_path.display()
                    ),
                    remediation: format!(
                        "Reinstall OpenCode assets so '{}' includes the required '{}' directory, then rerun 'sce doctor'.",
                        opencode_root.display(),
                        directory
                    ),
                    next_action: "manual_steps",
                });
            }
            Err(error) => {
                problems.push(DoctorProblem {
                    category: ProblemCategory::RepoAssets,
                    severity: ProblemSeverity::Error,
                    fixability: ProblemFixability::ManualOnly,
                    summary: format!(
                        "OpenCode required directory '{}' could not be inspected: {error}",
                        required_path.display()
                    ),
                    remediation: format!(
                        "Verify that '{}' is readable and rerun 'sce doctor'.",
                        required_path.display()
                    ),
                    next_action: "manual_steps",
                });
            }
        }
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
        .any(|entry| entry.as_str() == Some(OPENCODE_PLUGIN_MANIFEST_ENTRY))
    {
        None
    } else {
        Some(format!(
            "OpenCode plugin registry file '{}' does not register '{}'.",
            manifest_path.display(),
            OPENCODE_PLUGIN_MANIFEST_ENTRY
        ))
    }
}

#[cfg(test)]
fn opencode_plugin_asset() -> Option<&'static crate::services::setup::EmbeddedAsset> {
    iter_embedded_assets_for_setup_target(SetupTarget::OpenCode)
        .find(|asset| asset.relative_path == OPENCODE_PLUGIN_RELATIVE_PATH)
}

fn inspect_opencode_plugin_dependency_health(
    opencode_root: &Path,
    problems: &mut Vec<DoctorProblem>,
) {
    inspect_opencode_asset_presence(
        opencode_root,
        OPENCODE_PLUGIN_RUNTIME_RELATIVE_PATH,
        "OpenCode bash-policy runtime",
        "bash-policy runtime",
        problems,
    );
    inspect_opencode_asset_presence(
        opencode_root,
        OPENCODE_PLUGIN_PRESET_CATALOG_RELATIVE_PATH,
        "OpenCode bash-policy preset catalog",
        "bash-policy preset catalog",
        problems,
    );
}

fn inspect_opencode_asset_presence(
    opencode_root: &Path,
    relative_path: &str,
    summary_label: &str,
    remediation_label: &str,
    problems: &mut Vec<DoctorProblem>,
) {
    let asset_path = opencode_root.join(relative_path);
    let metadata = fs::metadata(&asset_path).ok();
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
fn format_report_lines(report: &HookDoctorReport) -> Vec<TaggedLine> {
    let mut lines = Vec::new();
    lines.push(TaggedLine {
        tag: readiness_tag(report.readiness),
        text: format!(
            "{}: {}",
            label("SCE doctor"),
            match report.readiness {
                Readiness::Ready => success("ready"),
                Readiness::NotReady => value("not ready"),
            }
        ),
    });
    lines.push(TaggedLine {
        tag: StatusTag::Pass,
        text: format!(
            "{}: {}",
            label("Mode"),
            match report.mode {
                DoctorMode::Diagnose => value("diagnose"),
                DoctorMode::Fix => value("fix"),
            }
        ),
    });
    lines.push(TaggedLine {
        tag: StatusTag::Pass,
        text: format!(
            "{}: {}",
            label("Database inventory"),
            match report.database_inventory {
                DoctorDatabaseInventory::Repo => value("repo"),
                DoctorDatabaseInventory::All => value("all"),
            }
        ),
    });

    lines.push(TaggedLine {
        tag: StatusTag::Pass,
        text: format!(
            "{}: {}",
            label("Hooks path source"),
            value(match report.hook_path_source {
                HookPathSource::Default => "default (.git/hooks)",
                HookPathSource::LocalConfig => "per-repo core.hooksPath",
                HookPathSource::GlobalConfig => "global core.hooksPath",
            })
        ),
    });

    lines.push(TaggedLine {
        tag: report
            .state_root
            .as_ref()
            .map_or(StatusTag::Miss, |location| {
                tag_for_location_state(location.state)
            }),
        text: format!(
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
        ),
    });

    lines.push(TaggedLine {
        tag: report
            .repository_root
            .as_ref()
            .map_or(StatusTag::Miss, |_| StatusTag::Pass),
        text: format!(
            "{}: {}",
            label("Repository root"),
            report.repository_root.as_ref().map_or_else(
                || value("(not detected)"),
                |path| value(&path.display().to_string())
            )
        ),
    });

    lines.push(TaggedLine {
        tag: report
            .hooks_directory
            .as_ref()
            .map_or(StatusTag::Miss, |_| StatusTag::Pass),
        text: format!(
            "{}: {}",
            label("Effective hooks directory"),
            report.hooks_directory.as_ref().map_or_else(
                || value("(not detected)"),
                |path| value(&path.display().to_string())
            )
        ),
    });

    lines.push(TaggedLine {
        tag: StatusTag::Pass,
        text: format!("{}:", heading("Config files")),
    });
    for location in &report.config_locations {
        lines.push(TaggedLine {
            tag: tag_for_location_state(location.state),
            text: format!(
                "  {}: {} ({})",
                label(location.label),
                value(location.state),
                value(&location.path.display().to_string())
            ),
        });
    }

    lines.push(TaggedLine {
        tag: report
            .agent_trace_local_db
            .as_ref()
            .map_or(StatusTag::Miss, |location| {
                tag_for_location_state(location.state)
            }),
        text: format!(
            "{}: {}",
            label("Agent Trace local DB"),
            report.agent_trace_local_db.as_ref().map_or_else(
                || value("(not detected)"),
                |location| format!(
                    "{} ({})",
                    value(location.state),
                    value(&location.path.display().to_string())
                )
            )
        ),
    });

    // Repo-scoped databases (empty by design)
    lines.push(TaggedLine {
        tag: StatusTag::Pass,
        text: format!("{}:", heading("Repo-scoped databases")),
    });
    if report.repo_databases.is_empty() {
        lines.push(TaggedLine {
            tag: StatusTag::Miss,
            text: value("  none"),
        });
    } else {
        for database in &report.repo_databases {
            lines.push(TaggedLine {
                tag: tag_for_database_status(database.status),
                text: format!("- {}", format_database_record(database)),
            });
        }
    }

    // All SCE databases (when --all-databases)
    if report.database_inventory == DoctorDatabaseInventory::All {
        lines.push(TaggedLine {
            tag: StatusTag::Pass,
            text: format!("{}:", heading("All SCE databases")),
        });
        if report.all_databases.is_empty() {
            lines.push(TaggedLine {
                tag: StatusTag::Miss,
                text: value("  none"),
            });
        } else {
            for database in &report.all_databases {
                lines.push(TaggedLine {
                    tag: tag_for_database_status(database.status),
                    text: format!(
                        "  {}: {} ({}) {}",
                        value(database_family(database.family)),
                        value(database_scope(database.scope)),
                        value(database_status(database.status)),
                        value(&database.canonical_path.display().to_string())
                    ),
                });
            }
        }
    }

    // Required hooks
    lines.push(TaggedLine {
        tag: StatusTag::Pass,
        text: format!("{}:", heading("Required hooks")),
    });
    for hook in &report.hooks {
        lines.push(TaggedLine {
            tag: tag_for_hook(hook),
            text: format!(
                "  {}: {} (content={}, executable={}) {}",
                value(hook.name),
                value(hook_state(hook)),
                value(hook_content_state(hook.content_state)),
                value(if hook.executable { "yes" } else { "no" }),
                value(&hook.path.display().to_string())
            ),
        });
    }

    // Problems
    if report.problems.is_empty() {
        lines.push(TaggedLine {
            tag: StatusTag::Pass,
            text: format!("{}: {}", label("Problems"), success("none")),
        });
    } else {
        lines.push(TaggedLine {
            tag: tag_for_problem_heading(&report.problems),
            text: format!("{}:", heading("Problems")),
        });
        for problem in &report.problems {
            lines.push(TaggedLine {
                tag: tag_for_problem_severity(problem.severity),
                text: format!(
                    "  [{}|{}|{}] {}",
                    value(problem_category(problem.category)),
                    value(problem_severity(problem.severity)),
                    value(problem_fixability(problem.fixability)),
                    value(&problem.summary)
                ),
            });
        }
    }

    lines
}

fn format_execution(execution: &DoctorExecution) -> String {
    let report = &execution.report;
    let mut lines = format_report_lines(report);

    if report.mode == DoctorMode::Fix {
        if execution.fix_results.is_empty() {
            lines.push(TaggedLine {
                tag: StatusTag::Pass,
                text: format!("{}: {}", label("Fix results"), value("none")),
            });
        } else {
            lines.push(TaggedLine {
                tag: tag_for_fix_results_heading(&execution.fix_results),
                text: format!("{}:", heading("Fix results")),
            });
            for fix_result in &execution.fix_results {
                lines.push(TaggedLine {
                    tag: tag_for_fix_result(fix_result.outcome),
                    text: format!(
                        "  [{}] {}",
                        value(fix_result_outcome(fix_result.outcome)),
                        value(&fix_result.detail)
                    ),
                });
            }
        }
    }

    format_tagged_lines(lines)
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
        "database_inventory": match report.database_inventory {
            DoctorDatabaseInventory::Repo => "repo",
            DoctorDatabaseInventory::All => "all",
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
        "repo_databases": report.repo_databases.iter().map(render_database_record_json).collect::<Vec<_>>(),
        "all_databases": report.all_databases.iter().map(render_database_record_json).collect::<Vec<_>>(),
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

fn format_tagged_lines(lines: Vec<TaggedLine>) -> String {
    lines
        .into_iter()
        .map(|line| format!("{} {}", status_tag_prefix(line.tag), line.text))
        .collect::<Vec<_>>()
        .join("\n")
}

fn status_tag_prefix(tag: StatusTag) -> String {
    let prefix = format!("[{}]", status_tag_label(tag));
    match tag {
        StatusTag::Pass => prefix.green().to_string(),
        StatusTag::Fail => prefix.red().to_string(),
        StatusTag::Warn => prefix.yellow().to_string(),
        StatusTag::Miss => prefix.blue().to_string(),
    }
}

fn status_tag_label(tag: StatusTag) -> &'static str {
    match tag {
        StatusTag::Pass => "PASS",
        StatusTag::Fail => "FAIL",
        StatusTag::Miss => "MISS",
        StatusTag::Warn => "WARN",
    }
}

fn readiness_tag(readiness: Readiness) -> StatusTag {
    match readiness {
        Readiness::Ready => StatusTag::Pass,
        Readiness::NotReady => StatusTag::Fail,
    }
}

fn tag_for_location_state(state: &str) -> StatusTag {
    match state {
        "present" => StatusTag::Pass,
        "expected" => StatusTag::Miss,
        _ => StatusTag::Warn,
    }
}

fn tag_for_database_status(status: DatabaseStatus) -> StatusTag {
    match status {
        DatabaseStatus::Present => StatusTag::Pass,
        DatabaseStatus::Missing => StatusTag::Miss,
    }
}

fn tag_for_hook(hook: &HookFileHealth) -> StatusTag {
    if hook_state(hook) == "ok" {
        StatusTag::Pass
    } else {
        StatusTag::Fail
    }
}

fn tag_for_problem_heading(problems: &[DoctorProblem]) -> StatusTag {
    if problems
        .iter()
        .any(|problem| problem.severity == ProblemSeverity::Error)
    {
        StatusTag::Fail
    } else {
        StatusTag::Warn
    }
}

fn tag_for_problem_severity(severity: ProblemSeverity) -> StatusTag {
    match severity {
        ProblemSeverity::Error => StatusTag::Fail,
        ProblemSeverity::Warning => StatusTag::Warn,
    }
}

fn tag_for_fix_results_heading(results: &[DoctorFixResultRecord]) -> StatusTag {
    if results
        .iter()
        .any(|result| result.outcome == FixResult::Failed)
    {
        StatusTag::Fail
    } else if results
        .iter()
        .any(|result| result.outcome == FixResult::Manual)
    {
        StatusTag::Warn
    } else {
        StatusTag::Pass
    }
}

fn tag_for_fix_result(outcome: FixResult) -> StatusTag {
    match outcome {
        FixResult::Fixed | FixResult::Skipped => StatusTag::Pass,
        FixResult::Manual => StatusTag::Warn,
        FixResult::Failed => StatusTag::Fail,
    }
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

fn format_database_record(database: &DatabaseHealth) -> String {
    let mut details = vec![
        format!("family={}", database_family(database.family)),
        format!("scope={}", database_scope(database.scope)),
        format!("status={}", database_status(database.status)),
        format!(
            "ownership={}",
            database_ownership_status(database.ownership_status)
        ),
        format!("path={}", database.canonical_path.display()),
    ];

    if let Some(repository_root) = &database.repository_root {
        details.push(format!("repository_root={}", repository_root.display()));
    }
    if let Some(repository_hash) = &database.repository_hash {
        details.push(format!("repository_hash={repository_hash}"));
    }
    details.push(format!(
        "active_repository={}",
        if database.belongs_to_active_repository {
            "yes"
        } else {
            "no"
        }
    ));

    details.join(", ")
}

fn render_database_record_json(database: &DatabaseHealth) -> serde_json::Value {
    json!({
        "family": database_family(database.family),
        "scope": database_scope(database.scope),
        "canonical_path": database.canonical_path.display().to_string(),
        "ownership_status": database_ownership_status(database.ownership_status),
        "status": database_status(database.status),
        "repository_root": database.repository_root.as_ref().map(|path| path.display().to_string()),
        "repository_hash": database.repository_hash,
        "belongs_to_active_repository": database.belongs_to_active_repository,
    })
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
                detail: "Automatic hook repair could not start because the repository root was not resolved during diagnosis.".to_string(),
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
            detail: "Automatic Agent Trace directory repair could not start because the expected local DB path was not resolved during diagnosis.".to_string(),
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

fn database_family(family: DatabaseFamily) -> &'static str {
    match family {
        DatabaseFamily::AgentTraceLocal => "agent_trace_local",
    }
}

fn database_scope(scope: DatabaseScope) -> &'static str {
    match scope {
        DatabaseScope::Global => "global",
    }
}

fn database_ownership_status(status: DatabaseOwnershipStatus) -> &'static str {
    match status {
        DatabaseOwnershipStatus::Canonical => "canonical",
    }
}

fn database_status(status: DatabaseStatus) -> &'static str {
    match status {
        DatabaseStatus::Present => "present",
        DatabaseStatus::Missing => "missing",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use anyhow::Result;
    use serde_json::Value;

    use super::{
        execute_doctor_with_dependencies, render_report, run_filesystem_auto_fixes,
        DoctorDatabaseInventory, DoctorDependencies, DoctorExecution, DoctorFormat, DoctorMode,
        DoctorProblem, DoctorRequest, FileLocationHealth, FixResult, HookDoctorReport,
        HookPathSource, ProblemCategory, ProblemFixability, ProblemSeverity, Readiness, NAME,
    };
    use crate::services::setup::RequiredHooksInstallOutcome;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Result<Self> {
            let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let path = std::env::temp_dir().join(format!("sce-doctor-{label}-{unique}"));
            fs::create_dir_all(&path)?;
            Ok(Self { path })
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn install_canonical_hooks(hooks_dir: &Path) -> Result<()> {
        fs::create_dir_all(hooks_dir)?;
        for asset in super::iter_required_hook_assets() {
            let path = hooks_dir.join(asset.relative_path);
            fs::write(&path, asset.bytes)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
            }
        }
        Ok(())
    }

    fn filesystem_problem(summary: &str) -> DoctorProblem {
        DoctorProblem {
            category: ProblemCategory::FilesystemPermissions,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::AutoFixable,
            summary: summary.to_string(),
            remediation: "Run 'sce doctor --fix'.".to_string(),
            next_action: "doctor_fix",
        }
    }

    fn problem_matches(
        problem: &Value,
        category: &str,
        severity: &str,
        fixability: &str,
        summary_fragment: &str,
    ) -> bool {
        problem
            .get("category")
            .and_then(Value::as_str)
            .is_some_and(|value| value == category)
            && problem
                .get("severity")
                .and_then(Value::as_str)
                .is_some_and(|value| value == severity)
            && problem
                .get("fixability")
                .and_then(Value::as_str)
                .is_some_and(|value| value == fixability)
            && problem
                .get("summary")
                .and_then(Value::as_str)
                .is_some_and(|value| value.contains(summary_fragment))
    }

    fn assert_all_lines_tagged(output: &str) {
        let prefixes = [
            super::status_tag_prefix(super::StatusTag::Pass),
            super::status_tag_prefix(super::StatusTag::Fail),
            super::status_tag_prefix(super::StatusTag::Miss),
            super::status_tag_prefix(super::StatusTag::Warn),
        ]
        .map(|prefix| format!("{prefix} "));
        for line in output.lines() {
            assert!(
                prefixes.iter().any(|prefix| line.starts_with(prefix)),
                "line missing status tag: '{line}'"
            );
        }
    }

    fn base_report(mode: DoctorMode, readiness: Readiness) -> HookDoctorReport {
        HookDoctorReport {
            mode,
            database_inventory: DoctorDatabaseInventory::Repo,
            readiness,
            state_root: Some(FileLocationHealth {
                label: "State root",
                path: PathBuf::from("/tmp/state"),
                state: "present",
            }),
            repository_root: Some(PathBuf::from("/tmp/repo")),
            hook_path_source: HookPathSource::Default,
            hooks_directory: Some(PathBuf::from("/tmp/repo/.git/hooks")),
            config_locations: vec![FileLocationHealth {
                label: "Global config",
                path: PathBuf::from("/tmp/config.json"),
                state: "present",
            }],
            agent_trace_local_db: Some(FileLocationHealth {
                label: "Agent Trace local DB",
                path: PathBuf::from("/tmp/state/sce/agent-trace/local.db"),
                state: "present",
            }),
            repo_databases: Vec::new(),
            all_databases: Vec::new(),
            hooks: Vec::new(),
            problems: Vec::new(),
        }
    }

    #[test]
    fn render_json_includes_stable_fields_without_filesystem() -> Result<()> {
        let output = render_report(
            DoctorRequest {
                mode: DoctorMode::Diagnose,
                database_inventory: DoctorDatabaseInventory::Repo,
                format: DoctorFormat::Json,
            },
            &super::execute_doctor(
                DoctorRequest {
                    mode: DoctorMode::Diagnose,
                    database_inventory: DoctorDatabaseInventory::Repo,
                    format: DoctorFormat::Text,
                },
                std::path::Path::new("/nonexistent"),
            ),
        )?;

        let parsed: Value = serde_json::from_str(&output)?;
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["command"], NAME);
        assert_eq!(parsed["mode"], "diagnose");
        assert_eq!(parsed["database_inventory"], "repo");
        assert!(parsed["readiness"].as_str().is_some());
        assert!(parsed["state_root"].is_null() || parsed["state_root"].is_object());
        assert!(parsed["hook_path_source"].as_str().is_some());
        assert!(parsed["config_paths"].is_array());
        assert!(parsed["repo_databases"].is_array());
        assert!(parsed["all_databases"].is_array());
        assert!(parsed["hooks"].is_array());
        assert!(parsed["problems"].is_array());
        assert!(parsed["fix_results"].is_array());
        Ok(())
    }

    #[test]
    fn render_fix_mode_json_includes_fix_results() -> Result<()> {
        let output = render_report(
            DoctorRequest {
                mode: DoctorMode::Fix,
                database_inventory: DoctorDatabaseInventory::Repo,
                format: DoctorFormat::Json,
            },
            &super::execute_doctor(
                DoctorRequest {
                    mode: DoctorMode::Fix,
                    database_inventory: DoctorDatabaseInventory::Repo,
                    format: DoctorFormat::Text,
                },
                std::path::Path::new("/nonexistent"),
            ),
        )?;

        let parsed: Value = serde_json::from_str(&output)?;
        assert_eq!(parsed["mode"], "fix");
        assert!(parsed["fix_results"].is_array());
        Ok(())
    }

    #[test]
    fn doctor_text_output_tags_all_lines_for_ready_report() {
        let execution = DoctorExecution {
            report: base_report(DoctorMode::Diagnose, Readiness::Ready),
            fix_results: Vec::new(),
        };
        let output = super::format_execution(&execution);

        assert_all_lines_tagged(&output);
    }

    #[test]
    fn doctor_text_output_tags_all_lines_for_not_ready_report() {
        let mut report = base_report(DoctorMode::Diagnose, Readiness::NotReady);
        report.problems.push(DoctorProblem {
            category: ProblemCategory::HookRollout,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: "Missing required hook".to_string(),
            remediation: "Install hooks".to_string(),
            next_action: "manual_steps",
        });
        let execution = DoctorExecution {
            report,
            fix_results: Vec::new(),
        };
        let output = super::format_execution(&execution);

        assert_all_lines_tagged(&output);
        assert!(output.contains(&super::status_tag_prefix(super::StatusTag::Fail)));
    }

    #[test]
    fn doctor_text_output_tags_all_lines_for_fix_results() {
        let execution = DoctorExecution {
            report: base_report(DoctorMode::Fix, Readiness::Ready),
            fix_results: vec![
                super::DoctorFixResultRecord {
                    category: ProblemCategory::HookRollout,
                    outcome: FixResult::Fixed,
                    detail: "Installed hook".to_string(),
                },
                super::DoctorFixResultRecord {
                    category: ProblemCategory::HookRollout,
                    outcome: FixResult::Failed,
                    detail: "Hook repair failed".to_string(),
                },
            ],
        };
        let output = super::format_execution(&execution);

        assert_all_lines_tagged(&output);
        assert!(output.contains(&super::status_tag_prefix(super::StatusTag::Fail)));
    }

    #[test]
    fn doctor_text_output_includes_warn_and_miss_tags() {
        let mut report = base_report(DoctorMode::Diagnose, Readiness::Ready);
        report.state_root = None;
        if let Some(location) = report.config_locations.first_mut() {
            location.state = "expected";
        }
        report.problems.push(DoctorProblem {
            category: ProblemCategory::RepoAssets,
            severity: ProblemSeverity::Warning,
            fixability: ProblemFixability::ManualOnly,
            summary: "warning from test".to_string(),
            remediation: "manual remediation".to_string(),
            next_action: "manual_steps",
        });

        let execution = DoctorExecution {
            report,
            fix_results: Vec::new(),
        };
        let output = super::format_execution(&execution);

        assert_all_lines_tagged(&output);
        assert!(output.contains(&super::status_tag_prefix(super::StatusTag::Warn)));
        assert!(output.contains(&super::status_tag_prefix(super::StatusTag::Miss)));
        assert!(output.contains("warning from test"));
    }

    #[test]
    fn doctor_reports_local_config_validation_failures() -> Result<()> {
        let test_dir = TestDir::new("doctor-local-config")?;
        let repository_root = test_dir.path().join("repo");
        let hooks_dir = repository_root.join(".git").join("hooks");
        let local_config_path = repository_root.join(".sce").join("config.json");
        install_canonical_hooks(&hooks_dir)?;
        fs::create_dir_all(
            local_config_path
                .parent()
                .expect("local config path should have parent"),
        )?;
        fs::write(&local_config_path, "{}")?;

        let repo_root = repository_root.clone();
        let hooks_dir = hooks_dir.clone();
        let run_git_command = move |_cwd: &Path, args: &[&str]| match args {
            ["rev-parse", "--show-toplevel"] => Some(repo_root.display().to_string()),
            ["rev-parse", "--is-bare-repository"] => Some("false".to_string()),
            ["rev-parse", "--git-path", "hooks"] => Some(hooks_dir.display().to_string()),
            _ => None,
        };
        let dependencies = DoctorDependencies {
            run_git_command: &run_git_command,
            check_git_available: &|| true,
            resolve_state_root: &|| Ok(test_dir.path().join("state-root")),
            resolve_global_config_path: &|| Ok(test_dir.path().join("config-root/sce/config.json")),
            resolve_agent_trace_local_db_path: &|| {
                Ok(test_dir.path().join("state-root/sce/agent-trace/local.db"))
            },
            validate_config_file: &|path: &Path| {
                if path.ends_with(Path::new(".sce/config.json")) {
                    anyhow::bail!("schema mismatch")
                }
                Ok(())
            },
            check_agent_trace_local_db_health: &|_| Ok(()),
            install_required_git_hooks: &|_| unreachable!("hook install should not run"),
            create_directory_all: &|_| unreachable!("directory creation should not run"),
        };

        let execution = execute_doctor_with_dependencies(
            DoctorRequest {
                mode: DoctorMode::Diagnose,
                database_inventory: DoctorDatabaseInventory::Repo,
                format: DoctorFormat::Text,
            },
            &repository_root,
            &dependencies,
        );

        assert_eq!(execution.report.readiness, Readiness::NotReady);
        assert!(execution.report.problems.iter().any(|problem| {
            problem.summary.contains("Local config file")
                && problem.summary.contains("schema mismatch")
        }));
        Ok(())
    }

    #[test]
   fn doctor_reports_state_root_failure_without_losing_global_config_path() -> Result<()> {
        let test_dir = TestDir::new("doctor-state-root-failure")?;
        let repository_root = test_dir.path().join("repo");
        let hooks_dir = repository_root.join(".git").join("hooks");
        install_canonical_hooks(&hooks_dir)?;

        let global_config_path = test_dir.path().join("config-root/sce/config.json");
        fs::create_dir_all(
            global_config_path
                .parent()
                .expect("global config path should have parent"),
        )?;
        fs::write(&global_config_path, "{}")?;

        let repo_root = repository_root.clone();
        let hooks_dir = hooks_dir.clone();
        let run_git_command = move |_cwd: &Path, args: &[&str]| match args {
            ["rev-parse", "--show-toplevel"] => Some(repo_root.display().to_string()),
            ["rev-parse", "--is-bare-repository"] => Some("false".to_string()),
            ["rev-parse", "--git-path", "hooks"] => Some(hooks_dir.display().to_string()),
            _ => None,
        };
        let dependencies = DoctorDependencies {
            run_git_command: &run_git_command,
            check_git_available: &|| true,
            resolve_state_root: &|| anyhow::bail!("state root unavailable"),
            resolve_global_config_path: &|| Ok(global_config_path.clone()),
            resolve_agent_trace_local_db_path: &|| {
                Ok(test_dir.path().join("state-root/sce/agent-trace/local.db"))
            },
            validate_config_file: &|_| Ok(()),
            check_agent_trace_local_db_health: &|_| Ok(()),
            install_required_git_hooks: &|_| unreachable!("hook install should not run"),
            create_directory_all: &|_| unreachable!("directory creation should not run"),
        };

        let execution = execute_doctor_with_dependencies(
            DoctorRequest {
                mode: DoctorMode::Diagnose,
                database_inventory: DoctorDatabaseInventory::Repo,
                format: DoctorFormat::Text,
            },
            &repository_root,
            &dependencies,
        );

        assert_eq!(execution.report.readiness, Readiness::NotReady);
        assert!(execution.report.problems.iter().any(|problem| {
            problem.summary == "Unable to resolve expected state root: state root unavailable"
        }));
        assert!(execution
            .report
            .config_locations
            .iter()
            .any(
                |location| location.label == "Global config" && location.path == global_config_path
            ));

        let output = render_report(
            DoctorRequest {
                mode: DoctorMode::Diagnose,
                database_inventory: DoctorDatabaseInventory::Repo,
                format: DoctorFormat::Json,
            },
            &execution,
        )?;
        let parsed: Value = serde_json::from_str(&output)?;
        assert_eq!(parsed["config_paths"][0]["label"], "Global config");
        assert_eq!(
            parsed["config_paths"][0]["path"],
            global_config_path.display().to_string()
        );
        Ok(())
    }
    
    #[test]
    fn doctor_skips_opencode_structure_checks_without_root() -> Result<()> {
        let test_dir = TestDir::new("doctor-opencode-structure-skip")?;
        let repository_root = test_dir.path().join("repo");
        let hooks_dir = repository_root.join(".git").join("hooks");
        install_canonical_hooks(&hooks_dir)?;

        let agent_trace_db = test_dir
            .path()
            .join("state-root")
            .join("sce")
            .join("agent-trace")
            .join("local.db");
        fs::create_dir_all(
            agent_trace_db
                .parent()
                .expect("agent trace path should have parent"),
        )?;

        let repo_root = repository_root.clone();
        let hooks_dir = hooks_dir.clone();
        let run_git_command = move |_cwd: &Path, args: &[&str]| match args {
            ["rev-parse", "--show-toplevel"] => Some(repo_root.display().to_string()),
            ["rev-parse", "--is-bare-repository"] => Some("false".to_string()),
            ["rev-parse", "--git-path", "hooks"] => Some(hooks_dir.display().to_string()),
            _ => None,
        };

        let state_root = test_dir.path().join("state-root");
        let resolve_state_root = move || Ok(state_root.clone());
        let resolve_agent_trace_local_db_path = move || Ok(agent_trace_db.clone());

        let dependencies = DoctorDependencies {
            run_git_command: &run_git_command,
            check_git_available: &|| true,
            resolve_state_root: &resolve_state_root,
            resolve_agent_trace_local_db_path: &resolve_agent_trace_local_db_path,
            validate_config_file: &|_| Ok(()),
            check_agent_trace_local_db_health: &|_| Ok(()),
            install_required_git_hooks: &|_| unreachable!("hook install should not run"),
            create_directory_all: &|_| unreachable!("directory creation should not run"),
        };

        let json_request = DoctorRequest {
            mode: DoctorMode::Diagnose,
            database_inventory: DoctorDatabaseInventory::Repo,
            format: DoctorFormat::Json,
        };
        let execution = execute_doctor_with_dependencies(
            DoctorRequest {
                mode: DoctorMode::Diagnose,
                database_inventory: DoctorDatabaseInventory::Repo,
                format: DoctorFormat::Text,
            },
            &repository_root,
            &dependencies,
        );
        let output = render_report(json_request, &execution)?;
        let parsed: Value = serde_json::from_str(&output)?;

        assert_eq!(parsed["readiness"], "ready");
        let problems = parsed["problems"].as_array().expect("problems array");
        assert!(problems.is_empty());
        Ok(())
    }

    #[test]
    fn doctor_reports_opencode_structure_missing_directories() -> Result<()> {
        let test_dir = TestDir::new("doctor-opencode-structure-missing")?;
        let repository_root = test_dir.path().join("repo");
        let hooks_dir = repository_root.join(".git").join("hooks");
        install_canonical_hooks(&hooks_dir)?;

        let opencode_root = repository_root.join(".opencode");
        fs::create_dir_all(&opencode_root)?;
        fs::write(
            opencode_root.join("opencode.json"),
            "{\"plugin\":[\"./plugins/sce-bash-policy.ts\"]}",
        )?;

        let agent_trace_db = test_dir
            .path()
            .join("state-root")
            .join("sce")
            .join("agent-trace")
            .join("local.db");
        fs::create_dir_all(
            agent_trace_db
                .parent()
                .expect("agent trace path should have parent"),
        )?;

        let repo_root = repository_root.clone();
        let hooks_dir = hooks_dir.clone();
        let run_git_command = move |_cwd: &Path, args: &[&str]| match args {
            ["rev-parse", "--show-toplevel"] => Some(repo_root.display().to_string()),
            ["rev-parse", "--is-bare-repository"] => Some("false".to_string()),
            ["rev-parse", "--git-path", "hooks"] => Some(hooks_dir.display().to_string()),
            _ => None,
        };

        let state_root = test_dir.path().join("state-root");
        let resolve_state_root = move || Ok(state_root.clone());
        let resolve_agent_trace_local_db_path = move || Ok(agent_trace_db.clone());

        let dependencies = DoctorDependencies {
            run_git_command: &run_git_command,
            check_git_available: &|| true,
            resolve_state_root: &resolve_state_root,
            resolve_agent_trace_local_db_path: &resolve_agent_trace_local_db_path,
            validate_config_file: &|_| Ok(()),
            check_agent_trace_local_db_health: &|_| Ok(()),
            install_required_git_hooks: &|_| unreachable!("hook install should not run"),
            create_directory_all: &|_| unreachable!("directory creation should not run"),
        };

        let json_request = DoctorRequest {
            mode: DoctorMode::Diagnose,
            database_inventory: DoctorDatabaseInventory::Repo,
            format: DoctorFormat::Json,
        };
        let execution = execute_doctor_with_dependencies(
            DoctorRequest {
                mode: DoctorMode::Diagnose,
                database_inventory: DoctorDatabaseInventory::Repo,
                format: DoctorFormat::Text,
            },
            &repository_root,
            &dependencies,
        );
        let output = render_report(json_request, &execution)?;
        let parsed: Value = serde_json::from_str(&output)?;

        assert_eq!(parsed["readiness"], "not_ready");
        let problems = parsed["problems"].as_array().expect("problems array");
        assert!(problems.iter().any(|problem| {
            problem_matches(
                problem,
                "repo_assets",
                "error",
                "manual_only",
                ".opencode/agent",
            )
        }));
        assert!(problems.iter().any(|problem| {
            problem_matches(
                problem,
                "repo_assets",
                "error",
                "manual_only",
                ".opencode/command",
            )
        }));
        assert!(problems.iter().any(|problem| {
            problem_matches(
                problem,
                "repo_assets",
                "error",
                "manual_only",
                ".opencode/skills",
            )
        }));
        Ok(())
    }

    #[test]
    fn doctor_reports_opencode_plugin_missing_file_warning() -> Result<()> {
        let test_dir = TestDir::new("doctor-opencode-file-missing")?;
        let repository_root = test_dir.path().join("repo");
        let hooks_dir = repository_root.join(".git").join("hooks");
        install_canonical_hooks(&hooks_dir)?;

        let opencode_root = repository_root.join(".opencode");
        fs::create_dir_all(&opencode_root)?;
        fs::create_dir_all(opencode_root.join("agent"))?;
        fs::create_dir_all(opencode_root.join("command"))?;
        fs::create_dir_all(opencode_root.join("skills"))?;
        fs::write(
            opencode_root.join("opencode.json"),
            "{\"plugin\":[\"./plugins/sce-bash-policy.ts\"]}",
        )?;

        let agent_trace_db = test_dir
            .path()
            .join("state-root")
            .join("sce")
            .join("agent-trace")
            .join("local.db");
        fs::create_dir_all(
            agent_trace_db
                .parent()
                .expect("agent trace path should have parent"),
        )?;

        let repo_root = repository_root.clone();
        let hooks_dir = hooks_dir.clone();
        let run_git_command = move |_cwd: &Path, args: &[&str]| match args {
            ["rev-parse", "--show-toplevel"] => Some(repo_root.display().to_string()),
            ["rev-parse", "--is-bare-repository"] => Some("false".to_string()),
            ["rev-parse", "--git-path", "hooks"] => Some(hooks_dir.display().to_string()),
            _ => None,
        };

        let state_root = test_dir.path().join("state-root");
        let resolve_state_root = move || Ok(state_root.clone());
        let resolve_agent_trace_local_db_path = move || Ok(agent_trace_db.clone());

        let dependencies = DoctorDependencies {
            run_git_command: &run_git_command,
            check_git_available: &|| true,
            resolve_state_root: &resolve_state_root,
            resolve_global_config_path: &|| Ok(test_dir.path().join("config-root/sce/config.json")),
            resolve_agent_trace_local_db_path: &resolve_agent_trace_local_db_path,
            validate_config_file: &|_| Ok(()),
            check_agent_trace_local_db_health: &|_| Ok(()),
            install_required_git_hooks: &|_| unreachable!("hook install should not run"),
            create_directory_all: &|_| unreachable!("directory creation should not run"),
        };

        let json_request = DoctorRequest {
            mode: DoctorMode::Diagnose,
            database_inventory: DoctorDatabaseInventory::Repo,
            format: DoctorFormat::Json,
        };
        let execution = execute_doctor_with_dependencies(
            DoctorRequest {
                mode: DoctorMode::Diagnose,
                database_inventory: DoctorDatabaseInventory::Repo,
                format: DoctorFormat::Text,
            },
            &repository_root,
            &dependencies,
        );
        let output = render_report(json_request, &execution)?;
        let parsed: Value = serde_json::from_str(&output)?;

        assert_eq!(parsed["readiness"], "ready");
        let problems = parsed["problems"].as_array().expect("problems array");
        assert!(problems.iter().any(|problem| {
            problem_matches(
                problem,
                "repo_assets",
                "warning",
                "manual_only",
                "is missing",
            )
        }));
        Ok(())
    }

    #[test]
    fn doctor_reports_opencode_plugin_runtime_missing_warning() -> Result<()> {
        let test_dir = TestDir::new("doctor-opencode-runtime-missing")?;
        let repository_root = test_dir.path().join("repo");
        let hooks_dir = repository_root.join(".git").join("hooks");
        install_canonical_hooks(&hooks_dir)?;

        let opencode_root = repository_root.join(".opencode");
        fs::create_dir_all(&opencode_root)?;
        fs::create_dir_all(opencode_root.join("agent"))?;
        fs::create_dir_all(opencode_root.join("command"))?;
        fs::create_dir_all(opencode_root.join("skills"))?;
        fs::write(
            opencode_root.join("opencode.json"),
            "{\"plugin\":[\"./plugins/sce-bash-policy.ts\"]}",
        )?;

        let canonical_plugin = super::opencode_plugin_asset()
            .expect("canonical OpenCode plugin asset should be embedded");
        let plugin_path = opencode_root.join("plugins").join("sce-bash-policy.ts");
        fs::create_dir_all(
            plugin_path
                .parent()
                .expect("plugin path should have parent"),
        )?;
        fs::write(&plugin_path, canonical_plugin.bytes)?;

        let preset_path = opencode_root.join("lib").join("bash-policy-presets.json");
        fs::create_dir_all(
            preset_path
                .parent()
                .expect("preset path should have parent"),
        )?;
        fs::write(&preset_path, "{}")?;

        let agent_trace_db = test_dir
            .path()
            .join("state-root")
            .join("sce")
            .join("agent-trace")
            .join("local.db");
        fs::create_dir_all(
            agent_trace_db
                .parent()
                .expect("agent trace path should have parent"),
        )?;

        let repo_root = repository_root.clone();
        let hooks_dir = hooks_dir.clone();
        let run_git_command = move |_cwd: &Path, args: &[&str]| match args {
            ["rev-parse", "--show-toplevel"] => Some(repo_root.display().to_string()),
            ["rev-parse", "--is-bare-repository"] => Some("false".to_string()),
            ["rev-parse", "--git-path", "hooks"] => Some(hooks_dir.display().to_string()),
            _ => None,
        };

        let state_root = test_dir.path().join("state-root");
        let resolve_state_root = move || Ok(state_root.clone());
        let resolve_agent_trace_local_db_path = move || Ok(agent_trace_db.clone());

        let dependencies = DoctorDependencies {
            run_git_command: &run_git_command,
            check_git_available: &|| true,
            resolve_state_root: &resolve_state_root,
            resolve_global_config_path: &|| Ok(test_dir.path().join("config-root/sce/config.json")),
            resolve_agent_trace_local_db_path: &resolve_agent_trace_local_db_path,
            validate_config_file: &|_| Ok(()),
            check_agent_trace_local_db_health: &|_| Ok(()),
            install_required_git_hooks: &|_| unreachable!("hook install should not run"),
            create_directory_all: &|_| unreachable!("directory creation should not run"),
        };

        let json_request = DoctorRequest {
            mode: DoctorMode::Diagnose,
            database_inventory: DoctorDatabaseInventory::Repo,
            format: DoctorFormat::Json,
        };
        let execution = execute_doctor_with_dependencies(
            DoctorRequest {
                mode: DoctorMode::Diagnose,
                database_inventory: DoctorDatabaseInventory::Repo,
                format: DoctorFormat::Text,
            },
            &repository_root,
            &dependencies,
        );
        let output = render_report(json_request, &execution)?;
        let parsed: Value = serde_json::from_str(&output)?;

        assert_eq!(parsed["readiness"], "ready");
        let problems = parsed["problems"].as_array().expect("problems array");
        assert!(problems.iter().any(|problem| {
            problem_matches(
                problem,
                "repo_assets",
                "warning",
                "manual_only",
                "bash-policy runtime",
            )
        }));
        Ok(())
    }

    #[test]
    fn doctor_reports_opencode_plugin_preset_catalog_missing_warning() -> Result<()> {
        let test_dir = TestDir::new("doctor-opencode-preset-missing")?;
        let repository_root = test_dir.path().join("repo");
        let hooks_dir = repository_root.join(".git").join("hooks");
        install_canonical_hooks(&hooks_dir)?;

        let opencode_root = repository_root.join(".opencode");
        fs::create_dir_all(&opencode_root)?;
        fs::create_dir_all(opencode_root.join("agent"))?;
        fs::create_dir_all(opencode_root.join("command"))?;
        fs::create_dir_all(opencode_root.join("skills"))?;
        fs::write(
            opencode_root.join("opencode.json"),
            "{\"plugin\":[\"./plugins/sce-bash-policy.ts\"]}",
        )?;

        let canonical_plugin = super::opencode_plugin_asset()
            .expect("canonical OpenCode plugin asset should be embedded");
        let plugin_path = opencode_root.join("plugins").join("sce-bash-policy.ts");
        fs::create_dir_all(
            plugin_path
                .parent()
                .expect("plugin path should have parent"),
        )?;
        fs::write(&plugin_path, canonical_plugin.bytes)?;

        let runtime_path = opencode_root
            .join("plugins")
            .join("bash-policy")
            .join("runtime.ts");
        fs::create_dir_all(
            runtime_path
                .parent()
                .expect("runtime path should have parent"),
        )?;
        fs::write(&runtime_path, "runtime")?;

        let agent_trace_db = test_dir
            .path()
            .join("state-root")
            .join("sce")
            .join("agent-trace")
            .join("local.db");
        fs::create_dir_all(
            agent_trace_db
                .parent()
                .expect("agent trace path should have parent"),
        )?;

        let repo_root = repository_root.clone();
        let hooks_dir = hooks_dir.clone();
        let run_git_command = move |_cwd: &Path, args: &[&str]| match args {
            ["rev-parse", "--show-toplevel"] => Some(repo_root.display().to_string()),
            ["rev-parse", "--is-bare-repository"] => Some("false".to_string()),
            ["rev-parse", "--git-path", "hooks"] => Some(hooks_dir.display().to_string()),
            _ => None,
        };

        let state_root = test_dir.path().join("state-root");
        let resolve_state_root = move || Ok(state_root.clone());
        let resolve_agent_trace_local_db_path = move || Ok(agent_trace_db.clone());

        let dependencies = DoctorDependencies {
            run_git_command: &run_git_command,
            check_git_available: &|| true,
            resolve_state_root: &resolve_state_root,
            resolve_global_config_path: &|| Ok(test_dir.path().join("config-root/sce/config.json")),
            resolve_agent_trace_local_db_path: &resolve_agent_trace_local_db_path,
            validate_config_file: &|_| Ok(()),
            check_agent_trace_local_db_health: &|_| Ok(()),
            install_required_git_hooks: &|_| unreachable!("hook install should not run"),
            create_directory_all: &|_| unreachable!("directory creation should not run"),
        };

        let json_request = DoctorRequest {
            mode: DoctorMode::Diagnose,
            database_inventory: DoctorDatabaseInventory::Repo,
            format: DoctorFormat::Json,
        };
        let execution = execute_doctor_with_dependencies(
            DoctorRequest {
                mode: DoctorMode::Diagnose,
                database_inventory: DoctorDatabaseInventory::Repo,
                format: DoctorFormat::Text,
            },
            &repository_root,
            &dependencies,
        );
        let output = render_report(json_request, &execution)?;
        let parsed: Value = serde_json::from_str(&output)?;

        assert_eq!(parsed["readiness"], "ready");
        let problems = parsed["problems"].as_array().expect("problems array");
        assert!(problems.iter().any(|problem| {
            problem_matches(
                problem,
                "repo_assets",
                "warning",
                "manual_only",
                "preset catalog",
            )
        }));
        Ok(())
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn render_all_database_inventory_json_includes_agent_trace_record() -> Result<()> {
        let test_dir = TestDir::new("doctor-all-databases")?;
        let repository_root = test_dir.path().join("repo");
        let hooks_dir = repository_root.join(".git").join("hooks");
        install_canonical_hooks(&hooks_dir)?;

        let agent_trace_db = test_dir
            .path()
            .join("state-root")
            .join("sce")
            .join("agent-trace")
            .join("local.db");
        fs::create_dir_all(
            agent_trace_db
                .parent()
                .expect("agent trace path should have parent"),
        )?;
        fs::write(&agent_trace_db, [])?;

        let state_root = test_dir.path().join("state-root");

        let repo_root = repository_root.clone();
        let hooks_dir = hooks_dir.clone();
        let run_git_command = move |_cwd: &Path, args: &[&str]| match args {
            ["rev-parse", "--show-toplevel"] => Some(repo_root.display().to_string()),
            ["rev-parse", "--is-bare-repository"] => Some("false".to_string()),
            ["rev-parse", "--git-path", "hooks"] => Some(hooks_dir.display().to_string()),
            _ => None,
        };
        let dependencies = DoctorDependencies {
            run_git_command: &run_git_command,
            check_git_available: &|| true,
            resolve_state_root: &|| Ok(state_root.clone()),
            resolve_global_config_path: &|| Ok(test_dir.path().join("config-root/sce/config.json")),
            resolve_agent_trace_local_db_path: &|| Ok(agent_trace_db.clone()),
            validate_config_file: &|_| Ok(()),
            check_agent_trace_local_db_health: &|_| Ok(()),
            install_required_git_hooks: &|_| unreachable!("hook install should not run"),
            create_directory_all: &|_| unreachable!("directory creation should not run"),
        };

        let request = DoctorRequest {
            mode: DoctorMode::Diagnose,
            database_inventory: DoctorDatabaseInventory::All,
            format: DoctorFormat::Json,
        };
        let execution = execute_doctor_with_dependencies(request, &repository_root, &dependencies);
        let output = render_report(request, &execution)?;

        let parsed: Value = serde_json::from_str(&output)?;
        assert_eq!(parsed["database_inventory"], "all");
        assert_eq!(parsed["all_databases"][0]["family"], "agent_trace_local");
        assert_eq!(parsed["all_databases"].as_array().map(Vec::len), Some(1));
        assert_eq!(parsed["repo_databases"].as_array().map(Vec::len), Some(0));
        Ok(())
    }

    #[test]
    fn fix_mode_creates_missing_agent_trace_directory() -> Result<()> {
        let test_dir = TestDir::new("agent-trace-fix")?;
        let repository_root = test_dir.path().join("repo");
        let hooks_dir = repository_root.join(".git").join("hooks");
        install_canonical_hooks(&hooks_dir)?;

        let db_path = test_dir
            .path()
            .join("state-root")
            .join("sce")
            .join("agent-trace")
            .join("local.db");
        let created_paths = Arc::new(Mutex::new(Vec::new()));
        let repo_root = repository_root.clone();
        let hooks_dir = hooks_dir.clone();
        let db_path_for_state_root = db_path.clone();
        let db_path_for_resolution = db_path.clone();
        let created_paths_for_fix = Arc::clone(&created_paths);
        let run_git_command = move |_cwd: &Path, args: &[&str]| match args {
            ["rev-parse", "--show-toplevel"] => Some(repo_root.display().to_string()),
            ["rev-parse", "--is-bare-repository"] => Some("false".to_string()),
            ["rev-parse", "--git-path", "hooks"] => Some(hooks_dir.display().to_string()),
            _ => None,
        };
        let check_git_available = || true;
        let resolve_state_root = move || {
            Ok(db_path_for_state_root
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .expect("db path should include state_root/sce/agent-trace/local.db"))
        };
        let resolve_agent_trace_local_db_path = move || Ok(db_path_for_resolution.clone());
        let validate_config_file = |_path: &Path| Ok(());
        let check_agent_trace_local_db_health = |_path: &Path| Ok(());
        let install_required_git_hooks = |_repo: &Path| {
            Ok(RequiredHooksInstallOutcome {
                repository_root: PathBuf::new(),
                hooks_directory: PathBuf::new(),
                hook_results: Vec::new(),
            })
        };
        let create_directory_all = move |path: &Path| {
            created_paths_for_fix
                .lock()
                .expect("lock poisoned")
                .push(path.to_path_buf());
            fs::create_dir_all(path).map_err(anyhow::Error::from)
        };
        let dependencies = DoctorDependencies {
            run_git_command: &run_git_command,
            check_git_available: &check_git_available,
            resolve_state_root: &resolve_state_root,
            resolve_global_config_path: &|| Ok(test_dir.path().join("config-root/sce/config.json")),
            resolve_agent_trace_local_db_path: &resolve_agent_trace_local_db_path,
            validate_config_file: &validate_config_file,
            check_agent_trace_local_db_health: &check_agent_trace_local_db_health,
            install_required_git_hooks: &install_required_git_hooks,
            create_directory_all: &create_directory_all,
        };

        let execution = execute_doctor_with_dependencies(
            DoctorRequest {
                mode: DoctorMode::Fix,
                database_inventory: DoctorDatabaseInventory::Repo,
                format: DoctorFormat::Text,
            },
            &repository_root,
            &dependencies,
        );

        assert_eq!(execution.report.readiness, Readiness::Ready);
        assert!(db_path.parent().is_some_and(Path::exists));
        assert!(execution.report.problems.is_empty());
        assert!(execution.fix_results.iter().any(|result| {
            result.category == ProblemCategory::FilesystemPermissions
                && result.outcome == FixResult::Fixed
                && result
                    .detail
                    .contains("Created the SCE-owned Agent Trace directory")
        }));
        assert_eq!(created_paths.lock().expect("lock poisoned").len(), 1);
        Ok(())
    }

    #[test]
    fn filesystem_auto_fix_refuses_non_canonical_directory() {
        let report = HookDoctorReport {
            mode: DoctorMode::Fix,
            database_inventory: DoctorDatabaseInventory::Repo,
            readiness: Readiness::NotReady,
            state_root: None,
            repository_root: None,
            hook_path_source: HookPathSource::Default,
            hooks_directory: None,
            config_locations: Vec::new(),
            agent_trace_local_db: Some(FileLocationHealth {
                label: "Agent Trace local DB",
                path: PathBuf::from("/tmp/unexpected/local.db"),
                state: "expected",
            }),
            repo_databases: Vec::new(),
            all_databases: Vec::new(),
            hooks: Vec::new(),
            problems: vec![filesystem_problem(
                "Agent Trace local DB parent directory is missing.",
            )],
        };

        let dependencies = DoctorDependencies {
            run_git_command: &|_, _| None,
            check_git_available: &|| false,
            resolve_state_root: &|| Ok(PathBuf::from("/tmp/state-root")),
            resolve_global_config_path: &|| Ok(PathBuf::from("/tmp/config-root/sce/config.json")),
            resolve_agent_trace_local_db_path: &|| {
                Ok(PathBuf::from("/tmp/canonical/sce/agent-trace/local.db"))
            },
            validate_config_file: &|_| Ok(()),
            check_agent_trace_local_db_health: &|_| Ok(()),
            install_required_git_hooks: &|_| unreachable!("hook install should not run"),
            create_directory_all: &|_| unreachable!("directory creation should be refused"),
        };

        let fix_results = run_filesystem_auto_fixes(&report, &dependencies);

        assert_eq!(fix_results.len(), 1);
        assert_eq!(
            fix_results[0].category,
            ProblemCategory::FilesystemPermissions
        );
        assert_eq!(fix_results[0].outcome, FixResult::Failed);
        assert!(fix_results[0]
            .detail
            .contains("does not match the canonical SCE-owned path"));
    }
}
