use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde_json::json;

use crate::app::{ContextWithRepoRoot, HasRepoRoot};
use crate::services::checkout::registry::{self, CheckoutRecord};
use crate::services::default_paths::{resolve_sce_default_locations, resolve_state_data_root};
use crate::services::lifecycle::{
    lifecycle_providers, FixOutcome, HealthCategory, HealthFixability, HealthProblem,
    HealthProblemKind, HealthSeverity, LifecycleProvider, LifecycleProviderId,
};
use crate::services::output_format::OutputFormat;
use crate::services::setup;

mod fixes;
mod inspect;
mod render;
pub(crate) mod types;

pub mod command;

use fixes::build_manual_fix_results;
use inspect::build_report_with_lifecycle_problems;
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
pub enum DoctorAction {
    Report,
    Dbs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DoctorRequest {
    pub action: DoctorAction,
    pub mode: DoctorMode,
    pub format: DoctorFormat,
}

struct DoctorDependencies<'a> {
    run_git_command: &'a dyn Fn(&Path, &[&str]) -> Option<String>,
    check_git_available: &'a dyn Fn() -> bool,
    resolve_state_root: &'a dyn Fn() -> Result<PathBuf>,
    resolve_global_config_path: &'a dyn Fn() -> Result<PathBuf>,
    validate_config_file: &'a dyn Fn(&Path) -> Result<()>,
}

struct DoctorExecution {
    report: HookDoctorReport,
    fix_results: Vec<DoctorFixResultRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProviderDoctorProblem {
    provider_id: LifecycleProviderId,
    problem: DoctorProblem,
}

pub fn run_doctor_with_context<C>(request: DoctorRequest, context: &C) -> Result<String>
where
    C: ContextWithRepoRoot,
{
    if request.action == DoctorAction::Dbs {
        return run_doctor_dbs(request.format);
    }

    let repository_root = if let Some(path) = context.repo_root() {
        path.to_path_buf()
    } else {
        let current_dir =
            std::env::current_dir().context("Failed to determine current directory")?;
        setup::ensure_git_repository(&current_dir).unwrap_or(current_dir)
    };
    let scoped_context = context.with_repo_root(&repository_root);
    let execution = execute_doctor_with_context(request, &repository_root, &scoped_context);
    render_report(request, &execution)
}

fn run_doctor_dbs(format: DoctorFormat) -> Result<String> {
    let mut checkouts = registry::list_checkouts().context("failed to read checkout registry")?;
    sort_checkouts_by_last_seen_desc(&mut checkouts);

    match format {
        DoctorFormat::Text => Ok(render_doctor_dbs_text(&checkouts)),
        DoctorFormat::Json => render_doctor_dbs_json(&checkouts),
    }
}

fn sort_checkouts_by_last_seen_desc(checkouts: &mut [CheckoutRecord]) {
    checkouts.sort_by(|left, right| {
        right
            .last_seen
            .cmp(&left.last_seen)
            .then_with(|| left.checkout_id.cmp(&right.checkout_id))
    });
}

fn render_doctor_dbs_text(checkouts: &[CheckoutRecord]) -> String {
    let mut lines = vec![String::from("SCE doctor dbs")];

    if checkouts.is_empty() {
        lines.push(String::from("no registered checkouts"));
        return lines.join("\n");
    }

    for checkout in checkouts {
        lines.push(format!("checkout_id: {}", checkout.checkout_id));
        lines.push(format!("  path: {}", checkout.path));
        lines.push(format!(
            "  database_path: {}",
            checkout.database_path.as_deref().unwrap_or("none")
        ));
        lines.push(format!("  last_seen: {}", checkout.last_seen));
        lines.push(format!(
            "  remote_url: {}",
            checkout.remote_url.as_deref().unwrap_or("none")
        ));
    }

    lines.join("\n")
}

fn render_doctor_dbs_json(checkouts: &[CheckoutRecord]) -> Result<String> {
    let payload = json!({
        "status": "ok",
        "command": NAME,
        "subcommand": "dbs",
        "checkouts": checkouts.iter().map(|checkout| json!({
            "checkout_id": checkout.checkout_id,
            "path": checkout.path,
            "database_path": checkout.database_path,
            "last_seen": checkout.last_seen,
            "remote_url": checkout.remote_url,
        })).collect::<Vec<_>>(),
    });

    serde_json::to_string_pretty(&payload).context("failed to serialize doctor dbs report to JSON")
}

fn execute_doctor_with_context(
    request: DoctorRequest,
    repository_root: &Path,
    context: &impl HasRepoRoot,
) -> DoctorExecution {
    execute_doctor_with_lifecycle_providers(
        request,
        repository_root,
        context,
        &DoctorDependencies {
            run_git_command: &run_git_command,
            check_git_available: &is_git_available,
            resolve_state_root: &resolve_state_data_root,
            resolve_global_config_path: &|| {
                Ok(resolve_sce_default_locations()?.global_config_file())
            },
            validate_config_file: &crate::services::config::validate_config_file,
        },
    )
}

fn execute_doctor_with_lifecycle_providers(
    request: DoctorRequest,
    repository_root: &Path,
    context: &impl HasRepoRoot,
    dependencies: &DoctorDependencies<'_>,
) -> DoctorExecution {
    let providers = lifecycle_providers(true);
    let initial_problems = diagnose_lifecycle_providers(context, &providers);
    let initial_doctor_problems = initial_problems
        .iter()
        .map(|problem| problem.problem.clone())
        .collect::<Vec<_>>();
    let initial_report = build_report_with_lifecycle_problems(
        request.mode,
        repository_root,
        dependencies,
        initial_doctor_problems,
    );

    if request.mode != DoctorMode::Fix {
        return DoctorExecution {
            report: initial_report,
            fix_results: Vec::new(),
        };
    }

    let mut fix_results = fix_lifecycle_providers(context, &providers, &initial_problems);
    let final_problems = diagnose_lifecycle_providers(context, &providers);
    let final_doctor_problems = final_problems
        .into_iter()
        .map(|problem| problem.problem)
        .collect::<Vec<_>>();
    let final_report = build_report_with_lifecycle_problems(
        request.mode,
        repository_root,
        dependencies,
        final_doctor_problems,
    );
    fix_results.extend(build_manual_fix_results(&final_report));

    DoctorExecution {
        report: final_report,
        fix_results,
    }
}

fn diagnose_lifecycle_providers(
    context: &impl HasRepoRoot,
    providers: &[LifecycleProvider],
) -> Vec<ProviderDoctorProblem> {
    providers
        .iter()
        .flat_map(|provider| {
            let provider_id = provider.id();
            provider
                .diagnose(context)
                .into_iter()
                .map(move |problem| ProviderDoctorProblem {
                    provider_id,
                    problem: doctor_problem_from_health(problem),
                })
        })
        .collect()
}

fn fix_lifecycle_providers(
    context: &impl HasRepoRoot,
    providers: &[LifecycleProvider],
    problems: &[ProviderDoctorProblem],
) -> Vec<DoctorFixResultRecord> {
    providers
        .iter()
        .flat_map(|provider| {
            let health_problems = problems
                .iter()
                .filter(|problem| problem.provider_id == provider.id())
                .map(|problem| health_problem_from_doctor(problem.problem.clone()))
                .collect::<Vec<_>>();
            provider.fix(context, &health_problems)
        })
        .map(doctor_fix_result_from_lifecycle)
        .collect()
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
    }
}

