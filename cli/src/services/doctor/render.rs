use anyhow::{Context, Result};
use serde_json::json;

use crate::services::hooks::mutation_scope_health::MutationScopeHealthStatus;
use crate::services::style::{heading, label, supports_color, value, OwoColorize};

use super::types::{
    fix_result_outcome, mutation_scope_health_status, mutation_scope_target_id, problem_category,
    problem_fixability, problem_severity, DoctorDisplayDetail, DoctorDisplayNode,
    DoctorDisplayNodeKind, DoctorDisplayStatus, HookContentState, HookDoctorReport, HookFileHealth,
    HookPathSource, IntegrationArea, IntegrationChildHealth, IntegrationContentState,
    IntegrationGroupHealth, IntegrationGroupKey, IntegrationTarget, MutationScopeHealthRow,
    PostCommitAutoSyncState, ProblemKind, ProblemSeverity, Readiness,
};
use super::{DoctorExecution, DoctorFormat, DoctorMode, DoctorRequest, NAME};
use crate::services::mutation_trace::runtime::{
    ReconciliationCounts, ReconciliationFix, ReconciliationRecommendation, StateWarning,
};

/// Guidance message rendered in the Integrations section when no integration
/// targets are configured, detected, or both.
const NO_INTEGRATIONS_MESSAGE: &str = "No integrations installed; run 'sce setup'";

pub(super) fn render_report(request: DoctorRequest, execution: &DoctorExecution) -> Result<String> {
    match request.format {
        DoctorFormat::Text => Ok(format_execution(execution)),
        DoctorFormat::Json => render_report_json(execution),
    }
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

    if let Some(recommendation) = &execution.ref_reconciliation {
        lines.push(format!("\n{}:", heading("Snapshot ref reconciliation")));
        lines.push(format!(
            "  [{}] {}",
            value("recommended"),
            value(&recommendation_text(recommendation))
        ));
    }

    lines.join("\n")
}

fn recommendation_text(recommendation: &ReconciliationRecommendation) -> String {
    let mut parts = vec!["Reconciliation is recommended. Run `sce doctor --fix`.".to_string()];
    if recommendation.last_attempt_failed {
        parts.push(format!(
            "The latest reconciliation attempt failed ({} consecutive failure(s)).",
            recommendation.consecutive_failures
        ));
    }
    if let Some(age) = recommendation.last_attempt_age_ms {
        parts.push(format!(
            "Last reconciliation attempt {} ago.",
            format_age(age)
        ));
    }
    if recommendation.state_unusable {
        parts.push("Maintenance state is unreadable or invalid.".to_string());
    }
    if recommendation.invalid_timestamps {
        parts.push("Maintenance state has invalid timestamps.".to_string());
    }
    if let Some(age) = recommendation.last_success_age_ms {
        parts.push(format!("Last success {} ago.", format_age(age)));
    }
    if let Some(age) = recommendation.last_advised_age_ms {
        parts.push(format!("Last reminder {} ago.", format_age(age)));
    }
    parts.join(" ")
}

fn format_age(age_ms: i64) -> String {
    let minutes = age_ms / 60_000;
    if minutes < 60 {
        format!("{minutes}m")
    } else if minutes < 60 * 24 {
        format!("{}h", minutes / 60)
    } else {
        format!("{}d", minutes / (60 * 24))
    }
}

fn format_report(report: &HookDoctorReport) -> String {
    format_report_with_color_policy(report, supports_color())
}

