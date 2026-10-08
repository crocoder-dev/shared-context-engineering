use std::path::Path;

use anyhow::{Context, Result};

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_db::{
    InsertMessageInsert, InsertPartInsert, MessageRole, PartType,
};

use super::super::{
    current_unix_time_ms, open_agent_trace_db_for_hook_runtime,
    prefixed_conversation_trace_session_id, CODEX_TOOL_NAME,
};
use super::{CodexHookEvent, NullableField};

pub(super) async fn handle(repository_root: &Path, event: &CodexHookEvent) -> Result<String> {
    handle_with_clock(repository_root, event, current_unix_time_ms).await
}

async fn handle_with_clock<F>(
    repository_root: &Path,
    event: &CodexHookEvent,
    now: F,
) -> Result<String>
where
    F: FnOnce() -> Result<i64>,
{
    let validated = validate_stop_event(event)?;

    let Some(last_assistant_message) = validated.last_assistant_message else {
        return Ok(String::new());
    };

    let generated_at_unix_ms = now()?;

    let mut db = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for Codex Stop persistence.",
    )
    .await?;

    persist_with(
        &mut db,
        &validated,
        last_assistant_message,
        generated_at_unix_ms,
    )
    .await
}

#[derive(Debug)]
struct ValidatedStop<'a> {
    session_id: &'a str,
    turn_id: &'a str,
    last_assistant_message: Option<&'a str>,
}

fn validate_stop_event(event: &CodexHookEvent) -> Result<ValidatedStop<'_>> {
    let session_id = required_trimmed_field(event.session_id.as_deref(), "session_id")?;
    let turn_id = required_trimmed_field(event.turn_id.as_deref(), "turn_id")?;
    let last_assistant_message = match &event.last_assistant_message {
        NullableField::Missing => {
            return Err(anyhow::anyhow!(
                "Invalid Codex Stop payload: field 'last_assistant_message' must be present."
            ))
        }
        NullableField::Null => None,
        NullableField::Value(text) => Some(text.as_str()),
    };

    Ok(ValidatedStop {
        session_id,
        turn_id,
        last_assistant_message,
    })
}

async fn persist_with(
    db: &mut RepositoryAgentTraceDb,
    validated: &ValidatedStop<'_>,
    last_assistant_message: &str,
    generated_at_unix_ms: i64,
) -> Result<String> {
    let prefixed_session_id =
        prefixed_conversation_trace_session_id(CODEX_TOOL_NAME, validated.session_id);
    let message_id = format!("cx:{}:assistant", validated.turn_id);

    db.insert_conversation_text_event(
        InsertMessageInsert {
            session_id: prefixed_session_id.clone(),
            message_id: message_id.clone(),
            role: MessageRole::Assistant,
            generated_at_unix_ms,
        },
        InsertPartInsert {
            part_type: PartType::Text,
            text: last_assistant_message.to_string(),
            session_id: prefixed_session_id,
            message_id,
            generated_at_unix_ms,
        },
    )
    .await
    .context("Failed to insert Codex Stop message/text-part event.")?;

    Ok(String::new())
}

fn required_trimmed_field<'a>(value: Option<&'a str>, field_name: &str) -> Result<&'a str> {
    match value.map(str::trim) {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(anyhow::anyhow!(
            "Invalid Codex Stop payload: field '{field_name}' must be a non-empty string."
        )),
    }
}
