use anyhow::{anyhow, bail, Context, Result};
use serde_json::{Map, Value};

use crate::services::hooks::{
    normalize_opencode_model_id, prefixed_diff_trace_session_id, OPENCODE_TOOL_NAME,
};

pub(super) const HOOK_EVENT_NAME_FIELD: &str = "hook_event_name";
pub(super) const SESSION_ID_FIELD: &str = "session_id";
pub(super) const CALL_ID_FIELD: &str = "call_id";
pub(super) const CWD_FIELD: &str = "cwd";
pub(super) const TOOL_NAME_FIELD: &str = "tool_name";
pub(super) const MODEL_FIELD: &str = "model";

pub(super) const HOOK_EVENT_TOOL_EXECUTE_BEFORE: &str = "ToolExecuteBefore";
pub(super) const HOOK_EVENT_SHELL_ENV: &str = "ShellEnv";
pub(super) const HOOK_EVENT_TOOL_EXECUTE_AFTER: &str = "ToolExecuteAfter";
pub(super) const HOOK_EVENT_TOOL_ERROR: &str = "ToolError";
pub(super) const HOOK_EVENT_SESSION_IDLE: &str = "SessionIdle";
pub(super) const HOOK_EVENT_SESSION_ERROR: &str = "SessionError";
pub(super) const HOOK_EVENT_SESSION_DELETED: &str = "SessionDeleted";
pub(super) const HOOK_EVENT_SERVER_DISPOSED: &str = "ServerDisposed";

