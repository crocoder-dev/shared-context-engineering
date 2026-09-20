use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::Value;

use super::state::{read_state, AdapterAttempt, AdapterState, AttemptPhase, RecoveryState};
use super::*;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn temp_git_dir(label: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "sce-pi-mutation-scope-lifecycle-{label}-{}-{id}",
        std::process::id()
    ))
}

const CWD: &str = "/repo/pi-checkout";

struct RecordingSeam {
    calls: Mutex<Vec<String>>,
    fail_operations: Vec<String>,
    fail_once_operations: Mutex<Vec<String>>,
}

impl RecordingSeam {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail_operations: Vec::new(),
            fail_once_operations: Mutex::new(Vec::new()),
        }
    }

    fn failing_on(operations: &[&str]) -> Self {
        Self {
            fail_operations: operations.iter().map(|op| (*op).to_string()).collect(),
            ..Self::new()
        }
    }

    fn failing_once_on(operations: &[&str]) -> Self {
        Self {
            fail_once_operations: Mutex::new(
                operations.iter().map(|op| (*op).to_string()).collect(),
            ),
            ..Self::new()
        }
    }

    fn handle(&self, payload: &str) -> Result<String> {
        let operation = operation_of(payload);
        {
            let mut calls = self.calls.lock().expect("seam mutex");
            calls.push(operation.clone());
        }
        if self.fail_operations.contains(&operation) {
            bail!("seam failure injected by test for '{operation}'");
        }
        {
            let mut once = self.fail_once_operations.lock().expect("seam mutex");
            if let Some(position) = once.iter().position(|candidate| candidate == &operation) {
                once.remove(position);
                bail!("transient seam failure injected once by test for '{operation}'");
            }
        }
        Ok(String::new())
    }

    fn operations(&self) -> Vec<String> {
        self.calls.lock().expect("seam mutex").clone()
    }
}

fn operation_of(payload: &str) -> String {
    let value: Value = serde_json::from_str(payload).expect("seam payload is JSON");
    value
        .get("operation")
        .and_then(Value::as_str)
        .expect("seam payload has an operation")
        .to_string()
}

fn drive(git_dir: &Path, seam: &RecordingSeam, payload: &str) -> Result<String> {
    let resolver = |_cwd: &str| Ok(git_dir.to_path_buf());
    let seam_fn = |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| seam.handle(payload);
    run_pi_mutation_scope_from_payload_with_seams(payload, None, &resolver, &seam_fn)
}

fn tool_call_event(tool_name: &str, tool_call_id: &str) -> String {
    json!({
        "hook_event_name": "ToolCall",
        "session_id": "ses-main",
        "tool_call_id": tool_call_id,
        "cwd": CWD,
        "tool_name": tool_name,
        "model": "openai-codex/gpt-5.5",
    })
    .to_string()
}

fn tool_result_event(tool_name: &str, tool_call_id: &str) -> String {
    json!({
        "hook_event_name": "ToolResult",
        "session_id": "ses-main",
        "tool_call_id": tool_call_id,
        "cwd": CWD,
        "tool_name": tool_name,
    })
    .to_string()
}

fn tool_execution_end_event(tool_name: &str, tool_call_id: &str) -> String {
    json!({
        "hook_event_name": "ToolExecutionEnd",
        "session_id": "ses-main",
        "tool_call_id": tool_call_id,
        "cwd": CWD,
        "tool_name": tool_name,
    })
    .to_string()
}

fn tool_execution_abandon_event(tool_name: &str, tool_call_id: &str) -> String {
    json!({
        "hook_event_name": "ToolExecutionAbandon",
        "session_id": "ses-main",
        "tool_call_id": tool_call_id,
        "cwd": CWD,
        "tool_name": tool_name,
    })
    .to_string()
}

fn tool_execution_abandon_event_for_session(
    tool_name: &str,
    session_id: &str,
    tool_call_id: &str,
) -> String {
    json!({
        "hook_event_name": "ToolExecutionAbandon",
        "session_id": session_id,
        "tool_call_id": tool_call_id,
        "cwd": CWD,
        "tool_name": tool_name,
    })
    .to_string()
}

fn cleanup(git_dir: &Path) {
    let _ = std::fs::remove_dir_all(git_dir);
}

fn tool_call_event_for_session(tool_name: &str, session_id: &str, tool_call_id: &str) -> String {
    json!({
        "hook_event_name": "ToolCall",
        "session_id": session_id,
        "tool_call_id": tool_call_id,
        "cwd": CWD,
        "tool_name": tool_name,
        "model": "openai-codex/gpt-5.5",
    })
    .to_string()
}

