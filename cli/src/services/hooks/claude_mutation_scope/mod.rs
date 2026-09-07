#![allow(dead_code)]

pub(crate) mod state;

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::services::checkout;
use crate::services::observability::traits::Logger;

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

type GitDirResolver<'a> = &'a dyn Fn(&str) -> Result<PathBuf>;

type IngressSeam<'a> = &'a dyn Fn(&Path, &str, Option<&dyn Logger>) -> Result<String>;

const ACTOR_KIND_CLAUDE_CODE: &str = "claude_code";

const FAIL_CLOSED_DENY_REASON: &str =
    "SCE could not establish mutation attribution for this tool execution.";
const EXPLICIT_BACKGROUND_SHELL_DENY_REASON: &str =
    "SCE mutation attribution does not yet support detached background shell execution. Run this command in the foreground.";

const PRE_TOOL_USE_FAIL_CLOSED_EVENT: &str =
    "sce.hooks.claude_mutation_scope.pre_tool_use_fail_closed";

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

pub(crate) fn run_claude_mutation_scope_subcommand(logger: Option<&dyn Logger>) -> Result<String> {
    let stdin_payload = super::read_hook_stdin()?;
    run_claude_mutation_scope_from_payload(&stdin_payload, logger)
}

pub(crate) fn run_claude_mutation_scope_from_payload(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let resolve_git_dir_fn = |cwd: &str| checkout::resolve_git_dir(Path::new(cwd));
    let seam_fn = |repository_root: &Path, payload: &str, logger: Option<&dyn Logger>| {
        super::mutation_scope::run_mutation_scope_from_payload(repository_root, payload, logger)
    };

    run_claude_mutation_scope_from_payload_with(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &seam_fn,
    )
}

fn run_claude_mutation_scope_from_payload_with(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> Result<String> {
    let event = parse_claude_hook_event(stdin_payload)?;
    dispatch_claude_hook_event(event, logger, resolve_git_dir, seam)
}

fn dispatch_claude_hook_event(
    event: ClaudeHookEvent,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> Result<String> {
    match event {
        ClaudeHookEvent::PreToolUse(execution) => Ok(handle_pre_tool_use(
            &execution,
            logger,
            resolve_git_dir,
            seam,
        )),
        ClaudeHookEvent::PostToolUse(identity) | ClaudeHookEvent::PostToolUseFailure(identity) => {
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
        ClaudeHookEvent::PermissionDenied(identity) => {
            let git_dir = resolve_git_dir(&identity.cwd)?;
            let repository_root = Path::new(&identity.cwd);
            handle_permission_denied(
                &git_dir,
                repository_root,
                &identity.attempt_key(),
                logger,
                seam,
            )
        }
        ClaudeHookEvent::Stop(session) | ClaudeHookEvent::StopFailure(session) => {
            let git_dir = resolve_git_dir(&session.cwd)?;
            let repository_root = Path::new(&session.cwd);
            let session_id = session.session_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id && attempt.agent_id.is_none()
            })
        }
        ClaudeHookEvent::UserPromptSubmit(session) => {
            let git_dir = resolve_git_dir(&session.cwd)?;
            let repository_root = Path::new(&session.cwd);
            let session_id = session.session_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id && attempt.agent_id.is_none()
            })
        }
        ClaudeHookEvent::SubagentStop(agent) => {
            let git_dir = resolve_git_dir(&agent.cwd)?;
            let repository_root = Path::new(&agent.cwd);
            let session_id = agent.session_id.clone();
            let agent_id = agent.agent_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id && attempt.agent_id.as_deref() == Some(&agent_id)
            })
        }
        ClaudeHookEvent::SessionEnd(session) => {
            let git_dir = resolve_git_dir(&session.cwd)?;
            let repository_root = Path::new(&session.cwd);
            let session_id = session.session_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id
            })
        }
        ClaudeHookEvent::WorktreeRemove(worktree_remove) => {
            let git_dir = resolve_git_dir(&worktree_remove.worktree_path)?;
            let repository_root = Path::new(&worktree_remove.worktree_path);
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |_attempt| true)
        }
        ClaudeHookEvent::SessionStart | ClaudeHookEvent::SubagentStart => Ok(String::new()),
    }
}

