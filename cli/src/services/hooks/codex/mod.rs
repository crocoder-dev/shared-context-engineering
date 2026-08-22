use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::services::observability::traits::Logger;

use super::read_hook_stdin;

mod apply_patch;
mod bash_policy;
mod stop;
mod user_prompt_submit;

const CODEX_HOOK_EVENT_USER_PROMPT_SUBMIT: &str = "UserPromptSubmit";
const CODEX_HOOK_EVENT_STOP: &str = "Stop";
const CODEX_HOOK_EVENT_PRE_TOOL_USE: &str = "PreToolUse";
const CODEX_HOOK_EVENT_POST_TOOL_USE: &str = "PostToolUse";
const CODEX_HOOK_TOOL_BASH: &str = "Bash";
const CODEX_HOOK_TOOL_APPLY_PATCH: &str = "apply_patch";

/// A single Codex hook lifecycle event, deserialized from the raw STDIN JSON
/// payload `sce hooks codex` receives via
/// `.codex/hooks/run-sce-or-show-install-guidance.sh`.
///
/// Working contract (see plan `context/plans/codex-cli-integration.md`
/// Assumptions): `hook_event_name` is present on every event; `session_id`,
/// `turn_id`, `cwd`, and `model` vary by event; `tool_name`/`tool_use_id`/
/// `tool_input`/`tool_response` are present only on `PreToolUse`/`PostToolUse`;
/// `prompt` is present only on `UserPromptSubmit`, matching Claude's own
/// `UserPromptSubmit` payload shape (see `transform_claude_user_prompt_submit_with`);
/// `last_assistant_message` is present only on `Stop`, matching Claude's own
/// `Stop` payload shape (see `transform_claude_stop_with`).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct CodexHookEvent {
    pub(crate) hook_event_name: String,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) turn_id: Option<String>,
    #[serde(default)]
    pub(crate) cwd: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) tool_name: Option<String>,
    #[serde(default)]
    pub(crate) tool_use_id: Option<String>,
    #[serde(default)]
    pub(crate) tool_input: Option<Value>,
    #[serde(default)]
    pub(crate) tool_response: Option<Value>,
    #[serde(default)]
    pub(crate) prompt: Option<String>,
    #[serde(default)]
    pub(crate) last_assistant_message: Option<String>,
}

/// The set of Codex hook-event/tool combinations `sce hooks codex` gives
/// distinct behavior. Every other combination classifies as `NoOp`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CodexDispatchArm {
    UserPromptSubmit,
    Stop,
    PreToolUseBash,
    PostToolUseApplyPatch,
    NoOp,
}

pub(crate) fn classify_codex_event(event: &CodexHookEvent) -> CodexDispatchArm {
    match (event.hook_event_name.as_str(), event.tool_name.as_deref()) {
        (CODEX_HOOK_EVENT_USER_PROMPT_SUBMIT, _) => CodexDispatchArm::UserPromptSubmit,
        (CODEX_HOOK_EVENT_STOP, _) => CodexDispatchArm::Stop,
        (CODEX_HOOK_EVENT_PRE_TOOL_USE, Some(CODEX_HOOK_TOOL_BASH)) => {
            CodexDispatchArm::PreToolUseBash
        }
        (CODEX_HOOK_EVENT_POST_TOOL_USE, Some(CODEX_HOOK_TOOL_APPLY_PATCH)) => {
            CodexDispatchArm::PostToolUseApplyPatch
        }
        _ => CodexDispatchArm::NoOp,
    }
}

pub(super) fn run_codex_subcommand(repository_root: &Path, logger: Option<&dyn Logger>) -> String {
    let stdin_payload = match read_hook_stdin() {
        Ok(payload) => payload,
        Err(error) => return log_codex_fail_open(&error, logger),
    };

    match run_codex_subcommand_from_payload(repository_root, &stdin_payload, logger) {
        Ok(output) => output,
        Err(error) => log_codex_fail_open(&error, logger),
    }
}

fn run_codex_subcommand_from_payload(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    run_codex_subcommand_from_payload_with_state_root(repository_root, stdin_payload, logger, None)
}

#[cfg(test)]
fn run_codex_subcommand_from_payload_at_state_root(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    state_root: &Path,
) -> Result<String> {
    run_codex_subcommand_from_payload_with_state_root(
        repository_root,
        stdin_payload,
        logger,
        Some(state_root),
    )
}

