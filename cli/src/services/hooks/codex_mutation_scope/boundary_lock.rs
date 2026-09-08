use std::path::{Path, PathBuf};
use std::time::Duration;

use super::os_lock::{AdvisoryLockError, OsAdvisoryLock};
use super::state::adapter_state_dir;

const ADAPTER_BOUNDARY_LOCK_FILE: &str = "codex-mutation-scope-boundary.lock";
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
    use std::io::Write as _;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::thread;

    use super::*;

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    fn unique_git_dir(label: &str) -> PathBuf {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "sce-codex-mutation-scope-boundary-{label}-{}-{id}",
            std::process::id()
        ))
    }

    const CHILD_ENV_GIT_DIR: &str = "SCE_BOUNDARY_LOCK_CHILD_GIT_DIR";
    const CHILD_ENV_TIMEOUT_MS: &str = "SCE_BOUNDARY_LOCK_CHILD_TIMEOUT_MS";
    const CHILD_HELPER_PATH: &str =
        "services::hooks::codex_mutation_scope::boundary_lock::tests::boundary_lock_child_helper";

    #[test]
    #[ignore = "subprocess helper, driven by process_death_releases_the_boundary_lock"]
    fn boundary_lock_child_helper() {
        let git_dir = PathBuf::from(
            std::env::var(CHILD_ENV_GIT_DIR).expect("child helper needs a git dir in the env"),
        );
        let timeout = Duration::from_millis(
            std::env::var(CHILD_ENV_TIMEOUT_MS)
                .expect("child helper needs a timeout in the env")
                .parse()
                .expect("timeout must parse"),
        );
        match AdapterBoundaryLock::acquire(&git_dir, timeout) {
            Ok(lock) => {
                print!("ACQUIRED");
                std::io::stdout().flush().expect("flush stdout");
                std::mem::forget(lock);
            }
            Err(AdvisoryLockError::TimedOut { .. }) => {
                print!("TIMEOUT");
                std::io::stdout().flush().expect("flush stdout");
            }
            Err(other) => panic!("unexpected child lock error: {other}"),
        }
    }

    fn run_child(git_dir: &Path, timeout: Duration) -> String {
        let exe = std::env::current_exe().expect("test executable path should resolve");
        let output = Command::new(exe)
            .args(["--exact", "--ignored", "--nocapture", CHILD_HELPER_PATH])
            .env(CHILD_ENV_GIT_DIR, git_dir)
            .env(CHILD_ENV_TIMEOUT_MS, timeout.as_millis().to_string())
            .output()
            .expect("the child test process should spawn");
        assert!(
            output.status.success(),
            "child process failed: stdout={:?} stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.contains("ACQUIRED") {
            "ACQUIRED".to_string()
        } else if stdout.contains("TIMEOUT") {
            "TIMEOUT".to_string()
        } else {
            panic!("child produced no lock verdict: {stdout}");
        }
    }

    #[test]
    fn process_death_releases_the_boundary_lock() {
        let git_dir = unique_git_dir("process-death");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let held = AdapterBoundaryLock::acquire(&git_dir, Duration::from_secs(5))
            .expect("the parent should acquire the boundary lock immediately");

        assert_eq!(
            run_child(&git_dir, Duration::from_millis(300)),
            "TIMEOUT",
            "a second OS process must not acquire the boundary lock while the parent holds it",
        );

        drop(held);

        assert_eq!(
            run_child(&git_dir, Duration::from_millis(300)),
            "ACQUIRED",
            "once the parent releases, a child process acquires it",
        );

        let reacquired = AdapterBoundaryLock::acquire(&git_dir, Duration::from_secs(2))
            .expect("the child's process exit must have released the boundary lock");
        drop(reacquired);

        let _ = std::fs::remove_dir_all(&git_dir);
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
            Path::new("codex-mutation-scope-state.lock").file_name(),
        );
    }
}