fn format_report_with_color_policy(report: &HookDoctorReport, color_enabled: bool) -> String {
    let blocking_problem_count = report
        .problems
        .iter()
        .filter(|problem| problem.severity == ProblemSeverity::Error)
        .count();
    let warning_problem_count = report
        .problems
        .iter()
        .filter(|problem| problem.severity == ProblemSeverity::Warning)
        .count();
    let mut lines = Vec::new();
    lines.push(match report.mode {
        DoctorMode::Diagnose => heading("SCE doctor"),
        DoctorMode::Fix => heading("SCE doctor fix"),
    });

    lines.push(format!("\n{}", heading("Environment")));
    for node in environment_nodes(report) {
        render_display_node(&mut lines, &node, color_enabled, 2, true);
    }

    lines.push(format!("\n{}", heading("Repository")));
    render_display_node(
        &mut lines,
        &top_level_node(
            "Git repository",
            repository_root_status(report),
            problem_details(report, |kind| {
                matches!(
                    kind,
                    ProblemKind::GitUnavailable
                        | ProblemKind::BareRepository
                        | ProblemKind::NotInsideGitRepository
                )
            }),
        ),
        color_enabled,
        2,
        true,
    );
    render_display_node(
        &mut lines,
        &top_level_node(
            post_commit_auto_sync_label(report.post_commit_auto_sync.state),
            post_commit_auto_sync_status(report.post_commit_auto_sync.state),
            Vec::new(),
        ),
        color_enabled,
        2,
        true,
    );
    render_display_node(
        &mut lines,
        &top_level_node(
            "Git hooks",
            git_hooks_status(report),
            problem_details(report, is_git_hooks_problem),
        ),
        color_enabled,
        2,
        true,
    );

    lines.push(format!("\n{}", heading("Integrations")));
    if report.integration_targets_absent {
        render_display_node(
            &mut lines,
            &top_level_node(
                NO_INTEGRATIONS_MESSAGE,
                DoctorDisplayStatus::Fail,
                problem_details(report, |kind| {
                    matches!(kind, ProblemKind::NoIntegrationsInstalled)
                }),
            ),
            color_enabled,
            2,
            true,
        );
    } else {
        for target in integration_targets_for_text(report) {
            lines.push(format!("  {}", integration_target_label(target)));
            for group in groups_for_target(report, target) {
                let node = integration_group_node(&group, report);
                render_display_node(&mut lines, &node, color_enabled, 4, true);
            }
            if let Some(row) = mutation_scope_health_row_for_target(report, target) {
                let node = mutation_scope_health_node(row);
                render_display_node(&mut lines, &node, color_enabled, 4, true);
            }
        }
    }

    lines.push(format!(
        "\n{}: {} blocking problem(s), {} warning(s)",
        label("Summary"),
        value(&blocking_problem_count.to_string()),
        value(&warning_problem_count.to_string())
    ));

    lines.join("\n")
}

fn post_commit_auto_sync_label(state: PostCommitAutoSyncState) -> &'static str {
    match state {
        PostCommitAutoSyncState::Disabled => {
            "Post-commit Agent Trace auto-sync (disabled by config)"
        }
        PostCommitAutoSyncState::NotApplicable => {
            "Post-commit Agent Trace auto-sync (not applicable)"
        }
        PostCommitAutoSyncState::Ready | PostCommitAutoSyncState::NotReady => {
            "Post-commit Agent Trace auto-sync"
        }
    }
}

fn post_commit_auto_sync_status(state: PostCommitAutoSyncState) -> DoctorDisplayStatus {
    match state {
        PostCommitAutoSyncState::Ready | PostCommitAutoSyncState::Disabled => {
            DoctorDisplayStatus::Pass
        }
        PostCommitAutoSyncState::NotReady => DoctorDisplayStatus::Fail,
        PostCommitAutoSyncState::NotApplicable => DoctorDisplayStatus::Miss,
    }
}

fn format_human_text_row(
    color_enabled: bool,
    indent: usize,
    status: DoctorDisplayStatus,
    name: &str,
) -> String {
    format!(
        "{}{} {}",
        " ".repeat(indent),
        value(&human_text_status_token(status, color_enabled)),
        value(name),
    )
}

fn environment_nodes(report: &HookDoctorReport) -> Vec<DoctorDisplayNode> {
    vec![
        top_level_node(
            "State",
            state_root_status(report),
            problem_details(report, |kind| {
                matches!(kind, ProblemKind::UnableToResolveStateRoot)
            }),
        ),
        top_level_node(
            "Configuration",
            configuration_status(report),
            problem_details(report, |kind| {
                matches!(
                    kind,
                    ProblemKind::GlobalConfigValidationFailed
                        | ProblemKind::UnableToResolveGlobalConfigPath
                        | ProblemKind::LocalConfigValidationFailed
                        | ProblemKind::AgentTraceDbConnectionFailed
                        | ProblemKind::AgentTraceDbSchemaNotReady
                )
            }),
        ),
        top_level_node(
            "Repository identity",
            repository_identity_status(report),
            problem_details(report, |kind| {
                matches!(
                    kind,
                    ProblemKind::UnableToResolveStateRoot
                        | ProblemKind::AgentTraceDbConnectionFailed
                        | ProblemKind::AgentTraceDbSchemaNotReady
                )
            }),
        ),
    ]
}

fn top_level_node(
    label: &str,
    status: DoctorDisplayStatus,
    details: Vec<DoctorDisplayDetail>,
) -> DoctorDisplayNode {
    DoctorDisplayNode::branch_with_status(
        DoctorDisplayNodeKind::Domain,
        label,
        status,
        details,
        Vec::new(),
    )
}

