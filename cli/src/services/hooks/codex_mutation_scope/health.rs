use std::path::Path;

use crate::services::hooks::mutation_scope_health::{
    MutationScopeAdapterHealth, MutationScopeHealthStatus,
};
use crate::services::mutation_trace::types::ActorKind;

use super::state::{self, RecoveryState};

pub(crate) fn classify_health(git_dir: &Path) -> MutationScopeAdapterHealth {
    let state = match state::read_state(git_dir) {
        Ok(state) => state,
        Err(error) => {
            return MutationScopeAdapterHealth::new(
                ActorKind::Codex,
                MutationScopeHealthStatus::Invalid,
                "Codex mutation-scope state file could not be read or parsed.",
            )
            .with_detail(error.to_string());
        }
    };

    match state.recovery {
        RecoveryState::Clear => MutationScopeAdapterHealth::new(
            ActorKind::Codex,
            MutationScopeHealthStatus::Healthy,
            "No persisted recovery condition blocks mutation-capable admission.",
        ),
        RecoveryState::Pending { .. } if state.attempts.is_empty() => {
            MutationScopeAdapterHealth::new(
                ActorKind::Codex,
                MutationScopeHealthStatus::Recovering,
                "Recovery is pending with no unresolved attempts; the next tracked PreToolUse call claims and completes the flush automatically.",
            )
        }
        RecoveryState::Pending { .. } => MutationScopeAdapterHealth::new(
            ActorKind::Codex,
            MutationScopeHealthStatus::Recovering,
            "Recovery is pending with unresolved attempts; an unrelated tracked PreToolUse remains denied by the global recovery barrier, but a later tracked PreToolUse in the same (session_id, turn_id) lane retries the stale predecessor's abandonment through the ordinary same-lane sweep, which can clear the attempt and advance recovery to a flush without manual intervention.",
        ),
        RecoveryState::Flushing { .. } if state.attempts.is_empty() => {
            MutationScopeAdapterHealth::new(
                ActorKind::Codex,
                MutationScopeHealthStatus::Recovering,
                "A recovery flush is in progress; an orphaned flush is reclaimed and its flush retried automatically on the next tracked PreToolUse boundary.",
            )
        }
        RecoveryState::Flushing { .. } => MutationScopeAdapterHealth::new(
            ActorKind::Codex,
            MutationScopeHealthStatus::Invalid,
            "Persisted state has a recovery flush in progress with unresolved attempts outstanding, a combination the adapter's state machine cannot legitimately produce.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::super::AttemptKey;
    use super::*;

    static NEXT_TEST_GIT_DIR_ID: AtomicU64 = AtomicU64::new(0);

    fn unique_test_git_dir(label: &str) -> PathBuf {
        let id = NEXT_TEST_GIT_DIR_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "sce-codex-mutation-scope-health-{label}-{}-{id}",
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

    #[test]
    fn absent_state_file_is_healthy() {
        let git_dir = unique_test_git_dir("absent");

        let health = classify_health(&git_dir);

        assert_eq!(health.adapter, ActorKind::Codex);
        assert_eq!(health.status, MutationScopeHealthStatus::Healthy);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn clear_recovery_is_healthy_even_with_live_attempts() {
        let git_dir = unique_test_git_dir("clear-with-attempts");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        state::seed_attempt_for_tests(
            &git_dir,
            &key("exec-1"),
            "turn-1",
            "Bash",
            state::AttemptPhase::Active,
        );

        let health = classify_health(&git_dir);

        assert_eq!(health.status, MutationScopeHealthStatus::Healthy);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn pending_recovery_with_empty_attempts_is_recovering_and_the_next_admission_claims_the_flush()
    {
        let git_dir = unique_test_git_dir("pending-empty-recovering");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let generation = state::arm_recovery(&git_dir).expect("arming recovery should succeed");

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Recovering
        );

        assert_eq!(
            state::admit_tracked_attempt(&git_dir, &key("exec-new"), "turn-1", "Bash")
                .expect("admit should not error"),
            state::AdmitDecision::FlushClaimed { generation },
            "Recovering must be proven by the next admission actually claiming the flush"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn pending_non_empty_recovery_denies_unrelated_admission_without_losing_the_recovery_path() {
        let git_dir = unique_test_git_dir("pending-non-empty-recovering");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        state::seed_attempt_for_tests(
            &git_dir,
            &key("exec-stuck"),
            "turn-1",
            "Bash",
            state::AttemptPhase::Active,
        );
        state::arm_recovery(&git_dir).expect("arming recovery should succeed");

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Recovering,
            "a proven same-lane self-healing path exists even though this state denies unrelated admission"
        );

        for attempt_number in 1..=2 {
            assert_eq!(
                state::admit_tracked_attempt(&git_dir, &key("exec-other"), "turn-2", "Bash")
                    .expect("admit should not error"),
                state::AdmitDecision::RecoveryBlocked,
                "admission #{attempt_number} in an unrelated lane must be denied without self-clearing"
            );
        }

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Recovering,
            "the classifier must still report Recovering after repeated unrelated denial; \
             Recovering does not mean every future call succeeds, only that a proven normal \
             self-healing route exists"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn orphaned_flushing_with_no_attempts_is_recovering_and_reclaimed_by_the_next_admission() {
        let git_dir = unique_test_git_dir("orphaned-flushing-recovering");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let generation = state::arm_recovery(&git_dir).expect("arming recovery should succeed");
        assert_eq!(
            state::admit_tracked_attempt(&git_dir, &key("seed"), "seed-turn", "Bash")
                .expect("seeding the flush claim should not error"),
            state::AdmitDecision::FlushClaimed { generation },
        );

        assert_eq!(
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Recovering
        );

        state::normalize_recovery_after_boundary_lock_acquired(&git_dir)
            .expect("normalize should succeed");
        assert_eq!(
            state::admit_tracked_attempt(&git_dir, &key("exec-new"), "turn-1", "Bash")
                .expect("admit should not error"),
            state::AdmitDecision::FlushClaimed { generation },
            "Recovering must be proven by the reclaimed flush being retried"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn flushing_with_unresolved_attempts_is_a_structurally_impossible_state_classified_invalid() {
        let git_dir = unique_test_git_dir("flushing-non-empty-invalid");
        let dir = state::adapter_state_dir(&git_dir);
        std::fs::create_dir_all(&dir).expect("state dir should be created");
        std::fs::write(
            state::state_path(&git_dir),
            serde_json::json!({
                "version": 3,
                "next_attempt_seq": 2,
                "next_recovery_generation": 2,
                "recovery": { "phase": "flushing", "generation": 1 },
                "attempts": [{
                    "attempt_seq": 1,
                    "scope_id": "cx-tool-v1|n=1|s=9:session-1|a=0:|t=6:exec-1",
                    "session_id": "session-1",
                    "turn_id": "turn-1",
                    "agent_id": null,
                    "tool_use_id": "exec-1",
                    "tool_name": "Bash",
                    "phase": "active",
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
}
