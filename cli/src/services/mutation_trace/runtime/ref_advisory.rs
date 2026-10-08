#![allow(
    dead_code,
    reason = "maintenance entrypoints are wired by later tasks of context/plans/mutation-cursor-ref-reconciliation-wiring.md"
)]

use std::path::Path;
use std::time::Duration;

use super::maintenance_state::{
    derive_git_dir, plan_advisory, read_state, state_path, write_state_atomically,
    AdvisoryDecision, MaintenanceState, PersistFailure,
};
use super::worktree_lock::{acquire_inner, WorktreeLockError};

pub(super) const RECONCILIATION_RECOMMENDED_MESSAGE: &str =
    "Reconciliation is recommended. Run sce doctor --fix.";

#[derive(Debug)]
pub(super) enum AdvisoryOutcome {
    Anchored,
    NoAction,
    Advised,
    AdvisedDurabilityUncertain { warning: PersistFailure },
    Busy,
    StateWriteFailed { warning: PersistFailure },
    StateUnavailable,
}

impl AdvisoryOutcome {
    pub(super) fn recommendation(&self) -> Option<&'static str> {
        match self {
            AdvisoryOutcome::Advised | AdvisoryOutcome::AdvisedDurabilityUncertain { .. } => {
                Some(RECONCILIATION_RECOMMENDED_MESSAGE)
            }
            AdvisoryOutcome::Anchored
            | AdvisoryOutcome::NoAction
            | AdvisoryOutcome::Busy
            | AdvisoryOutcome::StateWriteFailed { .. }
            | AdvisoryOutcome::StateUnavailable => None,
        }
    }

    pub(super) fn persistence_warning(&self) -> Option<&PersistFailure> {
        match self {
            AdvisoryOutcome::AdvisedDurabilityUncertain { warning }
            | AdvisoryOutcome::StateWriteFailed { warning } => Some(warning),
            AdvisoryOutcome::Anchored
            | AdvisoryOutcome::NoAction
            | AdvisoryOutcome::Advised
            | AdvisoryOutcome::Busy
            | AdvisoryOutcome::StateUnavailable => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdvisorySeverity {
    Debug,
    Warn,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AdvisoryReport {
    pub(crate) outcome: &'static str,
    pub(crate) severity: AdvisorySeverity,
    pub(crate) recommendation: Option<&'static str>,
    pub(crate) warning: Option<String>,
}

impl AdvisoryOutcome {
    pub(crate) fn report(&self) -> AdvisoryReport {
        let (outcome, severity) = match self {
            AdvisoryOutcome::Anchored => ("anchored", AdvisorySeverity::Debug),
            AdvisoryOutcome::NoAction => ("no_action", AdvisorySeverity::Debug),
            AdvisoryOutcome::Busy => ("busy", AdvisorySeverity::Debug),
            AdvisoryOutcome::Advised => ("advised", AdvisorySeverity::Warn),
            AdvisoryOutcome::AdvisedDurabilityUncertain { .. } => {
                ("advised_durability_uncertain", AdvisorySeverity::Warn)
            }
            AdvisoryOutcome::StateWriteFailed { .. } => {
                ("state_write_failed", AdvisorySeverity::Warn)
            }
            AdvisoryOutcome::StateUnavailable => ("state_unavailable", AdvisorySeverity::Warn),
        };
        AdvisoryReport {
            outcome,
            severity,
            recommendation: self.recommendation(),
            warning: self.persistence_warning().map(ToString::to_string),
        }
    }
}

fn system_unix_time_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

pub(crate) fn advise_after_completed_boundary(repository_root: &Path) -> AdvisoryReport {
    advise_if_due(repository_root, system_unix_time_ms).report()
}

fn outcome_after_write(
    decision: AdvisoryDecision,
    write: Result<(), PersistFailure>,
) -> AdvisoryOutcome {
    match (decision, write) {
        (AdvisoryDecision::Anchored, Ok(())) => AdvisoryOutcome::Anchored,
        (AdvisoryDecision::NoAction, Ok(())) => AdvisoryOutcome::NoAction,
        (AdvisoryDecision::Advised, Ok(())) => AdvisoryOutcome::Advised,
        (AdvisoryDecision::Advised, Err(warning @ PersistFailure::DurabilityUncertain { .. })) => {
            AdvisoryOutcome::AdvisedDurabilityUncertain { warning }
        }
        (_, Err(warning)) => AdvisoryOutcome::StateWriteFailed { warning },
    }
}

pub(super) fn advise_if_due<C>(repository_root: &Path, now: C) -> AdvisoryOutcome
where
    C: Fn() -> i64,
{
    advise_if_due_with(repository_root, now, write_state_atomically)
}

pub(super) fn advise_if_due_with<C, W>(
    repository_root: &Path,
    now: C,
    write_state: W,
) -> AdvisoryOutcome
where
    C: Fn() -> i64,
    W: Fn(&Path, &MaintenanceState) -> Result<(), PersistFailure>,
{
    let Some(git_dir) = derive_git_dir(repository_root) else {
        return AdvisoryOutcome::StateUnavailable;
    };
    let path = state_path(&git_dir);

    let Ok(unlocked) = read_state(&path) else {
        return AdvisoryOutcome::StateUnavailable;
    };
    let (decision, pending) = plan_advisory(&unlocked, now());
    if pending.is_none() && decision == AdvisoryDecision::NoAction {
        return AdvisoryOutcome::NoAction;
    }

    let lock = match acquire_inner(&git_dir, Duration::ZERO, || {}) {
        Ok(lock) => lock,
        Err(WorktreeLockError::TimedOut { .. }) => return AdvisoryOutcome::Busy,
        Err(_) => return AdvisoryOutcome::StateUnavailable,
    };

    let Ok(locked) = read_state(&path) else {
        return AdvisoryOutcome::StateUnavailable;
    };
    let now_ms = now();
    let (decision, pending) = plan_advisory(&locked, now_ms);

    let result = match pending {
        Some(next) => outcome_after_write(decision, write_state(&path, &next)),
        None => outcome_after_write(decision, Ok(())),
    };
    drop(lock);
    result
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::super::maintenance_state::{
        evaluate_recommendation, read_state, record_failure, record_success, state_path,
        write_state_atomically, write_state_atomically_with, AttemptOutcome,
        FaultInjectingFilesystem, MaintenanceState, PersistFailure, PersistPhase, StateRead,
        StoredReport, FUTURE_SKEW_TOLERANCE_MS, RECONCILIATION_ADVISORY_AFTER_MS,
    };
    use super::super::worktree_lock::acquire_inner;
    use super::{advise_if_due_with, AdvisoryOutcome, RECONCILIATION_RECOMMENDED_MESSAGE};

    const NOW: i64 = 1_000_000_000_000;

    fn init_repo(root: &Path) {
        std::fs::create_dir_all(root).expect("repo dir");
        let output = Command::new("git")
            .args(["init", "-q"])
            .current_dir(root)
            .output()
            .expect("git init");
        assert!(output.status.success());
    }

    fn real_write(path: &Path, state: &MaintenanceState) -> Result<(), PersistFailure> {
        write_state_atomically(path, state)
    }

    fn advise(root: &Path, now: i64) -> AdvisoryOutcome {
        advise_if_due_with(root, || now, real_write)
    }

    fn stored(root: &Path) -> MaintenanceState {
        match read_state(&state_path(&root.join(".git"))).expect("read state") {
            StateRead::Valid(state) => state,
            other => panic!("expected valid state, got {other:?}"),
        }
    }

    fn seed(root: &Path, state: &MaintenanceState) {
        let path = state_path(&root.join(".git"));
        std::fs::create_dir_all(path.parent().expect("parent")).expect("runtime dir");
        real_write(&path, state).expect("seed state");
    }

    #[test]
    fn maintenance_advisory_anchors_once_then_leaves_state_untouched() {
        let dir = tempfile::tempdir().expect("temp dir");
        init_repo(dir.path());

        assert!(matches!(advise(dir.path(), NOW), AdvisoryOutcome::Anchored));
        let path = state_path(&dir.path().join(".git"));
        let first = std::fs::read(&path).expect("state bytes");
        assert_eq!(stored(dir.path()).anchor, Some(NOW));

        assert!(matches!(
            advise(dir.path(), NOW + 1000),
            AdvisoryOutcome::NoAction
        ));
        assert_eq!(std::fs::read(&path).expect("state bytes"), first);
    }

    #[test]
    fn maintenance_advisory_advises_once_per_window_and_persists_before_returning() {
        let dir = tempfile::tempdir().expect("temp dir");
        init_repo(dir.path());
        advise(dir.path(), NOW);

        let due = NOW + RECONCILIATION_ADVISORY_AFTER_MS;
        assert!(matches!(advise(dir.path(), due), AdvisoryOutcome::Advised));
        assert_eq!(stored(dir.path()).last_advised, Some(due));
        assert!(matches!(
            advise(dir.path(), due + 1000),
            AdvisoryOutcome::NoAction
        ));
        assert!(matches!(
            advise(dir.path(), due + RECONCILIATION_ADVISORY_AFTER_MS),
            AdvisoryOutcome::Advised
        ));
    }

    #[test]
    fn maintenance_advisory_preserves_timestamps_within_skew_tolerance() {
        let dir = tempfile::tempdir().expect("temp dir");
        init_repo(dir.path());
        let near_future = MaintenanceState {
            last_success: Some(NOW + FUTURE_SKEW_TOLERANCE_MS),
            ..MaintenanceState::default()
        };
        seed(dir.path(), &near_future);
        let path = state_path(&dir.path().join(".git"));
        let before = std::fs::read(&path).expect("state bytes");

        assert!(matches!(advise(dir.path(), NOW), AdvisoryOutcome::NoAction));
        assert_eq!(std::fs::read(&path).expect("state bytes"), before);
    }

    #[test]
    fn maintenance_advisory_normalizes_far_future_timestamps_and_advises_immediately() {
        let dir = tempfile::tempdir().expect("temp dir");
        init_repo(dir.path());
        let invalid = MaintenanceState {
            anchor: Some(NOW + FUTURE_SKEW_TOLERANCE_MS + 1),
            last_success: Some(NOW + 10 * RECONCILIATION_ADVISORY_AFTER_MS),
            ..MaintenanceState::default()
        };
        seed(dir.path(), &invalid);

        assert!(matches!(advise(dir.path(), NOW), AdvisoryOutcome::Advised));
        let normalized = stored(dir.path());
        assert_eq!(normalized.last_advised, Some(NOW));
        assert_eq!(normalized.anchor, Some(NOW));
        assert_eq!(normalized.last_success, None);

        let path = state_path(&dir.path().join(".git"));
        let bytes = std::fs::read(&path).expect("state bytes");
        assert!(matches!(advise(dir.path(), NOW), AdvisoryOutcome::NoAction));
        assert_eq!(std::fs::read(&path).expect("state bytes"), bytes);
    }

    #[test]
    fn maintenance_advisory_reports_busy_and_leaves_invalid_state_byte_identical() {
        let dir = tempfile::tempdir().expect("temp dir");
        init_repo(dir.path());
        let invalid = MaintenanceState {
            anchor: Some(NOW + 10 * RECONCILIATION_ADVISORY_AFTER_MS),
            ..MaintenanceState::default()
        };
        seed(dir.path(), &invalid);
        let path = state_path(&dir.path().join(".git"));
        let before = std::fs::read(&path).expect("state bytes");

        let held = acquire_inner(&dir.path().join(".git"), std::time::Duration::ZERO, || {})
            .expect("hold lock");
        assert!(matches!(advise(dir.path(), NOW), AdvisoryOutcome::Busy));
        assert_eq!(std::fs::read(&path).expect("state bytes"), before);
        drop(held);

        let recommendation = evaluate_recommendation(&invalid, NOW);
        assert!(recommendation.recommended);
        assert!(recommendation.invalid_timestamps);
    }

    fn advise_with_fault(root: &Path, now: i64, phase: PersistPhase) -> AdvisoryOutcome {
        advise_if_due_with(
            root,
            || now,
            |path, state| {
                write_state_atomically_with(
                    &FaultInjectingFilesystem::failing_at(phase),
                    path,
                    state,
                )
            },
        )
    }

    fn failed_state(now: i64) -> MaintenanceState {
        record_failure(
            &MaintenanceState::default(),
            now,
            "agent_trace_db_unavailable",
            "agent trace database is missing",
        )
    }

    #[test]
    fn maintenance_first_failed_explicit_pass_without_anchor_is_recommended() {
        let state = failed_state(NOW);
        assert_eq!(state.anchor, None);
        assert_eq!(state.last_success, None);

        let recommendation = evaluate_recommendation(&state, NOW);
        assert!(recommendation.recommended);
        assert_eq!(
            recommendation.last_attempt_outcome,
            Some(AttemptOutcome::Failed)
        );
        assert_eq!(recommendation.consecutive_failures, 1);
        assert!(!evaluate_recommendation(&MaintenanceState::default(), NOW).recommended);
    }

    #[test]
    fn maintenance_failure_advisory_emits_once_and_doctor_still_recommends_repair() {
        let dir = tempfile::tempdir().expect("temp dir");
        init_repo(dir.path());
        seed(dir.path(), &failed_state(NOW));

        let first = advise(dir.path(), NOW + 1000);
        assert!(matches!(first, AdvisoryOutcome::Advised));
        assert_eq!(
            first.recommendation(),
            Some(RECONCILIATION_RECOMMENDED_MESSAGE)
        );
        let persisted = stored(dir.path());
        assert_eq!(persisted.last_advised, Some(NOW + 1000));
        assert_eq!(persisted.consecutive_failures, 1);
        assert_eq!(persisted.last_attempt_outcome, Some(AttemptOutcome::Failed));
        assert_eq!(persisted.last_failure, failed_state(NOW).last_failure);

        let path = state_path(&dir.path().join(".git"));
        let bytes = std::fs::read(&path).expect("state bytes");
        let second = advise(dir.path(), NOW + 2000);
        assert!(matches!(second, AdvisoryOutcome::NoAction));
        assert_eq!(second.recommendation(), None);
        assert_eq!(std::fs::read(&path).expect("state bytes"), bytes);

        let recommendation = evaluate_recommendation(&stored(dir.path()), NOW + 2000);
        assert!(recommendation.recommended);
        assert!(recommendation.last_advised_age_ms.is_some());
    }

    #[test]
    fn maintenance_failure_advisory_repeats_after_the_cadence_window() {
        let dir = tempfile::tempdir().expect("temp dir");
        init_repo(dir.path());
        seed(dir.path(), &failed_state(NOW));

        assert!(matches!(advise(dir.path(), NOW), AdvisoryOutcome::Advised));
        let later = NOW + RECONCILIATION_ADVISORY_AFTER_MS;
        assert!(matches!(
            advise(dir.path(), later),
            AdvisoryOutcome::Advised
        ));
        assert_eq!(stored(dir.path()).consecutive_failures, 1);
    }

    #[test]
    fn maintenance_repeated_failed_passes_keep_recommendation_active() {
        let first = failed_state(NOW);
        let second = record_failure(&first, NOW + 10, "pin_inventory", "inventory failed");
        let third = record_failure(&second, NOW + 20, "pin_inventory", "inventory failed");
        assert_eq!(third.consecutive_failures, 3);
        assert!(evaluate_recommendation(&first, NOW + 30).recommended);
        assert!(evaluate_recommendation(&second, NOW + 30).recommended);
        assert!(evaluate_recommendation(&third, NOW + 30).recommended);
    }

    #[test]
    fn maintenance_successful_pass_clears_failure_streak_and_recommendation() {
        let failed = record_failure(&failed_state(NOW), NOW + 10, "pin_inventory", "failed");
        let healed = record_success(
            &failed,
            NOW + 20,
            StoredReport {
                retained: 1,
                deleted: 0,
                local_required: 0,
            },
        );
        assert_eq!(healed.consecutive_failures, 0);
        assert_eq!(healed.last_failure, None);
        assert_eq!(healed.last_attempt_outcome, Some(AttemptOutcome::Completed));
        assert!(!evaluate_recommendation(&healed, NOW + 30).recommended);
    }

    #[test]
    fn maintenance_failure_after_recent_success_is_recommended_and_advised_despite_older_reminder()
    {
        let dir = tempfile::tempdir().expect("temp dir");
        init_repo(dir.path());
        let success = record_success(
            &MaintenanceState::default(),
            NOW,
            StoredReport {
                retained: 1,
                deleted: 0,
                local_required: 0,
            },
        );
        let reminded = MaintenanceState {
            last_advised: Some(NOW + 1000),
            ..success
        };
        assert!(!evaluate_recommendation(&reminded, NOW + 1500).recommended);

        let failed = record_failure(&reminded, NOW + 2000, "delete_transaction", "failed");
        assert!(evaluate_recommendation(&failed, NOW + 2500).recommended);
        seed(dir.path(), &failed);

        assert!(matches!(
            advise(dir.path(), NOW + 2500),
            AdvisoryOutcome::Advised
        ));
        assert!(matches!(
            advise(dir.path(), NOW + 3000),
            AdvisoryOutcome::NoAction
        ));
    }

    #[test]
    fn maintenance_advisory_rename_failure_preserves_state_and_next_call_can_advise() {
        let dir = tempfile::tempdir().expect("temp dir");
        init_repo(dir.path());
        advise(dir.path(), NOW);
        let path = state_path(&dir.path().join(".git"));
        let before = std::fs::read(&path).expect("state bytes");

        let due = NOW + RECONCILIATION_ADVISORY_AFTER_MS;
        let outcome = advise_with_fault(dir.path(), due, PersistPhase::Rename);
        assert!(matches!(
            outcome,
            AdvisoryOutcome::StateWriteFailed {
                warning: PersistFailure::NotApplied {
                    phase: PersistPhase::Rename,
                    ..
                }
            }
        ));
        assert_eq!(outcome.recommendation(), None);
        assert!(outcome.persistence_warning().is_some());
        assert_eq!(std::fs::read(&path).expect("state bytes"), before);
        assert!(evaluate_recommendation(&stored(dir.path()), due).recommended);
        assert!(matches!(advise(dir.path(), due), AdvisoryOutcome::Advised));
    }

    fn assert_durability_uncertain_advisory(phase: PersistPhase) {
        let dir = tempfile::tempdir().expect("temp dir");
        init_repo(dir.path());
        advise(dir.path(), NOW);

        let due = NOW + RECONCILIATION_ADVISORY_AFTER_MS;
        let outcome = advise_with_fault(dir.path(), due, phase);
        assert!(!matches!(outcome, AdvisoryOutcome::Advised));
        let AdvisoryOutcome::AdvisedDurabilityUncertain { warning } = &outcome else {
            panic!("expected AdvisedDurabilityUncertain for {phase:?}, got {outcome:?}");
        };
        assert!(matches!(
            warning,
            PersistFailure::DurabilityUncertain { phase: reported, .. } if *reported == phase
        ));
        assert_eq!(
            outcome.recommendation(),
            Some(RECONCILIATION_RECOMMENDED_MESSAGE)
        );
        assert!(outcome.persistence_warning().is_some());

        let visible = stored(dir.path());
        assert_eq!(visible.last_advised, Some(due));

        assert!(matches!(
            advise(dir.path(), due + 1000),
            AdvisoryOutcome::NoAction
        ));
        let recommendation = evaluate_recommendation(&visible, due + 1000);
        assert!(recommendation.recommended);
        assert!(recommendation.last_advised_age_ms.is_some());
    }

    #[test]
    fn maintenance_advisory_parent_directory_open_failure_is_durability_uncertain() {
        assert_durability_uncertain_advisory(PersistPhase::OpenParentDirectory);
    }

    #[test]
    fn maintenance_advisory_parent_directory_sync_failure_is_durability_uncertain() {
        assert_durability_uncertain_advisory(PersistPhase::SyncParentDirectory);
    }

    #[test]
    fn maintenance_advisory_durability_uncertain_failure_state_stays_recommended() {
        let dir = tempfile::tempdir().expect("temp dir");
        init_repo(dir.path());
        seed(dir.path(), &failed_state(NOW));

        let outcome = advise_with_fault(dir.path(), NOW + 10, PersistPhase::SyncParentDirectory);
        assert!(matches!(
            outcome,
            AdvisoryOutcome::AdvisedDurabilityUncertain { .. }
        ));
        let visible = stored(dir.path());
        assert_eq!(visible.last_advised, Some(NOW + 10));
        assert_eq!(visible.consecutive_failures, 1);
        assert!(evaluate_recommendation(&visible, NOW + 20).recommended);
    }

    #[test]
    fn maintenance_advisory_anchor_write_failure_reports_state_write_failed_without_advice() {
        let dir = tempfile::tempdir().expect("temp dir");
        init_repo(dir.path());

        let outcome = advise_with_fault(dir.path(), NOW, PersistPhase::Rename);
        assert!(matches!(outcome, AdvisoryOutcome::StateWriteFailed { .. }));
        assert_eq!(outcome.recommendation(), None);
        assert!(matches!(advise(dir.path(), NOW), AdvisoryOutcome::Anchored));
    }

    #[test]
    fn maintenance_advisory_recovers_corrupt_state_by_anchoring() {
        let dir = tempfile::tempdir().expect("temp dir");
        init_repo(dir.path());
        let path = state_path(&dir.path().join(".git"));
        std::fs::create_dir_all(path.parent().expect("parent")).expect("runtime dir");
        std::fs::write(&path, b"not json").expect("corrupt state");

        assert!(matches!(advise(dir.path(), NOW), AdvisoryOutcome::Anchored));
        assert_eq!(stored(dir.path()).anchor, Some(NOW));
    }

    #[test]
    fn maintenance_advisory_derives_linked_worktree_git_dir_from_subdirectory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let main = dir.path().join("main");
        init_repo(&main);
        for args in [
            vec!["config", "user.email", "t@example.invalid"],
            vec!["config", "user.name", "T"],
            vec!["commit", "-q", "--allow-empty", "-m", "init"],
            vec!["worktree", "add", "-q", "../linked"],
        ] {
            let output = Command::new("git")
                .args(&args)
                .current_dir(&main)
                .output()
                .expect("git");
            assert!(output.status.success(), "{args:?}");
        }
        let linked = dir.path().join("linked");
        let sub = linked.join("nested");
        std::fs::create_dir_all(&sub).expect("subdir");

        let expected = Command::new("git")
            .args(["rev-parse", "--absolute-git-dir"])
            .current_dir(&sub)
            .output()
            .expect("rev-parse");
        let expected = std::fs::canonicalize(String::from_utf8_lossy(&expected.stdout).trim())
            .expect("canonical");

        assert_eq!(super::derive_git_dir(&sub), Some(expected.clone()));
        assert!(matches!(advise(&sub, NOW), AdvisoryOutcome::Anchored));
        assert!(state_path(&expected).is_file());
    }
}
