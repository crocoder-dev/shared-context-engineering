#![allow(dead_code)]

pub(crate) mod state;

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::services::checkout;
use crate::services::observability::traits::Logger;

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

type GitDirResolver<'a> = &'a dyn Fn(&str) -> Result<PathBuf>;

type IngressSeam<'a> = &'a dyn Fn(&Path, &str, Option<&dyn Logger>) -> Result<String>;

const ACTOR_KIND_CODEX: &str = "codex";

const FAIL_CLOSED_DENY_REASON: &str =
    "SCE could not establish mutation attribution for this tool execution.";

const PRE_TOOL_USE_FAIL_CLOSED_EVENT: &str =
    "sce.hooks.codex_mutation_scope.pre_tool_use_fail_closed";

fn log_pre_tool_use_fail_closed(logger: Option<&dyn Logger>, context: &str, error: &anyhow::Error) {
    if let Some(log) = logger {
        log.warn(
            PRE_TOOL_USE_FAIL_CLOSED_EVENT,
            &error.to_string(),
            &[("context", context)],
            None,
        );
    }
}

pub(crate) fn run_codex_mutation_scope_subcommand(logger: Option<&dyn Logger>) -> Result<String> {
    let stdin_payload = super::read_hook_stdin()?;
    run_codex_mutation_scope_from_payload(&stdin_payload, logger)
}

pub(crate) fn run_codex_mutation_scope_from_payload(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let resolve_git_dir_fn = |cwd: &str| checkout::resolve_git_dir(Path::new(cwd));
    let seam_fn = |repository_root: &Path, payload: &str, logger: Option<&dyn Logger>| {
        super::mutation_scope::run_mutation_scope_from_payload(repository_root, payload, logger)
    };

    run_codex_mutation_scope_from_payload_with(stdin_payload, logger, &resolve_git_dir_fn, &seam_fn)
}

#[cfg(test)]
fn run_codex_mutation_scope_from_payload_at_state_root(
    state_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let resolve_git_dir_fn = |cwd: &str| checkout::resolve_git_dir(Path::new(cwd));
    let seam_fn = |repository_root: &Path, payload: &str, logger: Option<&dyn Logger>| {
        super::mutation_scope::run_mutation_scope_from_payload_at_state_root(
            repository_root,
            state_root,
            payload,
            logger,
        )
    };

    run_codex_mutation_scope_from_payload_with(stdin_payload, logger, &resolve_git_dir_fn, &seam_fn)
}

fn run_codex_mutation_scope_from_payload_with(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> Result<String> {
    let event = parse_codex_hook_event(stdin_payload)?;
    dispatch_codex_hook_event(event, logger, resolve_git_dir, seam)
}

fn dispatch_codex_hook_event(
    event: CodexHookEvent,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> Result<String> {
    match event {
        CodexHookEvent::PreToolUse(execution) => Ok(handle_pre_tool_use(
            &execution,
            logger,
            resolve_git_dir,
            seam,
        )),
        CodexHookEvent::PostToolUse(identity) => {
            let git_dir = resolve_git_dir(&identity.cwd)?;
            let repository_root = Path::new(&identity.cwd);
            handle_close(
                &git_dir,
                repository_root,
                &identity.attempt_key(),
                logger,
                seam,
            )
        }
        CodexHookEvent::Stop(turn) => {
            let git_dir = resolve_git_dir(&turn.cwd)?;
            let repository_root = Path::new(&turn.cwd);
            let session_id = turn.session_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id && attempt.agent_id.is_none()
            })
        }
        CodexHookEvent::Interrupt(turn) => {
            let git_dir = resolve_git_dir(&turn.cwd)?;
            let repository_root = Path::new(&turn.cwd);
            let session_id = turn.session_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id
            })
        }
        CodexHookEvent::SubagentStop(agent) => {
            let git_dir = resolve_git_dir(&agent.cwd)?;
            let repository_root = Path::new(&agent.cwd);
            let session_id = agent.session_id.clone();
            let agent_id = agent.agent_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id && attempt.agent_id.as_deref() == Some(&agent_id)
            })
        }
        CodexHookEvent::SessionEnd(session) => {
            let git_dir = resolve_git_dir(&session.cwd)?;
            let repository_root = Path::new(&session.cwd);
            let session_id = session.session_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id
            })
        }
    }
}

fn handle_pre_tool_use(
    execution: &CodexToolExecution,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> String {
    let identity = &execution.identity;

    if !matches!(
        classify_tool(&identity.tool_name),
        ToolClassification::TrackedMutation
    ) {
        return String::new();
    }

    let repository_root = Path::new(&identity.cwd);
    let git_dir = match resolve_git_dir(&identity.cwd) {
        Ok(git_dir) => git_dir,
        Err(error) => {
            log_pre_tool_use_fail_closed(logger, "resolve_git_dir", &error);
            return pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON);
        }
    };

    if matches!(
        apply_recovery_barrier(&git_dir, repository_root, logger, seam),
        BarrierOutcome::Deny
    ) {
        return pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON);
    }

    match establish_start(&git_dir, repository_root, identity, logger, seam) {
        Ok(()) => String::new(),
        Err(error) => {
            log_pre_tool_use_fail_closed(logger, "establish_start", &error);
            pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON)
        }
    }
}

