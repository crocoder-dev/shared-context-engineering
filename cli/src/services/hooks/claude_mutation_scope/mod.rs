#![allow(dead_code)]

pub(crate) mod health;
pub(crate) mod state;

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::services::mutation_trace::runtime::resolve_git_dir;
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
const MODEL_STATE_UNAVAILABLE_EVENT: &str =
    "sce.hooks.claude_mutation_scope.model_state_unavailable";

type ClaudeModelStateResolver<'a> = &'a dyn Fn(&Path, &str, &str) -> Result<Option<String>>;

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
    let resolve_git_dir_fn = |cwd: &str| resolve_git_dir(Path::new(cwd));
    let model_state_resolver =
        |repository_root: &Path, session_id: &str, agent_id: &str| -> Result<Option<String>> {
            let db = super::open_agent_trace_db_for_hook_runtime(
                repository_root,
                "Failed to open Agent Trace DB for Claude mutation-scope model resolution.",
            )?;
            Ok(db
                .claude_model_state_by_session_and_agent(session_id, agent_id)?
                .map(|state| state.model_id))
        };
    let seam_fn = |repository_root: &Path, payload: &str, logger: Option<&dyn Logger>| {
        super::mutation_scope::run_mutation_scope_from_payload(repository_root, payload, logger)
    };

    run_claude_mutation_scope_from_payload_with_resolver(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &model_state_resolver,
        &seam_fn,
    )
}

#[cfg(test)]
pub(crate) fn run_claude_mutation_scope_from_payload_at_state_root(
    state_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let resolve_git_dir_fn = |cwd: &str| resolve_git_dir(Path::new(cwd));
    let model_state_root = state_root.to_path_buf();
    let seam_state_root = state_root.to_path_buf();
    let model_state_resolver =
        move |repository_root: &Path, session_id: &str, agent_id: &str| -> Result<Option<String>> {
            let db = super::open_agent_trace_db_for_hook_runtime_at_state_root(
                repository_root,
                &model_state_root,
                "Failed to open Agent Trace DB for Claude mutation-scope model resolution.",
            )?;
            Ok(db
                .claude_model_state_by_session_and_agent(session_id, agent_id)?
                .map(|state| state.model_id))
        };
    let seam_fn = |repository_root: &Path, payload: &str, logger: Option<&dyn Logger>| {
        super::mutation_scope::run_mutation_scope_from_payload_at_state_root(
            repository_root,
            &seam_state_root,
            payload,
            logger,
        )
    };

    run_claude_mutation_scope_from_payload_with_resolver(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &model_state_resolver,
        &seam_fn,
    )
}

#[cfg(test)]
fn run_claude_mutation_scope_from_payload_with(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> Result<String> {
    let unavailable_model_state = |_repository_root: &Path,
                                   _session_id: &str,
                                   _agent_id: &str|
     -> Result<Option<String>> { Ok(None) };
    run_claude_mutation_scope_from_payload_with_resolver(
        stdin_payload,
        logger,
        resolve_git_dir,
        &unavailable_model_state,
        seam,
    )
}

fn run_claude_mutation_scope_from_payload_with_resolver(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    model_state_resolver: ClaudeModelStateResolver,
    seam: IngressSeam,
) -> Result<String> {
    let event = parse_claude_hook_event(stdin_payload)?;
    dispatch_claude_hook_event(event, logger, resolve_git_dir, model_state_resolver, seam)
}

fn dispatch_claude_hook_event(
    event: ClaudeHookEvent,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    model_state_resolver: ClaudeModelStateResolver,
    seam: IngressSeam,
) -> Result<String> {
    match event {
        ClaudeHookEvent::PreToolUse(execution) => Ok(handle_pre_tool_use(
            &execution,
            logger,
            resolve_git_dir,
            model_state_resolver,
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
    model_state_resolver: ClaudeModelStateResolver,
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

    match establish_start(
        &git_dir,
        repository_root,
        identity,
        logger,
        model_state_resolver,
        seam,
    ) {
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
    model_state_resolver: ClaudeModelStateResolver,
    seam: IngressSeam,
) -> Result<()> {
    let allocated = state::allocate_attempt(git_dir, &identity.attempt_key(), &identity.tool_name)?;
    let scope_id = &allocated.attempt.scope_id;
    let canonical_session_id =
        super::prefixed_diff_trace_session_id(super::CLAUDE_TOOL_NAME, &identity.session_id);
    let agent_id = identity.agent_id.as_deref().unwrap_or("");
    let model_id = match model_state_resolver(repository_root, &canonical_session_id, agent_id) {
        Ok(model_id) => model_id.and_then(|model| super::normalize_claude_model_id(&model)),
        Err(error) => {
            if let Some(log) = logger {
                log.warn(
                    MODEL_STATE_UNAVAILABLE_EVENT,
                    &error.to_string(),
                    &[("agent_id", agent_id)],
                    Some(&canonical_session_id),
                );
            }
            None
        }
    };
    let start_payload = scope_start_payload(
        scope_id,
        &claude_scope_start_event_id(scope_id),
        &canonical_session_id,
        model_id.as_deref(),
    );

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

fn scope_start_payload(
    scope_id: &str,
    event_id: &str,
    session_id: &str,
    model_id: Option<&str>,
) -> String {
    json!({
        "operation": "start",
        "scope_id": scope_id,
        "event_id": event_id,
        "actor_kind": ACTOR_KIND_CLAUDE_CODE,
        "provenance": {
            "session_id": session_id,
            "model_id": model_id,
        },
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
mod tests;
