use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

const SCE_RUNTIME_DIR: &str = "sce";

const WORKTREE_LOCK_FILE: &str = "mutation-cursor.lock";

const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug)]
struct WorktreeLockInner {
    file: File,
    unlock_on_drop: AtomicBool,
}

#[derive(Debug)]
pub struct WorktreeLock {
    inner: Arc<WorktreeLockInner>,
}

#[derive(Clone, Debug)]
pub struct WorktreeLockLease {
    _inner: Arc<WorktreeLockInner>,
}

#[derive(Debug)]
pub enum WorktreeLockError {
    TimedOut { path: PathBuf, timeout: Duration },
    Io(anyhow::Error),
    WorkerFailed(tokio::task::JoinError),
}

impl std::fmt::Display for WorktreeLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorktreeLockError::TimedOut { path, timeout } => write!(
                f,
                "Timed out after {timeout:?} waiting for worktree lock '{}'",
                path.display()
            ),
            WorktreeLockError::Io(source) => write!(f, "{source}"),
            WorktreeLockError::WorkerFailed(source) => {
                write!(f, "Worktree lock acquisition worker failed: {source}")
            }
        }
    }
}

impl std::error::Error for WorktreeLockError {}

impl WorktreeLock {
    pub async fn acquire_async(
        git_dir: &Path,
        timeout: Duration,
    ) -> Result<WorktreeLock, WorktreeLockError> {
        acquire_inner_async(git_dir, timeout, || {}).await
    }
}

pub(super) async fn acquire_inner_async<F>(
    git_dir: &Path,
    timeout: Duration,
    on_contention: F,
) -> Result<WorktreeLock, WorktreeLockError>
where
    F: FnOnce() + Send + 'static,
{
    let git_dir = git_dir.to_owned();
    tokio::task::spawn_blocking(move || acquire_inner(&git_dir, timeout, on_contention))
        .await
        .map_err(WorktreeLockError::WorkerFailed)?
}

pub(super) fn acquire_inner<F>(
    git_dir: &Path,
    timeout: Duration,
    on_contention: F,
) -> Result<WorktreeLock, WorktreeLockError>
where
    F: FnOnce(),
{
    let runtime_dir = git_dir.join(SCE_RUNTIME_DIR);
    std::fs::create_dir_all(&runtime_dir)
        .with_context(|| {
            format!(
                "Failed to create runtime directory '{}'",
                runtime_dir.display()
            )
        })
        .map_err(WorktreeLockError::Io)?;

    let lock_path = runtime_dir.join(WORKTREE_LOCK_FILE);
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| {
            format!(
                "Failed to open worktree lock file '{}'",
                lock_path.display()
            )
        })
        .map_err(WorktreeLockError::Io)?;

    let mut on_contention = Some(on_contention);
    let deadline = Instant::now() + timeout;
    loop {
        match file.try_lock() {
            Ok(()) => {
                return Ok(WorktreeLock {
                    inner: Arc::new(WorktreeLockInner {
                        file,
                        unlock_on_drop: AtomicBool::new(true),
                    }),
                });
            }
            Err(TryLockError::WouldBlock) => {
                if let Some(on_contention) = on_contention.take() {
                    on_contention();
                }
                let now = Instant::now();
                if now >= deadline {
                    return Err(WorktreeLockError::TimedOut {
                        path: lock_path,
                        timeout,
                    });
                }
                std::thread::sleep(LOCK_POLL_INTERVAL.min(deadline - now));
            }
            Err(TryLockError::Error(source)) => {
                return Err(WorktreeLockError::Io(anyhow::Error::new(source).context(
                    format!("Failed to acquire worktree lock '{}'", lock_path.display()),
                )));
            }
        }
    }
}

impl WorktreeLock {
    #[must_use]
    pub(super) fn lease(&self) -> WorktreeLockLease {
        WorktreeLockLease {
            _inner: Arc::clone(&self.inner),
        }
    }

    pub(super) fn close_without_unlock(self) {
        self.inner.unlock_on_drop.store(false, Ordering::Release);
    }
}

impl Drop for WorktreeLockInner {
    fn drop(&mut self) {
        if self.unlock_on_drop.load(Ordering::Acquire) {
            let _ = self.file.unlock();
        }
    }
}

#[cfg(unix)]
impl WorktreeLock {
    #[must_use]
    pub(crate) fn as_raw_fd(&self) -> std::os::unix::io::RawFd {
        use std::os::unix::io::AsRawFd;
        self.inner.file.as_raw_fd()
    }
}
