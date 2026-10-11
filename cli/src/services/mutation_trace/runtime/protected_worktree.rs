use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;

use crate::services::mutation_trace::types::WorktreeId;

use super::external_taint::ExternalTaintMarker;
use super::git_snapshot::{resolve_git_dir, resolve_worktree_id};
use super::worktree_lock::{
    acquire_inner_async, WorktreeLock, WorktreeLockError, WorktreeLockLease,
};

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
    MarkerWorkerFailed(tokio::task::JoinError),
}

impl std::fmt::Display for ProtectedWorktreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtectedWorktreeError::ExternalTaintMarker { operation, source } => write!(
                f,
                "External-taint marker {operation:?} operation failed before any \
                 protected runtime work began: {source}"
            ),
            ProtectedWorktreeError::MarkerWorkerFailed(source) => write!(
                f,
                "External-taint marker worker failed while holding the worktree lock; \
                 the marker state is unknown and no protected runtime work began: {source}"
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
    lock: WorktreeLock,
}

async fn arm_marker_with_lock<F>(
    git_dir: PathBuf,
    lock: WorktreeLock,
    before_persist: F,
) -> Result<(WorktreeLock, ExternalTaintMarker, bool), ProtectedWorktreeError>
where
    F: FnOnce() + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let marker = ExternalTaintMarker::new(&git_dir);
        let inherited_external_taint =
            marker
                .exists()
                .map_err(|source| ProtectedWorktreeError::ExternalTaintMarker {
                    operation: ExternalTaintOperation::Inspect,
                    source,
                })?;
        before_persist();
        marker
            .persist()
            .map_err(|source| ProtectedWorktreeError::ExternalTaintMarker {
                operation: ExternalTaintOperation::Persist,
                source,
            })?;
        Ok((lock, marker, inherited_external_taint))
    })
    .await
    .map_err(ProtectedWorktreeError::MarkerWorkerFailed)?
}

impl ProtectedWorktree {
    pub async fn acquire(repository_root: &Path) -> Result<Self, ProtectedWorktreeError> {
        Self::acquire_inner(repository_root, || {}).await
    }

    pub(super) async fn acquire_inner<F>(
        repository_root: &Path,
        on_lock_contention: F,
    ) -> Result<Self, ProtectedWorktreeError>
    where
        F: FnOnce() + Send + 'static,
    {
        Self::acquire_with_timeout(repository_root, WORKTREE_LOCK_TIMEOUT, on_lock_contention).await
    }

