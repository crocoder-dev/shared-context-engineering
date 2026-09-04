#![allow(dead_code)]

pub(crate) mod state;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{Map, Value};

const HOOK_EVENT_NAME_FIELD: &str = "hook_event_name";
const SESSION_ID_FIELD: &str = "session_id";
const CWD_FIELD: &str = "cwd";
const AGENT_ID_FIELD: &str = "agent_id";
const TOOL_NAME_FIELD: &str = "tool_name";
const TOOL_USE_ID_FIELD: &str = "tool_use_id";
const TOOL_INPUT_FIELD: &str = "tool_input";
const RUN_IN_BACKGROUND_FIELD: &str = "run_in_background";
const PROMPT_ID_FIELD: &str = "prompt_id";
const AGENT_TYPE_FIELD: &str = "agent_type";
const WORKTREE_PATH_FIELD: &str = "worktree_path";

const HOOK_EVENT_PRE_TOOL_USE: &str = "PreToolUse";
const HOOK_EVENT_POST_TOOL_USE: &str = "PostToolUse";
const HOOK_EVENT_POST_TOOL_USE_FAILURE: &str = "PostToolUseFailure";
const HOOK_EVENT_PERMISSION_DENIED: &str = "PermissionDenied";
const HOOK_EVENT_STOP: &str = "Stop";
const HOOK_EVENT_STOP_FAILURE: &str = "StopFailure";
const HOOK_EVENT_USER_PROMPT_SUBMIT: &str = "UserPromptSubmit";
const HOOK_EVENT_SUBAGENT_STOP: &str = "SubagentStop";
const HOOK_EVENT_SESSION_END: &str = "SessionEnd";
const HOOK_EVENT_WORKTREE_REMOVE: &str = "WorktreeRemove";
const HOOK_EVENT_SESSION_START: &str = "SessionStart";
const HOOK_EVENT_SUBAGENT_START: &str = "SubagentStart";

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

const DELEGATION_TOOL_NAME: &str = "Agent";
const KNOWN_READ_ONLY_TOOL_NAMES: &[&str] = &[
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

const BASH_TOOL_NAME: &str = "Bash";
const POWERSHELL_TOOL_NAME: &str = "PowerShell";

pub(crate) fn is_explicit_background_shell(tool_name: &str, run_in_background: bool) -> bool {
    run_in_background && (tool_name == BASH_TOOL_NAME || tool_name == POWERSHELL_TOOL_NAME)
}

const CLAUDE_SCOPE_ID_SCHEME: &str = "cc-tool-v1";

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

fn parse_tool_identity(object: &Map<String, Value>) -> Result<ClaudeToolIdentity> {
    Ok(ClaudeToolIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
        tool_name: required_non_blank_str(object, TOOL_NAME_FIELD)?,
        tool_use_id: required_non_blank_str(object, TOOL_USE_ID_FIELD)?,
        agent_id: optional_non_blank_str(object, AGENT_ID_FIELD)?,
    })
}

fn parse_pre_tool_use(object: &Map<String, Value>) -> Result<ClaudeToolExecution> {
    Ok(ClaudeToolExecution {
        identity: parse_tool_identity(object)?,
        prompt_id: optional_non_blank_str(object, PROMPT_ID_FIELD)?,
        agent_type: optional_non_blank_str(object, AGENT_TYPE_FIELD)?,
        run_in_background: parse_run_in_background(object)?,
    })
}

fn parse_run_in_background(object: &Map<String, Value>) -> Result<bool> {
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

fn parse_session_identity(object: &Map<String, Value>) -> Result<ClaudeSessionIdentity> {
    Ok(ClaudeSessionIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
    })
}

fn parse_agent_identity(object: &Map<String, Value>) -> Result<ClaudeAgentIdentity> {
    Ok(ClaudeAgentIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
        agent_id: required_non_blank_str(object, AGENT_ID_FIELD)?,
    })
}

