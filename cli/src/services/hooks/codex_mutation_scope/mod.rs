#![allow(dead_code)]

pub(crate) mod state;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{Map, Value};

const HOOK_EVENT_NAME_FIELD: &str = "hook_event_name";
const SESSION_ID_FIELD: &str = "session_id";
const TURN_ID_FIELD: &str = "turn_id";
const CWD_FIELD: &str = "cwd";
const AGENT_ID_FIELD: &str = "agent_id";
const AGENT_TYPE_FIELD: &str = "agent_type";
const TOOL_NAME_FIELD: &str = "tool_name";
const TOOL_USE_ID_FIELD: &str = "tool_use_id";

const HOOK_EVENT_PRE_TOOL_USE: &str = "PreToolUse";
const HOOK_EVENT_POST_TOOL_USE: &str = "PostToolUse";
const HOOK_EVENT_STOP: &str = "Stop";
const HOOK_EVENT_INTERRUPT: &str = "Interrupt";
const HOOK_EVENT_SUBAGENT_STOP: &str = "SubagentStop";
const HOOK_EVENT_SESSION_END: &str = "SessionEnd";

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

const TRACKED_MUTATION_TOOL_NAMES: &[&str] = &["Bash", "apply_patch"];
const DELEGATION_TOOL_NAMES: &[&str] = &["collaborationspawn_agent", "collaborationwait_agent"];
const MCP_TOOL_NAME_PREFIX: &str = "mcp__";

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

const CODEX_SCOPE_ID_SCHEME: &str = "cx-tool-v1";

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

fn parse_tool_identity(object: &Map<String, Value>) -> Result<CodexToolIdentity> {
    Ok(CodexToolIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        turn_id: required_non_blank_str(object, TURN_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
        agent_id: optional_non_blank_str(object, AGENT_ID_FIELD)?,
        tool_name: required_non_blank_str(object, TOOL_NAME_FIELD)?,
        tool_use_id: required_non_blank_str(object, TOOL_USE_ID_FIELD)?,
    })
}

fn parse_pre_tool_use(object: &Map<String, Value>) -> Result<CodexToolExecution> {
    Ok(CodexToolExecution {
        identity: parse_tool_identity(object)?,
        agent_type: optional_non_blank_str(object, AGENT_TYPE_FIELD)?,
    })
}

fn parse_turn_identity(object: &Map<String, Value>) -> Result<CodexTurnIdentity> {
    Ok(CodexTurnIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        turn_id: required_non_blank_str(object, TURN_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
    })
}

fn parse_agent_identity(object: &Map<String, Value>) -> Result<CodexAgentIdentity> {
    Ok(CodexAgentIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        turn_id: required_non_blank_str(object, TURN_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
        agent_id: required_non_blank_str(object, AGENT_ID_FIELD)?,
        agent_type: optional_non_blank_str(object, AGENT_TYPE_FIELD)?,
    })
}