fn problem_details<F>(report: &HookDoctorReport, matches: F) -> Vec<DoctorDisplayDetail>
where
    F: Fn(ProblemKind) -> bool,
{
    report
        .problems
        .iter()
        .filter(|problem| problem.scope.is_none() && matches(problem.kind))
        .map(|problem| DoctorDisplayDetail::Problem {
            summary: problem.summary.clone(),
            remediation: problem.remediation.clone(),
        })
        .collect()
}

fn scoped_problem_details(
    report: &HookDoctorReport,
    scope: IntegrationGroupKey,
) -> Vec<DoctorDisplayDetail> {
    report
        .problems
        .iter()
        .filter(|problem| problem.scope == Some(scope))
        .map(|problem| DoctorDisplayDetail::Problem {
            summary: problem.summary.clone(),
            remediation: problem.remediation.clone(),
        })
        .collect()
}

fn is_git_hooks_problem(kind: ProblemKind) -> bool {
    matches!(
        kind,
        ProblemKind::HooksDirectoryMissing
            | ProblemKind::HooksPathNotDirectory
            | ProblemKind::UnableToResolveGitHooksDirectory
            | ProblemKind::RequiredHookMissing
            | ProblemKind::HookNotExecutable
            | ProblemKind::HookContentStale
            | ProblemKind::HookReadFailed
    )
}

fn human_text_status_label(status: DoctorDisplayStatus) -> &'static str {
    match status {
        DoctorDisplayStatus::Pass => "PASS",
        DoctorDisplayStatus::Warn => "WARN",
        DoctorDisplayStatus::Fail => "FAIL",
        DoctorDisplayStatus::Miss => "MISS",
    }
}

fn human_text_status_token(status: DoctorDisplayStatus, color_enabled: bool) -> String {
    let token = format!("[{}]", human_text_status_label(status));

    if !color_enabled {
        return token;
    }

    match status {
        DoctorDisplayStatus::Pass => token.green().bold().to_string(),
        DoctorDisplayStatus::Warn => token.yellow().bold().to_string(),
        DoctorDisplayStatus::Fail | DoctorDisplayStatus::Miss => token.red().bold().to_string(),
    }
}

fn status_for_problems<F>(report: &HookDoctorReport, matches: F) -> DoctorDisplayStatus
where
    F: Fn(ProblemKind) -> bool,
{
    report
        .problems
        .iter()
        .filter(|problem| matches(problem.kind))
        .fold(DoctorDisplayStatus::Pass, |status, problem| {
            status.worst(match problem.severity {
                ProblemSeverity::Error => DoctorDisplayStatus::Fail,
                ProblemSeverity::Warning => DoctorDisplayStatus::Warn,
            })
        })
}

fn state_root_status(report: &HookDoctorReport) -> DoctorDisplayStatus {
    let status = status_for_problems(report, |kind| {
        matches!(kind, ProblemKind::UnableToResolveStateRoot)
    });
    if report.state_root.is_none() {
        status.worst(DoctorDisplayStatus::Miss)
    } else {
        status
    }
}

fn configuration_status(report: &HookDoctorReport) -> DoctorDisplayStatus {
    status_for_problems(report, |kind| {
        matches!(
            kind,
            ProblemKind::GlobalConfigValidationFailed
                | ProblemKind::UnableToResolveGlobalConfigPath
                | ProblemKind::LocalConfigValidationFailed
                | ProblemKind::UnableToResolveStateRoot
                | ProblemKind::AgentTraceDbConnectionFailed
                | ProblemKind::AgentTraceDbSchemaNotReady
        )
    })
}

fn repository_identity_status(report: &HookDoctorReport) -> DoctorDisplayStatus {
    let status = status_for_problems(report, |kind| {
        matches!(
            kind,
            ProblemKind::UnableToResolveStateRoot
                | ProblemKind::AgentTraceDbConnectionFailed
                | ProblemKind::AgentTraceDbSchemaNotReady
        )
    });
    if report.repository_root.is_none() {
        status.worst(DoctorDisplayStatus::Miss)
    } else {
        status
    }
}

fn repository_root_status(report: &HookDoctorReport) -> DoctorDisplayStatus {
    if report.problems.iter().any(|problem| {
        matches!(
            problem.kind,
            ProblemKind::BareRepository | ProblemKind::NotInsideGitRepository
        )
    }) {
        DoctorDisplayStatus::Fail
    } else if report.repository_root.is_some() {
        DoctorDisplayStatus::Pass
    } else {
        DoctorDisplayStatus::Miss
    }
}

