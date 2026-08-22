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

/// Captures a Codex `UserPromptSubmit` event as one `messages` row
/// (`role = "user"`) and one `parts` row (`part_type = "text"`, `text = prompt`)
/// under session `cx_<session_id>`, message `cx:<turn_id>:user`.
pub(super) fn handle(repository_root: &Path, event: &CodexHookEvent) -> Result<String> {
    let db = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for Codex UserPromptSubmit persistence.",
    )?;

    capture_with(&db, event, || current_unix_time_ms().unwrap_or(0))
}

/// Injectable counterpart of `handle` for deterministic testing against an
/// already-open Agent Trace DB.
fn capture_with<T>(
    db: &RepositoryAgentTraceDb,
    event: &CodexHookEvent,
    generate_timestamp_ms: T,
) -> Result<String>
where
    T: FnOnce() -> i64,
{
    let session_id = required_field(event.session_id.as_deref(), "session_id")?;
    let turn_id = required_field(event.turn_id.as_deref(), "turn_id")?;
    let prompt = required_field(event.prompt.as_deref(), "prompt")?;

    let prefixed_session_id = prefixed_conversation_trace_session_id(CODEX_TOOL_NAME, session_id);
    let message_id = format!("cx:{turn_id}:user");
    let generated_at_unix_ms = generate_timestamp_ms();

    db.insert_messages(vec![InsertMessageInsert {
        session_id: prefixed_session_id.clone(),
        message_id: message_id.clone(),
        role: MessageRole::User,
        generated_at_unix_ms,
    }])
    .context("Failed to insert Codex UserPromptSubmit message row.")?;

    db.insert_parts(vec![InsertPartInsert {
        part_type: PartType::Text,
        text: prompt.to_string(),
        session_id: prefixed_session_id,
        message_id,
        generated_at_unix_ms,
    }])
    .context("Failed to insert Codex UserPromptSubmit text part row.")?;

    Ok("codex hooks: UserPromptSubmit captured into messages/parts.".to_string())
}

fn required_field<'a>(value: Option<&'a str>, field_name: &str) -> Result<&'a str> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(anyhow::anyhow!(
            "Invalid Codex UserPromptSubmit payload: field '{field_name}' must be a non-empty string."
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn unique_test_db_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "sce-codex-user-prompt-submit-{label}-{}-{nonce}",
                std::process::id()
            ))
            .join("agent-trace.db")
    }

    fn remove_test_db(db_path: &Path) {
        if let Some(parent) = db_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    fn event(session_id: &str, turn_id: &str, prompt: &str) -> CodexHookEvent {
        CodexHookEvent {
            hook_event_name: "UserPromptSubmit".to_string(),
            session_id: Some(session_id.to_string()),
            turn_id: Some(turn_id.to_string()),
            cwd: None,
            model: None,
            tool_name: None,
            tool_use_id: None,
            tool_input: None,
            tool_response: None,
            prompt: Some(prompt.to_string()),
        }
    }

    fn message_rows(db: &RepositoryAgentTraceDb) -> Vec<(String, String, String)> {
        db.query_map(
            "SELECT session_id, message_id, role FROM messages ORDER BY id ASC",
            (),
            |row| {
                Ok((
                    row.get::<String>(0).map_err(anyhow::Error::from)?,
                    row.get::<String>(1).map_err(anyhow::Error::from)?,
                    row.get::<String>(2).map_err(anyhow::Error::from)?,
                ))
            },
        )
        .expect("messages query should succeed")
    }

    fn part_rows(db: &RepositoryAgentTraceDb) -> Vec<(String, String, String, String)> {
        db.query_map(
            "SELECT session_id, message_id, type, text FROM parts ORDER BY id ASC",
            (),
            |row| {
                Ok((
                    row.get::<String>(0).map_err(anyhow::Error::from)?,
                    row.get::<String>(1).map_err(anyhow::Error::from)?,
                    row.get::<String>(2).map_err(anyhow::Error::from)?,
                    row.get::<String>(3).map_err(anyhow::Error::from)?,
                ))
            },
        )
        .expect("parts query should succeed")
    }

    #[test]
    fn capture_with_produces_one_message_and_one_part_under_the_prefixed_session() {
        let db_path = unique_test_db_path("basic");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        let output = capture_with(&db, &event("session-1", "turn-1", "hello world"), || 1_000)
            .expect("capture should succeed");
        assert!(output.contains("UserPromptSubmit"));

        assert_eq!(
            message_rows(&db),
            vec![(
                "cx_session-1".to_string(),
                "cx:turn-1:user".to_string(),
                "user".to_string()
            )]
        );
        assert_eq!(
            part_rows(&db),
            vec![(
                "cx_session-1".to_string(),
                "cx:turn-1:user".to_string(),
                "text".to_string(),
                "hello world".to_string()
            )]
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn capture_with_keeps_an_already_prefixed_session_id_unchanged() {
        let db_path = unique_test_db_path("prefixed");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        capture_with(&db, &event("cx_session-1", "turn-1", "hi"), || 1_000)
            .expect("capture should succeed");

        assert_eq!(message_rows(&db)[0].0, "cx_session-1");

        remove_test_db(&db_path);
    }

    #[test]
    fn capture_with_does_not_duplicate_the_parent_message_on_reprocess() {
        let db_path = unique_test_db_path("dedupe");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let payload = event("session-1", "turn-1", "hello world");

        capture_with(&db, &payload, || 1_000).expect("first capture should succeed");
        capture_with(&db, &payload, || 2_000).expect("reprocessed capture should succeed");

        assert_eq!(
            message_rows(&db).len(),
            1,
            "reprocessing the same turn's UserPromptSubmit must not duplicate the parent message row"
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn capture_with_rejects_a_missing_prompt() {
        let db_path = unique_test_db_path("missing-prompt");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let mut payload = event("session-1", "turn-1", "hello world");
        payload.prompt = None;

        let error = capture_with(&db, &payload, || 1_000).expect_err("missing prompt should error");
        assert!(error.to_string().contains("'prompt'"));

        remove_test_db(&db_path);
    }

    #[test]
    fn capture_with_rejects_a_missing_turn_id() {
        let db_path = unique_test_db_path("missing-turn-id");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let mut payload = event("session-1", "turn-1", "hello world");
        payload.turn_id = None;

        let error =
            capture_with(&db, &payload, || 1_000).expect_err("missing turn_id should error");
        assert!(error.to_string().contains("'turn_id'"));

        remove_test_db(&db_path);
    }
}