fn tool_result_event_for_session(tool_name: &str, session_id: &str, tool_call_id: &str) -> String {
    json!({
        "hook_event_name": "ToolResult",
        "session_id": session_id,
        "tool_call_id": tool_call_id,
        "cwd": CWD,
        "tool_name": tool_name,
    })
    .to_string()
}

fn dead_process_owner() -> super::process_owner::ProcessOwner {
    let mut dead_child = std::process::Command::new("true")
        .spawn()
        .expect("spawning 'true' should succeed");
    let dead_pid = i32::try_from(dead_child.id()).expect("pid fits in i32");
    dead_child.wait().expect("child should exit and be reaped");
    super::process_owner::ProcessOwner {
        pid: dead_pid,
        instance_token: None,
    }
}

fn attempt_owned_by(state: &AdapterState, session_id: &str) -> AdapterAttempt {
    state
        .attempts
        .iter()
        .find(|attempt| attempt.session_id == session_id)
        .expect("attempt for session must exist")
        .clone()
}

#[test]
fn tool_call_establishes_a_write_ahead_start_and_replays_idempotently() {
    let git_dir = temp_git_dir("write-ahead-start");
    let seam = RecordingSeam::new();

    drive(&git_dir, &seam, &tool_call_event("write", "call_1")).expect("first Start");
    drive(&git_dir, &seam, &tool_call_event("write", "call_1"))
        .expect("duplicate Start is idempotent");

    assert_eq!(seam.operations(), vec!["start", "start"]);
    let state = read_state(&git_dir).expect("state readable");
    assert_eq!(state.attempts.len(), 1);
    assert_eq!(state.attempts[0].phase, AttemptPhase::PendingStart);
    assert_eq!(state.attempts[0].tool_name, "write");

    cleanup(&git_dir);
}

#[test]
fn concurrent_bash_calls_in_one_session_stay_separate_live_scopes() {
    let git_dir = temp_git_dir("concurrent-bash");
    let seam = RecordingSeam::new();

    drive(&git_dir, &seam, &tool_call_event("bash", "call_a")).expect("A Start");
    drive(&git_dir, &seam, &tool_call_event("bash", "call_b")).expect("B Start must not retire A");

    let state = read_state(&git_dir).expect("state readable");
    assert_eq!(state.attempts.len(), 2);
    assert!(state
        .attempts
        .iter()
        .all(|attempt| attempt.phase == AttemptPhase::PendingStart));

    cleanup(&git_dir);
}

#[test]
fn full_success_lifecycle_start_result_close() {
    let git_dir = temp_git_dir("success-lifecycle");
    let seam = RecordingSeam::new();

    drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("Start");
    assert_eq!(
        read_state(&git_dir).expect("state readable").attempts[0].phase,
        AttemptPhase::PendingStart
    );

    drive(&git_dir, &seam, &tool_result_event("bash", "call_1")).expect("tool_result");
    assert_eq!(
        read_state(&git_dir).expect("state readable").attempts[0].phase,
        AttemptPhase::Executed,
        "D5/D6: tool_result is the sole Executed-transition evidence"
    );

    drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_1"))
        .expect("tool_execution_end closes an Executed attempt");
    assert!(read_state(&git_dir)
        .expect("state readable")
        .attempts
        .is_empty());
    assert_eq!(seam.operations(), vec!["start", "close"]);

    cleanup(&git_dir);
}

#[test]
fn tool_execution_end_without_a_preceding_tool_result_abandons_never_closes() {
    let git_dir = temp_git_dir("d7-abandon");
    let seam = RecordingSeam::new();

    drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("Start");

    drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_1"))
        .expect("D7: a terminal event with no preceding tool_result must abandon");

    assert!(read_state(&git_dir)
        .expect("state readable")
        .attempts
        .is_empty());
    assert!(
        !seam.operations().contains(&"close".to_string()),
        "D7: an unexecuted attempt must never be closed"
    );
    assert!(seam.operations().contains(&"abandon".to_string()));
    assert!(read_state(&git_dir)
        .expect("state readable")
        .recovery
        .is_clear());

    cleanup(&git_dir);
}

#[test]
fn a_failed_close_falls_back_to_abandon_recovery() {
    let git_dir = temp_git_dir("close-failure-falls-back");
    let seam = RecordingSeam::failing_on(&["close"]);

    drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("Start");
    drive(&git_dir, &seam, &tool_result_event("bash", "call_1")).expect("tool_result");

    drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_1"))
        .expect("a Close failure must recover via abandon, not surface an error");

    assert!(read_state(&git_dir)
        .expect("state readable")
        .attempts
        .is_empty());
    assert!(seam.operations().contains(&"abandon".to_string()));

    cleanup(&git_dir);
}