enum BarrierOutcome {
    Proceed,
    Deny,
}

fn apply_recovery_barrier(
    git_dir: &Path,
    repository_root: &Path,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> BarrierOutcome {
    let state = match state::read_state(git_dir) {
        Ok(state) => state,
        Err(error) => {
            log_pre_tool_use_fail_closed(logger, "recovery_barrier.read_state", &error);
            return BarrierOutcome::Deny;
        }
    };

    if !state.recovery_pending {
        return BarrierOutcome::Proceed;
    }

    if !state.attempts.is_empty() {
        return BarrierOutcome::Deny;
    }

    match seam(repository_root, &flush_payload(), logger) {
        Ok(_) => match state::clear_recovery_pending(git_dir) {
            Ok(()) => BarrierOutcome::Proceed,
            Err(error) => {
                log_pre_tool_use_fail_closed(
                    logger,
                    "recovery_barrier.clear_recovery_pending",
                    &error,
                );
                BarrierOutcome::Deny
            }
        },
        Err(error) => {
            log_pre_tool_use_fail_closed(logger, "recovery_barrier.flush", &error);
            BarrierOutcome::Deny
        }
    }
}

fn establish_start(
    git_dir: &Path,
    repository_root: &Path,
    identity: &CodexToolIdentity,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<()> {
    let allocated = state::allocate_attempt(git_dir, &identity.attempt_key(), &identity.tool_name)?;
    let scope_id = &allocated.attempt.scope_id;
    let start_payload =
        scope_boundary_payload("start", scope_id, &codex_scope_start_event_id(scope_id));

    seam(repository_root, &start_payload, logger)?;
    state::mark_active(git_dir, scope_id)?;
    Ok(())
}

fn handle_close(
    git_dir: &Path,
    repository_root: &Path,
    key: &AttemptKey,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<String> {
    let current = state::read_state(git_dir)?;
    let Some(attempt) = current
        .attempts
        .iter()
        .find(|attempt| attempt_matches_key(attempt, key))
        .cloned()
    else {
        return Ok(String::new());
    };

    if attempt.phase == state::AttemptPhase::PendingStart {
        abandon_attempt(git_dir, repository_root, &attempt, logger, seam)?;
        return Ok(String::new());
    }

    let close_payload = scope_boundary_payload(
        "close",
        &attempt.scope_id,
        &codex_scope_close_event_id(&attempt.scope_id),
    );

    if seam(repository_root, &close_payload, logger).is_ok() {
        state::remove_attempt(git_dir, &attempt.scope_id)?;
    } else {
        abandon_attempt(git_dir, repository_root, &attempt, logger, seam)?;
    }
    Ok(String::new())
}

fn cleanup_attempts_matching(
    git_dir: &Path,
    repository_root: &Path,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
    predicate: impl Fn(&state::AdapterAttempt) -> bool,
) -> Result<String> {
    let current = state::read_state(git_dir)?;
    let stale: Vec<state::AdapterAttempt> = current
        .attempts
        .into_iter()
        .filter(|attempt| predicate(attempt))
        .collect();

    for attempt in &stale {
        abandon_attempt(git_dir, repository_root, attempt, logger, seam)?;
    }

    Ok(String::new())
}

fn attempt_matches_key(attempt: &state::AdapterAttempt, key: &AttemptKey) -> bool {
    attempt.session_id == key.session_id
        && attempt.agent_id == key.agent_id
        && attempt.tool_use_id == key.tool_use_id
}

fn abandon_attempt(
    git_dir: &Path,
    repository_root: &Path,
    attempt: &state::AdapterAttempt,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<()> {
    state::mark_recovery_pending(git_dir)?;

    seam(repository_root, &abandon_payload(&attempt.scope_id), logger)?;
    state::remove_attempt(git_dir, &attempt.scope_id)?;
    Ok(())
}

fn scope_boundary_payload(operation: &str, scope_id: &str, event_id: &str) -> String {
    json!({
        "operation": operation,
        "scope_id": scope_id,
        "event_id": event_id,
        "actor_kind": ACTOR_KIND_CODEX,
    })
    .to_string()
}

fn abandon_payload(scope_id: &str) -> String {
    json!({
        "operation": "abandon",
        "scope_id": scope_id,
    })
    .to_string()
}

fn flush_payload() -> String {
    json!({ "operation": "flush" }).to_string()
}

fn pre_tool_use_deny_json(reason: &str) -> String {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }
    })
    .to_string()
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

    mod driver {
        use std::cell::RefCell;
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::{Arc, Mutex};

        use anyhow::{anyhow, Result};

        use super::*;
        use crate::services::observability::traits::Logger;

        const CWD: &str = "/repo/checkout";

        static NEXT_TEST_GIT_DIR_ID: AtomicU64 = AtomicU64::new(0);

        fn unique_test_git_dir(label: &str) -> PathBuf {
            let id = NEXT_TEST_GIT_DIR_ID.fetch_add(1, Ordering::Relaxed);
            std::env::temp_dir().join(format!(
                "sce-codex-mutation-scope-driver-{label}-{}-{id}",
                std::process::id()
            ))
        }

        fn remove_test_git_dir(git_dir: &Path) {
            let _ = std::fs::remove_dir_all(git_dir);
        }

        #[allow(clippy::unnecessary_wraps)]
        fn ok_seam(_root: &Path, _payload: &str, _logger: Option<&dyn Logger>) -> Result<String> {
            Ok(String::new())
        }

        fn unreachable_seam(
            _root: &Path,
            payload: &str,
            _logger: Option<&dyn Logger>,
        ) -> Result<String> {
            panic!("the ingress seam must not be called for this payload: {payload}");
        }

        fn seam_failing_on(
            operation: &'static str,
        ) -> impl Fn(&Path, &str, Option<&dyn Logger>) -> Result<String> {
            seam_failing_on_any(vec![operation])
        }

        fn seam_failing_on_any(
            operations: Vec<&'static str>,
        ) -> impl Fn(&Path, &str, Option<&dyn Logger>) -> Result<String> {
            move |_root, payload, _logger| {
                if operations
                    .iter()
                    .any(|operation| payload.contains(&format!(r#""operation":"{operation}""#)))
                {
                    Err(anyhow!(
                        "seam failure injected by test for one of {operations:?}"
                    ))
                } else {
                    Ok(String::new())
                }
            }
        }

        fn fixed_resolver(git_dir: PathBuf) -> impl Fn(&str) -> Result<PathBuf> {
            move |_cwd| Ok(git_dir.clone())
        }

        #[derive(Clone, Default)]
        struct RecordingLogger {
            warnings: Arc<Mutex<Vec<(String, String)>>>,
        }

        impl RecordingLogger {
            fn warnings(&self) -> Vec<(String, String)> {
                self.warnings
                    .lock()
                    .expect("recording logger mutex must not be poisoned")
                    .clone()
            }
        }

        impl Logger for RecordingLogger {
            fn info(&self, _: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {}
            fn debug(&self, _: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {}

            fn warn(&self, event_id: &str, message: &str, _: &[(&str, &str)], _: Option<&str>) {
                self.warnings
                    .lock()
                    .expect("recording logger mutex must not be poisoned")
                    .push((event_id.to_string(), message.to_string()));
            }

            fn error(&self, _: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {}

            fn log_cli_error(&self, _: &crate::services::error::CliError, _: Option<&str>) {}
        }

        fn tool_event_json(event_name: &str, overrides: &[(&str, Value)]) -> String {
            let mut merged: Vec<(&str, Value)> =
                vec![(HOOK_EVENT_NAME_FIELD, Value::String(event_name.to_string()))];
            merged.extend(
                overrides
                    .iter()
                    .map(|(field, value)| (*field, value.clone())),
            );
            pre_tool_use_json(&merged)
        }

        fn post_tool_use_json(overrides: &[(&str, Value)]) -> String {
            tool_event_json(HOOK_EVENT_POST_TOOL_USE, overrides)
        }

        fn turn_scoped_payload(event_name: &str, session_id: &str, turn_id: &str) -> String {
            json!({
                HOOK_EVENT_NAME_FIELD: event_name,
                SESSION_ID_FIELD: session_id,
                TURN_ID_FIELD: turn_id,
                CWD_FIELD: CWD,
            })
            .to_string()
        }

        fn subagent_stop_payload(session_id: &str, turn_id: &str, agent_id: &str) -> String {
            json!({
                HOOK_EVENT_NAME_FIELD: HOOK_EVENT_SUBAGENT_STOP,
                SESSION_ID_FIELD: session_id,
                TURN_ID_FIELD: turn_id,
                CWD_FIELD: CWD,
                AGENT_ID_FIELD: agent_id,
            })
            .to_string()
        }

        fn session_end_payload(session_id: &str) -> String {
            json!({
                HOOK_EVENT_NAME_FIELD: HOOK_EVENT_SESSION_END,
                SESSION_ID_FIELD: session_id,
                CWD_FIELD: CWD,
            })
            .to_string()
        }

        fn read_state(git_dir: &Path) -> state::AdapterState {
            state::read_state(git_dir).expect("adapter state should be readable")
        }

        #[test]
        fn untracked_mcp_pre_tool_use_creates_no_scope_and_never_touches_seam_or_git_dir() {
            let resolver = |_: &str| -> Result<PathBuf> {
                panic!("an untracked tool must never resolve a git dir")
            };
            let payload = pre_tool_use_json(&[(
                TOOL_NAME_FIELD,
                Value::String("mcp__probe__mutate_success".to_string()),
            )]);

            let output = run_codex_mutation_scope_from_payload_with(
                &payload,
                None,
                &resolver,
                &unreachable_seam,
            )
            .expect("untracked PreToolUse should succeed");

            assert_eq!(output, "");
        }

        #[test]
        fn unknown_tool_pre_tool_use_creates_no_scope_ac3() {
            let resolver = |_: &str| -> Result<PathBuf> {
                panic!("an unknown tool must never resolve a git dir")
            };
            let payload = pre_tool_use_json(&[(
                TOOL_NAME_FIELD,
                Value::String("some_future_codex_tool".to_string()),
            )]);

            let output = run_codex_mutation_scope_from_payload_with(
                &payload,
                None,
                &resolver,
                &unreachable_seam,
            )
            .expect("unknown PreToolUse should succeed");

            assert_eq!(output, "");
        }

        #[test]
        fn delegation_tool_pre_tool_use_creates_no_scope_ac3() {
            let resolver = |_: &str| -> Result<PathBuf> {
                panic!("a delegation tool must never resolve a git dir")
            };
            for tool in ["collaborationspawn_agent", "collaborationwait_agent"] {
                let payload =
                    pre_tool_use_json(&[(TOOL_NAME_FIELD, Value::String(tool.to_string()))]);
                let output = run_codex_mutation_scope_from_payload_with(
                    &payload,
                    None,
                    &resolver,
                    &unreachable_seam,
                )
                .expect("delegation PreToolUse should succeed");
                assert_eq!(output, "", "tool {tool} must produce a neutral continue");
            }
        }

        #[test]
        fn untracked_pre_tool_use_leaves_the_state_store_untouched_ac9b() {
            let git_dir = unique_test_git_dir("untracked-state-untouched");
            std::fs::create_dir_all(&git_dir).expect("git dir should be created");
            let resolver = fixed_resolver(git_dir.clone());

            for tool in ["mcp__probe__mutate_success", "some_future_codex_tool"] {
                let payload =
                    pre_tool_use_json(&[(TOOL_NAME_FIELD, Value::String(tool.to_string()))]);
                run_codex_mutation_scope_from_payload_with(&payload, None, &resolver, &ok_seam)
                    .expect("untracked PreToolUse should succeed");
            }

            assert!(
                state::read_state(&git_dir)
                    .expect("absent state reads as default")
                    .attempts
                    .is_empty(),
                "AC9b: an untracked PreToolUse must not record an attempt"
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn successful_mcp_lifecycle_leaves_no_scope_ac9b() {
            let git_dir = unique_test_git_dir("mcp-success-lifecycle");
            let resolver = fixed_resolver(git_dir.clone());

            let pre = pre_tool_use_json(&[
                (
                    TOOL_NAME_FIELD,
                    Value::String("mcp__probe__mutate_success".to_string()),
                ),
                (TOOL_USE_ID_FIELD, Value::String("exec-mcp".to_string())),
            ]);
            let post = post_tool_use_json(&[
                (
                    TOOL_NAME_FIELD,
                    Value::String("mcp__probe__mutate_success".to_string()),
                ),
                (TOOL_USE_ID_FIELD, Value::String("exec-mcp".to_string())),
            ]);

            run_codex_mutation_scope_from_payload_with(&pre, None, &resolver, &unreachable_seam)
                .expect("MCP PreToolUse should succeed");
            run_codex_mutation_scope_from_payload_with(&post, None, &resolver, &unreachable_seam)
                .expect("MCP PostToolUse should be a no-op");

            assert!(read_state(&git_dir).attempts.is_empty());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn tracked_pre_tool_use_writes_ahead_start_then_returns_continue_ac6() {
            let git_dir = unique_test_git_dir("tracked-write-ahead");
            let resolver = fixed_resolver(git_dir.clone());

            let seen: RefCell<Vec<(String, bool)>> = RefCell::new(Vec::new());
            let seam =
                |root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    let phase_is_pending = state::read_state(&git_dir)
                        .expect("state readable inside seam")
                        .attempts
                        .first()
                        .is_some_and(|attempt| attempt.phase == state::AttemptPhase::PendingStart);
                    seen.borrow_mut()
                        .push((payload.to_string(), root == Path::new(CWD)));
                    assert!(
                        phase_is_pending,
                        "AC6: Start must be driven while the attempt is still PendingStart"
                    );
                    Ok(String::new())
                };

            let output = run_codex_mutation_scope_from_payload_with(
                &pre_tool_use_json(&[]),
                None,
                &resolver,
                &seam,
            )
            .expect("tracked PreToolUse should establish a Start");

            assert_eq!(
                output, "",
                "a successful Start returns Codex's neutral continue"
            );
            let calls = seen.into_inner();
            assert_eq!(calls.len(), 1);
            assert!(calls[0].0.contains(r#""operation":"start""#));
            assert!(calls[0].0.contains(r#""actor_kind":"codex""#));
            assert!(
                calls[0].1,
                "AC6: the seam receives the raw hook cwd as repository_root"
            );

            let final_state = read_state(&git_dir);
            assert_eq!(final_state.attempts.len(), 1);
            assert_eq!(final_state.attempts[0].phase, state::AttemptPhase::Active);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn duplicate_pre_tool_use_reuses_the_same_scope_id_ac4() {
            let git_dir = unique_test_git_dir("duplicate-pre");
            let resolver = fixed_resolver(git_dir.clone());

            run_codex_mutation_scope_from_payload_with(
                &pre_tool_use_json(&[]),
                None,
                &resolver,
                &ok_seam,
            )
            .expect("first PreToolUse should succeed");
            let scope_id_after_first = read_state(&git_dir).attempts[0].scope_id.clone();

            run_codex_mutation_scope_from_payload_with(
                &pre_tool_use_json(&[]),
                None,
                &resolver,
                &ok_seam,
            )
            .expect("replayed PreToolUse should succeed");

            let attempts = read_state(&git_dir).attempts;
            assert_eq!(
                attempts.len(),
                1,
                "AC4: a replayed live PreToolUse must not fork a new attempt"
            );
            assert_eq!(attempts[0].scope_id, scope_id_after_first);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn resolver_failure_denies_with_stable_reason_and_logs_the_detail_ac7() {
            let logger = RecordingLogger::default();
            let resolver = |_: &str| -> Result<PathBuf> {
                Err(anyhow!("boom: git rev-parse --git-dir failed"))
            };

            let output = run_codex_mutation_scope_from_payload_with(
                &pre_tool_use_json(&[]),
                Some(&logger),
                &resolver,
                &unreachable_seam,
            )
            .expect("a resolver failure must still return Ok with a deny payload");

            assert_eq!(output, pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON));
            assert!(
                !output.contains("boom"),
                "the internal detail must never leak to Codex"
            );
            assert!(
                !output.contains("allow"),
                "a fail-closed PreToolUse must never allow"
            );

            let warnings = logger.warnings();
            assert_eq!(warnings.len(), 1);
            assert_eq!(warnings[0].0, PRE_TOOL_USE_FAIL_CLOSED_EVENT);
            assert!(warnings[0].1.contains("boom"));
        }

        #[test]
        fn start_seam_failure_denies_with_stable_reason_and_logs_ac7() {
            let git_dir = unique_test_git_dir("start-seam-failure");
            let resolver = fixed_resolver(git_dir.clone());
            let logger = RecordingLogger::default();
            let seam = seam_failing_on("start");

            let output = run_codex_mutation_scope_from_payload_with(
                &pre_tool_use_json(&[]),
                Some(&logger),
                &resolver,
                &seam,
            )
            .expect("a Start failure must still return Ok with a deny payload");

            assert_eq!(output, pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON));
            assert!(
                !logger.warnings().is_empty(),
                "the Start failure must be logged"
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn delegation_and_untracked_pre_tool_use_are_never_fail_closed_ac7() {
            let resolver = |_: &str| -> Result<PathBuf> { Err(anyhow!("must not be called")) };
            for tool in [
                "mcp__probe__mutate_success",
                "some_future_codex_tool",
                "collaborationspawn_agent",
            ] {
                let payload =
                    pre_tool_use_json(&[(TOOL_NAME_FIELD, Value::String(tool.to_string()))]);
                let output = run_codex_mutation_scope_from_payload_with(
                    &payload,
                    None,
                    &resolver,
                    &unreachable_seam,
                )
                .expect("non-tracked PreToolUse should succeed");
                assert_eq!(
                    output, "",
                    "tool {tool} must not be denied for being untracked"
                );
            }
        }

        #[test]
        fn successful_close_removes_the_attempt_ac8() {
            let git_dir = unique_test_git_dir("close-success");
            let resolver = fixed_resolver(git_dir.clone());

            run_codex_mutation_scope_from_payload_with(
                &pre_tool_use_json(&[]),
                None,
                &resolver,
                &ok_seam,
            )
            .expect("PreToolUse should establish an active attempt");

            let seen: RefCell<Vec<String>> = RefCell::new(Vec::new());
            let seam =
                |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    seen.borrow_mut().push(payload.to_string());
                    Ok(String::new())
                };
            let output = run_codex_mutation_scope_from_payload_with(
                &post_tool_use_json(&[]),
                None,
                &resolver,
                &seam,
            )
            .expect("PostToolUse should close the scope");

            assert_eq!(output, "");
            let calls = seen.into_inner();
            assert_eq!(calls.len(), 1);
            assert!(calls[0].contains(r#""operation":"close""#));
            assert!(read_state(&git_dir).attempts.is_empty());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn pending_start_close_abandons_rather_than_late_starting_d11() {
            let git_dir = unique_test_git_dir("pending-start-close");
            let resolver = fixed_resolver(git_dir.clone());

            state::allocate_attempt(
                &git_dir,
                &AttemptKey {
                    session_id: "session-1".to_string(),
                    agent_id: None,
                    tool_use_id: "exec-1".to_string(),
                },
                "Bash",
            )
            .expect("seeding a pending_start attempt should succeed");

            let seen: RefCell<Vec<String>> = RefCell::new(Vec::new());
            let seam =
                |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    seen.borrow_mut().push(payload.to_string());
                    Ok(String::new())
                };
            run_codex_mutation_scope_from_payload_with(
                &post_tool_use_json(&[]),
                None,
                &resolver,
                &seam,
            )
            .expect("PostToolUse on a pending_start attempt should abandon");

            let calls = seen.into_inner();
            assert_eq!(calls.len(), 1);
            assert!(calls[0].contains(r#""operation":"abandon""#));
            let final_state = read_state(&git_dir);
            assert!(final_state.attempts.is_empty());
            assert!(final_state.recovery_pending);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn failed_close_abandons_and_arms_recovery_ac13() {
            let git_dir = unique_test_git_dir("failed-close");
            let resolver = fixed_resolver(git_dir.clone());

            run_codex_mutation_scope_from_payload_with(
                &pre_tool_use_json(&[]),
                None,
                &resolver,
                &ok_seam,
            )
            .expect("PreToolUse should establish an active attempt");

            let seam = seam_failing_on("close");
            run_codex_mutation_scope_from_payload_with(
                &post_tool_use_json(&[]),
                None,
                &resolver,
                &seam,
            )
            .expect("a failed Close should abandon, not propagate");

            let final_state = read_state(&git_dir);
            assert!(
                final_state.attempts.is_empty(),
                "the abandoned attempt is retired"
            );
            assert!(
                final_state.recovery_pending,
                "D11: a failed Close arms recovery"
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn failed_close_and_failed_abandon_keep_the_attempt_tracked_and_recovery_armed_d11() {
            let git_dir = unique_test_git_dir("failed-close-and-abandon");
            let resolver = fixed_resolver(git_dir.clone());

            run_codex_mutation_scope_from_payload_with(
                &pre_tool_use_json(&[]),
                None,
                &resolver,
                &ok_seam,
            )
            .expect("PreToolUse should establish an active attempt");

            let seam = seam_failing_on_any(vec!["close", "abandon"]);
            let error = run_codex_mutation_scope_from_payload_with(
                &post_tool_use_json(&[]),
                None,
                &resolver,
                &seam,
            )
            .expect_err("a failed Close then failed Abandon must propagate");
            assert!(error.to_string().contains("abandon"));

            let final_state = read_state(&git_dir);
            assert_eq!(final_state.attempts.len(), 1, "the attempt stays tracked");
            assert!(final_state.recovery_pending);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn post_tool_use_with_no_matching_attempt_is_a_noop() {
            let git_dir = unique_test_git_dir("close-no-attempt");
            let resolver = fixed_resolver(git_dir.clone());

            let output = run_codex_mutation_scope_from_payload_with(
                &post_tool_use_json(&[(
                    TOOL_NAME_FIELD,
                    Value::String("mcp__probe__mutate_success".to_string()),
                )]),
                None,
                &resolver,
                &unreachable_seam,
            )
            .expect("a PostToolUse with nothing to close is a no-op");

            assert_eq!(output, "");
            assert!(read_state(&git_dir).attempts.is_empty());

            remove_test_git_dir(&git_dir);
        }

        fn seed_attempt(
            git_dir: &Path,
            session_id: &str,
            agent_id: Option<&str>,
            tool_use_id: &str,
        ) {
            let allocated = state::allocate_attempt(
                git_dir,
                &AttemptKey {
                    session_id: session_id.to_string(),
                    agent_id: agent_id.map(str::to_string),
                    tool_use_id: tool_use_id.to_string(),
                },
                "Bash",
            )
            .expect("seeding an attempt should succeed");
            state::mark_active(git_dir, &allocated.attempt.scope_id)
                .expect("marking the seeded attempt active should succeed");
        }

        #[test]
        fn stop_sweeps_only_main_thread_attempts_d12() {
            let git_dir = unique_test_git_dir("stop-sweep");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt(&git_dir, "session-1", None, "exec-main");
            seed_attempt(&git_dir, "session-1", Some("agent-1"), "exec-agent");

            run_codex_mutation_scope_from_payload_with(
                &turn_scoped_payload(HOOK_EVENT_STOP, "session-1", "turn-1"),
                None,
                &resolver,
                &ok_seam,
            )
            .expect("Stop cleanup should succeed");

            let attempts = read_state(&git_dir).attempts;
            assert_eq!(attempts.len(), 1, "only the subagent attempt survives Stop");
            assert_eq!(attempts[0].tool_use_id, "exec-agent");

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn interrupt_sweeps_every_attempt_for_the_session_d12() {
            let git_dir = unique_test_git_dir("interrupt-sweep");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt(&git_dir, "session-1", None, "exec-main");
            seed_attempt(&git_dir, "session-1", Some("agent-1"), "exec-agent");
            seed_attempt(&git_dir, "session-2", None, "exec-other");

            run_codex_mutation_scope_from_payload_with(
                &turn_scoped_payload(HOOK_EVENT_INTERRUPT, "session-1", "turn-1"),
                None,
                &resolver,
                &ok_seam,
            )
            .expect("Interrupt cleanup should succeed");

            let attempts = read_state(&git_dir).attempts;
            assert_eq!(
                attempts.len(),
                1,
                "SIGINT retires the whole interrupted session"
            );
            assert_eq!(attempts[0].session_id, "session-2");

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn subagent_stop_sweeps_only_the_matching_agent_d12() {
            let git_dir = unique_test_git_dir("subagent-stop-sweep");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt(&git_dir, "session-1", Some("agent-a"), "exec-a");
            seed_attempt(&git_dir, "session-1", Some("agent-b"), "exec-b");
            seed_attempt(&git_dir, "session-1", None, "exec-main");

            run_codex_mutation_scope_from_payload_with(
                &subagent_stop_payload("session-1", "turn-1", "agent-a"),
                None,
                &resolver,
                &ok_seam,
            )
            .expect("SubagentStop cleanup should succeed");

            let mut remaining: Vec<String> = read_state(&git_dir)
                .attempts
                .into_iter()
                .map(|attempt| attempt.tool_use_id)
                .collect();
            remaining.sort();
            assert_eq!(
                remaining,
                vec!["exec-b".to_string(), "exec-main".to_string()]
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn session_end_sweeps_every_attempt_for_the_session_d12() {
            let git_dir = unique_test_git_dir("session-end-sweep");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt(&git_dir, "session-1", None, "exec-main");
            seed_attempt(&git_dir, "session-1", Some("agent-1"), "exec-agent");

            run_codex_mutation_scope_from_payload_with(
                &session_end_payload("session-1"),
                None,
                &resolver,
                &ok_seam,
            )
            .expect("SessionEnd cleanup should succeed");

            assert!(read_state(&git_dir).attempts.is_empty());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn lifecycle_cleanup_with_a_failed_abandon_keeps_the_attempt_tracked_d12() {
            let git_dir = unique_test_git_dir("sweep-failed-abandon");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt(&git_dir, "session-1", None, "exec-main");

            let seam = seam_failing_on("abandon");
            let error = run_codex_mutation_scope_from_payload_with(
                &session_end_payload("session-1"),
                None,
                &resolver,
                &seam,
            )
            .expect_err("a failed abandonment during cleanup must propagate");
            assert!(error.to_string().contains("abandon"));

            let final_state = read_state(&git_dir);
            assert_eq!(final_state.attempts.len(), 1);
            assert!(final_state.recovery_pending);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn recovery_barrier_denies_new_tracked_pre_tool_use_while_attempts_remain_ac12() {
            let git_dir = unique_test_git_dir("barrier-attempts-remain");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt(&git_dir, "session-1", None, "exec-live");
            state::mark_recovery_pending(&git_dir).expect("arming the barrier should succeed");

            let output = run_codex_mutation_scope_from_payload_with(
                &pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("exec-new".to_string()))]),
                None,
                &resolver,
                &unreachable_seam,
            )
            .expect("the barrier denial still returns Ok with a deny payload");

            assert_eq!(output, pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON));

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn recovery_barrier_does_not_affect_untracked_pre_tool_use_ac12() {
            let git_dir = unique_test_git_dir("barrier-untracked-unaffected");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt(&git_dir, "session-1", None, "exec-live");
            state::mark_recovery_pending(&git_dir).expect("arming the barrier should succeed");

            let output = run_codex_mutation_scope_from_payload_with(
                &pre_tool_use_json(&[(
                    TOOL_NAME_FIELD,
                    Value::String("mcp__probe__mutate_success".to_string()),
                )]),
                None,
                &resolver,
                &unreachable_seam,
            )
            .expect("an untracked PreToolUse ignores the barrier");

            assert_eq!(
                output, "",
                "AC12: the barrier never denies an untracked tool"
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn recovery_barrier_flushes_once_quiescent_then_starts_ac12() {
            let git_dir = unique_test_git_dir("barrier-flush-success");
            std::fs::create_dir_all(&git_dir).expect("git dir should be created");
            let resolver = fixed_resolver(git_dir.clone());

            let seeded = state::allocate_attempt(
                &git_dir,
                &AttemptKey {
                    session_id: "session-1".to_string(),
                    agent_id: None,
                    tool_use_id: "exec-seed".to_string(),
                },
                "Bash",
            )
            .expect("seeding a retired attempt should succeed");
            state::mark_recovery_pending(&git_dir).expect("arming the barrier should succeed");
            state::remove_attempt(&git_dir, &seeded.attempt.scope_id)
                .expect("removing the seeded attempt should succeed");

            let seen: RefCell<Vec<String>> = RefCell::new(Vec::new());
            let seam =
                |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    seen.borrow_mut().push(payload.to_string());
                    Ok(String::new())
                };

            let output = run_codex_mutation_scope_from_payload_with(
                &pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("exec-new".to_string()))]),
                None,
                &resolver,
                &seam,
            )
            .expect("a quiescent recovery should flush then proceed");

            assert_eq!(output, "");
            let operations = seen.into_inner();
            assert_eq!(
                operations.len(),
                2,
                "expected flush then start, got {operations:?}"
            );
            assert!(operations[0].contains(r#""operation":"flush""#));
            assert!(operations[1].contains(r#""operation":"start""#));

            let final_state = read_state(&git_dir);
            assert!(
                !final_state.recovery_pending,
                "a successful flush clears the barrier"
            );
            assert_eq!(final_state.attempts.len(), 1);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn recovery_barrier_stays_closed_when_flush_fails_ac12() {
            let git_dir = unique_test_git_dir("barrier-flush-failure");
            std::fs::create_dir_all(&git_dir).expect("git dir should be created");
            let resolver = fixed_resolver(git_dir.clone());

            let seeded = state::allocate_attempt(
                &git_dir,
                &AttemptKey {
                    session_id: "session-1".to_string(),
                    agent_id: None,
                    tool_use_id: "exec-seed".to_string(),
                },
                "Bash",
            )
            .expect("seeding a retired attempt should succeed");
            state::mark_recovery_pending(&git_dir).expect("arming the barrier should succeed");
            state::remove_attempt(&git_dir, &seeded.attempt.scope_id)
                .expect("removing the seeded attempt should succeed");

            let seam = seam_failing_on("flush");
            let output = run_codex_mutation_scope_from_payload_with(
                &pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("exec-new".to_string()))]),
                None,
                &resolver,
                &seam,
            )
            .expect("a failed flush still returns Ok with a deny payload");

            assert_eq!(output, pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON));
            let final_state = read_state(&git_dir);
            assert!(
                final_state.recovery_pending,
                "a failed flush keeps the barrier armed"
            );
            assert!(
                final_state.attempts.is_empty(),
                "a denied PreToolUse allocates nothing"
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn mcp_mutate_then_error_leaves_no_stale_state_ac9c() {
            let git_dir = unique_test_git_dir("mcp-mutate-then-error");
            let resolver = fixed_resolver(git_dir.clone());

            let mcp_pre = pre_tool_use_json(&[
                (
                    TOOL_NAME_FIELD,
                    Value::String("mcp__probe__mutate_then_error".to_string()),
                ),
                (TOOL_USE_ID_FIELD, Value::String("exec-mcp".to_string())),
            ]);
            run_codex_mutation_scope_from_payload_with(
                &mcp_pre,
                None,
                &resolver,
                &unreachable_seam,
            )
            .expect("MCP PreToolUse is a neutral continue");
            assert!(read_state(&git_dir).attempts.is_empty());

            run_codex_mutation_scope_from_payload_with(
                &turn_scoped_payload(HOOK_EVENT_STOP, "session-1", "turn-1"),
                None,
                &resolver,
                &ok_seam,
            )
            .expect("Stop finds nothing to retire");
            run_codex_mutation_scope_from_payload_with(
                &session_end_payload("session-1"),
                None,
                &resolver,
                &ok_seam,
            )
            .expect("SessionEnd finds nothing to retire");

            let final_state = read_state(&git_dir);
            assert!(final_state.attempts.is_empty());
            assert!(
                !final_state.recovery_pending,
                "AC9c: no Start ⇒ no abandon ⇒ no recovery_pending"
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn failed_mcp_then_tracked_successor_starts_clean_ac9d() {
            let git_dir = unique_test_git_dir("mcp-then-tracked");
            let resolver = fixed_resolver(git_dir.clone());

            let mcp_a = pre_tool_use_json(&[
                (
                    TOOL_NAME_FIELD,
                    Value::String("mcp__probe__mutate_then_error".to_string()),
                ),
                (TOOL_USE_ID_FIELD, Value::String("exec-a".to_string())),
            ]);
            let bash_b = pre_tool_use_json(&[
                (TOOL_NAME_FIELD, Value::String("Bash".to_string())),
                (TOOL_USE_ID_FIELD, Value::String("exec-b".to_string())),
            ]);

            run_codex_mutation_scope_from_payload_with(&mcp_a, None, &resolver, &unreachable_seam)
                .expect("MCP A is a neutral continue");
            run_codex_mutation_scope_from_payload_with(&bash_b, None, &resolver, &ok_seam)
                .expect("Bash B starts on its own merits");

            let attempts = read_state(&git_dir).attempts;
            assert_eq!(
                attempts.len(),
                1,
                "AC9d: B is the only live scope, no successor barrier ran"
            );
            assert_eq!(attempts[0].tool_use_id, "exec-b");
            assert_eq!(attempts[0].phase, state::AttemptPhase::Active);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn parallel_mcp_executions_create_no_scopes_ac9e() {
            let git_dir = unique_test_git_dir("parallel-mcp");
            let resolver = fixed_resolver(git_dir.clone());

            for tool_use_id in ["exec-par-a", "exec-par-b"] {
                let payload = pre_tool_use_json(&[
                    (
                        TOOL_NAME_FIELD,
                        Value::String("mcp__probe_par__slow_mutate".to_string()),
                    ),
                    (TOOL_USE_ID_FIELD, Value::String(tool_use_id.to_string())),
                ]);
                run_codex_mutation_scope_from_payload_with(
                    &payload,
                    None,
                    &resolver,
                    &unreachable_seam,
                )
                .expect("each overlapping MCP PreToolUse is a neutral continue");
                assert!(read_state(&git_dir).attempts.is_empty());
            }

            let final_state = read_state(&git_dir);
            assert!(final_state.attempts.is_empty());
            assert!(!final_state.recovery_pending);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn builtin_failed_a_then_b_never_leaves_a_zombie_scope_ac9a() {
            let git_dir = unique_test_git_dir("builtin-failed-a-then-b");
            let resolver = fixed_resolver(git_dir.clone());

            let predecessor_pre =
                pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("exec-a".to_string()))]);
            let predecessor_post =
                post_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("exec-a".to_string()))]);
            let successor_pre =
                pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("exec-b".to_string()))]);

            run_codex_mutation_scope_from_payload_with(&predecessor_pre, None, &resolver, &ok_seam)
                .expect("A starts");
            run_codex_mutation_scope_from_payload_with(
                &predecessor_post,
                None,
                &resolver,
                &ok_seam,
            )
            .expect("A closes on its terminal PostToolUse");
            run_codex_mutation_scope_from_payload_with(&successor_pre, None, &resolver, &ok_seam)
                .expect("B starts");

            let attempts = read_state(&git_dir).attempts;
            assert_eq!(
                attempts.len(),
                1,
                "AC9a: A is retired before B starts, no zombie"
            );
            assert_eq!(attempts[0].tool_use_id, "exec-b");

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn malformed_payload_propagates_as_a_real_error_not_fail_open() {
            let error = run_codex_mutation_scope_from_payload("not json", None).unwrap_err();
            assert!(error.to_string().contains("valid JSON"));
        }

        #[test]
        fn unsupported_event_name_propagates_as_a_real_error() {
            let payload = json!({
                HOOK_EVENT_NAME_FIELD: "UserPromptSubmit",
                SESSION_ID_FIELD: "session-1",
                CWD_FIELD: CWD,
            })
            .to_string();
            let error = run_codex_mutation_scope_from_payload(&payload, None).unwrap_err();
            assert!(error.to_string().contains("unsupported hook_event_name"));
        }
    }
}
