//! Renderers for `sce trace db list` (text and JSON).

use std::time::SystemTime;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::json;

use crate::services::output_format::OutputFormat;
use crate::services::style;
use crate::services::trace::discovery::{DiscoveredAgentTraceDb, Readiness};
use crate::services::trace::NAME;

const HEADING: &str = "SCE trace db list";
const EMPTY_MESSAGE: &str = "no agent-trace databases discovered";

const COL_ALIAS: &str = "Alias";
const COL_STATUS: &str = "Status";
const COL_UPDATED_AT: &str = "Updated at";
const COL_PATH: &str = "Path";

pub fn render(databases: &[DiscoveredAgentTraceDb], format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Text => Ok(render_text(databases)),
        OutputFormat::Json => render_json(databases),
    }
}

fn render_text(databases: &[DiscoveredAgentTraceDb]) -> String {
    let mut lines = vec![style::heading(HEADING)];

    if databases.is_empty() {
        lines.push(EMPTY_MESSAGE.to_string());
        return lines.join("\n");
    }

    let rows: Vec<(String, String, String, String)> = databases
        .iter()
        .map(|db| {
            (
                db.alias.clone(),
                status_label(&db.readiness),
                mtime_to_human_readable(db.mtime),
                db.path.display().to_string(),
            )
        })
        .collect();

    let alias_width = column_width(COL_ALIAS, rows.iter().map(|(a, _, _, _)| a.as_str()));
    let status_width = column_width(COL_STATUS, rows.iter().map(|(_, s, _, _)| s.as_str()));
    let updated_at_width = column_width(COL_UPDATED_AT, rows.iter().map(|(_, _, u, _)| u.as_str()));

    lines.push(format!(
        "{COL_ALIAS:<alias_width$}  {COL_STATUS:<status_width$}  {COL_UPDATED_AT:<updated_at_width$}  {COL_PATH}"
    ));

    for (alias, status, updated_at, path) in &rows {
        lines.push(format!(
            "{alias:<alias_width$}  {status:<status_width$}  {updated_at:<updated_at_width$}  {path}"
        ));
    }

    lines.join("\n")
}

fn render_json(databases: &[DiscoveredAgentTraceDb]) -> Result<String> {
    let entries: Vec<serde_json::Value> = databases
        .iter()
        .map(|db| {
            let (status, skip_reason) = match &db.readiness {
                Readiness::Ready => ("ready", None),
                Readiness::Skipped { missing_table } => {
                    ("skipped", Some(format!("missing table: {missing_table}")))
                }
            };
            let mut entry = json!({
                "alias": db.alias,
                "checkout_id": db.checkout_id,
                "path": db.path.display().to_string(),
                "status": status,
                "updated_at": mtime_to_rfc3339(db.mtime),
            });
            if let Some(reason) = skip_reason {
                entry
                    .as_object_mut()
                    .expect("json object")
                    .insert("skip_reason".to_string(), json!(reason));
            }
            entry
        })
        .collect();

    let payload = json!({
        "status": "ok",
        "command": NAME,
        "subcommand": "db.list",
        "databases": entries,
    });

    serde_json::to_string_pretty(&payload)
        .context("failed to serialize trace db list report to JSON")
}

fn status_label(readiness: &Readiness) -> String {
    match readiness {
        Readiness::Ready => "ready".to_string(),
        Readiness::Skipped { missing_table } => {
            format!("skipped: missing table '{missing_table}'")
        }
    }
}

fn mtime_to_rfc3339(mtime: SystemTime) -> String {
    let dt: DateTime<Utc> = mtime.into();
    dt.to_rfc3339()
}

