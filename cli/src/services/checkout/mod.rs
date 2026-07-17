//! Checkout identity service.
//!
//! Each cloned repository (and linked Git worktree) gets its own stable checkout
//! identity stored in `<git-dir>/sce/checkout-id`. The checkout ID is a `UUIDv7`
//! string, consistent with the existing `agent_trace_id` convention in this
//! codebase.
//!
//! Checkout databases are discovered via filesystem scan in `sce trace db list`
//! (see `cli/src/services/trace/`). There is no central registry file.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use uuid::Uuid;

/// Subdirectory inside `<git-dir>/` where SCE checkout metadata lives.
const SCE_CHECKOUT_DIR: &str = "sce";

/// File name for the checkout ID inside `<git-dir>/sce/`.
const CHECKOUT_ID_FILE: &str = "checkout-id";

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
/// If it does not exist, generates a new `UUIDv7`, writes it to the file, and returns it.
pub fn get_or_create_checkout_id(git_dir: &Path) -> Result<String> {
    if let Some(existing_id) = read_checkout_id(git_dir)? {
        return Ok(existing_id);
    }

    let checkout_id = Uuid::now_v7().to_string();

    let checkout_dir = git_dir.join(SCE_CHECKOUT_DIR);
    std::fs::create_dir_all(&checkout_dir).with_context(|| {
        format!(
            "Failed to create checkout directory '{}'",
            checkout_dir.display()
        )
    })?;

    let checkout_id_path = checkout_dir.join(CHECKOUT_ID_FILE);
    std::fs::write(&checkout_id_path, &checkout_id).with_context(|| {
        format!(
            "Failed to write checkout ID to '{}'",
            checkout_id_path.display()
        )
    })?;

    Ok(checkout_id)
}
