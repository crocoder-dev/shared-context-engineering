use crate::services::capabilities::GitOps;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::services::agent_trace_db::lifecycle::diagnose_agent_trace_db_health;
use crate::services::codex_hook_config;
use crate::services::codex_hook_policy::CodexHookPolicyReadiness;
use crate::services::codex_hook_trust;
use crate::services::config::schema::parse_file_config;
use crate::services::config::{self, ConfigPathSource, IntegrationTargetId};
use crate::services::default_paths::{
    agent_trace_db_path_for_repository, claude_asset, codex_asset, opencode_asset, pi_asset,
    repo_dir, InstallTargetPaths, RepoPaths,
};
use crate::services::hooks::mutation_scope_health::{
    MutationScopeAdapterHealth, MutationScopeHealthStatus, Repairability,
};
use crate::services::hooks::{
    claude_mutation_scope, codex_mutation_scope, mutation_scope, opencode_mutation_scope,
    pi_mutation_scope,
};
use crate::services::mutation_trace::runtime::resolve_git_dir;
use crate::services::repository_identity::resolve::{
    resolve_repository_identity, RepositoryIdentitySource,
};
use crate::services::setup::{
    config_merge, hook_merge, iter_embedded_assets_for_setup_target_with_selection,
    iter_required_hook_assets, persisted_optional_workflows, repair_merge_target_asset,
    EmbeddedAsset, SetupTarget,
};

use super::render::integration_target_label;
use super::types::{
    compute_readiness, mutation_scope_health_status, AgentTraceDbHealth, DoctorFixResultRecord,
    DoctorProblem, FileLocationHealth, FixResult, GlobalStateHealth, HookContentState,
    HookDoctorReport, HookFileHealth, HookPathSource, IntegrationArea, IntegrationChildHealth,
    IntegrationContentState, IntegrationGroupHealth, IntegrationGroupKey, IntegrationTarget,
    MutationScopeHealthRow, PostCommitAutoSyncHealth, PostCommitAutoSyncState, ProblemCategory,
    ProblemFixability, ProblemKind, ProblemSeverity, Readiness,
};
use super::{is_executable, DoctorDependencies, DoctorMode, REQUIRED_HOOKS};

pub(super) async fn build_report_with_lifecycle_problems(
    mode: DoctorMode,
    repository_root: &Path,
    dependencies: &DoctorDependencies<
        '_,
        impl crate::services::capabilities::GitOps,
        impl Fn() -> anyhow::Result<PathBuf>,
        impl Fn() -> anyhow::Result<PathBuf>,
        impl Fn() -> crate::services::codex_hook_policy::CodexHookPolicyReadiness,
    >,
    lifecycle_problems: Vec<DoctorProblem>,
    codex_policy_readiness: &CodexHookPolicyReadiness,
) -> HookDoctorReport {
    let mut report = build_report_without_service_owned_problem_checks(
        mode,
        repository_root,
        dependencies,
        lifecycle_problems,
        codex_policy_readiness,
    )
    .await;
    report.agent_trace_db =
        collect_agent_trace_db_health(repository_root, &mut report.problems).await;
    report.readiness = compute_readiness(&report.problems);
    report
}

async fn build_report_without_service_owned_problem_checks(
    mode: DoctorMode,
    repository_root: &Path,
    dependencies: &DoctorDependencies<
        '_,
        impl crate::services::capabilities::GitOps,
        impl Fn() -> anyhow::Result<PathBuf>,
        impl Fn() -> anyhow::Result<PathBuf>,
        impl Fn() -> crate::services::codex_hook_policy::CodexHookPolicyReadiness,
    >,
    mut problems: Vec<DoctorProblem>,
    codex_policy_readiness: &CodexHookPolicyReadiness,
) -> HookDoctorReport {
    let global_state = collect_global_state_locations(repository_root, dependencies);
    let agent_trace_db = collect_agent_trace_db_health(repository_root, &mut problems).await;
    let git_available = dependencies.git.is_available();

    let detected_repository_root = if git_available {
        doctor_git_output(
            dependencies.git,
            repository_root,
            &["rev-parse", "--show-toplevel"],
        )
        .map(PathBuf::from)
    } else {
        None
    };

    let bare_repository = if git_available {
        doctor_git_output(
            dependencies.git,
            repository_root,
            &["rev-parse", "--is-bare-repository"],
        )
        .is_some_and(|value| value == "true")
    } else {
        false
    };

    let hook_path_source =
        detect_hook_path_source(dependencies.git, git_available, repository_root);

    let hooks_directory = detected_repository_root.as_ref().and_then(|resolved_root| {
        doctor_git_output(
            dependencies.git,
            resolved_root,
            &["rev-parse", "--git-path", "hooks"],
        )
        .map(|value| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                resolved_root.join(path)
            }
        })
    });

    let hooks = if git_available && !bare_repository && detected_repository_root.is_some() {
        hooks_directory
            .as_deref()
            .map(collect_hook_file_health)
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let post_commit_auto_sync = collect_post_commit_auto_sync_health(
        repository_root,
        detected_repository_root.is_some(),
        &hooks,
    );

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
        codex_policy_readiness,
    );
    let mutation_scope_health = inspect_mutation_scope_health(
        git_available,
        bare_repository,
        detected_repository_root.as_deref(),
        &mut problems,
    )
    .await;

    HookDoctorReport {
        mode,
        readiness: Readiness::Ready,
        state_root: global_state.state_root,
        agent_trace_db,
        repository_root: detected_repository_root,
        hook_path_source,
        hooks_directory,
        post_commit_auto_sync,
        config_locations: global_state.config_locations,
        hooks,
        integration_groups,
        integration_targets_absent,
        mutation_scope_health,
        problems,
    }
}

