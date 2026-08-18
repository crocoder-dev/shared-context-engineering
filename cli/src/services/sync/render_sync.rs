//! Renderers for `sce sync` (text and JSON).

use anyhow::{Context, Result};
use serde_json::json;

use crate::services::output_format::OutputFormat;
use crate::services::style;
use crate::services::sync::sync::{AgentTraceSyncReport, StreamSyncReport};
use crate::services::sync::NAME;

const COMPLETE_HEADING: &str = "Agent Trace sync complete.";
const ALREADY_SYNCED_HEADING: &str = "Agent Trace already synced.";

pub fn render(report: &AgentTraceSyncReport, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Text => Ok(render_text(report)),
        OutputFormat::Json => render_json(report),
    }
}

fn render_text(report: &AgentTraceSyncReport) -> String {
    let uploaded = [
        report.streams.messages.uploaded,
        report.streams.parts.uploaded,
        report.streams.diff_traces.uploaded,
        report.streams.agent_traces.uploaded,
    ];
    let heading = if uploaded.iter().all(|count| *count == 0) {
        ALREADY_SYNCED_HEADING
    } else {
        COMPLETE_HEADING
    };

    style::heading(heading)
}

fn render_json(report: &AgentTraceSyncReport) -> Result<String> {
    let payload = json!({
        "status": "ok",
        "command": NAME,
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
    fn json_shape_matches_contract() {
        let payload = render_json(&sample_report()).expect("json render");
        let value: serde_json::Value = serde_json::from_str(&payload).expect("valid json");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["command"], "sync");
        assert!(value.get("subcommand").is_none());
        assert!(value.get("repositoryId").is_none());
        assert!(value.get("sourceInstanceId").is_none());

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
