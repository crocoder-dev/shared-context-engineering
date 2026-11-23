use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::app::{ContextWithRepoRoot, HasRepoRoot};
use crate::services::codex_hook_policy;
use crate::services::config::{ResolvedAccessPolicy, TelemetryPolicyState};
use crate::services::default_paths::{resolve_sce_default_locations, resolve_state_data_root};
use crate::services::lifecycle::{
    lifecycle_providers, FixOutcome, HealthProblem, LifecycleProvider,
};
use crate::services::managed_telemetry::ManagedTelemetryHealth;
use crate::services::mutation_trace::runtime::{
    inspect_reconciliation_recommendation, run_explicit_reconciliation, ReconciliationCounts,
    ReconciliationFix, ReconciliationRecommendation,
};
use crate::services::output_format::OutputFormat;
use crate::services::setup;

mod access;
mod fixes;
mod inspect;
mod managed_telemetry;
mod render;
pub(crate) mod types;

pub mod command;

use access::DoctorAccess;
use fixes::build_manual_fix_results;
use inspect::{
    build_report_with_lifecycle_problems, finalize_mutation_scope_repair_results,
    mutation_scope_repair_seam, repair_blocked_mutation_scope_targets_with_seam,
    repair_merge_target_configs,
};
use render::render_report;
use types::{DoctorFixResultRecord, DoctorProblem, FixResult, HookDoctorReport, ProblemCategory};

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
    managed_telemetry: ManagedTelemetryHealth,
    access: DoctorAccess,
}

struct DoctorExecution {
    report: HookDoctorReport,
    managed_telemetry: ManagedTelemetryHealth,
    access: DoctorAccess,
    fix_results: Vec<DoctorFixResultRecord>,
    ref_reconciliation: Option<ReconciliationRecommendation>,
    ref_reconciliation_fix: Option<ReconciliationFix>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProviderHealthProblem {
    provider: LifecycleProvider,
    problem: HealthProblem,
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
    let policy = access_policy_for(repository_root);
    let access =
        DoctorAccess::inspect(policy.as_ref(), || access::inspect_login(repository_root)).await;
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
            managed_telemetry: crate::services::managed_telemetry::inspect_health(
                access.telemetry == TelemetryPolicyState::Enabled,
            )
            .await,
            access,
        },
        mutation_scope_seam,
        reconcile_seam,
        recommendation_seam,
    )
    .await
}

fn access_policy_for(repository_root: &Path) -> Option<ResolvedAccessPolicy> {
    crate::services::config::resolve_observability_runtime_config(repository_root)
        .ok()
        .map(|config| config.access_policy)
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
    let managed_telemetry = dependencies.managed_telemetry;
    let access = dependencies.access.clone();

    let providers = lifecycle_providers(true);
    let initial_problems = diagnose_lifecycle_providers(context, &providers).await;
    let initial_doctor_problems = initial_problems
        .iter()
        .map(|entry| DoctorProblem::from(entry.problem.clone()))
        .chain(access.login_problem())
        .chain(managed_telemetry::problem(managed_telemetry))
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
            managed_telemetry,
            access,
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
        .map(|entry| DoctorProblem::from(entry.problem))
        .chain(access.login_problem())
        .chain(managed_telemetry::problem(managed_telemetry))
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
        managed_telemetry,
        access,
        fix_results,
        ref_reconciliation: None,
        ref_reconciliation_fix: Some(reconciliation_fix),
    }
}

fn reconciliation_completed_text(counts: &ReconciliationCounts) -> String {
    let mut text = format!(
        "Snapshot ref reconciliation completed: deleted {}, retained {}, locally required {}.",
        counts.deleted, counts.retained, counts.local_required
    );
    if counts.pruned_events > 0 {
        let _ = write!(
            text,
            " Pruned {} mutation event(s) whose snapshot trees are absent from this repository.",
            counts.pruned_events
        );
    }
    if counts.repinned > 0 {
        let _ = write!(
            text,
            " Re-pinned {} snapshot tree(s) that were missing pins.",
            counts.repinned
        );
    }
    text
}