fn parse_session_identity(object: &Map<String, Value>) -> Result<CodexSessionIdentity> {
    Ok(CodexSessionIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
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
    format!("Invalid Codex hook event payload from STDIN: {detail}.")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROBE01_SHELL_PRE: &str =
        include_str!("fixtures/probe01-apply-patch-and-shell-success.shell.pre_tool_use.json");
    const PROBE01_SHELL_POST: &str =
        include_str!("fixtures/probe01-apply-patch-and-shell-success.shell.post_tool_use.json");
    const PROBE01_APPLY_PATCH_PRE: &str = include_str!(
        "fixtures/probe01-apply-patch-and-shell-success.apply_patch.pre_tool_use.json"
    );
    const PROBE01_STOP: &str =
        include_str!("fixtures/probe01-apply-patch-and-shell-success.stop.json");
    const PROBE01_SESSION_END: &str =
        include_str!("fixtures/probe01-apply-patch-and-shell-success.session_end.json");
    const PROBE02_FAILED_SHELL_POST: &str =
        include_str!("fixtures/probe02-shell-partial-write-then-nonzero-exit.post_tool_use.json");
    const PROBE04_BLOCKED_PRE: &str = include_str!(
        "fixtures/probe04-pre-tool-use-hook-hookspecificoutput-deny.pre_tool_use.json"
    );
    const PROBE05_SHELL_PRE: &str =
        include_str!("fixtures/probe05-tool-vocabulary.shell-read-list-search.pre_tool_use.json");
    const PROBE08_SPAWN_AGENT_PRE: &str =
        include_str!("fixtures/probe08-subagent-delegation.spawn_agent.pre_tool_use.json");
    const PROBE08_WAIT_AGENT_PRE: &str =
        include_str!("fixtures/probe08-subagent-delegation.wait_agent.pre_tool_use.json");
    const PROBE08_AGENT_APPLY_PATCH_PRE: &str =
        include_str!("fixtures/probe08-subagent-delegation.agent-apply-patch.pre_tool_use.json");
    const PROBE08_AGENT_APPLY_PATCH_POST: &str =
        include_str!("fixtures/probe08-subagent-delegation.agent-apply-patch.post_tool_use.json");
    const PROBE08_SUBAGENT_STOP: &str =
        include_str!("fixtures/probe08-subagent-delegation.subagent_stop.json");
    const PROBE10_WORKTREE_PRE: &str =
        include_str!("fixtures/probe10-linked-worktree-cwd.pre_tool_use.json");
    const PROBE11_INTERRUPT: &str =
        include_str!("fixtures/probe11-interrupt-event-on-sigint.interrupt.json");
    const PROBE12_MCP_PRE: &str =
        include_str!("fixtures/probe12-mcp-mutate-success.pre_tool_use.json");
    const PROBE12_MCP_POST: &str =
        include_str!("fixtures/probe12-mcp-mutate-success.post_tool_use.json");
    const PROBE13_MCP_MUTATE_THEN_ERROR_PRE: &str =
        include_str!("fixtures/probe13-mcp-mutate-then-error.pre_tool_use.json");
    const PROBE13_MCP_SESSION_END: &str =
        include_str!("fixtures/probe13-mcp-mutate-then-error.session_end.json");

    fn pre_tool_use_json(overrides: &[(&str, Value)]) -> String {
        let mut object = Map::new();
        object.insert(
            HOOK_EVENT_NAME_FIELD.to_string(),
            Value::String(HOOK_EVENT_PRE_TOOL_USE.to_string()),
        );
        object.insert(
            SESSION_ID_FIELD.to_string(),
            Value::String("session-1".to_string()),
        );
        object.insert(
            TURN_ID_FIELD.to_string(),
            Value::String("turn-1".to_string()),
        );
        object.insert(
            CWD_FIELD.to_string(),
            Value::String("/repo/checkout".to_string()),
        );
        object.insert(
            TOOL_NAME_FIELD.to_string(),
            Value::String("Bash".to_string()),
        );
        object.insert(
            TOOL_USE_ID_FIELD.to_string(),
            Value::String("exec-1".to_string()),
        );
        for (field, value) in overrides {
            object.insert((*field).to_string(), value.clone());
        }
        Value::Object(object).to_string()
    }

    fn key(session_id: &str, agent_id: Option<&str>, tool_use_id: &str) -> AttemptKey {
        AttemptKey {
            session_id: session_id.to_string(),
            agent_id: agent_id.map(str::to_string),
            tool_use_id: tool_use_id.to_string(),
        }
    }

    fn pre_tool_use(payload: &str) -> CodexToolExecution {
        match parse_codex_hook_event(payload).expect("valid PreToolUse parses") {
            CodexHookEvent::PreToolUse(execution) => execution,
            other => panic!("expected PreToolUse, got {other:?}"),
        }
    }

    #[test]
    fn ac2_empty_payload_is_rejected() {
        let error = parse_codex_hook_event("   ").unwrap_err().to_string();
        assert_eq!(
            error,
            "Invalid Codex hook event payload from STDIN: expected a JSON object, got an empty payload."
        );
    }

    #[test]
    fn ac2_non_object_json_is_rejected() {
        for payload in ["[]", "\"PreToolUse\"", "42", "null"] {
            let error = parse_codex_hook_event(payload).unwrap_err().to_string();
            assert!(
                error.contains("expected a JSON object"),
                "payload {payload:?} produced {error:?}"
            );
        }
    }

    #[test]
    fn ac2_invalid_json_is_rejected() {
        let error = parse_codex_hook_event("{not json").unwrap_err().to_string();
        assert!(
            error.contains("Invalid Codex hook event payload from STDIN: expected valid JSON"),
            "{error:?}"
        );
    }

    #[test]
    fn ac2_unsupported_hook_event_name_is_rejected() {
        for name in [
            "SessionStart",
            "SubagentStart",
            "UserPromptSubmit",
            "PreCompact",
        ] {
            let payload =
                pre_tool_use_json(&[(HOOK_EVENT_NAME_FIELD, Value::String(name.to_string()))]);
            let error = parse_codex_hook_event(&payload).unwrap_err().to_string();
            assert!(
                error.contains(&format!("unsupported hook_event_name '{name}'")),
                "{error:?}"
            );
        }
    }

    #[test]
    fn ac2_missing_required_fields_are_rejected_without_fabricating_identity() {
        for field in [
            SESSION_ID_FIELD,
            TURN_ID_FIELD,
            CWD_FIELD,
            TOOL_NAME_FIELD,
            TOOL_USE_ID_FIELD,
        ] {
            let mut object: Map<String, Value> =
                serde_json::from_str(&pre_tool_use_json(&[])).unwrap();
            object.remove(field);
            let payload = Value::Object(object).to_string();

            let error = parse_codex_hook_event(&payload).unwrap_err().to_string();
            assert!(
                error.contains(&format!("'{field}'")),
                "missing {field} produced {error:?}"
            );
        }
    }

    #[test]
    fn ac2_blank_required_fields_are_rejected() {
        for field in [SESSION_ID_FIELD, TURN_ID_FIELD, CWD_FIELD, TOOL_NAME_FIELD] {
            let payload = pre_tool_use_json(&[(field, Value::String("   ".to_string()))]);
            let error = parse_codex_hook_event(&payload).unwrap_err().to_string();
            assert!(
                error.contains(&format!("field '{field}' must be a non-blank string")),
                "blank {field} produced {error:?}"
            );
        }
    }

    #[test]
    fn ac2_wrong_typed_fields_are_rejected() {
        let payload = pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::Bool(true))]);
        let error = parse_codex_hook_event(&payload).unwrap_err().to_string();
        assert!(
            error.contains("field 'tool_use_id' must be a string"),
            "{error:?}"
        );
    }

    #[test]
    fn ac2_wrong_typed_optional_agent_id_is_rejected() {
        let payload = pre_tool_use_json(&[(AGENT_ID_FIELD, Value::Bool(false))]);
        let error = parse_codex_hook_event(&payload).unwrap_err().to_string();
        assert!(
            error.contains("field 'agent_id' must be null, absent, or a non-blank string"),
            "{error:?}"
        );
    }

    #[test]
    fn ac2_pre_tool_use_fixtures_parse_to_expected_identity() {
        let shell = pre_tool_use(PROBE01_SHELL_PRE);
        assert_eq!(shell.identity.tool_name, "Bash");
        assert_eq!(
            shell.identity.tool_use_id,
            "exec-414820f5-555e-457a-92e7-60ddd27d4eec"
        );
        assert_eq!(
            shell.identity.session_id,
            "01a07c1e-e08e-7172-8032-cb9d62af21d9"
        );
        assert_eq!(
            shell.identity.turn_id,
            "01a07c1e-e0cc-75f1-a566-e790c06cb033"
        );
        assert_eq!(shell.identity.agent_id, None);
        assert!(!shell.identity.is_subagent());
        assert!(shell.identity.cwd.ends_with("/probe-repo"));

        let apply_patch = pre_tool_use(PROBE01_APPLY_PATCH_PRE);
        assert_eq!(apply_patch.identity.tool_name, "apply_patch");

        let vocab = pre_tool_use(PROBE05_SHELL_PRE);
        assert_eq!(vocab.identity.tool_name, "Bash");

        let worktree = pre_tool_use(PROBE10_WORKTREE_PRE);
        assert!(worktree.identity.cwd.ends_with("/probe-worktree"));

        let mcp = pre_tool_use(PROBE12_MCP_PRE);
        assert_eq!(mcp.identity.tool_name, "mcp__probe__mutate_success");

        let mcp_err = pre_tool_use(PROBE13_MCP_MUTATE_THEN_ERROR_PRE);
        assert!(is_mcp_tool_name(&mcp_err.identity.tool_name));
    }

    #[test]
    fn ac2_subagent_pre_tool_use_fixture_carries_agent_identity() {
        let execution = pre_tool_use(PROBE08_AGENT_APPLY_PATCH_PRE);
        assert_eq!(
            execution.identity.agent_id.as_deref(),
            Some("01a07c24-bb59-7ca0-80f7-99cf940a486e")
        );
        assert!(execution.identity.is_subagent());
        assert_eq!(execution.agent_type.as_deref(), Some("default"));
    }

    #[test]
    fn ac2_post_tool_use_fixtures_parse() {
        for (payload, tool_name, tool_use_id) in [
            (
                PROBE01_SHELL_POST,
                "Bash",
                "exec-414820f5-555e-457a-92e7-60ddd27d4eec",
            ),
            (
                PROBE02_FAILED_SHELL_POST,
                "Bash",
                "exec-52155265-e98d-423f-87f8-76ee56ff33b1",
            ),
            (
                PROBE12_MCP_POST,
                "mcp__probe__mutate_success",
                "exec-00988fad-6707-48ed-81b6-07bb11933886",
            ),
        ] {
            match parse_codex_hook_event(payload).expect("PostToolUse fixture parses") {
                CodexHookEvent::PostToolUse(identity) => {
                    assert_eq!(identity.tool_name, tool_name);
                    assert_eq!(identity.tool_use_id, tool_use_id);
                }
                other => panic!("expected PostToolUse, got {other:?}"),
            }
        }
    }

    #[test]
    fn ac2_subagent_post_tool_use_ties_to_its_pre_tool_use() {
        let CodexHookEvent::PostToolUse(post) =
            parse_codex_hook_event(PROBE08_AGENT_APPLY_PATCH_POST).unwrap()
        else {
            panic!("expected PostToolUse");
        };
        let pre = pre_tool_use(PROBE08_AGENT_APPLY_PATCH_PRE);
        assert_eq!(post.attempt_key(), pre.identity.attempt_key());
        assert!(post.attempt_key().agent_id.is_some());
    }

    #[test]
    fn ac2_terminal_lifecycle_fixtures_parse() {
        assert!(matches!(
            parse_codex_hook_event(PROBE01_STOP).unwrap(),
            CodexHookEvent::Stop(id) if id.turn_id == "01a07c1e-e0cc-75f1-a566-e790c06cb033"
        ));
        assert!(matches!(
            parse_codex_hook_event(PROBE11_INTERRUPT).unwrap(),
            CodexHookEvent::Interrupt(id) if id.session_id == "01a07c2f-ccbf-79f0-afb9-2d2ce919eea7"
        ));
        assert!(matches!(
            parse_codex_hook_event(PROBE08_SUBAGENT_STOP).unwrap(),
            CodexHookEvent::SubagentStop(id)
                if id.agent_id == "01a07c24-bb59-7ca0-80f7-99cf940a486e"
        ));
        for session_end in [PROBE01_SESSION_END, PROBE13_MCP_SESSION_END] {
            assert!(matches!(
                parse_codex_hook_event(session_end).unwrap(),
                CodexHookEvent::SessionEnd(_)
            ));
        }
    }

    #[test]
    fn ac2_session_end_needs_no_turn_id() {
        let CodexHookEvent::SessionEnd(identity) =
            parse_codex_hook_event(PROBE01_SESSION_END).unwrap()
        else {
            panic!("expected SessionEnd");
        };
        assert_eq!(identity.session_id, "01a07c1e-e08e-7172-8032-cb9d62af21d9");
    }

    #[test]
    fn ac3_classification_table() {
        let cases: &[(&str, ToolClassification)] = &[
            ("Bash", ToolClassification::TrackedMutation),
            ("apply_patch", ToolClassification::TrackedMutation),
            ("collaborationspawn_agent", ToolClassification::Delegation),
            ("collaborationwait_agent", ToolClassification::Delegation),
            ("mcp__probe__mutate_success", ToolClassification::Untracked),
            ("mcp__probe_par__slow_mutate", ToolClassification::Untracked),
            ("mcp__", ToolClassification::Untracked),
            ("Read", ToolClassification::Untracked),
            ("PowerShell", ToolClassification::Untracked),
            ("some_future_codex_tool", ToolClassification::Untracked),
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
    fn ac3_classification_is_total_and_single_valued() {
        for tool_name in [
            "Bash",
            "apply_patch",
            "collaborationspawn_agent",
            "collaborationwait_agent",
            "mcp__x__y",
            "unknown",
        ] {
            let _: ToolClassification = classify_tool(tool_name);
        }
    }

    #[test]
    fn ac3_delegation_and_untracked_tool_fixtures_do_not_yield_a_tracked_scope() {
        for payload in [
            PROBE08_SPAWN_AGENT_PRE,
            PROBE08_WAIT_AGENT_PRE,
            PROBE12_MCP_PRE,
            PROBE13_MCP_MUTATE_THEN_ERROR_PRE,
        ] {
            let execution = pre_tool_use(payload);
            let classification = classify_tool(&execution.identity.tool_name);
            assert_ne!(
                classification,
                ToolClassification::TrackedMutation,
                "tool {:?} must not be TrackedMutation",
                execution.identity.tool_name
            );
        }

        assert_eq!(
            classify_tool(&pre_tool_use(PROBE04_BLOCKED_PRE).identity.tool_name),
            ToolClassification::TrackedMutation
        );
    }

    #[test]
    fn ac3_is_mcp_tool_name() {
        assert!(is_mcp_tool_name("mcp__probe__mutate_success"));
        assert!(is_mcp_tool_name("mcp__"));
        assert!(!is_mcp_tool_name("Bash"));
        assert!(!is_mcp_tool_name("apply_patch"));
        assert!(!is_mcp_tool_name("collaborationspawn_agent"));
    }

    #[test]
    fn ac4_scope_id_is_deterministic_for_the_same_attempt_seq_and_key() {
        let k = key("session-1", None, "exec-1");
        assert_eq!(format_codex_scope_id(7, &k), format_codex_scope_id(7, &k));

        let scope_id = format_codex_scope_id(7, &k);
        assert_eq!(scope_id, "cx-tool-v1|n=7|s=9:session-1|a=0:|t=6:exec-1");
        assert_eq!(
            codex_scope_start_event_id(&scope_id),
            format!("{scope_id}|start")
        );
        assert_eq!(
            codex_scope_close_event_id(&scope_id),
            format!("{scope_id}|close")
        );
    }

    #[test]
    fn ac4_length_prefix_disambiguates_delimiter_collisions() {
        let a = key("a:b", None, "c");
        let b = key("a", None, "b:c");
        assert_ne!(format_codex_scope_id(1, &a), format_codex_scope_id(1, &b));
    }

    #[test]
    fn ac4_subagent_key_encodes_the_agent_id() {
        let main = key("session-1", None, "exec-1");
        let sub = key("session-1", Some("agent-1"), "exec-1");
        assert_ne!(
            format_codex_scope_id(1, &main),
            format_codex_scope_id(1, &sub)
        );
        assert_eq!(
            format_codex_scope_id(1, &sub),
            "cx-tool-v1|n=1|s=9:session-1|a=7:agent-1|t=6:exec-1"
        );
    }

    #[test]
    fn ac5_a_fresh_attempt_seq_yields_a_new_scope_id() {
        let k = key("session-1", None, "exec-1");
        assert_ne!(format_codex_scope_id(1, &k), format_codex_scope_id(2, &k));
        assert!(format_codex_scope_id(2, &k).contains("|n=2|"));
    }

    #[test]
    fn ac5_attempt_key_excludes_turn_id() {
        let base = pre_tool_use(&pre_tool_use_json(&[(
            TOOL_USE_ID_FIELD,
            Value::String("exec-9".to_string()),
        )]));
        let other_turn = pre_tool_use(&pre_tool_use_json(&[
            (TOOL_USE_ID_FIELD, Value::String("exec-9".to_string())),
            (TURN_ID_FIELD, Value::String("turn-99".to_string())),
        ]));
        assert_eq!(
            base.identity.attempt_key(),
            other_turn.identity.attempt_key()
        );
    }
}
