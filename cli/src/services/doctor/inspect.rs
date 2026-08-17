use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::services::agent_trace_db::lifecycle::diagnose_agent_trace_db_health;
use crate::services::checkout;
use crate::services::config::schema::parse_file_config;
use crate::services::config::{self, ConfigPathSource, IntegrationTargetId};
use crate::services::default_paths::{
    agent_trace_db_path_for_repository, claude_asset, opencode_asset, pi_asset, InstallTargetPaths,
    RepoPaths,
};
use crate::services::repository_identity::resolve::{
    resolve_repository_identity, RepositoryIdentitySource,
};
use crate::services::setup::{
    config_merge, hook_merge, iter_embedded_assets_for_setup_target_with_selection,
    iter_required_hook_assets, persisted_optional_workflows, repair_merge_target_asset,
    EmbeddedAsset, SetupTarget,
};

use super::types::{
    AgentTraceDbHealth, CheckoutIdentityHealth, DoctorFixResultRecord, DoctorProblem,
    FileLocationHealth, FixResult, GlobalStateHealth, HookContentState, HookDoctorReport,
    HookFileHealth, HookPathSource, IntegrationArea, IntegrationChildHealth,
    IntegrationContentState, IntegrationGroupHealth, IntegrationGroupKey, IntegrationTarget,
    ProblemCategory, ProblemFixability, ProblemKind, ProblemSeverity, Readiness,
};
use super::{is_executable, DoctorDependencies, DoctorMode, REQUIRED_HOOKS};

pub(super) fn build_report_with_lifecycle_problems(
    mode: DoctorMode,
    repository_root: &Path,
    dependencies: &DoctorDependencies<'_>,
    lifecycle_problems: Vec<DoctorProblem>,
) -> HookDoctorReport {
    let mut report = build_report_without_service_owned_problem_checks(
        mode,
        repository_root,
        dependencies,
        lifecycle_problems,
    );
    report.checkout_identity = collect_checkout_identity_health(repository_root);
    report.agent_trace_db = collect_agent_trace_db_health(repository_root, &mut report.problems);
    report.readiness = if report
        .problems
        .iter()
        .any(|problem| problem.severity == ProblemSeverity::Error)
    {
        Readiness::NotReady
    } else {
        Readiness::Ready
    };
    report
}