fn git_hooks_status(report: &HookDoctorReport) -> DoctorDisplayStatus {
    if report.problems.iter().any(|problem| {
        matches!(
            problem.kind,
            ProblemKind::HooksDirectoryMissing
                | ProblemKind::HooksPathNotDirectory
                | ProblemKind::UnableToResolveGitHooksDirectory
                | ProblemKind::RequiredHookMissing
                | ProblemKind::HookNotExecutable
                | ProblemKind::HookContentStale
                | ProblemKind::HookReadFailed
        )
    }) {
        return DoctorDisplayStatus::Fail;
    }
    if report
        .hooks
        .iter()
        .any(|hook| !matches!(hook_human_text_status(hook), DoctorDisplayStatus::Pass))
    {
        DoctorDisplayStatus::Fail
    } else if report.hooks_directory.is_some() {
        DoctorDisplayStatus::Pass
    } else {
        DoctorDisplayStatus::Miss
    }
}

fn hook_human_text_status(hook: &HookFileHealth) -> DoctorDisplayStatus {
    if !hook.exists {
        DoctorDisplayStatus::Miss
    } else if matches!(
        hook.content_state,
        HookContentState::Stale | HookContentState::Unknown
    ) || !hook.executable
    {
        DoctorDisplayStatus::Fail
    } else {
        DoctorDisplayStatus::Pass
    }
}

fn integration_group_status(
    group: &IntegrationGroupHealth,
    report: &HookDoctorReport,
) -> DoctorDisplayStatus {
    let child_status = group
        .children
        .iter()
        .fold(DoctorDisplayStatus::Pass, |status, child| {
            status.worst(match child.content_state {
                IntegrationContentState::Match => DoctorDisplayStatus::Pass,
                IntegrationContentState::Missing
                | IntegrationContentState::Mismatch
                | IntegrationContentState::Stale
                | IntegrationContentState::Malformed(_)
                | IntegrationContentState::ReadFailed(_)
                | IntegrationContentState::PolicyBlocked(_) => DoctorDisplayStatus::Fail,
                IntegrationContentState::NotTrusted(_)
                | IntegrationContentState::PolicyUnknown(_) => DoctorDisplayStatus::Warn,
            })
        });
    let problem_status = report
        .problems
        .iter()
        .filter(|problem| problem.scope == Some(group.key))
        .fold(DoctorDisplayStatus::Pass, |status, problem| {
            status.worst(match problem.severity {
                ProblemSeverity::Error => DoctorDisplayStatus::Fail,
                ProblemSeverity::Warning => DoctorDisplayStatus::Warn,
            })
        });
    child_status.worst(problem_status)
}

fn integration_group_node(
    group: &IntegrationGroupHealth,
    report: &HookDoctorReport,
) -> DoctorDisplayNode {
    let children = integration_asset_nodes(group);
    let status = integration_group_status(group, report);
    DoctorDisplayNode::branch_with_status(
        DoctorDisplayNodeKind::Group,
        integration_area_label(group.key.area),
        status,
        scoped_problem_details(report, group.key),
        children,
    )
}

fn integration_asset_nodes(group: &IntegrationGroupHealth) -> Vec<DoctorDisplayNode> {
    let mut nodes = Vec::new();
    for child in &group.children {
        let components = asset_path_components(group.key.area, &child.relative_path);
        let components = if components.is_empty() {
            vec![child.relative_path.clone()]
        } else {
            components
        };
        insert_asset_node(&mut nodes, &components, child);
    }
    nodes
}

fn asset_path_components(area: IntegrationArea, relative_path: &str) -> Vec<String> {
    let mut components = std::path::Path::new(relative_path)
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>();
    // Codex's relative paths keep their own `.agents/`/`.codex/` output-root
    // prefix (unlike OpenCode/Claude/Pi, whose relative paths are already
    // stripped of their single root), so drop that leading root segment
    // before the shared per-area prefix stripping below.
    if components
        .first()
        .is_some_and(|first| first == ".agents" || first == ".codex")
    {
        components.remove(0);
    }
    let expected_prefix = match area {
        IntegrationArea::Plugins => Some("plugins"),
        IntegrationArea::Agents => Some("agents"),
        IntegrationArea::Commands => Some("commands"),
        IntegrationArea::Skills => Some("skills"),
        IntegrationArea::Prompts => Some("prompts"),
        IntegrationArea::Extensions => Some("extensions"),
        IntegrationArea::Hooks => Some("hooks"),
    };
    if expected_prefix.is_some_and(|prefix| components.first().is_some_and(|first| first == prefix))
    {
        components.remove(0);
    }
    components
}

