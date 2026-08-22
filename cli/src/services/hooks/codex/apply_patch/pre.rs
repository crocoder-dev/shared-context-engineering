use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::services::config::resolve_agent_trace_storage_runtime_config;
use crate::services::default_paths::codex_apply_patch_pending_dir_for_repository;
use crate::services::repository_identity::resolve::resolve_repository_identity;

use super::super::super::current_unix_time_ms;
use super::super::CodexHookEvent;

/// The transient before-state snapshot recorded for one pending
/// `apply_patch` tool call, keyed by [`event_key`] on disk.
///
/// `super::post` reads this back to compute the observed after-state diff.
#[derive(Debug, Deserialize, Serialize)]
pub(super) struct PendingApplyPatchState {
    pub(super) before_tree_oid: String,
    pub(super) created_at_unix_ms: i64,
}

/// Captures the `PreToolUse(apply_patch)` before-state: a `git write-tree`
/// snapshot of the current worktree (tracked changes plus non-ignored
/// untracked files, not just `HEAD`) taken against a temporary Git index, so
/// the real index is never touched. The resulting `before_tree_oid` is
/// persisted to a pending-state file keyed by a hash of
/// `(session_id, turn_id, tool_use_id)`, for a later `PostToolUse` finalize
/// step to consume.
pub(in crate::services::hooks::codex) fn handle(
    repository_root: &Path,
    event: &CodexHookEvent,
) -> Result<String> {
    let storage_config = resolve_agent_trace_storage_runtime_config(repository_root)
        .context("Failed to resolve Agent Trace repository storage config for Codex apply_patch pending state.")?;
    let repository_identity = resolve_repository_identity(
        repository_root,
        storage_config.repository_id.as_deref(),
        &storage_config.repository_remote,
    )
    .map_err(|error| anyhow::anyhow!("{error}"))
    .context("Failed to resolve repository identity for Codex apply_patch pending state.")?;
    let pending_dir =
        codex_apply_patch_pending_dir_for_repository(&repository_identity.identity.repository_id)
            .context("Failed to resolve Codex apply_patch pending-state directory.")?;

    capture_with(repository_root, &pending_dir, event, || {
        current_unix_time_ms().unwrap_or(0)
    })
}

/// Injectable counterpart of `handle` for deterministic testing against an
/// explicit pending-state directory (so tests never touch the real user
/// state root).
fn capture_with<T>(
    repository_root: &Path,
    pending_dir: &Path,
    event: &CodexHookEvent,
    generate_timestamp_ms: T,
) -> Result<String>
where
    T: FnOnce() -> i64,
{
    let session_id = required_field(event.session_id.as_deref(), "session_id")?;
    let turn_id = required_field(event.turn_id.as_deref(), "turn_id")?;
    let tool_use_id = required_field(event.tool_use_id.as_deref(), "tool_use_id")?;

    let key = event_key(session_id, turn_id, tool_use_id);
    let before_tree_oid =
        snapshot_worktree_tree_oid(repository_root, pending_dir, &format!("{key}.index.tmp"))?;

    let pending_state = PendingApplyPatchState {
        before_tree_oid,
        created_at_unix_ms: generate_timestamp_ms(),
    };
    write_pending_state_atomic(pending_dir, &key, &pending_state)?;

    Ok(format!(
        "codex hooks: PreToolUse apply_patch before-state snapshot captured (event_key='{key}')."
    ))
}

pub(super) fn required_field<'a>(value: Option<&'a str>, field_name: &str) -> Result<&'a str> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(anyhow::anyhow!(
            "Invalid Codex PreToolUse apply_patch payload: field '{field_name}' must be a non-empty string."
        )),
    }
}

