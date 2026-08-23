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

/// Captures a Codex `Stop` event as one `messages` row (`role = "assistant"`)
/// and one `parts` row (`part_type = "text"`, `text = last_assistant_message`)
/// under session `cx_<session_id>`, message `cx:<turn_id>:assistant`.
///
/// Upstream Codex's `Stop` schema requires `session_id`, `turn_id`, and
/// `last_assistant_message` (typed `string | null`) on every Stop payload.
/// This handler validates all three *before* any side effect — timestamp
/// acquisition, Agent Trace DB access, or persistence — via
/// [`validate_stop_event`]. A missing/blank `session_id` or `turn_id`, or a
/// missing `last_assistant_message`, is a malformed payload that errors so
/// the outer Codex dispatcher fail-open boundary (`run_codex_subcommand` →
/// `log_codex_fail_open`) logs it and emits exact empty stdout with no DB
/// access — this is true even for an otherwise-valid explicit `null`: a
/// null Stop with a blank/missing identifier is still malformed and must
/// not reach the null no-op path. Only once identifiers and presence are
/// confirmed valid does an explicit `null` short-circuit as a silent
/// successful no-op *before* timestamp acquisition or the Agent Trace DB is
/// ever opened; a present value (including an explicit empty string,
/// persisted like any other text) is captured normally.
pub(super) fn handle(repository_root: &Path, event: &CodexHookEvent) -> Result<String> {
    handle_with_clock(repository_root, event, current_unix_time_ms)
}

/// Injectable-clock counterpart of `handle`. Timestamp acquisition is
/// fallible and its failure is propagated as `Err` rather than swallowed
/// internally, so the existing outer Codex fail-open boundary owns logging
/// and the empty-stdout contract for a failed clock exactly as it does for
/// any other handler error.
fn handle_with_clock<F>(repository_root: &Path, event: &CodexHookEvent, now: F) -> Result<String>
where
    F: FnOnce() -> Result<i64>,
{
    let validated = validate_stop_event(event)?;

    let Some(last_assistant_message) = validated.last_assistant_message else {
        return Ok(String::new());
    };

    let generated_at_unix_ms = now()?;

    let db = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for Codex Stop persistence.",
    )?;

    persist_with(
        &db,
        &validated,
        last_assistant_message,
        generated_at_unix_ms,
    )
}

/// A Codex `Stop` event whose `session_id`/`turn_id` are confirmed
/// non-blank and trimmed, and whose `last_assistant_message` presence has
/// already been confirmed (a missing field cannot produce a `ValidatedStop`
/// at all). `None` here means an explicit upstream `null` — the valid
/// "no assistant text this turn" no-op signal; `Some` carries a present
/// value (including an explicit empty string).
#[derive(Debug)]
struct ValidatedStop<'a> {
    session_id: &'a str,
    turn_id: &'a str,
    last_assistant_message: Option<&'a str>,
}

/// The single validation layer for `Stop` events: every required-field
/// check (`session_id`, `turn_id`, `last_assistant_message` presence) lives
/// here so no other function re-validates the same fields with subtly
/// different semantics. Runs before any timestamp acquisition or DB access.
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

/// Persists an already-validated `Stop` event with a known-present
/// assistant message against an already-open Agent Trace DB. Performs no
/// validation of its own.
fn persist_with(
    db: &RepositoryAgentTraceDb,
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
    .context("Failed to insert Codex Stop message/text-part event.")?;

    Ok(String::new())
}

/// Validates an identifier field (`session_id`/`turn_id`) is present and
/// non-blank, returning it trimmed so downstream prefixing/formatting never
/// persists incidental leading/trailing whitespace.
fn required_trimmed_field<'a>(value: Option<&'a str>, field_name: &str) -> Result<&'a str> {
    match value.map(str::trim) {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(anyhow::anyhow!(
            "Invalid Codex Stop payload: field '{field_name}' must be a non-empty string."
        )),
    }
}