fn detect_hook_path_source(
    git: &impl GitOps,
    git_available: bool,
    repository_root: &Path,
) -> HookPathSource {
    let local_hooks_path = if git_available {
        doctor_git_output(
            git,
            repository_root,
            &["config", "--local", "--get", "core.hooksPath"],
        )
    } else {
        None
    };
    let global_hooks_path = if git_available {
        doctor_git_output(
            git,
            repository_root,
            &["config", "--global", "--get", "core.hooksPath"],
        )
    } else {
        None
    };

    if local_hooks_path.is_some() {
        HookPathSource::LocalConfig
    } else if global_hooks_path.is_some() {
        HookPathSource::GlobalConfig
    } else {
        HookPathSource::Default
    }
}

async fn inspect_mutation_scope_health(
    git_available: bool,
    bare_repository: bool,
    detected_repository_root: Option<&Path>,
    problems: &mut Vec<DoctorProblem>,
) -> Vec<MutationScopeHealthRow> {
    if !git_available || bare_repository {
        return Vec::new();
    }
    let Some(resolved_root) = detected_repository_root else {
        return Vec::new();
    };
    let targets = resolve_doctor_integration_targets(resolved_root);
    if targets.is_empty() {
        return Vec::new();
    }
    let Ok(git_dir) = resolve_git_dir(resolved_root).await else {
        return Vec::new();
    };

    let mut rows = Vec::with_capacity(targets.len());
    for target_id in targets {
        let (target, health) = match target_id {
            IntegrationTargetId::Claude => (
                IntegrationTarget::ClaudeCode,
                claude_mutation_scope::health::classify_health(&git_dir).await,
            ),
            IntegrationTargetId::Codex => (
                IntegrationTarget::Codex,
                codex_mutation_scope::health::classify_health(&git_dir).await,
            ),
            IntegrationTargetId::Opencode => (
                IntegrationTarget::OpenCode,
                opencode_mutation_scope::health::classify_health(&git_dir).await,
            ),
            IntegrationTargetId::Pi => (
                IntegrationTarget::Pi,
                pi_mutation_scope::health::classify_health(&git_dir).await,
            ),
        };
        let remediation =
            push_mutation_scope_health_problem(target, &health, &git_dir, problems).await;
        rows.push(MutationScopeHealthRow {
            target,
            status: health.status,
            reason: health.reason,
            detail: health.detail,
            remediation,
        });
    }
    rows
}

pub(super) async fn repair_blocked_mutation_scope_targets_with_seam(
    initial_report: &HookDoctorReport,
    seam: &impl std::ops::AsyncFn(
        &std::path::Path,
        &str,
        Option<&crate::services::observability::traits::NoopLogger>,
    ) -> anyhow::Result<String>,
) -> Vec<IntegrationTarget> {
    let Some(repository_root) = initial_report.repository_root.as_deref() else {
        return Vec::new();
    };
    let Ok(git_dir) = resolve_git_dir(repository_root).await else {
        return Vec::new();
    };

    let mut repaired = Vec::new();
    for row in &initial_report.mutation_scope_health {
        if row.status == MutationScopeHealthStatus::Blocked {
            if let Some(target) =
                repair_blocked_mutation_scope_target(row.target, &git_dir, repository_root, seam)
                    .await
            {
                repaired.push(target);
            }
        }
    }
    repaired
}

pub(super) async fn mutation_scope_repair_seam<
    L: crate::services::observability::traits::Logger,
>(
    repository_root: &Path,
    payload: &str,
    logger: Option<&L>,
) -> anyhow::Result<String> {
    mutation_scope::run_mutation_scope_from_payload(repository_root, payload, logger).await
}

async fn repair_blocked_mutation_scope_target(
    target: IntegrationTarget,
    git_dir: &Path,
    repository_root: &Path,
    seam: &impl std::ops::AsyncFn(
        &std::path::Path,
        &str,
        Option<&crate::services::observability::traits::NoopLogger>,
    ) -> anyhow::Result<String>,
) -> Option<IntegrationTarget> {
    match target {
        IntegrationTarget::ClaudeCode => {
            if claude_repairability(git_dir).await != Repairability::AutoFixable {
                return None;
            }
            let _ =
                claude_mutation_scope::repair_blocked(git_dir, repository_root, None, seam).await;
            Some(target)
        }
        IntegrationTarget::OpenCode => {
            if opencode_repairability(git_dir).await != Repairability::AutoFixable {
                return None;
            }
            let _ =
                opencode_mutation_scope::repair_blocked(git_dir, repository_root, None, seam).await;
            Some(target)
        }
        IntegrationTarget::Pi | IntegrationTarget::Codex => None,
    }
}

async fn claude_repairability(git_dir: &Path) -> Repairability {
    match claude_mutation_scope::assess_repairability(git_dir).await {
        claude_mutation_scope::Repairability::AutoFixable => Repairability::AutoFixable,
        claude_mutation_scope::Repairability::ManualOnly => Repairability::ManualOnly,
    }
}

async fn opencode_repairability(git_dir: &Path) -> Repairability {
    match opencode_mutation_scope::assess_repairability(git_dir).await {
        opencode_mutation_scope::Repairability::AutoFixable => Repairability::AutoFixable,
        opencode_mutation_scope::Repairability::ManualOnly => Repairability::ManualOnly,
    }
}

async fn mutation_scope_repairability(target: IntegrationTarget, git_dir: &Path) -> Repairability {
    match target {
        IntegrationTarget::ClaudeCode => claude_repairability(git_dir).await,
        IntegrationTarget::OpenCode => opencode_repairability(git_dir).await,
        IntegrationTarget::Pi | IntegrationTarget::Codex => Repairability::ManualOnly,
    }
}

