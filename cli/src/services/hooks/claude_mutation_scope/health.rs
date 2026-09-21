use std::path::Path;

use crate::services::hooks::mutation_scope_health::{
    MutationScopeAdapterHealth, MutationScopeHealthStatus,
};
use crate::services::mutation_trace::types::ActorKind;

use super::state;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Repairability {
    AutoFixable,
    ManualOnly,
}

pub(crate) fn assess_repairability(git_dir: &Path) -> Repairability {
    let Ok(state) = state::read_state(git_dir) else {
        return Repairability::ManualOnly;
    };

    if !state.recovery_pending || state.attempts.is_empty() {
        return Repairability::ManualOnly;
    }

    let every_attempt_is_pending_abandon = state
        .attempts
        .iter()
        .all(|attempt| attempt.phase == state::AttemptPhase::PendingAbandon);

    if every_attempt_is_pending_abandon {
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
                ActorKind::ClaudeCode,
                MutationScopeHealthStatus::Invalid,
                "Claude mutation-scope state file could not be read or parsed.",
            )
            .with_detail(error.to_string());
        }
    };

    if !state.recovery_pending {
        return MutationScopeAdapterHealth::new(
            ActorKind::ClaudeCode,
            MutationScopeHealthStatus::Healthy,
            "No persisted recovery condition blocks mutation-capable admission.",
        );
    }

    if state.attempts.is_empty() {
        return MutationScopeAdapterHealth::new(
            ActorKind::ClaudeCode,
            MutationScopeHealthStatus::Recovering,
            "Recovery is pending with no unresolved attempts; the next tracked PreToolUse call flushes and clears it automatically.",
        );
    }

    MutationScopeAdapterHealth::new(
        ActorKind::ClaudeCode,
        MutationScopeHealthStatus::Blocked,
        "Recovery is pending with unresolved attempts; the recovery barrier's flush path only runs once attempts are empty, so future tracked PreToolUse calls deny without self-clearing.",
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use anyhow::anyhow;

    use super::super::{
        abandon_attempt, apply_recovery_barrier, cleanup_attempts_matching, repair_blocked,
        AttemptKey, BarrierOutcome, RepairOutcome,
    };
    use super::*;
    use crate::services::observability::traits::Logger;

    static NEXT_TEST_GIT_DIR_ID: AtomicU64 = AtomicU64::new(0);

    fn unique_test_git_dir(label: &str) -> PathBuf {
        let id = NEXT_TEST_GIT_DIR_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "sce-claude-mutation-scope-health-{label}-{}-{id}",
            std::process::id()
        ))
    }

    fn remove_test_git_dir(git_dir: &Path) {
        let _ = std::fs::remove_dir_all(git_dir);
    }

    fn key(tool_use_id: &str) -> AttemptKey {
        AttemptKey {
            session_id: "session-1".to_string(),
            agent_id: None,
            tool_use_id: tool_use_id.to_string(),
        }
    }

    fn failing_seam(
        _root: &Path,
        _payload: &str,
        _logger: Option<&dyn Logger>,
    ) -> anyhow::Result<String> {
        Err(anyhow!("seam failure injected by test"))
    }

    fn unreachable_seam(
        _root: &Path,
        payload: &str,
        _logger: Option<&dyn Logger>,
    ) -> anyhow::Result<String> {
        panic!(
            "the ingress seam must not be called while the recovery barrier is armed with non-empty attempts: {payload}"
        );
    }

    #[test]
    fn absent_state_file_is_healthy() {
        let git_dir = unique_test_git_dir("absent");

        let health = classify_health(&git_dir);

        assert_eq!(health.adapter, ActorKind::ClaudeCode);
        assert_eq!(health.status, MutationScopeHealthStatus::Healthy);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn recovery_pending_false_is_healthy_even_with_live_attempts() {
        let git_dir = unique_test_git_dir("recovery-false");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        state::allocate_attempt(&git_dir, &key("toolu_1"), "Write")
            .expect("allocation should succeed");

        let health = classify_health(&git_dir);

        assert_eq!(health.status, MutationScopeHealthStatus::Healthy);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn recovery_pending_true_with_empty_attempts_is_recovering() {
        let git_dir = unique_test_git_dir("recovering");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        state::mark_recovery_pending(&git_dir).expect("marking recovery pending should succeed");

        let health = classify_health(&git_dir);

        assert_eq!(health.status, MutationScopeHealthStatus::Recovering);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn recovery_pending_true_with_non_empty_attempts_is_blocked() {
        let git_dir = unique_test_git_dir("blocked");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        state::allocate_attempt(&git_dir, &key("toolu_1"), "Write")
            .expect("allocation should succeed");
        state::mark_recovery_pending(&git_dir).expect("marking recovery pending should succeed");

        let health = classify_health(&git_dir);

        assert_eq!(health.status, MutationScopeHealthStatus::Blocked);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn malformed_state_file_is_invalid_with_the_read_error_surfaced() {
        let git_dir = unique_test_git_dir("malformed");
        let path = state::state_path(&git_dir);
        std::fs::create_dir_all(path.parent().expect("state path has a parent"))
            .expect("state dir should be created");
        std::fs::write(&path, b"not json").expect("malformed file should be writable");

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

    #[test]
    fn stale_non_empty_attempts_after_a_failed_abandon_stays_blocked_across_repeated_pre_tool_use_ac3(
    ) {
        let git_dir = unique_test_git_dir("blocked-regression");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let repository_root = git_dir.as_path();

        let allocated = state::allocate_attempt(&git_dir, &key("toolu_1"), "Write")
            .expect("allocating the live attempt should succeed");

        let error = abandon_attempt(
            &git_dir,
            repository_root,
            &allocated.attempt,
            None,
            &failing_seam,
        )
        .expect_err("the injected abandon seam failure must propagate");
        assert!(error.to_string().contains("seam failure"));

        let seeded_state = state::read_state(&git_dir).expect("state should be readable");
        assert!(
            seeded_state.recovery_pending,
            "mark_recovery_pending must have run before the seam call failed"
        );
        assert_eq!(
            seeded_state.attempts.len(),
            1,
            "remove_attempt must never have run because the seam call failed"
        );
        assert_eq!(
            seeded_state.attempts[0].phase,
            state::AttemptPhase::PendingAbandon,
            "the attempt's abandon intent must be durably persisted before the seam call ran"
        );

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked
        );

        for attempt_number in 1..=2 {
            let outcome =
                apply_recovery_barrier(&git_dir, repository_root, None, &unreachable_seam);
            assert!(
                matches!(outcome, BarrierOutcome::Deny),
                "PreToolUse call #{attempt_number} must be denied without self-clearing"
            );
        }

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked,
            "the classifier must still report Blocked after repeated denial"
        );

        assert_eq!(
            assess_repairability(&git_dir),
            Repairability::AutoFixable,
            "a PendingAbandon attempt is an already-established abandon decision, safe to retry"
        );

        let healthy =
            |_root: &Path, _payload: &str, _logger: Option<&dyn Logger>| Ok(String::new());
        let outcome = repair_blocked(&git_dir, repository_root, None, &healthy)
            .expect("repair should not error");
        assert_eq!(outcome, RepairOutcome::Repaired);

        let final_status = classify_health(&git_dir).status;
        assert!(
            matches!(
                final_status,
                MutationScopeHealthStatus::Healthy | MutationScopeHealthStatus::Recovering
            ),
            "AC3/AC6: a reported repair must never leave the final health Blocked: {final_status:?}"
        );
        assert!(state::read_state(&git_dir)
            .expect("state readable")
            .attempts
            .is_empty());

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn mark_active_losing_the_race_against_an_established_pending_abandon_leaves_repairable_terminal_evidence(
    ) {
        let git_dir = unique_test_git_dir("mark-active-race");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let repository_root = git_dir.as_path();

        let allocated = state::allocate_attempt(&git_dir, &key("toolu_1"), "Write")
            .expect("allocation should succeed");

        state::mark_recovery_pending_and_pending_abandon(
            &git_dir,
            std::slice::from_ref(&allocated.attempt.scope_id),
        )
        .expect("atomic establishment should succeed");

        let error = state::mark_active(&git_dir, &allocated.attempt.scope_id)
            .expect_err("mark_active must fail once abandonment has been established");
        assert!(error.to_string().contains("abandon"));

        let state_after = state::read_state(&git_dir).expect("state readable");
        assert!(
            state_after.recovery_pending,
            "recovery_pending must remain true after losing the activation race"
        );
        assert_eq!(
            state_after.attempts[0].phase,
            state::AttemptPhase::PendingAbandon,
            "losing the activation race must leave PendingAbandon intact, not resurrect Active"
        );

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked
        );
        assert_eq!(
            assess_repairability(&git_dir),
            Repairability::AutoFixable,
            "losing the activation race must leave repairable established terminal evidence, \
             not fall back to ManualOnly"
        );

        let healthy =
            |_root: &Path, _payload: &str, _logger: Option<&dyn Logger>| Ok(String::new());
        let outcome = repair_blocked(&git_dir, repository_root, None, &healthy)
            .expect("repair should not error");
        assert_eq!(outcome, RepairOutcome::Repaired);

        let resolved = state::read_state(&git_dir).expect("state readable");
        assert!(resolved.attempts.is_empty());
        assert!(!resolved.recovery_pending);
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Healthy
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn repair_blocked_removes_only_successfully_abandoned_attempts_and_keeps_recovery_pending_when_one_fails(
    ) {
        let git_dir = unique_test_git_dir("repair-partial-batch");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let repository_root = git_dir.as_path();

        let first = state::allocate_attempt(&git_dir, &key("toolu-1"), "Write")
            .expect("first allocation should succeed");
        let second = state::allocate_attempt(&git_dir, &key("toolu-2"), "Write")
            .expect("second allocation should succeed");

        state::mark_recovery_pending_and_pending_abandon(
            &git_dir,
            &[
                first.attempt.scope_id.clone(),
                second.attempt.scope_id.clone(),
            ],
        )
        .expect("atomic establishment should succeed");

        let fail_first =
            |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> anyhow::Result<String> {
                if payload.contains(&first.attempt.scope_id) {
                    return Err(anyhow!(
                        "seam failure injected by test for the first attempt"
                    ));
                }
                Ok(String::new())
            };

        let outcome = repair_blocked(&git_dir, repository_root, None, &fail_first)
            .expect("repair should not error even though one seam call fails");
        assert_eq!(outcome, RepairOutcome::NoOp);

        let state = state::read_state(&git_dir).expect("state readable");
        assert_eq!(
            state.attempts.len(),
            1,
            "the successfully abandoned attempt must be removed"
        );
        assert_eq!(state.attempts[0].scope_id, first.attempt.scope_id);
        assert_eq!(state.attempts[0].phase, state::AttemptPhase::PendingAbandon);
        assert!(
            state.recovery_pending,
            "the barrier must remain armed while any attempt is still unresolved"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn assess_repairability_is_manual_only_when_the_adapter_is_not_blocked() {
        let git_dir = unique_test_git_dir("assess-not-blocked");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        assert_eq!(assess_repairability(&git_dir), Repairability::ManualOnly);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn assess_repairability_is_manual_only_for_a_legacy_attempt_never_marked_pending_abandon() {
        let git_dir = unique_test_git_dir("assess-legacy-no-pending-abandon");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        state::allocate_attempt(&git_dir, &key("toolu_1"), "Write")
            .expect("allocation should succeed");
        state::mark_recovery_pending(&git_dir).expect("marking recovery pending should succeed");

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked,
            "a legacy attempt left PendingStart under a stale recovery flag must still classify Blocked"
        );
        assert_eq!(
            assess_repairability(&git_dir),
            Repairability::ManualOnly,
            "an attempt with no established abandon intent is never treated as repairable"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn assess_repairability_is_manual_only_when_one_of_several_attempts_has_no_established_abandon_intent(
    ) {
        let git_dir = unique_test_git_dir("assess-mixed-phase");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let repository_root = git_dir.as_path();

        let doomed = state::allocate_attempt(&git_dir, &key("toolu-doomed"), "Write")
            .expect("allocating the doomed attempt should succeed");
        abandon_attempt(
            &git_dir,
            repository_root,
            &doomed.attempt,
            None,
            &failing_seam,
        )
        .expect_err("the injected abandon seam failure must propagate");

        state::allocate_attempt(&git_dir, &key("toolu-live"), "Write")
            .expect("allocating the untouched live attempt should succeed");

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked
        );
        assert_eq!(
            assess_repairability(&git_dir),
            Repairability::ManualOnly,
            "every attempt must be PendingAbandon before doctor may repair; a live attempt \
             with no established abandon intent must never be touched"
        );

        let outcome = repair_blocked(&git_dir, repository_root, None, &unreachable_seam)
            .expect("repair should not error");
        assert_eq!(
            outcome,
            RepairOutcome::NoOp,
            "a coexisting attempt with no established abandon intent must block the whole repair"
        );
        let untouched = state::read_state(&git_dir).expect("state readable");
        assert_eq!(
            untouched.attempts.len(),
            2,
            "neither attempt may be touched while any one lacks PendingAbandon evidence"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn repair_blocked_is_a_safe_no_op_when_an_attempt_is_no_longer_pending_abandon_by_the_time_the_lock_is_acquired(
    ) {
        let git_dir = unique_test_git_dir("repair-concurrent-race");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let repository_root = git_dir.as_path();

        let allocated = state::allocate_attempt(&git_dir, &key("toolu_1"), "Write")
            .expect("allocation should succeed");
        abandon_attempt(
            &git_dir,
            repository_root,
            &allocated.attempt,
            None,
            &failing_seam,
        )
        .expect_err("the injected abandon seam failure must propagate");

        assert_eq!(assess_repairability(&git_dir), Repairability::AutoFixable);

        state::set_attempt_phase_for_tests(
            &git_dir,
            &allocated.attempt.scope_id,
            state::AttemptPhase::Active,
        );

        let outcome = repair_blocked(&git_dir, repository_root, None, &unreachable_seam)
            .expect("repair should not error");

        assert_eq!(
            outcome,
            RepairOutcome::NoOp,
            "the fresh, lock-protected re-proof must refuse to act on state assessed before it changed"
        );
        let state = state::read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].phase, state::AttemptPhase::Active);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn repair_blocked_interrupted_by_a_failing_seam_leaves_state_a_later_repair_completes_without_duplication(
    ) {
        let git_dir = unique_test_git_dir("repair-interrupted-resume");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let repository_root = git_dir.as_path();

        let allocated = state::allocate_attempt(&git_dir, &key("toolu_1"), "Write")
            .expect("allocation should succeed");
        abandon_attempt(
            &git_dir,
            repository_root,
            &allocated.attempt,
            None,
            &failing_seam,
        )
        .expect_err("the injected abandon seam failure must propagate");

        let outcome = repair_blocked(&git_dir, repository_root, None, &failing_seam)
            .expect("repair should not error even though the seam abandon call fails again");
        assert_eq!(outcome, RepairOutcome::NoOp);

        let interrupted = state::read_state(&git_dir).expect("state readable");
        assert_eq!(interrupted.attempts.len(), 1);
        assert_eq!(
            interrupted.attempts[0].phase,
            state::AttemptPhase::PendingAbandon
        );
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked,
            "an interrupted repair must remain retryable, not resurrect or duplicate the attempt"
        );

        let healthy =
            |_root: &Path, _payload: &str, _logger: Option<&dyn Logger>| Ok(String::new());
        let outcome = repair_blocked(&git_dir, repository_root, None, &healthy)
            .expect("the later repair should complete without error");
        assert_eq!(outcome, RepairOutcome::Repaired);

        let resolved = state::read_state(&git_dir).expect("state readable");
        assert!(resolved.attempts.is_empty());
        assert!(!resolved.recovery_pending);
        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Healthy
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn cleanup_attempts_matching_durably_marks_every_matched_attempt_pending_abandon_before_any_seam_call_even_when_the_first_fails(
    ) {
        let git_dir = unique_test_git_dir("cleanup-batch-mark");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let repository_root = git_dir.as_path();

        let first = state::allocate_attempt(&git_dir, &key("toolu-1"), "Write")
            .expect("first allocation should succeed");
        let second = state::allocate_attempt(&git_dir, &key("toolu-2"), "Write")
            .expect("second allocation should succeed");

        let failing_on_first =
            |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> anyhow::Result<String> {
                if payload.contains(&first.attempt.scope_id) {
                    return Err(anyhow!(
                        "seam failure injected by test for the first attempt"
                    ));
                }
                Ok(String::new())
            };

        let error = cleanup_attempts_matching(
            &git_dir,
            repository_root,
            None,
            &failing_on_first,
            |_attempt| true,
        )
        .expect_err("a failure abandoning one attempt must still surface an error");
        assert!(error.to_string().contains("seam failure"));

        let seeded = state::read_state(&git_dir).expect("state readable");
        let remaining_first = seeded
            .attempts
            .iter()
            .find(|attempt| attempt.scope_id == first.attempt.scope_id)
            .expect("the failed attempt must still be tracked, not lost");
        assert_eq!(
            remaining_first.phase,
            state::AttemptPhase::PendingAbandon,
            "AC3/T04: every matched attempt must be durably marked before any seam call, \
             so the still-blocked first attempt keeps its retryable evidence"
        );
        assert!(
            seeded
                .attempts
                .iter()
                .all(|attempt| attempt.scope_id != second.attempt.scope_id),
            "the second attempt's own seam call must still have been attempted and succeeded"
        );

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked
        );
        assert_eq!(assess_repairability(&git_dir), Repairability::AutoFixable);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn apply_recovery_barrier_fails_closed_when_a_new_obligation_is_established_while_the_flush_seam_is_in_flight(
    ) {
        let git_dir = unique_test_git_dir("barrier-race-new-obligation");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let repository_root = git_dir.as_path();

        state::mark_recovery_pending(&git_dir).expect("arming the barrier should succeed");

        let racing_seam = |_root: &Path,
                           _payload: &str,
                           _logger: Option<&dyn Logger>|
         -> anyhow::Result<String> {
            let raced = state::allocate_attempt(&git_dir, &key("toolu-raced-in"), "Write")
                .expect("the racing allocation should succeed");
            state::mark_recovery_pending_and_pending_abandon(
                &git_dir,
                std::slice::from_ref(&raced.attempt.scope_id),
            )
            .expect("the racing establishment should succeed");
            Ok(String::new())
        };

        let outcome = apply_recovery_barrier(&git_dir, repository_root, None, &racing_seam);
        assert!(
            matches!(outcome, BarrierOutcome::Deny),
            "a fresh obligation established during the flush must fail the barrier closed, \
             not proceed on the stale pre-flush proof"
        );

        let after = state::read_state(&git_dir).expect("state readable");
        assert!(
            after.recovery_pending,
            "RecoveryNeverClearedWithUnresolvedAbandon: recovery must remain armed"
        );
        assert_eq!(after.attempts.len(), 1);
        assert_eq!(after.attempts[0].phase, state::AttemptPhase::PendingAbandon);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn repair_blocked_fails_closed_when_a_new_obligation_is_established_while_abandoning_another() {
        let git_dir = unique_test_git_dir("repair-race-new-obligation");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let repository_root = git_dir.as_path();

        let original = state::allocate_attempt(&git_dir, &key("toolu-original"), "Write")
            .expect("allocation should succeed");
        state::mark_recovery_pending_and_pending_abandon(
            &git_dir,
            std::slice::from_ref(&original.attempt.scope_id),
        )
        .expect("atomic establishment should succeed");

        let racing_seam = |_root: &Path,
                           _payload: &str,
                           _logger: Option<&dyn Logger>|
         -> anyhow::Result<String> {
            let raced = state::allocate_attempt(&git_dir, &key("toolu-raced-in"), "Write")
                .expect("the racing allocation should succeed");
            state::mark_recovery_pending_and_pending_abandon(
                &git_dir,
                std::slice::from_ref(&raced.attempt.scope_id),
            )
            .expect("the racing establishment should succeed");
            Ok(String::new())
        };

        let outcome = repair_blocked(&git_dir, repository_root, None, &racing_seam)
            .expect("repair should not error");
        assert_eq!(
            outcome,
            RepairOutcome::NoOp,
            "AC6: repair must not report Repaired when the atomic clear discovers another \
             unresolved obligation, even though its own abandon seam call succeeded"
        );

        let after = state::read_state(&git_dir).expect("state readable");
        assert!(
            after.recovery_pending,
            "RecoveryNeverClearedWithUnresolvedAbandon: recovery must remain armed while the \
             raced-in obligation is unresolved"
        );
        assert!(
            after
                .attempts
                .iter()
                .all(|attempt| attempt.scope_id != original.attempt.scope_id),
            "the original attempt's own abandon must still have completed"
        );
        assert_eq!(after.attempts.len(), 1);
        assert_eq!(after.attempts[0].phase, state::AttemptPhase::PendingAbandon);

        remove_test_git_dir(&git_dir);
    }
}
