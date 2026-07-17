//! Renderers for `sce trace status --all` (text and JSON).

use anyhow::{Context, Result};
use serde_json::json;

use crate::services::output_format::OutputFormat;
use crate::services::style;
use crate::services::trace::status_all::{DatabaseRow, DatabaseRowStatus, StatusAllReport};
use crate::services::trace::NAME;

const HEADING: &str = "SCE trace status (all)";
const TOTALS_HEADING: &str = "Totals";
const BY_DATABASE_HEADING: &str = "By database";

const COL_ALIAS: &str = "Alias";
const COL_SCOPE: &str = "Scope";
const COL_ID: &str = "ID";
const COL_STATUS: &str = "Status";
const COL_DIFFS: &str = "Diffs";
const COL_MESSAGES: &str = "Messages";
const COL_PARTS: &str = "Parts";
const COL_TRACES: &str = "Traces";
const COL_INTERSECTIONS: &str = "Intersections";
const SKIPPED_PLACEHOLDER: &str = "-";

pub fn render(report: &StatusAllReport, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Text => Ok(render_text(report)),
        OutputFormat::Json => render_json(report),
    }
}

fn render_text(report: &StatusAllReport) -> String {
    let mut lines = vec![style::heading(HEADING)];
    lines.push(format!(
        "Databases: {} discovered, {} ready, {} skipped",
        report.discovery.discovered, report.discovery.ready, report.discovery.skipped
    ));

    lines.push(String::new());
    lines.push(style::heading(TOTALS_HEADING));
    lines.push(format!("Diff traces: {}", report.totals.diff_traces));
    lines.push(format!("Messages: {}", report.totals.messages));
    lines.push(format!("Parts: {}", report.totals.parts));
    lines.push(format!("Agent traces: {}", report.totals.agent_traces));
    lines.push(format!(
        "Post-commit intersections: {}",
        report.totals.post_commit_patch_intersections
    ));
    lines.push(format!(
        "Last activity: {}",
        report
            .totals
            .last_activity
            .map_or_else(|| String::from("never"), |dt| dt.to_rfc3339())
    ));

    if !report.databases.is_empty() {
        lines.push(String::new());
        lines.push(style::heading(BY_DATABASE_HEADING));

        let headers = [
            COL_ALIAS,
            COL_SCOPE,
            COL_ID,
            COL_STATUS,
            COL_DIFFS,
            COL_MESSAGES,
            COL_PARTS,
            COL_TRACES,
            COL_INTERSECTIONS,
        ];
        let rows: Vec<[String; 9]> = report.databases.iter().map(format_row).collect();

        let widths: Vec<usize> = (0..headers.len())
            .map(|col| {
                rows.iter()
                    .map(|row| row[col].len())
                    .max()
                    .unwrap_or(0)
                    .max(headers[col].len())
            })
            .collect();

        lines.push(join_row(&headers.map(str::to_string), &widths));
        for row in &rows {
            lines.push(join_row(row, &widths));
        }
    }

    lines.join("\n")
}

fn join_row<const N: usize>(cells: &[String; N], widths: &[usize]) -> String {
    cells
        .iter()
        .enumerate()
        .map(|(i, cell)| format!("{cell:<width$}", width = widths[i]))
        .collect::<Vec<_>>()
        .join("  ")
        .trim_end()
        .to_string()
}

fn format_row(row: &DatabaseRow) -> [String; 9] {
    let scope = row.kind.label().to_string();
    let id = row.kind.identifier().to_string();

    match &row.status {
        DatabaseRowStatus::Ready { stats } => [
            row.alias.clone(),
            scope,
            id,
            "ready".to_string(),
            stats.diff_traces.to_string(),
            stats.messages.to_string(),
            stats.parts.to_string(),
            stats.agent_traces.to_string(),
            stats.post_commit_patch_intersections.to_string(),
        ],
        DatabaseRowStatus::Skipped { missing_table } => [
            row.alias.clone(),
            scope,
            id,
            format!("skipped: missing '{missing_table}'"),
            SKIPPED_PLACEHOLDER.to_string(),
            SKIPPED_PLACEHOLDER.to_string(),
            SKIPPED_PLACEHOLDER.to_string(),
            SKIPPED_PLACEHOLDER.to_string(),
            SKIPPED_PLACEHOLDER.to_string(),
        ],
    }
}

