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

    pub(crate) fn acquire(&self, adapter_state_dir: &Path) -> anyhow::Result<OsAdvisoryLock> {
        self.acquire_with_timeout(adapter_state_dir, self.default_timeout)
            .map_err(|error| anyhow!("{}: {error}", self.failure_context))
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

#[cfg(any())]
mod tests {
    use std::io::Write as _;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::thread;

    use super::*;
    use crate::services::hooks::{
        codex_mutation_scope, opencode_mutation_scope, pi_mutation_scope,
    };

    const TEST_LOCK: AdapterLockSpec = AdapterLockSpec::boundary("test-mutation-scope.lock");

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    fn unique_lock_dir(label: &str) -> PathBuf {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "sce-mutation-scope-lock-{label}-{}-{id}",
            std::process::id()
        ))
    }

    const CHILD_ENV_LOCK_DIR: &str = "SCE_MUTATION_SCOPE_LOCK_CHILD_DIR";
    const CHILD_ENV_TIMEOUT_MS: &str = "SCE_MUTATION_SCOPE_LOCK_CHILD_TIMEOUT_MS";
    const CHILD_HELPER_PATH: &str =
        "services::hooks::mutation_scope_lock::tests::mutation_scope_lock_child_helper";

    #[test]
    #[ignore = "subprocess helper, driven by process_death_releases_the_lock"]
    fn mutation_scope_lock_child_helper() {
        let lock_dir = PathBuf::from(
            std::env::var(CHILD_ENV_LOCK_DIR).expect("child helper needs a lock dir in the env"),
        );
        let timeout = Duration::from_millis(
            std::env::var(CHILD_ENV_TIMEOUT_MS)
                .expect("child helper needs a timeout in the env")
                .parse()
                .expect("timeout must parse"),
        );
        match TEST_LOCK.acquire_with_timeout(&lock_dir, timeout) {
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

    fn run_child(lock_dir: &Path, timeout: Duration) -> String {
        let exe = std::env::current_exe().expect("test executable path should resolve");
        let output = Command::new(exe)
            .args(["--exact", "--ignored", "--nocapture", CHILD_HELPER_PATH])
            .env(CHILD_ENV_LOCK_DIR, lock_dir)
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
    fn process_death_releases_the_lock() {
        let lock_dir = unique_lock_dir("process-death");

        let held = TEST_LOCK
            .acquire_with_timeout(&lock_dir, Duration::from_secs(5))
            .expect("the parent should acquire the lock immediately");

        assert_eq!(
            run_child(&lock_dir, Duration::from_millis(300)),
            "TIMEOUT",
            "a second OS process must not acquire the lock while the parent holds it",
        );

        drop(held);

        assert_eq!(
            run_child(&lock_dir, Duration::from_millis(300)),
            "ACQUIRED",
            "once the parent releases, a child process acquires it",
        );

        let reacquired = TEST_LOCK
            .acquire_with_timeout(&lock_dir, Duration::from_secs(2))
            .expect("the child's process exit must have released the lock");
        drop(reacquired);

        let _ = std::fs::remove_dir_all(&lock_dir);
    }

    #[test]
    fn a_leftover_lock_file_alone_does_not_block_acquisition_and_is_never_truncated_or_removed() {
        let lock_dir = unique_lock_dir("leftover-file");
        std::fs::create_dir_all(&lock_dir).expect("lock dir should be created");
        let lock_path = TEST_LOCK.path(&lock_dir);
        std::fs::write(&lock_path, b"leftover").expect("leftover lock file should be writable");

        let lock = TEST_LOCK
            .acquire_with_timeout(&lock_dir, Duration::from_millis(200))
            .expect("a lock file with no live OS owner must not block a new acquirer");
        drop(lock);

        assert_eq!(
            std::fs::read(&lock_path).expect("the lock file must survive release"),
            b"leftover",
            "ownership must never be expressed by truncating or removing the lock file",
        );

        let _ = std::fs::remove_dir_all(&lock_dir);
    }

    #[test]
    fn a_second_in_process_acquirer_blocks_until_the_first_releases() {
        let lock_dir = unique_lock_dir("in-process-contention");

        let holder = TEST_LOCK
            .acquire_with_timeout(&lock_dir, Duration::from_secs(5))
            .expect("first acquirer should succeed immediately");

        let (tx, rx) = mpsc::channel();
        let lock_dir_clone = lock_dir.clone();
        let handle = thread::spawn(move || {
            let result = TEST_LOCK.acquire_with_timeout(&lock_dir_clone, Duration::from_secs(5));
            let _ = tx.send(());
            result.is_ok()
        });

        assert!(
            rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "the second acquirer must not proceed while the first holds the lock",
        );

        drop(holder);

        rx.recv_timeout(Duration::from_secs(5))
            .expect("the second acquirer should complete once the first releases");
        assert!(handle
            .join()
            .expect("second acquirer thread should not panic"));

        let _ = std::fs::remove_dir_all(&lock_dir);
    }

    #[test]
    fn a_contended_acquire_times_out_with_a_role_specific_diagnostic() {
        let lock_dir = unique_lock_dir("timeout");
        let state_lock = AdapterLockSpec::state("test-mutation-scope-state.lock");

        let _boundary_holder = TEST_LOCK
            .acquire_with_timeout(&lock_dir, Duration::from_secs(5))
            .expect("boundary holder should acquire immediately");
        let _state_holder = state_lock
            .acquire_with_timeout(&lock_dir, Duration::from_secs(5))
            .expect("state holder should acquire immediately");

        for (spec, what) in [
            (TEST_LOCK, "adapter-boundary"),
            (state_lock, "adapter-state"),
        ] {
            let timeout = Duration::from_millis(100);
            let started = Instant::now();
            let Err(error) = spec.acquire_with_timeout(&lock_dir, timeout) else {
                panic!("a held {what} lock must not be acquired a second time");
            };
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "a contended acquire must give up at its timeout",
            );
            let expected = format!(
                "Timed out after {timeout:?} waiting for the {what} lock '{}'",
                spec.path(&lock_dir).display()
            );
            assert_eq!(error.to_string(), expected);
            assert!(matches!(
                &error,
                AdvisoryLockError::TimedOut { path, timeout: t, what: w }
                    if *path == spec.path(&lock_dir) && *t == timeout && *w == what
            ));
        }

        let _ = std::fs::remove_dir_all(&lock_dir);
    }

    #[test]
    fn adapter_lock_identities_are_exact_distinct_and_below_the_adapter_state_dir() {
        let git_dir = unique_lock_dir("adapter-identities");
        let rows = [
            (
                codex_mutation_scope::state::adapter_state_dir(&git_dir),
                codex_mutation_scope::state::STATE_LOCK,
                codex_mutation_scope::state::BOUNDARY_LOCK,
                "codex-mutation-scope-state.lock",
                "codex-mutation-scope-boundary.lock",
            ),
            (
                opencode_mutation_scope::state::adapter_state_dir(&git_dir),
                opencode_mutation_scope::state::STATE_LOCK,
                opencode_mutation_scope::state::BOUNDARY_LOCK,
                "opencode-mutation-scope-state.lock",
                "opencode-mutation-scope-boundary.lock",
            ),
            (
                pi_mutation_scope::state::adapter_state_dir(&git_dir),
                pi_mutation_scope::state::STATE_LOCK,
                pi_mutation_scope::state::BOUNDARY_LOCK,
                "pi-mutation-scope-state.lock",
                "pi-mutation-scope-boundary.lock",
            ),
        ];

        let sce_dir = git_dir.join("sce");
        for (state_dir, state_lock, boundary_lock, state_file, boundary_file) in rows {
            assert_eq!(state_dir, sce_dir);
            assert_eq!(state_lock, AdapterLockSpec::state(state_file));
            assert_eq!(boundary_lock, AdapterLockSpec::boundary(boundary_file));
            assert_eq!(state_lock.path(&state_dir), sce_dir.join(state_file));
            assert_eq!(boundary_lock.path(&state_dir), sce_dir.join(boundary_file));
            assert_ne!(state_lock.path(&state_dir), boundary_lock.path(&state_dir));
        }
    }
}
