use anyhow::{Context, Result};
use serde_json::json;

use crate::services::style::{heading, label, supports_color, value, OwoColorize};

use super::types::{
    fix_result_outcome, problem_category, problem_fixability, problem_severity,
    DoctorDisplayDetail, DoctorDisplayNode, DoctorDisplayNodeKind, DoctorDisplayStatus,
    HookContentState, HookDoctorReport, HookFileHealth, HookPathSource, IntegrationArea,
    IntegrationChildHealth, IntegrationContentState, IntegrationGroupHealth, IntegrationGroupKey,
    IntegrationTarget, PostCommitAutoSyncState, ProblemKind, ProblemSeverity, Readiness,
};
use super::{DoctorExecution, DoctorFormat, DoctorMode, DoctorRequest, NAME};

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

    lines.join("\n")
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
    }
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

fn integration_target_label(target: IntegrationTarget) -> &'static str {
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
        "checkout_identity": report.checkout_identity.as_ref().map(|identity| json!({
            "checkout_id": identity.checkout_id,
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