fn insert_asset_node(
    nodes: &mut Vec<DoctorDisplayNode>,
    components: &[String],
    child: &IntegrationChildHealth,
) {
    let label = components[0].clone();
    if components.len() == 1 {
        let mut node = child.display_node();
        node.label = label;
        nodes.push(node);
        return;
    }

    let index = nodes
        .iter()
        .position(|node| node.label == label)
        .unwrap_or_else(|| {
            nodes.push(DoctorDisplayNode::branch_with_status(
                DoctorDisplayNodeKind::Asset,
                label,
                DoctorDisplayStatus::Pass,
                Vec::new(),
                Vec::new(),
            ));
            nodes.len() - 1
        });
    insert_asset_node(&mut nodes[index].children, &components[1..], child);
    nodes[index].status = nodes[index]
        .children
        .iter()
        .fold(DoctorDisplayStatus::Pass, |status, child| {
            status.worst(child.status)
        });
}

fn render_display_node(
    lines: &mut Vec<String>,
    node: &DoctorDisplayNode,
    color_enabled: bool,
    indent: usize,
    expand_unhealthy: bool,
) {
    lines.push(format_human_text_row(
        color_enabled,
        indent,
        node.status,
        &node.label,
    ));
    if !expand_unhealthy || node.status == DoctorDisplayStatus::Pass {
        return;
    }

    for detail in &node.details {
        render_display_detail(lines, detail, indent + 2);
    }
    for child in &node.children {
        render_display_node(lines, child, color_enabled, indent + 2, true);
    }
}

fn render_display_detail(lines: &mut Vec<String>, detail: &DoctorDisplayDetail, indent: usize) {
    let prefix = " ".repeat(indent);
    match detail {
        DoctorDisplayDetail::MissingPath(path) => {
            lines.push(format!("{prefix}Missing: {}", path.display()));
        }
        DoctorDisplayDetail::ContentMismatch { path } => {
            lines.push(format!("{prefix}Path: {}", path.display()));
            lines.push(format!(
                "{prefix}Content mismatch: canonical content differs."
            ));
        }
        DoctorDisplayDetail::ReadFailed { path, error } => {
            lines.push(format!("{prefix}Path: {}", path.display()));
            lines.push(format!("{prefix}Read error: {error}"));
        }
        DoctorDisplayDetail::Stale { path } => {
            lines.push(format!("{prefix}Path: {}", path.display()));
            lines.push(format!(
                "{prefix}Stale: this registration does not match the canonical handler."
            ));
        }
        DoctorDisplayDetail::Malformed { path, error } => {
            lines.push(format!("{prefix}Path: {}", path.display()));
            lines.push(format!("{prefix}Malformed: {error}"));
        }
        DoctorDisplayDetail::NotTrusted { path, reason } => {
            lines.push(format!("{prefix}Path: {}", path.display()));
            lines.push(format!("{prefix}Not yet executable by Codex: {reason}"));
        }
        DoctorDisplayDetail::PolicyBlocked { path, reason } => {
            lines.push(format!("{prefix}Path: {}", path.display()));
            lines.push(format!("{prefix}Blocked by Codex policy: {reason}"));
        }
        DoctorDisplayDetail::PolicyUnknown { path, reason } => {
            lines.push(format!("{prefix}Path: {}", path.display()));
            lines.push(format!(
                "{prefix}Codex hook-discovery policy could not be determined: {reason}"
            ));
        }
        DoctorDisplayDetail::Problem {
            summary,
            remediation,
        } => {
            lines.push(format!("{prefix}Problem: {summary}"));
            lines.push(format!("{prefix}Remediation: {remediation}"));
        }
        DoctorDisplayDetail::MutationScopeHealth {
            reason,
            detail,
            remediation,
        } => {
            lines.push(format!("{prefix}Reason: {reason}"));
            if let Some(detail) = detail {
                lines.push(format!("{prefix}Detail: {detail}"));
            }
            if let Some(remediation) = remediation {
                lines.push(format!("{prefix}Remediation: {remediation}"));
            }
        }
    }
}

fn mutation_scope_health_row_for_target(
    report: &HookDoctorReport,
    target: IntegrationTarget,
) -> Option<&MutationScopeHealthRow> {
    report
        .mutation_scope_health
        .iter()
        .find(|row| row.target == target)
}