#[test]
fn execution_abandon_forces_abandon_even_when_the_attempt_is_already_executed() {
    let git_dir = temp_git_dir("d9-execution-abandon-executed");
    let seam = RecordingSeam::new();

    drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("Start");
    drive(&git_dir, &seam, &tool_result_event("bash", "call_1")).expect("tool_result");

    drive(
        &git_dir,
        &seam,
        &tool_execution_abandon_event("bash", "call_1"),
    )
    .expect("ExecutionAbandon must recover via abandon, not surface an error");

    assert!(read_state(&git_dir)
        .expect("state readable")
        .attempts
        .is_empty());
    assert!(
        !seam.operations().contains(&"close".to_string()),
        "D9: an explicit abandon request must never be treated as a Close, \
             even for an attempt already marked Executed"
    );
    assert!(seam.operations().contains(&"abandon".to_string()));

    cleanup(&git_dir);
}

#[test]
fn execution_abandon_on_a_pending_start_attempt_abandons() {
    let git_dir = temp_git_dir("d9-execution-abandon-pending-start");
    let seam = RecordingSeam::new();

    drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("Start");

    drive(
        &git_dir,
        &seam,
        &tool_execution_abandon_event("bash", "call_1"),
    )
    .expect("ExecutionAbandon must recover a PendingStart attempt via abandon");

    assert!(read_state(&git_dir)
        .expect("state readable")
        .attempts
        .is_empty());
    assert!(seam.operations().contains(&"abandon".to_string()));

    cleanup(&git_dir);
}

#[test]
fn execution_abandon_for_an_unknown_attempt_is_a_safe_no_op() {
    let git_dir = temp_git_dir("d9-execution-abandon-unknown");
    let seam = RecordingSeam::new();

    let result = drive(
        &git_dir,
        &seam,
        &tool_execution_abandon_event("bash", "call_1"),
    )
    .expect("an unknown attempt must be a safe no-op, never an error");

    assert_eq!(result, "");
    assert!(seam.operations().is_empty());

    cleanup(&git_dir);
}

#[test]
fn execution_abandon_for_an_untracked_tool_is_a_no_op() {
    let git_dir = temp_git_dir("d9-execution-abandon-untracked");
    let seam = RecordingSeam::new();

    let result = drive(
        &git_dir,
        &seam,
        &tool_execution_abandon_event("read", "call_1"),
    )
    .expect("untracked tools are never adapter-relevant");

    assert_eq!(result, "");
    assert!(seam.operations().is_empty());

    cleanup(&git_dir);
}

#[test]
fn duplicate_execution_abandon_on_an_already_abandoned_attempt_is_a_safe_no_op() {
    let git_dir = temp_git_dir("d9-execution-abandon-duplicate");
    let seam = RecordingSeam::new();

    drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("Start");

    drive(
        &git_dir,
        &seam,
        &tool_execution_abandon_event("bash", "call_1"),
    )
    .expect("first ExecutionAbandon retires the attempt");
    assert!(read_state(&git_dir)
        .expect("state readable")
        .attempts
        .is_empty());

    let result = drive(
        &git_dir,
        &seam,
        &tool_execution_abandon_event("bash", "call_1"),
    )
    .expect("a duplicate ExecutionAbandon for an already-retired attempt must be a safe no-op");

    assert_eq!(result, "");
    assert_eq!(
        seam.operations(),
        vec!["start", "flush", "abandon", "flush"],
        "a duplicate ExecutionAbandon must never issue a second abandon or a close"
    );

    cleanup(&git_dir);
}

#[test]
fn execution_abandon_for_one_session_never_touches_another_sessions_attempt_with_the_same_tool_call_id(
) {
    let git_dir = temp_git_dir("d9-execution-abandon-cross-session");
    let seam = RecordingSeam::new();

    drive(
        &git_dir,
        &seam,
        &tool_call_event_for_session("bash", "ses-a", "call_1"),
    )
    .expect("session A Start");
    drive(
        &git_dir,
        &seam,
        &tool_call_event_for_session("bash", "ses-b", "call_1"),
    )
    .expect("session B Start with the same tool_call_id");

    assert_eq!(
        read_state(&git_dir).expect("state readable").attempts.len(),
        2
    );

    drive(
        &git_dir,
        &seam,
        &tool_execution_abandon_event_for_session("bash", "ses-a", "call_1"),
    )
    .expect("ExecutionAbandon for session A must not error");

    let remaining = read_state(&git_dir).expect("state readable").attempts;
    assert_eq!(
        remaining.len(),
        1,
        "abandoning session A's attempt must leave session B's untouched"
    );
    assert_eq!(remaining[0].session_id, "ses-b");
    assert_eq!(remaining[0].tool_call_id, "call_1");
    assert_eq!(remaining[0].phase, AttemptPhase::PendingStart);

    drive(
        &git_dir,
        &seam,
        &tool_result_event_for_session("bash", "ses-b", "call_1"),
    )
    .expect("session B must still be able to progress normally after A's abandon");
    assert_eq!(
        read_state(&git_dir).expect("state readable").attempts[0].phase,
        AttemptPhase::Executed
    );

    cleanup(&git_dir);
}

