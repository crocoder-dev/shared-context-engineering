use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;

use crate::services::bash_policy::{
    evaluate_bash_command_policy, format_policy_block_message, PolicyEvaluation,
};
use crate::services::config;

use super::CodexHookEvent;

pub(crate) enum CodexBashPolicyDecision {
    Allowed,
    Blocked(String),
}

pub(crate) fn evaluate_codex_bash_policy(
    repository_root: &Path,
    command: &str,
) -> Result<CodexBashPolicyDecision> {
    let policy_config = config::resolve_bash_policy_runtime_config(repository_root)
        .context("Failed to resolve bash policy configuration for Codex PreToolUse Bash.")?;

    decision_from_evaluation(evaluate_bash_command_policy(
        command,
        policy_config.as_ref(),
    ))
}

fn decision_from_evaluation(evaluation: PolicyEvaluation) -> Result<CodexBashPolicyDecision> {
    match evaluation {
        PolicyEvaluation::Allowed { .. } => Ok(CodexBashPolicyDecision::Allowed),
        PolicyEvaluation::Blocked { policy, .. } => Ok(CodexBashPolicyDecision::Blocked(
            codex_bash_policy_deny_response(&format_policy_block_message(&policy))?,
        )),
    }
}

fn codex_bash_policy_deny_response(reason: &str) -> Result<String> {
    serde_json::to_string(&json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason
        }
    }))
    .context("Failed to serialize Codex PreToolUse Bash deny response.")
}

pub(crate) fn bash_command_from_tool_input(tool_input: Option<&serde_json::Value>) -> Result<&str> {
    tool_input
        .and_then(|value| value.get("command"))
        .and_then(serde_json::Value::as_str)
        .filter(|command| !command.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Invalid Codex PreToolUse Bash payload: tool_input.command must be a non-empty string."
            )
        })
}

pub(super) fn handle(repository_root: &Path, event: &CodexHookEvent) -> Result<String> {
    let command = bash_command_from_event(event)?;

    Ok(
        match evaluate_codex_bash_policy(repository_root, command)? {
            CodexBashPolicyDecision::Allowed => String::new(),
            CodexBashPolicyDecision::Blocked(response) => response,
        },
    )
}

fn bash_command_from_event(event: &CodexHookEvent) -> Result<&str> {
    bash_command_from_tool_input(event.tool_input.as_ref())
}
