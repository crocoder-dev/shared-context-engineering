use std::path::Path;

use crate::services::hooks::mutation_scope_health::{
    MutationScopeAdapterHealth, MutationScopeHealthStatus,
};
use crate::services::mutation_trace::types::ActorKind;

use super::process_owner::is_definitely_dead;
use super::state::{self, AttemptPhase, RecoveryState};

pub(crate) fn classify_health(git_dir: &Path) -> MutationScopeAdapterHealth {
    let state = match state::read_state(git_dir) {
        Ok(state) => state,
        Err(error) => {
            return MutationScopeAdapterHealth::new(
                ActorKind::Pi,
                MutationScopeHealthStatus::Invalid,
                "Pi mutation-scope state file could not be read or parsed.",
            )
            .with_detail(error.to_string());
        }
    };

    let has_pending_abandon = state
        .attempts
        .iter()
        .any(|attempt| attempt.phase == AttemptPhase::PendingAbandon);
    let has_dead_owner_live_attempt = state.attempts.iter().any(|attempt| {
        matches!(
            attempt.phase,
            AttemptPhase::PendingStart | AttemptPhase::Executed
        ) && is_definitely_dead(&attempt.owner)
    });

    match state.recovery {
        RecoveryState::Clear if has_pending_abandon => MutationScopeAdapterHealth::new(
            ActorKind::Pi,
            MutationScopeHealthStatus::Invalid,
            "Persisted state has a PendingAbandon attempt with recovery already Clear, a combination the adapter's recovery-flush state machine cannot legitimately produce (a PendingAbandon attempt is only ever created and removed as part of the same recovery-flush generation that arms and then clears RecoveryState).",
        ),
        RecoveryState::Clear if has_dead_owner_live_attempt => MutationScopeAdapterHealth::new(
            ActorKind::Pi,
            MutationScopeHealthStatus::Recovering,
            "A tracked PendingStart/Executed attempt's recorded owner process is positively dead. The D10 stale-owner sweep (reconcile_stale_owners) runs unconditionally on every future tracked Start from any session, before that session's own admission is even considered, and automatically retires the dead-owner attempt through the ordinary flush/abandon/flush recovery sequence.",
        ),
        RecoveryState::Clear => MutationScopeAdapterHealth::new(
            ActorKind::Pi,
            MutationScopeHealthStatus::Healthy,
            "No persisted recovery condition blocks mutation-capable admission. A live or unprovably-dead PendingStart/Executed attempt never blocks an unrelated tracked admission on this adapter.",
        ),
        RecoveryState::Pending { .. } => MutationScopeAdapterHealth::new(
            ActorKind::Pi,
            MutationScopeHealthStatus::Recovering,
            "Recovery is pending. A subsequent recovery-capable tracked Start whose key is not already represented by a nonterminal attempt can claim the pending generation and retry every outstanding PendingAbandon attempt through the ordinary recovery path. A duplicate Start for an already-tracked PendingStart/Executed key may be idempotently reused before the recovery-state gate, so not every individual Start necessarily advances recovery. Pending is Recovering because an ordinary future tracked admission can advance it without manual intervention; an unrelated stuck PendingStart/Executed attempt, if any, does not prevent this resolution, since this adapter never gates admission on another attempt's PendingStart/Executed phase.",
        ),
        RecoveryState::Flushing { .. } => MutationScopeAdapterHealth::new(
            ActorKind::Pi,
            MutationScopeHealthStatus::Recovering,
            "An orphaned recovery flush is reclaimed from Flushing to Pending by the very next tracked adapter boundary (Start, ToolExecutionEnd, or ToolExecutionAbandon all normalize it before doing anything else). A subsequent recovery-capable fresh tracked Start can then claim the reclaimed generation and retry recovery automatically, but a duplicate Start for an already-tracked nonterminal key is not guaranteed to be the one that does so.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    use serde_json::{json, Value};

    use super::super::process_owner::ProcessOwner;
    use super::super::{
        force_attempt_owner_dead_for_tests, run_pi_mutation_scope_from_payload_with_seams,
        AttemptKey,
    };
    use super::*;
    use crate::services::observability::traits::Logger;

    const FAIL_CLOSED_MESSAGE: &str =
        "SCE could not establish Pi mutation attribution for this tool execution.";
    const CWD: &str = "/repo/pi-checkout";

    static NEXT_TEST_GIT_DIR_ID: AtomicU64 = AtomicU64::new(0);

    fn unique_test_git_dir(label: &str) -> PathBuf {
        let id = NEXT_TEST_GIT_DIR_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "sce-pi-mutation-scope-health-{label}-{}-{id}",
            std::process::id()
        ))
    }

    fn remove_test_git_dir(git_dir: &Path) {
        let _ = std::fs::remove_dir_all(git_dir);
    }

    fn key(session_id: &str, tool_call_id: &str) -> AttemptKey {
        AttemptKey {
            session_id: session_id.to_string(),
            tool_call_id: tool_call_id.to_string(),
        }
    }

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

        fn handle(&self, payload: &str) -> anyhow::Result<String> {
            let operation = operation_of(payload);
            self.calls
                .lock()
                .expect("seam mutex")
                .push(operation.clone());
            if self.fail_operations.contains(&operation) {
                anyhow::bail!("seam failure injected by test for '{operation}'");
            }
            let mut once = self.fail_once_operations.lock().expect("seam mutex");
            if let Some(position) = once.iter().position(|candidate| candidate == &operation) {
                once.remove(position);
                anyhow::bail!("transient seam failure injected once by test for '{operation}'");
            }
            Ok(String::new())
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

    fn drive(git_dir: &Path, seam: &RecordingSeam, payload: &str) -> anyhow::Result<String> {
        let resolver = |_cwd: &str| Ok(git_dir.to_path_buf());
        let seam_fn =
            |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| seam.handle(payload);
        run_pi_mutation_scope_from_payload_with_seams(payload, None, &resolver, &seam_fn)
    }

    fn tool_call_event(session_id: &str, tool_call_id: &str) -> String {
        json!({
            "hook_event_name": "ToolCall",
            "session_id": session_id,
            "tool_call_id": tool_call_id,
            "cwd": CWD,
            "tool_name": "bash",
        })
        .to_string()
    }

    fn tool_result_event(session_id: &str, tool_call_id: &str) -> String {
        json!({
            "hook_event_name": "ToolResult",
            "session_id": session_id,
            "tool_call_id": tool_call_id,
            "cwd": CWD,
            "tool_name": "bash",
        })
        .to_string()
    }

    fn tool_execution_end_event(session_id: &str, tool_call_id: &str) -> String {
        json!({
            "hook_event_name": "ToolExecutionEnd",
            "session_id": session_id,
            "tool_call_id": tool_call_id,
            "cwd": CWD,
            "tool_name": "bash",
        })
        .to_string()
    }

    #[test]
    fn absent_state_file_is_healthy() {
        let git_dir = unique_test_git_dir("absent");

        let health = classify_health(&git_dir);

        assert_eq!(health.adapter, ActorKind::Pi);
        assert_eq!(health.status, MutationScopeHealthStatus::Healthy);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn clear_recovery_is_healthy_with_a_live_owner_pending_start_attempt() {
        let git_dir = unique_test_git_dir("clear-live-owner");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        state::seed_attempt_for_tests(
            &git_dir,
            &key("ses-1", "call-1"),
            "bash",
            AttemptPhase::PendingStart,
        );

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Healthy
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn clear_recovery_is_healthy_with_an_uncertain_owner_pending_start_attempt_never_swept_by_an_unrelated_start(
    ) {
        let git_dir = unique_test_git_dir("clear-uncertain-owner");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let attempt = state::seed_attempt_for_tests(
            &git_dir,
            &key("ses-a", "call-a"),
            "bash",
            AttemptPhase::PendingStart,
        );
        state::set_attempt_owner_for_tests(
            &git_dir,
            &attempt.scope_id,
            ProcessOwner {
                pid: std::process::id().cast_signed(),
                instance_token: None,
            },
        );

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Healthy,
            "a live pid with no instance-token evidence is conservatively treated as alive, never dead"
        );

        let seam = RecordingSeam::new();
        drive(&git_dir, &seam, &tool_call_event("ses-c", "call-c")).expect(
            "an unrelated session's Start must proceed without touching an uncertain-owner attempt",
        );
        assert_eq!(
            seam.calls.lock().expect("seam mutex").clone(),
            vec!["start".to_string()],
            "no D10 sweep may fire for an owner that cannot be positively proven dead"
        );

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Healthy
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn clear_recovery_with_a_dead_owner_pending_start_attempt_is_recovering_and_an_unrelated_session_start_sweeps_it_ac4(
    ) {
        let git_dir = unique_test_git_dir("clear-dead-owner-pending-start");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_call_event("ses-a", "call-a")).expect("A starts");
        let scope_a = state::read_state(&git_dir)
            .expect("state readable")
            .attempts[0]
            .scope_id
            .clone();
        force_attempt_owner_dead_for_tests(&git_dir, &scope_a);

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Recovering,
            "a dead-owner PendingStart attempt is unfinished recovery work with a proven \
             automatic sweep, not a durable wedge"
        );

        drive(&git_dir, &seam, &tool_call_event("ses-b", "call-b"))
            .expect("B's Start must recover A's stale owner without ever replaying A's own key");

        assert_eq!(
            seam.calls.lock().expect("seam mutex").clone(),
            vec!["start", "flush", "abandon", "flush", "start"],
            "D10: the dead-owner attempt must be retired through the ordinary flush/abandon/flush \
             sequence before B's own triggering Start is admitted"
        );
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Healthy,
            "once the sweep completes and B is admitted as an ordinary live attempt, no recovery \
             condition remains"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn clear_recovery_with_a_dead_owner_executed_attempt_is_recovering_and_is_swept_without_a_synthetic_close(
    ) {
        let git_dir = unique_test_git_dir("clear-dead-owner-executed");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_call_event("ses-a", "call-a")).expect("A starts");
        drive(&git_dir, &seam, &tool_result_event("ses-a", "call-a"))
            .expect("A's tool_result marks Executed");
        let state = state::read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts[0].phase, AttemptPhase::Executed);
        force_attempt_owner_dead_for_tests(&git_dir, &state.attempts[0].scope_id);

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Recovering
        );

        drive(&git_dir, &seam, &tool_call_event("ses-b", "call-b"))
            .expect("B's Start must recover A's dead Executed attempt");

        assert_eq!(
            seam.calls.lock().expect("seam mutex").clone(),
            vec!["start", "flush", "abandon", "flush", "start"],
            "ToolResult marks Executed locally without touching the seam"
        );
        assert!(
            !seam
                .calls
                .lock()
                .expect("seam mutex")
                .contains(&"close".to_string()),
            "a dead Executed attempt must never be given a synthetic delayed Close"
        );
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Healthy
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn pending_recovery_from_a_failed_terminal_abandon_is_recovering_and_self_heals_on_the_next_start(
    ) {
        let git_dir = unique_test_git_dir("pending-failed-abandon");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let crashing = RecordingSeam::failing_once_on(&["abandon"]);

        drive(&git_dir, &crashing, &tool_call_event("ses-1", "call-1")).expect("Start");
        drive(
            &git_dir,
            &crashing,
            &tool_execution_end_event("ses-1", "call-1"),
        )
        .expect("a transient abandon failure mid-recovery must not surface an error");

        let seeded = state::read_state(&git_dir).expect("state readable");
        assert_eq!(seeded.recovery, RecoveryState::Pending { generation: 1 });
        assert_eq!(seeded.attempts[0].phase, AttemptPhase::PendingAbandon);

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Recovering,
            "a proven self-healing path exists even though this state currently denies admission"
        );

        let still_failing = RecordingSeam::failing_on(&["abandon"]);
        for attempt_number in 1..=2 {
            let error = drive(
                &git_dir,
                &still_failing,
                &tool_call_event("ses-other", &format!("call-retry-{attempt_number}")),
            )
            .expect_err("admission while recovery is unresolved must stay fail-closed");
            assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE));
        }
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Recovering,
            "repeated denial under a still-failing seam must not change the classification"
        );

        let healthy = RecordingSeam::new();
        drive(&git_dir, &healthy, &tool_call_event("ses-2", "call-2"))
            .expect("recovery must self-heal and complete on the next successful invocation");

        let resolved = state::read_state(&git_dir).expect("state readable");
        assert!(resolved.recovery.is_clear());
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Healthy
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn pending_recovery_from_an_interrupted_dead_owner_sweep_is_recovering_and_denies_the_triggering_start_until_resumed(
    ) {
        let git_dir = unique_test_git_dir("pending-interrupted-sweep");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_call_event("ses-a", "call-a")).expect("A's Start");
        let scope_a = state::read_state(&git_dir)
            .expect("state readable")
            .attempts[0]
            .scope_id
            .clone();
        force_attempt_owner_dead_for_tests(&git_dir, &scope_a);

        let crashing = RecordingSeam::failing_once_on(&["abandon"]);
        drive(&git_dir, &crashing, &tool_call_event("ses-b", "call-b")).expect_err(
            "a Start that triggers a stale-owner recovery which fails mid-way must not commit",
        );

        let seeded = state::read_state(&git_dir).expect("state readable");
        assert_eq!(seeded.recovery, RecoveryState::Pending { generation: 1 });
        assert!(!seeded.attempts.iter().any(|a| a.session_id == "ses-b"));

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Recovering,
            "an interrupted dead-owner sweep still has a proven resumption path on the next \
             tracked Start"
        );

        drive(&git_dir, &crashing, &tool_call_event("ses-b", "call-b")).expect(
            "the next boundary-lock acquisition must resume and complete the pending recovery, \
             then admit B",
        );

        let resolved = state::read_state(&git_dir).expect("state readable");
        assert!(resolved.recovery.is_clear());
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Healthy
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn orphaned_flushing_is_recovering_and_reclaimed_by_the_next_boundary() {
        let git_dir = unique_test_git_dir("orphaned-flushing");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let attempt = state::seed_attempt_for_tests(
            &git_dir,
            &key("ses-stuck", "call-stuck"),
            "bash",
            AttemptPhase::PendingStart,
        );
        state::begin_terminal_cleanup(&git_dir, std::slice::from_ref(&attempt.scope_id))
            .expect("seeding an orphaned flush should succeed");
        assert!(matches!(
            state::read_state(&git_dir)
                .expect("state readable")
                .recovery,
            RecoveryState::Flushing { .. }
        ));

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Recovering,
            "a crash mid-flush leaves an orphaned Flushing that the next boundary reclaims"
        );

        let healthy = RecordingSeam::new();
        drive(&git_dir, &healthy, &tool_call_event("ses-new", "call-new"))
            .expect("the orphaned flush is reclaimed and retried, then the new call is admitted");

        let resolved = state::read_state(&git_dir).expect("state readable");
        assert!(resolved.recovery.is_clear());
        assert!(resolved
            .attempts
            .iter()
            .all(|a| a.session_id != "ses-stuck"));
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Healthy
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn clear_recovery_with_a_pending_abandon_attempt_is_a_structurally_impossible_state_classified_invalid(
    ) {
        let git_dir = unique_test_git_dir("clear-with-pending-abandon-invalid");
        let dir = state::adapter_state_dir(&git_dir);
        std::fs::create_dir_all(&dir).expect("state dir should be created");
        std::fs::write(
            state::state_path(&git_dir),
            serde_json::json!({
                "version": 2,
                "next_attempt_seq": 2,
                "next_recovery_generation": 1,
                "recovery": { "phase": "clear" },
                "attempts": [{
                    "attempt_seq": 1,
                    "scope_id": "pi-tool-v1|n=1|s=5:ses-1|c=6:call-1",
                    "session_id": "ses-1",
                    "tool_call_id": "call-1",
                    "tool_name": "bash",
                    "phase": "pending_abandon",
                    "owner": { "pid": 999_999, "instance_token": null },
                }],
            })
            .to_string(),
        )
        .expect("hand-seeded state file should be writable");

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Invalid
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn malformed_state_file_is_invalid_with_the_read_error_surfaced() {
        let git_dir = unique_test_git_dir("malformed");
        let dir = state::adapter_state_dir(&git_dir);
        std::fs::create_dir_all(&dir).expect("state dir should be created");
        std::fs::write(state::state_path(&git_dir), b"not json")
            .expect("malformed file should be writable");

        let health = classify_health(&git_dir);

        assert_eq!(health.status, MutationScopeHealthStatus::Invalid);
        assert!(
            health
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("malformed"),
            "the read error must be surfaced in the detail: {:?}",
            health.detail
        );

        remove_test_git_dir(&git_dir);
    }

    fn matrix_attempt(
        call_id: &str,
        phase: AttemptPhase,
        owner: ProcessOwner,
    ) -> state::AdapterAttempt {
        let attempt_key = key("ses-main", call_id);
        state::AdapterAttempt {
            attempt_seq: 1,
            scope_id: super::super::format_pi_scope_id(&attempt_key, 1),
            session_id: attempt_key.session_id,
            tool_call_id: attempt_key.tool_call_id,
            tool_name: "bash".to_string(),
            phase,
            owner,
        }
    }

    fn write_matrix_state(
        git_dir: &Path,
        recovery: RecoveryState,
        has_pending_abandon: bool,
        has_dead_owner_attempt: bool,
        live_owner: ProcessOwner,
        dead_owner: ProcessOwner,
    ) {
        std::fs::create_dir_all(state::adapter_state_dir(git_dir))
            .expect("adapter state dir should be created");

        let mut attempts = vec![matrix_attempt(
            "call-live",
            AttemptPhase::PendingStart,
            live_owner,
        )];
        if has_pending_abandon {
            attempts.push(matrix_attempt(
                "call-abandon",
                AttemptPhase::PendingAbandon,
                live_owner,
            ));
        }
        if has_dead_owner_attempt {
            attempts.push(matrix_attempt(
                "call-dead",
                AttemptPhase::Executed,
                dead_owner,
            ));
        }

        let state = state::AdapterState {
            version: 2,
            next_attempt_seq: 2,
            next_recovery_generation: 2,
            recovery,
            attempts,
        };
        std::fs::write(
            state::state_path(git_dir),
            serde_json::to_string(&state).expect("matrix state serializes"),
        )
        .expect("hand-built matrix state should be writable");
    }

    #[test]
    fn health_classification_matrix_covers_all_twelve_recovery_and_attempt_condition_combinations()
    {
        use MutationScopeHealthStatus::{Healthy, Invalid, Recovering};

        let mut dead_child = std::process::Command::new("true")
            .spawn()
            .expect("spawning 'true' should succeed");
        let dead_pid = i32::try_from(dead_child.id()).expect("pid fits in i32");
        dead_child.wait().expect("child should exit and be reaped");
        let dead_owner = ProcessOwner {
            pid: dead_pid,
            instance_token: None,
        };
        let live_owner = super::super::process_owner::current_process_owner();

        let clear = RecoveryState::Clear;
        let pending = RecoveryState::Pending { generation: 1 };
        let flushing = RecoveryState::Flushing { generation: 1 };

        let rows = [
            (clear, false, false, Healthy),
            (clear, false, true, Recovering),
            (clear, true, false, Invalid),
            (clear, true, true, Invalid),
            (pending, false, false, Recovering),
            (pending, false, true, Recovering),
            (pending, true, false, Recovering),
            (pending, true, true, Recovering),
            (flushing, false, false, Recovering),
            (flushing, false, true, Recovering),
            (flushing, true, false, Recovering),
            (flushing, true, true, Recovering),
        ];

        for (index, (recovery, has_pending_abandon, has_dead_owner_attempt, expected)) in
            rows.into_iter().enumerate()
        {
            let git_dir = unique_test_git_dir(&format!("matrix-{index}"));
            write_matrix_state(
                &git_dir,
                recovery,
                has_pending_abandon,
                has_dead_owner_attempt,
                live_owner,
                dead_owner,
            );

            assert_eq!(
                classify_health(&git_dir).status,
                expected,
                "row {index}: recovery={recovery:?} has_pending_abandon={has_pending_abandon} \
                 has_dead_owner_attempt={has_dead_owner_attempt}",
            );

            remove_test_git_dir(&git_dir);
        }
    }

    #[test]
    fn pending_recovery_reuses_a_duplicate_start_for_an_existing_nonterminal_key_without_advancing_recovery_then_a_fresh_start_recovers(
    ) {
        let git_dir = unique_test_git_dir("pending-duplicate-start-vs-fresh-start");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let setup = RecordingSeam::new();
        drive(&git_dir, &setup, &tool_call_event("ses-b", "call-b")).expect("B's Start");
        drive(&git_dir, &setup, &tool_call_event("ses-a", "call-a")).expect("A's Start");

        let crashing_abandon = RecordingSeam::failing_once_on(&["abandon"]);
        drive(
            &git_dir,
            &crashing_abandon,
            &tool_execution_end_event("ses-a", "call-a"),
        )
        .expect("a transient abandon failure mid-recovery must not surface an error");

        let seeded = state::read_state(&git_dir).expect("state readable");
        assert_eq!(seeded.recovery, RecoveryState::Pending { generation: 1 });
        let phase_by_session = |session_id: &str| {
            seeded
                .attempts
                .iter()
                .find(|attempt| attempt.session_id == session_id)
                .expect("attempt for session must exist")
                .phase
        };
        assert_eq!(phase_by_session("ses-a"), AttemptPhase::PendingAbandon);
        assert_eq!(phase_by_session("ses-b"), AttemptPhase::PendingStart);

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Recovering
        );

        let duplicate = RecordingSeam::new();
        drive(&git_dir, &duplicate, &tool_call_event("ses-b", "call-b"))
            .expect("a duplicate Start for an already-tracked nonterminal key stays idempotent");

        assert_eq!(
            duplicate.calls.lock().expect("seam mutex").clone(),
            vec!["start".to_string()],
            "a duplicate Start for B's own key must be reused without touching recovery"
        );
        let after_duplicate = state::read_state(&git_dir).expect("state readable");
        assert_eq!(
            after_duplicate.recovery,
            RecoveryState::Pending { generation: 1 },
            "recovery must remain Pending: the duplicate Start never reached the recovery-state gate"
        );
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Recovering,
            "reusing B's key must not change the classification"
        );

        let fresh = RecordingSeam::new();
        drive(&git_dir, &fresh, &tool_call_event("ses-c", "call-c")).expect(
            "C's fresh Start must claim and complete the pending recovery, then be admitted",
        );

        assert_eq!(
            fresh.calls.lock().expect("seam mutex").clone(),
            vec!["flush", "abandon", "flush", "start"],
            "C claims the Pending generation, resolve_recovery retires A, then C's own Start is admitted"
        );
        let resolved = state::read_state(&git_dir).expect("state readable");
        assert!(resolved.recovery.is_clear());
        assert!(!resolved.attempts.iter().any(|a| a.session_id == "ses-a"));
        assert!(resolved.attempts.iter().any(|a| a.session_id == "ses-b"));
        assert!(resolved.attempts.iter().any(|a| a.session_id == "ses-c"));
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Healthy
        );

        remove_test_git_dir(&git_dir);
    }
}
