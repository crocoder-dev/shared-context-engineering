use anyhow::{anyhow, bail, Context, Result};
use serde_json::{Map, Value};

pub(super) const HOOK_EVENT_NAME_FIELD: &str = "hook_event_name";
pub(super) const SESSION_ID_FIELD: &str = "session_id";
pub(super) const CWD_FIELD: &str = "cwd";
pub(super) const AGENT_ID_FIELD: &str = "agent_id";
pub(super) const TOOL_NAME_FIELD: &str = "tool_name";
pub(super) const TOOL_USE_ID_FIELD: &str = "tool_use_id";
pub(super) const TOOL_INPUT_FIELD: &str = "tool_input";
pub(super) const RUN_IN_BACKGROUND_FIELD: &str = "run_in_background";
pub(super) const PROMPT_ID_FIELD: &str = "prompt_id";
pub(super) const AGENT_TYPE_FIELD: &str = "agent_type";
pub(super) const WORKTREE_PATH_FIELD: &str = "worktree_path";

pub(super) const HOOK_EVENT_PRE_TOOL_USE: &str = "PreToolUse";
pub(super) const HOOK_EVENT_POST_TOOL_USE: &str = "PostToolUse";
pub(super) const HOOK_EVENT_POST_TOOL_USE_FAILURE: &str = "PostToolUseFailure";
pub(super) const HOOK_EVENT_PERMISSION_DENIED: &str = "PermissionDenied";
pub(super) const HOOK_EVENT_STOP: &str = "Stop";
pub(super) const HOOK_EVENT_STOP_FAILURE: &str = "StopFailure";
pub(super) const HOOK_EVENT_USER_PROMPT_SUBMIT: &str = "UserPromptSubmit";
pub(super) const HOOK_EVENT_SUBAGENT_STOP: &str = "SubagentStop";
pub(super) const HOOK_EVENT_SESSION_END: &str = "SessionEnd";
pub(super) const HOOK_EVENT_WORKTREE_REMOVE: &str = "WorktreeRemove";
pub(super) const HOOK_EVENT_SESSION_START: &str = "SessionStart";
pub(super) const HOOK_EVENT_SUBAGENT_START: &str = "SubagentStart";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClaudeHookEvent {
    PreToolUse(ClaudeToolExecution),
    PostToolUse(ClaudeToolIdentity),
    PostToolUseFailure(ClaudeToolIdentity),
    PermissionDenied(ClaudeToolIdentity),
    Stop(ClaudeSessionIdentity),
    StopFailure(ClaudeSessionIdentity),
    UserPromptSubmit(ClaudeSessionIdentity),
    SubagentStop(ClaudeAgentIdentity),
    SessionEnd(ClaudeSessionIdentity),
    WorktreeRemove(ClaudeWorktreeRemove),
    SessionStart,
    SubagentStart,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaudeToolIdentity {
    pub session_id: String,
    pub cwd: String,
    pub agent_id: Option<String>,
    pub tool_name: String,
    pub tool_use_id: String,
}

impl ClaudeToolIdentity {
    pub(crate) fn attempt_key(&self) -> AttemptKey {
        AttemptKey {
            session_id: self.session_id.clone(),
            agent_id: self.agent_id.clone(),
            tool_use_id: self.tool_use_id.clone(),
        }
    }

    pub(crate) fn is_subagent(&self) -> bool {
        self.agent_id.is_some()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaudeToolExecution {
    pub identity: ClaudeToolIdentity,
    pub prompt_id: Option<String>,
    pub agent_type: Option<String>,
    pub run_in_background: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaudeSessionIdentity {
    pub session_id: String,
    pub cwd: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaudeAgentIdentity {
    pub session_id: String,
    pub cwd: String,
    pub agent_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaudeWorktreeRemove {
    pub session_id: String,
    pub worktree_path: String,
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct AttemptKey {
    pub session_id: String,
    pub agent_id: Option<String>,
    pub tool_use_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolClassification {
    MutationCapable,
    ReadOnly,
    Delegation,
}

pub(super) const DELEGATION_TOOL_NAME: &str = "Agent";
pub(super) const KNOWN_READ_ONLY_TOOL_NAMES: &[&str] = &[
    "Read",
    "Glob",
    "Grep",
    "WebFetch",
    "WebSearch",
    "AskUserQuestion",
];

pub(crate) fn classify_tool(tool_name: &str) -> ToolClassification {
    if tool_name == DELEGATION_TOOL_NAME {
        return ToolClassification::Delegation;
    }
    if KNOWN_READ_ONLY_TOOL_NAMES.contains(&tool_name) {
        return ToolClassification::ReadOnly;
    }
    ToolClassification::MutationCapable
}

pub(super) const BASH_TOOL_NAME: &str = "Bash";
pub(super) const POWERSHELL_TOOL_NAME: &str = "PowerShell";

pub(crate) fn is_explicit_background_shell(tool_name: &str, run_in_background: bool) -> bool {
    run_in_background && (tool_name == BASH_TOOL_NAME || tool_name == POWERSHELL_TOOL_NAME)
}

pub(super) const CLAUDE_SCOPE_ID_SCHEME: &str = "cc-tool-v1";

pub(crate) fn format_claude_scope_id(attempt_seq: u64, key: &AttemptKey) -> String {
    let agent_id = key.agent_id.as_deref().unwrap_or("");
    format!(
        "{CLAUDE_SCOPE_ID_SCHEME}|n={attempt_seq}|s={}:{}|a={}:{}|t={}:{}",
        key.session_id.len(),
        key.session_id,
        agent_id.len(),
        agent_id,
        key.tool_use_id.len(),
        key.tool_use_id,
    )
}

pub(crate) fn claude_scope_start_event_id(scope_id: &str) -> String {
    format!("{scope_id}|start")
}

pub(crate) fn claude_scope_close_event_id(scope_id: &str) -> String {
    format!("{scope_id}|close")
}

pub(crate) fn parse_claude_hook_event(stdin_payload: &str) -> Result<ClaudeHookEvent> {
    if stdin_payload.trim().is_empty() {
        bail!(validation_error(
            "expected a JSON object, got an empty payload"
        ));
    }

    let parsed: Value = serde_json::from_str(stdin_payload)
        .with_context(|| validation_error("expected valid JSON"))?;
    let object = parsed
        .as_object()
        .ok_or_else(|| anyhow!(validation_error("expected a JSON object")))?;

    let hook_event_name = required_non_blank_str(object, HOOK_EVENT_NAME_FIELD)?;

    match hook_event_name.as_str() {
        HOOK_EVENT_PRE_TOOL_USE => parse_pre_tool_use(object).map(ClaudeHookEvent::PreToolUse),
        HOOK_EVENT_POST_TOOL_USE => parse_tool_identity(object).map(ClaudeHookEvent::PostToolUse),
        HOOK_EVENT_POST_TOOL_USE_FAILURE => {
            parse_tool_identity(object).map(ClaudeHookEvent::PostToolUseFailure)
        }
        HOOK_EVENT_PERMISSION_DENIED => {
            parse_tool_identity(object).map(ClaudeHookEvent::PermissionDenied)
        }
        HOOK_EVENT_STOP => parse_session_identity(object).map(ClaudeHookEvent::Stop),
        HOOK_EVENT_STOP_FAILURE => parse_session_identity(object).map(ClaudeHookEvent::StopFailure),
        HOOK_EVENT_USER_PROMPT_SUBMIT => {
            parse_session_identity(object).map(ClaudeHookEvent::UserPromptSubmit)
        }
        HOOK_EVENT_SUBAGENT_STOP => parse_agent_identity(object).map(ClaudeHookEvent::SubagentStop),
        HOOK_EVENT_SESSION_END => parse_session_identity(object).map(ClaudeHookEvent::SessionEnd),
        HOOK_EVENT_WORKTREE_REMOVE => {
            parse_worktree_remove(object).map(ClaudeHookEvent::WorktreeRemove)
        }
        HOOK_EVENT_SESSION_START => Ok(ClaudeHookEvent::SessionStart),
        HOOK_EVENT_SUBAGENT_START => Ok(ClaudeHookEvent::SubagentStart),
        other => bail!(validation_error(&format!(
            "unsupported hook_event_name '{other}'"
        ))),
    }
}

pub(super) fn parse_tool_identity(object: &Map<String, Value>) -> Result<ClaudeToolIdentity> {
    Ok(ClaudeToolIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
        tool_name: required_non_blank_str(object, TOOL_NAME_FIELD)?,
        tool_use_id: required_non_blank_str(object, TOOL_USE_ID_FIELD)?,
        agent_id: optional_non_blank_str(object, AGENT_ID_FIELD)?,
    })
}

pub(super) fn parse_pre_tool_use(object: &Map<String, Value>) -> Result<ClaudeToolExecution> {
    Ok(ClaudeToolExecution {
        identity: parse_tool_identity(object)?,
        prompt_id: optional_non_blank_str(object, PROMPT_ID_FIELD)?,
        agent_type: optional_non_blank_str(object, AGENT_TYPE_FIELD)?,
        run_in_background: parse_run_in_background(object)?,
    })
}

pub(super) fn parse_run_in_background(object: &Map<String, Value>) -> Result<bool> {
    let Some(tool_input) = object.get(TOOL_INPUT_FIELD) else {
        return Ok(false);
    };
    if tool_input.is_null() {
        return Ok(false);
    }
    let tool_input = tool_input.as_object().ok_or_else(|| {
        anyhow!(validation_error(&format!(
            "field '{TOOL_INPUT_FIELD}' must be a JSON object"
        )))
    })?;

    match tool_input.get(RUN_IN_BACKGROUND_FIELD) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => bail!(validation_error(&format!(
            "field '{TOOL_INPUT_FIELD}.{RUN_IN_BACKGROUND_FIELD}' must be a boolean"
        ))),
    }
}

pub(super) fn parse_session_identity(object: &Map<String, Value>) -> Result<ClaudeSessionIdentity> {
    Ok(ClaudeSessionIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
    })
}

pub(super) fn parse_agent_identity(object: &Map<String, Value>) -> Result<ClaudeAgentIdentity> {
    Ok(ClaudeAgentIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
        agent_id: required_non_blank_str(object, AGENT_ID_FIELD)?,
    })
}

pub(super) fn parse_worktree_remove(object: &Map<String, Value>) -> Result<ClaudeWorktreeRemove> {
    Ok(ClaudeWorktreeRemove {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        worktree_path: required_non_blank_str(object, WORKTREE_PATH_FIELD)?,
    })
}

pub(super) fn required_field<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a Value> {
    object.get(field).ok_or_else(|| {
        anyhow!(validation_error(&format!(
            "missing required field '{field}'"
        )))
    })
}

pub(super) fn required_str(object: &Map<String, Value>, field: &str) -> Result<String> {
    required_field(object, field)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| {
            anyhow!(validation_error(&format!(
                "field '{field}' must be a string"
            )))
        })
}

pub(super) fn required_non_blank_str(object: &Map<String, Value>, field: &str) -> Result<String> {
    let value = required_str(object, field)?;
    if value.trim().is_empty() {
        bail!(validation_error(&format!(
            "field '{field}' must be a non-blank string"
        )));
    }
    Ok(value)
}

pub(super) fn optional_non_blank_str(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Option<String>> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            if value.trim().is_empty() {
                bail!(validation_error(&format!(
                    "field '{field}' must be null, absent, or a non-blank string"
                )));
            }
            Ok(Some(value.clone()))
        }
        Some(_) => bail!(validation_error(&format!(
            "field '{field}' must be null, absent, or a non-blank string"
        ))),
    }
}

pub(super) fn validation_error(detail: &str) -> String {
    format!("Invalid Claude hook event payload from STDIN: {detail}.")
}
