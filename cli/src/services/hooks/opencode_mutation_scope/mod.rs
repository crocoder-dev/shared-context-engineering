#![allow(dead_code)]

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{Map, Value};

use crate::services::hooks::{
    normalize_opencode_model_id, prefixed_diff_trace_session_id, OPENCODE_TOOL_NAME,
};
use crate::services::observability::traits::Logger;

const HOOK_EVENT_NAME_FIELD: &str = "hook_event_name";
const SESSION_ID_FIELD: &str = "session_id";
const CALL_ID_FIELD: &str = "call_id";
const CWD_FIELD: &str = "cwd";
const TOOL_NAME_FIELD: &str = "tool_name";
const MODEL_FIELD: &str = "model";

const HOOK_EVENT_TOOL_EXECUTE_BEFORE: &str = "ToolExecuteBefore";
const HOOK_EVENT_SHELL_ENV: &str = "ShellEnv";
const HOOK_EVENT_TOOL_EXECUTE_AFTER: &str = "ToolExecuteAfter";
const HOOK_EVENT_TOOL_ERROR: &str = "ToolError";
const HOOK_EVENT_SESSION_IDLE: &str = "SessionIdle";
const HOOK_EVENT_SESSION_ERROR: &str = "SessionError";
const HOOK_EVENT_SESSION_DELETED: &str = "SessionDeleted";
const HOOK_EVENT_SERVER_DISPOSED: &str = "ServerDisposed";

const OPENCODE_TRACKED_TOOL_BASH: &str = "bash";

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
}

