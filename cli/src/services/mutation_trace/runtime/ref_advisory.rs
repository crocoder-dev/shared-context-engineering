#![allow(
    dead_code,
    reason = "maintenance entrypoints are wired by later tasks of context/plans/mutation-cursor-ref-reconciliation-wiring.md"
)]

use std::path::Path;
use std::time::Duration;

use super::maintenance_state::{
    derive_git_dir, plan_advisory, read_state, state_path, write_state_atomically,
    AdvisoryDecision, MaintenanceState,
};
use super::worktree_lock::{acquire_inner, WorktreeLockError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AdvisoryOutcome {
    Anchored,
    NoAction,
    Advised,
    Busy,
    StateUnavailable,
}

pub(super) fn advise_if_due<C>(repository_root: &Path, now: C) -> AdvisoryOutcome
where
    C: Fn() -> i64,
{
    advise_if_due_with(repository_root, now, |path, state| {
        write_state_atomically(path, state, |from, to| std::fs::rename(from, to))
    })
}

pub(super) fn advise_if_due_with<C, W>(
    repository_root: &Path,
    now: C,
    write_state: W,
) -> AdvisoryOutcome
where
    C: Fn() -> i64,
    W: Fn(&Path, &MaintenanceState) -> std::io::Result<()>,
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
    let outcome = match decision {
        AdvisoryDecision::Anchored => AdvisoryOutcome::Anchored,
        AdvisoryDecision::NoAction => AdvisoryOutcome::NoAction,
        AdvisoryDecision::Advised => AdvisoryOutcome::Advised,
    };

    let result = match pending {
        Some(next) => match write_state(&path, &next) {
            Ok(()) => outcome,
            Err(_) => AdvisoryOutcome::StateUnavailable,
        },
        None => outcome,
    };
    drop(lock);
    result
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::super::maintenance_state::{
        evaluate_recommendation, read_state, state_path, write_state_atomically, MaintenanceState,
        StateRead, FUTURE_SKEW_TOLERANCE_MS, RECONCILIATION_ADVISORY_AFTER_MS,
    };
    use super::super::worktree_lock::acquire_inner;
    use super::{advise_if_due_with, AdvisoryOutcome};

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

    fn real_write(path: &Path, state: &MaintenanceState) -> std::io::Result<()> {
        write_state_atomically(path, state, |from, to| std::fs::rename(from, to))
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

        assert_eq!(advise(dir.path(), NOW), AdvisoryOutcome::Anchored);
        let path = state_path(&dir.path().join(".git"));
        let first = std::fs::read(&path).expect("state bytes");
        assert_eq!(stored(dir.path()).anchor, Some(NOW));

        assert_eq!(advise(dir.path(), NOW + 1000), AdvisoryOutcome::NoAction);
        assert_eq!(std::fs::read(&path).expect("state bytes"), first);
    }

    #[test]
    fn maintenance_advisory_advises_once_per_window_and_persists_before_returning() {
        let dir = tempfile::tempdir().expect("temp dir");
        init_repo(dir.path());
        advise(dir.path(), NOW);

        let due = NOW + RECONCILIATION_ADVISORY_AFTER_MS;
        assert_eq!(advise(dir.path(), due), AdvisoryOutcome::Advised);
        assert_eq!(stored(dir.path()).last_advised, Some(due));
        assert_eq!(advise(dir.path(), due + 1000), AdvisoryOutcome::NoAction);
        assert_eq!(
            advise(dir.path(), due + RECONCILIATION_ADVISORY_AFTER_MS),
            AdvisoryOutcome::Advised
        );
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

        assert_eq!(advise(dir.path(), NOW), AdvisoryOutcome::NoAction);
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

        assert_eq!(advise(dir.path(), NOW), AdvisoryOutcome::Advised);
        let normalized = stored(dir.path());
        assert_eq!(normalized.last_advised, Some(NOW));
        assert_eq!(normalized.anchor, Some(NOW));
        assert_eq!(normalized.last_success, None);

        let path = state_path(&dir.path().join(".git"));
        let bytes = std::fs::read(&path).expect("state bytes");
        assert_eq!(advise(dir.path(), NOW), AdvisoryOutcome::NoAction);
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
        assert_eq!(advise(dir.path(), NOW), AdvisoryOutcome::Busy);
        assert_eq!(std::fs::read(&path).expect("state bytes"), before);
        drop(held);

        let recommendation = evaluate_recommendation(&invalid, NOW);
        assert!(recommendation.recommended);
        assert!(recommendation.invalid_timestamps);
    }

    #[test]
    fn maintenance_advisory_write_failure_emits_no_advice_and_keeps_previous_state() {
        let dir = tempfile::tempdir().expect("temp dir");
        init_repo(dir.path());
        advise(dir.path(), NOW);
        let path = state_path(&dir.path().join(".git"));
        let before = std::fs::read(&path).expect("state bytes");

        let due = NOW + RECONCILIATION_ADVISORY_AFTER_MS;
        let outcome = advise_if_due_with(
            dir.path(),
            || due,
            |path, state| {
                write_state_atomically(path, state, |_, _| {
                    Err(std::io::Error::other("rename interrupted"))
                })
            },
        );
        assert_eq!(outcome, AdvisoryOutcome::StateUnavailable);
        assert_eq!(std::fs::read(&path).expect("state bytes"), before);
        assert_eq!(advise(dir.path(), due), AdvisoryOutcome::Advised);
    }

    #[test]
    fn maintenance_advisory_recovers_corrupt_state_by_anchoring() {
        let dir = tempfile::tempdir().expect("temp dir");
        init_repo(dir.path());
        let path = state_path(&dir.path().join(".git"));
        std::fs::create_dir_all(path.parent().expect("parent")).expect("runtime dir");
        std::fs::write(&path, b"not json").expect("corrupt state");

        assert_eq!(advise(dir.path(), NOW), AdvisoryOutcome::Anchored);
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
        assert_eq!(advise(&sub, NOW), AdvisoryOutcome::Anchored);
        assert!(state_path(&expected).is_file());
    }
}
