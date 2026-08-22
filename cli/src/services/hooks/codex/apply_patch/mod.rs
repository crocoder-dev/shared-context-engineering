//! Parses Codex's custom `apply_patch` text format (`*** Begin Patch` ...
//! `*** End Patch`) into a typed [`CodexPatch`], normalizes it into an
//! SCE-supported unified diff, and persists non-empty results as a
//! `diff_traces` row for the `PostToolUse`/`apply_patch` dispatch arm.

mod normalize;
mod parser;
mod path;

use std::path::Path;

use anyhow::{Context, Result};

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_db::{DiffTraceInsert, PAYLOAD_TYPE_PATCH};
use crate::services::agent_trace_storage::{
    resolve_agent_trace_storage_for_hook_runtime_at_state_root, AgentTraceStorageContext,
};
use crate::services::observability::traits::Logger;

use normalize::normalize_codex_patch;
#[allow(unused_imports)]
use parser::{
    normalize_outer_apply_patch_input, parse_codex_apply_patch, CodexFileOperation, CodexHunk,
    CodexHunkLine, CodexPatch, CodexPatchParseError,
};
use path::resolve_codex_patch_paths;

use super::super::{
    current_unix_time_ms, normalize_codex_model_id, open_agent_trace_db_for_hook_runtime,
    prefixed_diff_trace_session_id, CODEX_TOOL_NAME,
};
use super::CodexHookEvent;

/// Handles a Codex `PostToolUse(apply_patch)` event: reads the raw patch text
/// from `tool_input.command`, parses it (T10), normalizes it (T11), and — for
/// a non-empty normalized result — persists one `diff_traces` row.
///
/// Every path here, success or fail-open, returns empty stdout: a missing or
/// non-string `command`, a parse failure (logged), and an empty normalized
/// patch (e.g. delete-only) all resolve to `Ok(String::new())` with no
/// evidence written.
pub(super) fn handle(
    repository_root: &Path,
    event: &CodexHookEvent,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    handle_with_state_root(repository_root, event, None, logger)
}

pub(super) fn handle_with_state_root(
    repository_root: &Path,
    event: &CodexHookEvent,
    state_root: Option<&Path>,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    // Validate the session before parsing, path resolution, or DB access so
    // invalid Codex events can never reach apply_patch persistence.
    required_session_id(event.session_id.as_deref())?;

    let Some(command) = apply_patch_command_from_event(event) else {
        return Ok(String::new());
    };

    let canonical_command = match normalize_outer_apply_patch_input(command) {
        Ok(command) => command,
        Err(parse_error) => {
            if let Some(log) = logger {
                log.error(
                    "sce.hooks.codex.apply_patch.parse_failed",
                    &parse_error.to_string(),
                    &[],
                    event.session_id.as_deref(),
                );
            }
            return Ok(String::new());
        }
    };

    let patch = match parse_codex_apply_patch(&canonical_command) {
        Ok(patch) => patch,
        Err(parse_error) => {
            if let Some(log) = logger {
                log.error(
                    "sce.hooks.codex.apply_patch.parse_failed",
                    &parse_error.to_string(),
                    &[],
                    event.session_id.as_deref(),
                );
            }
            return Ok(String::new());
        }
    };

    let mut patch = patch;
    if let Some(event_cwd) = event.cwd.as_deref() {
        if let Err(error) = resolve_codex_patch_paths(repository_root, event_cwd, &mut patch) {
            if let Some(log) = logger {
                log.error(
                    "sce.hooks.codex.apply_patch.path_resolution_failed",
                    &error.to_string(),
                    &[],
                    event.session_id.as_deref(),
                );
            }
            return Ok(String::new());
        }
    } else {
        if let Some(log) = logger {
            log.error(
                "sce.hooks.codex.apply_patch.path_resolution_failed",
                "Codex hook event cwd is missing or malformed.",
                &[],
                event.session_id.as_deref(),
            );
        }
        return Ok(String::new());
    }

    let normalized_patch =
        match normalize_codex_patch(&patch, event.tool_use_id.as_deref().unwrap_or_default()) {
            Ok(normalized_patch) => normalized_patch,
            Err(error) => {
                if let Some(log) = logger {
                    log.error(
                        "sce.hooks.codex.apply_patch.normalize_failed",
                        &error.to_string(),
                        &[],
                        event.session_id.as_deref(),
                    );
                }
                return Ok(String::new());
            }
        };
    if normalized_patch.is_empty() {
        return Ok(String::new());
    }

    let Ok(time_ms) = current_unix_time_ms() else {
        return Ok(String::new());
    };

    let db = match state_root {
        Some(state_root) => resolve_agent_trace_storage_for_hook_runtime_at_state_root(
            &AgentTraceStorageContext {
                repository_root,
                explicit_repository_id: None,
                repository_remote: "origin",
            },
            state_root,
        )
        .map(|storage| storage.db)
        .context("Failed to open Agent Trace DB for Codex apply_patch persistence."),
        None => open_agent_trace_db_for_hook_runtime(
            repository_root,
            "Failed to open Agent Trace DB for Codex apply_patch persistence.",
        ),
    }?;

    persist_with(&db, event, &normalized_patch, time_ms)
}