#[test]
fn a_terminal_recovery_flush_failure_leaves_a_pending_recovery_and_denies_new_admission() {
    let git_dir = temp_git_dir("recovery-flush-failure");
    let persistently_failing = RecordingSeam::failing_on(&["flush"]);

    drive(
        &git_dir,
        &persistently_failing,
        &tool_call_event("bash", "call_1"),
    )
    .expect("Start");
    drive(
        &git_dir,
        &persistently_failing,
        &tool_execution_end_event("bash", "call_1"),
    )
    .expect("abandon path swallows the flush failure rather than surfacing an error");

    assert_eq!(
        read_state(&git_dir).expect("state readable").recovery,
        RecoveryState::Pending { generation: 1 },
        "a failed ambiguity flush must leave recovery Pending, not Clear"
    );

    let error = drive(
        &git_dir,
        &persistently_failing,
        &tool_call_event("bash", "call_2"),
    )
    .expect_err("a new admission must fail closed while recovery remains unresolved");
    assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE));

    let recovered = RecordingSeam::new();
    drive(&git_dir, &recovered, &tool_call_event("bash", "call_3"))
        .expect("a new admission must self-heal once recovery can complete");
    assert!(read_state(&git_dir)
        .expect("state readable")
        .recovery
        .is_clear());
    let state = read_state(&git_dir).expect("state readable");
    assert_eq!(state.attempts.len(), 1);
    assert_eq!(state.attempts[0].tool_call_id, "call_3");

    cleanup(&git_dir);
}

#[test]
fn start_provenance_carries_the_prefixed_session_and_normalized_model_to_the_seam() {
    let git_dir = temp_git_dir("provenance-present");
    let captured: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let resolver = |_cwd: &str| Ok(git_dir.clone());
    let seam_fn = |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| {
        captured
            .lock()
            .expect("capture mutex")
            .push(payload.to_string());
        Ok(String::new())
    };

    run_pi_mutation_scope_from_payload_with_seams(
        &tool_call_event("bash", "call_model"),
        None,
        &resolver,
        &seam_fn,
    )
    .expect("Start should succeed");

    let payloads = captured.into_inner().expect("capture mutex");
    assert_eq!(payloads.len(), 1);
    let sent: Value = serde_json::from_str(&payloads[0]).expect("seam payload is JSON");
    assert_eq!(
        sent["provenance"]["session_id"].as_str(),
        Some("pi_ses-main")
    );
    assert_eq!(
        sent["provenance"]["model_id"].as_str(),
        Some("openai-codex/gpt-5.5")
    );

    cleanup(&git_dir);
}

#[test]
fn start_provenance_is_null_model_when_the_event_carries_no_model() {
    let git_dir = temp_git_dir("provenance-absent");
    let captured: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let resolver = |_cwd: &str| Ok(git_dir.clone());
    let seam_fn = |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| {
        captured
            .lock()
            .expect("capture mutex")
            .push(payload.to_string());
        Ok(String::new())
    };

    let payload = json!({
        "hook_event_name": "ToolCall",
        "session_id": "ses-main",
        "tool_call_id": "call_no_model",
        "cwd": CWD,
        "tool_name": "bash",
    })
    .to_string();

    run_pi_mutation_scope_from_payload_with_seams(&payload, None, &resolver, &seam_fn)
        .expect("Start should succeed");

    let payloads = captured.into_inner().expect("capture mutex");
    let sent: Value = serde_json::from_str(&payloads[0]).expect("seam payload is JSON");
    assert!(sent["provenance"]["model_id"].is_null());

    cleanup(&git_dir);
}

#[test]
fn untracked_tool_call_never_admits_an_attempt() {
    let git_dir = temp_git_dir("untracked-no-admit");
    let seam = RecordingSeam::new();

    drive(&git_dir, &seam, &tool_call_event("read", "call_ro")).expect("untracked is inert");
    assert!(seam.operations().is_empty());
    assert!(read_state(&git_dir)
        .expect("state readable")
        .attempts
        .is_empty());

    drive(&git_dir, &seam, &tool_result_event("read", "call_ro")).expect("untracked result inert");
    drive(
        &git_dir,
        &seam,
        &tool_execution_end_event("read", "call_ro"),
    )
    .expect("untracked terminal inert");
    assert!(seam.operations().is_empty());

    cleanup(&git_dir);
}

