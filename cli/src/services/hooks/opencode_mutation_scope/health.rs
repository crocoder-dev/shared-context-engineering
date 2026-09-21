use std::path::Path;

use crate::services::hooks::mutation_scope_health::{
    MutationScopeAdapterHealth, MutationScopeHealthStatus,
};
use crate::services::hooks::mutation_scope_owner::is_definitely_dead;
use crate::services::mutation_trace::types::ActorKind;

use super::state::{self, AttemptPhase, RecoveryState};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Repairability {
    AutoFixable,
    ManualOnly,
}

pub(crate) fn assess_repairability(git_dir: &Path) -> Repairability {
    let Ok(state) = state::read_state(git_dir) else {
        return Repairability::ManualOnly;
    };

    let pending_start: Vec<&state::AdapterAttempt> = state
        .attempts
        .iter()
        .filter(|attempt| attempt.phase == AttemptPhase::PendingStart)
        .collect();

    if pending_start.is_empty() {
        return Repairability::ManualOnly;
    }

    let every_owner_is_positively_dead = pending_start
        .iter()
        .all(|attempt| attempt.owner.as_ref().is_some_and(is_definitely_dead));

    if every_owner_is_positively_dead {
        Repairability::AutoFixable
    } else {
        Repairability::ManualOnly
    }
}

