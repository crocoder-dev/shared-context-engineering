use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::json;

use crate::app::{ContextWithRepoRoot, HasRepoRoot};
use crate::services::default_paths::{
    auth_db_path, resolve_sce_default_locations, resolve_state_data_root,
};
use crate::services::lifecycle::{
    lifecycle_providers, FixOutcome, HealthCategory, HealthFixability, HealthProblem,
    HealthProblemKind, HealthSeverity, LifecycleProvider, LifecycleProviderId,
};
use crate::services::output_format::OutputFormat;
use crate::services::setup;
use crate::services::style::{supports_color, value, OwoColorize};

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

/// A project DB discovered from project-*.db files on disk.
#[derive(Clone, Debug)]
struct DiscoveredCheckout {
    /// Stable `UUIDv7` checkout identity extracted from the filename.
    checkout_id: String,
    /// Absolute path to the per-checkout database file.
    database_path: String,
    /// ISO 8601 timestamp from file mtime.
    last_opened: String,
}

#[derive(Clone, Debug)]
struct ServiceDatabase {
    name: &'static str,
    database_path: String,
    last_opened: String,
}

#[derive(Clone, Debug)]
struct DoctorDbsReport {
    service_databases: Vec<ServiceDatabase>,
    project_databases: Vec<DiscoveredCheckout>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ServiceDatabaseColumnWidths {
    name: usize,
    last_opened: usize,
    database_path: usize,
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
    let mut project_databases = discover_checkouts_from_filesystem()
        .context("failed to discover checkouts from filesystem")?;
    sort_checkouts_by_last_opened_desc(&mut project_databases);

    let report = DoctorDbsReport {
        service_databases: collect_service_databases()?,
        project_databases,
    };

    match format {
        DoctorFormat::Text => Ok(render_doctor_dbs_text(&report)),
        DoctorFormat::Json => render_doctor_dbs_json(&report),
    }
}

fn collect_service_databases() -> Result<Vec<ServiceDatabase>> {
    let auth_database_path = auth_db_path().context("failed to resolve auth DB path")?;

    Ok(vec![service_database_record(
        "Auth DB",
        &auth_database_path,
    )?])
}

fn service_database_record(name: &'static str, database_path: &Path) -> Result<ServiceDatabase> {
    let last_opened = match fs::metadata(database_path) {
        Ok(metadata) if metadata.is_file() => modified_timestamp(&metadata),
        Ok(_) => String::from("unknown"),
        Err(error) if error.kind() == ErrorKind::NotFound => String::from("unknown"),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to read metadata for '{}'", database_path.display())
            });
        }
    };

    Ok(ServiceDatabase {
        name,
        database_path: database_path.display().to_string(),
        last_opened,
    })
}

fn modified_timestamp(metadata: &fs::Metadata) -> String {
    metadata.modified().ok().map_or_else(
        || String::from("unknown"),
        |mtime| {
            let dt: DateTime<Utc> = mtime.into();
            dt.to_rfc3339()
        },
    )
}

/// Scans `<state_root>/sce/` for `project-*.db` files and derives checkout
/// metadata from each discovered file.
fn discover_checkouts_from_filesystem() -> Result<Vec<DiscoveredCheckout>> {
    let state_root = resolve_state_data_root().context("failed to resolve state data root")?;
    let sce_dir = state_root.join("sce");

    if !sce_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut checkouts: Vec<DiscoveredCheckout> = Vec::new();

    for entry in fs::read_dir(&sce_dir)
        .with_context(|| format!("failed to read sce directory '{}'", sce_dir.display()))?
    {
        let entry = entry.with_context(|| {
            format!("failed to read directory entry in '{}'", sce_dir.display())
        })?;

        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        // Match project-{id}.db
        let Some(stripped) = file_name_str.strip_prefix("project-") else {
            continue;
        };
        let Some(checkout_id) = stripped.strip_suffix(".db") else {
            continue;
        };
        if checkout_id.is_empty() {
            continue;
        }

        let metadata = entry
            .metadata()
            .with_context(|| format!("failed to read metadata for '{}'", entry.path().display()))?;

        if !metadata.is_file() {
            continue;
        }

        let last_opened = modified_timestamp(&metadata);

        let database_path = entry
            .path()
            .to_str()
            .map_or_else(|| String::from("unknown"), String::from);

        checkouts.push(DiscoveredCheckout {
            checkout_id: checkout_id.to_string(),
            database_path,
            last_opened,
        });
    }

    Ok(checkouts)
}

fn sort_checkouts_by_last_opened_desc(checkouts: &mut [DiscoveredCheckout]) {
    checkouts.sort_by(|left, right| {
        right
            .last_opened
            .cmp(&left.last_opened)
            .then_with(|| left.checkout_id.cmp(&right.checkout_id))
    });
}

fn render_doctor_dbs_text(report: &DoctorDbsReport) -> String {
    render_doctor_dbs_text_with_color_policy(report, supports_color())
}

