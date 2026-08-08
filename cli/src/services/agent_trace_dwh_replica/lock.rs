use std::{
    fmt, fs,
    fs::File,
    path::{Path, PathBuf},
};

use fs4::fs_std::FileExt;

/// A held, non-blocking exclusive OS lock on a repository's Agent Trace DWH
/// sync replica bridge-lock file.
///
/// Exactly one `BridgeLock` may be held for a given path at a time: a second
/// attempt while this guard is alive (in this or another process) fails with
/// [`BridgeLockError::AlreadyHeld`]. The underlying lock file is created if
/// missing and is never deleted, by this guard or on drop; only the OS lock
/// is released, which happens automatically when the guard is dropped or the
/// owning process exits.
#[derive(Debug)]
pub struct BridgeLock {
    path: PathBuf,
    file: File,
}

impl BridgeLock {
    /// Acquire the bridge lock at `path` without blocking.
    ///
    /// Creates the lock file (and its parent directory) if missing. Returns
    /// [`BridgeLockError::AlreadyHeld`] when another process or guard
    /// currently holds the lock, and [`BridgeLockError::Io`] for any other
    /// filesystem failure.
    pub fn acquire(path: impl AsRef<Path>) -> Result<Self, BridgeLockError> {
        let path = path.as_ref().to_path_buf();

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| BridgeLockError::Io {
                path: path.clone(),
                source,
            })?;
        }

        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|source| BridgeLockError::Io {
                path: path.clone(),
                source,
            })?;

        let acquired = file
            .try_lock_exclusive()
            .map_err(|source| BridgeLockError::Io {
                path: path.clone(),
                source,
            })?;

        if !acquired {
            return Err(BridgeLockError::AlreadyHeld { path });
        }

        Ok(Self { path, file })
    }

    /// The bridge-lock file path this guard holds.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for BridgeLock {
    fn drop(&mut self) {
        // Best-effort: the OS also releases the lock when the file
        // descriptor closes at process exit. The lock file itself is
        // intentionally retained on disk.
        let _ = FileExt::unlock(&self.file);
    }
}