#[test]
fn a_pending_start_attempt_owned_by_a_dead_process_is_abandoned_not_replayed() {
    let git_dir = temp_git_dir("d10-dead-owner-abandon");
    let seam = RecordingSeam::new();

    drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("first Start");
    let scope_id = read_state(&git_dir).expect("state readable").attempts[0]
        .scope_id
        .clone();

    let mut dead_child = std::process::Command::new("true")
        .spawn()
        .expect("spawning 'true' should succeed");
    let dead_pid = i32::try_from(dead_child.id()).expect("pid fits in i32");
    dead_child.wait().expect("child should exit and be reaped");
    state::set_attempt_owner_for_tests(
        &git_dir,
        &scope_id,
        super::process_owner::ProcessOwner {
            pid: dead_pid,
            instance_token: None,
        },
    );

    drive(&git_dir, &seam, &tool_call_event("bash", "call_1"))
        .expect("a replay whose recorded owner is positively dead must abandon, not reuse");

    assert_eq!(
        seam.operations(),
        vec!["start", "flush", "abandon", "flush", "start"],
        "D10: a dead-owner PendingStart must be abandoned via the existing D8 flush/abandon/\
             flush pattern, then the triggering event admitted as a fresh attempt"
    );
    let state = read_state(&git_dir).expect("state readable");
    assert_eq!(state.attempts.len(), 1);
    assert_ne!(
        state.attempts[0].scope_id, scope_id,
        "the fresh attempt must never reuse the abandoned attempt's ScopeId"
    );
    assert_eq!(state.attempts[0].attempt_seq, 2);
    assert!(state.recovery.is_clear());

    cleanup(&git_dir);
}

#[test]
fn a_pending_start_attempt_owned_by_a_live_process_is_never_abandoned_by_a_replay() {
    let git_dir = temp_git_dir("d10-live-owner-no-abandon");
    let seam = RecordingSeam::new();

    drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("first Start");
    drive(&git_dir, &seam, &tool_call_event("bash", "call_1"))
        .expect("a replay owned by a still-live process must be treated as a normal replay");

    assert_eq!(
        seam.operations(),
        vec!["start", "start"],
        "no TTL and no elapsed time may ever cause an abandon here: the owner is this test \
             process's own live parent for the whole test"
    );
    let state = read_state(&git_dir).expect("state readable");
    assert_eq!(state.attempts.len(), 1);
    assert_eq!(state.attempts[0].phase, AttemptPhase::PendingStart);

    cleanup(&git_dir);
}

#[test]
fn a_dead_pending_start_attempt_is_recovered_by_an_unrelated_fresh_session_start() {
    let git_dir = temp_git_dir("d10-fresh-session-dead-pending-start");
    let seam = RecordingSeam::new();

    drive(
        &git_dir,
        &seam,
        &tool_call_event_for_session("bash", "sess-a", "call-a"),
    )
    .expect("A's Start");
    let scope_a =
        attempt_owned_by(&read_state(&git_dir).expect("state readable"), "sess-a").scope_id;
    state::set_attempt_owner_for_tests(&git_dir, &scope_a, dead_process_owner());

    drive(
        &git_dir,
        &seam,
        &tool_call_event_for_session("bash", "sess-b", "call-b"),
    )
    .expect(
        "B's Start must recover A's stale owner without ever replaying A's \
             (session_id, tool_call_id) key",
    );

    assert_eq!(
        seam.operations(),
        vec!["start", "flush", "abandon", "flush", "start"],
        "D10: a dead PendingStart owner discovered by an unrelated fresh-session Start must \
             be retired through the existing D8 flush/abandon/flush sequence before the \
             triggering Start is admitted"
    );
    let state = read_state(&git_dir).expect("state readable");
    assert_eq!(state.attempts.len(), 1);
    assert_eq!(state.attempts[0].session_id, "sess-b");
    assert_ne!(state.attempts[0].scope_id, scope_a);
    assert!(state.recovery.is_clear());

    cleanup(&git_dir);
}

