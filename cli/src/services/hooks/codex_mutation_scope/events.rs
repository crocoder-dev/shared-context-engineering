use anyhow::{anyhow, bail, Context, Result};
use serde_json::{Map, Value};

pub(super) const HOOK_EVENT_NAME_FIELD: &str = "hook_event_name";
pub(super) const SESSION_ID_FIELD: &str = "session_id";
pub(super) const TURN_ID_FIELD: &str = "turn_id";
pub(super) const CWD_FIELD: &str = "cwd";
pub(super) const AGENT_ID_FIELD: &str = "agent_id";
pub(super) const AGENT_TYPE_FIELD: &str = "agent_type";
pub(super) const MODEL_FIELD: &str = "model";
pub(super) const PROVENANCE_FIELD: &str = "provenance";
pub(super) const TOOL_NAME_FIELD: &str = "tool_name";
pub(super) const TOOL_USE_ID_FIELD: &str = "tool_use_id";
pub(super) const TOOL_INPUT_FIELD: &str = "tool_input";

pub(super) const CODEX_TRACKED_TOOL_BASH: &str = "Bash";

pub(super) const HOOK_EVENT_PRE_TOOL_USE: &str = "PreToolUse";
pub(super) const HOOK_EVENT_POST_TOOL_USE: &str = "PostToolUse";
pub(super) const HOOK_EVENT_STOP: &str = "Stop";
pub(super) const HOOK_EVENT_INTERRUPT: &str = "Interrupt";
pub(super) const HOOK_EVENT_SUBAGENT_STOP: &str = "SubagentStop";
pub(super) const HOOK_EVENT_SESSION_END: &str = "SessionEnd";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CodexHookEvent {
    PreToolUse(CodexToolExecution),
    PostToolUse(CodexToolIdentity),
    Stop(CodexTurnIdentity),
    Interrupt(CodexTurnIdentity),
    SubagentStop(CodexAgentIdentity),
    SessionEnd(CodexSessionIdentity),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexToolIdentity {
    pub session_id: String,
    pub turn_id: String,
    pub cwd: String,
    pub agent_id: Option<String>,
    pub tool_name: String,
    pub tool_use_id: String,
}

impl CodexToolIdentity {
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
pub(crate) struct CodexToolExecution {
    pub identity: CodexToolIdentity,
    pub agent_type: Option<String>,
    pub model: Option<String>,
    pub tool_input: Option<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexTurnIdentity {
    pub session_id: String,
    pub turn_id: String,
    pub cwd: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexAgentIdentity {
    pub session_id: String,
    pub turn_id: String,
    pub cwd: String,
    pub agent_id: String,
    pub agent_type: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexSessionIdentity {
    pub session_id: String,
    pub cwd: String,
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
    TrackedMutation,
    Delegation,
    Untracked,
}

pub(super) const TRACKED_MUTATION_TOOL_NAMES: &[&str] = &["Bash", "apply_patch"];
pub(super) const DELEGATION_TOOL_NAMES: &[&str] =
    &["collaborationspawn_agent", "collaborationwait_agent"];
pub(super) const MCP_TOOL_NAME_PREFIX: &str = "mcp__";

pub(crate) fn is_mcp_tool_name(tool_name: &str) -> bool {
    tool_name.starts_with(MCP_TOOL_NAME_PREFIX)
}

pub(crate) fn classify_tool(tool_name: &str) -> ToolClassification {
    if TRACKED_MUTATION_TOOL_NAMES.contains(&tool_name) {
        ToolClassification::TrackedMutation
    } else if DELEGATION_TOOL_NAMES.contains(&tool_name) {
        ToolClassification::Delegation
    } else {
        ToolClassification::Untracked
    }
}

pub(super) const CODEX_SCOPE_ID_SCHEME: &str = "cx-tool-v1";

pub(crate) fn format_codex_scope_id(attempt_seq: u64, key: &AttemptKey) -> String {
    let agent_id = key.agent_id.as_deref().unwrap_or("");
    format!(
        "{CODEX_SCOPE_ID_SCHEME}|n={attempt_seq}|s={}:{}|a={}:{}|t={}:{}",
        key.session_id.len(),
        key.session_id,
        agent_id.len(),
        agent_id,
        key.tool_use_id.len(),
        key.tool_use_id,
    )
}

pub(crate) fn codex_scope_start_event_id(scope_id: &str) -> String {
    format!("{scope_id}|start")
}

pub(crate) fn codex_scope_close_event_id(scope_id: &str) -> String {
    format!("{scope_id}|close")
}

pub(crate) fn parse_codex_hook_event(stdin_payload: &str) -> Result<CodexHookEvent> {
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
        HOOK_EVENT_PRE_TOOL_USE => parse_pre_tool_use(object).map(CodexHookEvent::PreToolUse),
        HOOK_EVENT_POST_TOOL_USE => parse_tool_identity(object).map(CodexHookEvent::PostToolUse),
        HOOK_EVENT_STOP => parse_turn_identity(object).map(CodexHookEvent::Stop),
        HOOK_EVENT_INTERRUPT => parse_turn_identity(object).map(CodexHookEvent::Interrupt),
        HOOK_EVENT_SUBAGENT_STOP => parse_agent_identity(object).map(CodexHookEvent::SubagentStop),
        HOOK_EVENT_SESSION_END => parse_session_identity(object).map(CodexHookEvent::SessionEnd),
        other => bail!(validation_error(&format!(
            "unsupported hook_event_name '{other}'"
        ))),
    }
}

pub(super) fn parse_tool_identity(object: &Map<String, Value>) -> Result<CodexToolIdentity> {
    Ok(CodexToolIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        turn_id: required_non_blank_str(object, TURN_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
        agent_id: optional_non_blank_str(object, AGENT_ID_FIELD)?,
        tool_name: required_non_blank_str(object, TOOL_NAME_FIELD)?,
        tool_use_id: required_non_blank_str(object, TOOL_USE_ID_FIELD)?,
    })
}

pub(super) fn parse_pre_tool_use(object: &Map<String, Value>) -> Result<CodexToolExecution> {
    Ok(CodexToolExecution {
        identity: parse_tool_identity(object)?,
        agent_type: optional_non_blank_str(object, AGENT_TYPE_FIELD)?,
        model: tolerated_model(object),
        tool_input: object.get(TOOL_INPUT_FIELD).cloned(),
    })
}

pub(super) fn parse_turn_identity(object: &Map<String, Value>) -> Result<CodexTurnIdentity> {
    Ok(CodexTurnIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        turn_id: required_non_blank_str(object, TURN_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
    })
}

pub(super) fn parse_agent_identity(object: &Map<String, Value>) -> Result<CodexAgentIdentity> {
    Ok(CodexAgentIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        turn_id: required_non_blank_str(object, TURN_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
        agent_id: required_non_blank_str(object, AGENT_ID_FIELD)?,
        agent_type: optional_non_blank_str(object, AGENT_TYPE_FIELD)?,
    })
}

pub(super) fn parse_session_identity(object: &Map<String, Value>) -> Result<CodexSessionIdentity> {
    Ok(CodexSessionIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
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

pub(super) fn tolerated_model(object: &Map<String, Value>) -> Option<String> {
    object
        .get(MODEL_FIELD)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

pub(super) fn validation_error(detail: &str) -> String {
    format!("Invalid Codex hook event payload from STDIN: {detail}.")
}
