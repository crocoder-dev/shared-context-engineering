use std::path::{Path, PathBuf};
use std::time::Duration;

use super::os_lock::{AdvisoryLockError, OsAdvisoryLock};
use super::state::adapter_state_dir;

const ADAPTER_BOUNDARY_LOCK_FILE: &str = "opencode-mutation-scope-boundary.lock";
const BOUNDARY_LOCK_WHAT: &str = "adapter-boundary";

pub(crate) const DEFAULT_BOUNDARY_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn boundary_lock_path(git_dir: &Path) -> PathBuf {
    adapter_state_dir(git_dir).join(ADAPTER_BOUNDARY_LOCK_FILE)
}

pub(crate) struct AdapterBoundaryLock {
    _inner: OsAdvisoryLock,
}

impl AdapterBoundaryLock {
    pub(crate) fn acquire(
        git_dir: &Path,
        timeout: Duration,
    ) -> Result<AdapterBoundaryLock, AdvisoryLockError> {
        let inner = OsAdvisoryLock::acquire(
            &adapter_state_dir(git_dir),
            boundary_lock_path(git_dir),
            timeout,
            BOUNDARY_LOCK_WHAT,
        )?;
        Ok(AdapterBoundaryLock { _inner: inner })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::thread;

    use super::*;

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    fn unique_git_dir(label: &str) -> PathBuf {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "sce-opencode-mutation-scope-boundary-{label}-{}-{id}",
            std::process::id()
        ))
    }

    #[test]
    fn a_leftover_boundary_lock_file_alone_does_not_block_acquisition() {
        let git_dir = unique_git_dir("leftover-file");
        std::fs::create_dir_all(adapter_state_dir(&git_dir)).expect("state dir should be created");
        std::fs::write(boundary_lock_path(&git_dir), b"leftover")
            .expect("leftover lock file should be writable");

        AdapterBoundaryLock::acquire(&git_dir, Duration::from_millis(200))
            .expect("a lock file with no live OS owner must not block a new acquirer");

        let _ = std::fs::remove_dir_all(&git_dir);
    }

    #[test]
    fn a_second_in_process_acquirer_blocks_until_the_first_releases() {
        let git_dir = unique_git_dir("in-process-contention");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let holder = AdapterBoundaryLock::acquire(&git_dir, Duration::from_secs(5))
            .expect("first acquirer should succeed immediately");

        let (tx, rx) = mpsc::channel();
        let git_dir_clone = git_dir.clone();
        let handle = thread::spawn(move || {
            let result = AdapterBoundaryLock::acquire(&git_dir_clone, Duration::from_secs(5));
            let _ = tx.send(());
            result.is_ok()
        });

        assert!(
            rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "the second acquirer must not proceed while the first holds the boundary lock",
        );

        drop(holder);

        rx.recv_timeout(Duration::from_secs(5))
            .expect("the second acquirer should complete once the first releases");
        assert!(handle
            .join()
            .expect("second acquirer thread should not panic"));

        let _ = std::fs::remove_dir_all(&git_dir);
    }

    #[test]
    fn the_boundary_lock_path_is_distinct_from_the_state_lock_and_lives_under_sce() {
        let git_dir = unique_git_dir("path-shape");
        let path = boundary_lock_path(&git_dir);
        assert!(path.starts_with(adapter_state_dir(&git_dir)));
        assert!(path.ends_with(ADAPTER_BOUNDARY_LOCK_FILE));
        assert_ne!(
            path.file_name(),
            Path::new("opencode-mutation-scope-state.lock").file_name(),
        );
    }
}