#[test]
fn a_dead_executed_attempt_is_recovered_by_a_fresh_session_start_without_a_synthetic_close() {
    let git_dir = temp_git_dir("d10-fresh-session-dead-executed");
    let seam = RecordingSeam::new();

    drive(
        &git_dir,
        &seam,
        &tool_call_event_for_session("bash", "sess-a", "call-a"),
    )
    .expect("A's Start");
    drive(
        &git_dir,
        &seam,
        &tool_result_event_for_session("bash", "sess-a", "call-a"),
    )
    .expect("A's tool_result marks Executed");
    let state = read_state(&git_dir).expect("state readable");
    let scope_a = attempt_owned_by(&state, "sess-a").scope_id;
    assert_eq!(
        attempt_owned_by(&state, "sess-a").phase,
        AttemptPhase::Executed
    );
    state::set_attempt_owner_for_tests(&git_dir, &scope_a, dead_process_owner());

    drive(
        &git_dir,
        &seam,
        &tool_call_event_for_session("bash", "sess-b", "call-b"),
    )
    .expect("B's Start must recover A's dead Executed attempt");

    assert_eq!(
        seam.operations(),
        vec!["start", "flush", "abandon", "flush", "start"],
        "a dead Executed attempt must be abandoned/rebaselined via D8, never given a \
             synthetic delayed Close"
    );
    assert!(
        !seam.operations().contains(&"close".to_string()),
        "D9: the current Git tree no longer represents the original terminal observation \
             time, so a dead Executed attempt must never be closed"
    );
    let state = read_state(&git_dir).expect("state readable");
    assert_eq!(state.attempts.len(), 1);
    assert_eq!(state.attempts[0].session_id, "sess-b");
    assert!(state.recovery.is_clear());

    cleanup(&git_dir);
}

#[test]
fn a_dead_owner_scope_is_recovered_while_a_live_owner_sibling_survives_untouched() {
    let git_dir = temp_git_dir("d10-dead-live-sibling-isolation");
    let seam = RecordingSeam::new();

    drive(
        &git_dir,
        &seam,
        &tool_call_event_for_session("bash", "sess-a", "call-a"),
    )
    .expect("A's Start (owner will die)");
    drive(
        &git_dir,
        &seam,
        &tool_call_event_for_session("bash", "sess-b", "call-b"),
    )
    .expect("B's Start (owner stays live)");

    let state = read_state(&git_dir).expect("state readable");
    let scope_a = attempt_owned_by(&state, "sess-a").scope_id;
    let scope_b = attempt_owned_by(&state, "sess-b").scope_id;
    state::set_attempt_owner_for_tests(&git_dir, &scope_a, dead_process_owner());

    drive(
        &git_dir,
        &seam,
        &tool_call_event_for_session("bash", "sess-c", "call-c"),
    )
    .expect("C's Start must recover only A");

    assert_eq!(
        seam.operations(),
        vec!["start", "start", "flush", "abandon", "flush", "start"],
        "exactly one abandon must occur, and only for A's own positively dead owner"
    );
    let state = read_state(&git_dir).expect("state readable");
    assert_eq!(state.attempts.len(), 2);
    assert!(!state.attempts.iter().any(|a| a.scope_id == scope_a));
    let b = attempt_owned_by(&state, "sess-b");
    assert_eq!(b.scope_id, scope_b);
    assert_eq!(
        b.phase,
        AttemptPhase::PendingStart,
        "B must survive reconciliation exactly as it was, untouched"
    );
    assert_eq!(
        attempt_owned_by(&state, "sess-c").phase,
        AttemptPhase::PendingStart
    );

    cleanup(&git_dir);
}

#[test]
fn multiple_dead_owner_scopes_are_retired_in_one_recovery_generation_while_a_live_sibling_survives()
{
    let git_dir = temp_git_dir("d10-multiple-dead-owner-scopes");
    let seam = RecordingSeam::new();

    drive(
        &git_dir,
        &seam,
        &tool_call_event_for_session("bash", "sess-a", "call-a"),
    )
    .expect("A's Start");
    drive(
        &git_dir,
        &seam,
        &tool_result_event_for_session("bash", "sess-a", "call-a"),
    )
    .expect("A's tool_result marks Executed");
    drive(
        &git_dir,
        &seam,
        &tool_call_event_for_session("bash", "sess-b", "call-b"),
    )
    .expect("B's Start");
    drive(
        &git_dir,
        &seam,
        &tool_call_event_for_session("bash", "sess-q", "call-q"),
    )
    .expect("Q's Start (owner stays live)");

    let state = read_state(&git_dir).expect("state readable");
    let scope_a = attempt_owned_by(&state, "sess-a").scope_id;
    let scope_b = attempt_owned_by(&state, "sess-b").scope_id;
    let scope_q = attempt_owned_by(&state, "sess-q").scope_id;
    let dead_owner = dead_process_owner();
    state::set_attempt_owner_for_tests(&git_dir, &scope_a, dead_owner);
    state::set_attempt_owner_for_tests(&git_dir, &scope_b, dead_owner);

    drive(
        &git_dir,
        &seam,
        &tool_call_event_for_session("bash", "sess-c", "call-c"),
    )
    .expect("C's Start must recover both A and B, grouped into one recovery generation");

    assert_eq!(
        seam.operations(),
        vec!["start", "start", "start", "flush", "abandon", "abandon", "flush", "start"],
        "a single flush/abandon.../flush recovery generation must retire every \
             independently-proven-dead scope owned by the same dead process together"
    );
    let state = read_state(&git_dir).expect("state readable");
    assert_eq!(state.attempts.len(), 2);
    assert!(!state.attempts.iter().any(|a| a.scope_id == scope_a));
    assert!(!state.attempts.iter().any(|a| a.scope_id == scope_b));
    assert_eq!(attempt_owned_by(&state, "sess-q").scope_id, scope_q);
    assert!(state.recovery.is_clear());

    cleanup(&git_dir);
}