    pub(super) async fn acquire_with_timeout<F>(
        repository_root: &Path,
        lock_timeout: Duration,
        on_lock_contention: F,
    ) -> Result<Self, ProtectedWorktreeError>
    where
        F: FnOnce() + Send + 'static,
    {
        let git_dir = resolve_git_dir(repository_root)
            .await
            .map_err(ProtectedWorktreeError::GitDirResolution)?;

        let lock = acquire_inner_async(&git_dir, lock_timeout, on_lock_contention)
            .await
            .map_err(ProtectedWorktreeError::LockAcquisition)?;

        let (lock, marker, inherited_external_taint) =
            arm_marker_with_lock(git_dir, lock, || {}).await?;

        let worktree_id = resolve_worktree_id(repository_root)
            .await
            .map_err(ProtectedWorktreeError::CheckoutIdentity)?;

        Ok(Self {
            marker,
            inherited_external_taint,
            worktree_id,
            lock,
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

    #[must_use]
    pub(super) fn lock_lease(&self) -> WorktreeLockLease {
        self.lock.lease()
    }

    #[cfg(unix)]
    #[must_use]
    pub(super) fn lock_raw_fd(&self) -> std::os::unix::io::RawFd {
        self.lock.as_raw_fd()
    }

    pub async fn complete(self) -> anyhow::Result<()> {
        self.complete_inner(|| {}).await
    }

    async fn complete_inner<F>(self, before_clear: F) -> anyhow::Result<()>
    where
        F: FnOnce() + Send + 'static,
    {
        tokio::task::spawn_blocking(move || self.complete_blocking(before_clear))
            .await
            .context("External-taint marker completion worker failed")?
    }

    fn complete_blocking<F>(self, before_clear: F) -> anyhow::Result<()>
    where
        F: FnOnce(),
    {
        before_clear();
        self.marker.clear()
    }

    #[cfg(unix)]
    pub(super) fn abandon_after_spawn_without_unlock(self) {
        self.lock.close_without_unlock();
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;
    use std::sync::mpsc;
    use std::time::Duration;

    use tokio::sync::oneshot;

    use super::super::external_taint::ExternalTaintMarker;
    use super::super::git_snapshot::resolve_git_dir;
    use super::super::worktree_lock::{acquire_inner_async, WorktreeLockError};
    use super::{arm_marker_with_lock, ProtectedWorktree};

    const LOCK_CONTENDED_TIMEOUT: Duration = Duration::from_millis(300);
    const LOCK_RELEASE_TIMEOUT: Duration = Duration::from_secs(10);
    const WORKER_HOOK_TIMEOUT: Duration = Duration::from_secs(10);
    const WORKER_PROGRESS_LIMIT: Duration = Duration::from_secs(2);

    fn git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_committed_repository(root: &Path) {
        std::fs::create_dir_all(root).expect("repo dir");
        git(root, &["init", "-q"]);
        git(root, &["config", "user.email", "protected@example.invalid"]);
        git(root, &["config", "user.name", "Protected Test"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("tracked.txt"), "tracked\n").expect("write tracked file");
        git(root, &["add", "tracked.txt"]);
        git(root, &["commit", "-q", "-m", "initial"]);
    }

    fn paused_worker_hook() -> (
        impl FnOnce() + Send + 'static,
        oneshot::Receiver<()>,
        mpsc::Sender<()>,
    ) {
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let hook = move || {
            let _ = entered_tx.send(());
            let _ = release_rx.recv_timeout(WORKER_HOOK_TIMEOUT);
        };
        (hook, entered_rx, release_tx)
    }

    async fn assert_runtime_worker_progresses() {
        tokio::time::timeout(
            WORKER_PROGRESS_LIMIT,
            tokio::spawn(tokio::time::sleep(Duration::from_millis(10))),
        )
        .await
        .expect("an unrelated Tokio timer should progress while marker I/O is paused")
        .expect("unrelated timer task should join");
    }

    async fn assert_worktree_lock_contended(git_dir: &Path) {
        let contended = acquire_inner_async(git_dir, LOCK_CONTENDED_TIMEOUT, || {}).await;
        assert!(
            matches!(contended, Err(WorktreeLockError::TimedOut { .. })),
            "the paused marker worker must still own the worktree lock, got {contended:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn cancelled_marker_arming_keeps_the_lock_until_the_marker_is_persisted() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo_root = dir.path().join("repo");
        init_committed_repository(&repo_root);
        let git_dir = resolve_git_dir(&repo_root).await.expect("git dir");
        let marker = ExternalTaintMarker::new(&git_dir);

        let lock = acquire_inner_async(&git_dir, LOCK_RELEASE_TIMEOUT, || {})
            .await
            .expect("initial worktree lock");
        let (hook, entered_rx, release_tx) = paused_worker_hook();
        let arming = tokio::spawn(arm_marker_with_lock(git_dir.clone(), lock, hook));

        tokio::time::timeout(WORKER_HOOK_TIMEOUT, entered_rx)
            .await
            .expect("arming worker should inspect the marker")
            .expect("arming hook sender should stay alive");
        assert!(!marker.exists().expect("inspect marker"));

        assert_runtime_worker_progresses().await;

        arming.abort();
        let aborted = arming.await.expect_err("arming caller should be aborted");
        assert!(aborted.is_cancelled());

        assert_worktree_lock_contended(&git_dir).await;

        release_tx.send(()).expect("release arming worker");
        drop(
            acquire_inner_async(&git_dir, LOCK_RELEASE_TIMEOUT, || {})
                .await
                .expect("the arming worker should release the lock after persisting"),
        );
        assert!(
            marker.exists().expect("inspect marker"),
            "cancelled arming must leave the marker armed"
        );

        let protected = ProtectedWorktree::acquire(&repo_root)
            .await
            .expect("protected worktree");
        assert!(protected.inherited_external_taint());
        protected.complete().await.expect("complete");
        assert!(!marker.exists().expect("inspect marker"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn cancelled_completion_clears_the_marker_before_a_later_operation_can_arm() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo_root = dir.path().join("repo");
        init_committed_repository(&repo_root);
        let git_dir = resolve_git_dir(&repo_root).await.expect("git dir");
        let marker = ExternalTaintMarker::new(&git_dir);

        let protected = ProtectedWorktree::acquire(&repo_root)
            .await
            .expect("first protected worktree");
        assert!(!protected.inherited_external_taint());
        assert!(marker.exists().expect("inspect marker"));

        let (hook, entered_rx, release_tx) = paused_worker_hook();
        let completion = tokio::spawn(protected.complete_inner(hook));

        tokio::time::timeout(WORKER_HOOK_TIMEOUT, entered_rx)
            .await
            .expect("completion worker should take ownership")
            .expect("completion hook sender should stay alive");

        assert_runtime_worker_progresses().await;

        completion.abort();
        let aborted = completion
            .await
            .expect_err("completion caller should be aborted");
        assert!(aborted.is_cancelled());

        assert_worktree_lock_contended(&git_dir).await;
        assert!(marker.exists().expect("inspect marker"));

        release_tx.send(()).expect("release completion worker");

        let next = ProtectedWorktree::acquire(&repo_root)
            .await
            .expect("second protected worktree");
        assert!(
            !next.inherited_external_taint(),
            "the earlier completion must clear its marker before releasing the lock"
        );
        assert!(
            marker.exists().expect("inspect marker"),
            "the later operation must keep its own armed marker"
        );

        next.complete().await.expect("complete second operation");
        assert!(!marker.exists().expect("inspect marker"));
    }
}