fn handle_pre_tool_use(
    execution: &ClaudeToolExecution,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> String {
    let identity = &execution.identity;

    if matches!(
        classify_tool(&identity.tool_name),
        ToolClassification::ReadOnly | ToolClassification::Delegation
    ) {
        return String::new();
    }

    if is_explicit_background_shell(&identity.tool_name, execution.run_in_background) {
        return pre_tool_use_deny_json(EXPLICIT_BACKGROUND_SHELL_DENY_REASON);
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
    identity: &ClaudeToolIdentity,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<()> {
    let allocated = state::allocate_attempt(git_dir, &identity.attempt_key(), &identity.tool_name)?;
    let scope_id = &allocated.attempt.scope_id;
    let start_payload =
        scope_boundary_payload("start", scope_id, &claude_scope_start_event_id(scope_id));

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
        &claude_scope_close_event_id(&attempt.scope_id),
    );

    if seam(repository_root, &close_payload, logger).is_ok() {
        state::remove_attempt(git_dir, &attempt.scope_id)?;
    } else {
        abandon_attempt(git_dir, repository_root, &attempt, logger, seam)?;
    }
    Ok(String::new())
}

fn handle_permission_denied(
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

    abandon_attempt(git_dir, repository_root, &attempt, logger, seam)?;
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
        "actor_kind": ACTOR_KIND_CLAUDE_CODE,
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

    mod driver {
        use std::cell::RefCell;
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::{Arc, Mutex};

        use anyhow::anyhow;

        use super::*;

        static NEXT_TEST_GIT_DIR_ID: AtomicU64 = AtomicU64::new(0);

        fn unique_test_git_dir(label: &str) -> PathBuf {
            let id = NEXT_TEST_GIT_DIR_ID.fetch_add(1, Ordering::Relaxed);
            std::env::temp_dir().join(format!(
                "sce-claude-mutation-scope-driver-{label}-{}-{id}",
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

        fn session_scoped_payload(event_name: &str, session_id: &str, cwd: &str) -> String {
            let mut object = serde_json::Map::new();
            object.insert(
                HOOK_EVENT_NAME_FIELD.to_string(),
                Value::String(event_name.to_string()),
            );
            object.insert(
                SESSION_ID_FIELD.to_string(),
                Value::String(session_id.to_string()),
            );
            object.insert(CWD_FIELD.to_string(), Value::String(cwd.to_string()));
            Value::Object(object).to_string()
        }

        #[test]
        fn read_only_tool_creates_no_scope_and_never_touches_the_seam_or_git_dir() {
            let resolver = |_: &str| -> Result<PathBuf> {
                panic!("a read-only tool must never resolve a git dir")
            };
            let payload =
                pre_tool_use_json(&[(TOOL_NAME_FIELD, Value::String("Read".to_string()))]);

            let output = run_claude_mutation_scope_from_payload_with(
                &payload,
                None,
                &resolver,
                &unreachable_seam,
            )
            .expect("read-only PreToolUse should succeed");

            assert_eq!(output, "");
        }

        #[test]
        fn delegation_tool_creates_no_scope_ac3() {
            let resolver = |_: &str| -> Result<PathBuf> {
                panic!("Agent delegation must never resolve a git dir")
            };
            let payload =
                pre_tool_use_json(&[(TOOL_NAME_FIELD, Value::String("Agent".to_string()))]);

            let output = run_claude_mutation_scope_from_payload_with(
                &payload,
                None,
                &resolver,
                &unreachable_seam,
            )
            .expect("Agent delegation PreToolUse should succeed");

            assert_eq!(output, "");
        }

        #[test]
        fn session_start_and_subagent_start_establish_no_scope_ac3() {
            let resolver = |_: &str| -> Result<PathBuf> {
                panic!("a lifecycle-only event must never resolve a git dir")
            };

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

                let output = run_claude_mutation_scope_from_payload_with(
                    &payload,
                    None,
                    &resolver,
                    &unreachable_seam,
                )
                .expect("a lifecycle-only event should succeed with no scope");
                assert_eq!(output, "");
            }
        }

        #[test]
        fn explicit_background_bash_is_denied_with_the_exact_reason_d20() {
            let git_dir = unique_test_git_dir("explicit-background-bash");
            let resolver = fixed_resolver(git_dir.clone());
            let payload = pre_tool_use_json(&[
                (TOOL_NAME_FIELD, Value::String("Bash".to_string())),
                (
                    TOOL_INPUT_FIELD,
                    serde_json::json!({ "run_in_background": true }),
                ),
            ]);

            let output = run_claude_mutation_scope_from_payload_with(
                &payload,
                None,
                &resolver,
                &unreachable_seam,
            )
            .expect("an explicit background shell should still return Ok with a deny payload");

            assert_eq!(
                output,
                pre_tool_use_deny_json(EXPLICIT_BACKGROUND_SHELL_DENY_REASON)
            );
            assert!(
                !git_dir.exists(),
                "D20: denial must precede any adapter-state I/O"
            );
        }

        #[test]
        fn explicit_background_powershell_is_denied_ac21() {
            let git_dir = unique_test_git_dir("explicit-background-powershell");
            let resolver = fixed_resolver(git_dir.clone());
            let payload = pre_tool_use_json(&[
                (TOOL_NAME_FIELD, Value::String("PowerShell".to_string())),
                (
                    TOOL_INPUT_FIELD,
                    serde_json::json!({ "run_in_background": true }),
                ),
            ]);

            let output = run_claude_mutation_scope_from_payload_with(
                &payload,
                None,
                &resolver,
                &unreachable_seam,
            )
            .expect("explicit background PowerShell should be denied");

            assert_eq!(
                output,
                pre_tool_use_deny_json(EXPLICIT_BACKGROUND_SHELL_DENY_REASON)
            );
        }

        #[test]
        fn write_ahead_pending_start_persists_before_the_seam_start_call_ac7() {
            let git_dir = unique_test_git_dir("write-ahead");
            let resolver = fixed_resolver(git_dir.clone());
            let git_dir_for_seam = git_dir.clone();
            let phase_seen_before_start: RefCell<Option<state::AttemptPhase>> = RefCell::new(None);
            let observed_roots: RefCell<Vec<PathBuf>> = RefCell::new(Vec::new());
            let seam =
                |root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    if payload.contains(r#""operation":"start""#) {
                        observed_roots.borrow_mut().push(root.to_path_buf());
                        let observed = state::read_state(&git_dir_for_seam)
                            .expect("state should be readable under git_dir inside the seam call");
                        *phase_seen_before_start.borrow_mut() =
                            observed.attempts.first().map(|attempt| attempt.phase);
                    }
                    Ok(String::new())
                };

            let payload = pre_tool_use_json(&[]);
            let output =
                run_claude_mutation_scope_from_payload_with(&payload, None, &resolver, &seam)
                    .expect("mutation-capable PreToolUse should succeed");

            assert_eq!(output, "");
            assert_eq!(
                phase_seen_before_start.into_inner(),
                Some(state::AttemptPhase::PendingStart),
                "AC7: the attempt must be durably pending_start (under git_dir) before the seam Start call"
            );
            assert_eq!(
                observed_roots.into_inner(),
                vec![PathBuf::from("/repo/checkout")],
                "the seam must receive the raw Claude cwd as repository_root, never the resolved git_dir"
            );

            let final_state = state::read_state(&git_dir).expect("state should be readable");
            assert_eq!(final_state.attempts.len(), 1);
            assert_eq!(
                final_state.attempts[0].phase,
                state::AttemptPhase::Active,
                "phase must become active after a successful Start"
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn linked_worktree_cwd_and_git_dir_are_never_conflated_for_pre_tool_use_start() {
            let git_dir = unique_test_git_dir("cwd-vs-git-dir-start");
            let raw_cwd = "/repo/.claude/worktrees/agent-123";
            let resolver = fixed_resolver(git_dir.clone());

            let observed_roots: RefCell<Vec<PathBuf>> = RefCell::new(Vec::new());
            let seam =
                |root: &Path, _payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    observed_roots.borrow_mut().push(root.to_path_buf());
                    Ok(String::new())
                };

            let payload = pre_tool_use_json(&[(CWD_FIELD, Value::String(raw_cwd.to_string()))]);
            run_claude_mutation_scope_from_payload_with(&payload, None, &resolver, &seam)
                .expect("PreToolUse Start should succeed");

            assert_ne!(
                PathBuf::from(raw_cwd),
                git_dir,
                "test sanity: the raw checkout path and the resolved git_dir must be deliberately distinct"
            );
            assert_eq!(
                observed_roots.into_inner(),
                vec![PathBuf::from(raw_cwd)],
                "the ingress seam must receive the raw Claude cwd, never git_dir"
            );

            let state =
                state::read_state(&git_dir).expect("state should be readable under git_dir");
            assert_eq!(
                state.attempts.len(),
                1,
                "adapter bookkeeping must be written under the resolved git_dir"
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn post_tool_use_close_uses_git_dir_for_state_and_raw_cwd_for_the_seam() {
            let git_dir = unique_test_git_dir("cwd-vs-git-dir-close");
            let raw_cwd = "/repo/.claude/worktrees/agent-123";
            let resolver = fixed_resolver(git_dir.clone());

            let pre_payload = pre_tool_use_json(&[(CWD_FIELD, Value::String(raw_cwd.to_string()))]);
            run_claude_mutation_scope_from_payload_with(&pre_payload, None, &resolver, &ok_seam)
                .expect("PreToolUse should establish an active attempt");

            let observed_roots: RefCell<Vec<PathBuf>> = RefCell::new(Vec::new());
            let seam =
                |root: &Path, _payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    observed_roots.borrow_mut().push(root.to_path_buf());
                    Ok(String::new())
                };
            let post_payload = pre_tool_use_json(&[
                (CWD_FIELD, Value::String(raw_cwd.to_string())),
                (
                    HOOK_EVENT_NAME_FIELD,
                    Value::String(HOOK_EVENT_POST_TOOL_USE.to_string()),
                ),
            ]);
            run_claude_mutation_scope_from_payload_with(&post_payload, None, &resolver, &seam)
                .expect("PostToolUse Close should succeed");

            assert_eq!(
                observed_roots.into_inner(),
                vec![PathBuf::from(raw_cwd)],
                "Close must invoke the seam with the raw Claude cwd, never git_dir"
            );

            let state =
                state::read_state(&git_dir).expect("state should be readable under git_dir");
            assert!(
                state.attempts.is_empty(),
                "the closed attempt must be removed from git_dir bookkeeping"
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn abandon_via_permission_denied_uses_git_dir_for_state_and_raw_cwd_for_the_seam() {
            let git_dir = unique_test_git_dir("cwd-vs-git-dir-abandon");
            let raw_cwd = "/repo/.claude/worktrees/agent-123";
            let resolver = fixed_resolver(git_dir.clone());

            let pre_payload = pre_tool_use_json(&[(CWD_FIELD, Value::String(raw_cwd.to_string()))]);
            run_claude_mutation_scope_from_payload_with(&pre_payload, None, &resolver, &ok_seam)
                .expect("PreToolUse should establish an active attempt");

            let observed_roots: RefCell<Vec<PathBuf>> = RefCell::new(Vec::new());
            let seam =
                |root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    if payload.contains(r#""operation":"abandon""#) {
                        observed_roots.borrow_mut().push(root.to_path_buf());
                    }
                    Ok(String::new())
                };
            let denied_payload = pre_tool_use_json(&[
                (CWD_FIELD, Value::String(raw_cwd.to_string())),
                (
                    HOOK_EVENT_NAME_FIELD,
                    Value::String(HOOK_EVENT_PERMISSION_DENIED.to_string()),
                ),
            ]);
            run_claude_mutation_scope_from_payload_with(&denied_payload, None, &resolver, &seam)
                .expect("PermissionDenied should succeed");

            assert_eq!(
                observed_roots.into_inner(),
                vec![PathBuf::from(raw_cwd)],
                "Abandon must invoke the seam with the raw Claude cwd, never git_dir"
            );

            let state =
                state::read_state(&git_dir).expect("state should be readable under git_dir");
            assert!(state.attempts.is_empty());
            assert!(state.recovery_pending);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn recovery_barrier_flush_uses_raw_cwd_for_the_seam_and_git_dir_for_state() {
            let git_dir = unique_test_git_dir("cwd-vs-git-dir-flush");
            let raw_cwd = "/repo/.claude/worktrees/agent-123";
            let resolver = fixed_resolver(git_dir.clone());
            std::fs::create_dir_all(&git_dir).expect("git dir should be created");

            let seeded = state::allocate_attempt(
                &git_dir,
                &AttemptKey {
                    session_id: "session-1".to_string(),
                    agent_id: None,
                    tool_use_id: "toolu_seed".to_string(),
                },
                "Write",
            )
            .expect("seed allocation should succeed");
            state::mark_recovery_pending(&git_dir).expect("arming the barrier should succeed");
            state::remove_attempt(&git_dir, &seeded.attempt.scope_id)
                .expect("removing the seed attempt should succeed");

            let observed_roots: RefCell<Vec<PathBuf>> = RefCell::new(Vec::new());
            let seam =
                |root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    if payload.contains(r#""operation":"flush""#) {
                        observed_roots.borrow_mut().push(root.to_path_buf());
                    }
                    Ok(String::new())
                };

            let new_pre = pre_tool_use_json(&[
                (CWD_FIELD, Value::String(raw_cwd.to_string())),
                (TOOL_USE_ID_FIELD, Value::String("toolu_new".to_string())),
            ]);
            run_claude_mutation_scope_from_payload_with(&new_pre, None, &resolver, &seam)
                .expect("quiescent recovery should flush against the raw checkout path");

            assert_eq!(
                observed_roots.into_inner(),
                vec![PathBuf::from(raw_cwd)],
                "flush must run against the raw Claude cwd, not git_dir"
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn duplicate_live_pre_tool_use_reuses_the_same_scope_and_start_event_id_ac4() {
            let git_dir = unique_test_git_dir("duplicate-delivery");
            let resolver = fixed_resolver(git_dir.clone());
            let start_event_ids: RefCell<Vec<String>> = RefCell::new(Vec::new());
            let seam =
                |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    if payload.contains(r#""operation":"start""#) {
                        let value: Value = serde_json::from_str(payload).unwrap();
                        start_event_ids
                            .borrow_mut()
                            .push(value["event_id"].as_str().unwrap().to_string());
                    }
                    Ok(String::new())
                };

            let payload = pre_tool_use_json(&[]);
            run_claude_mutation_scope_from_payload_with(&payload, None, &resolver, &seam)
                .expect("first delivery should succeed");
            run_claude_mutation_scope_from_payload_with(&payload, None, &resolver, &seam)
                .expect("duplicate delivery should succeed");

            let ids = start_event_ids.into_inner();
            assert_eq!(ids.len(), 2);
            assert_eq!(
                ids[0], ids[1],
                "AC4: duplicate delivery must reuse the same Start EventId"
            );

            let final_state = state::read_state(&git_dir).expect("state should be readable");
            assert_eq!(
                final_state.attempts.len(),
                1,
                "duplicate delivery must not create a second bookkeeping entry"
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn seam_start_failure_denies_and_leaves_the_attempt_pending_start_d8_d11() {
            let git_dir = unique_test_git_dir("start-failure");
            let resolver = fixed_resolver(git_dir.clone());
            let seam = seam_failing_on("start");

            let payload = pre_tool_use_json(&[]);
            let output =
                run_claude_mutation_scope_from_payload_with(&payload, None, &resolver, &seam)
                    .expect(
                        "a Start failure must still return Ok with a deny payload, not propagate",
                    );

            assert_eq!(output, pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON));

            let final_state = state::read_state(&git_dir).expect("state should be readable");
            assert_eq!(final_state.attempts.len(), 1);
            assert_eq!(
                final_state.attempts[0].phase,
                state::AttemptPhase::PendingStart,
                "D11: a failed Start must not be marked active nor removed"
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn pending_start_attempt_is_abandoned_not_late_started_on_a_terminal_signal_d11() {
            let git_dir = unique_test_git_dir("pending-start-then-terminal");
            let resolver = fixed_resolver(git_dir.clone());

            let start_failing_seam = seam_failing_on("start");
            let pre_payload = pre_tool_use_json(&[]);
            run_claude_mutation_scope_from_payload_with(
                &pre_payload,
                None,
                &resolver,
                &start_failing_seam,
            )
            .expect("the failed Start must still return Ok with a deny payload");

            let seen_operations: RefCell<Vec<String>> = RefCell::new(Vec::new());
            let recording =
                |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    seen_operations.borrow_mut().push(payload.to_string());
                    Ok(String::new())
                };
            let post_payload = pre_tool_use_json(&[(
                HOOK_EVENT_NAME_FIELD,
                Value::String(HOOK_EVENT_POST_TOOL_USE.to_string()),
            )]);
            let output = run_claude_mutation_scope_from_payload_with(
                &post_payload,
                None,
                &resolver,
                &recording,
            )
            .expect("PostToolUse for a pending_start attempt should succeed");

            assert_eq!(output, "");
            let operations = seen_operations.into_inner();
            assert_eq!(operations.len(), 1);
            assert!(
                operations[0].contains(r#""operation":"abandon""#),
                "D11: a pending_start attempt must be abandoned, not late-started, got: {operations:?}"
            );

            let final_state = state::read_state(&git_dir).expect("state should be readable");
            assert!(final_state.attempts.is_empty());
            assert!(final_state.recovery_pending);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn close_seam_failure_is_retired_through_abandonment_not_a_replayed_close_d12() {
            let git_dir = unique_test_git_dir("close-failure");
            let resolver = fixed_resolver(git_dir.clone());

            let pre_payload = pre_tool_use_json(&[]);
            run_claude_mutation_scope_from_payload_with(&pre_payload, None, &resolver, &ok_seam)
                .expect("PreToolUse should establish an active attempt");

            let close_failing_seam = seam_failing_on("close");
            let post_payload = pre_tool_use_json(&[(
                HOOK_EVENT_NAME_FIELD,
                Value::String(HOOK_EVENT_POST_TOOL_USE.to_string()),
            )]);
            let output = run_claude_mutation_scope_from_payload_with(
                &post_payload,
                None,
                &resolver,
                &close_failing_seam,
            )
            .expect("a Close failure must still succeed via abandonment");

            assert_eq!(output, "");
            let final_state = state::read_state(&git_dir).expect("state should be readable");
            assert!(
                final_state.attempts.is_empty(),
                "D12: after abandonment the attempt must be retired"
            );
            assert!(final_state.recovery_pending);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn post_tool_use_failure_also_closes_the_scope_d10() {
            let git_dir = unique_test_git_dir("post-tool-use-failure");
            let resolver = fixed_resolver(git_dir.clone());

            let pre_payload = pre_tool_use_json(&[]);
            run_claude_mutation_scope_from_payload_with(&pre_payload, None, &resolver, &ok_seam)
                .expect("PreToolUse should establish an active attempt");

            let seen_operations: RefCell<Vec<String>> = RefCell::new(Vec::new());
            let recording =
                |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    seen_operations.borrow_mut().push(payload.to_string());
                    Ok(String::new())
                };
            let failure_payload = pre_tool_use_json(&[(
                HOOK_EVENT_NAME_FIELD,
                Value::String(HOOK_EVENT_POST_TOOL_USE_FAILURE.to_string()),
            )]);
            let output = run_claude_mutation_scope_from_payload_with(
                &failure_payload,
                None,
                &resolver,
                &recording,
            )
            .expect("PostToolUseFailure should close the scope");

            assert_eq!(output, "");
            let operations = seen_operations.into_inner();
            assert_eq!(operations.len(), 1);
            assert!(operations[0].contains(r#""operation":"close""#));

            let final_state = state::read_state(&git_dir).expect("state should be readable");
            assert!(final_state.attempts.is_empty());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn post_tool_use_with_no_live_attempt_is_a_safe_no_op_d9() {
            let payload = pre_tool_use_json(&[(
                HOOK_EVENT_NAME_FIELD,
                Value::String(HOOK_EVENT_POST_TOOL_USE.to_string()),
            )]);
            let resolver =
                fixed_resolver(std::env::temp_dir().join("sce-unused-nonexistent-git-dir"));

            let output = run_claude_mutation_scope_from_payload_with(
                &payload,
                None,
                &resolver,
                &unreachable_seam,
            )
            .expect("a PostToolUse with no live attempt must be a safe no-op");

            assert_eq!(output, "");
        }

        #[test]
        fn permission_denied_abandons_a_live_attempt_d13() {
            let git_dir = unique_test_git_dir("permission-denied");
            let resolver = fixed_resolver(git_dir.clone());

            let pre_payload = pre_tool_use_json(&[]);
            run_claude_mutation_scope_from_payload_with(&pre_payload, None, &resolver, &ok_seam)
                .expect("PreToolUse should establish an active attempt");

            let seen_operations: RefCell<Vec<String>> = RefCell::new(Vec::new());
            let recording =
                |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    seen_operations.borrow_mut().push(payload.to_string());
                    Ok(String::new())
                };
            let denied_payload = pre_tool_use_json(&[(
                HOOK_EVENT_NAME_FIELD,
                Value::String(HOOK_EVENT_PERMISSION_DENIED.to_string()),
            )]);
            let output = run_claude_mutation_scope_from_payload_with(
                &denied_payload,
                None,
                &resolver,
                &recording,
            )
            .expect("PermissionDenied should succeed");

            assert_eq!(output, "");
            let operations = seen_operations.into_inner();
            assert_eq!(operations.len(), 1);
            assert!(operations[0].contains(r#""operation":"abandon""#));

            let final_state = state::read_state(&git_dir).expect("state should be readable");
            assert!(final_state.attempts.is_empty());
            assert!(final_state.recovery_pending);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn stop_abandons_only_stale_main_thread_attempts_d14() {
            let git_dir = unique_test_git_dir("stop-cleanup");
            let resolver = fixed_resolver(git_dir.clone());

            let main_pre =
                pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("toolu_main".to_string()))]);
            let subagent_pre = pre_tool_use_json(&[
                (TOOL_USE_ID_FIELD, Value::String("toolu_agent".to_string())),
                (AGENT_ID_FIELD, Value::String("agent-1".to_string())),
            ]);
            run_claude_mutation_scope_from_payload_with(&main_pre, None, &resolver, &ok_seam)
                .expect("main-thread PreToolUse should succeed");
            run_claude_mutation_scope_from_payload_with(&subagent_pre, None, &resolver, &ok_seam)
                .expect("subagent PreToolUse should succeed");

            let stop_payload =
                session_scoped_payload(HOOK_EVENT_STOP, "session-1", "/repo/checkout");
            let output = run_claude_mutation_scope_from_payload_with(
                &stop_payload,
                None,
                &resolver,
                &ok_seam,
            )
            .expect("Stop cleanup should succeed");
            assert_eq!(output, "");

            let final_state = state::read_state(&git_dir).expect("state should be readable");
            assert_eq!(
                final_state.attempts.len(),
                1,
                "only the subagent attempt should remain"
            );
            assert_eq!(final_state.attempts[0].tool_use_id, "toolu_agent");
            assert!(
                final_state.recovery_pending,
                "abandoning the stale main attempt must arm the barrier"
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn stop_failure_abandons_stale_main_thread_attempts_the_same_way_d15() {
            let git_dir = unique_test_git_dir("stop-failure-cleanup");
            let resolver = fixed_resolver(git_dir.clone());

            let main_pre =
                pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("toolu_main".to_string()))]);
            run_claude_mutation_scope_from_payload_with(&main_pre, None, &resolver, &ok_seam)
                .expect("main-thread PreToolUse should succeed");

            let stop_failure_payload =
                session_scoped_payload(HOOK_EVENT_STOP_FAILURE, "session-1", "/repo/checkout");
            run_claude_mutation_scope_from_payload_with(
                &stop_failure_payload,
                None,
                &resolver,
                &ok_seam,
            )
            .expect("StopFailure cleanup should succeed");

            let final_state = state::read_state(&git_dir).expect("state should be readable");
            assert!(final_state.attempts.is_empty());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn user_prompt_submit_abandons_only_stale_main_thread_attempts_d16() {
            let git_dir = unique_test_git_dir("user-prompt-submit-cleanup");
            let resolver = fixed_resolver(git_dir.clone());

            let main_pre =
                pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("toolu_main".to_string()))]);
            let subagent_pre = pre_tool_use_json(&[
                (TOOL_USE_ID_FIELD, Value::String("toolu_agent".to_string())),
                (AGENT_ID_FIELD, Value::String("agent-1".to_string())),
            ]);
            run_claude_mutation_scope_from_payload_with(&main_pre, None, &resolver, &ok_seam)
                .expect("main-thread PreToolUse should succeed");
            run_claude_mutation_scope_from_payload_with(&subagent_pre, None, &resolver, &ok_seam)
                .expect("subagent PreToolUse should succeed");

            let prompt_payload = session_scoped_payload(
                HOOK_EVENT_USER_PROMPT_SUBMIT,
                "session-1",
                "/repo/checkout",
            );
            run_claude_mutation_scope_from_payload_with(&prompt_payload, None, &resolver, &ok_seam)
                .expect("UserPromptSubmit cleanup should succeed");

            let final_state = state::read_state(&git_dir).expect("state should be readable");
            assert_eq!(
                final_state.attempts.len(),
                1,
                "the subagent attempt must survive"
            );
            assert_eq!(final_state.attempts[0].tool_use_id, "toolu_agent");

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn subagent_stop_abandons_only_the_matching_agent_id_attempts_d17() {
            let git_dir = unique_test_git_dir("subagent-stop-cleanup");
            let resolver = fixed_resolver(git_dir.clone());

            let first_agent_payload = pre_tool_use_json(&[
                (TOOL_USE_ID_FIELD, Value::String("toolu_a".to_string())),
                (AGENT_ID_FIELD, Value::String("agent-a".to_string())),
            ]);
            let second_agent_payload = pre_tool_use_json(&[
                (TOOL_USE_ID_FIELD, Value::String("toolu_b".to_string())),
                (AGENT_ID_FIELD, Value::String("agent-b".to_string())),
            ]);
            run_claude_mutation_scope_from_payload_with(
                &first_agent_payload,
                None,
                &resolver,
                &ok_seam,
            )
            .expect("agent-a PreToolUse should succeed");
            run_claude_mutation_scope_from_payload_with(
                &second_agent_payload,
                None,
                &resolver,
                &ok_seam,
            )
            .expect("agent-b PreToolUse should succeed");

            let mut object = serde_json::Map::new();
            object.insert(
                HOOK_EVENT_NAME_FIELD.to_string(),
                Value::String(HOOK_EVENT_SUBAGENT_STOP.to_string()),
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
                AGENT_ID_FIELD.to_string(),
                Value::String("agent-a".to_string()),
            );
            let subagent_stop_payload = Value::Object(object).to_string();

            run_claude_mutation_scope_from_payload_with(
                &subagent_stop_payload,
                None,
                &resolver,
                &ok_seam,
            )
            .expect("SubagentStop cleanup should succeed");

            let final_state = state::read_state(&git_dir).expect("state should be readable");
            assert_eq!(final_state.attempts.len(), 1);
            assert_eq!(final_state.attempts[0].tool_use_id, "toolu_b");

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn session_end_abandons_every_attempt_regardless_of_agent_id_d18() {
            let git_dir = unique_test_git_dir("session-end-cleanup");
            let resolver = fixed_resolver(git_dir.clone());

            let main_pre =
                pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("toolu_main".to_string()))]);
            let subagent_pre = pre_tool_use_json(&[
                (TOOL_USE_ID_FIELD, Value::String("toolu_agent".to_string())),
                (AGENT_ID_FIELD, Value::String("agent-1".to_string())),
            ]);
            run_claude_mutation_scope_from_payload_with(&main_pre, None, &resolver, &ok_seam)
                .expect("main-thread PreToolUse should succeed");
            run_claude_mutation_scope_from_payload_with(&subagent_pre, None, &resolver, &ok_seam)
                .expect("subagent PreToolUse should succeed");

            let session_end_payload =
                session_scoped_payload(HOOK_EVENT_SESSION_END, "session-1", "/repo/checkout");
            run_claude_mutation_scope_from_payload_with(
                &session_end_payload,
                None,
                &resolver,
                &ok_seam,
            )
            .expect("SessionEnd cleanup should succeed");

            let final_state = state::read_state(&git_dir).expect("state should be readable");
            assert!(final_state.attempts.is_empty());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn worktree_remove_resolves_git_dir_from_worktree_path_not_cwd_d22() {
            let main_git_dir = unique_test_git_dir("worktree-remove-main");
            let worktree_git_dir = unique_test_git_dir("worktree-remove-isolated");
            let main_git_dir_for_resolver = main_git_dir.clone();
            let worktree_git_dir_for_resolver = worktree_git_dir.clone();

            let resolver = move |cwd: &str| -> Result<PathBuf> {
                if cwd == "/repo/.claude/worktrees/agent-1" {
                    Ok(worktree_git_dir_for_resolver.clone())
                } else {
                    Ok(main_git_dir_for_resolver.clone())
                }
            };

            let subagent_pre = pre_tool_use_json(&[
                (
                    CWD_FIELD,
                    Value::String("/repo/.claude/worktrees/agent-1".to_string()),
                ),
                (AGENT_ID_FIELD, Value::String("agent-1".to_string())),
            ]);
            run_claude_mutation_scope_from_payload_with(&subagent_pre, None, &resolver, &ok_seam)
                .expect("isolated-worktree PreToolUse should succeed");

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
            let worktree_remove_payload = Value::Object(object).to_string();

            run_claude_mutation_scope_from_payload_with(
                &worktree_remove_payload,
                None,
                &resolver,
                &ok_seam,
            )
            .expect("WorktreeRemove cleanup should succeed");

            let worktree_state =
                state::read_state(&worktree_git_dir).expect("worktree state should be readable");
            assert!(
                worktree_state.attempts.is_empty(),
                "D22: WorktreeRemove must retire attempts under the worktree_path's git dir"
            );

            remove_test_git_dir(&main_git_dir);
            remove_test_git_dir(&worktree_git_dir);
        }

        #[test]
        fn recovery_barrier_denies_new_mutation_capable_pre_tool_use_while_attempts_remain_d19() {
            let git_dir = unique_test_git_dir("barrier-attempts-remain");
            std::fs::create_dir_all(&git_dir).expect("git dir should be created");
            let resolver = fixed_resolver(git_dir.clone());

            let surviving = state::allocate_attempt(
                &git_dir,
                &AttemptKey {
                    session_id: "session-1".to_string(),
                    agent_id: None,
                    tool_use_id: "toolu_surviving".to_string(),
                },
                "Write",
            )
            .expect("seeding a surviving attempt should succeed");
            let retiring = state::allocate_attempt(
                &git_dir,
                &AttemptKey {
                    session_id: "session-1".to_string(),
                    agent_id: None,
                    tool_use_id: "toolu_retiring".to_string(),
                },
                "Write",
            )
            .expect("seeding a retiring attempt should succeed");
            state::mark_recovery_pending(&git_dir).expect("arming the barrier should succeed");
            state::remove_attempt(&git_dir, &retiring.attempt.scope_id)
                .expect("removing the retiring attempt should succeed");
            let _ = surviving;

            let new_pre =
                pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("toolu_new".to_string()))]);
            let output = run_claude_mutation_scope_from_payload_with(
                &new_pre,
                None,
                &resolver,
                &unreachable_seam,
            )
            .expect("D19: barrier denial must still return Ok with a deny payload");

            assert_eq!(output, pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON));

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn recovery_barrier_flushes_once_quiescent_and_clears_before_starting_d19() {
            let git_dir = unique_test_git_dir("barrier-flush-success");
            std::fs::create_dir_all(&git_dir).expect("git dir should be created");
            let resolver = fixed_resolver(git_dir.clone());

            let seeded = state::allocate_attempt(
                &git_dir,
                &AttemptKey {
                    session_id: "session-1".to_string(),
                    agent_id: None,
                    tool_use_id: "toolu_seed".to_string(),
                },
                "Write",
            )
            .expect("seeding the retired attempt should succeed");
            state::mark_recovery_pending(&git_dir).expect("arming the barrier should succeed");
            state::remove_attempt(&git_dir, &seeded.attempt.scope_id)
                .expect("removing the seeded attempt should succeed");

            let seen_operations: RefCell<Vec<String>> = RefCell::new(Vec::new());
            let seam =
                |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    seen_operations.borrow_mut().push(payload.to_string());
                    Ok(String::new())
                };

            let new_pre =
                pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("toolu_new".to_string()))]);
            let output =
                run_claude_mutation_scope_from_payload_with(&new_pre, None, &resolver, &seam)
                    .expect("D19: a quiescent recovery should flush then proceed");

            assert_eq!(output, "");
            let operations = seen_operations.into_inner();
            assert_eq!(
                operations.len(),
                2,
                "expected flush then start, got: {operations:?}"
            );
            assert!(operations[0].contains(r#""operation":"flush""#));
            assert!(operations[1].contains(r#""operation":"start""#));

            let final_state = state::read_state(&git_dir).expect("state should be readable");
            assert!(
                !final_state.recovery_pending,
                "a successful flush must clear the barrier"
            );
            assert_eq!(final_state.attempts.len(), 1);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn recovery_barrier_stays_fail_closed_when_flush_fails_d19() {
            let git_dir = unique_test_git_dir("barrier-flush-failure");
            std::fs::create_dir_all(&git_dir).expect("git dir should be created");
            let resolver = fixed_resolver(git_dir.clone());

            let seeded = state::allocate_attempt(
                &git_dir,
                &AttemptKey {
                    session_id: "session-1".to_string(),
                    agent_id: None,
                    tool_use_id: "toolu_seed".to_string(),
                },
                "Write",
            )
            .expect("seeding the retired attempt should succeed");
            state::mark_recovery_pending(&git_dir).expect("arming the barrier should succeed");
            state::remove_attempt(&git_dir, &seeded.attempt.scope_id)
                .expect("removing the seeded attempt should succeed");

            let seam = seam_failing_on("flush");
            let new_pre =
                pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("toolu_new".to_string()))]);
            let output =
                run_claude_mutation_scope_from_payload_with(&new_pre, None, &resolver, &seam)
                    .expect("D19: a failed flush must still return Ok with a deny payload");

            assert_eq!(output, pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON));

            let final_state = state::read_state(&git_dir).expect("state should be readable");
            assert!(
                final_state.recovery_pending,
                "a failed flush must keep the barrier armed"
            );
            assert!(
                final_state.attempts.is_empty(),
                "a denied PreToolUse must not allocate a new attempt"
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn failed_close_and_failed_abandon_keep_recovery_armed_and_the_attempt_tracked_d12_d19() {
            let git_dir = unique_test_git_dir("close-and-abandon-failure");
            let resolver = fixed_resolver(git_dir.clone());

            let pre_payload = pre_tool_use_json(&[]);
            run_claude_mutation_scope_from_payload_with(&pre_payload, None, &resolver, &ok_seam)
                .expect("PreToolUse should establish an active attempt");

            let failing_seam = seam_failing_on_any(vec!["close", "abandon"]);
            let post_payload = pre_tool_use_json(&[(
                HOOK_EVENT_NAME_FIELD,
                Value::String(HOOK_EVENT_POST_TOOL_USE.to_string()),
            )]);
            let error = run_claude_mutation_scope_from_payload_with(
                &post_payload,
                None,
                &resolver,
                &failing_seam,
            )
            .expect_err(
                "a failed Close followed by a failed Abandon must propagate, not silently succeed",
            );
            assert!(error.to_string().contains("abandon"));

            let final_state = state::read_state(&git_dir).expect("state should be readable");
            assert_eq!(
                final_state.attempts.len(),
                1,
                "D12: an attempt whose abandonment failed must remain tracked"
            );
            assert!(
                final_state.recovery_pending,
                "D19: recovery must be armed even though abandonment itself failed"
            );

            let new_pre =
                pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("toolu_new".to_string()))]);
            let output = run_claude_mutation_scope_from_payload_with(
                &new_pre,
                None,
                &resolver,
                &unreachable_seam,
            )
            .expect("the barrier denial must still return Ok with a deny payload");
            assert_eq!(
                output,
                pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON),
                "the next mutation-capable PreToolUse must be denied, and no new Start may occur"
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn lifecycle_cleanup_with_a_failed_abandon_keeps_recovery_armed_and_the_attempt_tracked() {
            let git_dir = unique_test_git_dir("lifecycle-cleanup-abandon-failure");
            let resolver = fixed_resolver(git_dir.clone());

            let main_pre =
                pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("toolu_main".to_string()))]);
            run_claude_mutation_scope_from_payload_with(&main_pre, None, &resolver, &ok_seam)
                .expect("main-thread PreToolUse should succeed");

            let abandon_failing_seam = seam_failing_on("abandon");
            let stop_payload =
                session_scoped_payload(HOOK_EVENT_STOP, "session-1", "/repo/checkout");
            let error = run_claude_mutation_scope_from_payload_with(
                &stop_payload,
                None,
                &resolver,
                &abandon_failing_seam,
            )
            .expect_err("a failed abandonment during Stop cleanup must propagate");
            assert!(error.to_string().contains("abandon"));

            let final_state = state::read_state(&git_dir).expect("state should be readable");
            assert_eq!(
                final_state.attempts.len(),
                1,
                "the attempt whose abandonment failed must remain tracked"
            );
            assert!(
                final_state.recovery_pending,
                "the barrier must remain armed even though cleanup abandonment failed"
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn resolver_failure_logs_the_detailed_error_and_denies_with_the_stable_reason_ac8_d8() {
            let logger = RecordingLogger::default();
            let resolver = |_: &str| -> Result<PathBuf> {
                Err(anyhow!("boom: git rev-parse --git-dir failed"))
            };

            let payload = pre_tool_use_json(&[]);
            let output = run_claude_mutation_scope_from_payload_with(
                &payload,
                Some(&logger),
                &resolver,
                &unreachable_seam,
            )
            .expect("a resolver failure must still return Ok with a deny payload");

            assert_eq!(output, pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON));
            assert!(
                !output.contains("boom"),
                "the detailed internal error must never leak into Claude's deny reason"
            );
            assert!(
                !output.contains("allow"),
                "a fail-closed PreToolUse must never emit an allow decision"
            );

            let warnings = logger.warnings();
            assert_eq!(warnings.len(), 1);
            assert!(
                warnings[0].1.contains("boom"),
                "the detailed error must be logged for operators, got: {warnings:?}"
            );
        }

        #[test]
        fn start_seam_failure_logs_the_detailed_error_and_denies_with_the_stable_reason() {
            let git_dir = unique_test_git_dir("start-failure-logged");
            let resolver = fixed_resolver(git_dir.clone());
            let logger = RecordingLogger::default();
            let seam = seam_failing_on("start");

            let payload = pre_tool_use_json(&[]);
            let output = run_claude_mutation_scope_from_payload_with(
                &payload,
                Some(&logger),
                &resolver,
                &seam,
            )
            .expect("a Start failure must still return Ok with a deny payload");

            assert_eq!(output, pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON));

            let warnings = logger.warnings();
            assert!(!warnings.is_empty(), "the Start failure must be logged");
            assert!(warnings
                .iter()
                .any(|(_, message)| message.to_lowercase().contains("start")));

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn recovery_barrier_flush_failure_logs_the_detailed_error() {
            let git_dir = unique_test_git_dir("barrier-flush-failure-logged");
            std::fs::create_dir_all(&git_dir).expect("git dir should be created");
            let resolver = fixed_resolver(git_dir.clone());
            let logger = RecordingLogger::default();

            let seeded = state::allocate_attempt(
                &git_dir,
                &AttemptKey {
                    session_id: "session-1".to_string(),
                    agent_id: None,
                    tool_use_id: "toolu_seed".to_string(),
                },
                "Write",
            )
            .expect("seed allocation should succeed");
            state::mark_recovery_pending(&git_dir).expect("arming the barrier should succeed");
            state::remove_attempt(&git_dir, &seeded.attempt.scope_id)
                .expect("removing the seeded attempt should succeed");

            let seam = seam_failing_on("flush");
            let new_pre =
                pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("toolu_new".to_string()))]);
            let output = run_claude_mutation_scope_from_payload_with(
                &new_pre,
                Some(&logger),
                &resolver,
                &seam,
            )
            .expect("a failed flush must still return Ok with a deny payload");

            assert_eq!(output, pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON));

            let warnings = logger.warnings();
            assert!(!warnings.is_empty(), "the flush failure must be logged");

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn malformed_payload_propagates_as_a_real_error_not_fail_open() {
            let error = run_claude_mutation_scope_from_payload("not json", None).unwrap_err();
            assert!(error.to_string().contains("valid JSON"));
        }
    }
}
