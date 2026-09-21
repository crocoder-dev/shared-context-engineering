use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::services::structured_patch::{build_claude_post_tool_use_patch, PatchBuildResult};

use super::conversation_trace::{
    conversation_trace_validation_error, required_non_empty_string_field,
    CONVERSATION_TRACE_MESSAGE_PART_UPDATED, CONVERSATION_TRACE_MESSAGE_UPDATED,
};
use super::runtime::current_unix_time_ms;

pub(crate) fn transform_claude_user_prompt_submit(
    payload: &serde_json::Map<String, Value>,
) -> Result<Vec<Value>> {
    transform_claude_user_prompt_submit_with(
        payload,
        || {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let ts = uuid::Timestamp::from_unix(uuid::NoContext, now.as_secs(), now.subsec_nanos());
            uuid::Uuid::new_v7(ts)
        },
        || current_unix_time_ms().unwrap_or(0),
    )
}

pub(crate) fn transform_claude_user_prompt_submit_with<G, T>(
    payload: &serde_json::Map<String, Value>,
    generate_message_id: G,
    generate_timestamp_ms: T,
) -> Result<Vec<Value>>
where
    G: FnOnce() -> uuid::Uuid,
    T: FnOnce() -> i64,
{
    let event_name = required_non_empty_string_field(
        payload,
        "hook_event_name",
        conversation_trace_validation_error,
    )?;

    if event_name != "UserPromptSubmit" {
        let raw_content = serde_json::to_string(payload).unwrap_or_default();
        bail!(conversation_trace_validation_error(&format!(
            "unsupported Claude hook event '{event_name}': only 'UserPromptSubmit' is supported. Raw event: {raw_content}"
        )));
    }

    let session_id = required_non_empty_string_field(
        payload,
        "session_id",
        conversation_trace_validation_error,
    )?;
    let prompt =
        required_non_empty_string_field(payload, "prompt", conversation_trace_validation_error)?;

    let message_id = generate_message_id().to_string();
    let generated_at_unix_ms = generate_timestamp_ms();

    Ok(vec![
        json!({
            "type": CONVERSATION_TRACE_MESSAGE_UPDATED,
            "session_id": session_id,
            "message_id": message_id,
            "role": "user",
            "generated_at_unix_ms": generated_at_unix_ms,
        }),
        json!({
            "type": CONVERSATION_TRACE_MESSAGE_PART_UPDATED,
            "session_id": session_id,
            "message_id": message_id,
            "part_type": "text",
            "text": prompt,
            "generated_at_unix_ms": generated_at_unix_ms,
        }),
    ])
}

pub(crate) fn transform_claude_stop(
    payload: &serde_json::Map<String, Value>,
) -> Result<Vec<Value>> {
    transform_claude_stop_with(
        payload,
        || {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let ts = uuid::Timestamp::from_unix(uuid::NoContext, now.as_secs(), now.subsec_nanos());
            uuid::Uuid::new_v7(ts)
        },
        || current_unix_time_ms().unwrap_or(0),
    )
}

pub(crate) fn transform_claude_stop_with<G, T>(
    payload: &serde_json::Map<String, Value>,
    generate_message_id: G,
    generate_timestamp_ms: T,
) -> Result<Vec<Value>>
where
    G: FnOnce() -> uuid::Uuid,
    T: FnOnce() -> i64,
{
    let event_name = required_non_empty_string_field(
        payload,
        "hook_event_name",
        conversation_trace_validation_error,
    )?;

    if event_name != "Stop" {
        let raw_content = serde_json::to_string(payload).unwrap_or_default();
        bail!(conversation_trace_validation_error(&format!(
            "unsupported Claude hook event '{event_name}': only 'Stop' is supported. Raw event: {raw_content}"
        )));
    }

    let session_id = required_non_empty_string_field(
        payload,
        "session_id",
        conversation_trace_validation_error,
    )?;
    let last_assistant_message = required_non_empty_string_field(
        payload,
        "last_assistant_message",
        conversation_trace_validation_error,
    )?;

    let message_id = generate_message_id().to_string();
    let generated_at_unix_ms = generate_timestamp_ms();

    Ok(vec![
        json!({
            "type": CONVERSATION_TRACE_MESSAGE_UPDATED,
            "session_id": session_id,
            "message_id": message_id,
            "role": "assistant",
            "generated_at_unix_ms": generated_at_unix_ms,
        }),
        json!({
            "type": CONVERSATION_TRACE_MESSAGE_PART_UPDATED,
            "session_id": session_id,
            "message_id": message_id,
            "part_type": "text",
            "text": last_assistant_message,
            "generated_at_unix_ms": generated_at_unix_ms,
        }),
    ])
}
pub(crate) fn transform_claude_post_tool_use(
    payload: &serde_json::Map<String, Value>,
) -> Result<Vec<Value>> {
    transform_claude_post_tool_use_with(
        payload,
        || {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let ts = uuid::Timestamp::from_unix(uuid::NoContext, now.as_secs(), now.subsec_nanos());
            uuid::Uuid::new_v7(ts)
        },
        || current_unix_time_ms().unwrap_or(0),
    )
}

pub(crate) fn transform_claude_post_tool_use_with<G, T>(
    payload: &serde_json::Map<String, Value>,
    generate_message_id: G,
    generate_timestamp_ms: T,
) -> Result<Vec<Value>>
where
    G: FnOnce() -> uuid::Uuid,
    T: FnOnce() -> i64,
{
    let event_name = required_non_empty_string_field(
        payload,
        "hook_event_name",
        conversation_trace_validation_error,
    )?;

    if event_name != "PostToolUse" {
        let raw_content = serde_json::to_string(payload).unwrap_or_default();
        bail!(conversation_trace_validation_error(&format!(
            "unsupported Claude hook event '{event_name}': only 'PostToolUse' is supported. Raw event: {raw_content}"
        )));
    }

    let tool_name = payload
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if tool_name != "Write" && tool_name != "Edit" {
        return Ok(vec![]);
    }

    let session_id = required_non_empty_string_field(
        payload,
        "session_id",
        conversation_trace_validation_error,
    )?;

    let message_id = generate_message_id().to_string();
    let generated_at_unix_ms = generate_timestamp_ms();

    match build_claude_post_tool_use_patch(payload) {
        PatchBuildResult::Built(parsed_patch) => {
            let text = serde_json::to_string(&parsed_patch)?;
            let items = vec![
                json!({
                    "type": CONVERSATION_TRACE_MESSAGE_UPDATED,
                    "session_id": session_id,
                    "message_id": message_id,
                    "role": "assistant",
                    "generated_at_unix_ms": generated_at_unix_ms,
                }),
                json!({
                    "type": CONVERSATION_TRACE_MESSAGE_PART_UPDATED,
                    "session_id": session_id,
                    "message_id": message_id,
                    "part_type": "patch",
                    "text": text,
                    "generated_at_unix_ms": generated_at_unix_ms,
                }),
            ];
            Ok(items)
        }
        PatchBuildResult::Skipped(_) => Ok(vec![]),
    }
}
