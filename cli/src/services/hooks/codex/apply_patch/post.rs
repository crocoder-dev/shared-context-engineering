use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::services::config::resolve_agent_trace_storage_runtime_config;
use crate::services::default_paths::codex_apply_patch_pending_dir_for_repository;
use crate::services::repository_identity::resolve::resolve_repository_identity;

use super::super::CodexHookEvent;
use super::persist;
use super::pre::{
    event_key, pending_state_file_path, required_field, snapshot_worktree_tree_oid,
    PendingApplyPatchState,
};

/// The result of finalizing one Codex `apply_patch` tool call against its
/// recorded before-state.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum ApplyPatchFinalizeOutcome {
    /// The before/after snapshots differ; carries the observed unified diff.
    Diff(String),
    /// The before/after snapshots are identical; a successful no-op.
    NoOp,
}

/// Finalizes the `PostToolUse(apply_patch)` before/after attribution: looks
/// up the pending before-state snapshot written by `PreToolUse(apply_patch)`,
/// takes a second temporary-index snapshot for the after-state, and computes
/// the observed diff between the two. Persisting a non-empty diff as
/// `diff_traces`/conversation evidence is a later task's responsibility; this
/// step only computes it.
pub(in crate::services::hooks::codex) fn handle(
    repository_root: &Path,
    event: &CodexHookEvent,
) -> Result<String> {
    let storage_config = resolve_agent_trace_storage_runtime_config(repository_root).context(
        "Failed to resolve Agent Trace repository storage config for Codex apply_patch finalize.",
    )?;
    let repository_identity = resolve_repository_identity(
        repository_root,
        storage_config.repository_id.as_deref(),
        &storage_config.repository_remote,
    )
    .map_err(|error| anyhow::anyhow!("{error}"))
    .context("Failed to resolve repository identity for Codex apply_patch finalize.")?;
    let pending_dir =
        codex_apply_patch_pending_dir_for_repository(&repository_identity.identity.repository_id)
            .context("Failed to resolve Codex apply_patch pending-state directory.")?;

    match finalize_with(repository_root, &pending_dir, event)? {
        ApplyPatchFinalizeOutcome::Diff(patch) => {
            let persist_output = persist::handle(repository_root, event, &patch)?;
            Ok(format!(
                "codex hooks: PostToolUse apply_patch finalized with an observed diff ({} bytes); {persist_output}",
                patch.len()
            ))
        }
        ApplyPatchFinalizeOutcome::NoOp => Ok(String::from(
            "codex hooks: PostToolUse apply_patch finalized with no observed change (no-op).",
        )),
    }
}

/// Injectable counterpart of `handle` for deterministic testing against an
/// explicit pending-state directory (so tests never touch the real user
/// state root).
fn finalize_with(
    repository_root: &Path,
    pending_dir: &Path,
    event: &CodexHookEvent,
) -> Result<ApplyPatchFinalizeOutcome> {
    let session_id = required_field(event.session_id.as_deref(), "session_id")?;
    let turn_id = required_field(event.turn_id.as_deref(), "turn_id")?;
    let tool_use_id = required_field(event.tool_use_id.as_deref(), "tool_use_id")?;

    let key = event_key(session_id, turn_id, tool_use_id);
    let pending_path = pending_state_file_path(pending_dir, &key);

    let raw = match std::fs::read_to_string(&pending_path) {
        Ok(contents) => contents,
        Err(io_error) if io_error.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "No pending Codex apply_patch before-state found for event_key='{key}': PostToolUse apply_patch fired with no matching PreToolUse apply_patch. Failing open with no diff evidence."
            );
        }
        Err(io_error) => {
            return Err(anyhow::Error::new(io_error).context(format!(
                "Failed to read Codex apply_patch pending-state file for event_key='{key}'."
            )));
        }
    };

    let pending_state: PendingApplyPatchState = match serde_json::from_str(&raw) {
        Ok(state) => state,
        Err(error) => {
            // The file exists but is not usable; remove it so a corrupt
            // pending file does not linger indefinitely.
            let _ = std::fs::remove_file(&pending_path);
            bail!(
                "Malformed Codex apply_patch pending-state file for event_key='{key}': {error}. Failing open with no diff evidence."
            );
        }
    };

    let after_tree_oid = snapshot_worktree_tree_oid(
        repository_root,
        pending_dir,
        &format!("{key}.after.index.tmp"),
    )?;

    let diff = run_git_diff(
        repository_root,
        &pending_state.before_tree_oid,
        &after_tree_oid,
    )?;

    let _ = std::fs::remove_file(&pending_path);

    if diff.trim().is_empty() {
        Ok(ApplyPatchFinalizeOutcome::NoOp)
    } else {
        Ok(ApplyPatchFinalizeOutcome::Diff(diff))
    }
}

