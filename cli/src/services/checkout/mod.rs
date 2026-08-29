//! Checkout identity service.
//!
//! Each cloned repository (and linked Git worktree) gets its own stable checkout
//! identity stored in `<git-dir>/sce/checkout-id`. The checkout ID is a `UUIDv7`
//! string, consistent with the existing `agent_trace_id` convention in this
//! codebase.
//!
//! Checkout identity is repository metadata used by Agent Trace diagnostics.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use uuid::Uuid;

/// Subdirectory inside `<git-dir>/` where SCE checkout metadata lives.
const SCE_CHECKOUT_DIR: &str = "sce";

/// File name for the checkout ID inside `<git-dir>/sce/`.
const CHECKOUT_ID_FILE: &str = "checkout-id";

/// File name for the identity-creation lock inside `<git-dir>/sce/`.
const CHECKOUT_ID_LOCK_FILE: &str = "checkout-id.lock";

/// Resolves the Git directory (`.git` for normal clones, or the worktree-specific
/// path for linked worktrees) by running `git rev-parse --git-dir` from the
/// given repository root.
///
/// For a normal clone this returns `<repo_root>/.git`.
/// For a linked worktree it returns the worktree-specific Git directory
/// (e.g. `<main-repo>/.git/worktrees/<name>`).
pub fn resolve_git_dir(repo_root: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(repo_root)
        .output()
        .with_context(|| {
            format!(
                "Failed to run git rev-parse --git-dir in '{}'",
                repo_root.display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(anyhow!(
            "git rev-parse --git-dir failed in '{}': {}",
            repo_root.display(),
            stderr
        ));
    }

    let git_dir_relative = String::from_utf8(output.stdout)
        .with_context(|| "git rev-parse --git-dir emitted invalid UTF-8")?
        .trim()
        .to_string();

    // `git rev-parse --git-dir` returns a path relative to the repo root
    // (or an absolute path). Resolve it against the repo root.
    let git_dir = PathBuf::from(&git_dir_relative);
    if git_dir.is_absolute() {
        Ok(git_dir)
    } else {
        Ok(repo_root.join(git_dir))
    }
}

/// Reads an existing checkout ID from `<git_dir>/sce/checkout-id`.
///
/// Returns `Ok(Some(id))` if the file exists and contains a valid checkout ID.
/// Returns `Ok(None)` if the file does not exist.
/// Returns an error if the file exists but cannot be read or contains invalid data.
pub fn read_checkout_id(git_dir: &Path) -> Result<Option<String>> {
    let checkout_id_path = git_dir.join(SCE_CHECKOUT_DIR).join(CHECKOUT_ID_FILE);

    if !checkout_id_path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&checkout_id_path).with_context(|| {
        format!(
            "Failed to read checkout ID from '{}'",
            checkout_id_path.display()
        )
    })?;

    let id = content.trim().to_string();

    if id.is_empty() {
        return Err(anyhow!(
            "Checkout ID file '{}' is empty",
            checkout_id_path.display()
        ));
    }

    // Validate that the stored value is a valid UUIDv7.
    Uuid::parse_str(&id).with_context(|| {
        format!(
            "Invalid checkout ID '{}' in '{}'",
            id,
            checkout_id_path.display()
        )
    })?;

    Ok(Some(id))
}

/// Gets the existing checkout ID or creates a new one.
///
/// If `<git_dir>/sce/checkout-id` already exists, returns the stored ID (idempotent).
/// If it does not exist, acquires `<git_dir>/sce/checkout-id.lock`, generates a new
/// `UUIDv7`, writes it through a temporary file and an atomic rename, and returns it.
pub fn get_or_create_checkout_id(git_dir: &Path) -> Result<String> {
    if let Some(existing_id) = read_checkout_id(git_dir)? {
        return Ok(existing_id);
    }

    let checkout_dir = git_dir.join(SCE_CHECKOUT_DIR);
    std::fs::create_dir_all(&checkout_dir).with_context(|| {
        format!(
            "Failed to create checkout directory '{}'",
            checkout_dir.display()
        )
    })?;

    let lock_path = checkout_dir.join(CHECKOUT_ID_LOCK_FILE);
    let lock_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| {
            format!(
                "Failed to open checkout ID lock file '{}'",
                lock_path.display()
            )
        })?;
    lock_file.lock().with_context(|| {
        format!(
            "Failed to acquire checkout ID lock '{}'",
            lock_path.display()
        )
    })?;

    if let Some(existing_id) = read_checkout_id(git_dir)? {
        return Ok(existing_id);
    }

    let checkout_id = Uuid::now_v7().to_string();
    persist_checkout_id(&checkout_dir, &checkout_id)?;

    Ok(checkout_id)
}

