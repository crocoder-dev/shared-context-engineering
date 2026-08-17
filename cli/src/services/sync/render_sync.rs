//! Renderers for `sce sync` (text and JSON).

use anyhow::{Context, Result};
use serde_json::json;

use crate::services::output_format::OutputFormat;
use crate::services::style;
use crate::services::sync::sync::{AgentTraceSyncReport, StreamSyncReport};
use crate::services::sync::NAME;

const HEADING: &str = "Agent Trace sync complete.";

const COL_STREAM: &str = "Stream";
const COL_UPLOADED: &str = "Uploaded";
const COL_FINAL_CURSOR: &str = "Final cursor";

pub fn render(report: &AgentTraceSyncReport, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Text => Ok(render_text(report)),
        OutputFormat::Json => render_json(report),
    }
}

fn render_text(report: &AgentTraceSyncReport) -> String {
    let mut lines = vec![style::heading(HEADING)];
    lines.push(format!("Repository ID: {}", report.repository_id));
    lines.push(format!("Source instance ID: {}", report.source_instance_id));
    lines.push(String::new());

    let headers = [COL_STREAM, COL_UPLOADED, COL_FINAL_CURSOR];
    let rows: [[String; 3]; 4] = [
        format_row("messages", &report.streams.messages),
        format_row("parts", &report.streams.parts),
        format_row("diff_traces", &report.streams.diff_traces),
        format_row("agent_traces", &report.streams.agent_traces),
    ];

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

fn format_row(name: &str, stream: &StreamSyncReport) -> [String; 3] {
    [
        name.to_string(),
        stream.uploaded.to_string(),
        stream.final_cursor.to_string(),
    ]
}

fn render_json(report: &AgentTraceSyncReport) -> Result<String> {
    let payload = json!({
        "status": "ok",
        "command": NAME,
        "repositoryId": report.repository_id,
        "sourceInstanceId": report.source_instance_id,
        "streams": {
            "messages": stream_json(&report.streams.messages),
            "parts": stream_json(&report.streams.parts),
            "diffTraces": stream_json(&report.streams.diff_traces),
            "agentTraces": stream_json(&report.streams.agent_traces),
        },
    });

    serde_json::to_string_pretty(&payload).context("failed to serialize sync report to JSON")
}

fn stream_json(stream: &StreamSyncReport) -> serde_json::Value {
    json!({
        "uploaded": stream.uploaded,
        "initialCursor": stream.initial_cursor,
        "finalCursor": stream.final_cursor,
        "batches": stream.batches,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report() -> AgentTraceSyncReport {
        AgentTraceSyncReport {
            repository_id: "repo-123".to_string(),
            source_instance_id: "source-abc".to_string(),
            streams: crate::services::sync::sync::StreamSyncReports {
                messages: StreamSyncReport {
                    uploaded: 3,
                    initial_cursor: 10,
                    final_cursor: 13,
                    batches: 1,
                },
                parts: StreamSyncReport {
                    uploaded: 5,
                    initial_cursor: 20,
                    final_cursor: 25,
                    batches: 1,
                },
                diff_traces: StreamSyncReport {
                    uploaded: 0,
                    initial_cursor: 7,
                    final_cursor: 7,
                    batches: 0,
                },
                agent_traces: StreamSyncReport {
                    uploaded: 2,
                    initial_cursor: 1,
                    final_cursor: 3,
                    batches: 1,
                },
            },
        }
    }

    #[test]
    fn text_renders_concise_per_stream_layout_without_batches() {
        let rendered = render_text(&sample_report());
        assert!(rendered.contains("Agent Trace sync complete."));
        assert!(rendered.contains("Repository ID: repo-123"));
        assert!(rendered.contains("Source instance ID: source-abc"));
        assert!(rendered.contains("messages"));
        assert!(rendered.contains("parts"));
        assert!(rendered.contains("diff_traces"));
        assert!(rendered.contains("agent_traces"));
        // Concise: no per-batch or per-row detail is printed.
        assert!(!rendered.contains("batch"));
    }

    #[test]
    fn text_row_values_match_uploaded_and_final_cursor() {
        let rendered = render_text(&sample_report());
        let messages_line = rendered
            .lines()
            .find(|line| line.trim_start().starts_with("messages"))
            .expect("messages row present");
        assert!(messages_line.contains('3'));
        assert!(messages_line.contains("13"));
    }

    #[test]
    fn json_shape_matches_contract() {
        let payload = render_json(&sample_report()).expect("json render");
        let value: serde_json::Value = serde_json::from_str(&payload).expect("valid json");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["command"], "sync");
        assert!(value.get("subcommand").is_none());
        assert_eq!(value["repositoryId"], "repo-123");
        assert_eq!(value["sourceInstanceId"], "source-abc");

        let messages = &value["streams"]["messages"];
        assert_eq!(messages["uploaded"], 3);
        assert_eq!(messages["initialCursor"], 10);
        assert_eq!(messages["finalCursor"], 13);
        assert_eq!(messages["batches"], 1);

        let diff_traces = &value["streams"]["diffTraces"];
        assert_eq!(diff_traces["uploaded"], 0);
        assert_eq!(diff_traces["initialCursor"], 7);
        assert_eq!(diff_traces["finalCursor"], 7);
        assert_eq!(diff_traces["batches"], 0);

        let agent_traces = &value["streams"]["agentTraces"];
        assert_eq!(agent_traces["uploaded"], 2);
        assert_eq!(agent_traces["initialCursor"], 1);
        assert_eq!(agent_traces["finalCursor"], 3);
        assert_eq!(agent_traces["batches"], 1);

        assert!(value.get("diff_traces").is_none());
        assert!(value.get("agent_traces").is_none());
    }
}