pub(super) fn finalize_mutation_scope_repair_results(
    attempted_targets: &[IntegrationTarget],
    final_mutation_scope_health: &[MutationScopeHealthRow],
) -> Vec<DoctorFixResultRecord> {
    attempted_targets
        .iter()
        .filter_map(|target| {
            let row = final_mutation_scope_health
                .iter()
                .find(|row| row.target == *target)?;
            Some(mutation_scope_repair_result_from_final_row(*target, row))
        })
        .collect()
}

fn mutation_scope_repair_result_from_final_row(
    target: IntegrationTarget,
    row: &MutationScopeHealthRow,
) -> DoctorFixResultRecord {
    match row.status {
        MutationScopeHealthStatus::Healthy | MutationScopeHealthStatus::Recovering => {
            DoctorFixResultRecord {
                category: ProblemCategory::MutationScopeHealth,
                outcome: FixResult::Fixed,
                detail: format!(
                    "Recovered {} Agent tracing (now {}: {}).",
                    integration_target_label(target),
                    mutation_scope_health_status(row.status),
                    row.reason
                ),
            }
        }
        MutationScopeHealthStatus::Blocked | MutationScopeHealthStatus::Invalid => {
            DoctorFixResultRecord {
                category: ProblemCategory::MutationScopeHealth,
                outcome: FixResult::Manual,
                detail: row.remediation.clone().unwrap_or_else(|| {
                    format!(
                        "{} Agent tracing remains {} after an attempted repair.",
                        integration_target_label(target),
                        mutation_scope_health_status(row.status)
                    )
                }),
            }
        }
    }
}

fn mutation_scope_state_path(target: IntegrationTarget, git_dir: &Path) -> PathBuf {
    match target {
        IntegrationTarget::ClaudeCode => claude_mutation_scope::state::state_path(git_dir),
        IntegrationTarget::Codex => codex_mutation_scope::state::state_path(git_dir),
        IntegrationTarget::OpenCode => opencode_mutation_scope::state::state_path(git_dir),
        IntegrationTarget::Pi => pi_mutation_scope::state::state_path(git_dir),
    }
}

async fn push_mutation_scope_health_problem(
    target: IntegrationTarget,
    health: &MutationScopeAdapterHealth,
    git_dir: &Path,
    problems: &mut Vec<DoctorProblem>,
) -> Option<String> {
    let (kind, severity, fixability, next_action, remediation) = match health.status {
        MutationScopeHealthStatus::Healthy => return None,
        MutationScopeHealthStatus::Recovering => (
            ProblemKind::MutationScopeHealthRecovering,
            ProblemSeverity::Warning,
            ProblemFixability::NoActionRequired,
            "no_action_required",
            String::from(
                "This persisted state has a proven ordinary lifecycle/admission path that can \
                 advance recovery without manual intervention. Not every subsequent tool call \
                 is necessarily recovery-capable for every adapter. No manual action is \
                 required now. After subsequent adapter activity, rerun 'sce doctor' if the \
                 state remains recovering unexpectedly.",
            ),
        ),
        MutationScopeHealthStatus::Blocked => {
            match mutation_scope_repairability(target, git_dir).await {
                Repairability::AutoFixable => (
                    ProblemKind::MutationScopeHealthBlocked,
                    ProblemSeverity::Error,
                    ProblemFixability::AutoFixable,
                    "doctor_fix",
                    format!(
                        "Run 'sce doctor --fix' to recover this state: the owning process for \
                         the blocking attempt(s) has been positively proven dead, so automatic \
                         recovery is safe. The persisted state is at '{}'.",
                        mutation_scope_state_path(target, git_dir).display()
                    ),
                ),
                Repairability::ManualOnly => (
                    ProblemKind::MutationScopeHealthBlocked,
                    ProblemSeverity::Error,
                    ProblemFixability::ManualOnly,
                    "manual_steps",
                    format!(
                        "Agent tracing remains blocked. Inspect '{}'. 'sce doctor --fix' will \
                         not modify persisted mutation-scope recovery state automatically: \
                         clearing it could silently discard unresolved mutation-scope \
                         lifecycle or recovery evidence. No safe generic recovery command \
                         exists yet for this case; preserve the persisted state while \
                         reviewing this adapter's recovery model directly before taking \
                         manual action.",
                        mutation_scope_state_path(target, git_dir).display()
                    ),
                ),
            }
        }
        MutationScopeHealthStatus::Invalid => (
            ProblemKind::MutationScopeHealthInvalid,
            ProblemSeverity::Error,
            ProblemFixability::ManualOnly,
            "manual_steps",
            format!(
                "Agent tracing state could not be safely interpreted. Inspect '{}'. No safe \
                 generic recovery command exists yet; preserve the persisted state for \
                 diagnosis and review the adapter's recovery model directly.",
                mutation_scope_state_path(target, git_dir).display()
            ),
        ),
    };

    let detail_suffix = health
        .detail
        .as_deref()
        .map_or_else(String::new, |detail| format!(" ({detail})"));
    let summary = format!(
        "{} agent tracing is {}: {}{detail_suffix}",
        integration_target_label(target),
        mutation_scope_health_status(health.status),
        health.reason
    );

    problems.push(DoctorProblem {
        kind,
        category: ProblemCategory::MutationScopeHealth,
        severity,
        fixability,
        summary,
        remediation: remediation.clone(),
        next_action,
        scope: None,
        mutation_scope_target: Some(target),
    });

    Some(remediation)
}

