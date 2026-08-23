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
    handle_with_clock(repository_root, event, current_unix_time_ms)
}

/// Injectable-clock counterpart of `handle`. Validates `session_id`,
/// `turn_id`, and `prompt` (via [`validate_user_prompt_submit_event`])
/// *before* any side effect — a malformed payload never reaches timestamp
/// acquisition or Agent Trace DB access. Timestamp acquisition is itself
/// fallible and its failure is propagated as `Err` rather than swallowed
/// internally, so the existing outer Codex fail-open boundary
/// (`run_codex_subcommand` → `log_codex_fail_open`) owns logging and the
/// empty-stdout contract for both a malformed payload and a failed clock,
/// exactly as it does for any other handler error.
fn handle_with_clock<F>(repository_root: &Path, event: &CodexHookEvent, now: F) -> Result<String>
where
    F: FnOnce() -> Result<i64>,
{
    let validated = validate_user_prompt_submit_event(event)?;

    let generated_at_unix_ms = now()?;

    let db = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for Codex UserPromptSubmit persistence.",
    )?;

    persist_with(&db, &validated, generated_at_unix_ms)
}

/// A Codex `UserPromptSubmit` event whose `session_id`/`turn_id` are
/// confirmed non-blank and trimmed, and whose `prompt` is confirmed
/// present and non-blank (but left untrimmed — prompt text is not
/// whitespace-normalized).
struct ValidatedUserPromptSubmit<'a> {
    session_id: &'a str,
    turn_id: &'a str,
    prompt: &'a str,
}

/// The single validation layer for `UserPromptSubmit` events: every
/// required-field check (`session_id`, `turn_id`, `prompt`) lives here so
/// no other function re-validates the same fields with subtly different
/// semantics. Runs before any timestamp acquisition or DB access.
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

/// Persists an already-validated `UserPromptSubmit` event against an
/// already-open Agent Trace DB. Performs no validation of its own.
fn persist_with(
    db: &RepositoryAgentTraceDb,
    validated: &ValidatedUserPromptSubmit<'_>,
    generated_at_unix_ms: i64,
) -> Result<String> {
    let prefixed_session_id =
        prefixed_conversation_trace_session_id(CODEX_TOOL_NAME, validated.session_id);
    let message_id = format!("cx:{}:user", validated.turn_id);

    db.insert_messages(vec![InsertMessageInsert {
        session_id: prefixed_session_id.clone(),
        message_id: message_id.clone(),
        role: MessageRole::User,
        generated_at_unix_ms,
    }])
    .context("Failed to insert Codex UserPromptSubmit message row.")?;

    db.insert_parts(vec![InsertPartInsert {
        part_type: PartType::Text,
        text: validated.prompt.to_string(),
        session_id: prefixed_session_id,
        message_id,
        generated_at_unix_ms,
    }])
    .context("Failed to insert Codex UserPromptSubmit text part row.")?;

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

/// Validates an identifier field (`session_id`/`turn_id`) is present and
/// non-blank, returning it trimmed so downstream prefixing/formatting never
/// persists incidental leading/trailing whitespace.
fn required_trimmed_field<'a>(value: Option<&'a str>, field_name: &str) -> Result<&'a str> {
    match value.map(str::trim) {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(anyhow::anyhow!(
            "Invalid Codex UserPromptSubmit payload: field '{field_name}' must be a non-empty string."
        )),
    }
}