/// Deterministic, filesystem-safe key for one `apply_patch` tool call. Each
/// field is hashed as its own delimited byte segment (not string-concatenated)
/// so that ambiguous field boundaries cannot collide two distinct triples.
pub(super) fn event_key(session_id: &str, turn_id: &str, tool_use_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(session_id.as_bytes());
    hasher.update([0u8]);
    hasher.update(turn_id.as_bytes());
    hasher.update([0u8]);
    hasher.update(tool_use_id.as_bytes());
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Snapshots the current worktree state (tracked changes plus non-ignored
/// untracked files) into a tree object, using a scratch `GIT_INDEX_FILE` so
/// the repository's real index is never read or written. Shared by both the
/// `PreToolUse` before-state snapshot and the `PostToolUse` after-state
/// snapshot; `temp_index_file_name` must be unique per call site to avoid the
/// two snapshots colliding on the same scratch index file.
pub(super) fn snapshot_worktree_tree_oid(
    repository_root: &Path,
    pending_dir: &Path,
    temp_index_file_name: &str,
) -> Result<String> {
    std::fs::create_dir_all(pending_dir).with_context(|| {
        format!(
            "Failed to create Codex apply_patch pending-state directory '{}'.",
            pending_dir.display()
        )
    })?;

    let temp_index_file = pending_dir.join(temp_index_file_name);
    let outcome = (|| -> Result<String> {
        run_git_with_index(
            repository_root,
            &temp_index_file,
            &["read-tree", "HEAD"],
            "Failed to seed the temporary Codex apply_patch index from HEAD.",
        )?;
        run_git_with_index(
            repository_root,
            &temp_index_file,
            &["add", "-A"],
            "Failed to stage the current worktree state into the temporary Codex apply_patch index.",
        )?;
        run_git_with_index(
            repository_root,
            &temp_index_file,
            &["write-tree"],
            "Failed to write the Codex apply_patch before-state tree.",
        )
    })();

    let _ = std::fs::remove_file(&temp_index_file);

    outcome
}

fn run_git_with_index(
    repository_root: &Path,
    index_file: &Path,
    args: &[&str],
    context_message: &str,
) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository_root)
        .env("GIT_INDEX_FILE", index_file)
        .output()
        .with_context(|| {
            format!(
                "{context_message} (directory: '{}')",
                repository_root.display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let diagnostic = if stderr.is_empty() {
            String::from("git command exited with a non-zero status")
        } else {
            stderr
        };
        bail!("{context_message} {diagnostic}");
    }

    String::from_utf8(output.stdout)
        .context("git command output contained invalid UTF-8")
        .map(|value| value.trim().to_string())
}

fn write_pending_state_atomic(
    pending_dir: &Path,
    key: &str,
    state: &PendingApplyPatchState,
) -> Result<()> {
    std::fs::create_dir_all(pending_dir).with_context(|| {
        format!(
            "Failed to create Codex apply_patch pending-state directory '{}'.",
            pending_dir.display()
        )
    })?;

    let final_path = pending_state_file_path(pending_dir, key);
    let temp_path = pending_dir.join(format!("{key}.json.tmp-{}", std::process::id()));

    let payload = serde_json::to_vec(state)
        .context("Failed to serialize Codex apply_patch pending state.")?;
    std::fs::write(&temp_path, &payload).with_context(|| {
        format!(
            "Failed to write Codex apply_patch pending-state temp file '{}'.",
            temp_path.display()
        )
    })?;
    std::fs::rename(&temp_path, &final_path).with_context(|| {
        format!(
            "Failed to atomically finalize Codex apply_patch pending-state file '{}'.",
            final_path.display()
        )
    })?;

    Ok(())
}

pub(super) fn pending_state_file_path(pending_dir: &Path, key: &str) -> PathBuf {
    pending_dir.join(format!("{key}.json"))
}

#[cfg(test)]
mod tests {
    use std::{
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sce-codex-apply-patch-pre-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn git(repo_root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .output()
            .unwrap_or_else(|error| panic!("git {args:?} failed to spawn: {error}"));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_repo_with_initial_commit(repo_root: &Path) {
        git(repo_root, &["init", "-q"]);
        git(
            repo_root,
            &["config", "user.email", "codex-test@example.invalid"],
        );
        git(repo_root, &["config", "user.name", "Codex Test"]);
        std::fs::write(repo_root.join("tracked.txt"), "original\n").expect("write tracked file");
        git(repo_root, &["add", "tracked.txt"]);
        git(repo_root, &["commit", "-q", "-m", "initial commit"]);
    }

    fn event(session_id: &str, turn_id: &str, tool_use_id: &str) -> CodexHookEvent {
        CodexHookEvent {
            hook_event_name: "PreToolUse".to_string(),
            session_id: Some(session_id.to_string()),
            turn_id: Some(turn_id.to_string()),
            cwd: None,
            model: None,
            tool_name: Some("apply_patch".to_string()),
            tool_use_id: Some(tool_use_id.to_string()),
            tool_input: None,
            tool_response: None,
            prompt: None,
            last_assistant_message: None,
        }
    }

    #[test]
    fn event_key_is_deterministic_for_the_same_triple() {
        assert_eq!(
            event_key("session-1", "turn-1", "tool-1"),
            event_key("session-1", "turn-1", "tool-1")
        );
    }

    #[test]
    fn event_key_is_distinct_for_different_triples() {
        let base = event_key("session-1", "turn-1", "tool-1");
        assert_ne!(base, event_key("session-2", "turn-1", "tool-1"));
        assert_ne!(base, event_key("session-1", "turn-2", "tool-1"));
        assert_ne!(base, event_key("session-1", "turn-1", "tool-2"));
        // Field-boundary ambiguity: concatenation without a delimiter would
        // collide these two distinct triples.
        assert_ne!(event_key("ab", "c", "d"), event_key("a", "bc", "d"),);
    }

    #[test]
    fn event_key_is_safe_as_a_filesystem_path_segment() {
        let key = event_key("session/../etc", "turn\\1", "tool 1");
        assert!(key.chars().all(|character| character.is_ascii_hexdigit()));
        assert!(!key.contains(['/', '\\']));
    }

    #[test]
    fn capture_with_writes_a_pending_state_file_reflecting_the_dirty_worktree() {
        let repo_root = unique_temp_dir("repo");
        init_repo_with_initial_commit(&repo_root);
        // A pre-existing dirty (uncommitted) change before PreToolUse fires.
        std::fs::write(repo_root.join("tracked.txt"), "dirty\n").expect("dirty tracked file");
        std::fs::write(repo_root.join("untracked.txt"), "new\n").expect("untracked file");

        let pending_dir = unique_temp_dir("pending");

        let output = capture_with(
            &repo_root,
            &pending_dir,
            &event("session-1", "turn-1", "tool-1"),
            || 1_000,
        )
        .expect("capture should succeed");
        assert!(output.contains("before-state snapshot captured"));

        let key = event_key("session-1", "turn-1", "tool-1");
        let raw = std::fs::read_to_string(pending_state_file_path(&pending_dir, &key))
            .expect("pending state file should exist");
        let state: PendingApplyPatchState =
            serde_json::from_str(&raw).expect("pending state file should be valid JSON");

        assert_eq!(state.created_at_unix_ms, 1_000);

        // The snapshot must reflect the dirty worktree (tracked edit +
        // untracked file), not HEAD: `git write-tree` against a real index
        // seeded the same way (`read-tree HEAD` + `add -A`) must match.
        git(&repo_root, &["add", "-A"]);
        let expected_tree_oid = String::from_utf8(
            Command::new("git")
                .args(["write-tree"])
                .current_dir(&repo_root)
                .output()
                .expect("git write-tree should run")
                .stdout,
        )
        .expect("git write-tree output should be UTF-8")
        .trim()
        .to_string();

        assert_eq!(state.before_tree_oid, expected_tree_oid);

        std::fs::remove_dir_all(&repo_root).ok();
        std::fs::remove_dir_all(&pending_dir).ok();
    }

    #[test]
    fn capture_with_leaves_the_real_index_untouched() {
        let repo_root = unique_temp_dir("repo");
        init_repo_with_initial_commit(&repo_root);
        std::fs::write(repo_root.join("tracked.txt"), "dirty\n").expect("dirty tracked file");

        let pending_dir = unique_temp_dir("pending");

        capture_with(
            &repo_root,
            &pending_dir,
            &event("session-1", "turn-1", "tool-1"),
            || 1_000,
        )
        .expect("capture should succeed");

        let status = String::from_utf8(
            Command::new("git")
                .args(["status", "--porcelain"])
                .current_dir(&repo_root)
                .output()
                .expect("git status should run")
                .stdout,
        )
        .expect("git status output should be UTF-8");
        assert_eq!(
            status.trim_end(),
            " M tracked.txt",
            "the real index/worktree status must be unaffected by the snapshot"
        );

        std::fs::remove_dir_all(&repo_root).ok();
        std::fs::remove_dir_all(&pending_dir).ok();
    }

    #[test]
    fn capture_with_rejects_a_missing_tool_use_id() {
        let repo_root = unique_temp_dir("repo");
        init_repo_with_initial_commit(&repo_root);
        let pending_dir = unique_temp_dir("pending");

        let mut payload = event("session-1", "turn-1", "tool-1");
        payload.tool_use_id = None;

        let error = capture_with(&repo_root, &pending_dir, &payload, || 1_000)
            .expect_err("missing tool_use_id should error");
        assert!(error.to_string().contains("'tool_use_id'"));

        std::fs::remove_dir_all(&repo_root).ok();
        std::fs::remove_dir_all(&pending_dir).ok();
    }
}
