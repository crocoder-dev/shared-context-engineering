//! External-taint marker primitive for the mutation-cursor runtime boundary.
//!
//! A worktree-local filesystem marker at `<git-dir>/sce/mutation-cursor-tainted`.
//! Its existence is its entire state: armed write-ahead at the start of the
//! protected runtime section, cleared only after a proven durable completion, and
//! read by a later invocation as the external signal that the previous
//! invocation never proved a trustworthy durable completion.
//!
//! Durability mirrors [`crate::services::checkout`]'s `persist_checkout_id_inner`:
//! [`ExternalTaintMarker::persist`] creates and `fsync`s the marker file, and
//! both `persist` and [`ExternalTaintMarker::clear`] do a best-effort
//! `#[cfg(unix)]` parent-directory `sync_all` whose error is not propagated. This
//! protects against process error, non-graceful process exit, `SIGKILL`, and
//! normal runtime restart — not host power loss or a filesystem-level crash.
//!
//! The filesystem marker is never authoritative for normal cursor state, and it
//! is never deleted via `Drop`: only an explicit `clear()` after a successful
//! `CoordinateOutcome` removes it.

#![allow(dead_code)]

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Subdirectory inside `<git-dir>/` where SCE runtime metadata lives.
const SCE_RUNTIME_DIR: &str = "sce";

/// File name for the external-taint marker inside `<git-dir>/sce/`.
const MARKER_FILE: &str = "mutation-cursor-tainted";

/// Worktree-local external-taint marker.
///
/// Construct with [`ExternalTaintMarker::new`] from the worktree-specific Git
/// directory (as resolved by [`crate::services::checkout::resolve_git_dir`]).
/// Two linked worktrees resolve to two different Git directories and therefore
/// two independent markers.
#[derive(Debug, Clone)]
pub struct ExternalTaintMarker {
    marker_dir: PathBuf,
    marker_path: PathBuf,
}

impl ExternalTaintMarker {
    /// Builds the marker handle rooted at `<git_dir>/sce/mutation-cursor-tainted`.
    #[must_use]
    pub fn new(git_dir: &Path) -> Self {
        let marker_dir = git_dir.join(SCE_RUNTIME_DIR);
        let marker_path = marker_dir.join(MARKER_FILE);
        Self {
            marker_dir,
            marker_path,
        }
    }

    /// Returns `true` when the marker file is present.
    ///
    /// # Errors
    ///
    /// Returns an error when the marker path cannot be inspected for a reason
    /// other than absence.
    pub fn exists(&self) -> Result<bool> {
        match std::fs::symlink_metadata(&self.marker_path) {
            Ok(_) => Ok(true),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err).with_context(|| {
                format!(
                    "Failed to inspect external-taint marker '{}'",
                    self.marker_path.display()
                )
            }),
        }
    }

    /// Creates and `fsync`s the marker file.
    ///
    /// Idempotent: a marker that already exists is re-synced and left in place.
    ///
    /// # Errors
    ///
    /// Returns an error when the marker directory or file cannot be created or
    /// synced.
    pub fn persist(&self) -> Result<()> {
        std::fs::create_dir_all(&self.marker_dir).with_context(|| {
            format!(
                "Failed to create external-taint marker directory '{}'",
                self.marker_dir.display()
            )
        })?;

        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.marker_path)
            .with_context(|| {
                format!(
                    "Failed to create external-taint marker '{}'",
                    self.marker_path.display()
                )
            })?;
        file.sync_data().with_context(|| {
            format!(
                "Failed to sync external-taint marker '{}'",
                self.marker_path.display()
            )
        })?;
        drop(file);

        self.best_effort_sync_marker_dir();

        Ok(())
    }

    /// Removes the marker file.
    ///
    /// Idempotent: clearing an absent marker is success.
    ///
    /// # Errors
    ///
    /// Returns an error when the marker file exists but cannot be removed.
    pub fn clear(&self) -> Result<()> {
        match std::fs::remove_file(&self.marker_path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "Failed to clear external-taint marker '{}'",
                        self.marker_path.display()
                    )
                });
            }
        }

        self.best_effort_sync_marker_dir();

        Ok(())
    }

    #[cfg(unix)]
    fn best_effort_sync_marker_dir(&self) {
        if let Ok(dir) = std::fs::File::open(&self.marker_dir) {
            let _ = dir.sync_all();
        }
    }

    #[cfg(not(unix))]
    fn best_effort_sync_marker_dir(&self) {}
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_TEST_GIT_DIR_ID: AtomicU64 = AtomicU64::new(0);

    fn unique_test_git_dir(label: &str) -> PathBuf {
        let id = NEXT_TEST_GIT_DIR_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "sce-external-taint-{label}-{}-{id}",
            std::process::id()
        ))
    }

    fn remove_test_git_dir(git_dir: &Path) {
        let _ = std::fs::remove_dir_all(git_dir);
    }

    #[test]
    fn marker_is_worktree_scoped() {
        let git_dir_a = unique_test_git_dir("scoped-a");
        let git_dir_b = unique_test_git_dir("scoped-b");
        std::fs::create_dir_all(&git_dir_a).expect("git dir a should be created");
        std::fs::create_dir_all(&git_dir_b).expect("git dir b should be created");

        let marker_a = ExternalTaintMarker::new(&git_dir_a);
        let marker_b = ExternalTaintMarker::new(&git_dir_b);

        assert_ne!(
            marker_a.marker_path, marker_b.marker_path,
            "each git dir must derive an independent marker path"
        );

        marker_a.persist().expect("marker a should persist");

        assert!(
            marker_a
                .exists()
                .expect("marker a existence should resolve"),
            "the armed marker must be visible in its own worktree"
        );
        assert!(
            !marker_b
                .exists()
                .expect("marker b existence should resolve"),
            "arming worktree A must not arm worktree B"
        );

        remove_test_git_dir(&git_dir_a);
        remove_test_git_dir(&git_dir_b);
    }

    #[test]
    fn marker_persists_until_explicitly_cleared() {
        let git_dir = unique_test_git_dir("persists");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        ExternalTaintMarker::new(&git_dir)
            .persist()
            .expect("marker should persist");

        let reloaded = ExternalTaintMarker::new(&git_dir);
        assert!(
            reloaded.exists().expect("marker existence should resolve"),
            "the marker must survive reconstruction of the handle"
        );

        reloaded.clear().expect("marker should clear");
        assert!(
            !ExternalTaintMarker::new(&git_dir)
                .exists()
                .expect("marker existence should resolve"),
            "an explicitly cleared marker must be gone"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn persist_and_clear_are_idempotent() {
        let git_dir = unique_test_git_dir("idempotent");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let marker = ExternalTaintMarker::new(&git_dir);

        marker
            .clear()
            .expect("clear on absent marker should succeed");

        marker.persist().expect("first persist should succeed");
        marker.persist().expect("second persist should succeed");
        assert!(
            marker.exists().expect("marker existence should resolve"),
            "the marker must remain armed after repeated persist"
        );

        marker.clear().expect("first clear should succeed");
        marker.clear().expect("second clear should succeed");
        assert!(
            !marker.exists().expect("marker existence should resolve"),
            "the marker must remain cleared after repeated clear"
        );

        remove_test_git_dir(&git_dir);
    }
}
