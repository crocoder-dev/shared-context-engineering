use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context;

const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Debug)]
pub(crate) enum AdvisoryLockError {
    TimedOut {
        path: PathBuf,
        timeout: Duration,
        what: &'static str,
    },
    Io(anyhow::Error),
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
        }
    }
}

impl std::error::Error for AdvisoryLockError {}

pub(crate) struct OsAdvisoryLock {
    file: File,
}

impl OsAdvisoryLock {
    pub(crate) fn acquire(
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
