use anyhow::{anyhow, bail, Context, Result};
use serde_json::{Map, Value};

use crate::services::hooks::{normalize_pi_model_id, prefixed_diff_trace_session_id, PI_TOOL_NAME};

pub(super) const HOOK_EVENT_NAME_FIELD: &str = "hook_event_name";
pub(super) const SESSION_ID_FIELD: &str = "session_id";
pub(super) const TOOL_CALL_ID_FIELD: &str = "tool_call_id";
pub(super) const CWD_FIELD: &str = "cwd";
pub(super) const TOOL_NAME_FIELD: &str = "tool_name";
pub(super) const MODEL_FIELD: &str = "model";

pub(super) const HOOK_EVENT_TOOL_EXECUTION_START: &str = "ToolExecutionStart";
pub(super) const HOOK_EVENT_TOOL_CALL: &str = "ToolCall";
pub(super) const HOOK_EVENT_TOOL_RESULT: &str = "ToolResult";
pub(super) const HOOK_EVENT_TOOL_EXECUTION_END: &str = "ToolExecutionEnd";
pub(super) const HOOK_EVENT_TOOL_EXECUTION_ABANDON: &str = "ToolExecutionAbandon";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PiHookEvent {
    ExecutionStart(PiToolIdentity),
    Call(PiToolCall),
    Executed(PiToolIdentity),
    ExecutionEnd(PiToolIdentity),
    ExecutionAbandon(PiToolIdentity),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PiToolIdentity {
    pub session_id: String,
    pub tool_call_id: String,
    pub cwd: String,
    pub tool_name: String,
}

impl PiToolIdentity {
    pub(crate) fn attempt_key(&self) -> AttemptKey {
        AttemptKey {
            session_id: self.session_id.clone(),
            tool_call_id: self.tool_call_id.clone(),
        }
    }

    pub(crate) fn classification(&self) -> ToolClassification {
        classify_tool(&self.tool_name)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PiToolCall {
    pub identity: PiToolIdentity,
    pub model: Option<String>,
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct AttemptKey {
    pub session_id: String,
    pub tool_call_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolClassification {
    TrackedMutation,
    Untracked,
}

pub(super) const TRACKED_MUTATION_TOOL_NAMES: &[&str] = &["bash", "edit", "write"];

pub(crate) fn classify_tool(tool_name: &str) -> ToolClassification {
    if TRACKED_MUTATION_TOOL_NAMES.contains(&tool_name) {
        ToolClassification::TrackedMutation
    } else {
        ToolClassification::Untracked
    }
}

pub(super) const PI_SCOPE_ID_SCHEME: &str = "pi-tool-v1";

pub(crate) fn format_pi_scope_id(key: &AttemptKey, attempt_seq: u64) -> String {
    format!(
        "{PI_SCOPE_ID_SCHEME}|n={attempt_seq}|s={}:{}|c={}:{}",
        key.session_id.len(),
        key.session_id,
        key.tool_call_id.len(),
        key.tool_call_id,
    )
}

pub(crate) fn pi_scope_start_event_id(scope_id: &str) -> String {
    format!("{scope_id}|start")
}

pub(crate) fn pi_scope_close_event_id(scope_id: &str) -> String {
    format!("{scope_id}|close")
}

pub(super) const ACTOR_KIND_PI: &str = "pi";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PiScopeProvenance {
    pub session_id: String,
    pub model_id: Option<String>,
}

pub(crate) fn pi_scope_provenance(session_id: &str, model: Option<&str>) -> PiScopeProvenance {
    PiScopeProvenance {
        session_id: prefixed_diff_trace_session_id(PI_TOOL_NAME, session_id),
        model_id: model.and_then(normalize_pi_model_id),
    }
}

pub(crate) fn parse_pi_hook_event(stdin_payload: &str) -> Result<PiHookEvent> {
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
        HOOK_EVENT_TOOL_EXECUTION_START => {
            parse_tool_identity(object).map(PiHookEvent::ExecutionStart)
        }
        HOOK_EVENT_TOOL_CALL => parse_tool_call(object).map(PiHookEvent::Call),
        HOOK_EVENT_TOOL_RESULT => parse_tool_identity(object).map(PiHookEvent::Executed),
        HOOK_EVENT_TOOL_EXECUTION_END => parse_tool_identity(object).map(PiHookEvent::ExecutionEnd),
        HOOK_EVENT_TOOL_EXECUTION_ABANDON => {
            parse_tool_identity(object).map(PiHookEvent::ExecutionAbandon)
        }
        other => bail!(validation_error(&format!(
            "unsupported hook_event_name '{other}'"
        ))),
    }
}

pub(super) fn parse_tool_identity(object: &Map<String, Value>) -> Result<PiToolIdentity> {
    Ok(PiToolIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        tool_call_id: required_non_blank_str(object, TOOL_CALL_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
        tool_name: required_non_blank_str(object, TOOL_NAME_FIELD)?,
    })
}

pub(super) fn parse_tool_call(object: &Map<String, Value>) -> Result<PiToolCall> {
    Ok(PiToolCall {
        identity: parse_tool_identity(object)?,
        model: optional_non_blank_str(object, MODEL_FIELD)?,
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
    format!("Invalid Pi hook event payload from STDIN: {detail}.")
}
