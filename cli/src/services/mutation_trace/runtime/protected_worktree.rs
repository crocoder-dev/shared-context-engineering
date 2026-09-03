use std::path::Path;
use std::time::Duration;

use crate::services::checkout::{get_or_create_checkout_id, resolve_git_dir};
use crate::services::mutation_trace::types::WorktreeId;

use super::external_taint::ExternalTaintMarker;
use super::worktree_lock::{acquire_inner, WorktreeLock, WorktreeLockError};

pub const WORKTREE_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalTaintOperation {
    Inspect,
    Persist,
}

#[derive(Debug)]
pub enum ProtectedWorktreeError {
    GitDirResolution(anyhow::Error),
    LockAcquisition(WorktreeLockError),
    ExternalTaintMarker {
        operation: ExternalTaintOperation,
        source: anyhow::Error,
    },
    CheckoutIdentity(anyhow::Error),
}

impl std::fmt::Display for ProtectedWorktreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtectedWorktreeError::ExternalTaintMarker { operation, source } => write!(
                f,
                "External-taint marker {operation:?} operation failed before any \
                 protected runtime work began: {source}"
            ),
            ProtectedWorktreeError::LockAcquisition(source) => write!(f, "{source}"),
            ProtectedWorktreeError::GitDirResolution(source)
            | ProtectedWorktreeError::CheckoutIdentity(source) => write!(f, "{source}"),
        }
    }
}

impl std::error::Error for ProtectedWorktreeError {}

#[derive(Debug)]
pub struct ProtectedWorktree {
    marker: ExternalTaintMarker,
    inherited_external_taint: bool,
    worktree_id: WorktreeId,
    _lock: WorktreeLock,
}

impl ProtectedWorktree {
    pub fn acquire(repository_root: &Path) -> Result<Self, ProtectedWorktreeError> {
        Self::acquire_inner(repository_root, || {})
    }

    pub(super) fn acquire_inner<F>(
        repository_root: &Path,
        on_lock_contention: F,
    ) -> Result<Self, ProtectedWorktreeError>
    where
        F: FnOnce(),
    {
        Self::acquire_with_timeout(repository_root, WORKTREE_LOCK_TIMEOUT, on_lock_contention)
    }

    fn acquire_with_timeout<F>(
        repository_root: &Path,
        lock_timeout: Duration,
        on_lock_contention: F,
    ) -> Result<Self, ProtectedWorktreeError>
    where
        F: FnOnce(),
    {
        let git_dir =
            resolve_git_dir(repository_root).map_err(ProtectedWorktreeError::GitDirResolution)?;

        let lock = acquire_inner(&git_dir, lock_timeout, on_lock_contention)
            .map_err(ProtectedWorktreeError::LockAcquisition)?;

        let marker = ExternalTaintMarker::new(&git_dir);
        let inherited_external_taint =
            marker
                .exists()
                .map_err(|source| ProtectedWorktreeError::ExternalTaintMarker {
                    operation: ExternalTaintOperation::Inspect,
                    source,
                })?;
        marker
            .persist()
            .map_err(|source| ProtectedWorktreeError::ExternalTaintMarker {
                operation: ExternalTaintOperation::Persist,
                source,
            })?;

        let checkout_id = get_or_create_checkout_id(&git_dir)
            .map_err(ProtectedWorktreeError::CheckoutIdentity)?;

        Ok(Self {
            marker,
            inherited_external_taint,
            worktree_id: WorktreeId(checkout_id),
            _lock: lock,
        })
    }

    #[must_use]
    pub fn worktree_id(&self) -> &WorktreeId {
        &self.worktree_id
    }

    #[must_use]
    pub fn inherited_external_taint(&self) -> bool {
        self.inherited_external_taint
    }

