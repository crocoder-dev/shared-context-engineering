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
const COL_SCOPE: &str = "Scope";
const COL_ID: &str = "ID";
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

    let rows: Vec<(String, String, String, String, String, String)> = databases
        .iter()
        .map(|db| {
            (
                db.alias.clone(),
                db.kind.label().to_string(),
                db.kind.identifier().to_string(),
                status_label(&db.readiness),
                mtime_to_human_readable(db.mtime),
                db.path.display().to_string(),
            )
        })
        .collect();

    let alias_width = column_width(COL_ALIAS, rows.iter().map(|(a, _, _, _, _, _)| a.as_str()));
    let scope_width = column_width(COL_SCOPE, rows.iter().map(|(_, s, _, _, _, _)| s.as_str()));
    let id_width = column_width(COL_ID, rows.iter().map(|(_, _, id, _, _, _)| id.as_str()));
    let status_width = column_width(COL_STATUS, rows.iter().map(|(_, _, _, s, _, _)| s.as_str()));
    let updated_at_width = column_width(
        COL_UPDATED_AT,
        rows.iter().map(|(_, _, _, _, u, _)| u.as_str()),
    );

    lines.push(format!(
        "{COL_ALIAS:<alias_width$}  {COL_SCOPE:<scope_width$}  {COL_ID:<id_width$}  {COL_STATUS:<status_width$}  {COL_UPDATED_AT:<updated_at_width$}  {COL_PATH}"
    ));

    for (alias, scope, id, status, updated_at, path) in &rows {
        lines.push(format!(
            "{alias:<alias_width$}  {scope:<scope_width$}  {id:<id_width$}  {status:<status_width$}  {updated_at:<updated_at_width$}  {path}"
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
                "scope": db.kind.label(),
                "identifier": db.kind.identifier(),
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

    #[test]
    fn empty_discovery_renders_empty_message_text() {
        let rendered = render_text(&[]);
        assert!(rendered.contains(EMPTY_MESSAGE));
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
}