/// Computes the observed unified diff between the before/after tree
/// snapshots. A tree-to-tree diff needs no index at all, so this runs a
/// plain `git diff`, unlike the scratch-index snapshots above.
fn run_git_diff(
    repository_root: &Path,
    before_tree_oid: &str,
    after_tree_oid: &str,
) -> Result<String> {
    let output = Command::new("git")
        .args([
            "diff",
            "--binary",
            "--find-renames",
            before_tree_oid,
            after_tree_oid,
        ])
        .current_dir(repository_root)
        .output()
        .with_context(|| {
            format!(
                "Failed to compute the Codex apply_patch observed diff (directory: '{}').",
                repository_root.display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let diagnostic = if stderr.is_empty() {
            String::from("git diff exited with a non-zero status")
        } else {
            stderr
        };
        bail!("Failed to compute the Codex apply_patch observed diff: {diagnostic}");
    }

    String::from_utf8(output.stdout).context("git diff output contained invalid UTF-8")
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "sce-codex-apply-patch-post-{label}-{}-{nonce}",
            std::process::id()
        ))
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
        std::fs::create_dir_all(repo_root).expect("create repo root");
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
            hook_event_name: "PostToolUse".to_string(),
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

    fn write_pending_state(pending_dir: &Path, key: &str, before_tree_oid: &str) {
        std::fs::create_dir_all(pending_dir).expect("create pending dir");
        let state = PendingApplyPatchState {
            before_tree_oid: before_tree_oid.to_string(),
            created_at_unix_ms: 1_000,
        };
        std::fs::write(
            pending_state_file_path(pending_dir, key),
            serde_json::to_vec(&state).expect("serialize pending state"),
        )
        .expect("write pending state file");
    }

    fn write_tree_head(repo_root: &Path) -> String {
        String::from_utf8(
            Command::new("git")
                .args(["write-tree"])
                .current_dir(repo_root)
                .output()
                .expect("git write-tree should run")
                .stdout,
        )
        .expect("git write-tree output should be UTF-8")
        .trim()
        .to_string()
    }

    #[test]
    fn finalize_with_produces_a_diff_for_a_created_file() {
        let repo_root = unique_temp_dir("repo");
        init_repo_with_initial_commit(&repo_root);
        let pending_dir = unique_temp_dir("pending");

        let before_tree_oid = write_tree_head(&repo_root);
        let key = event_key("session-1", "turn-1", "tool-1");
        write_pending_state(&pending_dir, &key, &before_tree_oid);

        std::fs::write(repo_root.join("new.txt"), "created\n").expect("create new file");
        git(&repo_root, &["add", "-A"]);

        let outcome = finalize_with(
            &repo_root,
            &pending_dir,
            &event("session-1", "turn-1", "tool-1"),
        )
        .expect("finalize should succeed");

        match outcome {
            ApplyPatchFinalizeOutcome::Diff(diff) => {
                assert!(diff.contains("new.txt"));
                assert!(diff.contains("created"));
            }
            ApplyPatchFinalizeOutcome::NoOp => panic!("expected a non-empty diff"),
        }

        assert!(
            !pending_state_file_path(&pending_dir, &key).exists(),
            "pending state file should be consumed after finalize"
        );

        std::fs::remove_dir_all(&repo_root).ok();
        std::fs::remove_dir_all(&pending_dir).ok();
    }

    #[test]
    fn finalize_with_produces_a_diff_for_an_edited_file() {
        let repo_root = unique_temp_dir("repo");
        init_repo_with_initial_commit(&repo_root);
        let pending_dir = unique_temp_dir("pending");

        let before_tree_oid = write_tree_head(&repo_root);
        let key = event_key("session-2", "turn-1", "tool-1");
        write_pending_state(&pending_dir, &key, &before_tree_oid);

        std::fs::write(repo_root.join("tracked.txt"), "edited\n").expect("edit tracked file");

        let outcome = finalize_with(
            &repo_root,
            &pending_dir,
            &event("session-2", "turn-1", "tool-1"),
        )
        .expect("finalize should succeed");

        match outcome {
            ApplyPatchFinalizeOutcome::Diff(diff) => {
                assert!(diff.contains("tracked.txt"));
                assert!(diff.contains("edited"));
            }
            ApplyPatchFinalizeOutcome::NoOp => panic!("expected a non-empty diff"),
        }

        std::fs::remove_dir_all(&repo_root).ok();
        std::fs::remove_dir_all(&pending_dir).ok();
    }

    #[test]
    fn finalize_with_produces_a_diff_for_a_deleted_file() {
        let repo_root = unique_temp_dir("repo");
        init_repo_with_initial_commit(&repo_root);
        let pending_dir = unique_temp_dir("pending");

        let before_tree_oid = write_tree_head(&repo_root);
        let key = event_key("session-3", "turn-1", "tool-1");
        write_pending_state(&pending_dir, &key, &before_tree_oid);

        std::fs::remove_file(repo_root.join("tracked.txt")).expect("delete tracked file");

        let outcome = finalize_with(
            &repo_root,
            &pending_dir,
            &event("session-3", "turn-1", "tool-1"),
        )
        .expect("finalize should succeed");

        match outcome {
            ApplyPatchFinalizeOutcome::Diff(diff) => {
                assert!(diff.contains("tracked.txt"));
                assert!(diff.contains("deleted file"));
            }
            ApplyPatchFinalizeOutcome::NoOp => panic!("expected a non-empty diff"),
        }

        std::fs::remove_dir_all(&repo_root).ok();
        std::fs::remove_dir_all(&pending_dir).ok();
    }

    #[test]
    fn finalize_with_produces_a_diff_for_a_rename() {
        let repo_root = unique_temp_dir("repo");
        init_repo_with_initial_commit(&repo_root);
        // A rename needs enough content for git's similarity heuristic to
        // detect it rather than reporting a plain delete+add.
        let original_content = "line one\nline two\nline three\nline four\nline five\n";
        std::fs::write(repo_root.join("tracked.txt"), original_content)
            .expect("seed rename-detectable content");
        git(&repo_root, &["add", "-A"]);
        git(&repo_root, &["commit", "-q", "-m", "seed rename content"]);
        let pending_dir = unique_temp_dir("pending");

        let before_tree_oid = write_tree_head(&repo_root);
        let key = event_key("session-4", "turn-1", "tool-1");
        write_pending_state(&pending_dir, &key, &before_tree_oid);

        std::fs::rename(repo_root.join("tracked.txt"), repo_root.join("renamed.txt"))
            .expect("rename tracked file");

        let outcome = finalize_with(
            &repo_root,
            &pending_dir,
            &event("session-4", "turn-1", "tool-1"),
        )
        .expect("finalize should succeed");

        match outcome {
            ApplyPatchFinalizeOutcome::Diff(diff) => {
                assert!(diff.contains("tracked.txt"));
                assert!(diff.contains("renamed.txt"));
                assert!(diff.contains("rename from") || diff.contains("similarity index"));
            }
            ApplyPatchFinalizeOutcome::NoOp => panic!("expected a non-empty diff"),
        }

        std::fs::remove_dir_all(&repo_root).ok();
        std::fs::remove_dir_all(&pending_dir).ok();
    }

    #[test]
    fn finalize_with_treats_identical_before_and_after_as_a_no_op_and_still_consumes_pending_file()
    {
        let repo_root = unique_temp_dir("repo");
        init_repo_with_initial_commit(&repo_root);
        let pending_dir = unique_temp_dir("pending");

        let before_tree_oid = write_tree_head(&repo_root);
        let key = event_key("session-5", "turn-1", "tool-1");
        write_pending_state(&pending_dir, &key, &before_tree_oid);

        // No change made between PreToolUse and PostToolUse.
        let outcome = finalize_with(
            &repo_root,
            &pending_dir,
            &event("session-5", "turn-1", "tool-1"),
        )
        .expect("finalize should succeed");

        assert_eq!(outcome, ApplyPatchFinalizeOutcome::NoOp);
        assert!(
            !pending_state_file_path(&pending_dir, &key).exists(),
            "pending state file should be consumed even for a no-op"
        );

        std::fs::remove_dir_all(&repo_root).ok();
        std::fs::remove_dir_all(&pending_dir).ok();
    }

    #[test]
    fn finalize_with_excludes_a_pre_existing_dirty_change_from_the_observed_diff() {
        let repo_root = unique_temp_dir("repo");
        init_repo_with_initial_commit(&repo_root);
        // Change A: a pre-existing dirty change present before PreToolUse.
        std::fs::write(repo_root.join("tracked.txt"), "dirty-before-tool-call\n")
            .expect("seed pre-existing dirty change");
        let pending_dir = unique_temp_dir("pending");

        // Mirrors PreToolUse's own before-state snapshot mechanism so the
        // before_tree_oid reflects the dirty worktree, not HEAD.
        let before_tree_oid = snapshot_worktree_tree_oid(
            &repo_root,
            &pending_dir,
            "session-6-turn-1-tool-1.index.tmp",
        )
        .expect("before-state snapshot should succeed");
        let key = event_key("session-6", "turn-1", "tool-1");
        write_pending_state(&pending_dir, &key, &before_tree_oid);

        // Change B: the tool call's own edit, made after the before-state
        // snapshot was taken.
        std::fs::write(repo_root.join("new-from-tool.txt"), "change-b\n")
            .expect("write tool-call change");

        let outcome = finalize_with(
            &repo_root,
            &pending_dir,
            &event("session-6", "turn-1", "tool-1"),
        )
        .expect("finalize should succeed");

        match outcome {
            ApplyPatchFinalizeOutcome::Diff(diff) => {
                assert!(
                    diff.contains("new-from-tool.txt"),
                    "diff should contain change B"
                );
                assert!(
                    !diff.contains("dirty-before-tool-call"),
                    "diff should exclude change A, which predates PreToolUse"
                );
            }
            ApplyPatchFinalizeOutcome::NoOp => panic!("expected a non-empty diff"),
        }

        std::fs::remove_dir_all(&repo_root).ok();
        std::fs::remove_dir_all(&pending_dir).ok();
    }

    #[test]
    fn finalize_with_fails_open_when_no_pending_file_exists() {
        let repo_root = unique_temp_dir("repo");
        init_repo_with_initial_commit(&repo_root);
        let pending_dir = unique_temp_dir("pending");

        let error = finalize_with(
            &repo_root,
            &pending_dir,
            &event("session-7", "turn-1", "tool-1"),
        )
        .expect_err("finalize should fail open with no matching pending file");

        assert!(error.to_string().contains("No pending Codex apply_patch"));

        std::fs::remove_dir_all(&repo_root).ok();
        std::fs::remove_dir_all(&pending_dir).ok();
    }

    #[test]
    fn finalize_with_fails_open_and_removes_a_malformed_pending_file() {
        let repo_root = unique_temp_dir("repo");
        init_repo_with_initial_commit(&repo_root);
        let pending_dir = unique_temp_dir("pending");
        std::fs::create_dir_all(&pending_dir).expect("create pending dir");

        let key = event_key("session-8", "turn-1", "tool-1");
        std::fs::write(pending_state_file_path(&pending_dir, &key), b"not json")
            .expect("write malformed pending state file");

        let error = finalize_with(
            &repo_root,
            &pending_dir,
            &event("session-8", "turn-1", "tool-1"),
        )
        .expect_err("finalize should fail open on a malformed pending file");

        assert!(error.to_string().contains("Malformed Codex apply_patch"));
        assert!(
            !pending_state_file_path(&pending_dir, &key).exists(),
            "the malformed pending file should be removed rather than left behind"
        );

        std::fs::remove_dir_all(&repo_root).ok();
        std::fs::remove_dir_all(&pending_dir).ok();
    }

    #[test]
    fn finalize_with_rejects_a_missing_tool_use_id() {
        let repo_root = unique_temp_dir("repo");
        init_repo_with_initial_commit(&repo_root);
        let pending_dir = unique_temp_dir("pending");

        let mut missing_tool_use_id = event("session-9", "turn-1", "tool-1");
        missing_tool_use_id.tool_use_id = None;

        finalize_with(&repo_root, &pending_dir, &missing_tool_use_id)
            .expect_err("missing tool_use_id should error");

        std::fs::remove_dir_all(&repo_root).ok();
        std::fs::remove_dir_all(&pending_dir).ok();
    }
}
