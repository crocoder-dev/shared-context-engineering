use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

pub fn extract_claude_transcript_model(
    transcript_path: &Path,
    tool_use_id: &str,
) -> Option<String> {
    extract_claude_transcript_model_from_reader(
        File::open(transcript_path).map(BufReader::new),
        tool_use_id,
    )
}

fn extract_claude_transcript_model_from_reader<R: BufRead>(
    reader: io::Result<R>,
    tool_use_id: &str,
) -> Option<String> {
    let reader = reader.ok()?;

    for line in reader.lines() {
        let line = line.ok()?;
        if line.trim().is_empty() {
            continue;
        }

        let Ok(parsed) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(record) = parsed.as_object() else {
            continue;
        };

        let message = if let Some(message) = record.get("message").and_then(Value::as_object) {
            let is_assistant = record
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|value| value == "assistant")
                || message
                    .get("role")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value == "assistant");
            if !is_assistant {
                continue;
            }
            message
        } else {
            if record.get("role").and_then(Value::as_str) != Some("assistant") {
                continue;
            }
            record
        };

        let Some(content) = message.get("content").and_then(Value::as_array) else {
            continue;
        };
        let has_matching_tool_use = content.iter().any(|block| {
            block.as_object().is_some_and(|block| {
                block
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value == "tool_use")
                    && block
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|value| value == tool_use_id)
            })
        });

        if has_matching_tool_use {
            return message
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_string);
        }
    }

    None
}