fn mutation_scope_health_display_status(status: MutationScopeHealthStatus) -> DoctorDisplayStatus {
    match status {
        MutationScopeHealthStatus::Healthy => DoctorDisplayStatus::Pass,
        MutationScopeHealthStatus::Recovering => DoctorDisplayStatus::Warn,
        MutationScopeHealthStatus::Blocked | MutationScopeHealthStatus::Invalid => {
            DoctorDisplayStatus::Fail
        }
    }
}

fn mutation_scope_health_node(row: &MutationScopeHealthRow) -> DoctorDisplayNode {
    DoctorDisplayNode::branch_with_status(
        DoctorDisplayNodeKind::Asset,
        "Agent tracing",
        mutation_scope_health_display_status(row.status),
        vec![DoctorDisplayDetail::MutationScopeHealth {
            reason: row.reason.clone(),
            detail: row.detail.clone(),
            remediation: row.remediation.clone(),
        }],
        Vec::new(),
    )
}

fn integration_targets_for_text(report: &HookDoctorReport) -> Vec<IntegrationTarget> {
    [
        IntegrationTarget::ClaudeCode,
        IntegrationTarget::OpenCode,
        IntegrationTarget::Pi,
        IntegrationTarget::Codex,
    ]
    .into_iter()
    .filter(|target| {
        report
            .integration_groups
            .iter()
            .any(|group| group.key.target == *target)
    })
    .collect()
}

fn groups_for_target(
    report: &HookDoctorReport,
    target: IntegrationTarget,
) -> Vec<IntegrationGroupHealth> {
    let mut groups = report
        .integration_groups
        .iter()
        .filter(|group| group.key.target == target)
        .cloned()
        .collect::<Vec<_>>();
    groups.sort_by_key(|group| integration_area_order(target, group.key.area));
    groups
}

pub(super) fn integration_target_label(target: IntegrationTarget) -> &'static str {
    match target {
        IntegrationTarget::ClaudeCode => "Claude Code",
        IntegrationTarget::OpenCode => "OpenCode",
        IntegrationTarget::Pi => "Pi",
        IntegrationTarget::Codex => "Codex",
    }
}

fn integration_area_label(area: IntegrationArea) -> &'static str {
    match area {
        IntegrationArea::Plugins => "Plugins",
        IntegrationArea::Agents => "Agents",
        IntegrationArea::Commands => "Commands",
        IntegrationArea::Skills => "Skills",
        IntegrationArea::Prompts => "Prompts",
        IntegrationArea::Extensions => "Extensions",
        IntegrationArea::Hooks => "Hooks",
    }
}

fn integration_area_order(target: IntegrationTarget, area: IntegrationArea) -> usize {
    match target {
        IntegrationTarget::OpenCode => match area {
            IntegrationArea::Plugins => 0,
            IntegrationArea::Agents => 1,
            IntegrationArea::Commands => 2,
            IntegrationArea::Skills => 3,
            IntegrationArea::Prompts => 4,
            IntegrationArea::Extensions => 5,
            IntegrationArea::Hooks => 6,
        },
        IntegrationTarget::ClaudeCode => match area {
            IntegrationArea::Plugins => 0,
            IntegrationArea::Commands => 1,
            IntegrationArea::Skills => 2,
            IntegrationArea::Agents => 3,
            IntegrationArea::Prompts => 4,
            IntegrationArea::Extensions => 5,
            IntegrationArea::Hooks => 6,
        },
        IntegrationTarget::Pi => match area {
            IntegrationArea::Extensions => 0,
            IntegrationArea::Prompts => 1,
            IntegrationArea::Skills => 2,
            IntegrationArea::Plugins => 3,
            IntegrationArea::Agents => 4,
            IntegrationArea::Commands => 5,
            IntegrationArea::Hooks => 6,
        },
        IntegrationTarget::Codex => match area {
            IntegrationArea::Skills => 0,
            IntegrationArea::Hooks => 1,
            IntegrationArea::Plugins => 2,
            IntegrationArea::Agents => 3,
            IntegrationArea::Commands => 4,
            IntegrationArea::Prompts => 5,
            IntegrationArea::Extensions => 6,
        },
    }
}