/// Codex's `PostToolUse` `tool_input` for the `apply_patch` tool carries the
/// raw patch text under `command`, mirroring the `Bash` tool's `tool_input`
/// shape this module's sibling `bash_policy.rs` already relies on.
fn apply_patch_command_from_event(event: &CodexHookEvent) -> Option<&str> {
    event
        .tool_input
        .as_ref()
        .and_then(|value| value.get("command"))
        .and_then(|value| value.as_str())
}

fn required_session_id(value: Option<&str>) -> Result<&str> {
    match value.map(str::trim) {
        Some(value) if !value.is_empty() => Ok(value),
        _ => anyhow::bail!(
            "Invalid Codex apply_patch payload: field 'session_id' must be a trimmed, non-empty string."
        ),
    }
}

/// Injectable counterpart of `handle`'s persistence step, for deterministic
/// testing against an already-open Agent Trace DB — mirrors the
/// `user_prompt_submit`/`stop` sibling arms' `capture_with` pattern.
fn persist_with(
    db: &RepositoryAgentTraceDb,
    event: &CodexHookEvent,
    normalized_patch: &str,
    time_ms: i64,
) -> Result<String> {
    let session_id = prefixed_diff_trace_session_id(
        CODEX_TOOL_NAME,
        required_session_id(event.session_id.as_deref())?,
    );
    let model_id = event.model.as_deref().and_then(normalize_codex_model_id);

    db.insert_diff_trace(DiffTraceInsert {
        time_ms,
        session_id: &session_id,
        patch: normalized_patch,
        model_id: model_id.as_deref(),
        tool_name: CODEX_TOOL_NAME,
        tool_version: None,
        payload_type: PAYLOAD_TYPE_PATCH,
    })
    .context("Failed to persist Codex apply_patch diff-trace row.")?;

    Ok(String::new())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde_json::json;

    use crate::services::agent_trace::{build_agent_trace, AgentTraceMetadataInput};
    use crate::services::patch::{
        FileChangeKind, ParsedPatch, PatchFileChange, PatchHunk, TouchedLine, TouchedLineKind,
    };

    use super::*;

    const ADD_AND_UPDATE_PATCH: &str = "*** Begin Patch\n\
*** Add File: new_file.txt\n\
+hello world\n\
*** Update File: src/lib.rs\n\
@@\n\
-old_line\n\
+new_line\n\
*** End Patch";

    const MOVE_WITH_EDITS_PATCH: &str = "*** Begin Patch\n\
*** Update File: old_name.txt\n\
*** Move to: new_name.txt\n\
@@\n\
-old\n\
+new\n\
*** End Patch";

    const PURE_RENAME_PATCH: &str = "*** Begin Patch\n\
*** Update File: old_name.txt\n\
*** Move to: new_name.txt\n\
*** End Patch";

    const DELETE_ONLY_PATCH: &str = "*** Begin Patch\n\
*** Delete File: obsolete.txt\n\
*** End Patch";

    const MIXED_PATCH: &str = "*** Begin Patch\n\
*** Add File: a.txt\n\
+hello\n\
*** Delete File: b.txt\n\
*** Update File: c.txt\n\
@@\n\
-old\n\
+new\n\
*** End Patch";

    const MALFORMED_PATCH: &str = "not a real apply_patch payload";

    const UPDATE_ONLY_PATCH: &str = "*** Begin Patch\n\
*** Update File: src/lib.rs\n\
@@\n\
-old_line\n\
+new_line\n\
*** End Patch";

    fn event(session_id: &str, model: Option<&str>, command: &str) -> CodexHookEvent {
        event_with_tool_input(session_id, model, Some(json!({ "command": command })))
    }

    fn event_with_tool_input(
        session_id: &str,
        model: Option<&str>,
        tool_input: Option<serde_json::Value>,
    ) -> CodexHookEvent {
        CodexHookEvent {
            hook_event_name: "PostToolUse".to_string(),
            session_id: Some(session_id.to_string()),
            turn_id: Some("turn-1".to_string()),
            cwd: None,
            model: model.map(str::to_string),
            tool_name: Some("apply_patch".to_string()),
            tool_use_id: Some("tool-1".to_string()),
            tool_input,
            tool_response: None,
            prompt: None,
            last_assistant_message: None,
        }
    }

    fn normalized(raw: &str) -> String {
        normalize_codex_patch(
            &parse_codex_apply_patch(raw).expect("fixture patch should parse"),
            "tool-1",
        )
        .expect("fixture tool identity should normalize")
    }

    fn unique_test_db_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "sce-codex-apply-patch-{label}-{}-{nonce}",
                std::process::id()
            ))
            .join("agent-trace.db")
    }

    fn remove_test_db(db_path: &Path) {
        if let Some(parent) = db_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    // --- fail-open / successful no-op behaviors: `handle` returns before it
    // would ever open the Agent Trace DB, so a non-existent repository root
    // is safe to pass through unused. ---

    #[test]
    fn handle_fails_open_silently_when_tool_input_missing() {
        let output = handle(
            Path::new("/nonexistent"),
            &event_with_tool_input("session-1", None, None),
            None,
        )
        .expect("apply_patch handling should succeed");
        assert_eq!(output, "");
    }

    #[test]
    fn handle_fails_open_silently_when_command_missing() {
        let output = handle(
            Path::new("/nonexistent"),
            &event_with_tool_input("session-1", None, Some(json!({}))),
            None,
        )
        .expect("apply_patch handling should succeed");
        assert_eq!(output, "");
    }

    #[test]
    fn handle_fails_open_silently_when_command_non_string() {
        let output = handle(
            Path::new("/nonexistent"),
            &event_with_tool_input("session-1", None, Some(json!({"command": 42}))),
            None,
        )
        .expect("apply_patch handling should succeed");
        assert_eq!(output, "");
    }

    #[test]
    fn handle_fails_open_silently_on_malformed_patch_text() {
        let output = handle(
            Path::new("/nonexistent"),
            &event("session-1", None, MALFORMED_PATCH),
            None,
        )
        .expect("apply_patch handling should succeed");
        assert_eq!(output, "");
    }

    #[test]
    fn handle_is_a_successful_no_op_for_a_delete_only_patch() {
        let output = handle(
            Path::new("/nonexistent"),
            &event("session-1", None, DELETE_ONLY_PATCH),
            None,
        )
        .expect("apply_patch handling should succeed");
        assert_eq!(output, "");
    }

    #[test]
    fn persist_with_rejects_missing_empty_and_whitespace_session_ids_without_rows() {
        let db_path = unique_test_db_path("invalid-session");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let normalized_patch = normalized(UPDATE_ONLY_PATCH);

        for session_id in [None, Some(""), Some("   ")] {
            let mut invalid_event = event("session-1", None, UPDATE_ONLY_PATCH);
            invalid_event.session_id = session_id.map(str::to_string);
            assert!(persist_with(&db, &invalid_event, &normalized_patch, 1_000).is_err());
        }

        let recent = db
            .recent_diff_trace_patches(0, i64::MAX)
            .expect("diff trace query should succeed");
        assert_eq!(recent.loaded_count(), 0);

        remove_test_db(&db_path);
    }

    #[test]
    fn persist_with_trims_valid_session_ids_before_prefixing() {
        let db_path = unique_test_db_path("trimmed-session");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let normalized_patch = normalized(UPDATE_ONLY_PATCH);
        let mut trimmed_event = event(" session-1 ", None, UPDATE_ONLY_PATCH);

        persist_with(&db, &trimmed_event, &normalized_patch, 1_000)
            .expect("trimmed session should persist");
        trimmed_event.session_id = Some("cx_session-2".to_string());
        persist_with(&db, &trimmed_event, &normalized_patch, 2_000)
            .expect("already prefixed session should persist");

        let rows = db
            .query_map(
                "SELECT session_id FROM diff_traces ORDER BY id ASC",
                (),
                |row| row.get::<String>(0).map_err(anyhow::Error::from),
            )
            .expect("diff trace query should succeed");
        assert_eq!(rows, vec!["cx_session-1", "cx_session-2"]);

        remove_test_db(&db_path);
    }

    #[test]
    fn handle_is_a_successful_no_op_for_a_pure_rename_with_no_changed_lines() {
        let output = handle(
            Path::new("/nonexistent"),
            &event("session-1", None, PURE_RENAME_PATCH),
            None,
        )
        .expect("apply_patch handling should succeed");
        assert_eq!(output, "");
    }

    // --- persistence content (AC11-AC14): `persist_with` against a real,
    // directly-opened Agent Trace DB, mirroring `user_prompt_submit`/`stop`'s
    // own injectable-level testing precedent rather than the full
    // hook-runtime DB resolution (which requires a prior `sce setup`). ---

    #[test]
    fn apply_patch_persists_one_row_with_expected_field_values_for_add_and_update() {
        let db_path = unique_test_db_path("add-update");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        let normalized_patch = normalized(ADD_AND_UPDATE_PATCH);
        let output = persist_with(
            &db,
            &event("session-1", Some("gpt-5-codex"), ADD_AND_UPDATE_PATCH),
            &normalized_patch,
            1_000,
        )
        .expect("persist_with should succeed");
        assert_eq!(output, "");

        let recent = db
            .recent_diff_trace_patches(0, i64::MAX)
            .expect("diff trace query should succeed");
        assert_eq!(recent.loaded_count(), 1);
        assert_eq!(recent.skipped_count(), 0);

        let row = &recent.patches[0];
        assert_eq!(row.session_id, "cx_session-1");
        assert_eq!(row.tool_name.as_deref(), Some("codex"));
        assert_eq!(row.tool_version, None);
        assert_eq!(row.payload_type, "patch");
        assert_eq!(
            row.patch.files.len(),
            2,
            "Add File and Update File both persist evidence"
        );
        assert!(row
            .patch
            .files
            .iter()
            .flat_map(|file| &file.hunks)
            .all(|hunk| hunk.model_id.as_deref() == Some("gpt-5-codex")));

        remove_test_db(&db_path);
    }

    #[test]
    fn apply_patch_persists_truthful_model_ids_without_fabricating_openai() {
        let db_path = unique_test_db_path("model-provenance");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let normalized_patch = normalized(UPDATE_ONLY_PATCH);
        let cases = [
            (Some("openai/gpt-x"), Some("openai/gpt-x")),
            (
                Some("qualified/custom-provider/model"),
                Some("qualified/custom-provider/model"),
            ),
            (Some("custom-model"), Some("custom-model")),
            (Some("   "), None),
            (None, None),
        ];

        for (index, (model, _expected)) in cases.iter().enumerate() {
            persist_with(
                &db,
                &event(&format!("session-{index}"), *model, UPDATE_ONLY_PATCH),
                &normalized_patch,
                i64::try_from(index).expect("test index should fit") + 1_000,
            )
            .expect("model provenance should persist");
        }

        let recent = db
            .recent_diff_trace_patches(0, i64::MAX)
            .expect("diff trace query should succeed");
        let model_ids: Vec<Option<String>> = recent
            .patches
            .iter()
            .map(|row| row.patch.files[0].hunks[0].model_id.clone())
            .collect();
        let expected: Vec<Option<String>> = cases
            .iter()
            .map(|(_, expected)| expected.map(str::to_string))
            .collect();
        assert_eq!(model_ids, expected);

        remove_test_db(&db_path);
    }

    #[test]
    fn apply_patch_move_with_edits_persists_row_with_expected_paths() {
        let db_path = unique_test_db_path("move-edits");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        let normalized_patch = normalized(MOVE_WITH_EDITS_PATCH);
        persist_with(
            &db,
            &event("session-1", None, MOVE_WITH_EDITS_PATCH),
            &normalized_patch,
            1_000,
        )
        .expect("persist_with should succeed");

        let recent = db
            .recent_diff_trace_patches(0, i64::MAX)
            .expect("diff trace query should succeed");
        assert_eq!(recent.loaded_count(), 1);
        let file = &recent.patches[0].patch.files[0];
        assert_eq!(file.old_path, "old_name.txt");
        assert_eq!(file.new_path, "new_name.txt");
        assert_eq!(file.kind, FileChangeKind::Renamed);

        remove_test_db(&db_path);
    }

    #[test]
    fn apply_patch_mixed_operations_persists_only_add_and_update_evidence() {
        let db_path = unique_test_db_path("mixed");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        let normalized_patch = normalized(MIXED_PATCH);
        persist_with(
            &db,
            &event("session-1", None, MIXED_PATCH),
            &normalized_patch,
            1_000,
        )
        .expect("persist_with should succeed");

        let recent = db
            .recent_diff_trace_patches(0, i64::MAX)
            .expect("diff trace query should succeed");
        assert_eq!(recent.loaded_count(), 1);
        let file_paths: Vec<&str> = recent.patches[0]
            .patch
            .files
            .iter()
            .map(|file| file.new_path.as_str())
            .collect();
        assert_eq!(
            file_paths,
            vec!["a.txt", "c.txt"],
            "Delete File must not appear"
        );

        remove_test_db(&db_path);
    }

    /// AC15: a committed Codex `apply_patch` Update whose `diff_trace` carries
    /// synthetic, patch-local line numbers is still attributed through the
    /// existing, unmodified post-commit intersection pipeline
    /// (`build_agent_trace`, the same function the real `post-commit` hook
    /// flow calls) when the real committed line numbers differ, and the
    /// resulting Agent Trace identifies Codex as the tool and preserves the
    /// Codex model ID.
    #[test]
    fn apply_patch_diff_trace_attributes_through_agent_trace_pipeline_at_different_real_lines() {
        let db_path = unique_test_db_path("agent-trace-pipeline");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        let normalized_patch = normalized(UPDATE_ONLY_PATCH);
        persist_with(
            &db,
            &event("session-1", Some("gpt-5-codex"), UPDATE_ONLY_PATCH),
            &normalized_patch,
            1_000,
        )
        .expect("persist_with should succeed");

        let recent = db
            .recent_diff_trace_patches(0, i64::MAX)
            .expect("diff trace query should succeed");
        assert_eq!(recent.loaded_count(), 1);
        let constructed = &recent.patches[0];

        // A realistic post-commit patch where the same touched lines sit at
        // real line 42, far from the diff_trace's synthetic line 1, plus one
        // unrelated committed line that must not be attributed to Codex.
        let post_commit_patch = ParsedPatch {
            files: vec![PatchFileChange {
                old_path: "src/lib.rs".to_string(),
                new_path: "src/lib.rs".to_string(),
                kind: FileChangeKind::Modified,
                hunks: vec![PatchHunk {
                    old_start: 42,
                    old_count: 1,
                    new_start: 42,
                    new_count: 2,
                    model_id: None,
                    lines: vec![
                        TouchedLine {
                            kind: TouchedLineKind::Removed,
                            line_number: 42,
                            content: "old_line".to_string(),
                            session_id: None,
                        },
                        TouchedLine {
                            kind: TouchedLineKind::Added,
                            line_number: 42,
                            content: "new_line".to_string(),
                            session_id: None,
                        },
                        TouchedLine {
                            kind: TouchedLineKind::Added,
                            line_number: 43,
                            content: "unrelated_line".to_string(),
                            session_id: None,
                        },
                    ],
                }],
            }],
        };

        let agent_trace = build_agent_trace(
            &constructed.patch,
            &post_commit_patch,
            AgentTraceMetadataInput {
                commit_timestamp: "2026-04-23T10:20:30Z",
                commit_revision: "abc123def456",
                vcs_type: None,
                tool_name: constructed.tool_name.as_deref(),
                tool_version: constructed.tool_version.as_deref(),
            },
        )
        .expect("Agent Trace should build from the post-commit intersection");
        let agent_trace_json =
            serde_json::to_value(&agent_trace).expect("Agent Trace should serialize");

        assert_eq!(agent_trace_json["tool"]["name"], "codex");
        assert_eq!(
            agent_trace_json["files"][0]["conversations"][0]["contributor"]["model_id"],
            "gpt-5-codex"
        );

        remove_test_db(&db_path);
    }
}
