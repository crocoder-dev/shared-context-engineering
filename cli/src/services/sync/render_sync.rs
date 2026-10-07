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