#[test]
fn an_owner_that_cannot_be_positively_proven_dead_is_never_abandoned_by_an_unrelated_start() {
    let git_dir = temp_git_dir("d10-uncertain-owner-preserved");
    let seam = RecordingSeam::new();

    drive(
        &git_dir,
        &seam,
        &tool_call_event_for_session("bash", "sess-a", "call-a"),
    )
    .expect("A's Start");
    let scope_a =
        attempt_owned_by(&read_state(&git_dir).expect("state readable"), "sess-a").scope_id;
    state::set_attempt_owner_for_tests(
        &git_dir,
        &scope_a,
        super::process_owner::ProcessOwner {
            pid: std::process::id().cast_signed(),
            instance_token: None,
        },
    );

    drive(
        &git_dir,
        &seam,
        &tool_call_event_for_session("bash", "sess-c", "call-c"),
    )
    .expect(
        "C's Start must proceed without touching A, whose owner cannot be positively \
             proven dead",
    );

    assert_eq!(
        seam.operations(),
        vec!["start", "start"],
        "a live pid with no instance-token evidence must never be converted into proof of \
             death: uncertain identity is conservatively treated as alive"
    );
    let state = read_state(&git_dir).expect("state readable");
    assert_eq!(state.attempts.len(), 2);
    assert!(state
        .attempts
        .iter()
        .all(|attempt| attempt.phase == AttemptPhase::PendingStart));
    assert!(state.attempts.iter().any(|a| a.scope_id == scope_a));

    cleanup(&git_dir);
}

#[test]
fn an_interrupted_stale_owner_recovery_remains_pending_and_denies_the_triggering_start_until_resumed(
) {
    let git_dir = temp_git_dir("d10-interrupted-stale-recovery");
    let seam = RecordingSeam::new();

    drive(
        &git_dir,
        &seam,
        &tool_call_event_for_session("bash", "sess-a", "call-a"),
    )
    .expect("A's Start");
    let scope_a =
        attempt_owned_by(&read_state(&git_dir).expect("state readable"), "sess-a").scope_id;
    state::set_attempt_owner_for_tests(&git_dir, &scope_a, dead_process_owner());

    let crashing = RecordingSeam::failing_once_on(&["abandon"]);
    let error = drive(
        &git_dir,
        &crashing,
        &tool_call_event_for_session("bash", "sess-b", "call-b"),
    )
    .expect_err("a Start that triggers a stale-owner recovery which fails mid-way must not commit");
    assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE));

    let state = read_state(&git_dir).expect("state readable");
    assert_eq!(state.recovery, RecoveryState::Pending { generation: 1 });
    assert!(
        !state.attempts.iter().any(|a| a.session_id == "sess-b"),
        "B must never be admitted while A's stale-owner recovery is still pending"
    );
    assert_eq!(
        attempt_owned_by(&state, "sess-a").phase,
        AttemptPhase::PendingAbandon
    );

    drive(
        &git_dir,
        &crashing,
        &tool_call_event_for_session("bash", "sess-b", "call-b"),
    )
    .expect(
        "the next boundary-lock acquisition must resume and complete the pending recovery, \
             and only then admit B",
    );

    let state = read_state(&git_dir).expect("state readable");
    assert!(state.recovery.is_clear());
    assert_eq!(state.attempts.len(), 1);
    assert_eq!(state.attempts[0].session_id, "sess-b");
    assert!(!state.attempts.iter().any(|a| a.scope_id == scope_a));

    cleanup(&git_dir);
}

#[test]
fn duplicate_tool_result_after_close_is_a_safe_no_op() {
    let git_dir = temp_git_dir("duplicate-tool-result-after-close");
    let seam = RecordingSeam::new();

    drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("Start");
    drive(&git_dir, &seam, &tool_result_event("bash", "call_1")).expect("tool_result");
    drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_1")).expect("Close");

    drive(&git_dir, &seam, &tool_result_event("bash", "call_1"))
        .expect("a late duplicate tool_result after Close must be a safe no-op");
    drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_1"))
        .expect("a late duplicate tool_execution_end after Close must be a safe no-op");

    assert_eq!(
        seam.operations(),
        vec!["start", "close"],
        "a resurrected attempt must never re-enter the runtime seam after its own Close"
    );
    assert!(read_state(&git_dir)
        .expect("state readable")
        .attempts
        .is_empty());

    cleanup(&git_dir);
}

