use anyhow::{Context, Result};
use serde_json::json;

use crate::services::style::{heading, label, supports_color, value, OwoColorize};

use super::types::{
    fix_result_outcome, problem_category, problem_fixability, problem_severity, FileLocationHealth,
    HookContentState, HookDoctorReport, HookFileHealth, HookPathSource, HumanTextStatus,
    IntegrationChildHealth, IntegrationContentState, IntegrationGroupHealth, LocalDbParentStatus,
    LocalDbStatus, ProblemKind, ProblemSeverity, Readiness, OPENCODE_AGENTS_LABEL,
    OPENCODE_COMMANDS_LABEL, OPENCODE_PLUGINS_LABEL, OPENCODE_SKILLS_LABEL,
};
use super::{DoctorExecution, DoctorFormat, DoctorMode, DoctorRequest, NAME, REQUIRED_HOOKS};

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

#[allow(clippy::too_many_lines)]
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
    lines.push(format!(
        "{} {}",
        label("SCE doctor"),
        value(match report.mode {
            DoctorMode::Diagnose => "diagnose",
            DoctorMode::Fix => "fix",
        })
    ));

    lines.push(format!("\n{}:", heading("Environment")));
    lines.push(format_human_text_row(
        color_enabled,
        state_root_status(report),
        "State root",
        report.state_root.as_ref().map_or_else(
            || String::from("not detected"),
            |location| location.path.display().to_string(),
        ),
    ));

    lines.push(format!("\n{}:", heading("Configuration")));
    for location in &report.config_locations {
        lines.push(format_human_text_row(
            color_enabled,
            config_location_status(report, location),
            location.label,
            location.path.display().to_string(),
        ));
    }

    lines.push(format!("\n{}:", heading("Local database")));
    lines.push(format_human_text_row(
        color_enabled,
        local_db_status(report),
        "Database",
        report.local_db.db_path.as_ref().map_or_else(
            || String::from("not detected"),
            |path| path.display().to_string(),
        ),
    ));
    lines.push(format_human_text_row(
        color_enabled,
        local_db_parent_status(report),
        "Parent directory",
        report.local_db.parent_path.as_ref().map_or_else(
            || String::from("not detected"),
            |path| path.display().to_string(),
        ),
    ));

    lines.push(format!("\n{}:", heading("Repository")));
    lines.push(format_human_text_row(
        color_enabled,
        repository_root_status(report),
        "Repository",
        report.repository_root.as_ref().map_or_else(
            || String::from("not detected"),
            |path| path.display().to_string(),
        ),
    ));
    lines.push(format_human_text_row(
        color_enabled,
        hooks_directory_status(report),
        "Hooks",
        report.hooks_directory.as_ref().map_or_else(
            || String::from("not detected"),
            |path| path.display().to_string(),
        ),
    ));

    lines.push(format!("\n{}:", heading("Git Hooks")));
    if report.hooks.is_empty() {
        for hook_name in REQUIRED_HOOKS {
            lines.push(format_human_text_row(
                color_enabled,
                HumanTextStatus::Fail,
                hook_name,
                "not inspected",
            ));
        }
    }
    for hook in &report.hooks {
        lines.push(format_human_text_row(
            color_enabled,
            hook_human_text_status(hook),
            hook.name,
            hook.path.display().to_string(),
        ));
    }

    lines.push(format!("\n{}:", heading("Integrations")));
    for group in integration_groups_for_text(report) {
        lines.push(format_human_text_row(
            color_enabled,
            integration_group_status(&group, report.repository_root.is_some()),
            group.label,
            "",
        ));
        for child in &group.children {
            lines.push(format_human_text_child_row(
                color_enabled,
                integration_child_status(child, report.repository_root.is_some()),
                &child.relative_path,
                integration_child_detail(child),
            ));
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
    status: HumanTextStatus,
    name: &str,
    detail: impl AsRef<str>,
) -> String {
    let detail = detail.as_ref();

    if detail.is_empty() {
        format!(
            "  {} {}",
            value(&human_text_status_token(status, color_enabled)),
            value(name),
        )
    } else {
        format!(
            "  {} {} ({})",
            value(&human_text_status_token(status, color_enabled)),
            value(name),
            value(detail)
        )
    }
}

fn format_human_text_child_row(
    color_enabled: bool,
    status: HumanTextStatus,
    name: &str,
    detail: impl AsRef<str>,
) -> String {
    format!(
        "    {} {} ({})",
        value(&human_text_status_token(status, color_enabled)),
        value(name),
        value(detail.as_ref())
    )
}

fn human_text_status_label(status: HumanTextStatus) -> &'static str {
    match status {
        HumanTextStatus::Pass => "PASS",
        HumanTextStatus::Fail => "FAIL",
        HumanTextStatus::Miss => "MISS",
    }
}

fn human_text_status_token(status: HumanTextStatus, color_enabled: bool) -> String {
    let token = format!("[{}]", human_text_status_label(status));

    if !color_enabled {
        return token;
    }

    match status {
        HumanTextStatus::Pass => token.green().bold().to_string(),
        HumanTextStatus::Fail | HumanTextStatus::Miss => token.red().bold().to_string(),
    }
}

fn state_root_status(report: &HookDoctorReport) -> HumanTextStatus {
    if report
        .problems
        .iter()
        .any(|problem| problem.kind == ProblemKind::UnableToResolveStateRoot)
    {
        HumanTextStatus::Fail
    } else {
        HumanTextStatus::Pass
    }
}

