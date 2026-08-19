use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

/// Extract the model identity from a Claude JSONL transcript by matching an
/// assistant message whose `tool_use` content block has the given ID.
///
/// Transcript access and parsing are fail-open. Unreadable files, unreadable
/// lines, missing fields, and unmatched tool calls return `None`; malformed
/// unrelated JSONL records are skipped so later valid records can still match.
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

        // Current Claude transcripts wrap the assistant message in `message`.
        // Keep support for the earlier flat assistant-message shape as well.
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
            if !record
                .get("role")
                .and_then(Value::as_str)
                .is_some_and(|value| value == "assistant")
            {
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

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Error, ErrorKind};

    use super::*;

    fn transcript_reader(content: &str) -> io::Result<Cursor<&[u8]>> {
        Ok(Cursor::new(content.as_bytes()))
    }

    #[test]
    fn claude_transcript_reads_real_assistant_envelope_and_skips_malformed_records() {
        let transcript = concat!(
            "{malformed unrelated record}\n",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_use","id":"tool-123"}]}}"#,
            "\n",
            r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-1","content":[{"type":"text","text":"working"},{"type":"tool_use","id":"tool-123","name":"Write"}]}}"#,
            "\n"
        );

        let model =
            extract_claude_transcript_model_from_reader(transcript_reader(transcript), "tool-123");

        assert_eq!(model.as_deref(), Some("claude-opus-4-1"));
    }

    #[test]
    fn claude_transcript_returns_none_when_transcript_cannot_be_read() {
        let unavailable = Err(Error::new(ErrorKind::NotFound, "transcript unavailable"));

        assert_eq!(
            extract_claude_transcript_model_from_reader::<Cursor<&[u8]>>(unavailable, "tool-123"),
            None
        );
    }

    #[test]
    fn claude_transcript_returns_none_for_unmatched_tool_use_or_missing_model() {
        let unmatched = concat!(
            r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-1","content":[{"type":"tool_use","id":"other-tool"}]}}"#,
            "\n"
        );
        let missing_model = concat!(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"tool-123"}]}}"#,
            "\n"
        );

        assert_eq!(
            extract_claude_transcript_model_from_reader(transcript_reader(unmatched), "tool-123"),
            None
        );
        assert_eq!(
            extract_claude_transcript_model_from_reader(
                transcript_reader(missing_model),
                "tool-123"
            ),
            None
        );
    }
}