fn collect_post_commit_auto_sync_health(
    repository_root: &Path,
    repository_is_available: bool,
    hooks: &[HookFileHealth],
) -> PostCommitAutoSyncHealth {
    let resolved = config::resolve_agent_trace_auto_sync_runtime_config(repository_root).ok();
    let (enabled, source, config_source) =
        resolved.map_or((true, "unresolved", None), |resolved| {
            (
                resolved.value,
                resolved.source.as_str(),
                resolved
                    .source
                    .config_source()
                    .map(ConfigPathSource::as_str),
            )
        });

    let state = post_commit_auto_sync_state(enabled, repository_is_available, hooks);

    PostCommitAutoSyncHealth {
        state,
        enabled,
        source,
        config_source,
    }
}

fn post_commit_auto_sync_state(
    enabled: bool,
    repository_is_available: bool,
    hooks: &[HookFileHealth],
) -> PostCommitAutoSyncState {
    if !repository_is_available {
        PostCommitAutoSyncState::NotApplicable
    } else if !enabled {
        PostCommitAutoSyncState::Disabled
    } else if hooks.iter().any(|hook| {
        hook.name == "post-commit"
            && hook.exists
            && hook.executable
            && hook.content_state == HookContentState::Current
    }) {
        PostCommitAutoSyncState::Ready
    } else {
        PostCommitAutoSyncState::NotReady
    }
}

fn collect_global_state_locations(
    repository_root: &Path,
    dependencies: &DoctorDependencies<
        '_,
        impl crate::services::capabilities::GitOps,
        impl Fn() -> anyhow::Result<PathBuf>,
        impl Fn() -> anyhow::Result<PathBuf>,
        impl Fn() -> crate::services::codex_hook_policy::CodexHookPolicyReadiness,
    >,
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

async fn collect_agent_trace_db_health(
    repository_root: &Path,
    problems: &mut Vec<DoctorProblem>,
) -> Option<AgentTraceDbHealth> {
    let agent_trace_problems = diagnose_agent_trace_db_health(Some(repository_root)).await;
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
                scope: None,
                mutation_scope_target: None,
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
    if repo_paths.codex_dir().exists() {
        detected.push(IntegrationTargetId::Codex);
    }
    detected
}

fn inspect_repository_integrations(
    git_available: bool,
    bare_repository: bool,
    detected_repository_root: Option<&Path>,
    problems: &mut Vec<DoctorProblem>,
    codex_policy_readiness: &CodexHookPolicyReadiness,
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
                "No integrations are installed. Run 'sce setup' to install OpenCode, Claude, Pi, and/or Codex integration assets.",
            ),
            remediation: String::from(
                "Run 'sce setup --opencode', 'sce setup --claude', 'sce setup --pi', 'sce setup --codex', or 'sce setup --all' to install integration assets.",
            ),
            next_action: "manual_steps",
            scope: None,
            mutation_scope_target: None,
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
            IntegrationTargetId::Codex => {
                let codex_groups = collect_codex_integration_groups(
                    resolved_root,
                    &selected_optional_workflows,
                    &codex_hook_trust::default_trust_context(),
                    codex_policy_readiness,
                );
                inspect_codex_integration_health(&codex_groups, problems);
                integration_groups.extend(codex_groups);
            }
        }
    }

    integration_groups
}

/// `codex_policy_readiness` is accepted only to build `collect_codex_integration_groups`'
/// per-registration content state; it never influences which files get
/// repaired here (see `repair_codex_hooks_json_if_structurally_unhealthy`,
/// which looks only at structural state). `sce doctor --fix` never writes
/// Codex policy or trust state — both are Codex-owned and read-only from
/// SCE's perspective.
///
/// Repairs each merge-target asset (`.claude/settings.json`,
/// `.opencode/opencode.json`) whose SCE-owned fragment is currently missing or
/// stale, by reinstalling just that asset through the same merge-install path
/// `sce setup` uses. Assets whose fragment is already current are left
/// untouched, and a fully missing integration is left to the existing
/// "reinstall assets" guidance rather than being created here.
pub(super) fn repair_merge_target_configs(
    repository_root: &Path,
    codex_policy_readiness: &CodexHookPolicyReadiness,
) -> Vec<DoctorFixResultRecord> {
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

    if targets.contains(&IntegrationTargetId::Codex) {
        let codex_groups = collect_codex_integration_groups(
            repository_root,
            &selected_optional_workflows,
            &codex_hook_trust::default_trust_context(),
            codex_policy_readiness,
        );
        if let Some(result) =
            repair_codex_hooks_json_if_structurally_unhealthy(repository_root, &codex_groups)
        {
            results.push(result);
        }
    }

    results
}

