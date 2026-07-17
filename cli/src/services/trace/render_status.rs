//! Renderers for `sce trace status` (text and JSON).

use anyhow::{Context, Result};
use serde_json::json;

use crate::services::output_format::OutputFormat;
use crate::services::style;
use crate::services::trace::status::{DbStatus, StatusReport};
use crate::services::trace::NAME;

const HEADING: &str = "SCE trace status";

pub fn render(report: &StatusReport, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Text => Ok(render_text(report)),
        OutputFormat::Json => render_json(report),
    }
}

fn render_text(report: &StatusReport) -> String {
    let mut lines = vec![style::heading(HEADING)];
    if let Some(repository_id) = &report.repository_id {
        lines.push(format!("Repository ID: {repository_id}"));
    }
    if let Some(source) = &report.repository_identity_source {
        lines.push(format!("Repository identity source: {source}"));
    }
    if let Some(canonical_identity) = &report.canonical_identity {
        lines.push(format!("Canonical identity: {canonical_identity}"));
    }
    if let Some(remote) = &report.configured_remote {
        lines.push(format!("Configured remote: {remote}"));
    }
    lines.push(format!("Checkout ID: {}", report.checkout_id));
    lines.push(format!(
        "Repository-scoped database: {}",
        report.database_path.display()
    ));

    match &report.db_status {
        DbStatus::Ready {
            stats,
            last_activity,
        } => {
            lines.push(String::from("Status: ready"));
            lines.push(format!("Diff traces: {}", stats.diff_traces));
            lines.push(format!("Messages: {}", stats.messages));
            lines.push(format!("Parts: {}", stats.parts));
            lines.push(format!("Agent traces: {}", stats.agent_traces));
            lines.push(format!(
                "Post-commit intersections: {}",
                stats.post_commit_patch_intersections
            ));
            lines.push(format!(
                "Last activity: {}",
                last_activity.map_or_else(|| String::from("never"), |dt| dt.to_rfc3339())
            ));
        }
        DbStatus::Skipped { missing_table } => {
            lines.push(format!("Status: skipped: missing table '{missing_table}'"));
        }
    }

    lines.join("\n")
}