/// Test-only convenience wrapper preserving the pre-refactor `capture_with`
/// call shape (`event` + timestamp, against an already-open DB) for tests
/// that build a full `CodexHookEvent`. Routes through the same single
/// validation layer (`validate_stop_event`) as production `handle`, so it
/// exercises identical semantics — including the null no-op — rather than
/// re-implementing validation.
#[cfg(test)]
fn capture_with(
    db: &RepositoryAgentTraceDb,
    event: &CodexHookEvent,
    generated_at_unix_ms: i64,
) -> Result<String> {
    let validated = validate_stop_event(event)?;
    match validated.last_assistant_message {
        Some(last_assistant_message) => {
            persist_with(db, &validated, last_assistant_message, generated_at_unix_ms)
        }
        None => Ok(String::new()),
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
                "sce-codex-stop-{label}-{}-{nonce}",
                std::process::id()
            ))
            .join("agent-trace.db")
    }

    fn remove_test_db(db_path: &Path) {
        if let Some(parent) = db_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    fn event(session_id: &str, turn_id: &str, last_assistant_message: &str) -> CodexHookEvent {
        CodexHookEvent {
            hook_event_name: "Stop".to_string(),
            session_id: Some(session_id.to_string()),
            turn_id: Some(turn_id.to_string()),
            cwd: None,
            model: None,
            tool_name: None,
            tool_use_id: None,
            tool_input: None,
            tool_response: None,
            prompt: None,
            last_assistant_message: NullableField::Value(last_assistant_message.to_string()),
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

        let output = capture_with(&db, &event("session-1", "turn-1", "hello back"), 1_000)
            .expect("capture should succeed");
        assert_eq!(output, "");

        assert_eq!(
            message_rows(&db),
            vec![(
                "cx_session-1".to_string(),
                "cx:turn-1:assistant".to_string(),
                "assistant".to_string()
            )]
        );
        assert_eq!(
            part_rows(&db),
            vec![(
                "cx_session-1".to_string(),
                "cx:turn-1:assistant".to_string(),
                "text".to_string(),
                "hello back".to_string()
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
        let payload = event("session-1", "turn-1", "hello back");

        capture_with(&db, &payload, 1_000).expect("first capture should succeed");
        capture_with(&db, &payload, 2_000).expect("reprocessed capture should succeed");

        assert_eq!(
            message_rows(&db).len(),
            1,
            "reprocessing the same turn's Stop must not duplicate the parent message row"
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn capture_with_rejects_a_missing_last_assistant_message() {
        let db_path = unique_test_db_path("missing-last-assistant-message");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let mut payload = event("session-1", "turn-1", "hello back");
        payload.last_assistant_message = NullableField::Missing;

        let error = capture_with(&db, &payload, 1_000)
            .expect_err("missing last_assistant_message should error");
        assert!(error.to_string().contains("'last_assistant_message'"));

        remove_test_db(&db_path);
    }

    #[test]
    fn capture_with_is_a_no_op_for_a_null_last_assistant_message() {
        let db_path = unique_test_db_path("null-last-assistant-message");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let mut payload = event("session-1", "turn-1", "hello back");
        payload.last_assistant_message = NullableField::Null;

        let output = capture_with(&db, &payload, 1_000)
            .expect("null last_assistant_message is a valid no-op, not an error");
        assert_eq!(output, "");
        assert_eq!(message_rows(&db).len(), 0);
        assert_eq!(part_rows(&db).len(), 0);

        remove_test_db(&db_path);
    }

    #[test]
    fn capture_with_rejects_a_missing_turn_id() {
        let db_path = unique_test_db_path("missing-turn-id");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let mut payload = event("session-1", "turn-1", "hello back");
        payload.turn_id = None;

        let error = capture_with(&db, &payload, 1_000).expect_err("missing turn_id should error");
        assert!(error.to_string().contains("'turn_id'"));

        remove_test_db(&db_path);
    }

    #[test]
    fn capture_with_trims_padded_session_and_turn_ids_before_persisting() {
        let db_path = unique_test_db_path("trimmed-ids");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let mut payload = event(" session-1 ", " turn-1 ", "hello back");
        payload.session_id = Some(" session-1 ".to_string());
        payload.turn_id = Some(" turn-1 ".to_string());

        capture_with(&db, &payload, 1_000).expect("padded ids should persist trimmed");

        assert_eq!(
            message_rows(&db),
            vec![(
                "cx_session-1".to_string(),
                "cx:turn-1:assistant".to_string(),
                "assistant".to_string()
            )]
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn capture_with_rejects_a_whitespace_only_session_id() {
        let db_path = unique_test_db_path("blank-session-id");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let mut payload = event("session-1", "turn-1", "hello back");
        payload.session_id = Some("   ".to_string());

        let error = capture_with(&db, &payload, 1_000).expect_err("blank session_id should error");
        assert!(error.to_string().contains("'session_id'"));

        remove_test_db(&db_path);
    }

    #[test]
    fn capture_with_persists_an_explicit_empty_last_assistant_message() {
        let db_path = unique_test_db_path("explicit-empty");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let payload = event("session-1", "turn-1", "");

        let output =
            capture_with(&db, &payload, 1_000).expect("explicit empty text should persist");
        assert_eq!(output, "");

        assert_eq!(
            part_rows(&db),
            vec![(
                "cx_session-1".to_string(),
                "cx:turn-1:assistant".to_string(),
                "text".to_string(),
                String::new()
            )],
            "an explicit empty string is a present value, unlike null, and persists like any other text"
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn capture_with_persists_deserialized_raw_json_with_an_explicit_empty_string() {
        let db_path = unique_test_db_path("raw-json-empty-string");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let payload: CodexHookEvent = serde_json::from_str(
            r#"{"hook_event_name":"Stop","session_id":"s1","turn_id":"t1","last_assistant_message":""}"#,
        )
        .expect("raw JSON with an explicit empty string should deserialize");

        let output = capture_with(&db, &payload, 1_000)
            .expect("deserialized explicit empty string should persist");
        assert_eq!(output, "");
        assert_eq!(message_rows(&db).len(), 1);
        assert_eq!(
            part_rows(&db),
            vec![(
                "cx_s1".to_string(),
                "cx:t1:assistant".to_string(),
                "text".to_string(),
                String::new()
            )]
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn capture_with_persists_deserialized_raw_json_with_normal_text() {
        let db_path = unique_test_db_path("raw-json-normal-text");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let payload: CodexHookEvent = serde_json::from_str(
            r#"{"hook_event_name":"Stop","session_id":"s1","turn_id":"t1","last_assistant_message":"hello"}"#,
        )
        .expect("raw JSON with normal text should deserialize");

        let output =
            capture_with(&db, &payload, 1_000).expect("deserialized normal text should persist");
        assert_eq!(output, "");
        assert_eq!(message_rows(&db).len(), 1);
        assert_eq!(
            part_rows(&db),
            vec![(
                "cx_s1".to_string(),
                "cx:t1:assistant".to_string(),
                "text".to_string(),
                "hello".to_string()
            )]
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn handle_is_a_silent_no_op_for_a_null_last_assistant_message_without_opening_the_db() {
        let mut payload = event("session-1", "turn-1", "unused");
        payload.last_assistant_message = NullableField::Null;

        // A nonexistent repository root proves `handle` never reaches Agent
        // Trace DB resolution for a null `last_assistant_message`: DB opening
        // against a nonexistent repository would otherwise fail loudly.
        let output = handle(Path::new("/nonexistent-repository-root"), &payload)
            .expect("null last_assistant_message should be a silent successful no-op");
        assert_eq!(output, "");
    }

    #[test]
    fn handle_with_clock_errors_for_a_missing_last_assistant_message_without_calling_the_clock() {
        let mut payload = event("session-1", "turn-1", "unused");
        payload.last_assistant_message = NullableField::Missing;

        let error = handle_with_clock(Path::new("/nonexistent-repository-root"), &payload, || {
            panic!("clock must not be called for a missing last_assistant_message")
        })
        .expect_err("missing last_assistant_message should error");
        assert!(error.to_string().contains("'last_assistant_message'"));
    }

    #[test]
    fn handle_with_clock_is_a_silent_no_op_for_null_without_calling_the_clock_or_opening_the_db() {
        let mut payload = event("session-1", "turn-1", "unused");
        payload.last_assistant_message = NullableField::Null;

        // A failing/panicking clock closure and a nonexistent repository
        // root together prove `handle_with_clock` short-circuits before
        // timestamp acquisition and before Agent Trace DB resolution for an
        // explicit null.
        let output = handle_with_clock(Path::new("/nonexistent-repository-root"), &payload, || {
            panic!("clock must not be called for an explicit null last_assistant_message")
        })
        .expect("null last_assistant_message should be a silent successful no-op");
        assert_eq!(output, "");
    }

    #[test]
    fn handle_with_clock_propagates_a_timestamp_failure_as_an_error_with_no_persistence() {
        let payload = event("session-1", "turn-1", "hello back");

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
    fn handle_with_clock_rejects_a_null_stop_with_a_missing_session_id_without_calling_the_clock() {
        let mut payload = event("session-1", "turn-1", "unused");
        payload.session_id = None;
        payload.last_assistant_message = NullableField::Null;

        let error = handle_with_clock(Path::new("/nonexistent-repository-root"), &payload, || {
            panic!("clock must not be called for a malformed Stop payload")
        })
        .expect_err("a null Stop with a missing session_id must still be rejected as malformed");
        assert!(error.to_string().contains("'session_id'"));
    }

    #[test]
    fn handle_with_clock_rejects_a_null_stop_with_an_empty_session_id_without_calling_the_clock() {
        let mut payload = event("session-1", "turn-1", "unused");
        payload.session_id = Some(String::new());
        payload.last_assistant_message = NullableField::Null;

        let error = handle_with_clock(Path::new("/nonexistent-repository-root"), &payload, || {
            panic!("clock must not be called for a malformed Stop payload")
        })
        .expect_err("a null Stop with an empty session_id must still be rejected as malformed");
        assert!(error.to_string().contains("'session_id'"));
    }

    #[test]
    fn handle_with_clock_rejects_a_null_stop_with_a_whitespace_only_session_id_without_calling_the_clock(
    ) {
        let mut payload = event("session-1", "turn-1", "unused");
        payload.session_id = Some("   ".to_string());
        payload.last_assistant_message = NullableField::Null;

        let error = handle_with_clock(Path::new("/nonexistent-repository-root"), &payload, || {
            panic!("clock must not be called for a malformed Stop payload")
        })
        .expect_err(
            "a null Stop with a whitespace-only session_id must still be rejected as malformed",
        );
        assert!(error.to_string().contains("'session_id'"));
    }

    #[test]
    fn handle_with_clock_rejects_a_null_stop_with_a_missing_turn_id_without_calling_the_clock() {
        let mut payload = event("session-1", "turn-1", "unused");
        payload.turn_id = None;
        payload.last_assistant_message = NullableField::Null;

        let error = handle_with_clock(Path::new("/nonexistent-repository-root"), &payload, || {
            panic!("clock must not be called for a malformed Stop payload")
        })
        .expect_err("a null Stop with a missing turn_id must still be rejected as malformed");
        assert!(error.to_string().contains("'turn_id'"));
    }

    #[test]
    fn handle_with_clock_rejects_a_null_stop_with_an_empty_turn_id_without_calling_the_clock() {
        let mut payload = event("session-1", "turn-1", "unused");
        payload.turn_id = Some(String::new());
        payload.last_assistant_message = NullableField::Null;

        let error = handle_with_clock(Path::new("/nonexistent-repository-root"), &payload, || {
            panic!("clock must not be called for a malformed Stop payload")
        })
        .expect_err("a null Stop with an empty turn_id must still be rejected as malformed");
        assert!(error.to_string().contains("'turn_id'"));
    }

    #[test]
    fn handle_with_clock_rejects_a_null_stop_with_a_whitespace_only_turn_id_without_calling_the_clock(
    ) {
        let mut payload = event("session-1", "turn-1", "unused");
        payload.turn_id = Some("   ".to_string());
        payload.last_assistant_message = NullableField::Null;

        let error = handle_with_clock(Path::new("/nonexistent-repository-root"), &payload, || {
            panic!("clock must not be called for a malformed Stop payload")
        })
        .expect_err(
            "a null Stop with a whitespace-only turn_id must still be rejected as malformed",
        );
        assert!(error.to_string().contains("'turn_id'"));
    }

    #[test]
    fn handle_with_clock_is_a_silent_no_op_for_null_with_padded_ids_without_calling_the_clock() {
        let mut payload = event("session-1", "turn-1", "unused");
        payload.session_id = Some(" session-1 ".to_string());
        payload.turn_id = Some(" turn-1 ".to_string());
        payload.last_assistant_message = NullableField::Null;

        // Padded-but-otherwise-valid identifiers must validate under their
        // trimmed representation even though a null Stop persists nothing.
        let output = handle_with_clock(Path::new("/nonexistent-repository-root"), &payload, || {
            panic!("clock must not be called for an explicit null last_assistant_message")
        })
        .expect("a null Stop with padded-but-valid identifiers should still be a successful no-op");
        assert_eq!(output, "");
    }

    #[test]
    fn validate_stop_event_rejects_a_missing_last_assistant_message_with_valid_ids() {
        let mut payload = event("session-1", "turn-1", "unused");
        payload.last_assistant_message = NullableField::Missing;

        let error = validate_stop_event(&payload)
            .expect_err("missing last_assistant_message should be rejected");
        assert!(error.to_string().contains("'last_assistant_message'"));
    }

    #[test]
    fn validate_stop_event_returns_none_for_an_explicit_null_with_valid_ids() {
        let mut payload = event("session-1", "turn-1", "unused");
        payload.last_assistant_message = NullableField::Null;

        let validated =
            validate_stop_event(&payload).expect("valid ids with a null message should validate");
        assert_eq!(validated.session_id, "session-1");
        assert_eq!(validated.turn_id, "turn-1");
        assert_eq!(validated.last_assistant_message, None);
    }
}