    pub fn complete(self) -> anyhow::Result<()> {
        self.marker.clear()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::mpsc;
    use std::thread;

    use super::*;

    struct TestRepo {
        _temp_dir: tempfile::TempDir,
        repo_root: PathBuf,
        git_dir: PathBuf,
    }

    impl TestRepo {
        fn new(label: &str) -> Self {
            let temp_dir = tempfile::Builder::new()
                .prefix(&format!("sce-protected-worktree-{label}-"))
                .tempdir()
                .expect("test temp directory should be created");
            let repo_root = temp_dir.path().join("repo");
            std::fs::create_dir_all(&repo_root).expect("repository directory should be created");
            run_git(&repo_root, &["init", "--quiet"]);
            let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
            Self {
                _temp_dir: temp_dir,
                repo_root,
                git_dir,
            }
        }

        fn marker(&self) -> ExternalTaintMarker {
            ExternalTaintMarker::new(&self.git_dir)
        }

        fn marker_exists(&self) -> bool {
            self.marker()
                .exists()
                .expect("marker existence should resolve")
        }
    }

    fn run_git(dir: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git command should spawn");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn acquire_arms_a_fresh_marker_and_reports_no_inherited_taint() {
        let repo = TestRepo::new("fresh-marker");

        let guard = ProtectedWorktree::acquire(&repo.repo_root)
            .expect("the prefix should be establishable on a clean worktree");

        assert!(
            !guard.inherited_external_taint(),
            "a worktree with no marker on entry must not report inherited taint"
        );
        assert!(
            !guard.worktree_id().0.is_empty(),
            "the guard must expose the derived WorktreeId"
        );
        assert!(
            repo.marker_exists(),
            "acquire must arm the external-taint marker write-ahead"
        );

        guard
            .complete()
            .expect("completion should clear the marker");
        assert!(
            !repo.marker_exists(),
            "an explicit completion must clear the marker it armed"
        );
    }

    #[test]
    fn acquire_reports_a_marker_inherited_from_an_earlier_invocation() {
        let repo = TestRepo::new("inherited-marker");
        repo.marker()
            .persist()
            .expect("the earlier invocation's marker should arm");

        let guard = ProtectedWorktree::acquire(&repo.repo_root)
            .expect("an inherited marker must not fail the prefix");

        assert!(
            guard.inherited_external_taint(),
            "a marker present on entry must be reported as inherited taint"
        );
        assert!(
            repo.marker_exists(),
            "the inherited marker must stay armed for the protected operation"
        );
    }

    #[test]
    fn a_dropped_guard_releases_the_lock_but_leaves_the_marker_armed() {
        let repo = TestRepo::new("dropped-guard");

        let guard = ProtectedWorktree::acquire(&repo.repo_root)
            .expect("the prefix should be establishable on a clean worktree");
        drop(guard);

        assert!(
            repo.marker_exists(),
            "dropping a guard that never completed must leave the fence armed"
        );

        ProtectedWorktree::acquire(&repo.repo_root)
            .expect("a dropped guard must have released the worktree lock");
    }

    #[test]
    fn acquire_fails_with_lock_acquisition_while_the_lock_is_still_held() {
        let repo = TestRepo::new("lock-timeout");

        let held = acquire_inner(&repo.git_dir, Duration::from_secs(5), || {})
            .expect("the test should hold the worktree lock before the guard runs");

        let error = ProtectedWorktree::acquire_with_timeout(
            &repo.repo_root,
            Duration::from_millis(250),
            || {},
        )
        .expect_err("the prefix must not be establishable while the lock is held");
        assert!(
            matches!(
                error,
                ProtectedWorktreeError::LockAcquisition(WorktreeLockError::TimedOut { .. })
            ),
            "expected a LockAcquisition timeout, got {error:?}"
        );
        assert!(
            !repo.marker_exists(),
            "a prefix that never acquired the lock must not have armed the fence"
        );

        drop(held);
    }

    #[test]
    fn the_lock_is_held_for_the_whole_guard_lifetime() {
        let repo = TestRepo::new("lock-lifetime");

        let guard = ProtectedWorktree::acquire(&repo.repo_root)
            .expect("the prefix should be establishable on a clean worktree");

        let (contention_tx, contention_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let repo_root = repo.repo_root.clone();
        let worker = thread::spawn(move || {
            let result = ProtectedWorktree::acquire_inner(&repo_root, move || {
                contention_tx
                    .send(())
                    .expect("contention signal channel should still be open");
            });
            result_tx
                .send(())
                .expect("result signal channel should still be open");
            result
        });

        contention_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("a second acquirer should observe the held worktree lock");
        assert!(
            result_rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "a second acquirer must not establish the prefix while the guard is alive"
        );

        drop(guard);

        result_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the second acquirer should proceed once the guard is dropped");
        worker
            .join()
            .expect("the second acquirer thread should not panic")
            .expect("the second acquirer should succeed once the lock is released");
    }
}
