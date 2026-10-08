use std::path::Path;
use std::time::Duration;

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
    lock: WorktreeLock,
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

    async fn acquire_with_timeout<F>(
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

    pub fn complete(self) -> anyhow::Result<()> {
        self.marker.clear()
    }

    #[cfg(unix)]
    pub(super) fn abandon_after_spawn_without_unlock(self) {
        self.lock.close_without_unlock();
    }
}