fn health_problem_category(category: ProblemCategory) -> HealthCategory {
    match category {
        ProblemCategory::GlobalState => HealthCategory::GlobalState,
        ProblemCategory::RepositoryTargeting => HealthCategory::RepositoryTargeting,
        ProblemCategory::HookRollout => HealthCategory::HookRollout,
        ProblemCategory::RepoAssets => HealthCategory::RepoAssets,
        ProblemCategory::FilesystemPermissions => HealthCategory::FilesystemPermissions,
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
    }
}

fn health_problem_fixability(fixability: ProblemFixability) -> HealthFixability {
    match fixability {
        ProblemFixability::AutoFixable => HealthFixability::AutoFixable,
        ProblemFixability::ManualOnly => HealthFixability::ManualOnly,
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
        HealthProblemKind::OpenCodeIntegrationFilesMissing => {
            ProblemKind::OpenCodeIntegrationFilesMissing
        }
        HealthProblemKind::OpenCodeIntegrationContentMismatch => {
            ProblemKind::OpenCodeIntegrationContentMismatch
        }
        HealthProblemKind::OpenCodePluginRegistryInvalid => {
            ProblemKind::OpenCodePluginRegistryInvalid
        }
        HealthProblemKind::OpenCodeAssetMissingOrInvalid => {
            ProblemKind::OpenCodeAssetMissingOrInvalid
        }
        HealthProblemKind::HookReadFailed => ProblemKind::HookReadFailed,
        HealthProblemKind::OpenCodeAssetReadFailed => ProblemKind::OpenCodeAssetReadFailed,
        HealthProblemKind::AgentTraceDbConnectionFailed => {
            ProblemKind::AgentTraceDbConnectionFailed
        }
        HealthProblemKind::AgentTraceDbSchemaNotReady => ProblemKind::AgentTraceDbSchemaNotReady,
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
        ProblemKind::OpenCodeIntegrationFilesMissing => {
            HealthProblemKind::OpenCodeIntegrationFilesMissing
        }
        ProblemKind::OpenCodeIntegrationContentMismatch => {
            HealthProblemKind::OpenCodeIntegrationContentMismatch
        }
        ProblemKind::OpenCodePluginRegistryInvalid => {
            HealthProblemKind::OpenCodePluginRegistryInvalid
        }
        ProblemKind::OpenCodeAssetMissingOrInvalid => {
            HealthProblemKind::OpenCodeAssetMissingOrInvalid
        }
        ProblemKind::HookReadFailed => HealthProblemKind::HookReadFailed,
        ProblemKind::OpenCodeAssetReadFailed => HealthProblemKind::OpenCodeAssetReadFailed,
        ProblemKind::AgentTraceDbConnectionFailed => {
            HealthProblemKind::AgentTraceDbConnectionFailed
        }
        ProblemKind::AgentTraceDbSchemaNotReady => HealthProblemKind::AgentTraceDbSchemaNotReady,
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
