use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context};

const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(20);

pub(crate) const DEFAULT_STATE_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const DEFAULT_BOUNDARY_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

const STATE_LOCK_WHAT: &str = "adapter-state";
const BOUNDARY_LOCK_WHAT: &str = "adapter-boundary";

const STATE_LOCK_FAILURE_CONTEXT: &str = "Failed to acquire adapter-state lock";
const BOUNDARY_LOCK_FAILURE_CONTEXT: &str = "Failed to acquire adapter boundary lock";

#[derive(Debug)]
pub(crate) enum AdvisoryLockError {
    TimedOut {
        path: PathBuf,
        timeout: Duration,
        what: &'static str,
    },
    Io(anyhow::Error),
    WorkerFailed {
        what: &'static str,
        source: tokio::task::JoinError,
    },
}

impl std::fmt::Display for AdvisoryLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdvisoryLockError::TimedOut {
                path,
                timeout,
                what,
            } => write!(
                f,
                "Timed out after {timeout:?} waiting for the {what} lock '{}'",
                path.display()
            ),
            AdvisoryLockError::Io(source) => write!(f, "{source}"),
            AdvisoryLockError::WorkerFailed { what, source } => {
                write!(f, "The {what} lock acquisition worker failed: {source}")
            }
        }
    }
}

impl std::error::Error for AdvisoryLockError {}

pub(crate) struct OsAdvisoryLock {
    file: File,
}

impl OsAdvisoryLock {
    fn acquire(
        parent_dir: &Path,
        lock_path: PathBuf,
        timeout: Duration,
        what: &'static str,
    ) -> Result<OsAdvisoryLock, AdvisoryLockError> {
        std::fs::create_dir_all(parent_dir)
            .with_context(|| {
                format!(
                    "Failed to create {what} lock directory '{}'",
                    parent_dir.display()
                )
            })
            .map_err(AdvisoryLockError::Io)?;

        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("Failed to open {what} lock file '{}'", lock_path.display()))
            .map_err(AdvisoryLockError::Io)?;

        let deadline = Instant::now() + timeout;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(OsAdvisoryLock { file }),
                Err(TryLockError::WouldBlock) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(AdvisoryLockError::TimedOut {
                            path: lock_path,
                            timeout,
                            what,
                        });
                    }
                    std::thread::sleep(LOCK_POLL_INTERVAL.min(deadline - now));
                }
                Err(TryLockError::Error(source)) => {
                    return Err(AdvisoryLockError::Io(anyhow::Error::new(source).context(
                        format!("Failed to acquire {what} lock '{}'", lock_path.display()),
                    )));
                }
            }
        }
    }
}

impl Drop for OsAdvisoryLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AdapterLockSpec {
    file_name: &'static str,
    what: &'static str,
    default_timeout: Duration,
    failure_context: &'static str,
}

impl AdapterLockSpec {
    pub(crate) const fn state(file_name: &'static str) -> Self {
        AdapterLockSpec {
            file_name,
            what: STATE_LOCK_WHAT,
            default_timeout: DEFAULT_STATE_LOCK_TIMEOUT,
            failure_context: STATE_LOCK_FAILURE_CONTEXT,
        }
    }

    pub(crate) const fn boundary(file_name: &'static str) -> Self {
        AdapterLockSpec {
            file_name,
            what: BOUNDARY_LOCK_WHAT,
            default_timeout: DEFAULT_BOUNDARY_LOCK_TIMEOUT,
            failure_context: BOUNDARY_LOCK_FAILURE_CONTEXT,
        }
    }

    pub(crate) fn path(&self, adapter_state_dir: &Path) -> PathBuf {
        adapter_state_dir.join(self.file_name)
    }

    pub(crate) async fn acquire_async(
        &self,
        adapter_state_dir: &Path,
    ) -> anyhow::Result<OsAdvisoryLock> {
        self.acquire_with_timeout_async(adapter_state_dir, self.default_timeout)
            .await
            .map_err(|error| anyhow!("{}: {error}", self.failure_context))
    }

    pub(crate) async fn acquire_with_timeout_async(
        &self,
        adapter_state_dir: &Path,
        timeout: Duration,
    ) -> Result<OsAdvisoryLock, AdvisoryLockError> {
        let spec = *self;
        let adapter_state_dir = adapter_state_dir.to_owned();
        tokio::task::spawn_blocking(move || spec.acquire_with_timeout(&adapter_state_dir, timeout))
            .await
            .map_err(|source| AdvisoryLockError::WorkerFailed {
                what: spec.what,
                source,
            })?
    }

    pub(crate) fn acquire_with_timeout(
        &self,
        adapter_state_dir: &Path,
        timeout: Duration,
    ) -> Result<OsAdvisoryLock, AdvisoryLockError> {
        OsAdvisoryLock::acquire(
            adapter_state_dir,
            self.path(adapter_state_dir),
            timeout,
            self.what,
        )
    }
}
