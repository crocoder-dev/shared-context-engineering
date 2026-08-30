use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

const SCE_RUNTIME_DIR: &str = "sce";

const WORKTREE_LOCK_FILE: &str = "mutation-cursor.lock";

const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub struct WorktreeLock {
    file: File,
    path: PathBuf,
}

#[derive(Debug)]
pub enum WorktreeLockError {
    TimedOut { path: PathBuf, timeout: Duration },
    Io(anyhow::Error),
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
        }
    }
}

impl std::error::Error for WorktreeLockError {}

impl WorktreeLock {
    pub fn acquire(git_dir: &Path, timeout: Duration) -> Result<WorktreeLock, WorktreeLockError> {
        acquire_inner(git_dir, timeout, || {})
    }
}

fn acquire_inner<F>(
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
                    file,
                    path: lock_path,
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

impl Drop for WorktreeLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::thread;

    use super::*;

    static NEXT_TEST_GIT_DIR_ID: AtomicU64 = AtomicU64::new(0);

    fn unique_test_git_dir(label: &str) -> PathBuf {
        let id = NEXT_TEST_GIT_DIR_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "sce-worktree-lock-{label}-{}-{id}",
            std::process::id()
        ))
    }

    fn remove_test_git_dir(git_dir: &Path) {
        let _ = std::fs::remove_dir_all(git_dir);
    }

    #[test]
    fn a_second_acquirer_blocks_until_the_first_releases() {
        let git_dir = unique_test_git_dir("contention");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let first = WorktreeLock::acquire(&git_dir, Duration::from_secs(5))
            .expect("first acquirer should succeed immediately");

        let (contention_tx, contention_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let git_dir_clone = git_dir.clone();
        let handle = thread::spawn(move || {
            let result = acquire_inner(&git_dir_clone, Duration::from_secs(5), || {
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
            .expect("worker should observe OS-level lock contention (TryLockError::WouldBlock)");

        assert!(
            result_rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "second acquirer should not succeed while the first still holds the lock"
        );

        drop(first);

        result_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("second acquirer should complete once the first releases the lock");

        let result = handle
            .join()
            .expect("second acquirer thread should not panic");
        assert!(
            result.is_ok(),
            "second acquirer should succeed once the first releases the lock"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn distinct_worktree_paths_do_not_contend() {
        let git_dir_a = unique_test_git_dir("distinct-a");
        let git_dir_b = unique_test_git_dir("distinct-b");
        std::fs::create_dir_all(&git_dir_a).expect("git dir a should be created");
        std::fs::create_dir_all(&git_dir_b).expect("git dir b should be created");

        let lock_a = WorktreeLock::acquire(&git_dir_a, Duration::from_millis(200))
            .expect("lock on distinct path a should succeed");
        let lock_b = WorktreeLock::acquire(&git_dir_b, Duration::from_millis(200))
            .expect("lock on distinct path b should succeed independently of path a");

        drop(lock_a);
        drop(lock_b);
        remove_test_git_dir(&git_dir_a);
        remove_test_git_dir(&git_dir_b);
    }

    #[test]
    fn acquire_times_out_with_a_distinct_matchable_error_when_still_held() {
        let git_dir = unique_test_git_dir("timeout");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let holder = WorktreeLock::acquire(&git_dir, Duration::from_secs(5))
            .expect("first acquirer should succeed immediately");

        let result = WorktreeLock::acquire(&git_dir, Duration::from_millis(250));
        match result {
            Err(WorktreeLockError::TimedOut { timeout, .. }) => {
                assert_eq!(timeout, Duration::from_millis(250));
            }
            other => panic!("expected a distinct TimedOut error, got {other:?}"),
        }

        drop(holder);
        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn a_leftover_lock_file_with_no_active_os_lock_does_not_block_a_new_acquirer() {
        let git_dir = unique_test_git_dir("stale-file");
        let runtime_dir = git_dir.join(SCE_RUNTIME_DIR);
        std::fs::create_dir_all(&runtime_dir).expect("runtime dir should be created");

        let lock_path = runtime_dir.join(WORKTREE_LOCK_FILE);
        std::fs::write(&lock_path, b"leftover").expect("leftover lock file should be writable");

        let result = WorktreeLock::acquire(&git_dir, Duration::from_millis(200));
        assert!(
            result.is_ok(),
            "a lock file with no active OS lock held against it must not block a new acquirer"
        );

        remove_test_git_dir(&git_dir);
    }
}