fn reconciliation_fix_result(fix: &ReconciliationFix) -> DoctorFixResultRecord {
    let (outcome, detail) = match fix {
        ReconciliationFix::Completed(counts) => (
            FixResult::Fixed,
            reconciliation_completed_text(counts),
        ),
        ReconciliationFix::CompletedStatePersistFailed { counts, warning } => (
            FixResult::Fixed,
            format!(
                "{} Warning: {warning}",
                reconciliation_completed_text(counts)
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
) -> Vec<ProviderHealthProblem> {
    let mut problems = Vec::new();
    for &provider in providers {
        problems.extend(
            provider
                .diagnose(context)
                .await
                .into_iter()
                .map(|problem| ProviderHealthProblem { provider, problem }),
        );
    }
    problems
}

async fn fix_lifecycle_providers(
    context: &impl HasRepoRoot,
    providers: &[LifecycleProvider],
    problems: &[ProviderHealthProblem],
) -> Vec<DoctorFixResultRecord> {
    let mut results = Vec::new();
    for &provider in providers {
        let owned_problems = problems
            .iter()
            .filter(|entry| entry.provider == provider)
            .map(|entry| entry.problem.clone())
            .collect::<Vec<_>>();
        results.extend(
            provider
                .fix(context, &owned_problems)
                .await
                .into_iter()
                .map(doctor_fix_result_from_lifecycle),
        );
    }
    results
}

fn doctor_fix_result_from_lifecycle(
    result: crate::services::lifecycle::FixResultRecord,
) -> DoctorFixResultRecord {
    DoctorFixResultRecord {
        category: result.category,
        outcome: match result.outcome {
            FixOutcome::Fixed => FixResult::Fixed,
            FixOutcome::Skipped => FixResult::Skipped,
            FixOutcome::Failed => FixResult::Failed,
        },
        detail: result.detail,
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
        repinned: 0,
        pruned_events: 0,
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
    fn reconciliation_fix_results_render_expected_text() {
        struct Case {
            label: &'static str,
            fix: ReconciliationFix,
            outcome: FixResult,
            required: Vec<&'static str>,
            forbidden: Vec<&'static str>,
        }
        let cases = [
            Case {
                label: "completed",
                fix: ReconciliationFix::Completed(COUNTS),
                outcome: FixResult::Fixed,
                required: vec![
                    "Snapshot ref reconciliation completed: deleted 3, retained 2, locally required 1.",
                ],
                forbidden: vec!["Warning", "failed"],
            },
            Case {
                label: "pre-rename persistence failure",
                fix: ReconciliationFix::CompletedStatePersistFailed {
                    counts: COUNTS,
                    warning: not_applied(),
                },
                outcome: FixResult::Fixed,
                required: vec![
                    "deleted 3",
                    "Maintenance-state update was not applied.",
                    "disk full",
                ],
                forbidden: vec!["were not recorded", "failed"],
            },
            Case {
                label: "post-rename persistence failure",
                fix: ReconciliationFix::CompletedStatePersistFailed {
                    counts: COUNTS,
                    warning: durability_uncertain(),
                },
                outcome: FixResult::Fixed,
                required: vec![
                    "deleted 3",
                    "Maintenance-state durability could not be confirmed.",
                    "io error",
                ],
                forbidden: vec!["not applied", "were not recorded", "failed"],
            },
            Case {
                label: "unreadable previous state",
                fix: ReconciliationFix::CompletedStatePersistFailed {
                    counts: COUNTS,
                    warning: StateWarning::PreviousStateUnreadable {
                        cause: "permission denied".to_string(),
                    },
                },
                outcome: FixResult::Fixed,
                required: vec!["deleted 3", "could not be read", "permission denied"],
                forbidden: vec!["were not recorded", "failed"],
            },
            Case {
                label: "failed pass",
                fix: ReconciliationFix::Failed {
                    kind: "agent_trace_db_missing",
                    message: "db missing".to_string(),
                    state_warning: Some(not_applied()),
                },
                outcome: FixResult::Failed,
                required: vec![
                    "agent_trace_db_missing",
                    "db missing",
                    "Maintenance-state update was not applied.",
                ],
                forbidden: vec!["completed"],
            },
            Case {
                label: "busy pass",
                fix: ReconciliationFix::Busy,
                outcome: FixResult::Skipped,
                required: vec!["skipped"],
                forbidden: vec!["completed", "deleted"],
            },
        ];
        for case in cases {
            let record = reconciliation_fix_result(&case.fix);
            assert_eq!(record.outcome, case.outcome, "{}", case.label);
            for fragment in case.required {
                assert!(
                    record.detail.contains(fragment),
                    "{}: missing {fragment:?} in {:?}",
                    case.label,
                    record.detail
                );
            }
            for fragment in case.forbidden {
                assert!(
                    !record.detail.contains(fragment),
                    "{}: unexpected {fragment:?} in {:?}",
                    case.label,
                    record.detail
                );
            }
        }
    }
}