#[test]
fn duplicate_tool_execution_end_after_abandon_is_a_safe_no_op() {
    let git_dir = temp_git_dir("duplicate-terminal-after-abandon");
    let seam = RecordingSeam::new();

    drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("Start");
    drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_1")).expect("D7 abandon");

    drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_1"))
        .expect("a late duplicate terminal event after abandon must be a safe no-op");

    assert_eq!(
        seam.operations(),
        vec!["start", "flush", "abandon", "flush"],
        "a duplicate terminal delivery for an already-abandoned attempt must never issue a \
             second abandon"
    );

    cleanup(&git_dir);
}

#[test]
fn abandoning_one_sibling_never_touches_a_concurrent_sibling_in_the_same_session() {
    let git_dir = temp_git_dir("sibling-abandon-isolation");
    let seam = RecordingSeam::new();

    drive(&git_dir, &seam, &tool_call_event("bash", "call_a")).expect("A Start");
    drive(&git_dir, &seam, &tool_call_event("bash", "call_b")).expect("B Start");
    drive(&git_dir, &seam, &tool_result_event("bash", "call_b")).expect("B tool_result");

    drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_a"))
        .expect("A's terminal event with no tool_result must abandon only A");

    let state = read_state(&git_dir).expect("state readable");
    assert_eq!(
        state.attempts.len(),
        1,
        "abandoning A must never remove or block sibling B"
    );
    assert_eq!(state.attempts[0].tool_call_id, "call_b");
    assert_eq!(state.attempts[0].phase, AttemptPhase::Executed);

    drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_b"))
        .expect("B must still close normally after A's abandonment and recovery");
    assert!(read_state(&git_dir)
        .expect("state readable")
        .attempts
        .is_empty());
    assert_eq!(
        seam.operations(),
        vec!["start", "start", "flush", "abandon", "flush", "close"]
    );

    cleanup(&git_dir);
}

#[test]
fn a_crash_mid_abandon_loop_is_resumed_and_completed_on_the_next_boundary_lock_acquisition() {
    let git_dir = temp_git_dir("crash-mid-abandon-loop");
    let crashing = RecordingSeam::failing_once_on(&["abandon"]);

    drive(&git_dir, &crashing, &tool_call_event("bash", "call_1")).expect("Start");
    drive(
        &git_dir,
        &crashing,
        &tool_execution_end_event("bash", "call_1"),
    )
    .expect(
        "a transient abandon failure mid-recovery must leave recovery Pending, not surface \
             an error, simulating a crash between marking PendingAbandon and completing the \
             flush/abandon/flush sequence",
    );

    assert_eq!(
        read_state(&git_dir).expect("state readable").recovery,
        RecoveryState::Pending { generation: 1 },
        "the interrupted abandon loop must leave recovery durably Pending for the next \
             boundary-lock acquisition to resume, never Clear and never lost"
    );
    assert_eq!(
        read_state(&git_dir)
            .expect("state readable")
            .attempts
            .first()
            .expect("the doomed attempt must still be recorded")
            .phase,
        AttemptPhase::PendingAbandon
    );

    drive(&git_dir, &crashing, &tool_call_event("bash", "call_2"))
        .expect("recovery must self-heal and complete on the very next invocation");

    assert!(read_state(&git_dir)
        .expect("state readable")
        .recovery
        .is_clear());
    let state = read_state(&git_dir).expect("state readable");
    assert_eq!(state.attempts.len(), 1);
    assert_eq!(state.attempts[0].tool_call_id, "call_2");

    cleanup(&git_dir);
}

#[test]
fn a_reused_tool_call_id_after_terminal_cleanup_gets_a_distinct_scope_id() {
    let git_dir = temp_git_dir("terminal-scope-id-non-reuse");
    let seam = RecordingSeam::new();

    drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("first Start");
    drive(&git_dir, &seam, &tool_result_event("bash", "call_1")).expect("first tool_result");
    drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_1")).expect("first Close");
    assert!(read_state(&git_dir)
        .expect("state readable")
        .attempts
        .is_empty());

    drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("reused toolCallId Start");
    let state = read_state(&git_dir).expect("state readable");
    assert_eq!(state.attempts.len(), 1);
    assert_eq!(state.attempts[0].attempt_seq, 2);

    cleanup(&git_dir);
}
