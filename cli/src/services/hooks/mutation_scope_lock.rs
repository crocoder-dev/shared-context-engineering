use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context};

const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(20);

pub(crate) const DEFAULT_STATE_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const DEFAULT_BOUNDARY_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

const STATE_LOCK_WHAT: &str = "adapter-state";
const BOUNDARY_LOCK_WHAT: &str = "adapter-boundary";

const STATE_LOCK_FAILURE_CONTEXT: &str = "Failed to acquire adapter-state lock";
const BOUNDARY_LOCK_FAILURE_CONTEXT: &str = "Failed to acquire adapter boundary lock";
const LOCKED_OPERATION_WORKER_FAILURE_CONTEXT: &str = "The locked operation worker failed";

tokio::task_local! {
    static BOUNDARY_LEASE: Arc<OsAdvisoryLock>;
}

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

    pub(crate) async fn run_locked_blocking<T, F>(
        &self,
        adapter_state_dir: &Path,
        operation: F,
    ) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce() -> anyhow::Result<T> + Send + 'static,
    {
        let spec = *self;
        let adapter_state_dir = adapter_state_dir.to_owned();
        let boundary_lease = BOUNDARY_LEASE.try_with(Arc::clone).ok();
        tokio::task::spawn_blocking(move || {
            let _boundary_lease = boundary_lease;
            let _lock = spec
                .acquire_with_timeout(&adapter_state_dir, spec.default_timeout)
                .map_err(|error| anyhow!("{}: {error}", spec.failure_context))?;
            operation()
        })
        .await
        .map_err(|source| {
            anyhow!(
                "{LOCKED_OPERATION_WORKER_FAILURE_CONTEXT} for the {} lock: {source}",
                spec.what
            )
        })?
    }

    pub(crate) async fn run_under_boundary<T>(
        &self,
        adapter_state_dir: &Path,
        operation: impl std::ops::AsyncFnOnce() -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let lease = Arc::new(self.acquire_async(adapter_state_dir).await?);
        BOUNDARY_LEASE.scope(lease, operation()).await
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

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::{AdapterLockSpec, AdvisoryLockError};

    const TEST_BOUNDARY: AdapterLockSpec = AdapterLockSpec::boundary("test-boundary.lock");
    const TEST_STATE: AdapterLockSpec = AdapterLockSpec::state("test-state.lock");
    const PROBE_TIMEOUT: Duration = Duration::from_millis(200);
    const WORKER_TIMEOUT: Duration = Duration::from_secs(10);

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_boundary_keeps_its_lock_until_the_started_state_transition_finishes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let state_dir = dir.path().to_path_buf();
        let (started_tx, started_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();

        let boundary_task = tokio::spawn({
            let state_dir = state_dir.clone();
            async move {
                TEST_BOUNDARY
                    .run_under_boundary(&state_dir.clone(), async || {
                        TEST_STATE
                            .run_locked_blocking(&state_dir, move || {
                                started_tx.send(()).expect("started signal");
                                release_rx
                                    .recv_timeout(WORKER_TIMEOUT)
                                    .expect("release signal");
                                Ok(())
                            })
                            .await
                    })
                    .await
            }
        });

        tokio::task::spawn_blocking(move || started_rx.recv_timeout(WORKER_TIMEOUT))
            .await
            .expect("started wait joins")
            .expect("state transition starts");

        boundary_task.abort();
        assert!(boundary_task.await.expect_err("aborted").is_cancelled());

        let overtaken = TEST_BOUNDARY
            .acquire_with_timeout_async(&state_dir, PROBE_TIMEOUT)
            .await;
        assert!(
            matches!(overtaken, Err(AdvisoryLockError::TimedOut { .. })),
            "a second boundary must not overtake the paused state transition"
        );

        release_tx.send(()).expect("release worker");
        TEST_BOUNDARY
            .acquire_with_timeout_async(&state_dir, WORKER_TIMEOUT)
            .await
            .expect("boundary is free once the transition finished");
    }
}
