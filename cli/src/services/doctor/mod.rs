use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::app::{ContextWithRepoRoot, HasRepoRoot};
use crate::services::codex_hook_policy;
use crate::services::default_paths::{resolve_sce_default_locations, resolve_state_data_root};
use crate::services::lifecycle::{
    lifecycle_providers, FixOutcome, HealthCategory, HealthFixability, HealthProblem,
    HealthProblemKind, HealthSeverity, LifecycleProvider, LifecycleProviderId,
};
use crate::services::mutation_trace::runtime::{
    inspect_reconciliation_recommendation, run_explicit_reconciliation, ReconciliationFix,
    ReconciliationRecommendation,
};
use crate::services::output_format::OutputFormat;
use crate::services::setup;

mod fixes;
mod inspect;
mod render;
pub(crate) mod types;

pub mod command;

use fixes::build_manual_fix_results;
use inspect::{
    build_report_with_lifecycle_problems, finalize_mutation_scope_repair_results,
    mutation_scope_repair_seam, repair_blocked_mutation_scope_targets_with_seam,
    repair_merge_target_configs,
};
use render::render_report;
use types::{
    DoctorFixResultRecord, DoctorProblem, FixResult, HookDoctorReport, ProblemCategory,
    ProblemFixability, ProblemKind, ProblemSeverity,
};

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

struct DoctorDependencies<'a, G, S, C, P> {
    git: &'a G,
    resolve_state_root: &'a S,
    resolve_global_config_path: &'a C,
    probe_codex_hook_policy: &'a P,
}

