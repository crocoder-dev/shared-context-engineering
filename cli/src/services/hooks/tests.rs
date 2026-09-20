use std::{
    cell::RefCell,
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::*;
use crate::services::agent_trace_db::{
    ClaudeModelStateObservation, ObservationKind, ParsedDiffTracePatch, SkippedDiffTracePatch,
};

#[derive(Debug, Eq, PartialEq)]
struct CapturedPostCommitIntersectionInsert {
    commit_id: String,
    post_commit_time_ms: i64,
    recent_window_cutoff_ms: i64,
    recent_window_end_ms: i64,
    loaded_diff_trace_count: i64,
    skipped_diff_trace_count: i64,
    intersection_patch: String,
}

fn valid_patch_text(path: &str, content: &str) -> String {
    format!(
            "Index: {path}\n===================================================================\n--- {path}\n+++ {path}\n@@ -0,0 +1,1 @@\n+{content}\n"
        )
}

fn valid_patch(path: &str, content: &str) -> ParsedPatch {
    let patch_text = valid_patch_text(path, content);

    parse_patch_from_text(&patch_text, None).expect("test patch should parse")
}

#[test]
fn conversation_trace_mixed_payload_maps_to_message_and_part_insert_inputs() {
    let patch_text = valid_patch_text("src/lib.rs", "let answer = 42;");
    let question_text = serde_json::json!([
        {
            "question": "Proceed?",
            "answer": "Yes"
        }
    ])
    .to_string();
    let payload = serde_json::json!({
        "tool_name": "opencode",
        "payloads": [
            {
                "type": "message",
                "session_id": "session-1",
                "message_id": "message-1",
                "role": "assistant",
                "generated_at_unix_ms": 1_800_000_000_000_i64
            },
            {
                "type": "message.part",
                "session_id": "session-1",
                "message_id": "message-1",
                "part_type": "reasoning",
                "text": "thinking through validation",
                "generated_at_unix_ms": 1_800_000_000_001_i64
            },
            {
                "type": "message.part",
                "session_id": "session-1",
                "message_id": "message-1",
                "part_type": "patch",
                "text": patch_text,
                "generated_at_unix_ms": 1_800_000_000_002_i64
            },
            {
                "type": "message.part",
                "session_id": "session-1",
                "message_id": "message-1",
                "part_type": "question",
                "text": question_text,
                "generated_at_unix_ms": 1_800_000_000_003_i64
            }
        ]
    });

    let parsed = parse_conversation_trace_payload(&payload.to_string())
        .expect("conversation-trace mixed payload should parse");

    assert_eq!(parsed.attempted_count, 4);
    assert!(parsed.skipped.is_empty());
    assert!(parsed.message_updated.skipped.is_empty());
    assert!(parsed.message_part_updated.skipped.is_empty());

    assert_eq!(parsed.message_updated.inserts.len(), 1);
    let message = &parsed.message_updated.inserts[0];
    assert_eq!(message.session_id, "oc_session-1");
    assert_eq!(message.message_id, "message-1");
    assert_eq!(message.role, MessageRole::Assistant);
    assert_eq!(message.generated_at_unix_ms, 1_800_000_000_000_i64);

    assert_eq!(parsed.message_part_updated.inserts.len(), 3);
    let reasoning_part = &parsed.message_part_updated.inserts[0];
    assert_eq!(reasoning_part.session_id, "oc_session-1");
    assert_eq!(reasoning_part.message_id, "message-1");
    assert_eq!(reasoning_part.part_type, PartType::Reasoning);
    assert_eq!(reasoning_part.text, "thinking through validation");
    assert_eq!(reasoning_part.generated_at_unix_ms, 1_800_000_000_001_i64);

    let patch_part = &parsed.message_part_updated.inserts[1];
    assert_eq!(patch_part.session_id, "oc_session-1");
    assert_eq!(patch_part.message_id, "message-1");
    assert_eq!(patch_part.part_type, PartType::Patch);
    assert_eq!(
        patch_part.text,
        serialize_to_json(&valid_patch("src/lib.rs", "let answer = 42;"))
            .expect("test patch should serialize")
    );
    assert_eq!(patch_part.generated_at_unix_ms, 1_800_000_000_002_i64);

    let question_part = &parsed.message_part_updated.inserts[2];
    assert_eq!(question_part.session_id, "oc_session-1");
    assert_eq!(question_part.message_id, "message-1");
    assert_eq!(question_part.part_type, PartType::Question);
    assert_eq!(question_part.text, question_text);
    assert_eq!(question_part.generated_at_unix_ms, 1_800_000_000_003_i64);
}

#[test]
fn conversation_trace_mixed_payload_skips_malformed_sibling_items() {
    let invalid_question_text = serde_json::json!({
        "question": "Proceed?",
        "answer": "Yes"
    })
    .to_string();
    let payload = serde_json::json!({
        "tool_name": "opencode",
        "payloads": [
            {
                "type": "message",
                "session_id": "session-1",
                "message_id": "message-1",
                "role": "assistant",
                "generated_at_unix_ms": 1_800_000_000_000_i64
            },
            {
                "type": "message",
                "session_id": "session-2",
                "message_id": "message-2",
                "role": "system",
                "generated_at_unix_ms": 1_800_000_000_002_i64
            },
            {
                "type": "message.part",
                "session_id": "session-3",
                "message_id": "message-3",
                "part_type": "text",
                "generated_at_unix_ms": 1_800_000_000_003_i64
            },
            {
                "type": "message.part",
                "session_id": "session-4",
                "message_id": "message-4",
                "part_type": "patch",
                "text": "--- src/main.rs",
                "generated_at_unix_ms": 1_800_000_000_004_i64
            },
            {
                "type": "message.part",
                "session_id": "session-5",
                "message_id": "message-5",
                "part_type": "question",
                "text": invalid_question_text,
                "generated_at_unix_ms": 1_800_000_000_005_i64
            },
            {
                "type": "session.started",
                "session_id": "session-6"
            },
            42,
            {
                "type": null,
                "session_id": "session-7"
            }
        ]
    });

    let parsed = parse_conversation_trace_payload(&payload.to_string())
        .expect("conversation-trace mixed payload should parse with skipped items");

    assert_eq!(parsed.attempted_count, 8);
    assert_eq!(parsed.message_updated.inserts.len(), 1);
    assert_eq!(parsed.message_updated.skipped.len(), 1);
    assert_eq!(parsed.message_updated.skipped[0].index, 1);
    assert!(parsed.message_updated.skipped[0]
        .reason
        .contains("field 'role'"));
    assert_eq!(parsed.message_part_updated.inserts.len(), 0);
    assert_eq!(parsed.message_part_updated.skipped.len(), 3);
    assert_eq!(parsed.message_part_updated.skipped[0].index, 2);
    assert!(parsed.message_part_updated.skipped[0]
        .reason
        .contains("missing required field 'text'"));
    assert_eq!(parsed.message_part_updated.skipped[1].index, 3);
    assert!(parsed.message_part_updated.skipped[1]
        .reason
        .contains("neither valid patch-JSON nor a valid patch"));
    assert_eq!(parsed.message_part_updated.skipped[2].index, 4);
    assert!(parsed.message_part_updated.skipped[2]
        .reason
        .contains("question part must be a JSON array"));
    assert_eq!(parsed.skipped.len(), 3);
    assert_eq!(parsed.skipped[0].index, 5);
    assert!(parsed.skipped[0].reason.contains("field 'type'"));
    assert_eq!(parsed.skipped[1].index, 6);
    assert!(parsed.skipped[1]
        .reason
        .contains("payloads[6] must be an object"));
    assert_eq!(parsed.skipped[2].index, 7);
    assert!(parsed.skipped[2]
        .reason
        .contains("field 'type' must be a string"));
}

fn normalized_conversation_trace_message_payload(tool_name: &str, session_id: &str) -> String {
    serde_json::json!({
        "tool_name": tool_name,
        "payloads": [
            {
                "type": "message",
                "session_id": session_id,
                "message_id": "message-1",
                "role": "assistant",
                "generated_at_unix_ms": 1_800_000_000_000_i64
            }
        ]
    })
    .to_string()
}

#[test]
fn conversation_trace_normalized_payload_accepts_pi_tool_name_with_prefixed_session_id() {
    let stdin_payload = normalized_conversation_trace_message_payload("pi", "session-1");

    let parsed = parse_conversation_trace_payload(&stdin_payload)
        .expect("Pi normalized conversation-trace payload should parse");

    assert_eq!(parsed.message_updated.inserts.len(), 1);
    assert_eq!(parsed.message_updated.inserts[0].session_id, "pi_session-1");
}

#[test]
fn conversation_trace_normalized_payload_rejects_unsupported_tool_name() {
    let stdin_payload = normalized_conversation_trace_message_payload("cursor", "session-1");

    let error = parse_conversation_trace_payload(&stdin_payload)
        .expect_err("unsupported tool_name should be rejected");

    assert!(error.to_string().contains("unsupported tool_name 'cursor'"));
    assert!(error.to_string().contains("'opencode'"));
    assert!(error.to_string().contains("'pi'"));
}

#[test]
fn conversation_trace_normalized_payload_rejects_empty_tool_name() {
    let stdin_payload = normalized_conversation_trace_message_payload("", "session-1");

    let error = parse_conversation_trace_payload(&stdin_payload)
        .expect_err("empty tool_name should be rejected");

    assert!(error
        .to_string()
        .contains("field 'tool_name' must be a non-empty string"));
}

#[test]
fn conversation_trace_normalized_payload_rejects_missing_tool_name() {
    let stdin_payload = serde_json::json!({
        "payloads": [
            {
                "type": "message",
                "session_id": "session-1",
                "message_id": "message-1",
                "role": "assistant",
                "generated_at_unix_ms": 1_800_000_000_000_i64
            }
        ]
    })
    .to_string();

    let error = parse_conversation_trace_payload(&stdin_payload)
        .expect_err("missing tool_name should be rejected");

    assert!(error
        .to_string()
        .contains("missing required field 'tool_name'"));
}

#[test]
fn conversation_trace_normalized_payload_keeps_already_prefixed_session_id() {
    let stdin_payload = normalized_conversation_trace_message_payload("opencode", "oc_session-1");

    let parsed = parse_conversation_trace_payload(&stdin_payload)
        .expect("already-prefixed OpenCode session ID should parse");

    assert_eq!(parsed.message_updated.inserts[0].session_id, "oc_session-1");
}

#[test]
fn conversation_trace_raw_claude_event_uses_claude_identity_with_cc_prefixed_session_id() {
    let stdin_payload = serde_json::json!({
        "hook_event_name": "UserPromptSubmit",
        "session_id": "session-1",
        "prompt": "hello"
    })
    .to_string();

    let parsed = parse_conversation_trace_payload(&stdin_payload)
        .expect("raw Claude UserPromptSubmit event should parse");

    assert_eq!(parsed.message_updated.inserts.len(), 1);
    assert_eq!(parsed.message_updated.inserts[0].session_id, "cc_session-1");
}

fn diff_trace_payload(model_id: Option<&str>, tool_version: Option<&str>) -> DiffTracePayload {
    diff_trace_payload_with(
        "claude",
        "session-123",
        PAYLOAD_TYPE_STRUCTURED,
        model_id,
        tool_version,
    )
}

fn diff_trace_payload_with(
    tool_name: &str,
    session_id: &str,
    payload_type: &str,
    model_id: Option<&str>,
    tool_version: Option<&str>,
) -> DiffTracePayload {
    DiffTracePayload {
        session_id: String::from(session_id),
        diff: String::from("diff text"),
        time: 1_800_000_000_000_u64,
        model_id: model_id.map(String::from),
        agent_id: None,
        transcript_path: None,
        tool_name: String::from(tool_name),
        tool_version: tool_version.map(String::from),
        payload_type: String::from(payload_type),
    }
}

fn claude_model_test_event(transcript_path: &Path, tool_use_id: &str) -> Value {
    json!({
        "hook_event_name": "PostToolUse",
        "session_id": "session-123",
        "tool_name": "Write",
        "tool_use_id": tool_use_id,
        "transcript_path": transcript_path,
        "tool_input": {
            "file_path": "docs/status.md",
            "content": "# Status\n\nThe new state is complete.\n"
        },
        "tool_response": {
            "originalFile": "# Status\n\nThe old state is pending.\n",
            "structuredPatch": {
                "hunks": [{
                    "oldStart": 1,
                    "oldCount": 3,
                    "newStart": 1,
                    "newCount": 3,
                    "lines": [
                        " # Status",
                        " ",
                        "-The old state is pending.",
                        "+The new state is complete."
                    ]
                }]
            }
        }
    })
}

fn parsed_claude_model_id(event: &Value) -> Option<String> {
    match parse_diff_trace_payload(&event.to_string())
        .expect("Claude PostToolUse diff-trace payload should parse")
    {
        DiffTraceParseResult::Persist(payload) => payload.model_id,
        DiffTraceParseResult::NoOp(message) => {
            panic!("Claude Write payload should persist, got no-op: {message}")
        }
    }
}

fn parsed_claude_diff_trace(event: &Value) -> DiffTracePayload {
    match parse_diff_trace_payload(&event.to_string())
        .expect("Claude PostToolUse diff-trace payload should parse")
    {
        DiffTraceParseResult::Persist(payload) => payload,
        DiffTraceParseResult::NoOp(message) => {
            panic!("Claude Write payload should persist, got no-op: {message}")
        }
    }
}

fn unique_attribution_db_path(label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_nanos();
    std::env::temp_dir()
        .join(format!("sce-claude-model-attribution-{label}-{suffix}"))
        .join("agent-trace.db")
}

fn resolved_claude_model_id_with<F>(event: &Value, transcript_lookup: F) -> Option<String>
where
    F: FnOnce(&Path, &str) -> Option<String>,
{
    resolve_claude_model_id_with(
        event.as_object().expect("test event should be an object"),
        transcript_lookup,
    )
}

fn run_attribution_git(repo_root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .expect("git should start");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_attribution_git_repo(label: &str) -> PathBuf {
    let repo_root = unique_attribution_db_path(label)
        .parent()
        .expect("test repository should have a parent")
        .to_path_buf();
    fs::create_dir_all(&repo_root).expect("test repository directory should be created");
    run_attribution_git(&repo_root, &["init", "-q"]);
    run_attribution_git(
        &repo_root,
        &["remote", "add", "origin", "git@github.com:acme/widgets.git"],
    );
    repo_root
}

fn model_less_claude_diff_event(
    session_id: &str,
    tool_use_id: &str,
    agent_id: Option<&str>,
) -> Value {
    let mut event = claude_model_test_event(Path::new("/virtual/missing.jsonl"), tool_use_id);
    let object = event
        .as_object_mut()
        .expect("Claude test event should be an object");
    object.insert("session_id".to_string(), json!(session_id));
    object.remove("transcript_path");
    object.remove("tool_use_id");
    if let Some(agent_id) = agent_id {
        object.insert("agent_id".to_string(), json!(agent_id));
    }
    event
}

fn persisted_model_ids(db: &RepositoryAgentTraceDb) -> Vec<Option<String>> {
    db.query_map(
        "SELECT model_id FROM diff_traces ORDER BY id ASC",
        (),
        |row| row.get::<Option<String>>(0).map_err(Into::into),
    )
    .expect("persisted model IDs should be readable")
}

#[test]
fn claude_model_direct_nested_metadata_wins_over_transcript_without_double_prefixing() {
    let transcript_path = Path::new("/unused/direct-precedence.jsonl");
    let mut event = claude_model_test_event(transcript_path, "tool-123");
    event
        .as_object_mut()
        .expect("test event should be an object")
        .insert("model".to_string(), json!({ "id": "claude/direct-model" }));

    let model_id = resolved_claude_model_id_with(&event, |_, _| {
        panic!("transcript lookup must not run when direct metadata is present")
    });

    assert_eq!(model_id.as_deref(), Some("claude/direct-model"));
    assert_eq!(parsed_claude_model_id(&event), model_id);
}

#[test]
fn claude_model_falls_back_to_matching_transcript_and_normalizes_model() {
    let transcript_path = Path::new("/virtual/transcript-fallback.jsonl");
    let event = claude_model_test_event(transcript_path, "tool-123");

    let model_id = resolved_claude_model_id_with(&event, |path, tool_use_id| {
        assert_eq!(path, transcript_path);
        assert_eq!(tool_use_id, "tool-123");
        Some(String::from("claude/claude-opus-4-1"))
    });

    assert_eq!(model_id.as_deref(), Some("claude/claude-opus-4-1"));
}

#[test]
fn claude_model_remains_none_when_transcript_lookup_cannot_succeed() {
    let event = claude_model_test_event(Path::new("/virtual/missing.jsonl"), "tool-123");
    assert_eq!(resolved_claude_model_id_with(&event, |_, _| None), None);

    let mut event_without_lookup_fields = event;
    let payload = event_without_lookup_fields
        .as_object_mut()
        .expect("test event should be an object");
    payload.remove("transcript_path");
    payload.remove("tool_use_id");
    assert_eq!(
        resolved_claude_model_id_with(&event_without_lookup_fields, |_, _| {
            panic!("lookup must not run without transcript event metadata")
        }),
        None
    );
}

#[test]
fn claude_diff_trace_parser_keeps_agent_id_ephemeral_and_storage_free() {
    let mut event = claude_model_test_event(Path::new("/virtual/missing.jsonl"), "tool-123");
    event
        .as_object_mut()
        .expect("test event should be an object")
        .insert("agent_id".to_string(), json!(" agent-1 "));

    let payload = parsed_claude_diff_trace(&event);

    assert_eq!(payload.agent_id.as_deref(), Some("agent-1"));
    assert!(serde_json::to_value(&payload)
        .expect("internal payload should serialize")
        .get("agent_id")
        .is_none());
}

#[test]
fn claude_diff_trace_parser_keeps_transcript_path_ephemeral_and_storage_free() {
    let transcript_path = Path::new("/virtual/session-123.jsonl");
    let event = claude_model_test_event(transcript_path, "tool-123");

    let payload = parsed_claude_diff_trace(&event);

    assert_eq!(
        payload.transcript_path.as_deref(),
        Some("/virtual/session-123.jsonl")
    );
    assert!(serde_json::to_value(&payload)
        .expect("internal payload should serialize")
        .get("transcript_path")
        .is_none());
}

#[test]
fn claude_diff_trace_parser_leaves_transcript_path_none_without_the_field() {
    let mut event = claude_model_test_event(Path::new("/virtual/missing.jsonl"), "tool-123");
    event
        .as_object_mut()
        .expect("test event should be an object")
        .remove("transcript_path");

    assert_eq!(parsed_claude_diff_trace(&event).transcript_path, None);
}

#[test]
fn claude_diff_trace_normalized_opencode_payload_carries_no_transcript_path() {
    let stdin_payload = serde_json::json!({
        "sessionID": "session-123",
        "diff": "diff text",
        "time": 1_800_000_000_000_u64,
        "model_id": "anthropic/claude-opus-4",
        "tool_name": "opencode",
        "tool_version": null
    })
    .to_string();

    let parsed = parse_diff_trace_payload(&stdin_payload)
        .expect("normalized OpenCode diff-trace payload should parse");
    let payload = match parsed {
        DiffTraceParseResult::Persist(payload) => payload,
        DiffTraceParseResult::NoOp(message) => {
            panic!("normalized OpenCode payload should persist, got no-op: {message}")
        }
    };

    assert_eq!(payload.transcript_path, None);
}

#[test]
#[allow(clippy::too_many_lines)]
fn claude_model_attribution_end_to_end_persists_lifecycle_fallback_precedence_and_scope() {
    let repo_root = init_attribution_git_repo("end-to-end");
    let state_root = unique_attribution_db_path("end-to-end-state")
        .parent()
        .expect("test state should have a parent")
        .to_path_buf();
    let storage = resolve_agent_trace_storage_at_state_root(
        &AgentTraceStorageContext {
            repository_root: &repo_root,
            explicit_repository_id: None,
            repository_remote: "origin",
        },
        &state_root,
    )
    .expect("setup path should initialize the test repository DB");
    drop(storage);

    let session_start = json!({
        "hook_event_name": "SessionStart",
        "session_id": "session-123",
        "model": "model-a",
        "source": "startup"
    });
    assert_eq!(
        claude_model_state::run_claude_model_state_from_payload_at_state_root(
            &repo_root,
            &state_root,
            &session_start.to_string(),
            None,
            || Ok(10),
        ),
        ""
    );

    let db = open_agent_trace_db_for_hook_runtime_at_state_root(
        &repo_root,
        &state_root,
        "test DB should open after SessionStart",
    )
    .expect("test DB should open after SessionStart");
    assert_eq!(
        db.claude_model_state_by_session_and_agent("cc_session-123", "")
            .expect("SessionStart state should be readable")
            .expect("SessionStart should seed state")
            .model_id,
        "claude/model-a"
    );
    let session_start_event = model_less_claude_diff_event("session-123", "tool-a", None);
    persist_diff_trace_payload_to_agent_trace_db_with_db(
        &db,
        &parsed_claude_diff_trace(&session_start_event),
    )
    .expect("SessionStart state should attribute the next diff trace");
    drop(db);

    let post_model_switch = json!({
        "hook_event_name": "PostModelSwitch",
        "session_id": "session-123",
        "from_model": "model-a",
        "to_model": "model-b",
        "source": "picker"
    });
    assert_eq!(
        claude_model_state::run_claude_model_state_from_payload_at_state_root(
            &repo_root,
            &state_root,
            &post_model_switch.to_string(),
            None,
            || Ok(20),
        ),
        ""
    );

    let db = open_agent_trace_db_for_hook_runtime_at_state_root(
        &repo_root,
        &state_root,
        "test DB should open after PostModelSwitch",
    )
    .expect("test DB should open after PostModelSwitch");
    assert_eq!(
        db.claude_model_state_by_session_and_agent("cc_session-123", "")
            .expect("PostModelSwitch state should be readable")
            .expect("PostModelSwitch should update state")
            .model_id,
        "claude/model-b"
    );
    let switched_event = model_less_claude_diff_event("session-123", "tool-b", None);
    persist_diff_trace_payload_to_agent_trace_db_with_db(
        &db,
        &parsed_claude_diff_trace(&switched_event),
    )
    .expect("PostModelSwitch state should attribute the next diff trace");

    let mut direct_event = model_less_claude_diff_event("session-123", "tool-direct", None);
    direct_event
        .as_object_mut()
        .expect("Claude test event should be an object")
        .insert("model".to_string(), json!("model-c"));
    persist_diff_trace_payload_to_agent_trace_db_with_db(
        &db,
        &parsed_claude_diff_trace(&direct_event),
    )
    .expect("direct model attribution should persist");

    let transcript_path = state_root.join("transcript.jsonl");
    fs::write(
            &transcript_path,
            concat!(
                r#"{"type":"assistant","message":{"role":"assistant","model":"model-c","content":[{"type":"tool_use","id":"tool-transcript"}]}}"#,
                "\n"
            ),
        )
        .expect("transcript fixture should be written");
    let transcript_event = claude_model_test_event(&transcript_path, "tool-transcript");
    persist_diff_trace_payload_to_agent_trace_db_with_db(
        &db,
        &parsed_claude_diff_trace(&transcript_event),
    )
    .expect("transcript model attribution should persist");

    let no_state_event = model_less_claude_diff_event("session-without-state", "tool-none", None);
    persist_diff_trace_payload_to_agent_trace_db_with_db(
        &db,
        &parsed_claude_diff_trace(&no_state_event),
    )
    .expect("an attribution-less diff trace should still persist");

    let subagent_event =
        model_less_claude_diff_event("session-123", "tool-subagent", Some("subagent-1"));
    persist_diff_trace_payload_to_agent_trace_db_with_db(
        &db,
        &parsed_claude_diff_trace(&subagent_event),
    )
    .expect("a subagent diff trace should persist");

    assert_eq!(
        persisted_model_ids(&db),
        vec![
            Some(String::from("claude/model-a")),
            Some(String::from("claude/model-b")),
            Some(String::from("claude/model-c")),
            Some(String::from("claude/model-c")),
            None,
            None,
        ]
    );

    drop(db);
    fs::remove_file(transcript_path).expect("transcript fixture should be removed");
    fs::remove_dir_all(repo_root).expect("test repository should be removed");
    fs::remove_dir_all(state_root).expect("test state should be removed");
}

#[test]
fn claude_model_attribution_bridge_inheritance_seeds_state_and_diff_trace() {
    let repo_root = init_attribution_git_repo("bridge-inheritance");
    let state_root = unique_attribution_db_path("bridge-inheritance-state")
        .parent()
        .expect("test state should have a parent")
        .to_path_buf();
    let storage = resolve_agent_trace_storage_at_state_root(
        &AgentTraceStorageContext {
            repository_root: &repo_root,
            explicit_repository_id: None,
            repository_remote: "origin",
        },
        &state_root,
    )
    .expect("setup path should initialize the test repository DB");
    drop(storage);

    let sibling_transcript = state_root.join("session-old.jsonl");
    let current_transcript = state_root.join("session-current.jsonl");
    fs::write(
        &sibling_transcript,
        concat!(
            r#"{"type":"file-history-snapshot"}"#,
            "\n",
            r#"{"type":"bridge-session","sessionId":"session-old","bridgeSessionId":"cse_shared"}"#,
            "\n",
        ),
    )
    .expect("sibling transcript fixture should be written");
    fs::write(
            &current_transcript,
            concat!(
                r#"{"type":"file-history-snapshot"}"#,
                "\n",
                r#"{"type":"bridge-session","sessionId":"session-current","bridgeSessionId":"cse_shared"}"#,
                "\n",
            ),
        )
        .expect("current transcript fixture should be written");

    let db = open_agent_trace_db_for_hook_runtime_at_state_root(
        &repo_root,
        &state_root,
        "test DB should open before bridge inheritance",
    )
    .expect("test DB should open before bridge inheritance");
    db.upsert_claude_model_state(ClaudeModelStateObservation {
        session_id: String::from("cc_session-old"),
        agent_id: String::new(),
        model_id: String::from("claude/inherited-model"),
        observation_kind: ObservationKind::SessionStart,
        source: String::from("startup"),
        observed_at_ms: 5,
    })
    .expect("sibling state should be seeded");
    drop(db);

    let session_start = json!({
        "hook_event_name": "SessionStart",
        "session_id": "session-current",
        "source": "clear",
        "transcript_path": current_transcript,
    });
    assert_eq!(
        claude_model_state::run_claude_model_state_from_payload_at_state_root(
            &repo_root,
            &state_root,
            &session_start.to_string(),
            None,
            || Ok(10),
        ),
        ""
    );

    let db = open_agent_trace_db_for_hook_runtime_at_state_root(
        &repo_root,
        &state_root,
        "test DB should open after bridge inheritance",
    )
    .expect("test DB should open after bridge inheritance");
    let inherited = db
        .claude_model_state_by_session_and_agent("cc_session-current", "")
        .expect("inherited state lookup should succeed")
        .expect("current session should inherit sibling state");
    assert_eq!(inherited.model_id, "claude/inherited-model");
    assert_eq!(inherited.source, "bridge_inherited");
    assert_eq!(inherited.observation_kind, ObservationKind::SessionStart);
    assert_eq!(inherited.observed_at_ms, 10);

    let diff_event = model_less_claude_diff_event("session-current", "tool-inherited", None);
    persist_diff_trace_payload_to_agent_trace_db_with_db(
        &db,
        &parsed_claude_diff_trace(&diff_event),
    )
    .expect("inherited state should attribute the diff trace");
    assert_eq!(
        persisted_model_ids(&db),
        vec![Some(String::from("claude/inherited-model"))]
    );

    drop(db);
    fs::remove_file(sibling_transcript).expect("sibling transcript should be removed");
    fs::remove_file(current_transcript).expect("current transcript should be removed");
    fs::remove_dir_all(repo_root).expect("test repository should be removed");
    fs::remove_dir_all(state_root).expect("test state should be removed");
}

#[test]
fn claude_diff_trace_persistence_uses_state_only_after_direct_and_transcript() {
    let db_path = unique_attribution_db_path("precedence");
    let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
    db.upsert_claude_model_state(ClaudeModelStateObservation {
        session_id: String::from("cc_session-123"),
        agent_id: String::new(),
        model_id: String::from("claude/state-model"),
        observation_kind: ObservationKind::SessionStart,
        source: String::from("startup"),
        observed_at_ms: 1,
    })
    .expect("state should be seeded");

    let mut state_event = claude_model_test_event(Path::new("/virtual/missing.jsonl"), "state");
    let state_object = state_event
        .as_object_mut()
        .expect("test event should be an object");
    state_object.remove("transcript_path");
    state_object.remove("tool_use_id");
    let state_payload = parsed_claude_diff_trace(&state_event);
    persist_diff_trace_payload_to_agent_trace_db_with_db(&db, &state_payload)
        .expect("state fallback should persist");

    let mut direct_event = state_event.clone();
    direct_event
        .as_object_mut()
        .expect("test event should be an object")
        .insert("model".to_string(), json!("direct-model"));
    let direct_payload = parsed_claude_diff_trace(&direct_event);
    persist_diff_trace_payload_to_agent_trace_db_with_db(&db, &direct_payload)
        .expect("direct attribution should persist");

    let transcript_path = db_path.with_extension("jsonl");
    fs::write(
            &transcript_path,
            concat!(
                r#"{"type":"assistant","message":{"role":"assistant","model":"transcript-model","content":[{"type":"tool_use","id":"transcript"}]}}"#,
                "\n"
            ),
        )
        .expect("transcript fixture should be written");
    let transcript_event = claude_model_test_event(&transcript_path, "transcript");
    let transcript_payload = parsed_claude_diff_trace(&transcript_event);
    persist_diff_trace_payload_to_agent_trace_db_with_db(&db, &transcript_payload)
        .expect("transcript attribution should persist");

    let models = db
        .query_map(
            "SELECT model_id FROM diff_traces ORDER BY id ASC",
            (),
            |row| row.get::<Option<String>>(0).map_err(Into::into),
        )
        .expect("persisted models should be readable");
    assert_eq!(
        models,
        vec![
            Some(String::from("claude/state-model")),
            Some(String::from("claude/direct-model")),
            Some(String::from("claude/transcript-model")),
        ]
    );

    drop(db);
    fs::remove_file(transcript_path).expect("transcript fixture should be removed");
    fs::remove_dir_all(db_path.parent().expect("test DB should have a parent"))
        .expect("test DB directory should be removed");
}

#[test]
fn normalized_claude_tool_name_does_not_use_claude_state_fallback() {
    let db_path = unique_attribution_db_path("normalized-claude");
    let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
    db.upsert_claude_model_state(ClaudeModelStateObservation {
        session_id: String::from("cc_session-123"),
        agent_id: String::new(),
        model_id: String::from("claude/parent-model"),
        observation_kind: ObservationKind::SessionStart,
        source: String::from("startup"),
        observed_at_ms: 1,
    })
    .expect("parent state should be seeded");

    let payload = diff_trace_payload_with(
        CLAUDE_TOOL_NAME,
        "session-123",
        PAYLOAD_TYPE_PATCH,
        None,
        None,
    );
    persist_diff_trace_payload_to_agent_trace_db_with_db(&db, &payload)
        .expect("normalized Claude payload should persist");

    let model = db
        .query_map("SELECT model_id FROM diff_traces LIMIT 1", (), |row| {
            row.get::<Option<String>>(0).map_err(Into::into)
        })
        .expect("persisted model should be readable")
        .into_iter()
        .next()
        .expect("diff trace row should exist");
    assert_eq!(model, None);

    drop(db);
    fs::remove_dir_all(db_path.parent().expect("test DB should have a parent"))
        .expect("test DB directory should be removed");
}

#[test]
fn claude_diff_trace_state_lookup_isolated_to_exact_subagent_scope() {
    let db_path = unique_attribution_db_path("subagent");
    let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
    db.upsert_claude_model_state(ClaudeModelStateObservation {
        session_id: String::from("cc_session-123"),
        agent_id: String::new(),
        model_id: String::from("claude/parent-model"),
        observation_kind: ObservationKind::SessionStart,
        source: String::from("startup"),
        observed_at_ms: 1,
    })
    .expect("parent state should be seeded");

    let mut event = claude_model_test_event(Path::new("/virtual/missing.jsonl"), "subagent");
    let event_object = event
        .as_object_mut()
        .expect("test event should be an object");
    event_object.remove("transcript_path");
    event_object.remove("tool_use_id");
    event_object.insert("agent_id".to_string(), json!("subagent-1"));
    let payload = parsed_claude_diff_trace(&event);
    persist_diff_trace_payload_to_agent_trace_db_with_db(&db, &payload)
        .expect("subagent diff trace should persist");

    let model = db
        .query_map("SELECT model_id FROM diff_traces LIMIT 1", (), |row| {
            row.get::<Option<String>>(0).map_err(Into::into)
        })
        .expect("persisted model should be readable")
        .into_iter()
        .next()
        .expect("diff trace row should exist");
    assert_eq!(model, None);

    drop(db);
    fs::remove_dir_all(db_path.parent().expect("test DB should have a parent"))
        .expect("test DB directory should be removed");
}

fn write_bridge_transcript(path: &Path, bridge_session_id: &str) {
    fs::write(
        path,
        format!(
            concat!(
                "{{\"type\":\"file-history-snapshot\"}}\n",
                "{{\"type\":\"bridge-session\",\"sessionId\":\"s\",",
                "\"bridgeSessionId\":\"{bridge_session_id}\"}}\n"
            ),
            bridge_session_id = bridge_session_id,
        ),
    )
    .expect("bridge transcript fixture should be written");
}

#[test]
fn claude_diff_trace_seeds_bridge_chain_state_on_state_miss_and_reuses_it() {
    let db_path = unique_attribution_db_path("bridge-chain-seed");
    let dir = db_path
        .parent()
        .expect("test DB should have a parent")
        .to_path_buf();
    fs::create_dir_all(&dir).expect("test DB directory should be created");
    let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");

    db.upsert_claude_model_state(ClaudeModelStateObservation {
        session_id: String::from("cc_session-member"),
        agent_id: String::new(),
        model_id: String::from("claude/chain-model"),
        observation_kind: ObservationKind::SessionStart,
        source: String::from("startup"),
        observed_at_ms: 100,
    })
    .expect("chain member state should seed");

    let current_transcript = dir.join("session-current.jsonl");
    let member_transcript = dir.join("session-member.jsonl");
    write_bridge_transcript(&current_transcript, "cse_chain");
    write_bridge_transcript(&member_transcript, "cse_chain");

    let payload = parsed_claude_diff_trace(&claude_model_test_event(&current_transcript, "a"));
    persist_diff_trace_payload_to_agent_trace_db_with_db(&db, &payload)
        .expect("bridge chain seeding should persist");

    assert_eq!(
        persisted_model_ids(&db),
        vec![Some(String::from("claude/chain-model"))]
    );
    let seeded = db
        .claude_model_state_by_session_and_agent("cc_session-123", "")
        .expect("seeded lookup should succeed")
        .expect("current session should be seeded");
    assert_eq!(seeded.model_id, "claude/chain-model");
    assert_eq!(seeded.source, "bridge_inherited");

    fs::remove_file(&member_transcript).expect("member transcript should be removed");
    let payload_two = parsed_claude_diff_trace(&claude_model_test_event(&current_transcript, "b"));
    persist_diff_trace_payload_to_agent_trace_db_with_db(&db, &payload_two)
        .expect("second diff trace should persist");
    assert_eq!(
        persisted_model_ids(&db),
        vec![
            Some(String::from("claude/chain-model")),
            Some(String::from("claude/chain-model")),
        ]
    );
    let after = db
        .claude_model_state_by_session_and_agent("cc_session-123", "")
        .expect("lookup should succeed")
        .expect("row should still exist");
    assert_eq!(after.observed_at_ms, seeded.observed_at_ms);

    drop(db);
    fs::remove_dir_all(&dir).expect("test DB directory should be removed");
}

#[test]
fn claude_diff_trace_bridge_chain_selects_newest_observation_across_members() {
    let db_path = unique_attribution_db_path("bridge-chain-newest");
    let dir = db_path
        .parent()
        .expect("test DB should have a parent")
        .to_path_buf();
    fs::create_dir_all(&dir).expect("test DB directory should be created");
    let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");

    db.upsert_claude_model_state(ClaudeModelStateObservation {
        session_id: String::from("cc_session-root"),
        agent_id: String::new(),
        model_id: String::from("claude/sonnet-5"),
        observation_kind: ObservationKind::SessionStart,
        source: String::from("startup"),
        observed_at_ms: 10,
    })
    .expect("root state should seed");
    db.upsert_claude_model_state(ClaudeModelStateObservation {
        session_id: String::from("cc_session-mid"),
        agent_id: String::new(),
        model_id: String::from("claude/opus-5"),
        observation_kind: ObservationKind::PostModelSwitch,
        source: String::from("picker"),
        observed_at_ms: 20,
    })
    .expect("mid state should seed");

    let current_transcript = dir.join("session-current.jsonl");
    let root_transcript = dir.join("session-root.jsonl");
    let mid_transcript = dir.join("session-mid.jsonl");
    write_bridge_transcript(&mid_transcript, "cse_chain");
    write_bridge_transcript(&current_transcript, "cse_chain");
    thread::sleep(Duration::from_millis(15));
    write_bridge_transcript(&root_transcript, "cse_chain");

    let payload = parsed_claude_diff_trace(&claude_model_test_event(&current_transcript, "x"));
    persist_diff_trace_payload_to_agent_trace_db_with_db(&db, &payload)
        .expect("newest-observation resolution should persist");

    assert_eq!(
        persisted_model_ids(&db),
        vec![Some(String::from("claude/opus-5"))]
    );

    drop(db);
    fs::remove_dir_all(&dir).expect("test DB directory should be removed");
}

#[test]
fn claude_diff_trace_bridge_chain_fails_open_without_write_or_attribution() {
    let db_path = unique_attribution_db_path("bridge-chain-fail-open");
    let dir = db_path
        .parent()
        .expect("test DB should have a parent")
        .to_path_buf();
    fs::create_dir_all(&dir).expect("test DB directory should be created");
    let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");

    let mut event = claude_model_test_event(Path::new("/virtual/missing.jsonl"), "no-transcript");
    event
        .as_object_mut()
        .expect("event should be an object")
        .remove("transcript_path");
    persist_diff_trace_payload_to_agent_trace_db_with_db(&db, &parsed_claude_diff_trace(&event))
        .expect("missing transcript should fail open");

    let current_transcript = dir.join("session-current.jsonl");
    let member_transcript = dir.join("session-member.jsonl");
    write_bridge_transcript(&current_transcript, "cse_chain");
    write_bridge_transcript(&member_transcript, "cse_chain");
    persist_diff_trace_payload_to_agent_trace_db_with_db(
        &db,
        &parsed_claude_diff_trace(&claude_model_test_event(&current_transcript, "no-state")),
    )
    .expect("stateless chain should fail open");

    assert_eq!(persisted_model_ids(&db), vec![None, None]);
    assert!(
        db.claude_model_state_by_session_and_agent("cc_session-123", "")
            .expect("lookup should succeed")
            .is_none(),
        "no state row should be written on a fail-open branch"
    );

    drop(db);
    fs::remove_dir_all(&dir).expect("test DB directory should be removed");
}

#[test]
fn claude_diff_trace_bridge_chain_does_not_seed_subagent_scope() {
    let db_path = unique_attribution_db_path("bridge-chain-subagent");
    let dir = db_path
        .parent()
        .expect("test DB should have a parent")
        .to_path_buf();
    fs::create_dir_all(&dir).expect("test DB directory should be created");
    let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");

    db.upsert_claude_model_state(ClaudeModelStateObservation {
        session_id: String::from("cc_session-member"),
        agent_id: String::new(),
        model_id: String::from("claude/chain-model"),
        observation_kind: ObservationKind::SessionStart,
        source: String::from("startup"),
        observed_at_ms: 100,
    })
    .expect("chain member state should seed");

    let current_transcript = dir.join("session-current.jsonl");
    let member_transcript = dir.join("session-member.jsonl");
    write_bridge_transcript(&current_transcript, "cse_chain");
    write_bridge_transcript(&member_transcript, "cse_chain");

    let mut event = claude_model_test_event(&current_transcript, "subagent");
    event
        .as_object_mut()
        .expect("event should be an object")
        .insert("agent_id".to_string(), json!("subagent-1"));
    persist_diff_trace_payload_to_agent_trace_db_with_db(&db, &parsed_claude_diff_trace(&event))
        .expect("subagent diff trace should persist");

    assert_eq!(persisted_model_ids(&db), vec![None]);
    assert!(
        db.claude_model_state_by_session_and_agent("cc_session-123", "subagent-1")
            .expect("lookup should succeed")
            .is_none(),
        "subagent scope must not inherit main-session chain state"
    );

    drop(db);
    fs::remove_dir_all(&dir).expect("test DB directory should be removed");
}

#[test]
fn prefixed_diff_trace_session_id_prefixes_fresh_pi_session_id() {
    assert_eq!(
        prefixed_diff_trace_session_id("pi", "session-123"),
        "pi_session-123"
    );
}

#[test]
fn prefixed_diff_trace_session_id_keeps_already_prefixed_pi_session_id() {
    assert_eq!(
        prefixed_diff_trace_session_id("pi", "pi_session-123"),
        "pi_session-123"
    );
}

#[test]
fn prefixed_diff_trace_session_id_prefixes_fresh_codex_session_id() {
    assert_eq!(
        prefixed_diff_trace_session_id("codex", "session-123"),
        "cx_session-123"
    );
}

#[test]
fn prefixed_diff_trace_session_id_keeps_already_prefixed_codex_session_id() {
    assert_eq!(
        prefixed_diff_trace_session_id("codex", "cx_session-123"),
        "cx_session-123"
    );
}

#[test]
fn prefixed_diff_trace_session_id_adding_codex_does_not_affect_other_tool_prefixes() {
    assert_eq!(
        prefixed_diff_trace_session_id("opencode", "session-123"),
        "oc_session-123"
    );
    assert_eq!(
        prefixed_diff_trace_session_id("claude", "session-123"),
        "cc_session-123"
    );
    assert_eq!(
        prefixed_diff_trace_session_id("pi", "session-123"),
        "pi_session-123"
    );
}

#[test]
fn normalize_codex_model_id_preserves_fresh_model_id() {
    assert_eq!(
        normalize_codex_model_id("gpt-5.6-codex").as_deref(),
        Some("gpt-5.6-codex")
    );
}

#[test]
fn normalize_codex_model_id_preserves_qualified_model_ids() {
    for model in ["openai/gpt-x", "qualified/custom-provider/model"] {
        assert_eq!(normalize_codex_model_id(model).as_deref(), Some(model));
    }
}

#[test]
fn normalize_codex_model_id_preserves_unqualified_model_ids() {
    assert_eq!(
        normalize_codex_model_id("custom-codex-model").as_deref(),
        Some("custom-codex-model")
    );
}

#[test]
fn normalize_codex_model_id_returns_none_for_blank_model_ids() {
    assert_eq!(normalize_codex_model_id("   "), None);
}

#[test]
fn normalize_opencode_model_id_preserves_qualified_and_unqualified_ids() {
    for model in [
        "opencode/big-pickle",
        "anthropic/claude-sonnet-4",
        "custom-model",
    ] {
        assert_eq!(normalize_opencode_model_id(model).as_deref(), Some(model));
    }
    assert_eq!(
        normalize_opencode_model_id("  opencode/big-pickle  ").as_deref(),
        Some("opencode/big-pickle")
    );
}

#[test]
fn normalize_opencode_model_id_returns_none_for_blank_model_ids() {
    assert_eq!(normalize_opencode_model_id(""), None);
    assert_eq!(normalize_opencode_model_id("   "), None);
}

#[test]
fn pi_normalized_diff_trace_payload_persists_with_pi_prefixed_session_id() {
    let stdin_payload = serde_json::json!({
        "sessionID": "session-123",
        "diff": "diff text",
        "time": 1_800_000_000_000_u64,
        "model_id": "anthropic/claude-opus-4",
        "tool_name": "pi",
        "tool_version": null
    })
    .to_string();

    let parsed = parse_diff_trace_payload(&stdin_payload)
        .expect("normalized Pi diff-trace payload should parse");
    let payload = match parsed {
        DiffTraceParseResult::Persist(payload) => payload,
        DiffTraceParseResult::NoOp(message) => {
            panic!("Pi payload should persist, got no-op: {message}")
        }
    };

    assert_eq!(payload.tool_name, "pi");
    assert_eq!(payload.model_id.as_deref(), Some("anthropic/claude-opus-4"));
    assert_eq!(payload.tool_version, None);

    persist_diff_trace_payload_to_agent_trace_db_with(
        &payload,
        payload.model_id.as_deref(),
        payload.tool_version.as_deref(),
        |input| {
            assert_eq!(input.time_ms, 1_800_000_000_000_i64);
            assert_eq!(input.session_id, "pi_session-123");
            assert_eq!(input.model_id, Some("anthropic/claude-opus-4"));
            assert_eq!(input.tool_name, "pi");
            assert_eq!(input.tool_version, None);
            assert_eq!(input.payload_type, PAYLOAD_TYPE_PATCH);

            Ok(())
        },
    )
    .expect("Pi diff-trace payload should be persisted");
}

#[test]
fn post_commit_intersection_flow_preserves_pi_provenance() {
    let now_ms = 1_800_000_000_000_i64;
    let commit_time_ms = now_ms - 1_000;

    let output = run_post_commit_intersection_flow_with(
        Path::new("/repo"),
        |_| {
            Ok(PostCommitPatchData {
                commit_oid: String::from("def456"),
                commit_time_ms,
                parsed_patch: valid_patch("src/lib.rs", "shared line"),
            })
        },
        || Ok(now_ms),
        |_, _| {
            Ok(RecentDiffTracePatches {
                patches: vec![ParsedDiffTracePatch {
                    id: 9,
                    time_ms: now_ms - 500,
                    session_id: String::from("pi_valid-session"),
                    patch: valid_patch("src/lib.rs", "shared line"),
                    tool_name: Some(String::from("pi")),
                    tool_version: None,
                    payload_type: String::from(PAYLOAD_TYPE_PATCH),
                }],
                skipped: vec![],
            })
        },
        |_| Ok(()),
    )
    .expect("post-commit intersection flow should succeed");

    assert_eq!(output.combined_recent_patch.files.len(), 1);
    assert_eq!(output.tool_name, Some(String::from("pi")));
    assert_eq!(output.tool_version, None);
}

#[test]
fn diff_trace_db_persistence_uses_direct_payload_model_and_tool_version() {
    let payload = diff_trace_payload(Some("direct-model"), None);

    persist_diff_trace_payload_to_agent_trace_db_with(
        &payload,
        Some("direct-model"),
        Some("Claude Code 1.2.3"),
        |input| {
            assert_eq!(input.time_ms, 1_800_000_000_000_i64);
            assert_eq!(input.session_id, "cc_session-123");
            assert_eq!(input.model_id, Some("direct-model"));
            assert_eq!(input.tool_name, "claude");
            assert_eq!(input.tool_version, Some("Claude Code 1.2.3"));
            assert_eq!(input.payload_type, PAYLOAD_TYPE_STRUCTURED);

            Ok(())
        },
    )
    .expect("direct diff-trace attribution should be persisted");
}

#[test]
fn post_commit_intersection_flow_uses_same_window_end_for_query_and_persistence() {
    let now_ms = 1_800_000_000_000_i64;
    let commit_time_ms = now_ms - 1_000;
    let expected_cutoff_ms = now_ms - RECENT_DAYS_MILLIS;
    let query_window = RefCell::new(None);
    let persisted = RefCell::new(None);

    let output = run_post_commit_intersection_flow_with(
        Path::new("/repo"),
        |_| {
            Ok(PostCommitPatchData {
                commit_oid: String::from("abc123"),
                commit_time_ms,
                parsed_patch: valid_patch("src/lib.rs", "shared line"),
            })
        },
        || Ok(now_ms),
        |cutoff_ms, end_ms| {
            *query_window.borrow_mut() = Some((cutoff_ms, end_ms));

            Ok(RecentDiffTracePatches {
                patches: vec![ParsedDiffTracePatch {
                    id: 7,
                    time_ms: now_ms - 500,
                    session_id: String::from("oc_valid-session"),
                    patch: valid_patch("src/lib.rs", "shared line"),
                    tool_name: Some(String::from("opencode")),
                    tool_version: Some(String::from("1.2.3")),
                    payload_type: String::from(PAYLOAD_TYPE_PATCH),
                }],
                skipped: vec![SkippedDiffTracePatch {
                    id: 8,
                    time_ms: now_ms - 250,
                    session_id: String::from("oc_malformed-session"),
                    reason: String::from("invalid hunk header"),
                }],
            })
        },
        |insert_input| {
            *persisted.borrow_mut() = Some(CapturedPostCommitIntersectionInsert {
                commit_id: insert_input.commit_id.to_string(),
                post_commit_time_ms: insert_input.post_commit_time_ms,
                recent_window_cutoff_ms: insert_input.recent_window_cutoff_ms,
                recent_window_end_ms: insert_input.recent_window_end_ms,
                loaded_diff_trace_count: insert_input.loaded_diff_trace_count,
                skipped_diff_trace_count: insert_input.skipped_diff_trace_count,
                intersection_patch: insert_input.intersection_patch.to_string(),
            });

            Ok(())
        },
    )
    .expect("post-commit intersection flow should succeed");

    assert_eq!(
        query_window.into_inner(),
        Some((expected_cutoff_ms, now_ms))
    );

    let persisted = persisted
        .into_inner()
        .expect("intersection row should be persisted");
    assert_eq!(persisted.commit_id, "abc123");
    assert_eq!(persisted.post_commit_time_ms, commit_time_ms);
    assert_eq!(persisted.recent_window_cutoff_ms, expected_cutoff_ms);
    assert_eq!(persisted.recent_window_end_ms, now_ms);
    assert_eq!(persisted.loaded_diff_trace_count, 1);
    assert_eq!(persisted.skipped_diff_trace_count, 1);

    let intersection: ParsedPatch = serde_json::from_str(&persisted.intersection_patch)
        .expect("persisted intersection patch should deserialize");
    assert_eq!(intersection.files.len(), 1);
    assert_eq!(intersection.files[0].new_path, "src/lib.rs");
    assert_eq!(intersection.files[0].hunks[0].lines.len(), 1);
    assert_eq!(
        intersection.files[0].hunks[0].lines[0].content,
        "shared line"
    );

    assert_eq!(output.post_commit_data.commit_oid, "abc123");
    assert_eq!(output.post_commit_data.commit_time_ms, commit_time_ms);
    assert_eq!(output.combined_recent_patch.files.len(), 1);
    assert_eq!(output.combined_recent_patch.files[0].new_path, "src/lib.rs");
    assert_eq!(output.tool_name, Some(String::from("opencode")));
    assert_eq!(output.tool_version, Some(String::from("1.2.3")));
}

fn post_commit_flow_result() -> PostCommitIntersectionFlowResult {
    PostCommitIntersectionFlowResult {
        combined_recent_patch: valid_patch("src/lib.rs", "shared line"),
        post_commit_data: PostCommitPatchData {
            commit_oid: String::from("abc123"),
            commit_time_ms: 1_800_000_000_000,
            parsed_patch: valid_patch("src/lib.rs", "shared line"),
        },
        tool_name: None,
        tool_version: None,
    }
}

fn minimal_agent_trace() -> AgentTrace {
    serde_json::from_value(json!({ "files": [] })).expect("minimal Agent Trace should deserialize")
}

#[test]
fn post_commit_auto_sync_launches_after_successful_persistence_when_enabled() {
    let events = RefCell::new(Vec::new());

    let output = run_post_commit_subcommand_with(
        Path::new("/repo"),
        None,
        "",
        |_| {
            events.borrow_mut().push("intersection");
            Ok(post_commit_flow_result())
        },
        |_, _, _, _| {
            events.borrow_mut().push("persistence");
            Ok(minimal_agent_trace())
        },
        |_| {
            events.borrow_mut().push("config");
            Ok(true)
        },
        |_| {
            events.borrow_mut().push("launch");
            Ok(())
        },
        |_| {
            events.borrow_mut().push("checkpoint");
            Ok(())
        },
        None,
    )
    .expect("successful post-commit should remain successful");

    assert!(output.contains("post-commit hook processed intersection"));
    assert_eq!(
        events.into_inner(),
        vec![
            "intersection",
            "persistence",
            "checkpoint",
            "config",
            "launch"
        ]
    );
}

#[test]
fn post_commit_validation_failure_does_not_resolve_or_launch_auto_sync() {
    let validation_called = RefCell::new(false);
    let config_called = RefCell::new(false);
    let launch_called = RefCell::new(false);

    let error = run_post_commit_subcommand_with(
        Path::new("/repo"),
        None,
        "",
        |_| Ok(post_commit_flow_result()),
        |_, flow_result, vcs_type, remote_url| {
            run_post_commit_agent_trace_flow_with(
                flow_result,
                vcs_type,
                remote_url,
                &ParsedPatch { files: Vec::new() },
                |_| {
                    *validation_called.borrow_mut() = true;
                    Err(anyhow!("Agent Trace validation failed"))
                },
                |_| panic!("Agent Trace persistence must not run after validation failure"),
            )
        },
        |_| {
            *config_called.borrow_mut() = true;
            Ok(true)
        },
        |_| {
            *launch_called.borrow_mut() = true;
            Ok(())
        },
        |_| panic!("checkpoint must not run after persistence failure"),
        None,
    )
    .expect_err("validation failure should be returned");

    assert!(*validation_called.borrow());
    assert!(!error.to_string().is_empty());
    assert!(!*config_called.borrow());
    assert!(!*launch_called.borrow());
}

fn post_commit_flow_result_for(
    direct: ParsedPatch,
    committed: ParsedPatch,
) -> PostCommitIntersectionFlowResult {
    PostCommitIntersectionFlowResult {
        combined_recent_patch: direct,
        post_commit_data: PostCommitPatchData {
            commit_oid: String::from("abc123"),
            commit_time_ms: 1_800_000_000_000,
            parsed_patch: committed,
        },
        tool_name: Some(String::from("claude")),
        tool_version: Some(String::from("9.9.9")),
    }
}

fn persisted_post_commit_trace(
    flow_result: &PostCommitIntersectionFlowResult,
    mutation_ai_patch: &ParsedPatch,
) -> Value {
    let persisted = RefCell::new(None);

    run_post_commit_agent_trace_flow_with(
        flow_result,
        Some(AgentTraceVcsType::Git),
        "",
        mutation_ai_patch,
        |_| Ok(()),
        |insert| {
            *persisted.borrow_mut() = Some(insert.trace_json.to_string());
            Ok(())
        },
    )
    .expect("post-commit Agent Trace flow should build and persist");

    serde_json::from_str(
        persisted
            .into_inner()
            .expect("trace should have been persisted")
            .as_str(),
    )
    .expect("persisted trace JSON should parse")
}

#[test]
fn post_commit_agent_trace_flow_attributes_mutation_only_lines_as_ai_without_provenance() {
    let flow_result = post_commit_flow_result_for(
        ParsedPatch { files: Vec::new() },
        valid_patch("src/lib.rs", "mutated line"),
    );
    let mutation_ai_patch = valid_patch("src/lib.rs", "mutated line");

    let trace = persisted_post_commit_trace(&flow_result, &mutation_ai_patch);

    assert_eq!(
        trace["metadata"]["sce"]["line_changes"]["ai"]["added"],
        json!(1)
    );
    assert_eq!(
        trace["metadata"]["sce"]["line_changes"]["unknown"]["added"],
        json!(0)
    );
    assert!(
        trace.get("tool").is_none(),
        "mutation-only coverage fabricates no tool provenance"
    );
    let contributor = &trace["files"][0]["conversations"][0]["contributor"];
    assert_eq!(contributor["type"], json!("ai"));
    assert!(
        contributor.get("model_id").is_none(),
        "mutation-only coverage carries no model provenance"
    );
    assert!(
        trace["files"][0]["conversations"][0]
            .get("related")
            .is_none(),
        "mutation-only coverage carries no session provenance"
    );
}

#[test]
fn post_commit_agent_trace_flow_keeps_direct_provenance_when_direct_covers_the_line() {
    let flow_result = post_commit_flow_result_for(
        valid_patch("src/lib.rs", "shared line"),
        valid_patch("src/lib.rs", "shared line"),
    );

    let trace = persisted_post_commit_trace(&flow_result, &ParsedPatch { files: Vec::new() });

    assert_eq!(
        trace["metadata"]["sce"]["line_changes"]["ai"]["added"],
        json!(1)
    );
    assert_eq!(
        trace["tool"],
        json!({ "name": "claude", "version": "9.9.9" })
    );
    assert_eq!(
        trace["files"][0]["conversations"][0]["contributor"]["type"],
        json!("ai")
    );
}

#[test]
fn post_commit_agent_trace_flow_with_empty_mutation_patch_leaves_uncovered_lines_unknown() {
    let flow_result = post_commit_flow_result_for(
        ParsedPatch { files: Vec::new() },
        valid_patch("src/lib.rs", "human line"),
    );

    let trace = persisted_post_commit_trace(&flow_result, &ParsedPatch { files: Vec::new() });

    assert_eq!(
        trace["metadata"]["sce"]["line_changes"]["unknown"]["added"],
        json!(1)
    );
    assert_eq!(
        trace["metadata"]["sce"]["line_changes"]["ai"]["added"],
        json!(0)
    );
    assert!(trace.get("tool").is_none());
    assert_eq!(
        trace["files"][0]["conversations"][0]["contributor"]["type"],
        json!("unknown")
    );
}

mod mutation_attribution_e2e {
    use super::*;
    use crate::services::mutation_trace::runtime::resolve_post_commit_mutation_ai_patch;
    use crate::services::mutation_trace::runtime::resolve_worktree_id;
    use crate::services::mutation_trace::store::encode_revision;

    fn git(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("git should spawn");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("git output should be UTF-8")
    }

    fn commit_all(repo: &Path, message: &str) {
        git(repo, &["add", "-A"]);
        git(
            repo,
            &[
                "-c",
                "user.name=SCE Test",
                "-c",
                "user.email=sce@example.invalid",
                "commit",
                "-qm",
                message,
            ],
        );
    }

    struct E2eRepo {
        _temp: tempfile::TempDir,
        root: PathBuf,
        db_path: PathBuf,
    }

    impl E2eRepo {
        fn new(label: &str) -> Self {
            let temp = tempfile::Builder::new()
                .prefix(&format!("sce-mutation-attr-e2e-{label}-"))
                .tempdir()
                .expect("temp dir should be created");
            let root = temp.path().join("repo");
            fs::create_dir_all(&root).expect("repo dir should be created");
            git(&root, &["init", "-q"]);
            git(
                &root,
                &["remote", "add", "origin", "git@github.com:acme/widgets.git"],
            );
            fs::write(root.join("file.rs"), "one\n").expect("seed file should write");
            commit_all(&root, "base");
            let db_path = temp.path().join("agent-trace.db");
            RepositoryAgentTraceDb::new_at(&db_path)
                .expect("repository DB should open with schema");
            Self {
                _temp: temp,
                root,
                db_path,
            }
        }

        fn db(&self) -> RepositoryAgentTraceDb {
            RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&self.db_path)
                .expect("repository DB should reopen")
        }

        fn head_tree(&self) -> String {
            git(&self.root, &["rev-parse", "HEAD^{tree}"])
                .trim()
                .to_owned()
        }

        fn parent_tree(&self) -> String {
            git(&self.root, &["rev-parse", "HEAD~1^{tree}"])
                .trim()
                .to_owned()
        }

        fn checkout_id(&self) -> String {
            resolve_worktree_id(&self.root)
                .expect("worktree identity should resolve")
                .0
        }
    }

    fn seed_event(
        db: &RepositoryAgentTraceDb,
        worktree_id: &str,
        revision: u64,
        before_tree: &str,
        after_tree: &str,
        attribution_kind: &str,
        attribution_scope_id: Option<&str>,
    ) {
        db.execute(
            "INSERT INTO mutation_trace_events
                    (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
                     attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id,
                     boundary_event_id)
                 VALUES (?1, ?2, ?3, ?4, 0, 'healthy', ?5, ?6, 'flush', NULL, NULL)",
            (
                worktree_id,
                encode_revision(revision).as_slice(),
                before_tree,
                after_tree,
                attribution_kind,
                attribution_scope_id,
            ),
        )
        .expect("mutation event insert should succeed");
    }

    fn row_count(db: &RepositoryAgentTraceDb, table: &str) -> i64 {
        db.query_map(&format!("SELECT COUNT(*) FROM {table}"), (), |row| {
            row.get::<i64>(0).map_err(anyhow::Error::from)
        })
        .expect("count query should succeed")
        .into_iter()
        .next()
        .expect("count row should exist")
    }

    fn touched_line_count(patch: &ParsedPatch) -> usize {
        patch
            .files
            .iter()
            .flat_map(|file| file.hunks.iter())
            .map(|hunk| hunk.lines.len())
            .sum()
    }

    fn flow_result_for(repo: &E2eRepo, direct: ParsedPatch) -> PostCommitIntersectionFlowResult {
        let post_commit_data = capture_post_commit_patch_from_git(&repo.root)
            .expect("capturing the post-commit patch should succeed");
        PostCommitIntersectionFlowResult {
            combined_recent_patch: direct,
            post_commit_data,
            tool_name: None,
            tool_version: None,
        }
    }

    fn resolve_mutation_ai(
        repo: &E2eRepo,
        db: &RepositoryAgentTraceDb,
        flow_result: &PostCommitIntersectionFlowResult,
    ) -> ParsedPatch {
        let direct_intersection = intersect_patches_fn(
            &flow_result.combined_recent_patch,
            &flow_result.post_commit_data.parsed_patch,
        );
        resolve_post_commit_mutation_ai_patch(
            &repo.root,
            db,
            &direct_intersection,
            &flow_result.post_commit_data.parsed_patch,
        )
    }

    fn persist_trace(
        flow_result: &PostCommitIntersectionFlowResult,
        db: &RepositoryAgentTraceDb,
        mutation_ai_patch: &ParsedPatch,
    ) -> Value {
        let persisted = RefCell::new(None);
        run_post_commit_agent_trace_flow_with(
            flow_result,
            Some(AgentTraceVcsType::Git),
            "git@github.com:acme/widgets.git",
            mutation_ai_patch,
            |value| validate_agent_trace_value(value).map_err(|error| anyhow!(error.to_string())),
            |insert| {
                *persisted.borrow_mut() = Some(insert.trace_json.to_string());
                db.insert_agent_trace(insert).map(|_| ())
            },
        )
        .expect("the post-commit Agent Trace flow should build, validate, and persist");

        serde_json::from_str(
            persisted
                .into_inner()
                .expect("a trace should have been persisted")
                .as_str(),
        )
        .expect("the persisted trace JSON should parse")
    }

    #[test]
    fn a_mutation_only_line_persists_as_ai_without_fabricated_provenance() {
        let repo = E2eRepo::new("mutation-only");
        fs::write(repo.root.join("file.rs"), "one\ntwo\n").expect("the edit should write");
        commit_all(&repo.root, "add two");

        let db = repo.db();
        seed_event(
            &db,
            &repo.checkout_id(),
            1,
            &repo.parent_tree(),
            &repo.head_tree(),
            "ai_exclusive",
            Some("scope-x"),
        );

        let flow_result = flow_result_for(&repo, ParsedPatch { files: Vec::new() });
        let mutation_ai_patch = resolve_mutation_ai(&repo, &db, &flow_result);
        assert_eq!(
            touched_line_count(&mutation_ai_patch),
            1,
            "a healthy untainted exclusive event covers the committed line"
        );

        let trace = persist_trace(&flow_result, &db, &mutation_ai_patch);
        assert_eq!(
            trace["metadata"]["sce"]["line_changes"]["ai"]["added"],
            json!(1)
        );
        assert_eq!(
            trace["metadata"]["sce"]["line_changes"]["unknown"]["added"],
            json!(0)
        );
        assert!(
            trace.get("tool").is_none(),
            "mutation-only coverage fabricates no tool provenance"
        );
        let contributor = &trace["files"][0]["conversations"][0]["contributor"];
        assert_eq!(contributor["type"], json!("ai"));
        assert!(
            contributor.get("model_id").is_none(),
            "mutation-only coverage carries no model provenance"
        );

        assert_eq!(
            row_count(&db, "diff_traces"),
            0,
            "mutation evidence is never inserted into diff_traces"
        );
        assert_eq!(
            row_count(&db, "post_commit_patch_intersections"),
            0,
            "the direct-only intersection table is untouched by this flow"
        );
        assert_eq!(row_count(&db, "agent_traces"), 1);
    }

    #[test]
    fn direct_plus_mutation_evidence_completes_hunk_coverage_and_keeps_direct_provenance() {
        let repo = E2eRepo::new("direct-plus-mutation");
        fs::write(repo.root.join("file.rs"), "one\ntwo\nthree\n").expect("the edit should write");
        commit_all(&repo.root, "add two and three");

        let db = repo.db();
        seed_event(
            &db,
            &repo.checkout_id(),
            1,
            &repo.parent_tree(),
            &repo.head_tree(),
            "ai_exclusive",
            Some("scope-x"),
        );

        let direct = parse_patch_from_text(
                "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1,1 +1,2 @@\n one\n+two\n",
                None,
            )
            .expect("the direct patch should parse");
        let mut flow_result = flow_result_for(&repo, direct);
        flow_result.tool_name = Some(String::from("claude"));
        flow_result.tool_version = Some(String::from("9.9.9"));

        let mutation_ai_patch = resolve_mutation_ai(&repo, &db, &flow_result);
        assert_eq!(
            touched_line_count(&mutation_ai_patch),
            1,
            "only the line direct evidence did not cover is resolved from mutation history"
        );

        let trace = persist_trace(&flow_result, &db, &mutation_ai_patch);
        assert_eq!(
            trace["metadata"]["sce"]["line_changes"]["ai"]["added"],
            json!(2),
            "the union of direct and mutation coverage classifies the hunk ai"
        );
        assert_eq!(
            trace["metadata"]["sce"]["line_changes"]["unknown"]["added"],
            json!(0)
        );
        assert_eq!(
            trace["tool"],
            json!({ "name": "claude", "version": "9.9.9" })
        );
    }

    #[test]
    fn a_newer_nonexclusive_event_keeps_the_line_non_ai() {
        let repo = E2eRepo::new("newer-nonexclusive");
        fs::write(repo.root.join("file.rs"), "one\ntwo\n").expect("the edit should write");
        commit_all(&repo.root, "add two");

        let db = repo.db();
        let worktree = repo.checkout_id();
        seed_event(
            &db,
            &worktree,
            1,
            &repo.parent_tree(),
            &repo.head_tree(),
            "ai_exclusive",
            Some("scope-old"),
        );
        seed_event(
            &db,
            &worktree,
            2,
            &repo.parent_tree(),
            &repo.head_tree(),
            "ai_contended",
            None,
        );

        let flow_result = flow_result_for(&repo, ParsedPatch { files: Vec::new() });
        let mutation_ai_patch = resolve_mutation_ai(&repo, &db, &flow_result);
        assert_eq!(
            touched_line_count(&mutation_ai_patch),
            0,
            "the newer contended match resolves the line and blocks the older exclusive event"
        );

        let trace = persist_trace(&flow_result, &db, &mutation_ai_patch);
        assert_eq!(
            trace["metadata"]["sce"]["line_changes"]["unknown"]["added"],
            json!(1)
        );
        assert_eq!(
            trace["metadata"]["sce"]["line_changes"]["ai"]["added"],
            json!(0)
        );
        assert_eq!(
            trace["files"][0]["conversations"][0]["contributor"]["type"],
            json!("unknown")
        );
    }

    #[test]
    fn an_adversarial_foreign_worktree_event_cannot_block_the_current_worktrees_exclusive_event() {
        let repo = E2eRepo::new("adversarial-linked");

        let linked_root = repo
            .root
            .parent()
            .expect("the repo should have a parent directory")
            .join("linked");
        git(
            &repo.root,
            &[
                "worktree",
                "add",
                "-q",
                linked_root.to_str().expect("worktree path should be UTF-8"),
            ],
        );

        fs::write(repo.root.join("file.rs"), "one\ntwo\n").expect("the edit should write");
        commit_all(&repo.root, "add two");

        let db = repo.db();
        let current_worktree = repo.checkout_id();
        let foreign_worktree = resolve_worktree_id(&linked_root)
            .expect("the linked worktree's identity should resolve")
            .0;
        assert_ne!(
            current_worktree, foreign_worktree,
            "the linked worktree must derive its own distinct identity"
        );

        seed_event(
            &db,
            &current_worktree,
            1,
            &repo.parent_tree(),
            &repo.head_tree(),
            "ai_exclusive",
            Some("scope-current"),
        );
        seed_event(
            &db,
            &foreign_worktree,
            2,
            &repo.parent_tree(),
            &repo.head_tree(),
            "ai_contended",
            None,
        );

        let flow_result = flow_result_for(&repo, ParsedPatch { files: Vec::new() });
        let mutation_ai_patch = resolve_mutation_ai(&repo, &db, &flow_result);
        assert_eq!(
                touched_line_count(&mutation_ai_patch),
                1,
                "only the current worktree's history is eligible, so the older exclusive event contributes"
            );

        let trace = persist_trace(&flow_result, &db, &mutation_ai_patch);
        assert_eq!(
            trace["metadata"]["sce"]["line_changes"]["ai"]["added"],
            json!(1),
            "worktree isolation lets the current worktree's exclusive event classify the target ai"
        );
        assert_eq!(
            trace["files"][0]["conversations"][0]["contributor"]["type"],
            json!("ai")
        );
        assert!(trace.get("tool").is_none());
    }

    fn touched_contents(patch: &ParsedPatch) -> Vec<String> {
        patch
            .files
            .iter()
            .flat_map(|file| file.hunks.iter())
            .flat_map(|hunk| hunk.lines.iter())
            .map(|line| line.content.clone())
            .collect()
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn persistence_boundaries_stay_separated_across_diff_traces_intersection_and_agent_trace() {
        let repo = E2eRepo::new("persistence-boundary");

        fs::write(repo.root.join("file.rs"), "one\ntwo\n").expect("the direct edit should write");
        git(&repo.root, &["add", "-A"]);
        let intermediate_tree = git(&repo.root, &["write-tree"]).trim().to_owned();

        fs::write(repo.root.join("file.rs"), "one\ntwo\nthree\n")
            .expect("the mutation edit should write");
        commit_all(&repo.root, "add two and three");

        let base_tree = repo.parent_tree();
        let final_tree = repo.head_tree();
        assert_ne!(
            base_tree, intermediate_tree,
            "the direct edit must move the tree"
        );
        assert_ne!(
            intermediate_tree, final_tree,
            "the mutation edit must move the tree again"
        );

        let db = repo.db();

        let now_ms = current_unix_time_ms().expect("the clock should resolve");
        db.insert_diff_trace(DiffTraceInsert {
                time_ms: now_ms - 60_000,
                session_id: "cc_session-direct",
                patch: "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1,1 +1,2 @@\n one\n+two\n",
                model_id: Some("claude/model-direct"),
                tool_name: "claude",
                tool_version: Some("9.9.9"),
                payload_type: PAYLOAD_TYPE_PATCH,
            })
            .expect("the direct diff_traces row should insert");

        seed_event(
            &db,
            &repo.checkout_id(),
            1,
            &intermediate_tree,
            &final_tree,
            "ai_exclusive",
            Some("scope-mutation"),
        );

        let flow_result = run_post_commit_intersection_flow_with(
            &repo.root,
            capture_post_commit_patch_from_git,
            current_unix_time_ms,
            |cutoff_ms, end_ms| db.recent_diff_trace_patches(cutoff_ms, end_ms),
            |insert| db.insert_post_commit_patch_intersection(insert).map(|_| ()),
        )
        .expect("the real post-commit intersection flow should run");
        assert_eq!(
                touched_contents(&flow_result.combined_recent_patch),
                vec!["two".to_owned()],
                "the combined recent patch comes from the real diff_traces query, not an in-memory patch"
            );

        let mutation_ai_patch = resolve_mutation_ai(&repo, &db, &flow_result);
        assert_eq!(
            touched_contents(&mutation_ai_patch),
            vec!["three".to_owned()],
            "mutation history resolves only the committed line direct evidence missed"
        );

        persist_trace(&flow_result, &db, &mutation_ai_patch);

        assert_eq!(
            row_count(&db, "diff_traces"),
            1,
            "mutation attribution must not create another diff_traces row"
        );
        let stored_direct_patch: String = db
            .query_map("SELECT patch FROM diff_traces", (), |row| {
                row.get::<String>(0).map_err(anyhow::Error::from)
            })
            .expect("diff_traces query should succeed")
            .into_iter()
            .next()
            .expect("one diff_traces row should exist");
        let stored_direct = parse_patch_from_text(&stored_direct_patch, None)
            .expect("the stored direct patch should parse");
        assert_eq!(
            touched_contents(&stored_direct),
            vec!["two".to_owned()],
            "the direct diff_traces row contains 'two' and never 'three'"
        );

        assert_eq!(
            row_count(&db, "post_commit_patch_intersections"),
            1,
            "the intersection flow persists exactly one direct-only row"
        );
        let stored_intersection_json: String = db
            .query_map(
                "SELECT intersection_patch FROM post_commit_patch_intersections",
                (),
                |row| row.get::<String>(0).map_err(anyhow::Error::from),
            )
            .expect("intersection query should succeed")
            .into_iter()
            .next()
            .expect("one intersection row should exist");
        let stored_intersection = load_patch_from_json(&stored_intersection_json)
            .expect("the persisted intersection patch should reconstruct");
        assert_eq!(
            touched_contents(&stored_intersection),
            vec!["two".to_owned()],
            "post_commit_patch_intersections stays direct-only; the mutation line 'three' \
                 must never contaminate this table"
        );

        assert_eq!(row_count(&db, "agent_traces"), 1);
        let stored_trace_json: String = db
            .query_map("SELECT trace_json FROM agent_traces", (), |row| {
                row.get::<String>(0).map_err(anyhow::Error::from)
            })
            .expect("agent_traces query should succeed")
            .into_iter()
            .next()
            .expect("one Agent Trace row should exist");
        let trace: Value = serde_json::from_str(&stored_trace_json)
            .expect("the persisted Agent Trace JSON should parse");
        validate_agent_trace_value(&trace).expect(
                "the persisted agent_traces.trace_json validates against the embedded Agent Trace schema",
            );
        assert_eq!(
            trace["metadata"]["sce"]["line_changes"]["ai"]["added"],
            json!(2),
            "direct + mutation coverage classifies both committed added lines as ai"
        );
        assert_eq!(
            trace["metadata"]["sce"]["line_changes"]["unknown"]["added"],
            json!(0)
        );
        assert_eq!(
            trace["files"][0]["conversations"][0]["contributor"]["type"],
            json!("ai")
        );

        assert_eq!(
            trace["tool"],
            json!({ "name": "claude", "version": "9.9.9" })
        );

        assert_eq!(
            row_count(&db, "mutation_trace_events"),
            1,
            "attribution performs no mutation-cursor write"
        );
    }
}

mod mutation_provenance_e2e {
    use super::*;
    use crate::services::agent_trace_db::{ClaudeModelStateObservation, ObservationKind};
    use crate::services::agent_trace_storage::{
        resolve_agent_trace_storage_at_state_root, AgentTraceStorageContext,
    };
    use crate::services::hooks::claude_mutation_scope;
    use crate::services::hooks::codex_mutation_scope;
    use crate::services::hooks::opencode_mutation_scope;
    use crate::services::hooks::pi_mutation_scope;
    use crate::services::mutation_trace::runtime::resolve_git_dir;
    use crate::services::mutation_trace::runtime::resolve_post_commit_mutation_ai_patch;

    fn git(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("git should spawn");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("git output should be UTF-8")
    }

    fn row_count(db: &RepositoryAgentTraceDb, table: &str) -> i64 {
        db.query_map(&format!("SELECT COUNT(*) FROM {table}"), (), |row| {
            row.get::<i64>(0).map_err(anyhow::Error::from)
        })
        .expect("count query should succeed")
        .into_iter()
        .next()
        .expect("count row should exist")
    }

    struct ProvenanceE2eRepo {
        _temp: tempfile::TempDir,
        root: PathBuf,
        state_root: PathBuf,
        db_path: PathBuf,
    }

    impl ProvenanceE2eRepo {
        fn new(label: &str) -> Self {
            let temp = tempfile::Builder::new()
                .prefix(&format!("sce-mutation-provenance-e2e-{label}-"))
                .tempdir()
                .expect("temp dir should be created");
            let root = temp.path().join("repo");
            fs::create_dir_all(&root).expect("repo dir should be created");
            git(&root, &["init", "-q"]);
            git(&root, &["config", "user.email", "test@example.invalid"]);
            git(&root, &["config", "user.name", "SCE Test"]);
            git(
                &root,
                &["remote", "add", "origin", "git@github.com:acme/widgets.git"],
            );
            fs::write(root.join("file.txt"), "one\n").expect("seed file should write");
            git(&root, &["add", "-A"]);
            git(&root, &["commit", "-qm", "base"]);

            let state_root = temp.path().join("state");
            fs::create_dir_all(&state_root).expect("state root should be created");
            let storage = resolve_agent_trace_storage_at_state_root(
                &AgentTraceStorageContext {
                    repository_root: &root,
                    explicit_repository_id: None,
                    repository_remote: "origin",
                },
                &state_root,
            )
            .expect("state-root storage should initialize the repository DB");

            Self {
                _temp: temp,
                root,
                state_root,
                db_path: storage.db_path,
            }
        }

        fn db(&self) -> RepositoryAgentTraceDb {
            RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&self.db_path)
                .expect("repository DB should reopen")
        }

        fn cwd(&self) -> String {
            self.root.to_string_lossy().into_owned()
        }

        fn write_change(&self, content: &str) {
            fs::write(self.root.join("file.txt"), content).expect("mutation should write");
        }

        fn commit_change(&self) {
            git(&self.root, &["add", "-A"]);
            git(&self.root, &["commit", "-qm", "AI mutation"]);
        }

        fn mutation_events(&self) -> Vec<(String, Option<String>)> {
            self.db()
                .query_map(
                    "SELECT attribution_kind, attribution_scope_id \
                         FROM mutation_trace_events ORDER BY revision",
                    (),
                    |row| {
                        let attribution_kind = row.get::<String>(0).map_err(anyhow::Error::from)?;
                        let attribution_scope_id =
                            row.get::<Option<String>>(1).map_err(anyhow::Error::from)?;
                        Ok((attribution_kind, attribution_scope_id))
                    },
                )
                .expect("mutation-events query should succeed")
        }

        fn run_post_commit(&self) -> Value {
            let db = self.db();
            run_post_commit_subcommand_with(
                &self.root,
                Some(AgentTraceVcsType::Git),
                "git@github.com:acme/widgets.git",
                |root| {
                    run_post_commit_intersection_flow_with(
                        root,
                        capture_post_commit_patch_from_git,
                        current_unix_time_ms,
                        |cutoff_ms, end_ms| db.recent_diff_trace_patches(cutoff_ms, end_ms),
                        |insert| db.insert_post_commit_patch_intersection(insert).map(|_| ()),
                    )
                },
                |root, flow_result, vcs_type, remote_url| {
                    let direct_intersection = intersect_patches_fn(
                        &flow_result.combined_recent_patch,
                        &flow_result.post_commit_data.parsed_patch,
                    );
                    let mutation_ai_patch = resolve_post_commit_mutation_ai_patch(
                        root,
                        &db,
                        &direct_intersection,
                        &flow_result.post_commit_data.parsed_patch,
                    );

                    run_post_commit_agent_trace_flow_with(
                        flow_result,
                        vcs_type,
                        remote_url,
                        &mutation_ai_patch,
                        |value| {
                            validate_agent_trace_value(value)
                                .map_err(|error| anyhow!(error.to_string()))
                        },
                        |insert| db.insert_agent_trace(insert).map(|_| ()),
                    )
                },
                |_| Ok(false),
                |_| Ok(()),
                |_| db.passive_checkpoint(),
                None,
            )
            .expect("the real post-commit hook flow should persist Agent Trace");

            db.query_map("SELECT trace_json FROM agent_traces", (), |row| {
                row.get::<String>(0).map_err(anyhow::Error::from)
            })
            .expect("persisted Agent Trace should be readable")
            .into_iter()
            .next()
            .map(|trace| serde_json::from_str(&trace).expect("trace JSON should parse"))
            .expect("one Agent Trace row should exist")
        }
    }

    fn assert_mutation_trace_provenance(trace: &Value, model_id: &str, session_id: &str) {
        assert_eq!(trace["files"][0]["path"], json!("file.txt"));
        assert_eq!(
            trace["files"][0]["conversations"][0]["contributor"],
            json!({"type": "ai", "model_id": model_id})
        );
        assert_eq!(
            trace["files"][0]["conversations"][0]["related"],
            json!([{
                "type": "session",
                "url": format!("https://sce.crocoder.dev/sessions/{session_id}"),
            }])
        );
        assert_eq!(
            trace["metadata"]["sce"]["line_changes"]["ai"]["added"],
            json!(1)
        );
        assert_eq!(
            trace["metadata"]["sce"]["line_changes"]["unknown"]["added"],
            json!(0)
        );
    }

    fn opencode_before(
        cwd: &str,
        session_id: &str,
        call_id: &str,
        tool_name: &str,
        model: Option<&str>,
    ) -> String {
        let mut payload = json!({
            "hook_event_name": "ToolExecuteBefore",
            "session_id": session_id,
            "call_id": call_id,
            "cwd": cwd,
            "tool_name": tool_name,
        });
        if let Some(model) = model {
            payload["model"] = json!(model);
        }
        payload.to_string()
    }

    fn opencode_shell_env(
        cwd: &str,
        session_id: &str,
        call_id: &str,
        model: Option<&str>,
    ) -> String {
        let mut payload = json!({
            "hook_event_name": "ShellEnv",
            "session_id": session_id,
            "call_id": call_id,
            "cwd": cwd,
        });
        if let Some(model) = model {
            payload["model"] = json!(model);
        }
        payload.to_string()
    }

    fn opencode_after(cwd: &str, session_id: &str, call_id: &str, tool_name: &str) -> String {
        json!({
            "hook_event_name": "ToolExecuteAfter",
            "session_id": session_id,
            "call_id": call_id,
            "cwd": cwd,
            "tool_name": tool_name,
        })
        .to_string()
    }

    fn opencode_tool_error(cwd: &str, session_id: &str, call_id: &str, tool_name: &str) -> String {
        json!({
            "hook_event_name": "ToolError",
            "session_id": session_id,
            "call_id": call_id,
            "cwd": cwd,
            "tool_name": tool_name,
        })
        .to_string()
    }

    fn drive_opencode(repo: &ProvenanceE2eRepo, payload: &str) -> Result<String> {
        opencode_mutation_scope::run_opencode_mutation_scope_from_payload_at_state_root(
            &repo.state_root,
            payload,
            None,
        )
    }

    #[test]
    fn opencode_bash_mutation_persists_model_and_session_in_agent_trace() {
        let repo = ProvenanceE2eRepo::new("opencode-bash");
        let session_id = "ses_opencode_bash";
        let cwd = repo.cwd();

        drive_opencode(
            &repo,
            &opencode_shell_env(&cwd, session_id, "call_bash", Some("opencode/big-pickle")),
        )
        .expect("OpenCode shell.env should establish the bash scope");

        repo.write_change("one\nopencode bash mutation\n");

        drive_opencode(
            &repo,
            &opencode_after(&cwd, session_id, "call_bash", "bash"),
        )
        .expect("OpenCode ToolExecuteAfter should close the bash scope");
        repo.commit_change();

        let trace = repo.run_post_commit();
        assert_mutation_trace_provenance(&trace, "opencode/big-pickle", "oc_ses_opencode_bash");
        assert_eq!(row_count(&repo.db(), "diff_traces"), 0);
        assert_eq!(row_count(&repo.db(), "post_commit_patch_intersections"), 1);
        assert_eq!(row_count(&repo.db(), "mutation_trace_events"), 1);
        assert_eq!(row_count(&repo.db(), "agent_traces"), 1);
    }

    #[test]
    fn opencode_apply_patch_mutation_with_missing_model_persists_no_model_in_agent_trace() {
        let repo = ProvenanceE2eRepo::new("opencode-apply-patch");
        let session_id = "ses_opencode_no_model";
        let cwd = repo.cwd();

        drive_opencode(
            &repo,
            &opencode_before(&cwd, session_id, "call_patch", "apply_patch", None),
        )
        .expect("OpenCode ToolExecuteBefore should establish the apply_patch scope");

        repo.write_change("one\npatched without model evidence\n");

        drive_opencode(
            &repo,
            &opencode_after(&cwd, session_id, "call_patch", "apply_patch"),
        )
        .expect("OpenCode ToolExecuteAfter should close the apply_patch scope");
        repo.commit_change();

        let trace = repo.run_post_commit();
        assert_eq!(trace["files"][0]["path"], json!("file.txt"));
        let contributor = &trace["files"][0]["conversations"][0]["contributor"];
        assert_eq!(contributor["type"], json!("ai"));
        assert!(
            contributor.get("model_id").is_none(),
            "absent model evidence must never be guessed or fabricated"
        );
        assert_eq!(
            trace["files"][0]["conversations"][0]["related"],
            json!([{
                "type": "session",
                "url": "https://sce.crocoder.dev/sessions/oc_ses_opencode_no_model",
            }])
        );
    }

    #[test]
    fn opencode_write_mutation_persists_model_while_task_delegation_stays_zero_footprint() {
        let repo = ProvenanceE2eRepo::new("opencode-write-task");
        let session_id = "ses_opencode_write";
        let cwd = repo.cwd();

        drive_opencode(
            &repo,
            &opencode_before(&cwd, session_id, "call_task", "task", None),
        )
        .expect("task delegation ToolExecuteBefore is neutral");
        drive_opencode(
            &repo,
            &opencode_after(&cwd, session_id, "call_task", "task"),
        )
        .expect("task delegation ToolExecuteAfter is neutral");

        assert_eq!(
            row_count(&repo.db(), "mutation_trace_scopes"),
            0,
            "a delegation event must create no mutation scope"
        );

        drive_opencode(
            &repo,
            &opencode_before(
                &cwd,
                session_id,
                "call_write",
                "write",
                Some("opencode/big-pickle"),
            ),
        )
        .expect("OpenCode write ToolExecuteBefore should establish the scope");
        repo.write_change("one\nwrite mutation\n");
        drive_opencode(
            &repo,
            &opencode_after(&cwd, session_id, "call_write", "write"),
        )
        .expect("OpenCode write ToolExecuteAfter should close the scope");
        repo.commit_change();

        let trace = repo.run_post_commit();
        assert_mutation_trace_provenance(&trace, "opencode/big-pickle", "oc_ses_opencode_write");
        assert_eq!(
            row_count(&repo.db(), "mutation_trace_scopes"),
            1,
            "only the tracked write call created a scope"
        );
    }

    #[test]
    fn opencode_unknown_tool_events_create_no_scope_or_mutation_state() {
        let repo = ProvenanceE2eRepo::new("opencode-untracked");
        let session_id = "ses_opencode_untracked";
        let cwd = repo.cwd();

        for tool_name in ["read", "custom_mcp_tool", "totally_unknown_future_tool"] {
            let call_id = format!("call_{tool_name}");
            drive_opencode(
                &repo,
                &opencode_before(&cwd, session_id, &call_id, tool_name, None),
            )
            .unwrap_or_else(|_| panic!("{tool_name} ToolExecuteBefore should be neutral"));
            drive_opencode(
                &repo,
                &opencode_after(&cwd, session_id, &call_id, tool_name),
            )
            .unwrap_or_else(|_| panic!("{tool_name} ToolExecuteAfter should be neutral"));
            drive_opencode(
                &repo,
                &opencode_tool_error(&cwd, session_id, &call_id, tool_name),
            )
            .unwrap_or_else(|_| panic!("{tool_name} ToolError should be neutral"));
        }

        assert_eq!(row_count(&repo.db(), "mutation_trace_scopes"), 0);
        assert_eq!(row_count(&repo.db(), "mutation_trace_events"), 0);
    }

    #[test]
    fn opencode_child_task_session_gets_its_own_independent_scope_and_provenance() {
        let repo = ProvenanceE2eRepo::new("opencode-child-session");
        let parent_session = "ses_opencode_parent";
        let child_session = "ses_opencode_child";
        let cwd = repo.cwd();

        drive_opencode(
            &repo,
            &opencode_before(&cwd, parent_session, "call_task", "task", None),
        )
        .expect("the parent's task delegation is neutral");

        drive_opencode(
            &repo,
            &opencode_before(
                &cwd,
                child_session,
                "call_child_write",
                "write",
                Some("opencode/child-model"),
            ),
        )
        .expect("the child session's write ToolExecuteBefore should establish its own scope");
        repo.write_change("one\nchild session mutation\n");
        drive_opencode(
            &repo,
            &opencode_after(&cwd, child_session, "call_child_write", "write"),
        )
        .expect("the child session's write ToolExecuteAfter should close its own scope");
        repo.commit_change();

        let trace = repo.run_post_commit();
        assert_mutation_trace_provenance(&trace, "opencode/child-model", "oc_ses_opencode_child");
        assert_eq!(
            row_count(&repo.db(), "mutation_trace_scopes"),
            1,
            "the parent's task delegation created no scope; only the child session's write did"
        );
    }

    #[test]
    fn opencode_concurrent_reject_and_confirm_keeps_only_the_confirmed_mutation_ai() {
        let repo = ProvenanceE2eRepo::new("opencode-concurrent-reject");
        let session_id = "ses_opencode_concurrent";
        let cwd = repo.cwd();

        drive_opencode(
            &repo,
            &opencode_before(
                &cwd,
                session_id,
                "call_a_edit",
                "edit",
                Some("opencode/big-pickle"),
            ),
        )
        .expect("A's edit ToolExecuteBefore should establish a scope");
        drive_opencode(
            &repo,
            &opencode_before(
                &cwd,
                session_id,
                "call_b_write",
                "write",
                Some("opencode/big-pickle"),
            ),
        )
        .expect("B's write ToolExecuteBefore should establish a distinct concurrent scope");

        fs::write(repo.root.join("rejected.txt"), "rejected mutation\n")
            .expect("A's mutation should write");
        fs::write(
            repo.root.join("ambiguous.txt"),
            "B's mutation before recovery\n",
        )
        .expect("B's pre-recovery mutation should write");

        drive_opencode(
            &repo,
            &opencode_tool_error(&cwd, session_id, "call_a_edit", "edit"),
        )
        .expect("A's ToolError should abandon A's scope and consume the shared ambiguous interval");

        fs::write(
            repo.root.join("confirmed.txt"),
            "B's mutation after recovery\n",
        )
        .expect("B's post-recovery mutation should write");
        drive_opencode(
            &repo,
            &opencode_after(&cwd, session_id, "call_b_write", "write"),
        )
        .expect("B's ToolExecuteAfter should confirm exactly B's own surviving scope");

        git(&repo.root, &["add", "-A"]);
        git(&repo.root, &["commit", "-qm", "concurrent mutation"]);

        let db = repo.db();
        let post_commit_data = capture_post_commit_patch_from_git(&repo.root)
            .expect("capturing the post-commit patch should succeed");
        let mutation_ai_patch = resolve_post_commit_mutation_ai_patch(
            &repo.root,
            &db,
            &ParsedPatch { files: Vec::new() },
            &post_commit_data.parsed_patch,
        );

        let ai_paths: Vec<&str> = mutation_ai_patch
            .files
            .iter()
            .map(|file| file.new_path.as_str())
            .collect();
        assert!(
            !ai_paths.contains(&"rejected.txt"),
            "the abandoned scope's own mutation must never enter mutation_ai_patch"
        );
        assert!(
            !ai_paths.contains(&"ambiguous.txt"),
            "B's mutation made before the ambiguity-consuming flush is genuinely \
                 indistinguishable from A's and must stay non-AI, not merely non-A"
        );
        assert!(
            ai_paths.contains(&"confirmed.txt"),
            "B's own later mutation, made after A's interval was consumed and confirmed \
                 by B's own Close, must be attributed AI"
        );
    }

    #[test]
    fn opencode_and_codex_unconfirmed_overlap_stays_ineligible_until_codex_confirms() {
        let repo = ProvenanceE2eRepo::new("opencode-codex-overlap");
        let oc_session = "ses_opencode_overlap";
        let codex_session = "codex-overlap-session";
        let cwd = repo.cwd();

        drive_opencode(
            &repo,
            &opencode_before(
                &cwd,
                oc_session,
                "call_oc",
                "write",
                Some("opencode/big-pickle"),
            ),
        )
        .expect("OpenCode write should establish a scope");

        let codex_pre = json!({
            "hook_event_name": "PreToolUse",
            "session_id": codex_session,
            "turn_id": "codex-overlap-turn",
            "cwd": cwd,
            "tool_name": "Bash",
            "tool_use_id": "codex-overlap-bash",
            "model": "gpt-5.6-sol",
            "tool_input": {"command": "true"},
        });
        codex_mutation_scope::run_codex_mutation_scope_from_payload_at_state_root(
            &repo.state_root,
            &codex_pre.to_string(),
            None,
        )
        .expect("Codex Bash PreToolUse should establish a concurrent scope");

        repo.write_change("one\nopencode overlap mutation\n");

        drive_opencode(&repo, &opencode_after(&cwd, oc_session, "call_oc", "write"))
            .expect("OpenCode ToolExecuteAfter should close its own scope");

        let attribution_after_first_close = repo.mutation_events();
        assert_eq!(
            attribution_after_first_close
                .last()
                .map(|(kind, _)| kind.as_str()),
            Some("ineligible_unscoped"),
            "an unconfirmed live Codex scope must suppress OpenCode's own confirming close"
        );

        let codex_post = json!({
            "hook_event_name": "PostToolUse",
            "session_id": codex_session,
            "turn_id": "codex-overlap-turn",
            "cwd": cwd,
            "tool_name": "Bash",
            "tool_use_id": "codex-overlap-bash",
        });
        codex_mutation_scope::run_codex_mutation_scope_from_payload_at_state_root(
            &repo.state_root,
            &codex_post.to_string(),
            None,
        )
        .expect("Codex PostToolUse should close its own scope");

        drive_opencode(
            &repo,
            &opencode_before(
                &cwd,
                oc_session,
                "call_oc_2",
                "write",
                Some("opencode/big-pickle"),
            ),
        )
        .expect("a fresh OpenCode write should establish a new scope");
        repo.write_change("one\nopencode overlap mutation\nsecond change\n");
        drive_opencode(
            &repo,
            &opencode_after(&cwd, oc_session, "call_oc_2", "write"),
        )
        .expect("the fresh OpenCode scope should close cleanly once Codex is confirmed");

        let attribution_after_second_close = repo.mutation_events();
        assert_eq!(
                attribution_after_second_close
                    .last()
                    .map(|(kind, _)| kind.as_str()),
                Some("ai_exclusive"),
                "once every other live scope is confirmation-safe, a solo confirming close is AiExclusive"
            );
    }

    #[test]
    fn opencode_and_claude_overlap_produces_ai_contended() {
        let repo = ProvenanceE2eRepo::new("opencode-claude-overlap");
        let oc_session = "ses_opencode_contended";
        let claude_session = "claude-overlap-session";
        let cwd = repo.cwd();

        drive_opencode(
            &repo,
            &opencode_before(
                &cwd,
                oc_session,
                "call_oc_contended",
                "write",
                Some("opencode/big-pickle"),
            ),
        )
        .expect("OpenCode write should establish a scope");

        let claude_pre = json!({
            "hook_event_name": "PreToolUse",
            "session_id": claude_session,
            "cwd": cwd,
            "tool_name": "Bash",
            "tool_use_id": "claude-overlap-bash",
            "tool_input": {"command": "printf mutation"},
        });
        claude_mutation_scope::run_claude_mutation_scope_from_payload_at_state_root(
            &repo.state_root,
            &claude_pre.to_string(),
            None,
        )
        .expect(
            "Claude Bash PreToolUse should establish a concurrent, non-confirmation-required scope",
        );

        repo.write_change("one\ncontended mutation\n");

        drive_opencode(
            &repo,
            &opencode_after(&cwd, oc_session, "call_oc_contended", "write"),
        )
        .expect("OpenCode ToolExecuteAfter should confirm its own scope");

        let attribution = repo.mutation_events();
        assert_eq!(
                attribution.last().map(|(kind, _)| kind.as_str()),
                Some("ai_contended"),
                "a confirmed OpenCode close alongside a live non-confirmation-required Claude scope is contended, not suppressed"
            );

        let claude_post = json!({
            "hook_event_name": "PostToolUse",
            "session_id": claude_session,
            "cwd": cwd,
            "tool_name": "Bash",
            "tool_use_id": "claude-overlap-bash",
        });
        claude_mutation_scope::run_claude_mutation_scope_from_payload_at_state_root(
            &repo.state_root,
            &claude_post.to_string(),
            None,
        )
        .expect("Claude PostToolUse should close its own scope");
    }

    #[test]
    fn claude_bash_mutation_persists_model_and_session_in_agent_trace() {
        let repo = ProvenanceE2eRepo::new("claude");
        let session_id = "claude-session-e2e";
        let db = repo.db();
        db.upsert_claude_model_state(ClaudeModelStateObservation {
            session_id: format!("cc_{session_id}"),
            agent_id: String::new(),
            model_id: String::from("claude/opus-4-1"),
            observation_kind: ObservationKind::SessionStart,
            source: String::from("test"),
            observed_at_ms: 1,
        })
        .expect("Claude model state should be persisted");

        let cwd = repo.cwd();
        let pre = json!({
            "hook_event_name": "PreToolUse",
            "session_id": session_id,
            "cwd": cwd,
            "tool_name": "Bash",
            "tool_use_id": "claude-bash-e2e",
            "tool_input": {"command": "printf mutation"},
        });
        claude_mutation_scope::run_claude_mutation_scope_from_payload_at_state_root(
            &repo.state_root,
            &pre.to_string(),
            None,
        )
        .expect("Claude Bash PreToolUse should establish a scope");

        let post = json!({
            "hook_event_name": "PostToolUse",
            "session_id": session_id,
            "cwd": repo.cwd(),
            "tool_name": "Bash",
            "tool_use_id": "claude-bash-e2e",
        });
        repo.write_change("one\nclaude mutation\n");
        claude_mutation_scope::run_claude_mutation_scope_from_payload_at_state_root(
            &repo.state_root,
            &post.to_string(),
            None,
        )
        .expect("Claude Bash PostToolUse should close the scope");
        repo.commit_change();

        let trace = repo.run_post_commit();
        assert_mutation_trace_provenance(&trace, "claude/opus-4-1", "cc_claude-session-e2e");
        assert_eq!(row_count(&repo.db(), "diff_traces"), 0);
        assert_eq!(row_count(&repo.db(), "post_commit_patch_intersections"), 1);
        assert_eq!(row_count(&repo.db(), "mutation_trace_events"), 1);
        assert_eq!(row_count(&repo.db(), "agent_traces"), 1);
    }

    #[test]
    fn codex_bash_mutation_persists_model_and_session_in_agent_trace() {
        let repo = ProvenanceE2eRepo::new("codex");
        let session_id = "codex-session-e2e";
        let cwd = repo.cwd();
        let pre = json!({
            "hook_event_name": "PreToolUse",
            "session_id": session_id,
            "turn_id": "codex-turn-e2e",
            "cwd": cwd,
            "tool_name": "Bash",
            "tool_use_id": "codex-bash-e2e",
            "model": "gpt-5.6-sol",
            "tool_input": {"command": "true"},
        });
        codex_mutation_scope::run_codex_mutation_scope_from_payload_at_state_root(
            &repo.state_root,
            &pre.to_string(),
            None,
        )
        .expect("Codex Bash PreToolUse should establish a scope");

        let post = json!({
            "hook_event_name": "PostToolUse",
            "session_id": session_id,
            "turn_id": "codex-turn-e2e",
            "cwd": repo.cwd(),
            "tool_name": "Bash",
            "tool_use_id": "codex-bash-e2e",
        });
        repo.write_change("one\ncodex mutation\n");
        codex_mutation_scope::run_codex_mutation_scope_from_payload_at_state_root(
            &repo.state_root,
            &post.to_string(),
            None,
        )
        .expect("Codex Bash PostToolUse should close the scope");
        repo.commit_change();

        let trace = repo.run_post_commit();
        assert_mutation_trace_provenance(&trace, "gpt-5.6-sol", "cx_codex-session-e2e");
        assert_eq!(row_count(&repo.db(), "diff_traces"), 0);
        assert_eq!(row_count(&repo.db(), "post_commit_patch_intersections"), 1);
        assert_eq!(row_count(&repo.db(), "mutation_trace_events"), 1);
        assert_eq!(row_count(&repo.db(), "agent_traces"), 1);
    }

    fn pi_tool_call(
        cwd: &str,
        session_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        model: Option<&str>,
    ) -> String {
        let mut payload = json!({
            "hook_event_name": "ToolCall",
            "session_id": session_id,
            "tool_call_id": tool_call_id,
            "cwd": cwd,
            "tool_name": tool_name,
        });
        if let Some(model) = model {
            payload["model"] = json!(model);
        }
        payload.to_string()
    }

    fn pi_tool_result(cwd: &str, session_id: &str, tool_call_id: &str, tool_name: &str) -> String {
        json!({
            "hook_event_name": "ToolResult",
            "session_id": session_id,
            "tool_call_id": tool_call_id,
            "cwd": cwd,
            "tool_name": tool_name,
        })
        .to_string()
    }

    fn pi_tool_execution_end(
        cwd: &str,
        session_id: &str,
        tool_call_id: &str,
        tool_name: &str,
    ) -> String {
        json!({
            "hook_event_name": "ToolExecutionEnd",
            "session_id": session_id,
            "tool_call_id": tool_call_id,
            "cwd": cwd,
            "tool_name": tool_name,
        })
        .to_string()
    }

    fn drive_pi(repo: &ProvenanceE2eRepo, payload: &str) -> Result<String> {
        pi_mutation_scope::run_pi_mutation_scope_from_payload_at_state_root(
            &repo.state_root,
            payload,
            None,
        )
    }

    fn pi_confirmed_tool_case(tool_name: &str, label: &str) {
        let repo = ProvenanceE2eRepo::new(label);
        let session_id = format!("ses-{label}");
        let call_id = format!("call-{label}");
        let cwd = repo.cwd();

        drive_pi(
            &repo,
            &pi_tool_call(
                &cwd,
                &session_id,
                &call_id,
                tool_name,
                Some("anthropic/opus-5"),
            ),
        )
        .expect("Pi ToolCall should establish the tracked scope before execution");

        repo.write_change(&format!("one\npi {tool_name} mutation\n"));

        drive_pi(
            &repo,
            &pi_tool_result(&cwd, &session_id, &call_id, tool_name),
        )
        .expect("Pi ToolResult should mark the attempt executed");
        drive_pi(
            &repo,
            &pi_tool_execution_end(&cwd, &session_id, &call_id, tool_name),
        )
        .expect("Pi ToolExecutionEnd paired with an observed ToolResult should Close");
        repo.commit_change();

        let trace = repo.run_post_commit();
        assert_mutation_trace_provenance(&trace, "anthropic/opus-5", &format!("pi_{session_id}"));
        assert_eq!(row_count(&repo.db(), "diff_traces"), 0);
        assert_eq!(row_count(&repo.db(), "post_commit_patch_intersections"), 1);
        assert_eq!(row_count(&repo.db(), "mutation_trace_events"), 1);
        assert_eq!(row_count(&repo.db(), "agent_traces"), 1);
    }

    #[test]
    fn pi_bash_mutation_persists_model_and_session_in_agent_trace() {
        pi_confirmed_tool_case("bash", "pi-bash");
    }

    #[test]
    fn pi_write_mutation_persists_model_and_session_in_agent_trace() {
        pi_confirmed_tool_case("write", "pi-write");
    }

    #[test]
    fn pi_edit_mutation_persists_model_and_session_in_agent_trace() {
        pi_confirmed_tool_case("edit", "pi-edit");
    }

    #[test]
    fn pi_missing_model_preserves_session_with_null_model_in_agent_trace() {
        let repo = ProvenanceE2eRepo::new("pi-no-model");
        let session_id = "ses-pi-no-model";
        let cwd = repo.cwd();

        drive_pi(
            &repo,
            &pi_tool_call(&cwd, session_id, "call-1", "bash", None),
        )
        .expect("Pi ToolCall should establish the scope without model evidence");

        repo.write_change("one\npi mutation without model\n");

        drive_pi(&repo, &pi_tool_result(&cwd, session_id, "call-1", "bash"))
            .expect("Pi ToolResult should mark the attempt executed");
        drive_pi(
            &repo,
            &pi_tool_execution_end(&cwd, session_id, "call-1", "bash"),
        )
        .expect("Pi ToolExecutionEnd should close the scope");
        repo.commit_change();

        let trace = repo.run_post_commit();
        assert_eq!(trace["files"][0]["path"], json!("file.txt"));
        let contributor = &trace["files"][0]["conversations"][0]["contributor"];
        assert_eq!(contributor["type"], json!("ai"));
        assert!(
            contributor.get("model_id").is_none(),
            "absent model evidence must never be guessed or fabricated"
        );
        assert_eq!(
            trace["files"][0]["conversations"][0]["related"],
            json!([{
                "type": "session",
                "url": "https://sce.crocoder.dev/sessions/pi_ses-pi-no-model",
            }])
        );
    }

    #[test]
    fn pi_read_only_and_unknown_tools_create_no_scope_or_mutation_state() {
        let repo = ProvenanceE2eRepo::new("pi-untracked");
        let session_id = "ses-pi-untracked";
        let cwd = repo.cwd();

        for tool_name in [
            "read",
            "grep",
            "find",
            "ls",
            "custom_mcp_tool",
            "totally_unknown_future_tool",
            "user_bash",
        ] {
            let call_id = format!("call-{tool_name}");
            drive_pi(
                &repo,
                &pi_tool_call(&cwd, session_id, &call_id, tool_name, None),
            )
            .unwrap_or_else(|_| panic!("{tool_name} ToolCall should be neutral"));
            drive_pi(
                &repo,
                &pi_tool_result(&cwd, session_id, &call_id, tool_name),
            )
            .unwrap_or_else(|_| panic!("{tool_name} ToolResult should be neutral"));
            drive_pi(
                &repo,
                &pi_tool_execution_end(&cwd, session_id, &call_id, tool_name),
            )
            .unwrap_or_else(|_| panic!("{tool_name} ToolExecutionEnd should be neutral"));
        }

        assert_eq!(row_count(&repo.db(), "mutation_trace_scopes"), 0);
        assert_eq!(row_count(&repo.db(), "mutation_trace_events"), 0);
    }

    #[test]
    fn pi_later_extension_rejection_after_start_produces_no_mutation_ai_patch() {
        let repo = ProvenanceE2eRepo::new("pi-later-rejection");
        let session_id = "ses-pi-rejected";
        let cwd = repo.cwd();

        drive_pi(
            &repo,
            &pi_tool_call(&cwd, session_id, "call-1", "bash", Some("anthropic/opus-5")),
        )
        .expect("Pi ToolCall should establish the scope before a later extension can reject it");

        fs::write(
            repo.root.join("rejected.txt"),
            "should never be attributed AI\n",
        )
        .expect("the blocked attempt's incidental write should still land on disk");

        drive_pi(
            &repo,
            &pi_tool_execution_end(&cwd, session_id, "call-1", "bash"),
        )
        .expect("ToolExecutionEnd with no preceding ToolResult must abandon, not error");

        let scope_status = repo
            .db()
            .query_map("SELECT status FROM mutation_trace_scopes", (), |row| {
                row.get::<String>(0).map_err(anyhow::Error::from)
            })
            .expect("scope-status query should succeed");
        assert_eq!(
            scope_status,
            vec!["abandoned".to_string()],
            "the D7 abandon path must leave the scope durably abandoned, never closed or active"
        );

        git(&repo.root, &["add", "-A"]);
        git(&repo.root, &["commit", "-qm", "rejected mutation"]);

        let db = repo.db();
        let post_commit_data = capture_post_commit_patch_from_git(&repo.root)
            .expect("capturing the post-commit patch should succeed");
        let mutation_ai_patch = resolve_post_commit_mutation_ai_patch(
            &repo.root,
            &db,
            &ParsedPatch { files: Vec::new() },
            &post_commit_data.parsed_patch,
        );

        assert!(
                mutation_ai_patch.files.is_empty(),
                "a Start that never reached a confirmed Close must never produce mutation_ai_patch entries"
            );
    }

    #[test]
    fn pi_mutate_then_error_still_persists_confirmed_mutation_through_close() {
        let repo = ProvenanceE2eRepo::new("pi-error-executed");
        let session_id = "ses-pi-error";
        let cwd = repo.cwd();

        drive_pi(
            &repo,
            &pi_tool_call(&cwd, session_id, "call-1", "bash", Some("anthropic/opus-5")),
        )
        .expect("Pi ToolCall should establish the scope");

        repo.write_change("one\npartial mutation before failure\n");

        let mut result_payload: Value =
            serde_json::from_str(&pi_tool_result(&cwd, session_id, "call-1", "bash"))
                .expect("tool_result payload should parse as JSON");
        result_payload["isError"] = json!(true);
        drive_pi(&repo, &result_payload.to_string())
            .expect("a failed-but-executed ToolResult is still positive execution evidence");

        drive_pi(
            &repo,
            &pi_tool_execution_end(&cwd, session_id, "call-1", "bash"),
        )
        .expect("ToolExecutionEnd paired with an observed ToolResult must Close, not abandon");
        repo.commit_change();

        let trace = repo.run_post_commit();
        assert_mutation_trace_provenance(&trace, "anthropic/opus-5", "pi_ses-pi-error");
    }

    #[test]
    fn pi_concurrent_reject_and_confirm_keeps_only_the_confirmed_mutation_ai() {
        let repo = ProvenanceE2eRepo::new("pi-concurrent-reject");
        let session_id = "ses-pi-concurrent";
        let cwd = repo.cwd();

        drive_pi(
            &repo,
            &pi_tool_call(
                &cwd,
                session_id,
                "call-a-edit",
                "edit",
                Some("anthropic/opus-5"),
            ),
        )
        .expect("A's edit ToolCall should establish a scope");
        drive_pi(
            &repo,
            &pi_tool_call(
                &cwd,
                session_id,
                "call-b-write",
                "write",
                Some("anthropic/opus-5"),
            ),
        )
        .expect("B's write ToolCall should establish a distinct concurrent scope");

        fs::write(repo.root.join("rejected.txt"), "rejected mutation\n")
            .expect("A's mutation should write");
        fs::write(
            repo.root.join("ambiguous.txt"),
            "B's mutation before recovery\n",
        )
        .expect("B's pre-recovery mutation should write");

        drive_pi(
            &repo,
            &pi_tool_execution_end(&cwd, session_id, "call-a-edit", "edit"),
        )
        .expect(
            "A's ToolExecutionEnd with no ToolResult should abandon A's scope and consume \
                 the shared ambiguous interval",
        );

        fs::write(
            repo.root.join("confirmed.txt"),
            "B's mutation after recovery\n",
        )
        .expect("B's post-recovery mutation should write");

        drive_pi(
            &repo,
            &pi_tool_result(&cwd, session_id, "call-b-write", "write"),
        )
        .expect("B's ToolResult should mark it executed");
        drive_pi(
            &repo,
            &pi_tool_execution_end(&cwd, session_id, "call-b-write", "write"),
        )
        .expect("B's ToolExecutionEnd should confirm exactly B's own surviving scope");

        git(&repo.root, &["add", "-A"]);
        git(&repo.root, &["commit", "-qm", "concurrent mutation"]);

        let db = repo.db();
        let post_commit_data = capture_post_commit_patch_from_git(&repo.root)
            .expect("capturing the post-commit patch should succeed");
        let mutation_ai_patch = resolve_post_commit_mutation_ai_patch(
            &repo.root,
            &db,
            &ParsedPatch { files: Vec::new() },
            &post_commit_data.parsed_patch,
        );

        let ai_paths: Vec<&str> = mutation_ai_patch
            .files
            .iter()
            .map(|file| file.new_path.as_str())
            .collect();
        assert!(
            !ai_paths.contains(&"rejected.txt"),
            "the abandoned scope's own mutation must never enter mutation_ai_patch"
        );
        assert!(
            !ai_paths.contains(&"ambiguous.txt"),
            "B's mutation made before the ambiguity-consuming flush is genuinely \
                 indistinguishable from A's and must stay non-AI, not merely non-A"
        );
        assert!(
            ai_paths.contains(&"confirmed.txt"),
            "B's own later mutation, made after A's interval was consumed and confirmed \
                 by B's own Close, must be attributed AI"
        );
    }

    #[test]
    fn pi_and_claude_overlap_produces_ai_contended() {
        let repo = ProvenanceE2eRepo::new("pi-claude-overlap");
        let pi_session = "ses-pi-contended";
        let claude_session = "claude-pi-overlap-session";
        let cwd = repo.cwd();

        drive_pi(
            &repo,
            &pi_tool_call(
                &cwd,
                pi_session,
                "call-pi-contended",
                "write",
                Some("anthropic/opus-5"),
            ),
        )
        .expect("Pi write should establish a scope");

        let claude_pre = json!({
            "hook_event_name": "PreToolUse",
            "session_id": claude_session,
            "cwd": cwd,
            "tool_name": "Bash",
            "tool_use_id": "claude-pi-overlap-bash",
            "tool_input": {"command": "printf mutation"},
        });
        claude_mutation_scope::run_claude_mutation_scope_from_payload_at_state_root(
            &repo.state_root,
            &claude_pre.to_string(),
            None,
        )
        .expect(
            "Claude Bash PreToolUse should establish a concurrent, non-confirmation-required scope",
        );

        repo.write_change("one\npi+claude contended mutation\n");

        drive_pi(
            &repo,
            &pi_tool_result(&cwd, pi_session, "call-pi-contended", "write"),
        )
        .expect("Pi ToolResult should mark the attempt executed");
        drive_pi(
            &repo,
            &pi_tool_execution_end(&cwd, pi_session, "call-pi-contended", "write"),
        )
        .expect("Pi ToolExecutionEnd should confirm its own scope");

        let attribution = repo.mutation_events();
        assert_eq!(
                attribution.last().map(|(kind, _)| kind.as_str()),
                Some("ai_contended"),
                "a confirmed Pi close alongside a live non-confirmation-required Claude scope is contended, not suppressed"
            );

        let claude_post = json!({
            "hook_event_name": "PostToolUse",
            "session_id": claude_session,
            "cwd": cwd,
            "tool_name": "Bash",
            "tool_use_id": "claude-pi-overlap-bash",
        });
        claude_mutation_scope::run_claude_mutation_scope_from_payload_at_state_root(
            &repo.state_root,
            &claude_post.to_string(),
            None,
        )
        .expect("Claude PostToolUse should close its own scope");
    }

    #[test]
    fn pi_and_codex_overlap_stays_ineligible_until_codex_confirms() {
        let repo = ProvenanceE2eRepo::new("pi-codex-overlap");
        let pi_session = "ses-pi-overlap";
        let codex_session = "codex-pi-overlap-session";
        let cwd = repo.cwd();

        drive_pi(
            &repo,
            &pi_tool_call(
                &cwd,
                pi_session,
                "call-pi",
                "write",
                Some("anthropic/opus-5"),
            ),
        )
        .expect("Pi write should establish a scope");

        let codex_pre = json!({
            "hook_event_name": "PreToolUse",
            "session_id": codex_session,
            "turn_id": "codex-pi-overlap-turn",
            "cwd": cwd,
            "tool_name": "Bash",
            "tool_use_id": "codex-pi-overlap-bash",
            "model": "gpt-5.6-sol",
            "tool_input": {"command": "true"},
        });
        codex_mutation_scope::run_codex_mutation_scope_from_payload_at_state_root(
            &repo.state_root,
            &codex_pre.to_string(),
            None,
        )
        .expect("Codex Bash PreToolUse should establish a concurrent scope");

        repo.write_change("one\npi codex overlap mutation\n");

        drive_pi(&repo, &pi_tool_result(&cwd, pi_session, "call-pi", "write"))
            .expect("Pi ToolResult should mark the attempt executed");
        drive_pi(
            &repo,
            &pi_tool_execution_end(&cwd, pi_session, "call-pi", "write"),
        )
        .expect("Pi ToolExecutionEnd should attempt to confirm its own scope");

        let attribution_after_first_close = repo.mutation_events();
        assert_eq!(
            attribution_after_first_close
                .last()
                .map(|(kind, _)| kind.as_str()),
            Some("ineligible_unscoped"),
            "an unconfirmed live Codex scope must suppress Pi's own confirming close"
        );

        let codex_post = json!({
            "hook_event_name": "PostToolUse",
            "session_id": codex_session,
            "turn_id": "codex-pi-overlap-turn",
            "cwd": cwd,
            "tool_name": "Bash",
            "tool_use_id": "codex-pi-overlap-bash",
        });
        codex_mutation_scope::run_codex_mutation_scope_from_payload_at_state_root(
            &repo.state_root,
            &codex_post.to_string(),
            None,
        )
        .expect("Codex PostToolUse should close its own scope");

        drive_pi(
            &repo,
            &pi_tool_call(
                &cwd,
                pi_session,
                "call-pi-2",
                "write",
                Some("anthropic/opus-5"),
            ),
        )
        .expect("a fresh Pi write should establish a new scope");
        repo.write_change("one\npi codex overlap mutation\nsecond change\n");
        drive_pi(
            &repo,
            &pi_tool_result(&cwd, pi_session, "call-pi-2", "write"),
        )
        .expect("Pi ToolResult should mark the fresh attempt executed");
        drive_pi(
            &repo,
            &pi_tool_execution_end(&cwd, pi_session, "call-pi-2", "write"),
        )
        .expect("the fresh Pi scope should close cleanly once Codex is confirmed");

        let attribution_after_second_close = repo.mutation_events();
        assert_eq!(
                attribution_after_second_close
                    .last()
                    .map(|(kind, _)| kind.as_str()),
                Some("ai_exclusive"),
                "once every other live scope is confirmation-safe, a solo confirming close is AiExclusive"
            );
    }

    #[test]
    fn pi_and_opencode_overlap_stays_ineligible_until_opencode_confirms() {
        let repo = ProvenanceE2eRepo::new("pi-opencode-overlap");
        let pi_session = "ses-pi-oc-overlap";
        let oc_session = "ses_opencode_pi_overlap";
        let cwd = repo.cwd();

        drive_pi(
            &repo,
            &pi_tool_call(
                &cwd,
                pi_session,
                "call-pi",
                "write",
                Some("anthropic/opus-5"),
            ),
        )
        .expect("Pi write should establish a scope");

        drive_opencode(
            &repo,
            &opencode_before(
                &cwd,
                oc_session,
                "call_oc",
                "write",
                Some("opencode/big-pickle"),
            ),
        )
        .expect("OpenCode write ToolExecuteBefore should establish a concurrent scope");

        repo.write_change("one\npi opencode overlap mutation\n");

        drive_pi(&repo, &pi_tool_result(&cwd, pi_session, "call-pi", "write"))
            .expect("Pi ToolResult should mark the attempt executed");
        drive_pi(
            &repo,
            &pi_tool_execution_end(&cwd, pi_session, "call-pi", "write"),
        )
        .expect("Pi ToolExecutionEnd should attempt to confirm its own scope");

        let attribution_after_first_close = repo.mutation_events();
        assert_eq!(
            attribution_after_first_close
                .last()
                .map(|(kind, _)| kind.as_str()),
            Some("ineligible_unscoped"),
            "an unconfirmed live OpenCode scope must suppress Pi's own confirming close"
        );

        drive_opencode(&repo, &opencode_after(&cwd, oc_session, "call_oc", "write"))
            .expect("OpenCode ToolExecuteAfter should close its own scope");

        drive_pi(
            &repo,
            &pi_tool_call(
                &cwd,
                pi_session,
                "call-pi-2",
                "write",
                Some("anthropic/opus-5"),
            ),
        )
        .expect("a fresh Pi write should establish a new scope");
        repo.write_change("one\npi opencode overlap mutation\nsecond change\n");
        drive_pi(
            &repo,
            &pi_tool_result(&cwd, pi_session, "call-pi-2", "write"),
        )
        .expect("Pi ToolResult should mark the fresh attempt executed");
        drive_pi(
            &repo,
            &pi_tool_execution_end(&cwd, pi_session, "call-pi-2", "write"),
        )
        .expect("the fresh Pi scope should close cleanly once OpenCode is confirmed");

        let attribution_after_second_close = repo.mutation_events();
        assert_eq!(
                attribution_after_second_close
                    .last()
                    .map(|(kind, _)| kind.as_str()),
                Some("ai_exclusive"),
                "once every other live scope is confirmation-safe, a solo confirming close is AiExclusive"
            );
    }

    #[test]
    fn pi_stale_process_recovery_discards_ambiguous_interval_while_fresh_pi_work_remains_usable() {
        let repo = ProvenanceE2eRepo::new("pi-stale-recovery");
        let stale_session = "ses-pi-stale";
        let fresh_session = "ses-pi-fresh";
        let cwd = repo.cwd();

        drive_pi(
            &repo,
            &pi_tool_call(
                &cwd,
                stale_session,
                "call-stale",
                "bash",
                Some("anthropic/opus-5"),
            ),
        )
        .expect("the stale attempt's Pi ToolCall should establish a scope");

        let git_dir = resolve_git_dir(&repo.root).expect("git dir should resolve");
        let scope_id = pi_mutation_scope::state::read_state(&git_dir)
            .expect("state should be readable")
            .attempts
            .iter()
            .find(|attempt| attempt.session_id == stale_session)
            .expect("the stale attempt should exist")
            .scope_id
            .clone();
        pi_mutation_scope::force_attempt_owner_dead_for_tests(&git_dir, &scope_id);

        fs::write(
            repo.root.join("ambiguous.txt"),
            "left behind by the dead Pi process\n",
        )
        .expect("the stale attempt's own mutation should still land on disk");

        drive_pi(
                &repo,
                &pi_tool_call(
                    &cwd,
                    fresh_session,
                    "call-fresh",
                    "write",
                    Some("anthropic/opus-5"),
                ),
            )
            .expect("a fresh Pi ToolCall should trigger dead-owner recovery and then establish its own scope");

        fs::write(repo.root.join("confirmed.txt"), "the fresh Pi work\n")
            .expect("the fresh attempt's mutation should write");

        drive_pi(
            &repo,
            &pi_tool_result(&cwd, fresh_session, "call-fresh", "write"),
        )
        .expect("the fresh attempt's ToolResult should mark it executed");
        drive_pi(
            &repo,
            &pi_tool_execution_end(&cwd, fresh_session, "call-fresh", "write"),
        )
        .expect("the fresh attempt should close and reach AiExclusive");

        git(&repo.root, &["add", "-A"]);
        git(&repo.root, &["commit", "-qm", "stale recovery"]);

        let db = repo.db();
        let post_commit_data = capture_post_commit_patch_from_git(&repo.root)
            .expect("capturing the post-commit patch should succeed");
        let mutation_ai_patch = resolve_post_commit_mutation_ai_patch(
            &repo.root,
            &db,
            &ParsedPatch { files: Vec::new() },
            &post_commit_data.parsed_patch,
        );

        let ai_paths: Vec<&str> = mutation_ai_patch
            .files
            .iter()
            .map(|file| file.new_path.as_str())
            .collect();
        assert!(
            !ai_paths.contains(&"ambiguous.txt"),
            "the dead process's ambiguous interval must never be attributed AI"
        );
        assert!(
            ai_paths.contains(&"confirmed.txt"),
            "later fresh Pi work must remain usable and reach AiExclusive"
        );

        let attribution = repo.mutation_events();
        assert_eq!(
                attribution.last().map(|(kind, _)| kind.as_str()),
                Some("ai_exclusive"),
                "the fresh attempt, unencumbered by the recovered stale scope, should reach AiExclusive"
            );
    }
}

#[test]
fn post_commit_auto_sync_does_not_launch_when_disabled() {
    let launch_called = RefCell::new(false);

    run_post_commit_subcommand_with(
        Path::new("/repo"),
        None,
        "",
        |_| Ok(post_commit_flow_result()),
        |_, _, _, _| Ok(minimal_agent_trace()),
        |_| Ok(false),
        |_| {
            *launch_called.borrow_mut() = true;
            Ok(())
        },
        |_| Ok(()),
        None,
    )
    .expect("disabled auto-sync should not affect post-commit success");

    assert!(!*launch_called.borrow());
}

#[test]
fn post_commit_persistence_failure_does_not_launch_auto_sync() {
    let launch_called = RefCell::new(false);

    let error = run_post_commit_subcommand_with(
        Path::new("/repo"),
        None,
        "",
        |_| Ok(post_commit_flow_result()),
        |_, _, _, _| Err(anyhow!("Agent Trace persistence failed")),
        |_| panic!("auto-sync config must not be resolved after persistence failure"),
        |_| {
            *launch_called.borrow_mut() = true;
            Ok(())
        },
        |_| panic!("checkpoint must not run after persistence failure"),
        None,
    )
    .expect_err("persistence failure should be returned");

    assert!(error.to_string().contains("persistence failed"));
    assert!(!*launch_called.borrow());
}

#[test]
fn post_commit_auto_sync_launcher_failure_is_fail_open() {
    let output = run_post_commit_subcommand_with(
        Path::new("/repo"),
        None,
        "",
        |_| Ok(post_commit_flow_result()),
        |_, _, _, _| Ok(minimal_agent_trace()),
        |_| Ok(true),
        |_| Err(anyhow!("spawn unavailable")),
        |_| Ok(()),
        None,
    )
    .expect("launcher failure must not affect post-commit success");

    assert!(output.contains("post-commit hook processed intersection"));
}

#[derive(Default)]
struct RecordingLogger {
    warnings: std::sync::Mutex<Vec<(String, String)>>,
}

impl Logger for RecordingLogger {
    fn info(
        &self,
        _event_id: &str,
        _message: &str,
        _fields: &[(&str, &str)],
        _session_id: Option<&str>,
    ) {
    }

    fn debug(
        &self,
        _event_id: &str,
        _message: &str,
        _fields: &[(&str, &str)],
        _session_id: Option<&str>,
    ) {
    }

    fn warn(
        &self,
        event_id: &str,
        message: &str,
        _fields: &[(&str, &str)],
        _session_id: Option<&str>,
    ) {
        self.warnings
            .lock()
            .expect("warnings mutex should not be poisoned")
            .push((event_id.to_string(), message.to_string()));
    }

    fn error(
        &self,
        _event_id: &str,
        _message: &str,
        _fields: &[(&str, &str)],
        _session_id: Option<&str>,
    ) {
    }

    fn log_cli_error(&self, _error: &crate::services::error::CliError, _session_id: Option<&str>) {}
}

#[test]
fn post_commit_checkpoint_runs_once_after_successful_persistence() {
    let events = RefCell::new(Vec::new());

    let output = run_post_commit_subcommand_with(
        Path::new("/repo"),
        None,
        "",
        |_| Ok(post_commit_flow_result()),
        |_, _, _, _| {
            events.borrow_mut().push("persistence");
            Ok(minimal_agent_trace())
        },
        |_| Ok(false),
        |_| Ok(()),
        |_| {
            events.borrow_mut().push("checkpoint");
            Ok(())
        },
        None,
    )
    .expect("successful checkpoint should not affect post-commit success");

    assert!(output.contains("post-commit hook processed intersection"));
    assert_eq!(events.into_inner(), vec!["persistence", "checkpoint"]);
}

#[test]
fn post_commit_checkpoint_failure_is_fail_open_and_logs_warning() {
    let logger = RecordingLogger::default();
    let persisted = RefCell::new(false);

    let output = run_post_commit_subcommand_with(
        Path::new("/repo"),
        None,
        "",
        |_| Ok(post_commit_flow_result()),
        |_, _, _, _| {
            *persisted.borrow_mut() = true;
            Ok(minimal_agent_trace())
        },
        |_| Ok(false),
        |_| Ok(()),
        |_| Err(anyhow!("checkpoint failed")),
        Some(&logger),
    )
    .expect("checkpoint failure must not affect post-commit success");

    assert!(output.contains("post-commit hook processed intersection"));
    assert!(*persisted.borrow());
    assert_eq!(
        logger
            .warnings
            .into_inner()
            .expect("warnings mutex should not be poisoned"),
        vec![(
            String::from("sce.agent_trace_db.passive_checkpoint_failed"),
            String::from("checkpoint failed")
        )]
    );
}