/// Test-only convenience wrapper preserving the pre-refactor `capture_with`
/// call shape (`event` + timestamp, against an already-open DB) for tests
/// that build a full `CodexHookEvent`. Routes through the same single
/// validation layer (`validate_user_prompt_submit_event`) as production
/// `handle`.
#[cfg(test)]
fn capture_with(
    db: &RepositoryAgentTraceDb,
    event: &CodexHookEvent,
    generated_at_unix_ms: i64,
) -> Result<String> {
    let validated = validate_user_prompt_submit_event(event)?;
    persist_with(db, &validated, generated_at_unix_ms)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::super::NullableField;
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
            last_assistant_message: NullableField::Missing,
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

        let output = capture_with(&db, &event("session-1", "turn-1", "hello world"), 1_000)
            .expect("capture should succeed");
        assert_eq!(output, "");

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

        capture_with(&db, &event("cx_session-1", "turn-1", "hi"), 1_000)
            .expect("capture should succeed");

        assert_eq!(message_rows(&db)[0].0, "cx_session-1");

        remove_test_db(&db_path);
    }

    #[test]
    fn capture_with_does_not_duplicate_the_parent_message_on_reprocess() {
        let db_path = unique_test_db_path("dedupe");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let payload = event("session-1", "turn-1", "hello world");

        capture_with(&db, &payload, 1_000).expect("first capture should succeed");
        capture_with(&db, &payload, 2_000).expect("reprocessed capture should succeed");

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

        let error = capture_with(&db, &payload, 1_000).expect_err("missing prompt should error");
        assert!(error.to_string().contains("'prompt'"));

        remove_test_db(&db_path);
    }

    #[test]
    fn capture_with_rejects_a_missing_turn_id() {
        let db_path = unique_test_db_path("missing-turn-id");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let mut payload = event("session-1", "turn-1", "hello world");
        payload.turn_id = None;

        let error = capture_with(&db, &payload, 1_000).expect_err("missing turn_id should error");
        assert!(error.to_string().contains("'turn_id'"));

        remove_test_db(&db_path);
    }

    #[test]
    fn capture_with_trims_padded_session_and_turn_ids_before_persisting() {
        let db_path = unique_test_db_path("trimmed-ids");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let mut payload = event("session-1", "turn-1", "hello world");
        payload.session_id = Some(" session-1 ".to_string());
        payload.turn_id = Some(" turn-1 ".to_string());

        capture_with(&db, &payload, 1_000).expect("padded ids should persist trimmed");

        assert_eq!(
            message_rows(&db),
            vec![(
                "cx_session-1".to_string(),
                "cx:turn-1:user".to_string(),
                "user".to_string()
            )]
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn capture_with_rejects_a_whitespace_only_turn_id() {
        let db_path = unique_test_db_path("blank-turn-id");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let mut payload = event("session-1", "turn-1", "hello world");
        payload.turn_id = Some("   ".to_string());

        let error = capture_with(&db, &payload, 1_000).expect_err("blank turn_id should error");
        assert!(error.to_string().contains("'turn_id'"));

        remove_test_db(&db_path);
    }

    #[test]
    fn handle_with_clock_propagates_a_timestamp_failure_as_an_error_with_no_persistence() {
        let payload = event("session-1", "turn-1", "hello world");

        // A nonexistent repository root additionally proves the failed
        // clock is consulted (and propagated) before Agent Trace DB
        // resolution is ever attempted: a subsequent DB-open attempt
        // against this path would fail loudly instead.
        let error = handle_with_clock(Path::new("/nonexistent-repository-root"), &payload, || {
            Err(anyhow::anyhow!("clock failed"))
        })
        .expect_err("a failed clock must propagate as an error for the outer fail-open boundary");
        assert!(error.to_string().contains("clock failed"));
    }

    #[test]
    fn handle_with_clock_rejects_a_missing_session_id_without_calling_the_clock() {
        let mut payload = event("session-1", "turn-1", "hello world");
        payload.session_id = None;

        let error = handle_with_clock(Path::new("/nonexistent-repository-root"), &payload, || {
            panic!("clock must not be called for a malformed UserPromptSubmit payload")
        })
        .expect_err("missing session_id should be rejected before the clock is consulted");
        assert!(error.to_string().contains("'session_id'"));
    }

    #[test]
    fn handle_with_clock_rejects_a_whitespace_only_session_id_without_calling_the_clock() {
        let mut payload = event("session-1", "turn-1", "hello world");
        payload.session_id = Some("   ".to_string());

        let error = handle_with_clock(Path::new("/nonexistent-repository-root"), &payload, || {
            panic!("clock must not be called for a malformed UserPromptSubmit payload")
        })
        .expect_err("whitespace-only session_id should be rejected before the clock is consulted");
        assert!(error.to_string().contains("'session_id'"));
    }

    #[test]
    fn handle_with_clock_rejects_a_missing_turn_id_without_calling_the_clock() {
        let mut payload = event("session-1", "turn-1", "hello world");
        payload.turn_id = None;

        let error = handle_with_clock(Path::new("/nonexistent-repository-root"), &payload, || {
            panic!("clock must not be called for a malformed UserPromptSubmit payload")
        })
        .expect_err("missing turn_id should be rejected before the clock is consulted");
        assert!(error.to_string().contains("'turn_id'"));
    }

    #[test]
    fn handle_with_clock_rejects_a_whitespace_only_turn_id_without_calling_the_clock() {
        let mut payload = event("session-1", "turn-1", "hello world");
        payload.turn_id = Some("   ".to_string());

        let error = handle_with_clock(Path::new("/nonexistent-repository-root"), &payload, || {
            panic!("clock must not be called for a malformed UserPromptSubmit payload")
        })
        .expect_err("whitespace-only turn_id should be rejected before the clock is consulted");
        assert!(error.to_string().contains("'turn_id'"));
    }

    #[test]
    fn handle_with_clock_rejects_a_missing_prompt_without_calling_the_clock() {
        let mut payload = event("session-1", "turn-1", "hello world");
        payload.prompt = None;

        let error = handle_with_clock(Path::new("/nonexistent-repository-root"), &payload, || {
            panic!("clock must not be called for a malformed UserPromptSubmit payload")
        })
        .expect_err("missing prompt should be rejected before the clock is consulted");
        assert!(error.to_string().contains("'prompt'"));
    }

    #[test]
    fn handle_with_clock_rejects_a_whitespace_only_prompt_without_calling_the_clock() {
        let mut payload = event("session-1", "turn-1", "hello world");
        payload.prompt = Some("   ".to_string());

        let error = handle_with_clock(Path::new("/nonexistent-repository-root"), &payload, || {
            panic!("clock must not be called for a malformed UserPromptSubmit payload")
        })
        .expect_err("whitespace-only prompt should be rejected before the clock is consulted");
        assert!(error.to_string().contains("'prompt'"));
    }
}