struct DoctorExecution {
    report: HookDoctorReport,
    fix_results: Vec<DoctorFixResultRecord>,
    ref_reconciliation: Option<ReconciliationRecommendation>,
    ref_reconciliation_fix: Option<ReconciliationFix>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProviderDoctorProblem {
    provider_id: LifecycleProviderId,
    problem: DoctorProblem,
}

pub async fn run_doctor_with_context<C>(request: DoctorRequest, context: &C) -> Result<String>
where
    C: ContextWithRepoRoot + crate::app::HasGit,
{
    let repository_root = if let Some(path) = context.repo_root() {
        path.to_path_buf()
    } else {
        let current_dir =
            std::env::current_dir().context("Failed to determine current directory")?;
        setup::ensure_git_repository(&current_dir).unwrap_or(current_dir)
    };
    let scoped_context = context.with_repo_root(&repository_root);
    let execution = execute_doctor_with_context(
        request,
        &repository_root,
        &scoped_context,
        &mutation_scope_repair_seam::<crate::services::observability::traits::NoopLogger>,
        &run_explicit_reconciliation,
        &inspect_reconciliation_recommendation,
    )
    .await;
    render_report(request, &execution)
}

async fn execute_doctor_with_context(
    request: DoctorRequest,
    repository_root: &Path,
    context: &(impl HasRepoRoot + crate::app::HasGit),
    mutation_scope_seam: &impl std::ops::AsyncFn(
        &Path,
        &str,
        Option<&crate::services::observability::traits::NoopLogger>,
    ) -> anyhow::Result<String>,
    reconcile_seam: &impl std::ops::AsyncFn(&Path) -> ReconciliationFix,
    recommendation_seam: &impl Fn(&Path) -> Option<ReconciliationRecommendation>,
) -> DoctorExecution {
    execute_doctor_with_lifecycle_providers(
        request,
        repository_root,
        context,
        &DoctorDependencies {
            git: context.git(),
            resolve_state_root: &resolve_state_data_root,
            resolve_global_config_path: &|| {
                Ok(resolve_sce_default_locations()?.global_config_file())
            },
            probe_codex_hook_policy: &codex_hook_policy::probe_default,
        },
        mutation_scope_seam,
        reconcile_seam,
        recommendation_seam,
    )
    .await
}

async fn execute_doctor_with_lifecycle_providers(
    request: DoctorRequest,
    repository_root: &Path,
    context: &impl HasRepoRoot,
    dependencies: &DoctorDependencies<
        '_,
        impl crate::services::capabilities::GitOps,
        impl Fn() -> Result<PathBuf>,
        impl Fn() -> Result<PathBuf>,
        impl Fn() -> crate::services::codex_hook_policy::CodexHookPolicyReadiness,
    >,
    mutation_scope_seam: &impl std::ops::AsyncFn(
        &Path,
        &str,
        Option<&crate::services::observability::traits::NoopLogger>,
    ) -> anyhow::Result<String>,
    reconcile_seam: &impl std::ops::AsyncFn(&Path) -> ReconciliationFix,
    recommendation_seam: &impl Fn(&Path) -> Option<ReconciliationRecommendation>,
) -> DoctorExecution {
    let policy_readiness = (dependencies.probe_codex_hook_policy)();

    let providers = lifecycle_providers(true);
    let initial_problems = diagnose_lifecycle_providers(context, &providers).await;
    let initial_doctor_problems = initial_problems
        .iter()
        .map(|problem| problem.problem.clone())
        .collect::<Vec<_>>();
    let initial_report = build_report_with_lifecycle_problems(
        request.mode,
        repository_root,
        dependencies,
        initial_doctor_problems,
        &policy_readiness,
    )
    .await;

    if request.mode != DoctorMode::Fix {
        return DoctorExecution {
            report: initial_report,
            fix_results: Vec::new(),
            ref_reconciliation: recommendation_seam(repository_root),
            ref_reconciliation_fix: None,
        };
    }

    let mut fix_results = fix_lifecycle_providers(context, &providers, &initial_problems).await;
    fix_results.extend(repair_merge_target_configs(
        repository_root,
        &policy_readiness,
    ));
    let mutation_scope_repairs =
        repair_blocked_mutation_scope_targets_with_seam(&initial_report, mutation_scope_seam).await;
    let final_problems = diagnose_lifecycle_providers(context, &providers).await;
    let final_doctor_problems = final_problems
        .into_iter()
        .map(|problem| problem.problem)
        .collect::<Vec<_>>();
    let final_report = build_report_with_lifecycle_problems(
        request.mode,
        repository_root,
        dependencies,
        final_doctor_problems,
        &policy_readiness,
    )
    .await;
    fix_results.extend(finalize_mutation_scope_repair_results(
        &mutation_scope_repairs,
        &final_report.mutation_scope_health,
    ));
    fix_results.extend(build_manual_fix_results(
        &final_report,
        &mutation_scope_repairs,
    ));

    let reconciliation_fix = reconcile_seam(repository_root).await;
    fix_results.push(reconciliation_fix_result(&reconciliation_fix));

    DoctorExecution {
        report: final_report,
        fix_results,
        ref_reconciliation: None,
        ref_reconciliation_fix: Some(reconciliation_fix),
    }
}

fn reconciliation_fix_result(fix: &ReconciliationFix) -> DoctorFixResultRecord {
    let (outcome, detail) = match fix {
        ReconciliationFix::Completed(counts) => (
            FixResult::Fixed,
            format!(
                "Snapshot ref reconciliation completed: deleted {}, retained {}, locally required {}.",
                counts.deleted, counts.retained, counts.local_required
            ),
        ),
        ReconciliationFix::CompletedStatePersistFailed { counts, warning } => (
            FixResult::Fixed,
            format!(
                "Snapshot ref reconciliation completed: deleted {}, retained {}, locally required {}. Warning: {warning}",
                counts.deleted, counts.retained, counts.local_required
            ),
        ),
        ReconciliationFix::Failed {
            kind,
            message,
            state_warning,
        } => {
            let mut detail = format!("Snapshot ref reconciliation failed ({kind}): {message}");
            if let Some(warning) = state_warning {
                detail = format!("{detail} Warning: {warning}");
            }
            (FixResult::Failed, detail)
        }
        ReconciliationFix::Busy => (
            FixResult::Skipped,
            "Snapshot ref reconciliation skipped: another mutation boundary or maintenance pass holds the worktree lock. Retry `sce doctor --fix`.".to_string(),
        ),
    };
    DoctorFixResultRecord {
        category: ProblemCategory::MutationScopeHealth,
        outcome,
        detail,
    }
}

async fn diagnose_lifecycle_providers(
    context: &impl HasRepoRoot,
    providers: &[LifecycleProvider],
) -> Vec<ProviderDoctorProblem> {
    let mut problems = Vec::new();
    for provider in providers {
        let provider_id = provider.id();
        problems.extend(provider.diagnose(context).await.into_iter().map(|problem| {
            ProviderDoctorProblem {
                provider_id,
                problem: doctor_problem_from_health(problem),
            }
        }));
    }
    problems
}

async fn fix_lifecycle_providers(
    context: &impl HasRepoRoot,
    providers: &[LifecycleProvider],
    problems: &[ProviderDoctorProblem],
) -> Vec<DoctorFixResultRecord> {
    let mut results = Vec::new();
    for provider in providers {
        let health_problems = problems
            .iter()
            .filter(|problem| problem.provider_id == provider.id())
            .map(|problem| health_problem_from_doctor(problem.problem.clone()))
            .collect::<Vec<_>>();
        results.extend(
            provider
                .fix(context, &health_problems)
                .await
                .into_iter()
                .map(doctor_fix_result_from_lifecycle),
        );
    }
    results
}

fn doctor_problem_from_health(problem: HealthProblem) -> DoctorProblem {
    DoctorProblem {
        kind: doctor_problem_kind(problem.kind),
        category: doctor_problem_category(problem.category),
        severity: doctor_problem_severity(problem.severity),
        fixability: doctor_problem_fixability(problem.fixability),
        summary: problem.summary,
        remediation: problem.remediation,
        next_action: problem.next_action,
        scope: None,
        mutation_scope_target: None,
    }
}

fn health_problem_from_doctor(problem: DoctorProblem) -> HealthProblem {
    HealthProblem {
        kind: health_problem_kind(problem.kind),
        category: health_problem_category(problem.category),
        severity: health_problem_severity(problem.severity),
        fixability: health_problem_fixability(problem.fixability),
        summary: problem.summary,
        remediation: problem.remediation,
        next_action: problem.next_action,
    }
}

fn doctor_fix_result_from_lifecycle(
    result: crate::services::lifecycle::FixResultRecord,
) -> DoctorFixResultRecord {
    DoctorFixResultRecord {
        category: doctor_problem_category(result.category),
        outcome: match result.outcome {
            FixOutcome::Fixed => FixResult::Fixed,
            FixOutcome::Skipped => FixResult::Skipped,
            FixOutcome::Failed => FixResult::Failed,
        },
        detail: result.detail,
    }
}

fn doctor_problem_category(category: HealthCategory) -> ProblemCategory {
    match category {
        HealthCategory::GlobalState => ProblemCategory::GlobalState,
        HealthCategory::RepositoryTargeting => ProblemCategory::RepositoryTargeting,
        HealthCategory::HookRollout => ProblemCategory::HookRollout,
        HealthCategory::RepoAssets => ProblemCategory::RepoAssets,
        HealthCategory::FilesystemPermissions => ProblemCategory::FilesystemPermissions,
        HealthCategory::MutationScopeHealth => ProblemCategory::MutationScopeHealth,
    }
}

fn health_problem_category(category: ProblemCategory) -> HealthCategory {
    match category {
        ProblemCategory::GlobalState => HealthCategory::GlobalState,
        ProblemCategory::RepositoryTargeting => HealthCategory::RepositoryTargeting,
        ProblemCategory::HookRollout => HealthCategory::HookRollout,
        ProblemCategory::RepoAssets => HealthCategory::RepoAssets,
        ProblemCategory::FilesystemPermissions => HealthCategory::FilesystemPermissions,
        ProblemCategory::MutationScopeHealth => HealthCategory::MutationScopeHealth,
    }
}

fn doctor_problem_severity(severity: HealthSeverity) -> ProblemSeverity {
    match severity {
        HealthSeverity::Error => ProblemSeverity::Error,
        HealthSeverity::Warning => ProblemSeverity::Warning,
    }
}

fn health_problem_severity(severity: ProblemSeverity) -> HealthSeverity {
    match severity {
        ProblemSeverity::Error => HealthSeverity::Error,
        ProblemSeverity::Warning => HealthSeverity::Warning,
    }
}

fn doctor_problem_fixability(fixability: HealthFixability) -> ProblemFixability {
    match fixability {
        HealthFixability::AutoFixable => ProblemFixability::AutoFixable,
        HealthFixability::ManualOnly => ProblemFixability::ManualOnly,
        HealthFixability::NoActionRequired => ProblemFixability::NoActionRequired,
    }
}

fn health_problem_fixability(fixability: ProblemFixability) -> HealthFixability {
    match fixability {
        ProblemFixability::AutoFixable => HealthFixability::AutoFixable,
        ProblemFixability::ManualOnly => HealthFixability::ManualOnly,
        ProblemFixability::NoActionRequired => HealthFixability::NoActionRequired,
    }
}

fn doctor_problem_kind(kind: HealthProblemKind) -> ProblemKind {
    match kind {
        HealthProblemKind::GitUnavailable => ProblemKind::GitUnavailable,
        HealthProblemKind::BareRepository => ProblemKind::BareRepository,
        HealthProblemKind::NotInsideGitRepository => ProblemKind::NotInsideGitRepository,
        HealthProblemKind::UnableToResolveGitHooksDirectory => {
            ProblemKind::UnableToResolveGitHooksDirectory
        }
        HealthProblemKind::UnableToResolveStateRoot => ProblemKind::UnableToResolveStateRoot,
        HealthProblemKind::GlobalConfigValidationFailed => {
            ProblemKind::GlobalConfigValidationFailed
        }
        HealthProblemKind::UnableToResolveGlobalConfigPath => {
            ProblemKind::UnableToResolveGlobalConfigPath
        }
        HealthProblemKind::LocalConfigValidationFailed => ProblemKind::LocalConfigValidationFailed,
        HealthProblemKind::HooksDirectoryMissing => ProblemKind::HooksDirectoryMissing,
        HealthProblemKind::HooksPathNotDirectory => ProblemKind::HooksPathNotDirectory,
        HealthProblemKind::RequiredHookMissing => ProblemKind::RequiredHookMissing,
        HealthProblemKind::HookNotExecutable => ProblemKind::HookNotExecutable,
        HealthProblemKind::HookContentStale => ProblemKind::HookContentStale,
        HealthProblemKind::NoIntegrationsInstalled => ProblemKind::NoIntegrationsInstalled,
        HealthProblemKind::OpenCodeIntegrationFilesMissing => {
            ProblemKind::OpenCodeIntegrationFilesMissing
        }
        HealthProblemKind::OpenCodeIntegrationContentMismatch => {
            ProblemKind::OpenCodeIntegrationContentMismatch
        }
        HealthProblemKind::ClaudeIntegrationFilesMissing => {
            ProblemKind::ClaudeIntegrationFilesMissing
        }
        HealthProblemKind::ClaudeIntegrationContentMismatch => {
            ProblemKind::ClaudeIntegrationContentMismatch
        }
        HealthProblemKind::PiIntegrationFilesMissing => ProblemKind::PiIntegrationFilesMissing,
        HealthProblemKind::PiIntegrationContentMismatch => {
            ProblemKind::PiIntegrationContentMismatch
        }
        HealthProblemKind::CodexIntegrationFilesMissing => {
            ProblemKind::CodexIntegrationFilesMissing
        }
        HealthProblemKind::CodexIntegrationContentMismatch => {
            ProblemKind::CodexIntegrationContentMismatch
        }
        HealthProblemKind::OpenCodePluginRegistryInvalid => {
            ProblemKind::OpenCodePluginRegistryInvalid
        }
        HealthProblemKind::OpenCodeAssetMissingOrInvalid => {
            ProblemKind::OpenCodeAssetMissingOrInvalid
        }
        HealthProblemKind::HookReadFailed => ProblemKind::HookReadFailed,
        HealthProblemKind::OpenCodeAssetReadFailed => ProblemKind::OpenCodeAssetReadFailed,
        HealthProblemKind::ClaudeAssetReadFailed => ProblemKind::ClaudeAssetReadFailed,
        HealthProblemKind::PiAssetReadFailed => ProblemKind::PiAssetReadFailed,
        HealthProblemKind::CodexAssetReadFailed => ProblemKind::CodexAssetReadFailed,
        HealthProblemKind::CodexHookRegistrationMalformed => {
            ProblemKind::CodexHookRegistrationMalformed
        }
        HealthProblemKind::CodexHookRegistrationNotTrusted => {
            ProblemKind::CodexHookRegistrationNotTrusted
        }
        HealthProblemKind::CodexHookRegistrationPolicyBlocked => {
            ProblemKind::CodexHookRegistrationPolicyBlocked
        }
        HealthProblemKind::CodexHookRegistrationPolicyUnknown => {
            ProblemKind::CodexHookRegistrationPolicyUnknown
        }
        HealthProblemKind::AgentTraceDbConnectionFailed => {
            ProblemKind::AgentTraceDbConnectionFailed
        }
        HealthProblemKind::AgentTraceDbSchemaNotReady => ProblemKind::AgentTraceDbSchemaNotReady,
        HealthProblemKind::MutationScopeHealthRecovering => {
            ProblemKind::MutationScopeHealthRecovering
        }
        HealthProblemKind::MutationScopeHealthBlocked => ProblemKind::MutationScopeHealthBlocked,
        HealthProblemKind::MutationScopeHealthInvalid => ProblemKind::MutationScopeHealthInvalid,
    }
}

fn health_problem_kind(kind: ProblemKind) -> HealthProblemKind {
    match kind {
        ProblemKind::GitUnavailable => HealthProblemKind::GitUnavailable,
        ProblemKind::BareRepository => HealthProblemKind::BareRepository,
        ProblemKind::NotInsideGitRepository => HealthProblemKind::NotInsideGitRepository,
        ProblemKind::UnableToResolveGitHooksDirectory => {
            HealthProblemKind::UnableToResolveGitHooksDirectory
        }
        ProblemKind::UnableToResolveStateRoot => HealthProblemKind::UnableToResolveStateRoot,
        ProblemKind::GlobalConfigValidationFailed => {
            HealthProblemKind::GlobalConfigValidationFailed
        }
        ProblemKind::UnableToResolveGlobalConfigPath => {
            HealthProblemKind::UnableToResolveGlobalConfigPath
        }
        ProblemKind::LocalConfigValidationFailed => HealthProblemKind::LocalConfigValidationFailed,
        ProblemKind::HooksDirectoryMissing => HealthProblemKind::HooksDirectoryMissing,
        ProblemKind::HooksPathNotDirectory => HealthProblemKind::HooksPathNotDirectory,
        ProblemKind::RequiredHookMissing => HealthProblemKind::RequiredHookMissing,
        ProblemKind::HookNotExecutable => HealthProblemKind::HookNotExecutable,
        ProblemKind::HookContentStale => HealthProblemKind::HookContentStale,
        ProblemKind::NoIntegrationsInstalled => HealthProblemKind::NoIntegrationsInstalled,
        ProblemKind::OpenCodeIntegrationFilesMissing => {
            HealthProblemKind::OpenCodeIntegrationFilesMissing
        }
        ProblemKind::OpenCodeIntegrationContentMismatch => {
            HealthProblemKind::OpenCodeIntegrationContentMismatch
        }
        ProblemKind::ClaudeIntegrationFilesMissing => {
            HealthProblemKind::ClaudeIntegrationFilesMissing
        }
        ProblemKind::ClaudeIntegrationContentMismatch => {
            HealthProblemKind::ClaudeIntegrationContentMismatch
        }
        ProblemKind::PiIntegrationFilesMissing => HealthProblemKind::PiIntegrationFilesMissing,
        ProblemKind::PiIntegrationContentMismatch => {
            HealthProblemKind::PiIntegrationContentMismatch
        }
        ProblemKind::CodexIntegrationFilesMissing => {
            HealthProblemKind::CodexIntegrationFilesMissing
        }
        ProblemKind::CodexIntegrationContentMismatch => {
            HealthProblemKind::CodexIntegrationContentMismatch
        }
        ProblemKind::OpenCodePluginRegistryInvalid => {
            HealthProblemKind::OpenCodePluginRegistryInvalid
        }
        ProblemKind::OpenCodeAssetMissingOrInvalid => {
            HealthProblemKind::OpenCodeAssetMissingOrInvalid
        }
        ProblemKind::HookReadFailed => HealthProblemKind::HookReadFailed,
        ProblemKind::OpenCodeAssetReadFailed => HealthProblemKind::OpenCodeAssetReadFailed,
        ProblemKind::ClaudeAssetReadFailed => HealthProblemKind::ClaudeAssetReadFailed,
        ProblemKind::PiAssetReadFailed => HealthProblemKind::PiAssetReadFailed,
        ProblemKind::CodexAssetReadFailed => HealthProblemKind::CodexAssetReadFailed,
        ProblemKind::CodexHookRegistrationMalformed => {
            HealthProblemKind::CodexHookRegistrationMalformed
        }
        ProblemKind::CodexHookRegistrationNotTrusted => {
            HealthProblemKind::CodexHookRegistrationNotTrusted
        }
        ProblemKind::CodexHookRegistrationPolicyBlocked => {
            HealthProblemKind::CodexHookRegistrationPolicyBlocked
        }
        ProblemKind::CodexHookRegistrationPolicyUnknown => {
            HealthProblemKind::CodexHookRegistrationPolicyUnknown
        }
        ProblemKind::AgentTraceDbConnectionFailed => {
            HealthProblemKind::AgentTraceDbConnectionFailed
        }
        ProblemKind::AgentTraceDbSchemaNotReady => HealthProblemKind::AgentTraceDbSchemaNotReady,
        ProblemKind::MutationScopeHealthRecovering => {
            HealthProblemKind::MutationScopeHealthRecovering
        }
        ProblemKind::MutationScopeHealthBlocked => HealthProblemKind::MutationScopeHealthBlocked,
        ProblemKind::MutationScopeHealthInvalid => HealthProblemKind::MutationScopeHealthInvalid,
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

#[cfg(test)]
mod reconciliation_fix_tests {
    use super::{reconciliation_fix_result, FixResult, ReconciliationFix};
    use crate::services::mutation_trace::runtime::{ReconciliationCounts, StateWarning};

    const COUNTS: ReconciliationCounts = ReconciliationCounts {
        deleted: 3,
        retained: 2,
        local_required: 1,
    };

    fn not_applied() -> StateWarning {
        StateWarning::NotApplied {
            phase: "rename",
            cause: "disk full".to_string(),
        }
    }

    fn durability_uncertain() -> StateWarning {
        StateWarning::DurabilityUncertain {
            phase: "sync_parent_directory",
            cause: "io error".to_string(),
        }
    }

    #[test]
    fn completed_pass_reports_counts_as_fixed() {
        let record = reconciliation_fix_result(&ReconciliationFix::Completed(COUNTS));
        assert_eq!(record.outcome, FixResult::Fixed);
        assert_eq!(
            record.detail,
            "Snapshot ref reconciliation completed: deleted 3, retained 2, locally required 1."
        );
    }

    #[test]
    fn completed_pass_with_pre_rename_failure_says_update_was_not_applied() {
        let record = reconciliation_fix_result(&ReconciliationFix::CompletedStatePersistFailed {
            counts: COUNTS,
            warning: not_applied(),
        });
        assert_eq!(record.outcome, FixResult::Fixed);
        assert!(record.detail.contains("deleted 3"));
        assert!(record
            .detail
            .contains("Maintenance-state update was not applied."));
        assert!(record.detail.contains("disk full"));
        assert!(!record.detail.contains("were not recorded"));
    }

    #[test]
    fn completed_pass_with_post_rename_failure_says_durability_unconfirmed() {
        let record = reconciliation_fix_result(&ReconciliationFix::CompletedStatePersistFailed {
            counts: COUNTS,
            warning: durability_uncertain(),
        });
        assert_eq!(record.outcome, FixResult::Fixed);
        assert!(record
            .detail
            .contains("Maintenance-state durability could not be confirmed."));
        assert!(!record.detail.contains("not applied"));
        assert!(!record.detail.contains("were not recorded"));
    }

    #[test]
    fn unreadable_previous_state_does_not_claim_nothing_was_recorded() {
        let record = reconciliation_fix_result(&ReconciliationFix::CompletedStatePersistFailed {
            counts: COUNTS,
            warning: StateWarning::PreviousStateUnreadable {
                cause: "permission denied".to_string(),
            },
        });
        assert!(record.detail.contains("could not be read"));
        assert!(record.detail.contains("permission denied"));
        assert!(!record.detail.contains("were not recorded"));
    }

    #[test]
    fn failed_pass_is_never_reported_as_success() {
        let record = reconciliation_fix_result(&ReconciliationFix::Failed {
            kind: "agent_trace_db_missing",
            message: "db missing".to_string(),
            state_warning: Some(not_applied()),
        });
        assert_eq!(record.outcome, FixResult::Failed);
        assert!(record.detail.contains("agent_trace_db_missing"));
        assert!(record.detail.contains("db missing"));
        assert!(record
            .detail
            .contains("Maintenance-state update was not applied."));
    }

    #[test]
    fn busy_pass_is_skipped_not_fixed() {
        let record = reconciliation_fix_result(&ReconciliationFix::Busy);
        assert_eq!(record.outcome, FixResult::Skipped);
        assert!(record.detail.contains("skipped"));
    }
}
