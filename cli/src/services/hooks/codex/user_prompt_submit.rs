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
use super::CodexHookEvent;

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
    let validated = validate_user_prompt_submit_event(event)?;

    let generated_at_unix_ms = now()?;

    let mut db = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for Codex UserPromptSubmit persistence.",
    )
    .await?;

    persist_with(&mut db, &validated, generated_at_unix_ms).await
}

struct ValidatedUserPromptSubmit<'a> {
    session_id: &'a str,
    turn_id: &'a str,
    prompt: &'a str,
}

fn validate_user_prompt_submit_event(
    event: &CodexHookEvent,
) -> Result<ValidatedUserPromptSubmit<'_>> {
    let session_id = required_trimmed_field(event.session_id.as_deref(), "session_id")?;
    let turn_id = required_trimmed_field(event.turn_id.as_deref(), "turn_id")?;
    let prompt = required_field(event.prompt.as_deref(), "prompt")?;

    Ok(ValidatedUserPromptSubmit {
        session_id,
        turn_id,
        prompt,
    })
}

async fn persist_with(
    db: &mut RepositoryAgentTraceDb,
    validated: &ValidatedUserPromptSubmit<'_>,
    generated_at_unix_ms: i64,
) -> Result<String> {
    let prefixed_session_id =
        prefixed_conversation_trace_session_id(CODEX_TOOL_NAME, validated.session_id);
    let message_id = format!("cx:{}:user", validated.turn_id);

    db.insert_conversation_text_event(
        InsertMessageInsert {
            session_id: prefixed_session_id.clone(),
            message_id: message_id.clone(),
            role: MessageRole::User,
            generated_at_unix_ms,
        },
        InsertPartInsert {
            part_type: PartType::Text,
            text: validated.prompt.to_string(),
            session_id: prefixed_session_id,
            message_id,
            generated_at_unix_ms,
        },
    )
    .await
    .context("Failed to insert Codex UserPromptSubmit message/text-part event.")?;

    Ok(String::new())
}

fn required_field<'a>(value: Option<&'a str>, field_name: &str) -> Result<&'a str> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(anyhow::anyhow!(
            "Invalid Codex UserPromptSubmit payload: field '{field_name}' must be a non-empty string."
        )),
    }
}

fn required_trimmed_field<'a>(value: Option<&'a str>, field_name: &str) -> Result<&'a str> {
    match value.map(str::trim) {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(anyhow::anyhow!(
            "Invalid Codex UserPromptSubmit payload: field '{field_name}' must be a non-empty string."
        )),
    }
}