pub(super) const OPENCODE_TRACKED_TOOL_BASH: &str = "bash";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OpenCodeHookEvent {
    ToolExecuteBefore(OpenCodeToolExecution),
    ShellEnv(OpenCodeShellStart),
    ToolExecuteAfter(OpenCodeToolIdentity),
    ToolError(OpenCodeCallIdentity),
    SessionIdle(OpenCodeSessionIdentity),
    SessionError(OpenCodeSessionIdentity),
    SessionDeleted(OpenCodeSessionIdentity),
    ServerDisposed(OpenCodeWorkspaceIdentity),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenCodeCallIdentity {
    pub session_id: String,
    pub call_id: String,
    pub cwd: String,
    pub tool_name: String,
}

impl OpenCodeCallIdentity {
    pub(crate) fn attempt_key(&self) -> AttemptKey {
        AttemptKey {
            session_id: self.session_id.clone(),
            call_id: self.call_id.clone(),
        }
    }

    pub(crate) fn classification(&self) -> ToolClassification {
        classify_tool(&self.tool_name)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenCodeToolIdentity {
    pub session_id: String,
    pub call_id: String,
    pub cwd: String,
    pub tool_name: String,
}

impl OpenCodeToolIdentity {
    pub(crate) fn attempt_key(&self) -> AttemptKey {
        AttemptKey {
            session_id: self.session_id.clone(),
            call_id: self.call_id.clone(),
        }
    }

    pub(crate) fn classification(&self) -> ToolClassification {
        classify_tool(&self.tool_name)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenCodeToolExecution {
    pub identity: OpenCodeToolIdentity,
    pub model: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenCodeShellStart {
    pub session_id: String,
    pub call_id: String,
    pub cwd: String,
    pub model: Option<String>,
}

impl OpenCodeShellStart {
    pub(crate) fn attempt_key(&self) -> AttemptKey {
        AttemptKey {
            session_id: self.session_id.clone(),
            call_id: self.call_id.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenCodeSessionIdentity {
    pub session_id: String,
    pub cwd: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenCodeWorkspaceIdentity {
    pub cwd: String,
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct AttemptKey {
    pub session_id: String,
    pub call_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolClassification {
    TrackedMutation,
    Delegation,
    Untracked,
}

pub(super) const TRACKED_MUTATION_TOOL_NAMES: &[&str] = &["bash", "write", "edit", "apply_patch"];
pub(super) const DELEGATION_TOOL_NAMES: &[&str] = &["task"];

pub(crate) fn classify_tool(tool_name: &str) -> ToolClassification {
    if TRACKED_MUTATION_TOOL_NAMES.contains(&tool_name) {
        ToolClassification::TrackedMutation
    } else if DELEGATION_TOOL_NAMES.contains(&tool_name) {
        ToolClassification::Delegation
    } else {
        ToolClassification::Untracked
    }
}

pub(super) const OPENCODE_SCOPE_ID_SCHEME: &str = "oc-tool-v1";

pub(crate) fn format_opencode_scope_id(key: &AttemptKey) -> String {
    format!(
        "{OPENCODE_SCOPE_ID_SCHEME}|s={}:{}|c={}:{}",
        key.session_id.len(),
        key.session_id,
        key.call_id.len(),
        key.call_id,
    )
}

pub(crate) fn opencode_scope_start_event_id(scope_id: &str) -> String {
    format!("{scope_id}|start")
}

pub(crate) fn opencode_scope_close_event_id(scope_id: &str) -> String {
    format!("{scope_id}|close")
}

pub(super) const ACTOR_KIND_OPENCODE: &str = "opencode";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenCodeScopeProvenance {
    pub session_id: String,
    pub model_id: Option<String>,
}

pub(crate) fn opencode_scope_provenance(
    session_id: &str,
    model: Option<&str>,
) -> OpenCodeScopeProvenance {
    OpenCodeScopeProvenance {
        session_id: prefixed_diff_trace_session_id(OPENCODE_TOOL_NAME, session_id),
        model_id: model.and_then(normalize_opencode_model_id),
    }
}

pub(crate) fn parse_opencode_hook_event(stdin_payload: &str) -> Result<OpenCodeHookEvent> {
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
        HOOK_EVENT_TOOL_EXECUTE_BEFORE => {
            parse_tool_execution(object).map(OpenCodeHookEvent::ToolExecuteBefore)
        }
        HOOK_EVENT_SHELL_ENV => parse_shell_start(object).map(OpenCodeHookEvent::ShellEnv),
        HOOK_EVENT_TOOL_EXECUTE_AFTER => {
            parse_tool_identity(object).map(OpenCodeHookEvent::ToolExecuteAfter)
        }
        HOOK_EVENT_TOOL_ERROR => parse_call_identity(object).map(OpenCodeHookEvent::ToolError),
        HOOK_EVENT_SESSION_IDLE => {
            parse_session_identity(object).map(OpenCodeHookEvent::SessionIdle)
        }
        HOOK_EVENT_SESSION_ERROR => {
            parse_session_identity(object).map(OpenCodeHookEvent::SessionError)
        }
        HOOK_EVENT_SESSION_DELETED => {
            parse_session_identity(object).map(OpenCodeHookEvent::SessionDeleted)
        }
        HOOK_EVENT_SERVER_DISPOSED => {
            parse_workspace_identity(object).map(OpenCodeHookEvent::ServerDisposed)
        }
        other => bail!(validation_error(&format!(
            "unsupported hook_event_name '{other}'"
        ))),
    }
}

fn parse_tool_identity(object: &Map<String, Value>) -> Result<OpenCodeToolIdentity> {
    Ok(OpenCodeToolIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        call_id: required_non_blank_str(object, CALL_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
        tool_name: required_non_blank_str(object, TOOL_NAME_FIELD)?,
    })
}

fn parse_tool_execution(object: &Map<String, Value>) -> Result<OpenCodeToolExecution> {
    Ok(OpenCodeToolExecution {
        identity: parse_tool_identity(object)?,
        model: optional_non_blank_str(object, MODEL_FIELD)?,
    })
}

fn parse_shell_start(object: &Map<String, Value>) -> Result<OpenCodeShellStart> {
    Ok(OpenCodeShellStart {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        call_id: required_non_blank_str(object, CALL_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
        model: optional_non_blank_str(object, MODEL_FIELD)?,
    })
}

fn parse_call_identity(object: &Map<String, Value>) -> Result<OpenCodeCallIdentity> {
    Ok(OpenCodeCallIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        call_id: required_non_blank_str(object, CALL_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
        tool_name: required_non_blank_str(object, TOOL_NAME_FIELD)?,
    })
}

fn parse_session_identity(object: &Map<String, Value>) -> Result<OpenCodeSessionIdentity> {
    Ok(OpenCodeSessionIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
    })
}

fn parse_workspace_identity(object: &Map<String, Value>) -> Result<OpenCodeWorkspaceIdentity> {
    Ok(OpenCodeWorkspaceIdentity {
        cwd: required_non_blank_str(object, CWD_FIELD)?,
    })
}

fn required_field<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a Value> {
    object.get(field).ok_or_else(|| {
        anyhow!(validation_error(&format!(
            "missing required field '{field}'"
        )))
    })
}

fn required_str(object: &Map<String, Value>, field: &str) -> Result<String> {
    required_field(object, field)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| {
            anyhow!(validation_error(&format!(
                "field '{field}' must be a string"
            )))
        })
}

fn required_non_blank_str(object: &Map<String, Value>, field: &str) -> Result<String> {
    let value = required_str(object, field)?;
    if value.trim().is_empty() {
        bail!(validation_error(&format!(
            "field '{field}' must be a non-blank string"
        )));
    }
    Ok(value)
}

fn optional_non_blank_str(object: &Map<String, Value>, field: &str) -> Result<Option<String>> {
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

fn validation_error(detail: &str) -> String {
    format!("Invalid OpenCode hook event payload from STDIN: {detail}.")
}