fn build_report_without_service_owned_problem_checks(
    mode: DoctorMode,
    repository_root: &Path,
    dependencies: &DoctorDependencies<'_>,
    mut problems: Vec<DoctorProblem>,
) -> HookDoctorReport {
    let global_state = collect_global_state_locations(repository_root, dependencies);
    let checkout_identity = collect_checkout_identity_health(repository_root);
    let agent_trace_db = collect_agent_trace_db_health(repository_root, &mut problems);
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

    let hooks = if git_available && !bare_repository && detected_repository_root.is_some() {
        hooks_directory
            .as_deref()
            .map(collect_hook_file_health)
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let integration_targets_absent = should_show_no_integrations_message(
        git_available,
        bare_repository,
        detected_repository_root.as_deref(),
    );
    let integration_groups = inspect_repository_integrations(
        git_available,
        bare_repository,
        detected_repository_root.as_deref(),
        &mut problems,
    );

    HookDoctorReport {
        mode,
        readiness: Readiness::Ready,
        state_root: global_state.state_root,
        checkout_identity,
        agent_trace_db,
        repository_root: detected_repository_root,
        hook_path_source,
        hooks_directory,
        config_locations: global_state.config_locations,
        hooks,
        integration_groups,
        integration_targets_absent,
        problems,
    }
}

fn collect_global_state_locations(
    repository_root: &Path,
    dependencies: &DoctorDependencies<'_>,
) -> GlobalStateHealth {
    let state_root =
        (dependencies.resolve_state_root)()
            .ok()
            .map(|state_root| FileLocationHealth {
                label: "State root",
                state: if state_root.exists() {
                    "present"
                } else {
                    "expected"
                },
                path: state_root,
            });

    let mut config_locations = Vec::new();
    if let Ok(global_path) = (dependencies.resolve_global_config_path)() {
        config_locations.push(FileLocationHealth {
            label: "Global config",
            state: if global_path.exists() {
                "present"
            } else {
                "expected"
            },
            path: global_path,
        });
    }

    let local_path = RepoPaths::new(repository_root).sce_config_file();
    config_locations.push(FileLocationHealth {
        label: "Local config",
        state: if local_path.exists() {
            "present"
        } else {
            "expected"
        },
        path: local_path,
    });

    GlobalStateHealth {
        state_root,
        config_locations,
    }
}

fn collect_checkout_identity_health(repository_root: &Path) -> Option<CheckoutIdentityHealth> {
    let git_dir = checkout::resolve_git_dir(repository_root).ok()?;
    let checkout_id = checkout::read_checkout_id(&git_dir).ok()??;

    Some(CheckoutIdentityHealth { checkout_id })
}

fn collect_agent_trace_db_health(
    repository_root: &Path,
    problems: &mut Vec<DoctorProblem>,
) -> Option<AgentTraceDbHealth> {
    let agent_trace_problems = diagnose_agent_trace_db_health(Some(repository_root));
    let mut agent_trace_db = None;

    for problem in &agent_trace_problems {
        if matches!(
            problem.kind,
            crate::services::lifecycle::HealthProblemKind::UnableToResolveStateRoot
        ) {
            problems.push(DoctorProblem {
                kind: ProblemKind::UnableToResolveStateRoot,
                category: ProblemCategory::GlobalState,
                severity: ProblemSeverity::Error,
                fixability: ProblemFixability::ManualOnly,
                summary: problem.summary.clone(),
                remediation: problem.remediation.clone(),
                next_action: problem.next_action,
            });
            continue;
        }

        agent_trace_db = resolve_agent_trace_db_location(repository_root);
    }

    if agent_trace_db.is_none() {
        agent_trace_db = resolve_agent_trace_db_location(repository_root);
    }

    agent_trace_db
}

fn resolve_agent_trace_db_location(repository_root: &Path) -> Option<AgentTraceDbHealth> {
    let storage_config =
        config::resolve_agent_trace_storage_runtime_config(repository_root).ok()?;
    let resolved = resolve_repository_identity(
        repository_root,
        storage_config.repository_id.as_deref(),
        &storage_config.repository_remote,
    )
    .ok()?;
    let db_path = agent_trace_db_path_for_repository(&resolved.identity.repository_id).ok()?;
    let state = if db_path.exists() {
        "present"
    } else {
        "expected"
    };
    let (identity_source, configured_remote) = match resolved.source {
        RepositoryIdentitySource::ExplicitConfig => (String::from("explicit_config"), None),
        RepositoryIdentitySource::RemoteUrl { remote_name } => {
            (String::from("remote_url"), Some(remote_name))
        }
    };

    Some(AgentTraceDbHealth {
        label: "Agent Trace repository DB",
        path: db_path,
        state,
        repository_id: resolved.identity.repository_id,
        canonical_identity: resolved.identity.canonical_identity,
        identity_source,
        configured_remote,
    })
}

fn collect_hook_file_health(directory: &Path) -> Vec<HookFileHealth> {
    REQUIRED_HOOKS
        .iter()
        .map(|hook_name| {
            let hook_path = directory.join(hook_name);
            let metadata = fs::metadata(&hook_path).ok();
            let exists = metadata.is_some();
            let executable = metadata
                .as_ref()
                .is_some_and(|entry| entry.is_file() && is_executable(entry));
            let content_state =
                inspect_hook_content_state_without_problem(hook_name, &hook_path, exists);

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

fn inspect_hook_content_state_without_problem(
    hook_name: &str,
    hook_path: &Path,
    exists: bool,
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
        Ok(bytes) => hook_managed_block_content_state(hook_name, &bytes, expected_hook.bytes),
        Err(_) => HookContentState::Unknown,
    }
}

/// Classifies a hook's on-disk bytes against the canonical template by SCE
/// managed-block currency (merging the canonical block into `bytes` is a
/// no-op) rather than whole-file equality, so foreign content a repository
/// has appended around the block does not read as drift. An unbalanced or
/// partial managed block is also reported `Stale`, since it needs the same
/// `--fix` repair as a drifted one.
fn hook_managed_block_content_state(
    hook_name: &str,
    bytes: &[u8],
    canonical: &[u8],
) -> HookContentState {
    match hook_merge::merge_or_create_hook(Some(bytes), canonical, hook_name) {
        Ok(merge) if merge.bytes == bytes => HookContentState::Current,
        Ok(_) | Err(_) => HookContentState::Stale,
    }
}

#[allow(dead_code)]
fn inspect_repository_hooks(
    repository_root: &Path,
    git_available: bool,
    bare_repository: bool,
    detected_repository_root: Option<&Path>,
    hooks_directory: Option<&Path>,
    problems: &mut Vec<DoctorProblem>,
) -> Vec<HookFileHealth> {
    if !git_available {
        problems.push(DoctorProblem {
            kind: ProblemKind::GitUnavailable,
            category: ProblemCategory::RepositoryTargeting,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: String::from("Git is not available on this machine."),
            remediation: String::from("Install an accessible 'git' binary and ensure it is on PATH before rerunning 'sce doctor'."),
            next_action: "manual_steps",
        });
        return Vec::new();
    }

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
        return Vec::new();
    }

    if detected_repository_root.is_none() {
        problems.push(DoctorProblem {
            kind: ProblemKind::NotInsideGitRepository,
            category: ProblemCategory::RepositoryTargeting,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: String::from("The current directory is not inside a git repository."),
            remediation: String::from("Run 'sce doctor' from inside the target repository working tree to inspect repo-scoped SCE hook health."),
            next_action: "manual_steps",
        });
        return Vec::new();
    }

    if let Some(directory) = hooks_directory {
        return collect_hook_health(directory, problems);
    }

    let _ = repository_root;
    problems.push(DoctorProblem {
        kind: ProblemKind::UnableToResolveGitHooksDirectory,
        category: ProblemCategory::RepositoryTargeting,
        severity: ProblemSeverity::Error,
        fixability: ProblemFixability::ManualOnly,
        summary: String::from("Unable to resolve git hooks directory."),
        remediation: String::from("Verify that git repository inspection succeeds and rerun 'sce doctor' inside a non-bare git repository."),
        next_action: "manual_steps",
    });
    Vec::new()
}

/// Returns `true` when the doctor was able to check for integration targets
/// and found none (neither configured in `.sce/config.json` nor detected
/// via repo-root `.opencode/` / `.claude/` / `.pi/` directories).
fn should_show_no_integrations_message(
    git_available: bool,
    bare_repository: bool,
    detected_repository_root: Option<&Path>,
) -> bool {
    if !git_available || bare_repository {
        return false;
    }
    let Some(root) = detected_repository_root else {
        return false;
    };
    resolve_doctor_integration_targets(root).is_empty()
}

fn resolve_doctor_integration_targets(repository_root: &Path) -> Vec<IntegrationTargetId> {
    let repo_paths = RepoPaths::new(repository_root);
    let config_path = repo_paths.sce_config_file();

    // Try reading config first
    if config_path.exists() {
        if let Ok(raw) = std::fs::read_to_string(&config_path) {
            if let Ok(config) =
                parse_file_config(&raw, &config_path, ConfigPathSource::DefaultDiscoveredLocal)
            {
                if let Some(integrations) = config.integrations {
                    // integrations key present with a target property
                    if integrations.value.target.is_empty() {
                        // Empty target array — user has not recorded any integration targets
                        return Vec::new();
                    }
                    // Non-empty configured targets
                    return integrations.value.target;
                }
            }
        }
    }

    // Fallback: no integrations config — detect installed directories
    let mut detected = Vec::new();
    if repo_paths.opencode_dir().exists() {
        detected.push(IntegrationTargetId::Opencode);
    }
    if repo_paths.claude_dir().exists() {
        detected.push(IntegrationTargetId::Claude);
    }
    if repo_paths.pi_dir().exists() {
        detected.push(IntegrationTargetId::Pi);
    }
    detected
}

fn inspect_repository_integrations(
    git_available: bool,
    bare_repository: bool,
    detected_repository_root: Option<&Path>,
    problems: &mut Vec<DoctorProblem>,
) -> Vec<IntegrationGroupHealth> {
    if !git_available || bare_repository {
        return Vec::new();
    }

    let Some(resolved_root) = detected_repository_root else {
        return Vec::new();
    };

    let targets = resolve_doctor_integration_targets(resolved_root);
    if targets.is_empty() {
        problems.push(DoctorProblem {
            kind: ProblemKind::NoIntegrationsInstalled,
            category: ProblemCategory::RepoAssets,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: String::from(
                "No integrations are installed. Run 'sce setup' to install OpenCode, Claude, and/or Pi integration assets.",
            ),
            remediation: String::from(
                "Run 'sce setup --opencode', 'sce setup --claude', 'sce setup --pi', or 'sce setup --all' to install integration assets.",
            ),
            next_action: "manual_steps",
        });
        return Vec::new();
    }
    let mut integration_groups = Vec::new();

    // An optional workflow's assets are expected on disk only when repo-local
    // config records it as selected; an unrecorded selection means not selected.
    let selected_optional_workflows = persisted_optional_workflows(resolved_root);

    for target in &targets {
        match target {
            IntegrationTargetId::Opencode => {
                let opencode_groups = collect_opencode_integration_groups(
                    resolved_root,
                    &selected_optional_workflows,
                );
                inspect_opencode_integration_health(resolved_root, &opencode_groups, problems);
                integration_groups.extend(opencode_groups);
            }
            IntegrationTargetId::Claude => {
                let claude_groups =
                    collect_claude_integration_groups(resolved_root, &selected_optional_workflows);
                inspect_claude_integration_health(&claude_groups, problems);
                integration_groups.extend(claude_groups);
            }
            IntegrationTargetId::Pi => {
                let pi_groups =
                    collect_pi_integration_groups(resolved_root, &selected_optional_workflows);
                inspect_pi_integration_health(&pi_groups, problems);
                integration_groups.extend(pi_groups);
            }
        }
    }

    integration_groups
}

/// Repairs each merge-target asset (`.claude/settings.json`,
/// `.opencode/opencode.json`) whose SCE-owned fragment is currently missing or
/// stale, by reinstalling just that asset through the same merge-install path
/// `sce setup` uses. Assets whose fragment is already current are left
/// untouched, and a fully missing integration is left to the existing
/// "reinstall assets" guidance rather than being created here.
pub(super) fn repair_merge_target_configs(repository_root: &Path) -> Vec<DoctorFixResultRecord> {
    let targets = resolve_doctor_integration_targets(repository_root);
    let selected_optional_workflows = persisted_optional_workflows(repository_root);
    let mut results = Vec::new();

    if targets.contains(&IntegrationTargetId::Claude) {
        let claude_groups =
            collect_claude_integration_groups(repository_root, &selected_optional_workflows);
        if let Some(result) = repair_merge_target_if_mismatched(
            repository_root,
            SetupTarget::Claude,
            claude_asset::SETTINGS_FILE,
            &claude_groups,
        ) {
            results.push(result);
        }
    }

    if targets.contains(&IntegrationTargetId::Opencode) {
        let opencode_groups =
            collect_opencode_integration_groups(repository_root, &selected_optional_workflows);
        if let Some(result) = repair_merge_target_if_mismatched(
            repository_root,
            SetupTarget::OpenCode,
            OPENCODE_CONFIG_RELATIVE_PATH,
            &opencode_groups,
        ) {
            results.push(result);
        }
    }

    results
}

fn repair_merge_target_if_mismatched(
    repository_root: &Path,
    target: SetupTarget,
    relative_path: &str,
    groups: &[IntegrationGroupHealth],
) -> Option<DoctorFixResultRecord> {
    let is_mismatched = groups
        .iter()
        .flat_map(|group| &group.children)
        .any(|child| {
            child.relative_path == relative_path
                && matches!(child.content_state, IntegrationContentState::Mismatch)
        });
    if !is_mismatched {
        return None;
    }

    Some(
        match repair_merge_target_asset(repository_root, target, relative_path) {
            Ok(()) => DoctorFixResultRecord {
                category: ProblemCategory::RepoAssets,
                outcome: FixResult::Fixed,
                detail: format!("Merged canonical SCE fragments into '{relative_path}'."),
            },
            Err(error) => DoctorFixResultRecord {
                category: ProblemCategory::RepoAssets,
                outcome: FixResult::Failed,
                detail: format!(
                    "Failed to merge canonical SCE fragments into '{relative_path}': {error}"
                ),
            },
        },
    )
}

#[allow(dead_code)]
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
            kind: ProblemKind::UnableToResolveStateRoot,
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
                        kind: ProblemKind::GlobalConfigValidationFailed,
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
            kind: ProblemKind::UnableToResolveGlobalConfigPath,
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
                kind: ProblemKind::LocalConfigValidationFailed,
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

    GlobalStateHealth {
        state_root: state_root_health,
        config_locations,
    }
}

#[allow(dead_code)]
fn collect_hook_health(directory: &Path, problems: &mut Vec<DoctorProblem>) -> Vec<HookFileHealth> {
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

fn inspect_opencode_integration_health(
    repository_root: &Path,
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    push_opencode_integration_missing_problems(integration_groups, problems);
    push_opencode_integration_mismatch_problems(integration_groups, problems);
    push_opencode_integration_read_fail_problems(integration_groups, problems);
    inspect_opencode_plugin_registry_health(repository_root, problems);

    let install_targets = InstallTargetPaths::new(repository_root);
    inspect_opencode_plugin_dependency_health(&install_targets, problems);
}

fn inspect_claude_integration_health(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    push_claude_integration_missing_problems(integration_groups, problems);
    push_claude_integration_mismatch_problems(integration_groups, problems);
    push_claude_integration_read_fail_problems(integration_groups, problems);
}

fn inspect_pi_integration_health(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    push_pi_integration_missing_problems(integration_groups, problems);
    push_pi_integration_mismatch_problems(integration_groups, problems);
    push_pi_integration_read_fail_problems(integration_groups, problems);
}

fn push_opencode_integration_missing_problems(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    for group in integration_groups {
        let missing_children = group
            .children
            .iter()
            .filter(|child| matches!(&child.content_state, IntegrationContentState::Missing))
            .collect::<Vec<_>>();
        if missing_children.is_empty() {
            continue;
        }

        let missing_paths = missing_children
            .iter()
            .map(|child| format!("'{}'", child.path.display()))
            .collect::<Vec<_>>()
            .join(", ");
        problems.push(DoctorProblem {
            kind: ProblemKind::OpenCodeIntegrationFilesMissing,
            category: ProblemCategory::RepoAssets,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!(
                "{} required file(s) are missing: {}.",
                group.display_label(), missing_paths
            ),
            remediation: format!(
                "Reinstall repo-root OpenCode assets to restore the missing {} file(s), then rerun 'sce doctor'.",
                group.display_label().to_ascii_lowercase()
            ),
            next_action: "manual_steps",
        });
    }
}

fn push_opencode_integration_mismatch_problems(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    for group in integration_groups {
        let mismatched_children = group
            .children
            .iter()
            .filter(|child| matches!(&child.content_state, IntegrationContentState::Mismatch))
            .collect::<Vec<_>>();
        if mismatched_children.is_empty() {
            continue;
        }

        let mismatched_paths = mismatched_children
            .iter()
            .map(|child| format!("'{}'", child.path.display()))
            .collect::<Vec<_>>()
            .join(", ");
        problems.push(DoctorProblem {
            kind: ProblemKind::OpenCodeIntegrationContentMismatch,
            category: ProblemCategory::RepoAssets,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!(
                "{} file(s) differ from the canonical embedded content: {}.",
                group.display_label(), mismatched_paths
            ),
            remediation: format!(
                "Reinstall repo-root OpenCode assets to restore the canonical {} content, then rerun 'sce doctor'.",
                group.display_label().to_ascii_lowercase()
            ),
            next_action: "manual_steps",
        });
    }
}

fn push_opencode_integration_read_fail_problems(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    for group in integration_groups {
        for child in &group.children {
            let IntegrationContentState::ReadFailed(error) = &child.content_state else {
                continue;
            };
            problems.push(DoctorProblem {
                kind: ProblemKind::OpenCodeAssetReadFailed,
                category: ProblemCategory::FilesystemPermissions,
                severity: ProblemSeverity::Error,
                fixability: ProblemFixability::ManualOnly,
                summary: format!(
                    "Unable to read OpenCode asset '{}' at '{}': {error}",
                    child.relative_path,
                    child.path.display()
                ),
                remediation: format!(
                    "Verify that '{}' is readable before rerunning 'sce doctor'.",
                    child.path.display()
                ),
                next_action: "manual_steps",
            });
        }
    }
}

fn push_claude_integration_missing_problems(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    for group in integration_groups {
        let missing_children = group
            .children
            .iter()
            .filter(|child| matches!(&child.content_state, IntegrationContentState::Missing))
            .collect::<Vec<_>>();
        if missing_children.is_empty() {
            continue;
        }

        let missing_paths = missing_children
            .iter()
            .map(|child| format!("'{}'", child.path.display()))
            .collect::<Vec<_>>()
            .join(", ");
        problems.push(DoctorProblem {
            kind: ProblemKind::ClaudeIntegrationFilesMissing,
            category: ProblemCategory::RepoAssets,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!(
                "{} required file(s) are missing: {}.",
                group.display_label(), missing_paths
            ),
            remediation: format!(
                "Reinstall repo-root Claude assets to restore the missing {} file(s), then rerun 'sce doctor'.",
                group.display_label().to_ascii_lowercase()
            ),
            next_action: "manual_steps",
        });
    }
}

fn push_claude_integration_mismatch_problems(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    for group in integration_groups {
        let mismatched_children = group
            .children
            .iter()
            .filter(|child| matches!(&child.content_state, IntegrationContentState::Mismatch))
            .collect::<Vec<_>>();
        if mismatched_children.is_empty() {
            continue;
        }

        let mismatched_paths = mismatched_children
            .iter()
            .map(|child| format!("'{}'", child.path.display()))
            .collect::<Vec<_>>()
            .join(", ");
        problems.push(DoctorProblem {
            kind: ProblemKind::ClaudeIntegrationContentMismatch,
            category: ProblemCategory::RepoAssets,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!(
                "{} file(s) differ from the canonical embedded content: {}.",
                group.display_label(), mismatched_paths
            ),
            remediation: format!(
                "Reinstall repo-root Claude assets to restore the canonical {} content, then rerun 'sce doctor'.",
                group.display_label().to_ascii_lowercase()
            ),
            next_action: "manual_steps",
        });
    }
}

fn push_claude_integration_read_fail_problems(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    for group in integration_groups {
        for child in &group.children {
            let IntegrationContentState::ReadFailed(error) = &child.content_state else {
                continue;
            };
            problems.push(DoctorProblem {
                kind: ProblemKind::ClaudeAssetReadFailed,
                category: ProblemCategory::FilesystemPermissions,
                severity: ProblemSeverity::Error,
                fixability: ProblemFixability::ManualOnly,
                summary: format!(
                    "Unable to read Claude asset '{}' at '{}': {error}",
                    child.relative_path,
                    child.path.display()
                ),
                remediation: format!(
                    "Verify that '{}' is readable before rerunning 'sce doctor'.",
                    child.path.display()
                ),
                next_action: "manual_steps",
            });
        }
    }
}

fn push_pi_integration_missing_problems(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    for group in integration_groups {
        let missing_children = group
            .children
            .iter()
            .filter(|child| matches!(&child.content_state, IntegrationContentState::Missing))
            .collect::<Vec<_>>();
        if missing_children.is_empty() {
            continue;
        }

        let missing_paths = missing_children
            .iter()
            .map(|child| format!("'{}'", child.path.display()))
            .collect::<Vec<_>>()
            .join(", ");
        problems.push(DoctorProblem {
            kind: ProblemKind::PiIntegrationFilesMissing,
            category: ProblemCategory::RepoAssets,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!(
                "{} required file(s) are missing: {}.",
                group.display_label(), missing_paths
            ),
            remediation: format!(
                "Reinstall repo-root Pi assets to restore the missing {} file(s), then rerun 'sce doctor'.",
                group.display_label().to_ascii_lowercase()
            ),
            next_action: "manual_steps",
        });
    }
}

fn push_pi_integration_mismatch_problems(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    for group in integration_groups {
        let mismatched_children = group
            .children
            .iter()
            .filter(|child| matches!(&child.content_state, IntegrationContentState::Mismatch))
            .collect::<Vec<_>>();
        if mismatched_children.is_empty() {
            continue;
        }

        let mismatched_paths = mismatched_children
            .iter()
            .map(|child| format!("'{}'", child.path.display()))
            .collect::<Vec<_>>()
            .join(", ");
        problems.push(DoctorProblem {
            kind: ProblemKind::PiIntegrationContentMismatch,
            category: ProblemCategory::RepoAssets,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!(
                "{} file(s) differ from the canonical embedded content: {}.",
                group.display_label(), mismatched_paths
            ),
            remediation: format!(
                "Reinstall repo-root Pi assets to restore the canonical {} content, then rerun 'sce doctor'.",
                group.display_label().to_ascii_lowercase()
            ),
            next_action: "manual_steps",
        });
    }
}

fn push_pi_integration_read_fail_problems(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    for group in integration_groups {
        for child in &group.children {
            let IntegrationContentState::ReadFailed(error) = &child.content_state else {
                continue;
            };
            problems.push(DoctorProblem {
                kind: ProblemKind::PiAssetReadFailed,
                category: ProblemCategory::FilesystemPermissions,
                severity: ProblemSeverity::Error,
                fixability: ProblemFixability::ManualOnly,
                summary: format!(
                    "Unable to read Pi asset '{}' at '{}': {error}",
                    child.relative_path,
                    child.path.display()
                ),
                remediation: format!(
                    "Verify that '{}' is readable before rerunning 'sce doctor'.",
                    child.path.display()
                ),
                next_action: "manual_steps",
            });
        }
    }
}

fn inspect_opencode_plugin_registry_health(
    repository_root: &Path,
    problems: &mut Vec<DoctorProblem>,
) {
    let repo_paths = RepoPaths::new(repository_root);
    let manifest_path = repo_paths.opencode_manifest_file();
    let manifest_metadata = fs::metadata(&manifest_path).ok();
    let manifest_is_file = manifest_metadata
        .as_ref()
        .is_some_and(std::fs::Metadata::is_file);
    if manifest_is_file {
        return;
    }

    let summary = if manifest_metadata.is_some() {
        format!(
            "OpenCode plugin registry path '{}' is not a file.",
            manifest_path.display()
        )
    } else {
        format!(
            "OpenCode plugin registry file '{}' is missing.",
            manifest_path.display()
        )
    };
    problems.push(DoctorProblem {
        kind: ProblemKind::OpenCodePluginRegistryInvalid,
        category: ProblemCategory::RepoAssets,
        severity: ProblemSeverity::Error,
        fixability: ProblemFixability::ManualOnly,
        summary,
        remediation: format!(
            "Reinstall OpenCode assets to restore the canonical plugin registry at '{}', then rerun 'sce doctor'.",
            manifest_path.display()
        ),
        next_action: "manual_steps",
    });
}

fn inspect_opencode_plugin_dependency_health(
    install_targets: &InstallTargetPaths,
    problems: &mut Vec<DoctorProblem>,
) {
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
        kind: ProblemKind::OpenCodeAssetMissingOrInvalid,
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

fn collect_opencode_integration_groups(
    repository_root: &Path,
    selected_optional_workflows: &[String],
) -> Vec<IntegrationGroupHealth> {
    let repo_paths = RepoPaths::new(repository_root);
    let opencode_root = repo_paths.opencode_dir();
    let manifest_path = repo_paths.opencode_manifest_file();
    let embedded_assets = iter_embedded_assets_for_setup_target_with_selection(
        SetupTarget::OpenCode,
        selected_optional_workflows,
    )
    .collect::<Vec<_>>();
    let mut plugin_children = Vec::new();
    let mut agent_children = Vec::new();
    let mut command_children = Vec::new();
    let mut skill_children = Vec::new();

    let manifest_child = embedded_assets
        .iter()
        .find(|asset| asset.relative_path == OPENCODE_CONFIG_RELATIVE_PATH)
        .map_or_else(
            || build_integration_child_presence_only("opencode.json", &manifest_path),
            |asset| {
                build_integration_child_from_asset(
                    &opencode_root,
                    asset,
                    Some(&MergeTargetAsset::OpenCodeConfig),
                )
            },
        );
    plugin_children.push(manifest_child);

    for asset in embedded_assets {
        if asset.relative_path == OPENCODE_CONFIG_RELATIVE_PATH {
            continue;
        }
        let child = build_integration_child_from_asset(&opencode_root, asset, None);

        if child
            .relative_path
            .starts_with(&format!("{}/", opencode_asset::PLUGINS_DIR))
            || child
                .relative_path
                .starts_with(&format!("{}/", opencode_asset::LIB_DIR))
        {
            plugin_children.push(child);
        } else if child
            .relative_path
            .starts_with(&format!("{}/", opencode_asset::OPENCODE_AGENT_DIR))
        {
            agent_children.push(child);
        } else if child
            .relative_path
            .starts_with(&format!("{}/", opencode_asset::OPENCODE_COMMAND_DIR))
        {
            command_children.push(child);
        } else if child
            .relative_path
            .starts_with(&format!("{}/", opencode_asset::SKILLS_DIR))
        {
            skill_children.push(child);
        }
    }

    sort_integration_children(&mut plugin_children);
    sort_integration_children(&mut agent_children);
    sort_integration_children(&mut command_children);
    sort_integration_children(&mut skill_children);

    vec![
        IntegrationGroupHealth::new(
            IntegrationGroupKey::new(IntegrationTarget::OpenCode, IntegrationArea::Plugins),
            plugin_children,
        ),
        IntegrationGroupHealth::new(
            IntegrationGroupKey::new(IntegrationTarget::OpenCode, IntegrationArea::Agents),
            agent_children,
        ),
        IntegrationGroupHealth::new(
            IntegrationGroupKey::new(IntegrationTarget::OpenCode, IntegrationArea::Commands),
            command_children,
        ),
        IntegrationGroupHealth::new(
            IntegrationGroupKey::new(IntegrationTarget::OpenCode, IntegrationArea::Skills),
            skill_children,
        ),
    ]
}

fn collect_claude_integration_groups(
    repository_root: &Path,
    selected_optional_workflows: &[String],
) -> Vec<IntegrationGroupHealth> {
    let repo_paths = RepoPaths::new(repository_root);
    let claude_root = repo_paths.claude_dir();
    let embedded_assets = iter_embedded_assets_for_setup_target_with_selection(
        SetupTarget::Claude,
        selected_optional_workflows,
    )
    .collect::<Vec<_>>();
    let mut plugin_children = Vec::new();
    let mut agent_children = Vec::new();
    let mut command_children = Vec::new();
    let mut skill_children = Vec::new();

    for asset in embedded_assets {
        let merge_target = if asset.relative_path == claude_asset::SETTINGS_FILE {
            Some(&MergeTargetAsset::ClaudeSettings)
        } else {
            None
        };
        let child = build_integration_child_from_asset(&claude_root, asset, merge_target);

        if child.relative_path == claude_asset::SETTINGS_FILE
            || child
                .relative_path
                .starts_with(&format!("{}/", claude_asset::HOOKS_DIR))
        {
            plugin_children.push(child);
        } else if child
            .relative_path
            .starts_with(&format!("{}/", claude_asset::AGENTS_DIR))
        {
            agent_children.push(child);
        } else if child
            .relative_path
            .starts_with(&format!("{}/", claude_asset::COMMANDS_DIR))
        {
            command_children.push(child);
        } else if child
            .relative_path
            .starts_with(&format!("{}/", claude_asset::SKILLS_DIR))
        {
            skill_children.push(child);
        }
    }

    sort_integration_children(&mut plugin_children);
    sort_integration_children(&mut agent_children);
    sort_integration_children(&mut command_children);
    sort_integration_children(&mut skill_children);

    vec![
        IntegrationGroupHealth::new(
            IntegrationGroupKey::new(IntegrationTarget::ClaudeCode, IntegrationArea::Plugins),
            plugin_children,
        ),
        IntegrationGroupHealth::new(
            IntegrationGroupKey::new(IntegrationTarget::ClaudeCode, IntegrationArea::Agents),
            agent_children,
        ),
        IntegrationGroupHealth::new(
            IntegrationGroupKey::new(IntegrationTarget::ClaudeCode, IntegrationArea::Commands),
            command_children,
        ),
        IntegrationGroupHealth::new(
            IntegrationGroupKey::new(IntegrationTarget::ClaudeCode, IntegrationArea::Skills),
            skill_children,
        ),
    ]
}

fn collect_pi_integration_groups(
    repository_root: &Path,
    selected_optional_workflows: &[String],
) -> Vec<IntegrationGroupHealth> {
    let repo_paths = RepoPaths::new(repository_root);
    let pi_root = repo_paths.pi_dir();
    let embedded_assets = iter_embedded_assets_for_setup_target_with_selection(
        SetupTarget::Pi,
        selected_optional_workflows,
    )
    .collect::<Vec<_>>();
    let mut prompt_children = Vec::new();
    let mut skill_children = Vec::new();
    let mut extension_children = Vec::new();

    for asset in embedded_assets {
        let child = build_integration_child_from_asset(&pi_root, asset, None);

        if child
            .relative_path
            .starts_with(&format!("{}/", pi_asset::PROMPTS_DIR))
        {
            prompt_children.push(child);
        } else if child
            .relative_path
            .starts_with(&format!("{}/", pi_asset::SKILLS_DIR))
        {
            skill_children.push(child);
        } else if child
            .relative_path
            .starts_with(&format!("{}/", pi_asset::EXTENSIONS_DIR))
        {
            extension_children.push(child);
        }
    }

    sort_integration_children(&mut prompt_children);
    sort_integration_children(&mut skill_children);
    sort_integration_children(&mut extension_children);

    vec![
        IntegrationGroupHealth::new(
            IntegrationGroupKey::new(IntegrationTarget::Pi, IntegrationArea::Prompts),
            prompt_children,
        ),
        IntegrationGroupHealth::new(
            IntegrationGroupKey::new(IntegrationTarget::Pi, IntegrationArea::Skills),
            skill_children,
        ),
        IntegrationGroupHealth::new(
            IntegrationGroupKey::new(IntegrationTarget::Pi, IntegrationArea::Extensions),
            extension_children,
        ),
    ]
}

fn sort_integration_children(children: &mut [IntegrationChildHealth]) {
    children.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
}

/// The relative path of the `OpenCode` merge-target asset within `.opencode/`.
const OPENCODE_CONFIG_RELATIVE_PATH: &str = "opencode.json";

/// Identifies the two setup assets that are installed by JSON merge
/// (`config_merge`) rather than whole-file replacement, and therefore need
/// SCE-fragment-based content inspection instead of byte-exact `sha256`.
enum MergeTargetAsset {
    ClaudeSettings,
    OpenCodeConfig,
}

fn build_integration_child_from_asset(
    integration_root: &Path,
    asset: &EmbeddedAsset,
    merge_target: Option<&MergeTargetAsset>,
) -> IntegrationChildHealth {
    let path = integration_root.join(asset.relative_path);
    let content_state = match merge_target {
        Some(MergeTargetAsset::ClaudeSettings) => inspect_merge_target_asset_state(
            &path,
            asset.bytes,
            config_merge::claude_settings_fragment_is_current,
        ),
        Some(MergeTargetAsset::OpenCodeConfig) => inspect_merge_target_asset_state(
            &path,
            asset.bytes,
            config_merge::opencode_config_fragment_is_current,
        ),
        None => inspect_integration_asset_state(&path, &asset.sha256),
    };
    IntegrationChildHealth {
        relative_path: asset.relative_path.to_string(),
        path,
        content_state,
    }
}

/// Content state for a merge-target asset: `Match` when the existing file
/// already carries a current, complete copy of the SCE-owned fragment
/// alongside whatever else it holds; `Mismatch` when that fragment is absent
/// or stale, or when the existing file cannot be parsed as JSON (a merge
/// cannot succeed either way, so both drift and hard-error surface the same
/// remediation: reinstall/`sce doctor --fix`).
fn inspect_merge_target_asset_state(
    path: &Path,
    generated_bytes: &[u8],
    fragment_is_current: fn(&[u8], &[u8]) -> anyhow::Result<bool>,
) -> IntegrationContentState {
    if !path_is_file(path) {
        return IntegrationContentState::Missing;
    }

    match fs::read(path) {
        Ok(existing_bytes) => {
            if fragment_is_current(&existing_bytes, generated_bytes).unwrap_or(false) {
                IntegrationContentState::Match
            } else {
                IntegrationContentState::Mismatch
            }
        }
        Err(error) => IntegrationContentState::ReadFailed(error.to_string()),
    }
}

fn build_integration_child_presence_only(
    relative_path: &str,
    path: &Path,
) -> IntegrationChildHealth {
    let content_state = if path_is_file(path) {
        IntegrationContentState::Match
    } else {
        IntegrationContentState::Missing
    };
    IntegrationChildHealth {
        relative_path: relative_path.to_string(),
        path: path.to_path_buf(),
        content_state,
    }
}

fn inspect_integration_asset_state(
    path: &Path,
    expected_sha256: &[u8; 32],
) -> IntegrationContentState {
    if !path_is_file(path) {
        return IntegrationContentState::Missing;
    }

    match fs::read(path) {
        Ok(bytes) => {
            let digest: [u8; 32] = Sha256::digest(&bytes).into();
            if &digest == expected_sha256 {
                IntegrationContentState::Match
            } else {
                IntegrationContentState::Mismatch
            }
        }
        Err(error) => IntegrationContentState::ReadFailed(error.to_string()),
    }
}

fn path_is_file(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

#[allow(dead_code)]
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
        Ok(bytes) => hook_managed_block_content_state(hook_name, &bytes, expected_hook.bytes),
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        collect_claude_integration_groups, collect_hook_file_health,
        collect_opencode_integration_groups, collect_pi_integration_groups,
        inspect_claude_integration_health, HookContentState, IntegrationContentState,
        IntegrationGroupHealth,
    };
    use crate::services::setup::OPTIONAL_WORKFLOWS;

    /// The collectors only read file state, so a non-existent root is enough to
    /// observe which children they expect.
    fn absent_repository_root() -> PathBuf {
        PathBuf::from("/nonexistent-sce-doctor-optional-workflow-fixture")
    }

    fn brownfield_slugs() -> (&'static str, &'static str) {
        let workflow = OPTIONAL_WORKFLOWS
            .iter()
            .find(|workflow| workflow.id == "brownfield")
            .expect("brownfield is an optional workflow in the embedded catalog");
        (workflow.command_slug, workflow.skill_slug)
    }

    fn child_paths(groups: &[IntegrationGroupHealth]) -> Vec<String> {
        groups
            .iter()
            .flat_map(|group| group.children.iter())
            .map(|child| child.relative_path.clone())
            .collect()
    }

    #[test]
    fn integration_children_exclude_unselected_optional_workflow() {
        let root = absent_repository_root();
        let (command_slug, skill_slug) = brownfield_slugs();

        let cases = [
            (
                child_paths(&collect_opencode_integration_groups(&root, &[])),
                format!("command/{command_slug}.md"),
            ),
            (
                child_paths(&collect_claude_integration_groups(&root, &[])),
                format!("commands/{command_slug}.md"),
            ),
            (
                child_paths(&collect_pi_integration_groups(&root, &[])),
                format!("prompts/{command_slug}.md"),
            ),
        ];

        let skill_prefix = format!("skills/{skill_slug}/");
        for (paths, command_path) in cases {
            assert!(
                !paths.contains(&command_path),
                "{command_path} still expected"
            );
            assert!(
                !paths.iter().any(|path| path.starts_with(&skill_prefix)),
                "{skill_prefix} assets still expected"
            );
            assert!(
                paths.iter().any(|path| path.ends_with("validate.md")),
                "core workflow assets were dropped"
            );
        }
    }

    #[test]
    fn integration_children_include_selected_optional_workflow() {
        let root = absent_repository_root();
        let (command_slug, skill_slug) = brownfield_slugs();
        let selection = vec![String::from("brownfield")];

        let cases = [
            (
                child_paths(&collect_opencode_integration_groups(&root, &selection)),
                format!("command/{command_slug}.md"),
            ),
            (
                child_paths(&collect_claude_integration_groups(&root, &selection)),
                format!("commands/{command_slug}.md"),
            ),
            (
                child_paths(&collect_pi_integration_groups(&root, &selection)),
                format!("prompts/{command_slug}.md"),
            ),
        ];

        let skill_prefix = format!("skills/{skill_slug}/");
        for (paths, command_path) in cases {
            assert!(paths.contains(&command_path), "{command_path} not expected");
            assert!(
                paths.iter().any(|path| path.starts_with(&skill_prefix)),
                "{skill_prefix} assets not expected"
            );
        }
    }

    #[test]
    fn missing_file_problems_follow_the_optional_workflow_selection() {
        let root = absent_repository_root();
        let (command_slug, _) = brownfield_slugs();
        let command_path = format!("commands/{command_slug}.md");

        let unselected_groups = collect_claude_integration_groups(&root, &[]);
        let mut unselected_problems = Vec::new();
        inspect_claude_integration_health(&unselected_groups, &mut unselected_problems);
        assert!(
            !unselected_problems
                .iter()
                .any(|problem| problem.summary.contains(command_slug)),
            "an unselected optional workflow produced a problem"
        );
        assert!(
            !unselected_problems.is_empty(),
            "core assets are absent, so missing-file problems are still expected"
        );

        let selection = vec![String::from("brownfield")];
        let selected_groups = collect_claude_integration_groups(&root, &selection);
        assert!(selected_groups.iter().any(|group| group
            .children
            .iter()
            .any(|child| child.relative_path == command_path
                && matches!(child.content_state, IntegrationContentState::Missing))));

        let mut selected_problems = Vec::new();
        inspect_claude_integration_health(&selected_groups, &mut selected_problems);
        assert!(
            selected_problems
                .iter()
                .any(|problem| problem.summary.contains(&command_path)),
            "a selected optional workflow's missing file was not reported"
        );
    }

    fn unique_temp_repository_root(label: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sce-doctor-merge-target-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp repository root");
        dir
    }

    fn embedded_claude_settings_bytes() -> &'static [u8] {
        crate::services::setup::iter_embedded_assets_for_setup_target_with_selection(
            crate::services::setup::SetupTarget::Claude,
            &[] as &[String],
        )
        .find(|asset| {
            asset.relative_path == crate::services::default_paths::claude_asset::SETTINGS_FILE
        })
        .expect("embedded Claude catalog carries settings.json")
        .bytes
    }

    fn embedded_opencode_config_bytes() -> &'static [u8] {
        crate::services::setup::iter_embedded_assets_for_setup_target_with_selection(
            crate::services::setup::SetupTarget::OpenCode,
            &[] as &[String],
        )
        .find(|asset| asset.relative_path == "opencode.json")
        .expect("embedded OpenCode catalog carries opencode.json")
        .bytes
    }

    #[test]
    fn claude_settings_reports_match_despite_extra_user_permissions() {
        let root = unique_temp_repository_root("claude-pass");
        let claude_dir = root.join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();

        let generated_bytes = embedded_claude_settings_bytes();
        let installed_bytes =
            crate::services::setup::config_merge::merge_or_create_claude_settings(
                None,
                generated_bytes,
                "settings.json",
            )
            .unwrap();
        let mut installed: serde_json::Value = serde_json::from_slice(&installed_bytes).unwrap();
        installed["permissions"] = serde_json::json!({"allow": ["Bash(git *)"]});
        std::fs::write(
            claude_dir.join("settings.json"),
            serde_json::to_vec_pretty(&installed).unwrap(),
        )
        .unwrap();

        let groups = collect_claude_integration_groups(&root, &[]);
        let settings_child = groups
            .iter()
            .flat_map(|group| &group.children)
            .find(|child| child.relative_path == "settings.json")
            .expect("settings.json child present");
        assert!(matches!(
            settings_child.content_state,
            IntegrationContentState::Match
        ));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claude_settings_reports_mismatch_when_sce_hook_entry_deleted_then_fix_repairs_it() {
        let root = unique_temp_repository_root("claude-fix");
        let claude_dir = root.join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();

        let generated_bytes = embedded_claude_settings_bytes();
        let installed_bytes =
            crate::services::setup::config_merge::merge_or_create_claude_settings(
                None,
                generated_bytes,
                "settings.json",
            )
            .unwrap();
        let mut drifted: serde_json::Value = serde_json::from_slice(&installed_bytes).unwrap();
        drifted["permissions"] = serde_json::json!({"allow": ["Bash(git *)"]});
        // Drop every hook event's entries to simulate a deleted SCE hook entry.
        for (_, entries) in drifted["hooks"].as_object_mut().unwrap() {
            *entries = serde_json::json!([]);
        }
        let settings_path = claude_dir.join("settings.json");
        std::fs::write(&settings_path, serde_json::to_vec_pretty(&drifted).unwrap()).unwrap();

        let groups = collect_claude_integration_groups(&root, &[]);
        let settings_child = groups
            .iter()
            .flat_map(|group| &group.children)
            .find(|child| child.relative_path == "settings.json")
            .expect("settings.json child present");
        assert!(matches!(
            settings_child.content_state,
            IntegrationContentState::Mismatch
        ));

        let fix_results = super::repair_merge_target_configs(&root);
        assert!(
            fix_results
                .iter()
                .any(|result| matches!(result.outcome, super::FixResult::Fixed)),
            "expected the drifted settings.json to be repaired"
        );

        let repaired: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&settings_path).unwrap()).unwrap();
        assert_eq!(repaired["permissions"]["allow"][0], "Bash(git *)");

        let groups_after_fix = collect_claude_integration_groups(&root, &[]);
        let settings_child_after_fix = groups_after_fix
            .iter()
            .flat_map(|group| &group.children)
            .find(|child| child.relative_path == "settings.json")
            .expect("settings.json child present");
        assert!(matches!(
            settings_child_after_fix.content_state,
            IntegrationContentState::Match
        ));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn opencode_config_reports_match_despite_extra_user_plugin_then_drift_and_fix() {
        let root = unique_temp_repository_root("opencode-fix");
        let opencode_dir = root.join(".opencode");
        std::fs::create_dir_all(&opencode_dir).unwrap();

        let generated_bytes = embedded_opencode_config_bytes();
        let installed_bytes =
            crate::services::setup::config_merge::merge_or_create_opencode_config(
                None,
                generated_bytes,
                "opencode.json",
            )
            .unwrap();
        let mut installed: serde_json::Value = serde_json::from_slice(&installed_bytes).unwrap();
        installed["model"] = serde_json::json!("anthropic/claude");
        installed["plugin"]
            .as_array_mut()
            .unwrap()
            .insert(0, serde_json::json!("./plugins/my-plugin.ts"));
        let manifest_path = opencode_dir.join("opencode.json");
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&installed).unwrap(),
        )
        .unwrap();

        let groups = collect_opencode_integration_groups(&root, &[]);
        let manifest_child = groups
            .iter()
            .flat_map(|group| &group.children)
            .find(|child| child.relative_path == "opencode.json")
            .expect("opencode.json child present");
        assert!(matches!(
            manifest_child.content_state,
            IntegrationContentState::Match
        ));

        // Drop the plugin array entirely to simulate a stale/removed SCE registration.
        installed["plugin"] = serde_json::json!(["./plugins/my-plugin.ts"]);
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&installed).unwrap(),
        )
        .unwrap();

        let drifted_groups = collect_opencode_integration_groups(&root, &[]);
        let drifted_child = drifted_groups
            .iter()
            .flat_map(|group| &group.children)
            .find(|child| child.relative_path == "opencode.json")
            .expect("opencode.json child present");
        assert!(matches!(
            drifted_child.content_state,
            IntegrationContentState::Mismatch
        ));

        let fix_results = super::repair_merge_target_configs(&root);
        assert!(
            fix_results
                .iter()
                .any(|result| matches!(result.outcome, super::FixResult::Fixed)),
            "expected the drifted opencode.json to be repaired"
        );

        let repaired: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        assert_eq!(repaired["model"], "anthropic/claude");
        let plugin = repaired["plugin"].as_array().unwrap();
        assert!(plugin.contains(&serde_json::json!("./plugins/my-plugin.ts")));
        assert!(plugin.contains(&serde_json::json!("./plugins/sce-bash-policy.ts")));
        assert!(plugin.contains(&serde_json::json!("./plugins/sce-agent-trace.ts")));

        std::fs::remove_dir_all(&root).ok();
    }

    fn canonical_pre_commit_bytes() -> &'static [u8] {
        crate::services::setup::iter_required_hook_assets()
            .find(|asset| asset.relative_path == "pre-commit")
            .expect("embedded catalog carries pre-commit")
            .bytes
    }

    #[cfg(unix)]
    fn mark_executable(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("mark hook executable");
    }

    #[test]
    fn hook_with_foreign_content_and_current_block_reports_current() {
        let dir = unique_temp_repository_root("hook-foreign-current");
        let foreign_prefix = b"#!/bin/sh\necho husky-style-guard\n".to_vec();
        let merge = crate::services::setup::hook_merge::merge_or_create_hook(
            Some(&foreign_prefix),
            canonical_pre_commit_bytes(),
            "pre-commit",
        )
        .expect("merge over foreign hook should succeed");

        let hook_path = dir.join("pre-commit");
        std::fs::write(&hook_path, &merge.bytes).expect("write foreign-plus-block hook");
        #[cfg(unix)]
        mark_executable(&hook_path);

        let health = collect_hook_file_health(&dir);
        let pre_commit = health
            .iter()
            .find(|hook| hook.name == "pre-commit")
            .expect("pre-commit health present");
        assert_eq!(pre_commit.content_state, HookContentState::Current);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hook_with_drifted_managed_block_reports_stale() {
        let dir = unique_temp_repository_root("hook-drifted-stale");
        let canonical_text =
            String::from_utf8(canonical_pre_commit_bytes().to_vec()).expect("hook is utf8");
        let drifted_text = canonical_text.replace(
            "sce hooks pre-commit \"$@\"",
            "sce hooks pre-commit \"$@\" # drifted",
        );
        assert_ne!(
            drifted_text, canonical_text,
            "drift fixture should actually differ from canonical"
        );

        let hook_path = dir.join("pre-commit");
        std::fs::write(&hook_path, drifted_text.as_bytes()).expect("write drifted hook");
        #[cfg(unix)]
        mark_executable(&hook_path);

        let health = collect_hook_file_health(&dir);
        let pre_commit = health
            .iter()
            .find(|hook| hook.name == "pre-commit")
            .expect("pre-commit health present");
        assert_eq!(pre_commit.content_state, HookContentState::Stale);

        std::fs::remove_dir_all(&dir).ok();
    }

    fn init_git_repo(label: &str) -> PathBuf {
        let repo = unique_temp_repository_root(label);
        let output = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .output()
            .expect("git init should spawn");
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        repo
    }

    #[test]
    fn fix_repairs_drifted_hook_content_while_preserving_foreign_content() {
        let repo = init_git_repo("hook-fix-repair");

        let initial_outcome = crate::services::setup::install_required_git_hooks(&repo)
            .expect("initial hook install should succeed");
        let pre_commit_path = initial_outcome
            .hook_results
            .iter()
            .find(|result| result.hook_name == "pre-commit")
            .expect("pre-commit hook installed")
            .hook_path
            .clone();
        let hooks_directory = pre_commit_path
            .parent()
            .expect("hook path has a parent directory")
            .to_path_buf();

        let foreign_prefix = b"#!/bin/sh\necho husky-style-guard\n".to_vec();
        let foreign_plus_block = crate::services::setup::hook_merge::merge_or_create_hook(
            Some(&foreign_prefix),
            canonical_pre_commit_bytes(),
            "pre-commit",
        )
        .expect("merge over foreign hook should succeed");
        let drifted_text = String::from_utf8(foreign_plus_block.bytes.clone())
            .expect("hook is utf8")
            .replace(
                "sce hooks pre-commit \"$@\"",
                "sce hooks pre-commit \"$@\" # drifted",
            );
        std::fs::write(&pre_commit_path, drifted_text.as_bytes())
            .expect("seed foreign-plus-drifted-block hook");
        #[cfg(unix)]
        mark_executable(&pre_commit_path);

        let health_before = collect_hook_file_health(&hooks_directory);
        let pre_commit_before = health_before
            .iter()
            .find(|hook| hook.name == "pre-commit")
            .expect("pre-commit health present");
        assert_eq!(pre_commit_before.content_state, HookContentState::Stale);

        crate::services::setup::install_required_git_hooks(&repo)
            .expect("'--fix' repair reuses the canonical setup hook installation");

        let repaired_bytes = std::fs::read(&pre_commit_path).expect("read repaired hook");
        assert!(
            repaired_bytes.starts_with(&foreign_prefix),
            "foreign content should survive the repair"
        );

        let health_after = collect_hook_file_health(&hooks_directory);
        let pre_commit_after = health_after
            .iter()
            .find(|hook| hook.name == "pre-commit")
            .expect("pre-commit health present");
        assert_eq!(pre_commit_after.content_state, HookContentState::Current);

        std::fs::remove_dir_all(&repo).ok();
    }
}
