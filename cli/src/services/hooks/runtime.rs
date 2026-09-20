use std::io::{self, Read};
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
#[cfg(test)]
use crate::services::agent_trace_storage::resolve_agent_trace_storage_for_hook_runtime_at_state_root;
use crate::services::agent_trace_storage::{
    resolve_agent_trace_storage_for_hook_runtime, AgentTraceStorageContext,
};
use crate::services::config;
use anyhow::{bail, Context, Result};

pub(crate) const CLAUDE_MODEL_ID_PREFIX: &str = "claude/";
pub(crate) const DIFF_TRACE_OPENCODE_SESSION_ID_PREFIX: &str = "oc_";
pub(crate) const DIFF_TRACE_CLAUDE_SESSION_ID_PREFIX: &str = "cc_";
pub(crate) const DIFF_TRACE_PI_SESSION_ID_PREFIX: &str = "pi_";
pub(crate) const DIFF_TRACE_CODEX_SESSION_ID_PREFIX: &str = "cx_";
pub(crate) const OPENCODE_TOOL_NAME: &str = "opencode";
pub(crate) const CLAUDE_TOOL_NAME: &str = "claude";
pub(crate) const PI_TOOL_NAME: &str = "pi";
pub(crate) const CODEX_TOOL_NAME: &str = "codex";
pub(crate) const NORMALIZED_CONVERSATION_TRACE_TOOL_NAMES: &[&str] =
    &[OPENCODE_TOOL_NAME, PI_TOOL_NAME];
pub(crate) type PayloadValidationError = fn(&str) -> String;

pub(crate) fn prefixed_diff_trace_session_id(tool_name: &str, raw_session_id: &str) -> String {
    prefixed_session_id(tool_name, raw_session_id)
}

pub(crate) fn prefixed_conversation_trace_session_id(
    tool_name: &str,
    raw_session_id: &str,
) -> String {
    prefixed_session_id(tool_name, raw_session_id)
}

pub(crate) fn prefixed_session_id(tool_name: &str, raw_session_id: &str) -> String {
    let prefix = match tool_name {
        OPENCODE_TOOL_NAME => DIFF_TRACE_OPENCODE_SESSION_ID_PREFIX,
        CLAUDE_TOOL_NAME => DIFF_TRACE_CLAUDE_SESSION_ID_PREFIX,
        PI_TOOL_NAME => DIFF_TRACE_PI_SESSION_ID_PREFIX,
        CODEX_TOOL_NAME => DIFF_TRACE_CODEX_SESSION_ID_PREFIX,
        _ => return raw_session_id.to_string(),
    };

    if raw_session_id.starts_with(prefix) {
        raw_session_id.to_string()
    } else {
        format!("{prefix}{raw_session_id}")
    }
}
pub(crate) fn open_agent_trace_db_for_hook_runtime(
    repository_root: &Path,
    context_message: &'static str,
) -> Result<RepositoryAgentTraceDb> {
    let storage_config = config::resolve_agent_trace_storage_runtime_config(repository_root)
        .context("Failed to resolve Agent Trace repository storage config.")?;
    let storage_context = AgentTraceStorageContext {
        repository_root,
        explicit_repository_id: storage_config.repository_id.as_deref(),
        repository_remote: &storage_config.repository_remote,
    };

    resolve_agent_trace_storage_for_hook_runtime(&storage_context)
        .map(|storage| storage.db)
        .context(context_message)
}

#[cfg(test)]
pub(crate) fn open_agent_trace_db_for_hook_runtime_at_state_root(
    repository_root: &Path,
    state_root: &Path,
    context_message: &'static str,
) -> Result<RepositoryAgentTraceDb> {
    let storage_config = config::resolve_agent_trace_storage_runtime_config(repository_root)
        .context("Failed to resolve Agent Trace repository storage config.")?;
    let storage_context = AgentTraceStorageContext {
        repository_root,
        explicit_repository_id: storage_config.repository_id.as_deref(),
        repository_remote: &storage_config.repository_remote,
    };

    resolve_agent_trace_storage_for_hook_runtime_at_state_root(&storage_context, state_root)
        .map(|storage| storage.db)
        .context(context_message)
}

pub(crate) fn current_unix_time_ms() -> Result<i64> {
    i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())
        .context("Current time exceeds i64 range for post-commit intersection.")
}
pub(crate) fn read_hook_stdin() -> Result<String> {
    let mut stdin_payload = String::new();
    io::stdin()
        .read_to_string(&mut stdin_payload)
        .context("Failed to read hook input from STDIN.")?;
    Ok(stdin_payload)
}

pub(crate) fn run_git_command_capture_stdout(
    repository_root: &Path,
    args: &[&str],
    context_message: &str,
) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository_root)
        .output()
        .with_context(|| {
            format!(
                "{} (directory: '{}')",
                context_message,
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

    String::from_utf8(output.stdout).context("git command output contained invalid UTF-8")
}

pub(crate) fn resolve_runtime_state(repository_root: &Path) -> Result<HookRuntimeState> {
    Ok(HookRuntimeState {
        sce_disabled: env_flag_is_truthy("SCE_DISABLED"),
        attribution_hooks_enabled: config::resolve_hook_runtime_config(repository_root)?
            .attribution_hooks_enabled,
    })
}

pub(crate) fn env_flag_is_truthy(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| env_value_is_truthy(&value))
}

pub(crate) fn env_value_is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub(crate) fn commit_msg_policy_gate_passed(runtime: &HookRuntimeState) -> bool {
    !runtime.sce_disabled && runtime.attribution_hooks_enabled
}

pub(crate) fn pre_commit_no_op_reason(runtime: &HookRuntimeState) -> HookNoOpReason {
    if runtime.sce_disabled {
        HookNoOpReason::Disabled
    } else {
        HookNoOpReason::AttributionOnlyCommitMsgMode
    }
}

pub(crate) fn post_rewrite_no_op_reason(runtime: &HookRuntimeState) -> HookNoOpReason {
    if runtime.sce_disabled {
        HookNoOpReason::Disabled
    } else {
        HookNoOpReason::AttributionOnlyCommitMsgMode
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HookRuntimeState {
    pub sce_disabled: bool,
    pub attribution_hooks_enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HookNoOpReason {
    Disabled,
    AttributionOnlyCommitMsgMode,
}