fn parse_worktree_remove(object: &Map<String, Value>) -> Result<ClaudeWorktreeRemove> {
    Ok(ClaudeWorktreeRemove {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        worktree_path: required_non_blank_str(object, WORKTREE_PATH_FIELD)?,
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
    format!("Invalid Claude hook event payload from STDIN: {detail}.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pre_tool_use_json(overrides: &[(&str, Value)]) -> String {
        let mut object = serde_json::Map::new();
        object.insert(
            HOOK_EVENT_NAME_FIELD.to_string(),
            Value::String(HOOK_EVENT_PRE_TOOL_USE.to_string()),
        );
        object.insert(
            SESSION_ID_FIELD.to_string(),
            Value::String("session-1".to_string()),
        );
        object.insert(
            CWD_FIELD.to_string(),
            Value::String("/repo/checkout".to_string()),
        );
        object.insert(
            TOOL_NAME_FIELD.to_string(),
            Value::String("Write".to_string()),
        );
        object.insert(
            TOOL_USE_ID_FIELD.to_string(),
            Value::String("toolu_1".to_string()),
        );
        for (field, value) in overrides {
            object.insert((*field).to_string(), value.clone());
        }
        Value::Object(object).to_string()
    }

    fn identity(session_id: &str, agent_id: Option<&str>, tool_use_id: &str) -> AttemptKey {
        AttemptKey {
            session_id: session_id.to_string(),
            agent_id: agent_id.map(str::to_string),
            tool_use_id: tool_use_id.to_string(),
        }
    }

    #[test]
    fn pre_tool_use_parses_required_and_optional_fields() {
        let payload = pre_tool_use_json(&[
            (AGENT_ID_FIELD, Value::String("agent-1".to_string())),
            (PROMPT_ID_FIELD, Value::String("prompt-1".to_string())),
            (
                AGENT_TYPE_FIELD,
                Value::String("general-purpose".to_string()),
            ),
        ]);

        let event = parse_claude_hook_event(&payload).expect("valid PreToolUse parses");
        let ClaudeHookEvent::PreToolUse(execution) = event else {
            panic!("expected PreToolUse");
        };

        assert_eq!(execution.identity.session_id, "session-1");
        assert_eq!(execution.identity.cwd, "/repo/checkout");
        assert_eq!(execution.identity.tool_name, "Write");
        assert_eq!(execution.identity.tool_use_id, "toolu_1");
        assert_eq!(execution.identity.agent_id.as_deref(), Some("agent-1"));
        assert_eq!(execution.prompt_id.as_deref(), Some("prompt-1"));
        assert_eq!(execution.agent_type.as_deref(), Some("general-purpose"));
        assert!(!execution.run_in_background);
    }

    #[test]
    fn pre_tool_use_agent_id_absent_means_main_thread() {
        let payload = pre_tool_use_json(&[]);

        let event = parse_claude_hook_event(&payload).unwrap();
        let ClaudeHookEvent::PreToolUse(execution) = event else {
            panic!("expected PreToolUse");
        };

        assert_eq!(execution.identity.agent_id, None);
        assert!(!execution.identity.is_subagent());
    }

    #[test]
    fn pre_tool_use_agent_id_present_means_subagent() {
        let payload = pre_tool_use_json(&[(AGENT_ID_FIELD, Value::String("agent-1".to_string()))]);

        let event = parse_claude_hook_event(&payload).unwrap();
        let ClaudeHookEvent::PreToolUse(execution) = event else {
            panic!("expected PreToolUse");
        };

        assert!(execution.identity.is_subagent());
    }

    #[test]
    fn pre_tool_use_prompt_id_and_agent_type_are_optional() {
        let payload = pre_tool_use_json(&[]);

        let event = parse_claude_hook_event(&payload).unwrap();
        let ClaudeHookEvent::PreToolUse(execution) = event else {
            panic!("expected PreToolUse");
        };

        assert_eq!(execution.prompt_id, None);
        assert_eq!(execution.agent_type, None);
    }

    #[test]
    fn missing_required_fields_are_rejected_without_fabricating_identity() {
        for field in [
            SESSION_ID_FIELD,
            CWD_FIELD,
            TOOL_NAME_FIELD,
            TOOL_USE_ID_FIELD,
        ] {
            let mut object: serde_json::Map<String, Value> =
                serde_json::from_str(&pre_tool_use_json(&[])).unwrap();
            object.remove(field);
            let payload = Value::Object(object).to_string();

            let error = parse_claude_hook_event(&payload).unwrap_err();
            assert!(
                error.to_string().contains(&format!("'{field}'")),
                "expected missing-field error to name '{field}', got: {error}"
            );
        }
    }

    #[test]
    fn wrong_type_required_field_is_rejected() {
        let payload = pre_tool_use_json(&[(SESSION_ID_FIELD, Value::from(42))]);

        let error = parse_claude_hook_event(&payload).unwrap_err();
        assert!(error.to_string().contains("session_id"));
    }

    #[test]
    fn empty_string_required_field_is_rejected() {
        let payload = pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String(String::new()))]);

        let error = parse_claude_hook_event(&payload).unwrap_err();
        assert!(error.to_string().contains("non-blank"));
    }

    #[test]
    fn wrong_type_optional_field_is_rejected() {
        let payload = pre_tool_use_json(&[(PROMPT_ID_FIELD, Value::from(1))]);

        let error = parse_claude_hook_event(&payload).unwrap_err();
        assert!(error.to_string().contains("prompt_id"));
    }

    #[test]
    fn empty_optional_field_is_rejected() {
        let payload = pre_tool_use_json(&[(AGENT_ID_FIELD, Value::String(String::new()))]);

        let error = parse_claude_hook_event(&payload).unwrap_err();
        assert!(error.to_string().contains("agent_id"));
    }

    #[test]
    fn null_optional_field_is_none() {
        let payload = pre_tool_use_json(&[(AGENT_ID_FIELD, Value::Null)]);

        let event = parse_claude_hook_event(&payload).unwrap();
        let ClaudeHookEvent::PreToolUse(execution) = event else {
            panic!("expected PreToolUse");
        };
        assert_eq!(execution.identity.agent_id, None);
    }

    #[test]
    fn empty_payload_is_rejected() {
        let error = parse_claude_hook_event("").unwrap_err();
        assert!(error.to_string().contains("empty payload"));

        let error = parse_claude_hook_event("   ").unwrap_err();
        assert!(error.to_string().contains("empty payload"));
    }

    #[test]
    fn malformed_json_is_rejected() {
        let error = parse_claude_hook_event("{not json").unwrap_err();
        assert!(error.to_string().contains("valid JSON"));
    }

    #[test]
    fn non_object_json_is_rejected() {
        let error = parse_claude_hook_event("[1, 2, 3]").unwrap_err();
        assert!(error.to_string().contains("JSON object"));
    }

    #[test]
    fn unsupported_hook_event_name_is_rejected() {
        let payload = pre_tool_use_json(&[(
            HOOK_EVENT_NAME_FIELD,
            Value::String("PostToolBatch".to_string()),
        )]);

        let error = parse_claude_hook_event(&payload).unwrap_err();
        assert!(error.to_string().contains("unsupported hook_event_name"));
    }

    #[test]
    fn post_tool_use_and_failure_and_permission_denied_share_tool_identity_shape() {
        for event_name in [
            HOOK_EVENT_POST_TOOL_USE,
            HOOK_EVENT_POST_TOOL_USE_FAILURE,
            HOOK_EVENT_PERMISSION_DENIED,
        ] {
            let payload = pre_tool_use_json(&[(
                HOOK_EVENT_NAME_FIELD,
                Value::String(event_name.to_string()),
            )]);

            let event = parse_claude_hook_event(&payload).unwrap();
            let identity = match event {
                ClaudeHookEvent::PostToolUse(identity)
                | ClaudeHookEvent::PostToolUseFailure(identity)
                | ClaudeHookEvent::PermissionDenied(identity) => identity,
                other => panic!("expected a tool-identity event, got {other:?}"),
            };
            assert_eq!(identity.session_id, "session-1");
            assert_eq!(identity.tool_use_id, "toolu_1");
        }
    }

    #[test]
    fn session_scoped_lifecycle_events_parse_session_identity() {
        for event_name in [
            HOOK_EVENT_STOP,
            HOOK_EVENT_STOP_FAILURE,
            HOOK_EVENT_USER_PROMPT_SUBMIT,
            HOOK_EVENT_SESSION_END,
        ] {
            let mut object = serde_json::Map::new();
            object.insert(
                HOOK_EVENT_NAME_FIELD.to_string(),
                Value::String(event_name.to_string()),
            );
            object.insert(
                SESSION_ID_FIELD.to_string(),
                Value::String("session-1".to_string()),
            );
            object.insert(CWD_FIELD.to_string(), Value::String("/repo".to_string()));
            let payload = Value::Object(object).to_string();

            let event = parse_claude_hook_event(&payload).unwrap();
            let identity = match event {
                ClaudeHookEvent::Stop(identity)
                | ClaudeHookEvent::StopFailure(identity)
                | ClaudeHookEvent::UserPromptSubmit(identity)
                | ClaudeHookEvent::SessionEnd(identity) => identity,
                other => panic!("expected a session-identity event, got {other:?}"),
            };
            assert_eq!(identity.session_id, "session-1");
            assert_eq!(identity.cwd, "/repo");
        }
    }

    #[test]
    fn subagent_stop_requires_agent_id() {
        let mut object = serde_json::Map::new();
        object.insert(
            HOOK_EVENT_NAME_FIELD.to_string(),
            Value::String(HOOK_EVENT_SUBAGENT_STOP.to_string()),
        );
        object.insert(
            SESSION_ID_FIELD.to_string(),
            Value::String("session-1".to_string()),
        );
        object.insert(CWD_FIELD.to_string(), Value::String("/repo".to_string()));
        let payload = Value::Object(object).to_string();

        let error = parse_claude_hook_event(&payload).unwrap_err();
        assert!(error.to_string().contains("agent_id"));
    }

    #[test]
    fn subagent_stop_parses_agent_identity() {
        let mut object = serde_json::Map::new();
        object.insert(
            HOOK_EVENT_NAME_FIELD.to_string(),
            Value::String(HOOK_EVENT_SUBAGENT_STOP.to_string()),
        );
        object.insert(
            SESSION_ID_FIELD.to_string(),
            Value::String("session-1".to_string()),
        );
        object.insert(CWD_FIELD.to_string(), Value::String("/repo".to_string()));
        object.insert(
            AGENT_ID_FIELD.to_string(),
            Value::String("agent-1".to_string()),
        );
        let payload = Value::Object(object).to_string();

        let event = parse_claude_hook_event(&payload).unwrap();
        let ClaudeHookEvent::SubagentStop(identity) = event else {
            panic!("expected SubagentStop");
        };
        assert_eq!(identity.agent_id, "agent-1");
    }

    #[test]
    fn worktree_remove_requires_worktree_path_not_cwd() {
        let mut object = serde_json::Map::new();
        object.insert(
            HOOK_EVENT_NAME_FIELD.to_string(),
            Value::String(HOOK_EVENT_WORKTREE_REMOVE.to_string()),
        );
        object.insert(
            SESSION_ID_FIELD.to_string(),
            Value::String("session-1".to_string()),
        );
        object.insert(
            WORKTREE_PATH_FIELD.to_string(),
            Value::String("/repo/.claude/worktrees/agent-1".to_string()),
        );
        let payload = Value::Object(object).to_string();

        let event = parse_claude_hook_event(&payload).unwrap();
        let ClaudeHookEvent::WorktreeRemove(worktree_remove) = event else {
            panic!("expected WorktreeRemove");
        };
        assert_eq!(worktree_remove.session_id, "session-1");
        assert_eq!(
            worktree_remove.worktree_path,
            "/repo/.claude/worktrees/agent-1"
        );
    }

    #[test]
    fn session_start_and_subagent_start_establish_no_scope_payload() {
        for event_name in [HOOK_EVENT_SESSION_START, HOOK_EVENT_SUBAGENT_START] {
            let mut object = serde_json::Map::new();
            object.insert(
                HOOK_EVENT_NAME_FIELD.to_string(),
                Value::String(event_name.to_string()),
            );
            object.insert(
                SESSION_ID_FIELD.to_string(),
                Value::String("session-1".to_string()),
            );
            let payload = Value::Object(object).to_string();

            let event = parse_claude_hook_event(&payload).unwrap();
            assert!(matches!(
                event,
                ClaudeHookEvent::SessionStart | ClaudeHookEvent::SubagentStart
            ));
        }
    }

    #[test]
    fn run_in_background_true_is_parsed() {
        let payload = pre_tool_use_json(&[(
            TOOL_INPUT_FIELD,
            serde_json::json!({ "run_in_background": true }),
        )]);

        let event = parse_claude_hook_event(&payload).unwrap();
        let ClaudeHookEvent::PreToolUse(execution) = event else {
            panic!("expected PreToolUse");
        };
        assert!(execution.run_in_background);
    }

    #[test]
    fn run_in_background_false_is_parsed() {
        let payload = pre_tool_use_json(&[(
            TOOL_INPUT_FIELD,
            serde_json::json!({ "run_in_background": false }),
        )]);

        let event = parse_claude_hook_event(&payload).unwrap();
        let ClaudeHookEvent::PreToolUse(execution) = event else {
            panic!("expected PreToolUse");
        };
        assert!(!execution.run_in_background);
    }

    #[test]
    fn run_in_background_absent_defaults_to_false() {
        let payload = pre_tool_use_json(&[(
            TOOL_INPUT_FIELD,
            serde_json::json!({ "command": "echo hi" }),
        )]);

        let event = parse_claude_hook_event(&payload).unwrap();
        let ClaudeHookEvent::PreToolUse(execution) = event else {
            panic!("expected PreToolUse");
        };
        assert!(!execution.run_in_background);
    }

    #[test]
    fn tool_input_absent_defaults_run_in_background_to_false() {
        let mut object: serde_json::Map<String, Value> =
            serde_json::from_str(&pre_tool_use_json(&[])).unwrap();
        object.remove(TOOL_INPUT_FIELD);
        let payload = Value::Object(object).to_string();

        let event = parse_claude_hook_event(&payload).unwrap();
        let ClaudeHookEvent::PreToolUse(execution) = event else {
            panic!("expected PreToolUse");
        };
        assert!(!execution.run_in_background);
    }

    #[test]
    fn run_in_background_wrong_type_is_rejected() {
        let payload = pre_tool_use_json(&[(
            TOOL_INPUT_FIELD,
            serde_json::json!({ "run_in_background": "yes" }),
        )]);

        let error = parse_claude_hook_event(&payload).unwrap_err();
        assert!(error.to_string().contains("run_in_background"));
    }

    #[test]
    fn tool_input_wrong_type_is_rejected() {
        let payload = pre_tool_use_json(&[(TOOL_INPUT_FIELD, Value::String("nope".to_string()))]);

        let error = parse_claude_hook_event(&payload).unwrap_err();
        assert!(error.to_string().contains("tool_input"));
    }

    #[test]
    fn known_mutation_capable_tools_are_classified_mutation_capable() {
        for tool_name in [
            "Bash",
            "PowerShell",
            "Write",
            "Edit",
            "NotebookEdit",
            "MultiEdit",
        ] {
            assert_eq!(
                classify_tool(tool_name),
                ToolClassification::MutationCapable,
                "expected {tool_name} to be MutationCapable"
            );
        }
    }

    #[test]
    fn known_read_only_tools_are_classified_read_only() {
        for tool_name in [
            "Read",
            "Glob",
            "Grep",
            "WebFetch",
            "WebSearch",
            "AskUserQuestion",
        ] {
            assert_eq!(
                classify_tool(tool_name),
                ToolClassification::ReadOnly,
                "expected {tool_name} to be ReadOnly"
            );
        }
    }

    #[test]
    fn agent_is_classified_delegation() {
        assert_eq!(classify_tool("Agent"), ToolClassification::Delegation);
    }

    #[test]
    fn mcp_tools_are_classified_mutation_capable() {
        assert_eq!(
            classify_tool("mcp__claude-in-chrome__navigate"),
            ToolClassification::MutationCapable
        );
    }

    #[test]
    fn unknown_tool_names_are_conservatively_mutation_capable() {
        assert_eq!(
            classify_tool("SomeBrandNewTool"),
            ToolClassification::MutationCapable
        );
    }

    const PROBE14_BASH_RUN_IN_BACKGROUND_TRUE: &str =
        include_str!("fixtures/probe14-run-in-background-true.pre_tool_use.json");
    const PROBE15_BASH_RUN_IN_BACKGROUND_FALSE: &str =
        include_str!("fixtures/probe15-run-in-background-false-hard-gate.pre_tool_use.json");

    fn parsed_pre_tool_use(payload: &str) -> ClaudeToolExecution {
        let event = parse_claude_hook_event(payload).unwrap();
        let ClaudeHookEvent::PreToolUse(execution) = event else {
            panic!("expected PreToolUse");
        };
        execution
    }

    #[test]
    fn real_bash_run_in_background_true_fixture_is_explicit_background_shell() {
        let execution = parsed_pre_tool_use(PROBE14_BASH_RUN_IN_BACKGROUND_TRUE);

        assert_eq!(execution.identity.tool_name, "Bash");
        assert!(execution.run_in_background);
        assert!(is_explicit_background_shell(
            &execution.identity.tool_name,
            execution.run_in_background
        ));
    }

    #[test]
    fn real_bash_run_in_background_false_fixture_is_not_explicit_background_shell() {
        let execution = parsed_pre_tool_use(PROBE15_BASH_RUN_IN_BACKGROUND_FALSE);

        assert_eq!(execution.identity.tool_name, "Bash");
        assert!(!execution.run_in_background);
        assert!(!is_explicit_background_shell(
            &execution.identity.tool_name,
            execution.run_in_background
        ));
    }

    #[test]
    fn powershell_with_run_in_background_true_is_explicit_background_shell() {
        assert!(is_explicit_background_shell("PowerShell", true));
    }

    #[test]
    fn powershell_with_run_in_background_false_is_not_explicit_background_shell() {
        assert!(!is_explicit_background_shell("PowerShell", false));
    }

    #[test]
    fn write_with_run_in_background_true_is_not_explicit_background_shell() {
        assert!(!is_explicit_background_shell("Write", true));
    }

    #[test]
    fn same_attempt_seq_and_key_is_deterministic() {
        let key = identity("session-1", Some("agent-1"), "toolu_1");

        let first = format_claude_scope_id(3, &key);
        let second = format_claude_scope_id(3, &key);

        assert_eq!(
            first, second,
            "AC4: duplicate delivery must reuse the same ScopeId"
        );
        assert_eq!(
            claude_scope_start_event_id(&first),
            claude_scope_start_event_id(&second)
        );
    }

    #[test]
    fn fresh_attempt_seq_yields_a_new_scope_id() {
        let key = identity("session-1", None, "toolu_1");

        let first = format_claude_scope_id(1, &key);
        let second = format_claude_scope_id(2, &key);

        assert_ne!(
            first, second,
            "AC5: a fresh attempt_seq for the same tool_use_id must get a new ScopeId"
        );
    }

    #[test]
    fn main_and_distinct_agents_produce_distinct_scope_ids() {
        let main = identity("session-1", None, "toolu_1");
        let agent_a = identity("session-1", Some("A"), "toolu_1");
        let agent_b = identity("session-1", Some("B"), "toolu_1");

        let main_scope = format_claude_scope_id(1, &main);
        let scope_for_a = format_claude_scope_id(1, &agent_a);
        let scope_for_b = format_claude_scope_id(1, &agent_b);

        assert_ne!(
            main_scope, scope_for_a,
            "AC6: main vs agent_id=A must differ"
        );
        assert_ne!(
            main_scope, scope_for_b,
            "AC6: main vs agent_id=B must differ"
        );
        assert_ne!(
            scope_for_a, scope_for_b,
            "AC6: agent_id=A vs agent_id=B must differ"
        );
    }

    #[test]
    fn event_id_derivation_is_a_pure_function_of_scope_id() {
        let scope_id = format_claude_scope_id(7, &identity("session-1", None, "toolu_1"));

        assert_eq!(
            claude_scope_start_event_id(&scope_id),
            format!("{scope_id}|start")
        );
        assert_eq!(
            claude_scope_close_event_id(&scope_id),
            format!("{scope_id}|close")
        );
        assert_ne!(
            claude_scope_start_event_id(&scope_id),
            claude_scope_close_event_id(&scope_id)
        );
    }

    #[test]
    fn length_prefixing_disambiguates_delimiter_characters_inside_fields() {
        let tricky = identity(
            "sess|a=0:x|t=1:y",
            Some("agent|with|pipes"),
            "tool:with:colons",
        );

        let scope_id = format_claude_scope_id(1, &tricky);

        let agent_id = tricky.agent_id.as_deref().unwrap();
        let expected = format!(
            "cc-tool-v1|n=1|s={}:{}|a={}:{}|t={}:{}",
            tricky.session_id.len(),
            tricky.session_id,
            agent_id.len(),
            agent_id,
            tricky.tool_use_id.len(),
            tricky.tool_use_id,
        );

        assert_eq!(scope_id, expected);
    }

    #[test]
    fn attempt_key_projects_only_the_execution_key_fields() {
        let identity_a = ClaudeToolIdentity {
            session_id: "session-1".to_string(),
            cwd: "/repo".to_string(),
            agent_id: Some("agent-1".to_string()),
            tool_name: "Write".to_string(),
            tool_use_id: "toolu_1".to_string(),
        };
        let identity_b = ClaudeToolIdentity {
            tool_name: "Bash".to_string(),
            cwd: "/other".to_string(),
            ..identity_a.clone()
        };

        assert_eq!(
            identity_a.attempt_key(),
            identity_b.attempt_key(),
            "attempt_key must depend only on (session_id, agent_id, tool_use_id)"
        );
    }
}
