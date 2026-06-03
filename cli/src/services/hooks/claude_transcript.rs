use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

/// Extract the model identity from a Claude JSONL transcript by matching an
/// assistant message whose `tool_use` content block has the given `tool_use_id`.
///
/// Returns `None` when:
/// - the transcript file is missing or unreadable
/// - any JSONL line is malformed
/// - no assistant message with a matching `tool_use` content block is found
/// - the matching assistant message has no `model` field or it is not a string
///
/// Extracted from the capture flow in T02 to enrich `PostToolUse` capture artifacts.
pub fn extract_claude_transcript_model(
    transcript_path: &Path,
    tool_use_id: &str,
) -> Option<String> {
    let file = File::open(transcript_path).ok()?;
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = line.ok()?;
        if line.trim().is_empty() {
            continue;
        }

        let parsed: Value = serde_json::from_str(&line).ok()?;
        let obj = parsed.as_object()?;

        // Determine whether this line represents an assistant message.
        // The actual Claude transcript wraps the message inside a "message" envelope:
        //   {"type":"assistant","message":{"role":"assistant","model":"...","content":[...]}}
        // Fall back to top-level "role" for simpler/legacy JSONL formats.
        let msg = if let Some(msg_obj) = obj.get("message").and_then(|m| m.as_object()) {
            let is_assistant = obj
                .get("type")
                .and_then(|t| t.as_str())
                .is_some_and(|t| t == "assistant")
                || msg_obj
                    .get("role")
                    .and_then(|r| r.as_str())
                    .is_some_and(|r| r == "assistant");
            if !is_assistant {
                continue;
            }
            msg_obj
        } else {
            // Flat format: {"role":"assistant","model":"...","content":[...]}
            match obj.get("role").and_then(|r| r.as_str()) {
                Some("assistant") => {}
                _ => continue,
            }
            obj
        };

        // Scan content blocks for a matching tool_use id.
        let Some(content) = msg.get("content").and_then(|c| c.as_array()) else {
            continue;
        };
        let has_match = content.iter().any(|block| {
            block
                .as_object()
                .and_then(|b| b.get("type"))
                .and_then(|t| t.as_str())
                .is_some_and(|t| t == "tool_use")
                && block
                    .as_object()
                    .and_then(|b| b.get("id"))
                    .and_then(|id| id.as_str())
                    .is_some_and(|id| id == tool_use_id)
        });

        if !has_match {
            continue;
        }

        // Return the model field from the matching assistant message.
        return msg.get("model").and_then(|m| m.as_str()).map(String::from);
    }

    None
}