/// Repairs `.codex/hooks.json` when any required registration is
/// structurally unhealthy (missing, stale, or the whole document is
/// malformed). Never runs merely because a registration is not yet trusted
/// by Codex: SCE cannot fix that, and reinstalling changes nothing about
/// trust. Malformed documents are still attempted so the existing
/// merge-refuses-to-overwrite-invalid-JSON behavior surfaces as a normal
/// `Failed` fix result rather than being silently skipped.
fn repair_codex_hooks_json_if_structurally_unhealthy(
    repository_root: &Path,
    groups: &[IntegrationGroupHealth],
) -> Option<DoctorFixResultRecord> {
    let is_structurally_unhealthy = groups
        .iter()
        .flat_map(|group| &group.children)
        .filter(|child| {
            child
                .relative_path
                .starts_with(&format!("{CODEX_HOOKS_JSON_RELATIVE_PATH}#"))
        })
        .any(|child| {
            matches!(
                child.content_state,
                IntegrationContentState::Missing
                    | IntegrationContentState::Stale
                    | IntegrationContentState::Malformed(_)
            )
        });
    if !is_structurally_unhealthy {
        return None;
    }

    Some(
        match repair_merge_target_asset(
            repository_root,
            SetupTarget::Codex,
            CODEX_HOOKS_JSON_RELATIVE_PATH,
        ) {
            Ok(()) => DoctorFixResultRecord {
                category: ProblemCategory::RepoAssets,
                outcome: FixResult::Fixed,
                detail: format!(
                    "Merged canonical SCE hook registrations into '{CODEX_HOOKS_JSON_RELATIVE_PATH}'."
                ),
            },
            Err(error) => DoctorFixResultRecord {
                category: ProblemCategory::RepoAssets,
                outcome: FixResult::Failed,
                detail: format!(
                    "Failed to merge canonical SCE hook registrations into \
                     '{CODEX_HOOKS_JSON_RELATIVE_PATH}': {error}"
                ),
            },
        },
    )
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

fn inspect_opencode_integration_health(
    repository_root: &Path,
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    push_opencode_integration_missing_problems(integration_groups, problems);
    push_opencode_integration_mismatch_problems(integration_groups, problems);
    push_opencode_integration_read_fail_problems(integration_groups, problems);
    inspect_opencode_plugin_registry_health(repository_root, problems);
    inspect_opencode_plugin_ordering_health(repository_root, problems);

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

fn inspect_codex_integration_health(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    push_codex_integration_missing_problems(integration_groups, problems);
    push_codex_integration_mismatch_problems(integration_groups, problems);
    push_codex_integration_read_fail_problems(integration_groups, problems);
    push_codex_hook_malformed_problems(integration_groups, problems);
    push_codex_hook_policy_blocked_problems(integration_groups, problems);
    push_codex_hook_policy_unknown_problems(integration_groups, problems);
    push_codex_hook_trust_problems(integration_groups, problems);
}

fn push_codex_hook_malformed_problems(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    for group in integration_groups {
        let Some(error) = group
            .children
            .iter()
            .find_map(|child| match &child.content_state {
                IntegrationContentState::Malformed(error) => Some(error.clone()),
                _ => None,
            })
        else {
            continue;
        };

        problems.push(DoctorProblem {
            kind: ProblemKind::CodexHookRegistrationMalformed,
            category: ProblemCategory::RepoAssets,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!(
                "'.codex/hooks.json' cannot be structurally validated, so its required \
                 registrations cannot be verified: {error}"
            ),
            remediation: "Fix or remove the invalid '.codex/hooks.json' by hand, then rerun \
                           'sce setup --codex' or 'sce doctor' to reinstall it; SCE will not \
                           overwrite content it cannot safely merge."
                .to_string(),
            next_action: "manual_steps",
            scope: Some(group.key),
            mutation_scope_target: None,
        });
    }
}

/// Remediation for `PolicyBlocked` is deliberately administrative: SCE cannot
/// change Codex's enterprise/managed policy, so it must never suggest
/// re-trusting or reinstalling the hook (that would not fix anything).
const CODEX_HOOK_POLICY_BLOCKED_REMEDIATION: &str =
    "Ask the Codex administrator to allow project hooks or provide the SCE hook through an \
     allowed managed-hook mechanism. 'sce doctor --fix' cannot repair this: it is a Codex \
     enterprise/managed policy setting, not an SCE-owned file.";

fn push_codex_hook_policy_blocked_problems(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    for group in integration_groups {
        let blocked_children = group
            .children
            .iter()
            .filter(|child| {
                matches!(
                    child.content_state,
                    IntegrationContentState::PolicyBlocked(_)
                )
            })
            .map(|child| child.relative_path.as_str())
            .collect::<Vec<_>>();
        if blocked_children.is_empty() {
            continue;
        }

        let details = blocked_children.join(", ");
        problems.push(DoctorProblem {
            kind: ProblemKind::CodexHookRegistrationPolicyBlocked,
            category: ProblemCategory::RepoAssets,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!(
                "Codex project hooks are disabled by the effective allow_managed_hooks_only \
                 policy. SCE registrations in '.codex/hooks.json' will not be loaded by Codex: \
                 {details}."
            ),
            remediation: CODEX_HOOK_POLICY_BLOCKED_REMEDIATION.to_string(),
            next_action: "manual_steps",
            scope: Some(group.key),
            mutation_scope_target: None,
        });
    }
}

fn push_codex_hook_policy_unknown_problems(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    for group in integration_groups {
        let unknown_children = group
            .children
            .iter()
            .filter_map(|child| match &child.content_state {
                IntegrationContentState::PolicyUnknown(reason) => {
                    Some((child.relative_path.as_str(), reason.as_str()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let Some((_, reason)) = unknown_children.first().copied() else {
            continue;
        };

        let details = unknown_children
            .iter()
            .map(|(relative_path, _)| format!("'{relative_path}'"))
            .collect::<Vec<_>>()
            .join(", ");
        problems.push(DoctorProblem {
            kind: ProblemKind::CodexHookRegistrationPolicyUnknown,
            category: ProblemCategory::RepoAssets,
            severity: ProblemSeverity::Warning,
            fixability: ProblemFixability::ManualOnly,
            summary: format!(
                "Unable to determine whether Codex policy permits project hooks: {reason}. \
                 These current SCE registrations cannot be confirmed healthy: {details}."
            ),
            remediation: "Ensure the 'codex' CLI is installed and reachable on PATH, then rerun \
                           'sce doctor' so it can re-probe Codex's effective hook-discovery \
                           policy; 'sce doctor --fix' cannot repair this."
                .to_string(),
            next_action: "manual_steps",
            scope: Some(group.key),
            mutation_scope_target: None,
        });
    }
}

fn push_codex_hook_trust_problems(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    for group in integration_groups {
        let not_trusted_children = group
            .children
            .iter()
            .filter_map(|child| match &child.content_state {
                IntegrationContentState::NotTrusted(reason) => {
                    Some((child.relative_path.as_str(), reason.as_str()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if not_trusted_children.is_empty() {
            continue;
        }

        let details = not_trusted_children
            .iter()
            .map(|(relative_path, reason)| format!("'{relative_path}' ({reason})"))
            .collect::<Vec<_>>()
            .join(", ");
        problems.push(DoctorProblem {
            kind: ProblemKind::CodexHookRegistrationNotTrusted,
            category: ProblemCategory::RepoAssets,
            severity: ProblemSeverity::Warning,
            fixability: ProblemFixability::ManualOnly,
            summary: format!(
                "Codex has not yet marked these current SCE hook registrations as trusted and \
                 will not execute them: {details}."
            ),
            remediation: CODEX_HOOK_TRUST_GUIDANCE.to_string(),
            next_action: "manual_steps",
            scope: Some(group.key),
            mutation_scope_target: None,
        });
    }
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
            scope: Some(group.key),
            mutation_scope_target: None,
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
            scope: Some(group.key),
            mutation_scope_target: None,
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
                scope: Some(group.key),
                mutation_scope_target: None,
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
            scope: Some(group.key),
            mutation_scope_target: None,
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
            scope: Some(group.key),
            mutation_scope_target: None,
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
                scope: Some(group.key),
                mutation_scope_target: None,
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
            scope: Some(group.key),
            mutation_scope_target: None,
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
            scope: Some(group.key),
            mutation_scope_target: None,
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
                scope: Some(group.key),
                mutation_scope_target: None,
            });
        }
    }
}

/// Codex requires the project's `.codex/hooks.json` to be reviewed and
/// trusted inside the Codex CLI before it will execute; doctor can only
/// diagnose the file on disk and reinstall it, never grant that trust.
const CODEX_HOOK_TRUST_GUIDANCE: &str = "Codex also requires reviewing and trusting this project's hooks inside the Codex CLI before they take effect; 'sce doctor' cannot grant that trust on your behalf.";

fn push_codex_integration_missing_problems(
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
            .map(|child| format!("'{}'", child.relative_path))
            .collect::<Vec<_>>()
            .join(", ");
        let mut remediation = format!(
            "Reinstall repo-root Codex assets to restore the missing {} file(s), then rerun 'sce doctor'.",
            group.display_label().to_ascii_lowercase()
        );
        if group.key.area == IntegrationArea::Hooks {
            remediation.push(' ');
            remediation.push_str(CODEX_HOOK_TRUST_GUIDANCE);
        }
        problems.push(DoctorProblem {
            kind: ProblemKind::CodexIntegrationFilesMissing,
            category: ProblemCategory::RepoAssets,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!(
                "{} required file(s) are missing: {}.",
                group.display_label(),
                missing_paths
            ),
            remediation,
            next_action: "manual_steps",
            scope: Some(group.key),
            mutation_scope_target: None,
        });
    }
}

fn push_codex_integration_mismatch_problems(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    for group in integration_groups {
        let mismatched_children = group
            .children
            .iter()
            .filter(|child| {
                matches!(
                    &child.content_state,
                    IntegrationContentState::Mismatch | IntegrationContentState::Stale
                )
            })
            .collect::<Vec<_>>();
        if mismatched_children.is_empty() {
            continue;
        }

        let mismatched_paths = mismatched_children
            .iter()
            .map(|child| format!("'{}'", child.relative_path))
            .collect::<Vec<_>>()
            .join(", ");
        let mut remediation = format!(
            "Reinstall repo-root Codex assets to restore the canonical {} content, then rerun 'sce doctor'.",
            group.display_label().to_ascii_lowercase()
        );
        if group.key.area == IntegrationArea::Hooks {
            remediation.push(' ');
            remediation.push_str(CODEX_HOOK_TRUST_GUIDANCE);
        }
        problems.push(DoctorProblem {
            kind: ProblemKind::CodexIntegrationContentMismatch,
            category: ProblemCategory::RepoAssets,
            severity: ProblemSeverity::Error,
            fixability: ProblemFixability::ManualOnly,
            summary: format!(
                "{} file(s) differ from the canonical embedded content: {}.",
                group.display_label(),
                mismatched_paths
            ),
            remediation,
            next_action: "manual_steps",
            scope: Some(group.key),
            mutation_scope_target: None,
        });
    }
}

fn push_codex_integration_read_fail_problems(
    integration_groups: &[IntegrationGroupHealth],
    problems: &mut Vec<DoctorProblem>,
) {
    for group in integration_groups {
        for child in &group.children {
            let IntegrationContentState::ReadFailed(error) = &child.content_state else {
                continue;
            };
            problems.push(DoctorProblem {
                kind: ProblemKind::CodexAssetReadFailed,
                category: ProblemCategory::FilesystemPermissions,
                severity: ProblemSeverity::Error,
                fixability: ProblemFixability::ManualOnly,
                summary: format!(
                    "Unable to read Codex asset '{}' at '{}': {error}",
                    child.relative_path,
                    child.path.display()
                ),
                remediation: format!(
                    "Verify that '{}' is readable before rerunning 'sce doctor'.",
                    child.path.display()
                ),
                next_action: "manual_steps",
                scope: Some(group.key),
                mutation_scope_target: None,
            });
        }
    }
}

const OPENCODE_MUTATION_SCOPE_PLUGIN_ENTRY: &str = "./plugins/sce-mutation-scope.ts";

fn inspect_opencode_plugin_ordering_health(
    repository_root: &Path,
    problems: &mut Vec<DoctorProblem>,
) {
    let repo_paths = RepoPaths::new(repository_root);
    let manifest_path = repo_paths.opencode_manifest_file();
    let Ok(bytes) = fs::read(&manifest_path) else {
        return;
    };
    let Ok(manifest) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return;
    };
    let Some(plugins) = manifest.get("plugin").and_then(serde_json::Value::as_array) else {
        return;
    };
    let entries: Vec<&str> = plugins
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    let Some(position) = entries
        .iter()
        .position(|entry| *entry == OPENCODE_MUTATION_SCOPE_PLUGIN_ENTRY)
    else {
        return;
    };
    if position + 1 == entries.len() {
        return;
    }

    let trailing = entries[position + 1..].join(", ");
    problems.push(DoctorProblem {
        kind: ProblemKind::OpenCodePluginRegistryInvalid,
        category: ProblemCategory::RepoAssets,
        severity: ProblemSeverity::Error,
        fixability: ProblemFixability::ManualOnly,
        summary: format!(
            "OpenCode plugin registry '{}' lists '{OPENCODE_MUTATION_SCOPE_PLUGIN_ENTRY}' before other plugins ({trailing}); it must be the final plugin so an earlier plugin can reject a tool before the mutation-scope Start is established.",
            manifest_path.display()
        ),
        remediation: format!(
            "Move '{OPENCODE_MUTATION_SCOPE_PLUGIN_ENTRY}' to the end of the 'plugin' array in '{}', or reinstall OpenCode assets, then rerun 'sce doctor'.",
            manifest_path.display()
        ),
        next_action: "manual_steps",
        scope: Some(IntegrationGroupKey::new(
            IntegrationTarget::OpenCode,
            IntegrationArea::Plugins,
        )),
        mutation_scope_target: None,
    });
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
        scope: Some(IntegrationGroupKey::new(
            IntegrationTarget::OpenCode,
            IntegrationArea::Plugins,
        )),
        mutation_scope_target: None,
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
        scope: Some(IntegrationGroupKey::new(
            IntegrationTarget::OpenCode,
            IntegrationArea::Plugins,
        )),
        mutation_scope_target: None,
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
    sort_integration_children(&mut command_children);
    sort_integration_children(&mut skill_children);

    vec![
        IntegrationGroupHealth::new(
            IntegrationGroupKey::new(IntegrationTarget::ClaudeCode, IntegrationArea::Plugins),
            plugin_children,
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

/// Codex's embedded-asset relative paths keep their own `.agents/`/`.codex/`
/// output-root prefix (see `codex_asset`), so the integration root is the
/// repository root itself rather than a single per-target subdirectory.
fn collect_codex_integration_groups(
    repository_root: &Path,
    selected_optional_workflows: &[String],
    trust_context: &codex_hook_trust::TrustContext,
    policy_readiness: &CodexHookPolicyReadiness,
) -> Vec<IntegrationGroupHealth> {
    let codex_root = InstallTargetPaths::new(repository_root).codex_target_dir();
    let embedded_assets = iter_embedded_assets_for_setup_target_with_selection(
        SetupTarget::Codex,
        selected_optional_workflows,
    )
    .collect::<Vec<_>>();
    let mut skill_children = Vec::new();
    let mut hook_children = Vec::new();
    let mut hooks_json_generated_bytes: Option<&'static [u8]> = None;

    for asset in embedded_assets {
        if asset.relative_path == CODEX_HOOKS_JSON_RELATIVE_PATH {
            // `.codex/hooks.json` is diagnosed per required registration
            // (structural state plus Codex's own hook-trust readiness)
            // instead of as a single whole-file child; see
            // `codex_hooks_json_registration_children`.
            hooks_json_generated_bytes = Some(asset.bytes);
            continue;
        }
        let child = build_integration_child_from_asset(&codex_root, asset, None);

        if child
            .relative_path
            .starts_with(&format!("{}/", codex_asset::SKILLS_DIR))
        {
            skill_children.push(child);
        } else if child
            .relative_path
            .starts_with(&format!("{}/", repo_dir::CODEX))
        {
            hook_children.push(child);
        }
    }

    if let Some(generated_bytes) = hooks_json_generated_bytes {
        let hooks_json_path = codex_root.join(CODEX_HOOKS_JSON_RELATIVE_PATH);
        hook_children.extend(codex_hooks_json_registration_children(
            &hooks_json_path,
            generated_bytes,
            trust_context,
            policy_readiness,
        ));
    }

    sort_integration_children(&mut skill_children);
    sort_integration_children(&mut hook_children);

    vec![
        IntegrationGroupHealth::new(
            IntegrationGroupKey::new(IntegrationTarget::Codex, IntegrationArea::Skills),
            skill_children,
        ),
        IntegrationGroupHealth::new(
            IntegrationGroupKey::new(IntegrationTarget::Codex, IntegrationArea::Hooks),
            hook_children,
        ),
    ]
}

/// `.codex/hooks.json`'s relative path within Codex's embedded-asset set.
const CODEX_HOOKS_JSON_RELATIVE_PATH: &str = ".codex/hooks.json";

fn codex_hooks_json_registration_children(
    hooks_json_path: &Path,
    generated_bytes: &[u8],
    trust_context: &codex_hook_trust::TrustContext,
    policy_readiness: &CodexHookPolicyReadiness,
) -> Vec<IntegrationChildHealth> {
    let existing_bytes = match fs::read(hooks_json_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return codex_required_registration_children(
                hooks_json_path,
                &IntegrationContentState::ReadFailed(error.to_string()),
            );
        }
    };

    let document_diagnosis =
        match codex_hook_config::diagnose_document(existing_bytes.as_deref(), generated_bytes) {
            Ok(document_diagnosis) => document_diagnosis,
            Err(error) => codex_hook_config::HooksDocumentDiagnosis::Malformed(error.to_string()),
        };

    match document_diagnosis {
        codex_hook_config::HooksDocumentDiagnosis::Absent => {
            codex_required_registration_children(hooks_json_path, &IntegrationContentState::Missing)
        }
        codex_hook_config::HooksDocumentDiagnosis::Malformed(error) => {
            codex_required_registration_children(
                hooks_json_path,
                &IntegrationContentState::Malformed(error),
            )
        }
        codex_hook_config::HooksDocumentDiagnosis::Registrations(diagnoses) => diagnoses
            .iter()
            .map(|registration_diagnosis| {
                codex_hook_registration_child(
                    hooks_json_path,
                    registration_diagnosis,
                    trust_context,
                    policy_readiness,
                )
            })
            .collect(),
    }
}

fn codex_registration_suffix(
    command: codex_hook_config::CodexHookCommand,
    event: &str,
    matcher: Option<&str>,
) -> String {
    match command {
        codex_hook_config::CodexHookCommand::Codex => match matcher {
            Some(matcher) => format!("{event}({matcher})"),
            None => event.to_string(),
        },
        codex_hook_config::CodexHookCommand::MutationScope => format!("{event}(mutation-scope)"),
    }
}

fn codex_required_registration_children(
    hooks_json_path: &Path,
    content_state: &IntegrationContentState,
) -> Vec<IntegrationChildHealth> {
    codex_hook_config::required_registrations()
        .into_iter()
        .map(|(command, event, matcher)| {
            let suffix = codex_registration_suffix(command, event, matcher);
            IntegrationChildHealth {
                relative_path: format!("{CODEX_HOOKS_JSON_RELATIVE_PATH}#{suffix}"),
                path: hooks_json_path.to_path_buf(),
                content_state: content_state.clone(),
            }
        })
        .collect()
}

/// Human-readable explanation for `IntegrationContentState::PolicyBlocked`,
/// shared by both the per-registration content state and the aggregate
/// problem summary so their wording stays in sync.
const CODEX_HOOK_POLICY_BLOCKED_REASON: &str =
    "Codex's effective 'allow_managed_hooks_only' policy is enabled, so Codex will not load \
     this project-owned (non-managed) '.codex/hooks.json' registration.";

fn codex_hook_registration_child(
    hooks_json_path: &Path,
    diagnosis: &codex_hook_config::RegistrationDiagnosis,
    trust_context: &codex_hook_trust::TrustContext,
    policy_readiness: &CodexHookPolicyReadiness,
) -> IntegrationChildHealth {
    let suffix = codex_registration_suffix(diagnosis.command, diagnosis.event, diagnosis.matcher);
    let relative_path = format!("{CODEX_HOOKS_JSON_RELATIVE_PATH}#{suffix}");

    // Decision order (AC28): structural state wins first (a missing or stale
    // registration has no canonical on-disk handler for Codex to ever load,
    // so policy/trust cannot apply); only a structurally current registration
    // is further gated on Codex's effective hook-discovery *policy*, and only
    // once policy allows project hooks at all is per-handler *trust*
    // consulted. Policy and trust are independent dimensions — see
    // `codex_hook_policy` and `codex_hook_trust`'s module documentation.
    let content_state = match &diagnosis.state {
        codex_hook_config::RegistrationStructuralState::Missing => IntegrationContentState::Missing,
        codex_hook_config::RegistrationStructuralState::Stale => IntegrationContentState::Stale,
        codex_hook_config::RegistrationStructuralState::PresentAndCurrent => {
            let (Some(handler), Some(position)) = (&diagnosis.owned_handler, diagnosis.position)
            else {
                // Structurally impossible: `PresentAndCurrent` always carries
                // both. Treat defensively as stale rather than panicking.
                return IntegrationChildHealth {
                    relative_path,
                    path: hooks_json_path.to_path_buf(),
                    content_state: IntegrationContentState::Stale,
                };
            };

            match policy_readiness {
                CodexHookPolicyReadiness::PolicyBlocked => IntegrationContentState::PolicyBlocked(
                    CODEX_HOOK_POLICY_BLOCKED_REASON.to_string(),
                ),
                CodexHookPolicyReadiness::Unknown(reason) => {
                    IntegrationContentState::PolicyUnknown(reason.clone())
                }
                CodexHookPolicyReadiness::ProjectHooksAllowed => {
                    match codex_hook_trust::trust_readiness(
                        trust_context,
                        hooks_json_path,
                        diagnosis.event,
                        diagnosis.matcher,
                        handler,
                        position,
                    ) {
                        codex_hook_trust::TrustReadiness::Trusted => IntegrationContentState::Match,
                        codex_hook_trust::TrustReadiness::Untrusted => {
                            IntegrationContentState::NotTrusted("untrusted".to_string())
                        }
                        codex_hook_trust::TrustReadiness::Modified => {
                            IntegrationContentState::NotTrusted("modified".to_string())
                        }
                        codex_hook_trust::TrustReadiness::Disabled => {
                            IntegrationContentState::NotTrusted("disabled".to_string())
                        }
                        codex_hook_trust::TrustReadiness::Unknown(_) => {
                            IntegrationContentState::NotTrusted("unknown".to_string())
                        }
                    }
                }
            }
        }
    };

    IntegrationChildHealth {
        relative_path,
        path: hooks_json_path.to_path_buf(),
        content_state,
    }
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

fn doctor_git_output(git: &impl GitOps, repository_root: &Path, args: &[&str]) -> Option<String> {
    let output = git.run_command(repository_root, args).ok()?;
    let trimmed = output.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
