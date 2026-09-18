use std::path::Path;

use crate::services::hooks::mutation_scope_health::{
    MutationScopeAdapterHealth, MutationScopeHealthStatus,
};
use crate::services::mutation_trace::types::ActorKind;

use super::state;

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

    use super::super::{abandon_attempt, apply_recovery_barrier, AttemptKey, BarrierOutcome};
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
            classify_health(&git_dir).status,
            MutationScopeHealthStatus::Blocked
        );

        for attempt_number in 1..=2 {
            let outcome = apply_recovery_barrier(&git_dir, repository_root, None, &unreachable_seam);
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

        remove_test_git_dir(&git_dir);
    }
}