impl OpenCodeCallIdentity {
    pub(crate) fn attempt_key(&self) -> AttemptKey {
        AttemptKey {
            session_id: self.session_id.clone(),
            call_id: self.call_id.clone(),
        }
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

const TRACKED_MUTATION_TOOL_NAMES: &[&str] = &["bash", "write", "edit", "apply_patch"];
const DELEGATION_TOOL_NAMES: &[&str] = &["task"];

pub(crate) fn classify_tool(tool_name: &str) -> ToolClassification {
    if TRACKED_MUTATION_TOOL_NAMES.contains(&tool_name) {
        ToolClassification::TrackedMutation
    } else if DELEGATION_TOOL_NAMES.contains(&tool_name) {
        ToolClassification::Delegation
    } else {
        ToolClassification::Untracked
    }
}

const OPENCODE_SCOPE_ID_SCHEME: &str = "oc-tool-v1";

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

const ACTOR_KIND_OPENCODE: &str = "opencode";

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

pub(crate) fn run_opencode_mutation_scope_subcommand(
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let stdin_payload = super::read_hook_stdin()?;
    run_opencode_mutation_scope_from_payload(&stdin_payload, logger)
}

pub(crate) fn run_opencode_mutation_scope_from_payload(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let _ = logger;
    parse_opencode_hook_event(stdin_payload)?;
    Ok(String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_event_json(hook_event_name: &str, overrides: &[(&str, Value)]) -> String {
        let mut object = Map::new();
        object.insert(
            HOOK_EVENT_NAME_FIELD.to_string(),
            Value::String(hook_event_name.to_string()),
        );
        object.insert(
            SESSION_ID_FIELD.to_string(),
            Value::String("ses_main".to_string()),
        );
        object.insert(
            CALL_ID_FIELD.to_string(),
            Value::String("call_1".to_string()),
        );
        object.insert(
            CWD_FIELD.to_string(),
            Value::String("/repo/checkout".to_string()),
        );
        object.insert(
            TOOL_NAME_FIELD.to_string(),
            Value::String("write".to_string()),
        );
        for (field, value) in overrides {
            object.insert((*field).to_string(), value.clone());
        }
        Value::Object(object).to_string()
    }

    fn key(session_id: &str, call_id: &str) -> AttemptKey {
        AttemptKey {
            session_id: session_id.to_string(),
            call_id: call_id.to_string(),
        }
    }

    fn tool_execution(payload: &str) -> OpenCodeToolExecution {
        match parse_opencode_hook_event(payload).expect("valid ToolExecuteBefore parses") {
            OpenCodeHookEvent::ToolExecuteBefore(execution) => execution,
            other => panic!("expected ToolExecuteBefore, got {other:?}"),
        }
    }

    #[test]
    fn empty_payload_is_rejected() {
        let error = parse_opencode_hook_event("   ").unwrap_err().to_string();
        assert_eq!(
            error,
            "Invalid OpenCode hook event payload from STDIN: expected a JSON object, got an empty payload."
        );
    }

    #[test]
    fn non_object_json_is_rejected() {
        for payload in ["[]", "\"ToolExecuteBefore\"", "42", "null"] {
            let error = parse_opencode_hook_event(payload).unwrap_err().to_string();
            assert!(
                error.contains("expected a JSON object"),
                "payload {payload:?} produced {error:?}"
            );
        }
    }

    #[test]
    fn invalid_json_is_rejected() {
        let error = parse_opencode_hook_event("{not json")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Invalid OpenCode hook event payload from STDIN: expected valid JSON"),
            "{error:?}"
        );
    }

    #[test]
    fn unsupported_hook_event_name_is_rejected() {
        for name in ["PreToolUse", "ToolExecute", "chat.params", ""] {
            let payload = tool_event_json(name, &[]);
            let error = parse_opencode_hook_event(&payload).unwrap_err().to_string();
            assert!(
                error.contains("hook_event_name"),
                "name {name:?} produced {error:?}"
            );
        }
    }

    #[test]
    fn missing_required_fields_are_rejected_without_fabricating_identity() {
        for field in [SESSION_ID_FIELD, CALL_ID_FIELD, CWD_FIELD, TOOL_NAME_FIELD] {
            let mut object: Map<String, Value> =
                serde_json::from_str(&tool_event_json(HOOK_EVENT_TOOL_EXECUTE_BEFORE, &[]))
                    .unwrap();
            object.remove(field);
            let payload = Value::Object(object).to_string();

            let error = parse_opencode_hook_event(&payload).unwrap_err().to_string();
            assert!(
                error.contains(&format!("'{field}'")),
                "missing {field} produced {error:?}"
            );
        }
    }

    #[test]
    fn blank_required_fields_are_rejected() {
        for field in [SESSION_ID_FIELD, CALL_ID_FIELD, CWD_FIELD, TOOL_NAME_FIELD] {
            let payload = tool_event_json(
                HOOK_EVENT_TOOL_EXECUTE_BEFORE,
                &[(field, Value::String("   ".to_string()))],
            );
            let error = parse_opencode_hook_event(&payload).unwrap_err().to_string();
            assert!(
                error.contains(&format!("field '{field}' must be a non-blank string")),
                "blank {field} produced {error:?}"
            );
        }
    }

    #[test]
    fn wrong_typed_fields_are_rejected() {
        let payload = tool_event_json(
            HOOK_EVENT_TOOL_EXECUTE_BEFORE,
            &[(CALL_ID_FIELD, Value::Bool(true))],
        );
        let error = parse_opencode_hook_event(&payload).unwrap_err().to_string();
        assert!(
            error.contains("field 'call_id' must be a string"),
            "{error:?}"
        );
    }

    #[test]
    fn wrong_typed_optional_model_is_rejected() {
        let payload = tool_event_json(
            HOOK_EVENT_TOOL_EXECUTE_BEFORE,
            &[(MODEL_FIELD, Value::Bool(false))],
        );
        let error = parse_opencode_hook_event(&payload).unwrap_err().to_string();
        assert!(
            error.contains("field 'model' must be null, absent, or a non-blank string"),
            "{error:?}"
        );
    }

    #[test]
    fn tool_execute_before_parses_identity_and_model() {
        let execution = tool_execution(&tool_event_json(
            HOOK_EVENT_TOOL_EXECUTE_BEFORE,
            &[
                (TOOL_NAME_FIELD, Value::String("edit".to_string())),
                (
                    MODEL_FIELD,
                    Value::String("opencode/big-pickle".to_string()),
                ),
            ],
        ));
        assert_eq!(execution.identity.session_id, "ses_main");
        assert_eq!(execution.identity.call_id, "call_1");
        assert_eq!(execution.identity.tool_name, "edit");
        assert_eq!(execution.model.as_deref(), Some("opencode/big-pickle"));
        assert_eq!(
            execution.identity.classification(),
            ToolClassification::TrackedMutation
        );
    }

    #[test]
    fn tool_execute_before_model_is_optional() {
        let execution = tool_execution(&tool_event_json(HOOK_EVENT_TOOL_EXECUTE_BEFORE, &[]));
        assert_eq!(execution.model, None);
    }

    #[test]
    fn shell_env_parses_without_a_tool_name() {
        let mut object = Map::new();
        object.insert(
            HOOK_EVENT_NAME_FIELD.to_string(),
            Value::String(HOOK_EVENT_SHELL_ENV.to_string()),
        );
        object.insert(
            SESSION_ID_FIELD.to_string(),
            Value::String("ses_main".to_string()),
        );
        object.insert(
            CALL_ID_FIELD.to_string(),
            Value::String("call_bash".to_string()),
        );
        object.insert(CWD_FIELD.to_string(), Value::String("/repo".to_string()));
        object.insert(
            MODEL_FIELD.to_string(),
            Value::String("opencode/big-pickle".to_string()),
        );
        let payload = Value::Object(object).to_string();

        let OpenCodeHookEvent::ShellEnv(shell) = parse_opencode_hook_event(&payload).unwrap()
        else {
            panic!("expected ShellEnv");
        };
        assert_eq!(shell.call_id, "call_bash");
        assert_eq!(shell.model.as_deref(), Some("opencode/big-pickle"));
        assert_eq!(shell.attempt_key(), key("ses_main", "call_bash"));
    }

    #[test]
    fn tool_execute_after_parses_tool_identity() {
        let OpenCodeHookEvent::ToolExecuteAfter(identity) =
            parse_opencode_hook_event(&tool_event_json(HOOK_EVENT_TOOL_EXECUTE_AFTER, &[]))
                .unwrap()
        else {
            panic!("expected ToolExecuteAfter");
        };
        assert_eq!(identity.attempt_key(), key("ses_main", "call_1"));
        assert_eq!(identity.tool_name, "write");
    }

    #[test]
    fn terminal_events_parse_their_minimal_identity() {
        for name in [
            HOOK_EVENT_SESSION_IDLE,
            HOOK_EVENT_SESSION_ERROR,
            HOOK_EVENT_SESSION_DELETED,
        ] {
            let mut object = Map::new();
            object.insert(
                HOOK_EVENT_NAME_FIELD.to_string(),
                Value::String(name.to_string()),
            );
            object.insert(
                SESSION_ID_FIELD.to_string(),
                Value::String("ses_main".to_string()),
            );
            object.insert(CWD_FIELD.to_string(), Value::String("/repo".to_string()));
            let payload = Value::Object(object).to_string();

            let event = parse_opencode_hook_event(&payload).unwrap();
            let identity = match event {
                OpenCodeHookEvent::SessionIdle(identity)
                | OpenCodeHookEvent::SessionError(identity)
                | OpenCodeHookEvent::SessionDeleted(identity) => identity,
                other => panic!("expected a session-identity event, got {other:?}"),
            };
            assert_eq!(identity.session_id, "ses_main");
            assert_eq!(identity.cwd, "/repo");
        }

        let mut object = Map::new();
        object.insert(
            HOOK_EVENT_NAME_FIELD.to_string(),
            Value::String(HOOK_EVENT_TOOL_ERROR.to_string()),
        );
        object.insert(
            SESSION_ID_FIELD.to_string(),
            Value::String("ses_main".to_string()),
        );
        object.insert(
            CALL_ID_FIELD.to_string(),
            Value::String("call_1".to_string()),
        );
        object.insert(CWD_FIELD.to_string(), Value::String("/repo".to_string()));
        let OpenCodeHookEvent::ToolError(identity) =
            parse_opencode_hook_event(&Value::Object(object).to_string()).unwrap()
        else {
            panic!("expected ToolError");
        };
        assert_eq!(identity.attempt_key(), key("ses_main", "call_1"));

        let mut object = Map::new();
        object.insert(
            HOOK_EVENT_NAME_FIELD.to_string(),
            Value::String(HOOK_EVENT_SERVER_DISPOSED.to_string()),
        );
        object.insert(CWD_FIELD.to_string(), Value::String("/repo".to_string()));
        let OpenCodeHookEvent::ServerDisposed(workspace) =
            parse_opencode_hook_event(&Value::Object(object).to_string()).unwrap()
        else {
            panic!("expected ServerDisposed");
        };
        assert_eq!(workspace.cwd, "/repo");
    }

    #[test]
    fn classification_table() {
        let cases: &[(&str, ToolClassification)] = &[
            ("bash", ToolClassification::TrackedMutation),
            ("write", ToolClassification::TrackedMutation),
            ("edit", ToolClassification::TrackedMutation),
            ("apply_patch", ToolClassification::TrackedMutation),
            ("task", ToolClassification::Delegation),
            ("read", ToolClassification::Untracked),
            ("glob", ToolClassification::Untracked),
            ("grep", ToolClassification::Untracked),
            ("webfetch", ToolClassification::Untracked),
            ("websearch", ToolClassification::Untracked),
            ("todowrite", ToolClassification::Untracked),
            ("probe_mutate", ToolClassification::Untracked),
            (
                "brave-search_brave_web_search",
                ToolClassification::Untracked,
            ),
            ("Bash", ToolClassification::Untracked),
            ("some_future_opencode_tool", ToolClassification::Untracked),
            ("", ToolClassification::Untracked),
        ];
        for (tool_name, expected) in cases {
            assert_eq!(
                classify_tool(tool_name),
                *expected,
                "classify_tool({tool_name:?})"
            );
        }
    }

    #[test]
    fn classification_is_total_and_single_valued() {
        for tool_name in ["bash", "write", "edit", "apply_patch", "task", "read", "x"] {
            let _: ToolClassification = classify_tool(tool_name);
        }
    }

    #[test]
    fn scope_id_is_deterministic_for_the_same_key() {
        let k = key("ses_main", "call_1");
        assert_eq!(format_opencode_scope_id(&k), format_opencode_scope_id(&k));

        let scope_id = format_opencode_scope_id(&k);
        assert_eq!(scope_id, "oc-tool-v1|s=8:ses_main|c=6:call_1");
        assert_eq!(
            opencode_scope_start_event_id(&scope_id),
            format!("{scope_id}|start")
        );
        assert_eq!(
            opencode_scope_close_event_id(&scope_id),
            format!("{scope_id}|close")
        );
        assert_ne!(
            opencode_scope_start_event_id(&scope_id),
            opencode_scope_close_event_id(&scope_id)
        );
    }

    #[test]
    fn duplicate_events_reuse_the_same_scope_id() {
        let payload = tool_event_json(
            HOOK_EVENT_TOOL_EXECUTE_BEFORE,
            &[(TOOL_NAME_FIELD, Value::String("bash".to_string()))],
        );
        let first = tool_execution(&payload).identity.attempt_key();
        let second = tool_execution(&payload).identity.attempt_key();
        assert_eq!(
            format_opencode_scope_id(&first),
            format_opencode_scope_id(&second)
        );
    }

    #[test]
    fn length_prefix_disambiguates_delimiter_collisions() {
        let a = key("s|c=1:x", "y");
        let b = key("s", "1:x|y");
        assert_ne!(format_opencode_scope_id(&a), format_opencode_scope_id(&b));

        let tricky = key("ses|c=0:x", "call:with:colons");
        assert_eq!(
            format_opencode_scope_id(&tricky),
            format!(
                "oc-tool-v1|s={}:{}|c={}:{}",
                tricky.session_id.len(),
                tricky.session_id,
                tricky.call_id.len(),
                tricky.call_id,
            )
        );
    }

    #[test]
    fn parallel_call_ids_in_one_session_stay_distinguishable() {
        let a = tool_execution(&tool_event_json(
            HOOK_EVENT_TOOL_EXECUTE_BEFORE,
            &[
                (TOOL_NAME_FIELD, Value::String("bash".to_string())),
                (CALL_ID_FIELD, Value::String("call_a".to_string())),
            ],
        ));
        let b = tool_execution(&tool_event_json(
            HOOK_EVENT_TOOL_EXECUTE_BEFORE,
            &[
                (TOOL_NAME_FIELD, Value::String("bash".to_string())),
                (CALL_ID_FIELD, Value::String("call_b".to_string())),
            ],
        ));
        assert_ne!(a.identity.attempt_key(), b.identity.attempt_key());
        assert_ne!(
            format_opencode_scope_id(&a.identity.attempt_key()),
            format_opencode_scope_id(&b.identity.attempt_key())
        );
    }

    #[test]
    fn task_child_session_identity_flows_through_the_attempt_key() {
        let child = tool_execution(&tool_event_json(
            HOOK_EVENT_TOOL_EXECUTE_BEFORE,
            &[
                (TOOL_NAME_FIELD, Value::String("bash".to_string())),
                (SESSION_ID_FIELD, Value::String("ses_child".to_string())),
                (CALL_ID_FIELD, Value::String("call_child".to_string())),
            ],
        ));
        assert_eq!(child.identity.attempt_key(), key("ses_child", "call_child"));
        assert_ne!(
            format_opencode_scope_id(&child.identity.attempt_key()),
            format_opencode_scope_id(&key("ses_main", "call_child"))
        );
    }

    #[test]
    fn attempt_key_projects_only_session_and_call() {
        let identity_a = OpenCodeToolIdentity {
            session_id: "ses_main".to_string(),
            call_id: "call_1".to_string(),
            cwd: "/repo".to_string(),
            tool_name: "write".to_string(),
        };
        let identity_b = OpenCodeToolIdentity {
            tool_name: "bash".to_string(),
            cwd: "/other".to_string(),
            ..identity_a.clone()
        };
        assert_eq!(identity_a.attempt_key(), identity_b.attempt_key());
    }

    #[test]
    fn provenance_canonicalizes_the_session_and_normalizes_the_model() {
        let provenance = opencode_scope_provenance("ses_main", Some("opencode/big-pickle"));
        assert_eq!(provenance.session_id, "oc_ses_main");
        assert_eq!(provenance.model_id.as_deref(), Some("opencode/big-pickle"));
    }

    #[test]
    fn provenance_keeps_an_already_prefixed_session_id() {
        let provenance = opencode_scope_provenance("oc_ses_main", None);
        assert_eq!(provenance.session_id, "oc_ses_main");
    }

    #[test]
    fn provenance_without_model_evidence_is_null() {
        for model in [None, Some(""), Some("   ")] {
            let provenance = opencode_scope_provenance("ses_main", model);
            assert_eq!(provenance.model_id, None, "model {model:?}");
            assert_eq!(provenance.session_id, "oc_ses_main", "model {model:?}");
        }
    }

    #[test]
    fn provenance_is_built_from_a_parsed_start_event() {
        let execution = tool_execution(&tool_event_json(
            HOOK_EVENT_TOOL_EXECUTE_BEFORE,
            &[
                (TOOL_NAME_FIELD, Value::String("write".to_string())),
                (
                    MODEL_FIELD,
                    Value::String("opencode/big-pickle".to_string()),
                ),
            ],
        ));
        let provenance =
            opencode_scope_provenance(&execution.identity.session_id, execution.model.as_deref());
        assert_eq!(provenance.session_id, "oc_ses_main");
        assert_eq!(provenance.model_id.as_deref(), Some("opencode/big-pickle"));
    }

    #[test]
    fn run_from_payload_is_neutral_for_a_well_formed_event() {
        let payload = tool_event_json(
            HOOK_EVENT_TOOL_EXECUTE_BEFORE,
            &[(TOOL_NAME_FIELD, Value::String("write".to_string()))],
        );
        assert_eq!(
            run_opencode_mutation_scope_from_payload(&payload, None).unwrap(),
            String::new()
        );
    }

    #[test]
    fn run_from_payload_is_neutral_for_untracked_and_delegation_events() {
        for tool_name in ["read", "task", "probe_mutate"] {
            let payload = tool_event_json(
                HOOK_EVENT_TOOL_EXECUTE_BEFORE,
                &[(TOOL_NAME_FIELD, Value::String(tool_name.to_string()))],
            );
            assert_eq!(
                run_opencode_mutation_scope_from_payload(&payload, None).unwrap(),
                String::new()
            );
        }
    }

    #[test]
    fn run_from_payload_surfaces_malformed_input() {
        let error = run_opencode_mutation_scope_from_payload("{bad", None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("expected valid JSON"), "{error:?}");
    }
}
