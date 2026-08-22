use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;

use crate::services::bash_policy::{
    evaluate_bash_command_policy, format_policy_block_message, PolicyEvaluation,
};
use crate::services::config;
use crate::services::config::policy::BashPolicyConfig;

use super::CodexHookEvent;

/// Routes a Codex `PreToolUse(Bash)` event through the existing SCE Bash
/// policy engine (`evaluate_bash_command_policy` in
/// `cli/src/services/bash_policy.rs`) unchanged — no reimplemented matching.
///
/// An allowed command produces silent hook success (empty stdout, no
/// model-visible output). A blocked command produces Codex's own native
/// `PreToolUse` deny response: `{"hookSpecificOutput": {"hookEventName":
/// "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason":
/// ...}}`, confirmed against Codex's real hook contract (`openai/codex`
/// issue #28437) — identical in shape to `render_claude_hook_result` in
/// `bash_policy.rs`. Neither branch reads or writes `diff_traces`, a
/// snapshot, or any pending-state file; `apply_patch` handling is a
/// different dispatch arm (T10/T11).
pub(super) fn handle(repository_root: &Path, event: &CodexHookEvent) -> Result<String> {
    let command = bash_command_from_event(event)?;

    let policy_config = config::resolve_bash_policy_runtime_config(repository_root)
        .context("Failed to resolve bash policy configuration for Codex PreToolUse Bash.")?;

    render_bash_policy_response(command, policy_config.as_ref())
}

fn render_bash_policy_response(
    command: &str,
    policy_config: Option<&BashPolicyConfig>,
) -> Result<String> {
    match evaluate_bash_command_policy(command, policy_config) {
        PolicyEvaluation::Allowed { .. } => Ok(String::new()),
        PolicyEvaluation::Blocked { policy, .. } => serde_json::to_string(&json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": format_policy_block_message(&policy)
            }
        }))
        .context("Failed to serialize Codex PreToolUse Bash deny response."),
    }
}

/// Codex's `PreToolUse` `tool_input` for the `Bash` tool carries the shell
/// command string under `command`, mirroring Claude's own `Bash` `tool_input`
/// shape (`ClaudeBashToolInput` in `bash_policy.rs`). This is a working
/// assumption pending direct confirmation against a live Codex CLI payload
/// (see plan `context/plans/codex-cli-integration.md` Assumptions and T06's
/// precedent for adjusting only field extraction, not architecture, if
/// reality differs).
fn bash_command_from_event(event: &CodexHookEvent) -> Result<&str> {
    event
        .tool_input
        .as_ref()
        .and_then(|value| value.get("command"))
        .and_then(|value| value.as_str())
        .filter(|command| !command.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Invalid Codex PreToolUse Bash payload: tool_input.command must be a non-empty string."
            )
        })
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde_json::json;

    use super::*;
    use crate::services::agent_trace_storage::{
        resolve_agent_trace_storage_at_state_root, AgentTraceStorageContext,
    };
    use crate::services::config::policy::CustomBashPolicyEntry;

    fn event_with_tool_input(tool_input: Option<serde_json::Value>) -> CodexHookEvent {
        CodexHookEvent {
            hook_event_name: "PreToolUse".to_string(),
            session_id: Some("session-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            cwd: None,
            model: None,
            tool_name: Some("Bash".to_string()),
            tool_use_id: Some("tool-1".to_string()),
            tool_input,
            tool_response: None,
            prompt: None,
            last_assistant_message: None,
        }
    }

    fn blocking_policy_config() -> BashPolicyConfig {
        BashPolicyConfig {
            presets: Vec::new(),
            custom: vec![CustomBashPolicyEntry {
                id: "block-rm".to_string(),
                argv_prefix: vec!["rm".to_string()],
                satisfied_by: Vec::new(),
                message: "This repository does not allow `rm` via the bash tool.".to_string(),
            }],
        }
    }

    #[test]
    fn bash_command_from_event_reads_tool_input_command() {
        let event = event_with_tool_input(Some(json!({"command": "echo hi"})));
        assert_eq!(bash_command_from_event(&event).unwrap(), "echo hi");
    }

    #[test]
    fn bash_command_from_event_rejects_missing_tool_input() {
        let event = event_with_tool_input(None);
        let error = bash_command_from_event(&event).expect_err("missing tool_input should error");
        assert!(error.to_string().contains("tool_input.command"));
    }

    #[test]
    fn bash_command_from_event_rejects_blank_command() {
        let event = event_with_tool_input(Some(json!({"command": "   "})));
        assert!(bash_command_from_event(&event).is_err());
    }

    #[test]
    fn render_bash_policy_response_is_silent_for_an_allowed_command() {
        let output = render_bash_policy_response("echo generated > generated.txt", None)
            .expect("evaluation should succeed");
        assert_eq!(output, "");
    }

    #[test]
    fn render_bash_policy_response_denies_with_codex_native_shape_for_a_blocked_command() {
        let config = blocking_policy_config();
        let output = render_bash_policy_response("rm -rf /tmp/x", Some(&config))
            .expect("evaluation should succeed");

        let parsed: serde_json::Value =
            serde_json::from_str(&output).expect("deny output should be valid JSON");
        assert_eq!(
            parsed["hookSpecificOutput"]["hookEventName"],
            json!("PreToolUse")
        );
        assert_eq!(
            parsed["hookSpecificOutput"]["permissionDecision"],
            json!("deny")
        );
        let reason = parsed["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .expect("reason should be a string");
        assert!(reason.contains("block-rm"));
        assert!(reason.contains("does not allow `rm`"));
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sce-codex-bash-policy-{label}-{}-{nonce}",
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

    #[test]
    fn codex_bash_pre_tool_use_path_creates_no_diff_trace_for_a_filesystem_mutation_command() {
        let repo_root = unique_temp_dir("repo");
        git(&repo_root, &["init", "-q"]);
        git(
            &repo_root,
            &[
                "remote",
                "add",
                "origin",
                "https://example.invalid/codex-bash-policy-test.git",
            ],
        );
        let state_root = unique_temp_dir("state");

        let payload = json!({
            "hook_event_name": "PreToolUse",
            "session_id": "session-1",
            "turn_id": "turn-1",
            "tool_name": "Bash",
            "tool_use_id": "tool-1",
            "tool_input": {"command": "echo generated > generated.txt"}
        })
        .to_string();

        let output = super::super::run_codex_subcommand_from_payload(&repo_root, &payload, None)
            .expect("Codex Bash PreToolUse dispatch should succeed");
        assert_eq!(output, "", "an allowed command must be silent");

        let storage = resolve_agent_trace_storage_at_state_root(
            &AgentTraceStorageContext {
                repository_root: &repo_root,
                explicit_repository_id: None,
                repository_remote: "origin",
            },
            &state_root,
        )
        .expect("Agent Trace storage should resolve for the scratch repo");

        let recent = storage
            .db
            .recent_diff_trace_patches(0, i64::MAX)
            .expect("diff trace query should succeed");
        assert_eq!(
            recent.loaded_count(),
            0,
            "the Codex Bash hook path must create no diff_traces rows"
        );

        std::fs::remove_dir_all(&repo_root).ok();
        std::fs::remove_dir_all(&state_root).ok();
    }
}