/// Failure acquiring a [`BridgeLock`].
#[derive(Debug)]
pub enum BridgeLockError {
    /// Another process or guard already owns the lock at `path`.
    AlreadyHeld { path: PathBuf },
    /// A filesystem operation failed while acquiring the lock.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for BridgeLockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyHeld { path } => write!(
                f,
                "Agent Trace DWH replica at {} is already in use by another process; \
                 stop the other process (or wait for it to exit) and retry",
                path.display()
            ),
            Self::Io { path, source } => write!(
                f,
                "failed to acquire Agent Trace DWH replica bridge lock at {}: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for BridgeLockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::AlreadyHeld { .. } => None,
            Self::Io { source, .. } => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn unique_test_lock_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "sce-agent-trace-dwh-replica-lock-{label}-{}-{nonce}",
                std::process::id()
            ))
            .join("agent-trace-sync.db.bridge-lock")
    }

    fn remove_test_lock(path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn agent_trace_dwh_replica_lock_acquire_creates_and_retains_the_lock_file() {
        let path = unique_test_lock_path("creates-file");

        let guard = BridgeLock::acquire(&path).expect("first acquire should succeed");
        assert!(path.is_file(), "lock file should be created");
        assert_eq!(guard.path(), path.as_path());

        drop(guard);
        assert!(
            path.is_file(),
            "lock file must remain on disk after the guard drops"
        );

        remove_test_lock(&path);
    }

    #[test]
    fn agent_trace_dwh_replica_lock_concurrent_acquire_is_rejected_with_actionable_guidance() {
        let path = unique_test_lock_path("concurrent-rejected");

        let _first = BridgeLock::acquire(&path).expect("first acquire should succeed");
        let error = BridgeLock::acquire(&path).expect_err("second concurrent acquire should fail");

        assert!(matches!(error, BridgeLockError::AlreadyHeld { .. }));
        let message = error.to_string();
        assert!(message.contains("already in use"));
        assert!(message.contains("retry"));

        remove_test_lock(&path);
    }

    #[test]
    fn agent_trace_dwh_replica_lock_is_reacquirable_after_the_guard_drops() {
        let path = unique_test_lock_path("reacquire-after-drop");

        let first = BridgeLock::acquire(&path).expect("first acquire should succeed");
        drop(first);

        let second =
            BridgeLock::acquire(&path).expect("acquire should succeed after prior guard dropped");
        drop(second);

        remove_test_lock(&path);
    }

    #[test]
    fn agent_trace_dwh_replica_lock_is_reacquirable_after_a_real_subprocess_holding_it_is_killed() {
        let path = unique_test_lock_path("reacquire-after-process-death");
        let ready_path = path.with_extension("ready-signal");
        let _ = fs::remove_file(&ready_path);

        // Re-invoke this same compiled test binary as a child process that
        // acquires the lock and blocks forever, then signals readiness via a
        // separate file. Killing that child (rather than letting `Drop` run)
        // proves the OS releases the flock on process death, not merely on
        // our own unlock call.
        let current_exe = std::env::current_exe().expect("resolve current test binary");
        let mut child = Command::new(current_exe)
            .arg("agent_trace_dwh_replica_lock_hold_lock_until_killed_for_subprocess_death_test")
            .arg("--ignored")
            .arg("--test-threads=1")
            .env("SCE_BRIDGE_LOCK_HOLD_PATH", &path)
            .env("SCE_BRIDGE_LOCK_READY_PATH", &ready_path)
            .spawn()
            .expect("spawn lock-holding subprocess");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !ready_path.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "subprocess did not signal lock acquisition in time"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        let contended =
            BridgeLock::acquire(&path).expect_err("lock should be held by the live subprocess");
        assert!(matches!(contended, BridgeLockError::AlreadyHeld { .. }));

        child.kill().expect("kill lock-holding subprocess");
        child.wait().expect("reap killed subprocess");

        let guard = BridgeLock::acquire(&path)
            .expect("lock should be acquirable once the killed process releases it");
        drop(guard);

        let _ = fs::remove_file(&ready_path);
        remove_test_lock(&path);
    }

    /// Not a real test case: a subprocess entry point spawned by
    /// `agent_trace_dwh_replica_lock_is_reacquirable_after_a_real_subprocess_holding_it_is_killed`.
    /// Acquires the lock at `SCE_BRIDGE_LOCK_HOLD_PATH`, signals readiness by
    /// creating `SCE_BRIDGE_LOCK_READY_PATH`, then blocks until killed.
    #[test]
    #[ignore = "invoked as a subprocess helper, not a standalone test"]
    fn agent_trace_dwh_replica_lock_hold_lock_until_killed_for_subprocess_death_test() {
        let path = PathBuf::from(
            std::env::var("SCE_BRIDGE_LOCK_HOLD_PATH")
                .expect("SCE_BRIDGE_LOCK_HOLD_PATH should be set by the parent test"),
        );
        let ready_path = PathBuf::from(
            std::env::var("SCE_BRIDGE_LOCK_READY_PATH")
                .expect("SCE_BRIDGE_LOCK_READY_PATH should be set by the parent test"),
        );

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create lock parent dir in subprocess helper");
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .expect("open lock file in subprocess helper");
        assert!(
            file.try_lock_exclusive()
                .expect("subprocess helper should be able to lock"),
            "subprocess helper should acquire an uncontended lock"
        );

        fs::write(&ready_path, b"ready").expect("write readiness signal");
        std::thread::sleep(std::time::Duration::from_hours(1));
    }

    #[test]
    fn agent_trace_dwh_replica_lock_same_process_reacquisition_is_rejected_while_first_guard_is_alive(
    ) {
        let path = unique_test_lock_path("same-process-contention");

        let _guard = BridgeLock::acquire(&path).expect("first acquire should succeed");
        let error = BridgeLock::acquire(&path)
            .expect_err("a second guard in the same process must not double-acquire the lock");
        assert!(matches!(error, BridgeLockError::AlreadyHeld { .. }));

        remove_test_lock(&path);
    }

    #[test]
    fn agent_trace_dwh_replica_lock_holding_the_bridge_lock_does_not_block_source_agent_trace_db_work(
    ) {
        use crate::services::agent_trace_db::{
            repository::RepositoryAgentTraceDb, DiffTraceInsert, PAYLOAD_TYPE_PATCH,
        };

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let repo_dir = std::env::temp_dir().join(format!(
            "sce-agent-trace-dwh-replica-lock-source-isolation-{}-{nonce}",
            std::process::id()
        ));
        let lock_path = repo_dir.join("agent-trace-sync.db.bridge-lock");
        let source_db_path = repo_dir.join("agent-trace.db");

        let guard = BridgeLock::acquire(&lock_path).expect("bridge lock should acquire");

        let source_db =
            RepositoryAgentTraceDb::new_at(&source_db_path).expect("source DB should open");
        source_db
            .insert_diff_trace(DiffTraceInsert {
                time_ms: 1_000,
                session_id: "session-1",
                patch: "Index: notes/a.md\n===================================================================\n--- notes/a.md\n+++ notes/a.md\n@@ -0,0 +1,1 @@\n+hello\n",
                model_id: None,
                tool_name: "opencode",
                tool_version: None,
                payload_type: PAYLOAD_TYPE_PATCH,
            })
            .expect("source DB work should succeed while the bridge lock is held");

        drop(source_db);
        drop(guard);
        let _ = fs::remove_dir_all(&repo_dir);
    }
}