fn render_json(report: &StatusReport) -> Result<String> {
    let mut payload = json!({
        "status": "ok",
        "command": NAME,
        "subcommand": "status",
        "repository_id": report.repository_id,
        "repository_identity_source": report.repository_identity_source,
        "canonical_identity": report.canonical_identity,
        "configured_remote": report.configured_remote,
        "checkout_id": report.checkout_id,
        "database_scope": if report.repository_id.is_some() { "repository" } else { "legacy_checkout" },
        "database_path": report.database_path.display().to_string(),
    });

    let object = payload.as_object_mut().expect("payload is object");
    match &report.db_status {
        DbStatus::Ready {
            stats,
            last_activity,
        } => {
            object.insert("db_status".to_string(), json!("ready"));
            object.insert(
                "stats".to_string(),
                json!({
                    "diff_traces": stats.diff_traces,
                    "messages": stats.messages,
                    "parts": stats.parts,
                    "agent_traces": stats.agent_traces,
                    "post_commit_patch_intersections": stats.post_commit_patch_intersections,
                }),
            );
            object.insert(
                "last_activity".to_string(),
                last_activity.map_or(serde_json::Value::Null, |dt| json!(dt.to_rfc3339())),
            );
        }
        DbStatus::Skipped { missing_table } => {
            object.insert("db_status".to_string(), json!("skipped"));
            object.insert(
                "skip_reason".to_string(),
                json!(format!("missing table: {missing_table}")),
            );
        }
    }

    serde_json::to_string_pretty(&payload)
        .context("failed to serialize trace status report to JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use chrono::{DateTime, Utc};

    use crate::services::trace::stats::AgentTraceDbStats;

    fn ready_report() -> StatusReport {
        let last =
            DateTime::<Utc>::from_timestamp_millis(1_782_650_096_789).expect("timestamp parses");
        StatusReport {
            repository_id: Some(String::from("repo123")),
            repository_identity_source: Some(String::from("remote_url")),
            canonical_identity: Some(String::from("github.com/acme/widgets")),
            configured_remote: Some(String::from("origin")),
            checkout_id: String::from("01900000-0000-7000-8000-000000000abc"),
            database_path: PathBuf::from("/tmp/sce/repos/repo123/agent-trace.db"),
            db_status: DbStatus::Ready {
                stats: AgentTraceDbStats {
                    diff_traces: 7,
                    messages: 4,
                    parts: 11,
                    agent_traces: 3,
                    post_commit_patch_intersections: 1,
                    last_activity: Some(last),
                },
                last_activity: Some(last),
            },
        }
    }

    fn skipped_report() -> StatusReport {
        StatusReport {
            repository_id: None,
            repository_identity_source: None,
            canonical_identity: None,
            configured_remote: None,
            checkout_id: String::from("01900000-0000-7000-8000-000000000def"),
            database_path: PathBuf::from("/tmp/agent-trace-def.db"),
            db_status: DbStatus::Skipped {
                missing_table: String::from("agent_traces"),
            },
        }
    }

    #[test]
    fn ready_text_renders_all_counts_and_last_activity() {
        let rendered = render_text(&ready_report());
        assert!(rendered.contains("SCE trace status"));
        assert!(rendered.contains("Repository ID: repo123"));
        assert!(rendered.contains("Repository identity source: remote_url"));
        assert!(rendered.contains("Canonical identity: github.com/acme/widgets"));
        assert!(rendered.contains("Configured remote: origin"));
        assert!(rendered.contains("Checkout ID: 01900000-0000-7000-8000-000000000abc"));
        assert!(
            rendered.contains("Repository-scoped database: /tmp/sce/repos/repo123/agent-trace.db")
        );
        assert!(rendered.contains("Status: ready"));
        assert!(rendered.contains("Diff traces: 7"));
        assert!(rendered.contains("Messages: 4"));
        assert!(rendered.contains("Parts: 11"));
        assert!(rendered.contains("Agent traces: 3"));
        assert!(rendered.contains("Post-commit intersections: 1"));
        assert!(rendered.contains("Last activity: 2026-06-28T"));
    }

    #[test]
    fn ready_text_renders_never_when_last_activity_absent() {
        let mut report = ready_report();
        if let DbStatus::Ready {
            ref mut stats,
            ref mut last_activity,
        } = report.db_status
        {
            stats.last_activity = None;
            *last_activity = None;
        }
        let rendered = render_text(&report);
        assert!(rendered.contains("Last activity: never"));
    }

    #[test]
    fn skipped_text_renders_skip_reason() {
        let rendered = render_text(&skipped_report());
        assert!(rendered.contains("Status: skipped: missing table 'agent_traces'"));
        assert!(!rendered.contains("Diff traces:"));
    }

    #[test]
    fn ready_json_shape_matches_contract() {
        let payload = render_json(&ready_report()).expect("json render");
        let value: serde_json::Value = serde_json::from_str(&payload).expect("valid json");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["command"], "trace");
        assert_eq!(value["subcommand"], "status");
        assert_eq!(value["db_status"], "ready");
        assert_eq!(value["database_scope"], "repository");
        assert_eq!(value["repository_identity_source"], "remote_url");
        assert_eq!(value["canonical_identity"], "github.com/acme/widgets");
        assert_eq!(value["configured_remote"], "origin");
        assert!(value.get("skip_reason").is_none());
        assert_eq!(value["stats"]["diff_traces"], 7);
        assert_eq!(value["stats"]["messages"], 4);
        assert_eq!(value["stats"]["parts"], 11);
        assert_eq!(value["stats"]["agent_traces"], 3);
        assert_eq!(value["stats"]["post_commit_patch_intersections"], 1);
        assert!(value["last_activity"].is_string());
    }

    #[test]
    fn skipped_json_shape_matches_contract() {
        let payload = render_json(&skipped_report()).expect("json render");
        let value: serde_json::Value = serde_json::from_str(&payload).expect("valid json");
        assert_eq!(value["db_status"], "skipped");
        assert_eq!(value["database_scope"], "legacy_checkout");
        assert_eq!(value["skip_reason"], "missing table: agent_traces");
        assert!(value.get("stats").is_none());
        assert!(value.get("last_activity").is_none());
    }
}