fn mutation_scope_health_json(rows: &[MutationScopeHealthRow]) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|row| {
            json!({
                "target": mutation_scope_target_id(row.target),
                "status": mutation_scope_health_status(row.status),
                "reason": row.reason,
                "detail": row.detail,
            })
        })
        .collect::<Vec<_>>()
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

    let mutation_scope_health = mutation_scope_health_json(&report.mutation_scope_health);

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
        "agent_trace_db": report.agent_trace_db.as_ref().map(|location| json!({
            "label": location.label,
            "scope": "repository",
            "path": location.path.display().to_string(),
            "state": location.state,
            "repository_id": location.repository_id,
            "repository_identity_source": location.identity_source,
            "canonical_identity": location.canonical_identity,
            "configured_remote": location.configured_remote,
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
        "post_commit_auto_sync": {
            "state": post_commit_auto_sync_state(report.post_commit_auto_sync.state),
            "enabled": report.post_commit_auto_sync.enabled,
            "source": report.post_commit_auto_sync.source,
            "config_source": report.post_commit_auto_sync.config_source,
        },
        "config_paths": config_paths,
        "hooks": hooks,
        "mutation_scope_health": mutation_scope_health,
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
        "ref_reconciliation": execution.ref_reconciliation.as_ref().map(ref_reconciliation_json),
        "ref_reconciliation_fix": execution.ref_reconciliation_fix.as_ref().map(ref_reconciliation_fix_json),
    });

    serde_json::to_string_pretty(&payload).context("failed to serialize doctor report to JSON")
}

