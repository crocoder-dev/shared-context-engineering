use std::path::Path;

use anyhow::{Context, Result};

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_db::{
    DiffTraceInsert, InsertMessageInsert, InsertPartInsert, MessageRole, PartType,
    PAYLOAD_TYPE_PATCH,
};

use super::super::super::{
    current_unix_time_ms, normalize_codex_model_id, open_agent_trace_db_for_hook_runtime,
    prefixed_conversation_trace_session_id, prefixed_diff_trace_session_id, CODEX_TOOL_NAME,
};
use super::super::CodexHookEvent;
use super::pre::required_field;

/// Persists a non-empty `PostToolUse(apply_patch)` observed diff as one
/// `diff_traces` row and one assistant `messages`/`parts` (`part_type =
/// "patch"`) pair, both under session `cx_<session_id>`.
pub(super) fn handle(
    repository_root: &Path,
    event: &CodexHookEvent,
    patch: &str,
) -> Result<String> {
    let db = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for Codex apply_patch diff persistence.",
    )?;

    persist_with(&db, event, patch, || current_unix_time_ms().unwrap_or(0))
}

/// Injectable counterpart of `handle` for deterministic testing against an
/// already-open Agent Trace DB.
fn persist_with<T>(
    db: &RepositoryAgentTraceDb,
    event: &CodexHookEvent,
    patch: &str,
    generate_timestamp_ms: T,
) -> Result<String>
where
    T: FnOnce() -> i64,
{
    let session_id = required_field(event.session_id.as_deref(), "session_id")?;
    let turn_id = required_field(event.turn_id.as_deref(), "turn_id")?;
    let tool_use_id = required_field(event.tool_use_id.as_deref(), "tool_use_id")?;

    let diff_trace_session_id = prefixed_diff_trace_session_id(CODEX_TOOL_NAME, session_id);
    let conversation_session_id =
        prefixed_conversation_trace_session_id(CODEX_TOOL_NAME, session_id);
    let model_id = event.model.as_deref().and_then(normalize_codex_model_id);
    let time_ms = generate_timestamp_ms();

    db.insert_diff_trace(DiffTraceInsert {
        time_ms,
        session_id: &diff_trace_session_id,
        patch,
        model_id: model_id.as_deref(),
        tool_name: CODEX_TOOL_NAME,
        tool_version: None,
        payload_type: PAYLOAD_TYPE_PATCH,
    })
    .context("Failed to insert Codex apply_patch diff-trace row.")?;

    let message_id = format!("cx:{turn_id}:{tool_use_id}:patch");

    db.insert_messages(vec![InsertMessageInsert {
        session_id: conversation_session_id.clone(),
        message_id: message_id.clone(),
        role: MessageRole::Assistant,
        generated_at_unix_ms: time_ms,
    }])
    .context("Failed to insert Codex apply_patch patch message row.")?;

    db.insert_parts(vec![InsertPartInsert {
        part_type: PartType::Patch,
        text: patch.to_string(),
        session_id: conversation_session_id,
        message_id,
        generated_at_unix_ms: time_ms,
    }])
    .context("Failed to insert Codex apply_patch patch part row.")?;

    Ok(
        "codex hooks: PostToolUse apply_patch persisted diff evidence and patch conversation."
            .to_string(),
    )
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
                "sce-codex-apply-patch-persist-{label}-{}-{nonce}",
                std::process::id()
            ))
            .join("agent-trace.db")
    }

    fn remove_test_db(db_path: &Path) {
        if let Some(parent) = db_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    fn event(
        session_id: &str,
        turn_id: &str,
        tool_use_id: &str,
        model: Option<&str>,
    ) -> CodexHookEvent {
        CodexHookEvent {
            hook_event_name: "PostToolUse".to_string(),
            session_id: Some(session_id.to_string()),
            turn_id: Some(turn_id.to_string()),
            cwd: None,
            model: model.map(str::to_string),
            tool_name: Some("apply_patch".to_string()),
            tool_use_id: Some(tool_use_id.to_string()),
            tool_input: None,
            tool_response: None,
            prompt: None,
            last_assistant_message: None,
        }
    }

    fn diff_trace_rows(
        db: &RepositoryAgentTraceDb,
    ) -> Vec<(String, String, Option<String>, String, String)> {
        db.query_map(
            "SELECT session_id, patch, model_id, tool_name, payload_type FROM diff_traces ORDER BY id ASC",
            (),
            |row| {
                Ok((
                    row.get::<String>(0).map_err(anyhow::Error::from)?,
                    row.get::<String>(1).map_err(anyhow::Error::from)?,
                    row.get::<Option<String>>(2).map_err(anyhow::Error::from)?,
                    row.get::<String>(3).map_err(anyhow::Error::from)?,
                    row.get::<String>(4).map_err(anyhow::Error::from)?,
                ))
            },
        )
        .expect("diff_traces query should succeed")
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
    fn persist_with_inserts_one_diff_trace_row_with_expected_fields() {
        let db_path = unique_test_db_path("diff-trace");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        persist_with(
            &db,
            &event("session-1", "turn-1", "tool-1", Some("gpt-5.6-codex")),
            "diff text",
            || 1_000,
        )
        .expect("persist should succeed");

        assert_eq!(
            diff_trace_rows(&db),
            vec![(
                "cx_session-1".to_string(),
                "diff text".to_string(),
                Some("openai/gpt-5.6-codex".to_string()),
                "codex".to_string(),
                "patch".to_string(),
            )]
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn persist_with_inserts_one_assistant_patch_message_and_part() {
        let db_path = unique_test_db_path("conversation");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        persist_with(
            &db,
            &event("session-2", "turn-1", "tool-1", None),
            "diff text",
            || 1_000,
        )
        .expect("persist should succeed");

        assert_eq!(
            message_rows(&db),
            vec![(
                "cx_session-2".to_string(),
                "cx:turn-1:tool-1:patch".to_string(),
                "assistant".to_string(),
            )]
        );
        assert_eq!(
            part_rows(&db),
            vec![(
                "cx_session-2".to_string(),
                "cx:turn-1:tool-1:patch".to_string(),
                "patch".to_string(),
                "diff text".to_string(),
            )]
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn persist_with_omits_model_id_when_event_model_is_absent() {
        let db_path = unique_test_db_path("no-model");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        persist_with(
            &db,
            &event("session-3", "turn-1", "tool-1", None),
            "diff text",
            || 1_000,
        )
        .expect("persist should succeed");

        assert_eq!(diff_trace_rows(&db)[0].2, None);

        remove_test_db(&db_path);
    }

    #[test]
    fn persist_with_rejects_a_missing_tool_use_id() {
        let db_path = unique_test_db_path("missing-tool-use-id");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let mut payload = event("session-4", "turn-1", "tool-1", None);
        payload.tool_use_id = None;

        let error = persist_with(&db, &payload, "diff text", || 1_000)
            .expect_err("missing tool_use_id should error");
        assert!(error.to_string().contains("'tool_use_id'"));

        remove_test_db(&db_path);
    }
}