fn render_json(report: &StatusAllReport) -> Result<String> {
    let databases: Vec<serde_json::Value> = report
        .databases
        .iter()
        .map(|row| match &row.status {
            DatabaseRowStatus::Ready { stats } => json!({
                "alias": row.alias,
                "scope": row.kind.label(),
                "identifier": row.kind.identifier(),
                "path": row.path.display().to_string(),
                "status": "ready",
                "diff_traces": stats.diff_traces,
                "messages": stats.messages,
                "parts": stats.parts,
                "agent_traces": stats.agent_traces,
                "post_commit_patch_intersections": stats.post_commit_patch_intersections,
                "last_activity": stats
                    .last_activity
                    .map_or(serde_json::Value::Null, |dt| json!(dt.to_rfc3339())),
            }),
            DatabaseRowStatus::Skipped { missing_table } => json!({
                "alias": row.alias,
                "scope": row.kind.label(),
                "identifier": row.kind.identifier(),
                "path": row.path.display().to_string(),
                "status": "skipped",
                "skip_reason": format!("missing table: {missing_table}"),
            }),
        })
        .collect();

    let payload = json!({
        "status": "ok",
        "command": NAME,
        "subcommand": "status.all",
        "discovery": {
            "discovered": report.discovery.discovered,
            "ready": report.discovery.ready,
            "skipped": report.discovery.skipped,
        },
        "totals": {
            "diff_traces": report.totals.diff_traces,
            "messages": report.totals.messages,
            "parts": report.totals.parts,
            "agent_traces": report.totals.agent_traces,
            "post_commit_patch_intersections": report.totals.post_commit_patch_intersections,
            "last_activity": report
                .totals
                .last_activity
                .map_or(serde_json::Value::Null, |dt| json!(dt.to_rfc3339())),
        },
        "databases": databases,
    });

    serde_json::to_string_pretty(&payload)
        .context("failed to serialize trace status.all report to JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use crate::services::agent_trace_db::{
        AgentTraceDb, DiffTraceInsert, InsertMessageInsert, MessageRole,
    };
    use crate::services::trace::status_all::aggregate_status_all_in;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sce-trace-render-status-all-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn touch_mtime(path: &std::path::Path, mtime: SystemTime) {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open db file for mtime update");
        file.set_modified(mtime).expect("set mtime");
    }

    fn seed_ready_db(path: &std::path::Path, diffs: u64, msgs: u64) {
        let db = AgentTraceDb::open_at(path).expect("migrated DB should open");
        for i in 0..diffs {
            db.insert_diff_trace(DiffTraceInsert {
                time_ms: 1_000 + i64::try_from(i).expect("idx fits"),
                session_id: "s1",
                patch: "p",
                model_id: Some("m1"),
                tool_name: "claude",
                tool_version: Some("1"),
                payload_type: "patch",
            })
            .expect("diff");
        }
        for i in 0..msgs {
            db.insert_message(InsertMessageInsert {
                session_id: "s1".into(),
                message_id: format!("m{i}"),
                role: MessageRole::User,
                generated_at_unix_ms: 1_000 + i64::try_from(i).expect("idx fits"),
            })
            .expect("msg");
        }
    }

    fn seed_partial_db(path: &std::path::Path) {
        let db = AgentTraceDb::open_for_hooks_without_migrations_at(path)
            .expect("open without migrations");
        db.execute(
            "CREATE TABLE IF NOT EXISTS diff_traces (id INTEGER PRIMARY KEY)",
            (),
        )
        .expect("create diff_traces");
        drop(db);
    }

    #[test]
    fn empty_renders_text_with_zeroed_summary_and_totals() {
        let dir = unique_temp_dir("empty-text");
        let report = aggregate_status_all_in(&dir, true).expect("aggregate");
        let rendered = render_text(&report);
        assert!(rendered.contains("Databases: 0 discovered, 0 ready, 0 skipped"));
        assert!(rendered.contains("Diff traces: 0"));
        assert!(rendered.contains("Last activity: never"));
        assert!(!rendered.contains(BY_DATABASE_HEADING));
    }

    #[test]
    fn empty_renders_json_with_zeroed_shape() {
        let dir = unique_temp_dir("empty-json");
        let report = aggregate_status_all_in(&dir, true).expect("aggregate");
        let payload = render_json(&report).expect("json render");
        let value: serde_json::Value = serde_json::from_str(&payload).expect("valid json");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["command"], "trace");
        assert_eq!(value["subcommand"], "status.all");
        assert_eq!(value["discovery"]["discovered"], 0);
        assert_eq!(value["discovery"]["ready"], 0);
        assert_eq!(value["discovery"]["skipped"], 0);
        assert_eq!(value["totals"]["diff_traces"], 0);
        assert!(value["totals"]["last_activity"].is_null());
        assert_eq!(value["databases"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn mixed_fixture_renders_text_blocks_with_per_database_rows() {
        let dir = unique_temp_dir("mixed-text");
        let ready_newest = dir.join("agent-trace-aaaa.db");
        let ready_older = dir.join("agent-trace-bbbb.db");
        let skipped = dir.join("agent-trace-cccc.db");

        seed_ready_db(&ready_newest, 3, 2);
        seed_ready_db(&ready_older, 1, 1);
        seed_partial_db(&skipped);

        let base = SystemTime::now();
        touch_mtime(&ready_newest, base);
        touch_mtime(&ready_older, base - Duration::from_secs(5));
        touch_mtime(&skipped, base - Duration::from_secs(10));

        let report = aggregate_status_all_in(&dir, true).expect("aggregate");
        let rendered = render_text(&report);

        assert!(rendered.contains("Databases: 3 discovered, 2 ready, 1 skipped"));
        assert!(rendered.contains("Diff traces: 4"));
        assert!(rendered.contains("Messages: 3"));
        assert!(rendered.contains(BY_DATABASE_HEADING));
        assert!(rendered.contains("agent_trace_0"));
        assert!(rendered.contains("agent_trace_1"));
        assert!(rendered.contains("agent_trace_2"));
        assert!(rendered.contains("ready"));
        assert!(rendered.contains("skipped: missing 'post_commit_patch_intersections'"));
    }

    #[test]
    fn mixed_fixture_renders_json_aggregate_and_breakdown() {
        let dir = unique_temp_dir("mixed-json");
        let ready_newest = dir.join("agent-trace-aaaa.db");
        let ready_older = dir.join("agent-trace-bbbb.db");
        let skipped = dir.join("agent-trace-cccc.db");

        seed_ready_db(&ready_newest, 2, 1);
        seed_ready_db(&ready_older, 1, 0);
        seed_partial_db(&skipped);

        let base = SystemTime::now();
        touch_mtime(&ready_newest, base);
        touch_mtime(&ready_older, base - Duration::from_secs(5));
        touch_mtime(&skipped, base - Duration::from_secs(10));

        let report = aggregate_status_all_in(&dir, true).expect("aggregate");
        let payload = render_json(&report).expect("json render");
        let value: serde_json::Value = serde_json::from_str(&payload).expect("valid json");

        assert_eq!(value["discovery"]["discovered"], 3);
        assert_eq!(value["discovery"]["ready"], 2);
        assert_eq!(value["discovery"]["skipped"], 1);
        assert_eq!(value["totals"]["diff_traces"], 3);
        assert_eq!(value["totals"]["messages"], 1);

        let databases = value["databases"].as_array().expect("databases array");
        assert_eq!(databases.len(), 3);
        assert_eq!(databases[0]["alias"], "agent_trace_0");
        assert_eq!(databases[0]["status"], "ready");
        assert_eq!(databases[0]["diff_traces"], 2);
        assert_eq!(databases[1]["alias"], "agent_trace_1");
        assert_eq!(databases[1]["status"], "ready");
        assert_eq!(databases[1]["diff_traces"], 1);
        assert_eq!(databases[2]["alias"], "agent_trace_2");
        assert_eq!(databases[2]["status"], "skipped");
        assert_eq!(
            databases[2]["skip_reason"],
            "missing table: post_commit_patch_intersections"
        );
    }
}