fn post_commit_auto_sync_state(state: PostCommitAutoSyncState) -> &'static str {
    match state {
        PostCommitAutoSyncState::Ready => "ready",
        PostCommitAutoSyncState::Disabled => "disabled",
        PostCommitAutoSyncState::NotReady => "not_ready",
        PostCommitAutoSyncState::NotApplicable => "not_applicable",
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

fn ref_reconciliation_json(recommendation: &ReconciliationRecommendation) -> serde_json::Value {
    json!({
        "recommended": true,
        "last_attempt_failed": recommendation.last_attempt_failed,
        "consecutive_failures": recommendation.consecutive_failures,
        "state_unusable": recommendation.state_unusable,
        "invalid_timestamps": recommendation.invalid_timestamps,
        "last_success_age_ms": recommendation.last_success_age_ms,
        "last_attempt_age_ms": recommendation.last_attempt_age_ms,
        "last_advised_age_ms": recommendation.last_advised_age_ms,
        "text": recommendation_text(recommendation),
    })
}

fn ref_reconciliation_fix_json(fix: &ReconciliationFix) -> serde_json::Value {
    match fix {
        ReconciliationFix::Completed(counts) => json!({
            "outcome": "completed",
            "counts": reconciliation_counts_json(counts),
            "failure": null,
            "state_warning": null,
        }),
        ReconciliationFix::CompletedStatePersistFailed { counts, warning } => json!({
            "outcome": "completed_state_persist_failed",
            "counts": reconciliation_counts_json(counts),
            "failure": null,
            "state_warning": state_warning_json(warning),
        }),
        ReconciliationFix::Failed {
            kind,
            message,
            state_warning,
        } => json!({
            "outcome": "failed",
            "counts": null,
            "failure": { "kind": kind, "message": message },
            "state_warning": state_warning.as_ref().map(state_warning_json),
        }),
        ReconciliationFix::Busy => json!({
            "outcome": "busy",
            "counts": null,
            "failure": null,
            "state_warning": null,
        }),
    }
}

fn reconciliation_counts_json(counts: &ReconciliationCounts) -> serde_json::Value {
    json!({
        "deleted": counts.deleted,
        "retained": counts.retained,
        "local_required": counts.local_required,
    })
}

fn state_warning_json(warning: &StateWarning) -> serde_json::Value {
    json!({
        "kind": warning.kind(),
        "phase": warning.phase(),
        "message": warning.message(),
        "cause": warning.cause(),
    })
}

#[cfg(test)]
mod reconciliation_render_tests {
    use serde_json::json;

    use super::{recommendation_text, ref_reconciliation_fix_json};
    use crate::services::mutation_trace::runtime::{
        ReconciliationCounts, ReconciliationFix, ReconciliationRecommendation, StateWarning,
    };

    const COUNTS: ReconciliationCounts = ReconciliationCounts {
        deleted: 3,
        retained: 2,
        local_required: 1,
    };

    fn recommendation(last_attempt_age_ms: Option<i64>) -> ReconciliationRecommendation {
        ReconciliationRecommendation {
            invalid_timestamps: false,
            state_unusable: false,
            last_success_age_ms: Some(3_600_000 * 30),
            last_attempt_age_ms,
            last_advised_age_ms: Some(120_000),
            last_attempt_failed: last_attempt_age_ms.is_some(),
            consecutive_failures: 2,
        }
    }

    #[test]
    fn recommendation_shows_last_attempt_age_for_failed_recent_attempt() {
        let text = recommendation_text(&recommendation(Some(12 * 60_000)));
        assert!(text.contains("Last reconciliation attempt 12m ago."));
        assert!(
            text.contains("The latest reconciliation attempt failed (2 consecutive failure(s)).")
        );
        assert!(text.contains("Last success 1d ago."));
        assert!(text.contains("Last reminder 2m ago."));
    }

    #[test]
    fn recommendation_omits_last_attempt_age_when_no_attempt_exists() {
        let text = recommendation_text(&recommendation(None));
        assert!(!text.contains("Last reconciliation attempt"));
        assert!(text.contains("Reconciliation is recommended."));
    }

    #[test]
    fn json_ref_reconciliation_fix_results_match_contract() {
        let counts = json!({ "deleted": 3, "retained": 2, "local_required": 1 });
        let cases = [
            (
                "completed",
                ReconciliationFix::Completed(COUNTS),
                json!({
                    "outcome": "completed",
                    "counts": counts,
                    "failure": null,
                    "state_warning": null,
                }),
            ),
            (
                "failed without warning",
                ReconciliationFix::Failed {
                    kind: "agent_trace_db_missing",
                    message: "db missing".to_string(),
                    state_warning: None,
                },
                json!({
                    "outcome": "failed",
                    "counts": null,
                    "failure": { "kind": "agent_trace_db_missing", "message": "db missing" },
                    "state_warning": null,
                }),
            ),
            (
                "failed with warning",
                ReconciliationFix::Failed {
                    kind: "missing_required_pins",
                    message: "pins".to_string(),
                    state_warning: Some(StateWarning::NotApplied {
                        phase: "write_staging",
                        cause: "full".to_string(),
                    }),
                },
                json!({
                    "outcome": "failed",
                    "counts": null,
                    "failure": { "kind": "missing_required_pins", "message": "pins" },
                    "state_warning": {
                        "kind": "not_applied",
                        "phase": "write_staging",
                        "message": "Maintenance-state update was not applied.",
                        "cause": "full",
                    },
                }),
            ),
            (
                "busy",
                ReconciliationFix::Busy,
                json!({
                    "outcome": "busy",
                    "counts": null,
                    "failure": null,
                    "state_warning": null,
                }),
            ),
        ];
        for (label, fix, expected) in cases {
            assert_eq!(ref_reconciliation_fix_json(&fix), expected, "{label}");
        }
    }

    #[test]
    fn json_completed_state_persist_failed_results_match_contract() {
        let counts = json!({ "deleted": 3, "retained": 2, "local_required": 1 });
        let cases = [
            (
                "completed + not applied",
                ReconciliationFix::CompletedStatePersistFailed {
                    counts: COUNTS,
                    warning: StateWarning::NotApplied {
                        phase: "rename",
                        cause: "denied".to_string(),
                    },
                },
                json!({
                    "outcome": "completed_state_persist_failed",
                    "counts": counts,
                    "failure": null,
                    "state_warning": {
                        "kind": "not_applied",
                        "phase": "rename",
                        "message": "Maintenance-state update was not applied.",
                        "cause": "denied",
                    },
                }),
            ),
            (
                "completed + durability uncertain",
                ReconciliationFix::CompletedStatePersistFailed {
                    counts: COUNTS,
                    warning: StateWarning::DurabilityUncertain {
                        phase: "sync_parent_directory",
                        cause: "eio".to_string(),
                    },
                },
                json!({
                    "outcome": "completed_state_persist_failed",
                    "counts": counts,
                    "failure": null,
                    "state_warning": {
                        "kind": "durability_uncertain",
                        "phase": "sync_parent_directory",
                        "message": "Maintenance-state durability could not be confirmed.",
                        "cause": "eio",
                    },
                }),
            ),
            (
                "completed + previous state unreadable",
                ReconciliationFix::CompletedStatePersistFailed {
                    counts: COUNTS,
                    warning: StateWarning::PreviousStateUnreadable {
                        cause: "eacces".to_string(),
                    },
                },
                json!({
                    "outcome": "completed_state_persist_failed",
                    "counts": counts,
                    "failure": null,
                    "state_warning": {
                        "kind": "previous_state_unreadable",
                        "phase": null,
                        "message":
                            "Maintenance state could not be read; the update was not attempted.",
                        "cause": "eacces",
                    },
                }),
            ),
        ];
        for (label, fix, expected) in cases {
            assert_eq!(ref_reconciliation_fix_json(&fix), expected, "{label}");
        }
    }
}