fn config_location_status(
    report: &HookDoctorReport,
    location: &FileLocationHealth,
) -> HumanTextStatus {
    if report.problems.iter().any(|problem| {
        problem.summary.starts_with(location.label) && problem.summary.contains("failed validation")
    }) {
        HumanTextStatus::Fail
    } else {
        HumanTextStatus::Pass
    }
}

fn local_db_status(report: &HookDoctorReport) -> HumanTextStatus {
    match report.local_db.db_status {
        LocalDbStatus::Healthy => HumanTextStatus::Pass,
        LocalDbStatus::Unhealthy | LocalDbStatus::Unresolvable => HumanTextStatus::Fail,
    }
}

fn local_db_parent_status(report: &HookDoctorReport) -> HumanTextStatus {
    match report.local_db.parent_status {
        LocalDbParentStatus::Healthy => HumanTextStatus::Pass,
        LocalDbParentStatus::Missing
        | LocalDbParentStatus::NotDirectory
        | LocalDbParentStatus::NotWritable
        | LocalDbParentStatus::Unresolvable => HumanTextStatus::Fail,
    }
}

fn repository_root_status(report: &HookDoctorReport) -> HumanTextStatus {
    let has_blocking_problem = report.problems.iter().any(|p| {
        matches!(
            p.kind,
            ProblemKind::BareRepository | ProblemKind::NotInsideGitRepository
        )
    });
    if has_blocking_problem {
        HumanTextStatus::Fail
    } else if report.repository_root.is_some() {
        HumanTextStatus::Pass
    } else {
        HumanTextStatus::Miss
    }
}

fn hooks_directory_status(report: &HookDoctorReport) -> HumanTextStatus {
    let has_blocking_problem = report.problems.iter().any(|p| {
        matches!(
            p.kind,
            ProblemKind::HooksDirectoryMissing
                | ProblemKind::HooksPathNotDirectory
                | ProblemKind::UnableToResolveGitHooksDirectory
        )
    });
    if has_blocking_problem {
        HumanTextStatus::Fail
    } else if report.hooks_directory.is_some() {
        HumanTextStatus::Pass
    } else {
        HumanTextStatus::Miss
    }
}

fn hook_human_text_status(hook: &HookFileHealth) -> HumanTextStatus {
    if !hook.exists {
        HumanTextStatus::Miss
    } else if matches!(
        hook.content_state,
        HookContentState::Stale | HookContentState::Unknown
    ) || !hook.executable
    {
        HumanTextStatus::Fail
    } else {
        HumanTextStatus::Pass
    }
}

fn integration_groups_for_text(report: &HookDoctorReport) -> Vec<IntegrationGroupHealth> {
    if report.repository_root.is_none() {
        return vec![
            IntegrationGroupHealth {
                label: OPENCODE_PLUGINS_LABEL,
                children: Vec::new(),
            },
            IntegrationGroupHealth {
                label: OPENCODE_AGENTS_LABEL,
                children: Vec::new(),
            },
            IntegrationGroupHealth {
                label: OPENCODE_COMMANDS_LABEL,
                children: Vec::new(),
            },
            IntegrationGroupHealth {
                label: OPENCODE_SKILLS_LABEL,
                children: Vec::new(),
            },
        ];
    }

    report.integration_groups.clone()
}

fn integration_group_status(
    group: &IntegrationGroupHealth,
    repository_available: bool,
) -> HumanTextStatus {
    if !repository_available
        || group
            .children
            .iter()
            .any(|child| !matches!(&child.content_state, IntegrationContentState::Match))
    {
        HumanTextStatus::Fail
    } else {
        HumanTextStatus::Pass
    }
}

fn integration_child_status(
    child: &IntegrationChildHealth,
    repository_available: bool,
) -> HumanTextStatus {
    if repository_available {
        match &child.content_state {
            IntegrationContentState::Match => HumanTextStatus::Pass,
            IntegrationContentState::Missing => HumanTextStatus::Miss,
            IntegrationContentState::Mismatch | IntegrationContentState::ReadFailed(_) => {
                HumanTextStatus::Fail
            }
        }
    } else {
        HumanTextStatus::Fail
    }
}

fn integration_child_detail(child: &IntegrationChildHealth) -> String {
    match &child.content_state {
        IntegrationContentState::Mismatch => {
            format!("{} - content mismatch", child.path.display())
        }
        IntegrationContentState::ReadFailed(_) => {
            format!("{} - read failed", child.path.display())
        }
        IntegrationContentState::Match | IntegrationContentState::Missing => {
            child.path.display().to_string()
        }
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
        "local_db": json!({
            "db_path": report.local_db.db_path.as_ref().map(|path| path.display().to_string()),
            "parent_path": report.local_db.parent_path.as_ref().map(|path| path.display().to_string()),
            "db_status": local_db_status_json(report.local_db.db_status),
            "parent_status": local_db_parent_status_json(report.local_db.parent_status),
        }),
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

fn local_db_status_json(status: LocalDbStatus) -> &'static str {
    match status {
        LocalDbStatus::Healthy => "healthy",
        LocalDbStatus::Unhealthy => "unhealthy",
        LocalDbStatus::Unresolvable => "unresolvable",
    }
}

fn local_db_parent_status_json(status: LocalDbParentStatus) -> &'static str {
    match status {
        LocalDbParentStatus::Healthy => "healthy",
        LocalDbParentStatus::Missing => "missing",
        LocalDbParentStatus::NotDirectory => "not_directory",
        LocalDbParentStatus::NotWritable => "not_writable",
        LocalDbParentStatus::Unresolvable => "unresolvable",
    }
}