fn render_doctor_dbs_text_with_color_policy(
    report: &DoctorDbsReport,
    color_enabled: bool,
) -> String {
    let mut lines = vec![format!(
        "{} {}",
        dbs_label("SCE doctor", color_enabled),
        value("dbs")
    )];

    lines.push(format!("\n{}:", dbs_heading("Service DBs", color_enabled)));
    lines.extend(format_service_database_rows(&report.service_databases));

    if report.project_databases.is_empty() {
        lines.push(format!("\n{}:", dbs_heading("Project DBs", color_enabled)));
        lines.push(format!("  {}", value("no discovered project DBs")));
    } else {
        lines.push(format!("\n{}:", dbs_heading("Project DBs", color_enabled)));
        lines.extend(format_discovered_checkout_rows(&report.project_databases));
    }

    lines.push(format!(
        "\n{}: {} service DB(s), {} discovered project DB(s)",
        dbs_label("Summary", color_enabled),
        value(&report.service_databases.len().to_string()),
        value(&report.project_databases.len().to_string())
    ));

    lines.join("\n")
}

fn dbs_heading(text: &str, color_enabled: bool) -> String {
    if color_enabled {
        text.cyan().bold().to_string()
    } else {
        text.to_string()
    }
}

fn dbs_label(text: &str, color_enabled: bool) -> String {
    if color_enabled {
        text.cyan().to_string()
    } else {
        text.to_string()
    }
}

fn format_service_database_rows(databases: &[ServiceDatabase]) -> Vec<String> {
    let name_header = "Name";
    let last_opened_header = "Last Opened";
    let database_path_header = "Database Path";

    let widths = ServiceDatabaseColumnWidths {
        name: databases
            .iter()
            .map(|database| database.name.len())
            .max()
            .unwrap_or(0)
            .max(name_header.len()),
        last_opened: databases
            .iter()
            .map(|database| database.last_opened.len())
            .max()
            .unwrap_or(0)
            .max(last_opened_header.len()),
        database_path: databases
            .iter()
            .map(|database| database.database_path.len())
            .max()
            .unwrap_or(0)
            .max(database_path_header.len()),
    };

    let mut rows = vec![
        format_service_database_row(
            name_header,
            last_opened_header,
            database_path_header,
            widths,
        ),
        format_service_database_row(
            &"-".repeat(widths.name),
            &"-".repeat(widths.last_opened),
            &"-".repeat(widths.database_path),
            widths,
        ),
    ];

    for database in databases {
        rows.push(format_service_database_row(
            database.name,
            &database.last_opened,
            &database.database_path,
            widths,
        ));
    }

    rows
}

fn format_service_database_row(
    name: &str,
    last_opened: &str,
    database_path: &str,
    widths: ServiceDatabaseColumnWidths,
) -> String {
    let ServiceDatabaseColumnWidths {
        name: name_width,
        last_opened: last_opened_width,
        database_path: database_path_width,
    } = widths;

    format!(
        "  {name:<name_width$}  {last_opened:<last_opened_width$}  {database_path:<database_path_width$}"
    )
}

fn format_discovered_checkout_rows(checkouts: &[DiscoveredCheckout]) -> Vec<String> {
    let checkout_id_header = "Project ID";
    let last_opened_header = "Last Opened";
    let database_path_header = "Project DB Path";

    let checkout_id_width = checkouts
        .iter()
        .map(|checkout| checkout.checkout_id.len())
        .max()
        .unwrap_or(0)
        .max(checkout_id_header.len());
    let last_opened_width = checkouts
        .iter()
        .map(|checkout| checkout.last_opened.len())
        .max()
        .unwrap_or(0)
        .max(last_opened_header.len());
    let database_path_width = checkouts
        .iter()
        .map(|checkout| checkout.database_path.len())
        .max()
        .unwrap_or(0)
        .max(database_path_header.len());

    let mut rows = vec![
        format_dbs_table_row(
            checkout_id_header,
            last_opened_header,
            database_path_header,
            checkout_id_width,
            last_opened_width,
            database_path_width,
        ),
        format_dbs_table_row(
            &"-".repeat(checkout_id_width),
            &"-".repeat(last_opened_width),
            &"-".repeat(database_path_width),
            checkout_id_width,
            last_opened_width,
            database_path_width,
        ),
    ];

    for checkout in checkouts {
        rows.push(format_dbs_table_row(
            &checkout.checkout_id,
            &checkout.last_opened,
            &checkout.database_path,
            checkout_id_width,
            last_opened_width,
            database_path_width,
        ));
    }

    rows
}

fn format_dbs_table_row(
    checkout_id: &str,
    last_opened: &str,
    database_path: &str,
    checkout_id_width: usize,
    last_opened_width: usize,
    database_path_width: usize,
) -> String {
    format!(
        "  {checkout_id:<checkout_id_width$}  {last_opened:<last_opened_width$}  {database_path:<database_path_width$}"
    )
}

fn render_doctor_dbs_json(report: &DoctorDbsReport) -> Result<String> {
    let payload = json!({
        "status": "ok",
        "command": NAME,
        "subcommand": "dbs",
        "databases": report.service_databases.iter().map(|database| json!({
            "name": database.name,
            "database_path": database.database_path,
            "last_opened": database.last_opened,
        })).collect::<Vec<_>>(),
        "checkouts": report.project_databases.iter().map(|checkout| json!({
            "checkout_id": checkout.checkout_id,
            "database_path": checkout.database_path,
            "last_opened": checkout.last_opened,
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