fn persist_checkout_id(checkout_dir: &Path, checkout_id: &str) -> Result<()> {
    persist_checkout_id_inner(checkout_dir, checkout_id, |_, _| Ok(()))
}

fn persist_checkout_id_inner<F>(
    checkout_dir: &Path,
    checkout_id: &str,
    before_rename: F,
) -> Result<()>
where
    F: FnOnce(&Path, &Path) -> Result<()>,
{
    let checkout_id_path = checkout_dir.join(CHECKOUT_ID_FILE);
    let tmp_path = checkout_dir.join(format!("checkout-id.tmp-{checkout_id}"));

    let mut tmp_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)
        .with_context(|| {
            format!(
                "Failed to create temporary checkout ID file '{}'",
                tmp_path.display()
            )
        })?;
    tmp_file
        .write_all(checkout_id.as_bytes())
        .with_context(|| {
            format!(
                "Failed to write checkout ID to temporary file '{}'",
                tmp_path.display()
            )
        })?;
    tmp_file.sync_data().with_context(|| {
        format!(
            "Failed to sync temporary checkout ID file '{}'",
            tmp_path.display()
        )
    })?;
    drop(tmp_file);

    before_rename(&tmp_path, &checkout_id_path)?;

    std::fs::rename(&tmp_path, &checkout_id_path).with_context(|| {
        format!(
            "Failed to rename '{}' to '{}'",
            tmp_path.display(),
            checkout_id_path.display()
        )
    })?;

    #[cfg(unix)]
    {
        if let Ok(dir) = std::fs::File::open(checkout_dir) {
            let _ = dir.sync_all();
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;

    static NEXT_TEST_GIT_DIR_ID: AtomicU64 = AtomicU64::new(0);

    fn unique_test_git_dir(label: &str) -> PathBuf {
        let id = NEXT_TEST_GIT_DIR_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "sce-checkout-identity-{label}-{}-{id}",
            std::process::id()
        ))
    }

    fn remove_test_git_dir(git_dir: &Path) {
        let _ = std::fs::remove_dir_all(git_dir);
    }

    #[test]
    fn concurrent_first_time_callers_converge_on_one_checkout_id() {
        let git_dir = unique_test_git_dir("concurrent-first-time");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let git_dir = git_dir.clone();
                thread::spawn(move || {
                    get_or_create_checkout_id(&git_dir).expect("checkout id should be created")
                })
            })
            .collect();

        let ids: Vec<String> = handles
            .into_iter()
            .map(|handle| handle.join().expect("thread should not panic"))
            .collect();

        let first = ids[0].clone();
        assert!(
            ids.iter().all(|id| *id == first),
            "all concurrent callers should converge on one checkout id, got {ids:?}"
        );

        let persisted =
            read_checkout_id(&git_dir).expect("checkout id should be readable after creation");
        assert_eq!(persisted, Some(first));

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn already_created_checkout_id_is_read_without_acquiring_lock() {
        let git_dir = unique_test_git_dir("fast-path-no-lock");
        let checkout_dir = git_dir.join(SCE_CHECKOUT_DIR);
        std::fs::create_dir_all(&checkout_dir).expect("checkout dir should be created");

        let existing_id = Uuid::now_v7().to_string();
        std::fs::write(checkout_dir.join(CHECKOUT_ID_FILE), &existing_id)
            .expect("seeded checkout id should be written");

        let lock_path = checkout_dir.join(CHECKOUT_ID_LOCK_FILE);
        let lock_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .expect("lock file should be created");
        lock_file.lock().expect("lock should be acquired");

        let (tx, rx) = mpsc::channel();
        let git_dir_clone = git_dir.clone();
        thread::spawn(move || {
            let result = get_or_create_checkout_id(&git_dir_clone);
            let _ = tx.send(result);
        });

        let result = rx
            .recv_timeout(Duration::from_millis(500))
            .expect("fast path should not block on the identity-creation lock");
        assert_eq!(result.expect("existing id should be read"), existing_id);

        drop(lock_file);
        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn completed_rename_leaves_the_canonical_path_with_a_complete_id() {
        let git_dir = unique_test_git_dir("completed-rename");
        let checkout_dir = git_dir.join(SCE_CHECKOUT_DIR);
        std::fs::create_dir_all(&checkout_dir).expect("checkout dir should be created");

        let checkout_id = Uuid::now_v7().to_string();
        persist_checkout_id(&checkout_dir, &checkout_id)
            .expect("persistence should succeed without an injected interruption");

        let persisted = read_checkout_id(&git_dir)
            .expect("checkout id should be readable")
            .expect("checkout id should exist");
        assert_eq!(persisted, checkout_id);
        Uuid::parse_str(&persisted).expect("persisted id should be a complete, valid UUID");

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn interruption_before_rename_leaves_the_canonical_path_absent() {
        let git_dir = unique_test_git_dir("interrupted-before-rename");
        let checkout_dir = git_dir.join(SCE_CHECKOUT_DIR);
        std::fs::create_dir_all(&checkout_dir).expect("checkout dir should be created");

        let checkout_id = Uuid::now_v7().to_string();
        let result =
            persist_checkout_id_inner(&checkout_dir, &checkout_id, |tmp_path, canonical_path| {
                assert!(
                    tmp_path.exists(),
                    "temp file should exist by the time the pre-rename hook runs"
                );
                assert_eq!(
                std::fs::read_to_string(tmp_path).expect("temp file should be readable"),
                checkout_id,
                "temp file should already contain the complete id before the injected interruption"
            );
                assert!(
                    !canonical_path.exists(),
                    "canonical path should still be absent at the pre-rename hook"
                );
                Err(anyhow!("injected interruption before rename"))
            });

        assert!(
            result.is_err(),
            "persistence should surface the injected pre-rename interruption"
        );

        let canonical_path = checkout_dir.join(CHECKOUT_ID_FILE);
        assert!(
            !canonical_path.exists(),
            "canonical checkout-id path must stay absent when interrupted before rename"
        );
        assert_eq!(
            read_checkout_id(&git_dir).expect("read should not error on an absent canonical id"),
            None
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn an_orphaned_temp_file_does_not_block_convergence_on_the_canonical_id() {
        let git_dir = unique_test_git_dir("orphaned-temp-file");
        let checkout_dir = git_dir.join(SCE_CHECKOUT_DIR);
        std::fs::create_dir_all(&checkout_dir).expect("checkout dir should be created");

        let orphaned_tmp_path = checkout_dir.join(format!("checkout-id.tmp-{}", Uuid::now_v7()));
        std::fs::write(&orphaned_tmp_path, b"orphaned-partial-content")
            .expect("orphaned temp file should be written");

        let checkout_id = get_or_create_checkout_id(&git_dir)
            .expect("orphaned temp file should not block id creation");
        Uuid::parse_str(&checkout_id).expect("returned id should be a complete, valid UUID");

        let persisted = read_checkout_id(&git_dir)
            .expect("checkout id should be readable")
            .expect("checkout id should exist");
        assert_eq!(persisted, checkout_id);

        assert!(orphaned_tmp_path.exists());

        remove_test_git_dir(&git_dir);
    }
}
