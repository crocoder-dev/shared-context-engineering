use anyhow::{Context, Result};
use serde_json::json;

use crate::services::style::{heading, label, supports_color, value, OwoColorize};

use super::types::{
    fix_result_outcome, problem_category, problem_fixability, problem_severity,
    DoctorDisplayStatus, HookContentState, HookDoctorReport, HookFileHealth, HookPathSource,
    IntegrationArea, IntegrationContentState, IntegrationGroupHealth, IntegrationTarget,
    ProblemKind, ProblemSeverity, Readiness,
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
    lines.push(format_human_text_row(
        color_enabled,
        2,
        state_root_status(report),
        "State",
    ));
    lines.push(format_human_text_row(
        color_enabled,
        2,
        configuration_status(report),
        "Configuration",
    ));
    lines.push(format_human_text_row(
        color_enabled,
        2,
        repository_identity_status(report),
        "Repository identity",
    ));

    lines.push(format!("\n{}", heading("Repository")));
    lines.push(format_human_text_row(
        color_enabled,
        2,
        repository_root_status(report),
        "Git repository",
    ));
    lines.push(format_human_text_row(
        color_enabled,
        2,
        git_hooks_status(report),
        "Git hooks",
    ));

    lines.push(format!("\n{}", heading("Integrations")));
    if report.integration_targets_absent {
        lines.push(format_human_text_row(
            color_enabled,
            2,
            DoctorDisplayStatus::Fail,
            NO_INTEGRATIONS_MESSAGE,
        ));
    } else {
        for target in integration_targets_for_text(report) {
            lines.push(format!("  {}", integration_target_label(target)));
            for group in groups_for_target(report, target) {
                lines.push(format_human_text_row(
                    color_enabled,
                    4,
                    integration_group_status(&group, report),
                    integration_area_label(group.key.area),
                ));
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
    status_for_problems(report, |kind| {
        matches!(kind, ProblemKind::UnableToResolveStateRoot)
    })
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
                | IntegrationContentState::ReadFailed(_) => DoctorDisplayStatus::Fail,
            })
        });
    let problem_status = if group.key.target == IntegrationTarget::OpenCode
        && group.key.area == IntegrationArea::Plugins
    {
        status_for_problems(report, |kind| {
            matches!(
                kind,
                ProblemKind::OpenCodePluginRegistryInvalid
                    | ProblemKind::OpenCodeAssetMissingOrInvalid
            )
        })
    } else {
        DoctorDisplayStatus::Pass
    };
    child_status.worst(problem_status)
}

fn integration_targets_for_text(report: &HookDoctorReport) -> Vec<IntegrationTarget> {
    [
        IntegrationTarget::ClaudeCode,
        IntegrationTarget::OpenCode,
        IntegrationTarget::Pi,
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
    }
}

fn integration_area_order(target: IntegrationTarget, area: IntegrationArea) -> usize {
    match target {
        IntegrationTarget::ClaudeCode | IntegrationTarget::OpenCode => match area {
            IntegrationArea::Plugins => 0,
            IntegrationArea::Agents => 1,
            IntegrationArea::Commands => 2,
            IntegrationArea::Skills => 3,
            IntegrationArea::Prompts => 4,
            IntegrationArea::Extensions => 5,
        },
        IntegrationTarget::Pi => match area {
            IntegrationArea::Extensions => 0,
            IntegrationArea::Prompts => 1,
            IntegrationArea::Skills => 2,
            IntegrationArea::Plugins => 3,
            IntegrationArea::Agents => 4,
            IntegrationArea::Commands => 5,
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
