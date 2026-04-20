use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::services::default_paths::{opencode_asset, InstallTargetPaths, RepoPaths};
use crate::services::setup::{
    iter_embedded_assets_for_setup_target, iter_required_hook_assets, EmbeddedAsset, SetupTarget,
};

use super::types::{
    DoctorProblem, FileLocationHealth, GlobalStateHealth, HookContentState, HookDoctorReport,
    HookFileHealth, HookPathSource, IntegrationChildHealth, IntegrationContentState,
    IntegrationGroupHealth, ProblemCategory, ProblemFixability, ProblemKind, ProblemSeverity,
    Readiness, OPENCODE_AGENTS_LABEL, OPENCODE_COMMANDS_LABEL, OPENCODE_PLUGINS_LABEL,
    OPENCODE_SKILLS_LABEL,
};
use super::{is_executable, DoctorDependencies, DoctorMode, REQUIRED_HOOKS};

pub(super) fn build_report_with_dependencies(
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

    let hooks = inspect_repository_hooks(
        repository_root,
        git_available,
        bare_repository,
        detected_repository_root.as_deref(),
        hooks_directory.as_deref(),
        &mut problems,
    );

    let integration_groups = inspect_repository_integrations(
        git_available,
        bare_repository,
        detected_repository_root.as_deref(),
        &mut problems,
    );

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
        hooks,
        integration_groups,
        problems,
    }
}

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

    let integration_groups = collect_opencode_integration_groups(resolved_root);
    inspect_opencode_integration_health(resolved_root, &integration_groups, problems);
    integration_groups
}

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
                group.label, missing_paths
            ),
            remediation: format!(
                "Reinstall repo-root OpenCode assets to restore the missing {} file(s), then rerun 'sce doctor'.",
                group.label.to_ascii_lowercase()
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
                group.label, mismatched_paths
            ),
            remediation: format!(
                "Reinstall repo-root OpenCode assets to restore the canonical {} content, then rerun 'sce doctor'.",
                group.label.to_ascii_lowercase()
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

fn collect_opencode_integration_groups(repository_root: &Path) -> Vec<IntegrationGroupHealth> {
    let repo_paths = RepoPaths::new(repository_root);
    let opencode_root = repo_paths.opencode_dir();
    let manifest_path = repo_paths.opencode_manifest_file();
    let embedded_assets =
        iter_embedded_assets_for_setup_target(SetupTarget::OpenCode).collect::<Vec<_>>();
    let mut plugin_children = Vec::new();
    let mut agent_children = Vec::new();
    let mut command_children = Vec::new();
    let mut skill_children = Vec::new();

    let manifest_child = embedded_assets
        .iter()
        .find(|asset| asset.relative_path == "opencode.json")
        .map_or_else(
            || build_integration_child_presence_only("opencode.json", &manifest_path),
            |asset| build_integration_child_from_asset(&opencode_root, asset),
        );
    plugin_children.push(manifest_child);

    for asset in embedded_assets {
        if asset.relative_path == "opencode.json" {
            continue;
        }
        let child = build_integration_child_from_asset(&opencode_root, asset);

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
        IntegrationGroupHealth {
            label: OPENCODE_PLUGINS_LABEL,
            children: plugin_children,
        },
        IntegrationGroupHealth {
            label: OPENCODE_AGENTS_LABEL,
            children: agent_children,
        },
        IntegrationGroupHealth {
            label: OPENCODE_COMMANDS_LABEL,
            children: command_children,
        },
        IntegrationGroupHealth {
            label: OPENCODE_SKILLS_LABEL,
            children: skill_children,
        },
    ]
}

fn sort_integration_children(children: &mut [IntegrationChildHealth]) {
    children.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
}

fn build_integration_child_from_asset(
    opencode_root: &Path,
    asset: &EmbeddedAsset,
) -> IntegrationChildHealth {
    let path = opencode_root.join(asset.relative_path);
    let content_state = inspect_opencode_asset_state(&path, &asset.sha256);
    IntegrationChildHealth {
        relative_path: asset.relative_path.to_string(),
        path,
        content_state,
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

fn inspect_opencode_asset_state(
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