pub(crate) fn classify_health(git_dir: &Path) -> MutationScopeAdapterHealth {
    let state = match state::read_state(git_dir) {
        Ok(state) => state,
        Err(error) => {
            return MutationScopeAdapterHealth::new(
                ActorKind::OpenCode,
                MutationScopeHealthStatus::Invalid,
                "OpenCode mutation-scope state file could not be read or parsed.",
            )
            .with_detail(error.to_string());
        }
    };

    let has_pending_abandon = state
        .attempts
        .iter()
        .any(|attempt| attempt.phase == AttemptPhase::PendingAbandon);
    let has_pending_start = state
        .attempts
        .iter()
        .any(|attempt| attempt.phase == AttemptPhase::PendingStart);

    match state.recovery {
        RecoveryState::Clear if has_pending_abandon => MutationScopeAdapterHealth::new(
            ActorKind::OpenCode,
            MutationScopeHealthStatus::Invalid,
            "Persisted state has a PendingAbandon attempt with recovery already Clear, a combination the adapter's recovery-flush state machine cannot legitimately produce (a PendingAbandon attempt is only ever removed as part of the same recovery flush that clears recovery to Clear).",
        ),
        _ if has_pending_start => MutationScopeAdapterHealth::new(
            ActorKind::OpenCode,
            MutationScopeHealthStatus::Blocked,
            "A tracked attempt is stuck in PendingStart; only that same call's own ToolExecuteAfter/ToolError boundary retires a PendingStart attempt, and resolve_recovery only retries attempts already in PendingAbandon, so a concurrently Pending/Flushing recovery generation for an unrelated attempt can clear without ever touching this one, leaving future tracked admissions from other calls denied without self-clearing.",
        ),
        RecoveryState::Clear => MutationScopeAdapterHealth::new(
            ActorKind::OpenCode,
            MutationScopeHealthStatus::Healthy,
            "No persisted recovery condition blocks mutation-capable admission.",
        ),
        RecoveryState::Pending { .. } => MutationScopeAdapterHealth::new(
            ActorKind::OpenCode,
            MutationScopeHealthStatus::Recovering,
            "Recovery is pending; the next tracked admission from any call claims the flush and retries any outstanding abandonment automatically, regardless of which call performs it.",
        ),
        RecoveryState::Flushing { .. } => MutationScopeAdapterHealth::new(
            ActorKind::OpenCode,
            MutationScopeHealthStatus::Recovering,
            "An orphaned recovery flush is reclaimed from Flushing to Pending by the next tracked adapter boundary. A subsequent recovery-capable tracked admission claims the pending generation and retries recovery automatically.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    use serde_json::{json, Value};

    use super::super::events::AttemptKey;
    use super::super::lifecycle::run_opencode_mutation_scope_from_payload_with_seams;
    use super::*;
    use crate::services::observability::traits::Logger;

    const FAIL_CLOSED_MESSAGE: &str =
        "SCE could not establish OpenCode mutation attribution for this tool execution.";
    const CWD: &str = "/repo/opencode-checkout";

    static NEXT_TEST_GIT_DIR_ID: AtomicU64 = AtomicU64::new(0);

    fn unique_test_git_dir(label: &str) -> PathBuf {
        let id = NEXT_TEST_GIT_DIR_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "sce-opencode-mutation-scope-health-{label}-{}-{id}",
            std::process::id()
        ))
    }

    fn remove_test_git_dir(git_dir: &Path) {
        let _ = std::fs::remove_dir_all(git_dir);
    }

    fn key(call_id: &str) -> AttemptKey {
        AttemptKey {
            session_id: "ses-main".to_string(),
            call_id: call_id.to_string(),
        }
    }

    fn tool_before(tool_name: &str, call_id: &str) -> String {
        json!({
            "hook_event_name": "ToolExecuteBefore",
            "session_id": "ses-main",
            "call_id": call_id,
            "cwd": CWD,
            "tool_name": tool_name,
        })
        .to_string()
    }

    fn tool_error(tool_name: &str, call_id: &str) -> String {
        json!({
            "hook_event_name": "ToolError",
            "session_id": "ses-main",
            "call_id": call_id,
            "cwd": CWD,
            "tool_name": tool_name,
        })
        .to_string()
    }

    struct RecordingSeam {
        calls: Mutex<Vec<String>>,
        fail_operations: Vec<String>,
        fail_operation_occurrence: Option<(String, usize)>,
    }

    impl RecordingSeam {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail_operations: Vec::new(),
                fail_operation_occurrence: None,
            }
        }

        fn failing_on(operations: &[&str]) -> Self {
            Self {
                fail_operations: operations.iter().map(|op| (*op).to_string()).collect(),
                ..Self::new()
            }
        }

        fn failing_on_nth_occurrence(operation: &str, occurrence: usize) -> Self {
            Self {
                fail_operation_occurrence: Some((operation.to_string(), occurrence)),
                ..Self::new()
            }
        }

        fn handle(&self, payload: &str) -> anyhow::Result<String> {
            let operation = operation_of(payload);
            let occurrence = {
                let mut calls = self.calls.lock().expect("seam mutex");
                calls.push(operation.clone());
                calls
                    .iter()
                    .filter(|candidate| *candidate == &operation)
                    .count()
            };
            if self.fail_operations.contains(&operation) {
                anyhow::bail!("seam failure injected by test for '{operation}'");
            }
            if let Some((target, target_occurrence)) = &self.fail_operation_occurrence {
                if target == &operation && *target_occurrence == occurrence {
                    anyhow::bail!(
                        "seam failure injected by test for '{operation}' occurrence {occurrence}"
                    );
                }
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
        run_opencode_mutation_scope_from_payload_with_seams(payload, None, &resolver, &seam_fn)
    }

    #[test]
    fn absent_state_file_is_healthy() {
        let git_dir = unique_test_git_dir("absent");

        let health = classify_health(&git_dir);

        assert_eq!(health.adapter, ActorKind::OpenCode);
        assert_eq!(health.status, MutationScopeHealthStatus::Healthy);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn clear_recovery_is_healthy_even_with_active_attempts() {
        let git_dir = unique_test_git_dir("clear-with-active");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        state::seed_attempt_for_tests(&git_dir, &key("call-1"), "write", AttemptPhase::Active);

        let health = classify_health(&git_dir);

        assert_eq!(health.status, MutationScopeHealthStatus::Healthy);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn pending_start_with_clear_recovery_is_blocked_and_denies_repeated_unrelated_admissions_ac4() {
        let git_dir = unique_test_git_dir("pending-start-blocked");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let failing_start = RecordingSeam::failing_on(&["start"]);
        drive(&git_dir, &failing_start, &tool_before("write", "call-1"))
            .expect_err("a failed Start seam must fail closed, leaving the attempt PendingStart");
        assert_eq!(
            state::read_state(&git_dir)
                .expect("state readable")
                .attempts[0]
                .phase,
            AttemptPhase::PendingStart,
        );

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked,
            "a stale PendingStart with no recovery in progress has no automatic sweep for unrelated admissions"
        );

        let healthy = RecordingSeam::new();
        for (index, call_id) in ["call-2", "call-3"].into_iter().enumerate() {
            let error =
                drive(&git_dir, &healthy, &tool_before("write", call_id)).expect_err(&format!(
                    "unrelated admission #{} must be denied without self-clearing",
                    index + 1
                ));
            assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE));
        }

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked,
            "the classifier must still report Blocked after repeated unrelated denial"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn pending_recovery_with_a_pending_start_attempt_is_blocked_even_though_an_unrelated_pending_abandon_can_still_clear_ac4(
    ) {
        let git_dir = unique_test_git_dir("pending-recovery-with-pending-start");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        {
            let ok_seam = RecordingSeam::new();
            drive(&git_dir, &ok_seam, &tool_before("write", "call-a")).expect("A starts");
        }

        let failing_start = RecordingSeam::failing_on(&["start"]);
        drive(&git_dir, &failing_start, &tool_before("write", "call-b"))
            .expect_err("B's Start seam fails, leaving B PendingStart");
        assert_eq!(
            state::read_state(&git_dir)
                .expect("state readable")
                .attempts
                .iter()
                .find(|a| a.call_id == "call-b")
                .expect("B is tracked")
                .phase,
            AttemptPhase::PendingStart,
        );

        let failing_flush = RecordingSeam::failing_on(&["flush"]);
        drive(&git_dir, &failing_flush, &tool_error("write", "call-a"))
            .expect("A's terminal cleanup returns best-effort even though its flush fails");

        let seeded = state::read_state(&git_dir).expect("state readable");
        assert_eq!(seeded.attempts.len(), 2, "both A and B are still tracked");
        assert_eq!(
            seeded
                .attempts
                .iter()
                .find(|a| a.call_id == "call-a")
                .expect("A is tracked")
                .phase,
            AttemptPhase::PendingAbandon,
        );
        assert_eq!(
            seeded
                .attempts
                .iter()
                .find(|a| a.call_id == "call-b")
                .expect("B is tracked")
                .phase,
            AttemptPhase::PendingStart,
        );
        assert!(
            matches!(seeded.recovery, RecoveryState::Pending { .. }),
            "the failed ambiguity flush relinquishes Flushing back to Pending: {:?}",
            seeded.recovery
        );

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked,
            "B's PendingStart has no automatic sweep, so this state has no future path back to \
             normal admission that doesn't depend on B's own missing terminal event, even though \
             A's PendingAbandon under the same Pending recovery generation could still self-heal \
             on its own"
        );

        let healthy = RecordingSeam::new();
        let error = drive(&git_dir, &healthy, &tool_before("write", "call-c"))
            .expect_err("C is unrelated to both A and B and must still be denied");
        assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE));

        let resolved = state::read_state(&git_dir).expect("state readable");
        assert!(
            resolved.recovery.is_clear(),
            "A's recovery generation advanced to completion via C's admission attempt: {:?}",
            resolved.recovery
        );
        assert!(
            resolved.attempts.iter().all(|a| a.call_id != "call-a"),
            "A was cleaned up by the same recovery resolution that denied C"
        );
        assert_eq!(
            resolved
                .attempts
                .iter()
                .find(|a| a.call_id == "call-b")
                .expect("B is still tracked")
                .phase,
            AttemptPhase::PendingStart,
            "B's PendingStart survives the recovery generation that cleared A"
        );

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked,
            "recovery for A fully resolved to Clear, but B's PendingStart durably wedges the \
             adapter: this is the reachable Pending+PendingStart -> Clear+PendingStart counterexample"
        );

        let error = drive(&git_dir, &healthy, &tool_before("write", "call-d"))
            .expect_err("D is denied again; the adapter never self-clears without B's own event");
        assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE));
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked,
            "repeated unrelated denial must not change the classification"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn orphaned_flushing_with_a_pending_start_attempt_is_blocked_not_recovering_ac4() {
        let git_dir = unique_test_git_dir("orphaned-flushing-with-pending-start");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        {
            let ok_seam = RecordingSeam::new();
            drive(&git_dir, &ok_seam, &tool_before("write", "call-a")).expect("A starts");
        }

        let failing_start = RecordingSeam::failing_on(&["start"]);
        drive(&git_dir, &failing_start, &tool_before("write", "call-b"))
            .expect_err("B's Start seam fails, leaving B PendingStart");

        let doomed = state::read_state(&git_dir)
            .expect("state readable")
            .attempts
            .into_iter()
            .find(|a| a.call_id == "call-a")
            .expect("A is tracked");

        state::begin_terminal_cleanup(&git_dir, std::slice::from_ref(&doomed.scope_id))
            .expect("seeding an orphaned flush should succeed");
        let seeded = state::read_state(&git_dir).expect("state readable");
        assert!(matches!(seeded.recovery, RecoveryState::Flushing { .. }));
        assert_eq!(
            seeded
                .attempts
                .iter()
                .find(|a| a.call_id == "call-b")
                .expect("B is tracked")
                .phase,
            AttemptPhase::PendingStart,
        );

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked,
            "the next boundary reclaims Flushing -> Pending and can advance A's recovery, but \
             B's PendingStart has no reclaim path, so admission stays durably wedged"
        );

        let healthy = RecordingSeam::new();
        let error = drive(&git_dir, &healthy, &tool_before("write", "call-c"))
            .expect_err("C is unrelated to both A and B and must still be denied");
        assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE));

        let resolved = state::read_state(&git_dir).expect("state readable");
        assert!(
            resolved.recovery.is_clear(),
            "the orphaned flush was reclaimed and A's recovery generation completed: {:?}",
            resolved.recovery
        );
        assert!(resolved.attempts.iter().all(|a| a.call_id != "call-a"));
        assert_eq!(
            resolved
                .attempts
                .iter()
                .find(|a| a.call_id == "call-b")
                .expect("B is still tracked")
                .phase,
            AttemptPhase::PendingStart,
        );
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked,
            "A's orphaned-flush recovery reaching Clear does not rescue B's PendingStart"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn pending_recovery_with_pending_abandon_attempts_is_recovering_and_an_unrelated_admission_clears_it(
    ) {
        let git_dir = unique_test_git_dir("pending-non-empty-recovering");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        {
            let ok_seam = RecordingSeam::new();
            drive(&git_dir, &ok_seam, &tool_before("write", "call-1")).expect("Start");
        }

        let failing = RecordingSeam::failing_on(&["abandon"]);
        drive(&git_dir, &failing, &tool_error("write", "call-1"))
            .expect("a terminal failure whose abandon fails still returns best-effort");

        let seeded = state::read_state(&git_dir).expect("state readable");
        assert_eq!(
            seeded.attempts.len(),
            1,
            "the doomed attempt is not forgotten"
        );
        assert_eq!(seeded.attempts[0].phase, AttemptPhase::PendingAbandon);
        assert!(matches!(seeded.recovery, RecoveryState::Pending { .. }));

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Recovering,
            "a proven self-healing path exists even though this state currently denies admission"
        );

        for attempt_number in 1..=2 {
            let error = drive(
                &git_dir,
                &failing,
                &tool_before("write", &format!("call-retry-{attempt_number}")),
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
        drive(&git_dir, &healthy, &tool_before("write", "call-2"))
            .expect("an unrelated call's admission resolves recovery once the seam succeeds");

        let resolved = state::read_state(&git_dir).expect("state readable");
        assert!(resolved.recovery.is_clear());
        assert!(resolved.attempts.iter().all(|a| a.call_id != "call-1"));
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Healthy
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn pending_recovery_with_empty_attempts_is_recovering_and_the_next_admission_clears_it() {
        let git_dir = unique_test_git_dir("pending-empty-recovering");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        {
            let ok_seam = RecordingSeam::new();
            drive(&git_dir, &ok_seam, &tool_before("edit", "call-1")).expect("Start");
        }

        let rebaseline_failing = RecordingSeam::failing_on_nth_occurrence("flush", 2);
        drive(&git_dir, &rebaseline_failing, &tool_error("edit", "call-1"))
            .expect("a terminal failure whose rebaseline flush fails still returns");

        let seeded = state::read_state(&git_dir).expect("state readable");
        assert!(
            seeded.attempts.is_empty(),
            "the abandon succeeded so the attempt was removed before the rebaseline flush failed"
        );
        assert!(matches!(seeded.recovery, RecoveryState::Pending { .. }));

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Recovering
        );

        let healthy = RecordingSeam::new();
        drive(&git_dir, &healthy, &tool_before("edit", "call-2")).expect("retry admits new work");

        let resolved = state::read_state(&git_dir).expect("state readable");
        assert!(resolved.recovery.is_clear());
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Healthy
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn orphaned_flushing_with_pending_abandon_attempts_is_recovering_and_reclaimed_by_the_next_boundary(
    ) {
        let git_dir = unique_test_git_dir("orphaned-flushing-recovering");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let attempt = state::seed_attempt_for_tests(
            &git_dir,
            &key("call-stuck"),
            "write",
            AttemptPhase::Active,
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
        drive(&git_dir, &healthy, &tool_before("write", "call-new"))
            .expect("the orphaned flush is reclaimed and retried, then the new call is admitted");

        let resolved = state::read_state(&git_dir).expect("state readable");
        assert!(resolved.recovery.is_clear());
        assert!(resolved.attempts.iter().all(|a| a.call_id != "call-stuck"));
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
                "version": 1,
                "next_recovery_generation": 1,
                "recovery": { "phase": "clear" },
                "attempts": [{
                    "scope_id": "oc-tool-v1|s=8:ses-main|c=6:call-1",
                    "session_id": "ses-main",
                    "call_id": "call-1",
                    "tool_name": "write",
                    "phase": "pending_abandon",
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

    fn matrix_attempt(call_id: &str, phase: AttemptPhase) -> state::AdapterAttempt {
        let key = AttemptKey {
            session_id: "ses-main".to_string(),
            call_id: call_id.to_string(),
        };

        state::AdapterAttempt {
            scope_id: super::super::format_opencode_scope_id(&key),
            session_id: key.session_id,
            call_id: key.call_id,
            tool_name: "write".to_string(),
            phase,
            owner: None,
        }
    }

    fn write_matrix_state(
        git_dir: &Path,
        recovery: RecoveryState,
        has_pending_abandon: bool,
        has_pending_start: bool,
    ) {
        std::fs::create_dir_all(state::adapter_state_dir(git_dir))
            .expect("adapter state dir should be created");

        let mut attempts = vec![matrix_attempt("call-active", AttemptPhase::Active)];
        if has_pending_abandon {
            attempts.push(matrix_attempt("call-abandon", AttemptPhase::PendingAbandon));
        }
        if has_pending_start {
            attempts.push(matrix_attempt("call-start", AttemptPhase::PendingStart));
        }

        let state = state::AdapterState {
            version: 1,
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
    fn health_classification_matrix_covers_all_twelve_recovery_and_attempt_phase_combinations() {
        use MutationScopeHealthStatus::{Blocked, Healthy, Invalid, Recovering};

        let clear = RecoveryState::Clear;
        let pending = RecoveryState::Pending { generation: 1 };
        let flushing = RecoveryState::Flushing { generation: 1 };

        let rows = [
            (clear, false, false, Healthy),
            (clear, false, true, Blocked),
            (clear, true, false, Invalid),
            (clear, true, true, Invalid),
            (pending, false, false, Recovering),
            (pending, false, true, Blocked),
            (pending, true, false, Recovering),
            (pending, true, true, Blocked),
            (flushing, false, false, Recovering),
            (flushing, false, true, Blocked),
            (flushing, true, false, Recovering),
            (flushing, true, true, Blocked),
        ];

        for (index, (recovery, has_pending_abandon, has_pending_start, expected)) in
            rows.into_iter().enumerate()
        {
            let git_dir = unique_test_git_dir(&format!("matrix-{index}"));
            write_matrix_state(&git_dir, recovery, has_pending_abandon, has_pending_start);

            assert_eq!(
                classify_health(&git_dir).status,
                expected,
                "row {index}: recovery={recovery:?} has_pending_abandon={has_pending_abandon} \
                 has_pending_start={has_pending_start}",
            );

            remove_test_git_dir(&git_dir);
        }
    }

    fn dead_owner() -> crate::services::hooks::mutation_scope_owner::ProcessOwner {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawning 'true' should succeed");
        let pid = i32::try_from(child.id()).expect("pid fits in i32");
        child.wait().expect("child should exit and be reaped");
        crate::services::hooks::mutation_scope_owner::ProcessOwner {
            pid,
            instance_token: None,
        }
    }

    fn live_owner() -> crate::services::hooks::mutation_scope_owner::ProcessOwner {
        crate::services::hooks::mutation_scope_owner::current_process_owner()
    }

    fn seed_pending_start_with_owner(
        git_dir: &Path,
        call_id: &str,
        owner: Option<crate::services::hooks::mutation_scope_owner::ProcessOwner>,
    ) -> state::AdapterAttempt {
        let attempt = state::seed_attempt_for_tests(
            git_dir,
            &key(call_id),
            "write",
            AttemptPhase::PendingStart,
        );
        state::set_attempt_owner_for_tests(git_dir, &attempt.scope_id, owner)
    }

    #[test]
    fn assess_repairability_is_manual_only_when_the_adapter_is_not_blocked() {
        let git_dir = unique_test_git_dir("assess-not-blocked");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        assert_eq!(assess_repairability(&git_dir), Repairability::ManualOnly);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn assess_repairability_is_manual_only_for_a_legacy_pending_start_attempt_with_no_recorded_owner(
    ) {
        let git_dir = unique_test_git_dir("assess-legacy-no-owner");
        let dir = state::adapter_state_dir(&git_dir);
        std::fs::create_dir_all(&dir).expect("state dir should be created");
        std::fs::write(
            state::state_path(&git_dir),
            serde_json::json!({
                "version": 1,
                "next_recovery_generation": 1,
                "recovery": { "phase": "clear" },
                "attempts": [{
                    "scope_id": "oc-tool-v1|s=8:ses-main|c=6:call-1",
                    "session_id": "ses-main",
                    "call_id": "call-1",
                    "tool_name": "write",
                    "phase": "pending_start",
                }],
            })
            .to_string(),
        )
        .expect("legacy state file with no owner field should be writable");

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked,
            "a legacy state file predating owner evidence must still classify Blocked"
        );
        assert_eq!(
            assess_repairability(&git_dir),
            Repairability::ManualOnly,
            "no recorded owner is never treated as proof of death"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn assess_repairability_is_manual_only_when_the_pending_start_owner_is_live() {
        let git_dir = unique_test_git_dir("assess-live-owner");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        seed_pending_start_with_owner(&git_dir, "call-1", Some(live_owner()));

        assert_eq!(assess_repairability(&git_dir), Repairability::ManualOnly);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn assess_repairability_is_auto_fixable_when_every_pending_start_owner_is_positively_dead() {
        let git_dir = unique_test_git_dir("assess-dead-owner");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        seed_pending_start_with_owner(&git_dir, "call-1", Some(dead_owner()));

        assert_eq!(assess_repairability(&git_dir), Repairability::AutoFixable);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn assess_repairability_is_manual_only_when_one_of_several_pending_start_owners_is_live() {
        let git_dir = unique_test_git_dir("assess-mixed-owners");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        seed_pending_start_with_owner(&git_dir, "call-dead", Some(dead_owner()));
        seed_pending_start_with_owner(&git_dir, "call-live", Some(live_owner()));

        assert_eq!(
            assess_repairability(&git_dir),
            Repairability::ManualOnly,
            "every contributing PendingStart attempt must have a proven-dead owner"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn repair_blocked_clears_a_dead_owner_pending_start_end_to_end() {
        let git_dir = unique_test_git_dir("repair-end-to-end");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        seed_pending_start_with_owner(&git_dir, "call-1", Some(dead_owner()));

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked
        );

        let healthy = RecordingSeam::new();
        let repository_root = Path::new(CWD);
        let outcome = super::super::lifecycle::repair_blocked(
            &git_dir,
            repository_root,
            None,
            &|_root: &Path, payload: &str, _logger: Option<&dyn Logger>| healthy.handle(payload),
        )
        .expect("repair should not error");

        assert_eq!(outcome, super::super::lifecycle::RepairOutcome::Repaired);
        let final_status = classify_health(&git_dir).status;
        assert!(
            matches!(
                final_status,
                MutationScopeHealthStatus::Healthy | MutationScopeHealthStatus::Recovering
            ),
            "a reported repair must never leave the final health Blocked: {final_status:?}"
        );
        assert!(state::read_state(&git_dir)
            .expect("state readable")
            .attempts
            .is_empty());

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn repair_blocked_is_a_safe_no_op_when_the_pending_start_owner_is_live() {
        let git_dir = unique_test_git_dir("repair-live-owner-noop");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let attempt = seed_pending_start_with_owner(&git_dir, "call-1", Some(live_owner()));

        let seam = RecordingSeam::new();
        let repository_root = Path::new(CWD);
        let outcome = super::super::lifecycle::repair_blocked(
            &git_dir,
            repository_root,
            None,
            &|_root: &Path, payload: &str, _logger: Option<&dyn Logger>| seam.handle(payload),
        )
        .expect("repair should not error");

        assert_eq!(outcome, super::super::lifecycle::RepairOutcome::NoOp);
        assert!(
            seam.calls.lock().expect("seam mutex").is_empty(),
            "a live owner must never trigger any seam call"
        );
        let state = state::read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].scope_id, attempt.scope_id);
        assert_eq!(state.attempts[0].phase, AttemptPhase::PendingStart);
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn repair_blocked_refuses_to_abandon_an_attempt_a_concurrent_process_already_started_before_the_lock_is_acquired(
    ) {
        let git_dir = unique_test_git_dir("repair-concurrent-race");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let attempt = seed_pending_start_with_owner(&git_dir, "call-1", Some(dead_owner()));

        assert_eq!(assess_repairability(&git_dir), Repairability::AutoFixable);

        state::mark_active(&git_dir, &attempt.scope_id)
            .expect("simulating the concurrent owning process completing its own Start");

        let seam = RecordingSeam::new();
        let repository_root = Path::new(CWD);
        let outcome = super::super::lifecycle::repair_blocked(
            &git_dir,
            repository_root,
            None,
            &|_root: &Path, payload: &str, _logger: Option<&dyn Logger>| seam.handle(payload),
        )
        .expect("repair should not error");

        assert_eq!(
            outcome,
            super::super::lifecycle::RepairOutcome::NoOp,
            "the fresh, lock-protected re-proof must refuse to act on state assessed before it changed"
        );
        assert!(
            seam.calls.lock().expect("seam mutex").is_empty(),
            "no seam call may fire once the attempt is no longer PendingStart"
        );
        let state = state::read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].phase, AttemptPhase::Active);
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Healthy,
            "the concurrently-started attempt must be left exactly as the owning process left it"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn repair_blocked_is_all_or_nothing_when_auto_fixable_assessment_becomes_stale() {
        let git_dir = unique_test_git_dir("repair-all-or-nothing-stale");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let attempt0 = seed_pending_start_with_owner(&git_dir, "call-0", Some(dead_owner()));
        let attempt1 = seed_pending_start_with_owner(&git_dir, "call-1", Some(dead_owner()));

        assert_eq!(
            assess_repairability(&git_dir),
            Repairability::AutoFixable,
            "doctor's initial, unlocked assessment sees both owners positively dead"
        );

        state::set_attempt_owner_for_tests(&git_dir, &attempt1.scope_id, Some(live_owner()));

        let seam = RecordingSeam::new();
        let repository_root = Path::new(CWD);
        let outcome = super::super::lifecycle::repair_blocked(
            &git_dir,
            repository_root,
            None,
            &|_root: &Path, payload: &str, _logger: Option<&dyn Logger>| seam.handle(payload),
        )
        .expect("repair should not error");

        assert_eq!(
            outcome,
            super::super::lifecycle::RepairOutcome::NoOp,
            "the fresh repair-time re-proof must reject the whole batch once any current \
             PendingStart owner is no longer positively dead"
        );

        let state = state::read_state(&git_dir).expect("state readable");
        assert_eq!(
            state
                .attempts
                .iter()
                .find(|a| a.scope_id == attempt0.scope_id)
                .expect("attempt0 is still tracked")
                .phase,
            AttemptPhase::PendingStart,
            "the still-dead attempt0 must not be transitioned to PendingAbandon when another \
             current blocker fails the all-dead proof"
        );
        assert_eq!(
            state
                .attempts
                .iter()
                .find(|a| a.scope_id == attempt1.scope_id)
                .expect("attempt1 is still tracked")
                .phase,
            AttemptPhase::PendingStart,
        );

        assert!(
            state.recovery.is_clear(),
            "a failed re-proof must perform no durable recovery transition: {:?}",
            state.recovery
        );

        assert!(
            seam.calls.lock().expect("seam mutex").is_empty(),
            "the state transaction must return None before resolve_recovery is entered, so no \
             seam call (flush/abandon) may fire"
        );

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked,
            "both attempts remain PendingStart, so the adapter stays Blocked"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn repair_blocked_interrupted_before_the_seam_resolves_leaves_state_the_ordinary_recovery_path_completes_without_duplication(
    ) {
        let git_dir = unique_test_git_dir("repair-interrupted-resume");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        seed_pending_start_with_owner(&git_dir, "call-1", Some(dead_owner()));

        let failing_abandon = RecordingSeam::failing_on(&["abandon"]);
        let repository_root = Path::new(CWD);
        let outcome = super::super::lifecycle::repair_blocked(
            &git_dir,
            repository_root,
            None,
            &|_root: &Path, payload: &str, _logger: Option<&dyn Logger>| {
                failing_abandon.handle(payload)
            },
        )
        .expect("repair should not error even though the seam abandon call fails");

        assert_eq!(outcome, super::super::lifecycle::RepairOutcome::NoOp);
        let interrupted = state::read_state(&git_dir).expect("state readable");
        assert_eq!(interrupted.attempts.len(), 1);
        assert_eq!(interrupted.attempts[0].phase, AttemptPhase::PendingAbandon);
        assert!(
            matches!(interrupted.recovery, RecoveryState::Pending { .. }),
            "a failed seam call during repair must relinquish Flushing back to Pending: {:?}",
            interrupted.recovery
        );
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Recovering,
            "an interrupted repair must never remain Blocked"
        );

        let healthy = RecordingSeam::new();
        drive(&git_dir, &healthy, &tool_before("write", "call-2"))
            .expect("the ordinary recovery path resumes the interrupted repair automatically");

        let resolved = state::read_state(&git_dir).expect("state readable");
        assert!(resolved.recovery.is_clear());
        assert_eq!(
            resolved.attempts.len(),
            1,
            "the interrupted repair's attempt must not be duplicated"
        );
        assert_eq!(resolved.attempts[0].call_id, "call-2");
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Healthy
        );

        remove_test_git_dir(&git_dir);
    }
}