fn mtime_to_human_readable(mtime: SystemTime) -> String {
    let dt: DateTime<Utc> = mtime.into();
    dt.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

fn column_width<'a, I: Iterator<Item = &'a str>>(header: &str, values: I) -> usize {
    values.map(str::len).max().unwrap_or(0).max(header.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    use crate::services::agent_trace_db::AgentTraceDb;
    use crate::services::trace::discovery::discover_agent_trace_dbs_in;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sce-trace-render-list-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn create_full_schema_db(path: &std::path::Path) {
        let db = AgentTraceDb::open_at(path).expect("agent trace DB should open with migrations");
        drop(db);
    }

    fn create_partial_schema_db(path: &std::path::Path) {
        let db = AgentTraceDb::open_for_hooks_without_migrations_at(path)
            .expect("agent trace DB should open without migrations");
        db.execute(
            "CREATE TABLE IF NOT EXISTS diff_traces (id INTEGER PRIMARY KEY)",
            (),
        )
        .expect("create diff_traces");
        // Intentionally missing post_commit_patch_intersections.
        drop(db);
    }

    fn touch_mtime(path: &std::path::Path, mtime: SystemTime) {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open db file for mtime update");
        file.set_modified(mtime).expect("set mtime");
    }

    #[test]
    fn empty_discovery_renders_empty_message_text() {
        let dir = unique_temp_dir("empty-text");
        let rendered = render_text(&[]);
        assert!(rendered.contains(EMPTY_MESSAGE));
        // Avoid unused dir warning.
        let _ = dir;
    }

    #[test]
    fn empty_discovery_renders_empty_databases_json() {
        let payload = render_json(&[]).expect("json render");
        let value: serde_json::Value = serde_json::from_str(&payload).expect("valid json");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["command"], "trace");
        assert_eq!(value["subcommand"], "db.list");
        assert_eq!(value["databases"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn mixed_fixture_renders_text_table_with_ready_and_skipped_rows() {
        let dir = unique_temp_dir("text-table");
        let ready_a = dir.join("agent-trace-aaaa.db");
        let ready_b = dir.join("agent-trace-bbbb.db");
        let skipped = dir.join("agent-trace-cccc.db");

        create_full_schema_db(&ready_a);
        create_full_schema_db(&ready_b);
        create_partial_schema_db(&skipped);

        let base = SystemTime::now();
        touch_mtime(&ready_a, base);
        touch_mtime(&ready_b, base - Duration::from_secs(5));
        touch_mtime(&skipped, base - Duration::from_secs(10));

        let discovered = discover_agent_trace_dbs_in(&dir).expect("discovery");
        let rendered = render_text(&discovered);

        assert!(rendered.contains("Alias"));
        assert!(rendered.contains("Status"));
        assert!(rendered.contains("Path"));
        assert!(rendered.contains("Updated at"));
        assert!(rendered.contains("agent_trace_0"));
        assert!(rendered.contains("agent_trace_1"));
        assert!(rendered.contains("agent_trace_2"));
        assert!(rendered.contains("ready"));
        assert!(rendered.contains("skipped: missing table 'post_commit_patch_intersections'"));
        assert!(rendered.contains(&ready_a.display().to_string()));
        assert!(rendered.contains(&skipped.display().to_string()));
    }

    #[test]
    fn mixed_fixture_renders_json_shape() {
        let dir = unique_temp_dir("json-shape");
        let ready = dir.join("agent-trace-aaaa.db");
        let skipped = dir.join("agent-trace-bbbb.db");

        create_full_schema_db(&ready);
        create_partial_schema_db(&skipped);

        let base = SystemTime::now();
        touch_mtime(&ready, base);
        touch_mtime(&skipped, base - Duration::from_secs(5));

        let discovered = discover_agent_trace_dbs_in(&dir).expect("discovery");
        let payload = render_json(&discovered).expect("json render");
        let value: serde_json::Value = serde_json::from_str(&payload).expect("valid json");

        assert_eq!(value["status"], "ok");
        assert_eq!(value["command"], "trace");
        assert_eq!(value["subcommand"], "db.list");
        let databases = value["databases"].as_array().expect("databases array");
        assert_eq!(databases.len(), 2);

        assert_eq!(databases[0]["alias"], "agent_trace_0");
        assert_eq!(databases[0]["checkout_id"], "aaaa");
        assert_eq!(databases[0]["status"], "ready");
        assert!(databases[0].get("skip_reason").is_none());
        assert_eq!(databases[0]["path"], ready.display().to_string());
        assert!(databases[0]["updated_at"].is_string());

        assert_eq!(databases[1]["alias"], "agent_trace_1");
        assert_eq!(databases[1]["checkout_id"], "bbbb");
        assert_eq!(databases[1]["status"], "skipped");
        assert_eq!(
            databases[1]["skip_reason"],
            "missing table: post_commit_patch_intersections"
        );
    }
}