fn run_codex_subcommand_from_payload_with_state_root(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    state_root: Option<&Path>,
) -> Result<String> {
    let event: CodexHookEvent = serde_json::from_str(stdin_payload)
        .context("Invalid Codex hook payload from STDIN: expected valid JSON.")?;

    Ok(match classify_codex_event(&event) {
        CodexDispatchArm::UserPromptSubmit => user_prompt_submit::handle(repository_root, &event)?,
        CodexDispatchArm::Stop => stop::handle(repository_root, &event)?,
        CodexDispatchArm::PreToolUseBash => bash_policy::handle(repository_root, &event)?,
        CodexDispatchArm::PostToolUseApplyPatch => match state_root {
            Some(state_root) => apply_patch::handle_with_state_root(
                repository_root,
                &event,
                Some(state_root),
                logger,
            )?,
            None => apply_patch::handle(repository_root, &event, logger)?,
        },
        CodexDispatchArm::NoOp => String::new(),
    })
}

fn log_codex_fail_open(error: &anyhow::Error, logger: Option<&dyn Logger>) -> String {
    if let Some(log) = logger {
        log.error("sce.hooks.codex.error", &error.to_string(), &[], None);
    }

    String::new()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde_json::json;

    use crate::services::agent_trace_storage::{
        resolve_agent_trace_storage_at_state_root,
        resolve_agent_trace_storage_for_hook_runtime_at_state_root, AgentTraceStorageContext,
    };

    use super::*;

    fn event(hook_event_name: &str, tool_name: Option<&str>) -> CodexHookEvent {
        CodexHookEvent {
            hook_event_name: hook_event_name.to_string(),
            session_id: Some("abc123".to_string()),
            turn_id: Some("turn-1".to_string()),
            cwd: None,
            model: None,
            tool_name: tool_name.map(str::to_string),
            tool_use_id: None,
            tool_input: None,
            tool_response: None,
            prompt: None,
            last_assistant_message: None,
        }
    }

    #[test]
    fn classify_codex_event_routes_user_prompt_submit() {
        assert_eq!(
            classify_codex_event(&event("UserPromptSubmit", None)),
            CodexDispatchArm::UserPromptSubmit
        );
    }

    #[test]
    fn classify_codex_event_routes_stop() {
        assert_eq!(
            classify_codex_event(&event("Stop", None)),
            CodexDispatchArm::Stop
        );
    }

    #[test]
    fn classify_codex_event_routes_pre_tool_use_bash() {
        assert_eq!(
            classify_codex_event(&event("PreToolUse", Some("Bash"))),
            CodexDispatchArm::PreToolUseBash
        );
    }

    #[test]
    fn classify_codex_event_routes_pre_tool_use_apply_patch_to_no_op() {
        assert_eq!(
            classify_codex_event(&event("PreToolUse", Some("apply_patch"))),
            CodexDispatchArm::NoOp
        );
    }

    #[test]
    fn classify_codex_event_routes_post_tool_use_apply_patch() {
        assert_eq!(
            classify_codex_event(&event("PostToolUse", Some("apply_patch"))),
            CodexDispatchArm::PostToolUseApplyPatch
        );
    }

    #[test]
    fn classify_codex_event_routes_unknown_pre_tool_use_tool_name_to_no_op() {
        assert_eq!(
            classify_codex_event(&event("PreToolUse", Some("Edit"))),
            CodexDispatchArm::NoOp
        );
    }

    #[test]
    fn classify_codex_event_routes_pre_tool_use_with_no_tool_name_to_no_op() {
        assert_eq!(
            classify_codex_event(&event("PreToolUse", None)),
            CodexDispatchArm::NoOp
        );
    }

    #[test]
    fn classify_codex_event_routes_post_tool_use_bash_to_no_op() {
        assert_eq!(
            classify_codex_event(&event("PostToolUse", Some("Bash"))),
            CodexDispatchArm::NoOp
        );
    }

    #[test]
    fn classify_codex_event_routes_unrecognized_hook_event_name_to_no_op() {
        assert_eq!(
            classify_codex_event(&event("SessionStart", None)),
            CodexDispatchArm::NoOp
        );
    }

    #[test]
    fn run_codex_subcommand_from_payload_no_ops_unsupported_combination_without_error() {
        let payload = r#"{"hook_event_name":"PreToolUse","session_id":"s1","tool_name":"Read"}"#;

        let output = run_codex_subcommand_from_payload(Path::new("/tmp"), payload, None)
            .expect("no-op dispatch should succeed");

        assert_eq!(output, "");
    }

    #[test]
    fn run_codex_subcommand_from_payload_no_ops_unrecognized_hook_event_name_without_error() {
        let payload = r#"{"hook_event_name":"SessionStart","session_id":"s1"}"#;

        let output = run_codex_subcommand_from_payload(Path::new("/tmp"), payload, None)
            .expect("no-op dispatch should succeed");

        assert_eq!(output, "");
    }

    #[test]
    fn run_codex_subcommand_from_payload_rejects_non_json_stdin() {
        let error = run_codex_subcommand_from_payload(Path::new("/tmp"), "not json", None)
            .expect_err("malformed payload should fail parsing");

        assert!(error.to_string().contains("Invalid Codex hook payload"));
    }

    #[test]
    fn run_codex_subcommand_fails_open_on_malformed_stdin_payload() {
        let error = anyhow::anyhow!("Invalid Codex hook payload from STDIN: expected valid JSON.");

        let output = log_codex_fail_open(&error, None);

        assert_eq!(output, "");
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sce-codex-pipeline-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temporary directory should be created");
        path
    }

    fn git(repository_root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repository_root)
            .output()
            .unwrap_or_else(|error| panic!("git {args:?} failed to spawn: {error}"));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn initialize_repository(label: &str) -> (PathBuf, PathBuf) {
        let repository_root = unique_temp_dir(&format!("{label}-repo"));
        git(&repository_root, &["init", "-q"]);
        git(
            &repository_root,
            &[
                "remote",
                "add",
                "origin",
                "https://example.invalid/codex-t19.git",
            ],
        );
        let state_root = unique_temp_dir(&format!("{label}-state"));
        let context = AgentTraceStorageContext {
            repository_root: &repository_root,
            explicit_repository_id: None,
            repository_remote: "origin",
        };
        let storage = resolve_agent_trace_storage_at_state_root(&context, &state_root)
            .expect("repository Agent Trace DB should initialize");
        drop(storage);
        (repository_root, state_root)
    }

    fn codex_apply_patch_payload(
        cwd: &Path,
        session_id: &str,
        model: &str,
        tool_use_id: &str,
        command: &str,
    ) -> String {
        json!({
            "hook_event_name": "PostToolUse",
            "session_id": session_id,
            "turn_id": "turn-realistic",
            "cwd": cwd,
            "model": model,
            "tool_name": "apply_patch",
            "tool_use_id": tool_use_id,
            "tool_input": {"command": command},
            "tool_response": {"success": true}
        })
        .to_string()
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn realistic_post_tool_use_patch_flows_through_repository_db_and_post_commit_attribution() {
        let (repository_root, state_root) = initialize_repository("end-to-end");
        let source_dir = repository_root.join("src");
        fs::create_dir_all(&source_dir).expect("source directory should be created");
        fs::write(source_dir.join("lib.rs"), "prefix\nold_line\nsuffix\n")
            .expect("initial source should be written");
        git(&repository_root, &["add", "."]);
        git(
            &repository_root,
            &[
                "-c",
                "user.name=SCE Test",
                "-c",
                "user.email=sce@example.invalid",
                "commit",
                "-qm",
                "initial",
            ],
        );

        let command = "<<\"EOF\"\n*** Begin Patch\n*** Update File: lib.rs\n@@\n-old_line\n+new_line\n*** End Patch\nEOF";
        let payload = codex_apply_patch_payload(
            &source_dir,
            " session-realistic ",
            "custom/codex-model",
            "tool-realistic-1",
            command,
        );
        let output = run_codex_subcommand_from_payload_at_state_root(
            &repository_root,
            &payload,
            None,
            &state_root,
        )
        .expect("realistic Codex PostToolUse dispatch should succeed");
        assert_eq!(output, "", "successful apply_patch hooks are silent");

        fs::write(source_dir.join("lib.rs"), "prefix\nnew_line\nsuffix\n")
            .expect("updated source should be written");
        git(&repository_root, &["add", "."]);
        git(
            &repository_root,
            &[
                "-c",
                "user.name=SCE Test",
                "-c",
                "user.email=sce@example.invalid",
                "commit",
                "-qm",
                "apply patch",
            ],
        );

        let context = AgentTraceStorageContext {
            repository_root: &repository_root,
            explicit_repository_id: None,
            repository_remote: "origin",
        };
        let storage =
            resolve_agent_trace_storage_for_hook_runtime_at_state_root(&context, &state_root)
                .expect("repository Agent Trace DB should reopen");
        let recent = storage
            .db
            .recent_diff_trace_patches(0, i64::MAX)
            .expect("stored Codex patch should be queryable");
        assert_eq!(recent.loaded_count(), 1);
        let stored_file = &recent.patches[0].patch.files[0];
        assert_eq!(recent.patches[0].session_id, "cx_session-realistic");
        assert_eq!(recent.patches[0].tool_name.as_deref(), Some("codex"));
        assert_eq!(recent.patches[0].tool_version, None);
        assert_eq!(recent.patches[0].payload_type, "patch");
        assert_eq!(recent.patches[0].patch.files.len(), 1);
        assert_eq!(stored_file.old_path, "src/lib.rs");
        assert_eq!(stored_file.new_path, "src/lib.rs");
        assert_eq!(
            stored_file.hunks[0].model_id.as_deref(),
            Some("custom/codex-model")
        );

        let flow = super::super::run_post_commit_intersection_flow_with(
            &repository_root,
            super::super::capture_post_commit_patch_from_git,
            super::super::current_unix_time_ms,
            |cutoff_ms, end_ms| storage.db.recent_diff_trace_patches(cutoff_ms, end_ms),
            |insert| {
                storage
                    .db
                    .insert_post_commit_patch_intersection(insert)
                    .map(|_| ())
            },
        )
        .expect("post-commit intersection should use the stored Codex evidence");
        let trace = super::super::run_post_commit_agent_trace_flow_with(
            &flow,
            None,
            "https://example.invalid/codex-t19.git",
            |value| {
                crate::services::agent_trace::validate_agent_trace_value(value)
                    .map_err(|error| anyhow::anyhow!(error.to_string()))
            },
            |insert| storage.db.insert_agent_trace(insert).map(|_| ()),
        )
        .expect("post-commit Agent Trace should persist");
        assert_eq!(
            trace.tool.as_ref().and_then(|tool| tool.name.as_deref()),
            Some("codex")
        );

        let intersections = storage
            .db
            .query_map(
                "SELECT intersection_patch FROM post_commit_patch_intersections ORDER BY id",
                (),
                |row| row.get::<String>(0).map_err(anyhow::Error::from),
            )
            .expect("intersection row should be queryable");
        assert_eq!(intersections.len(), 1);
        let intersection: serde_json::Value =
            serde_json::from_str(&intersections[0]).expect("intersection JSON should parse");
        assert_eq!(
            intersection["files"][0]["hunks"][0]["lines"][1]["session_id"],
            "cx_session-realistic"
        );

        let traces = storage
            .db
            .query_map(
                "SELECT trace_json FROM agent_traces ORDER BY id",
                (),
                |row| row.get::<String>(0).map_err(anyhow::Error::from),
            )
            .expect("Agent Trace row should be queryable");
        assert_eq!(traces.len(), 1);
        let trace_json: serde_json::Value =
            serde_json::from_str(&traces[0]).expect("stored Agent Trace JSON should parse");
        assert_eq!(trace_json["tool"]["name"], "codex");
        assert_eq!(
            trace_json["files"][0]["conversations"][0]["contributor"]["model_id"],
            "custom/codex-model"
        );

        fs::remove_dir_all(&repository_root).ok();
        fs::remove_dir_all(&state_root).ok();
    }

    #[test]
    fn delete_only_and_pure_rename_apply_patch_events_persist_no_rows() {
        let (repository_root, state_root) = initialize_repository("no-row-boundaries");
        let delete_payload = codex_apply_patch_payload(
            &repository_root,
            "session-delete",
            "custom/model",
            "tool-delete",
            "*** Begin Patch\n*** Delete File: obsolete.txt\n*** End Patch",
        );
        let rename_payload = codex_apply_patch_payload(
            &repository_root,
            "session-rename",
            "custom/model",
            "tool-rename",
            "*** Begin Patch\n*** Update File: old.txt\n*** Move to: new.txt\n*** End Patch",
        );

        for payload in [delete_payload, rename_payload] {
            let output = run_codex_subcommand_from_payload_at_state_root(
                &repository_root,
                &payload,
                None,
                &state_root,
            )
            .expect("delete and pure-rename hooks should fail open successfully");
            assert_eq!(output, "");
        }

        let storage = resolve_agent_trace_storage_for_hook_runtime_at_state_root(
            &AgentTraceStorageContext {
                repository_root: &repository_root,
                explicit_repository_id: None,
                repository_remote: "origin",
            },
            &state_root,
        )
        .expect("repository Agent Trace DB should reopen");
        let recent = storage
            .db
            .recent_diff_trace_patches(0, i64::MAX)
            .expect("diff trace query should succeed");
        assert_eq!(recent.loaded_count(), 0);
        assert_eq!(recent.skipped_count(), 0);

        fs::remove_dir_all(&repository_root).ok();
        fs::remove_dir_all(&state_root).ok();
    }
}
