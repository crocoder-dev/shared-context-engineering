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
    let resolve_git_dir_fn = |cwd: &str| checkout::resolve_git_dir(Path::new(cwd));
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
    let resolve_git_dir_fn = |cwd: &str| checkout::resolve_git_dir(Path::new(cwd));
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
        use std::cell::{Cell, RefCell};
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

        fn start_with_model_resolver(
            payload: &str,
            logger: Option<&dyn Logger>,
            resolve_git_dir: GitDirResolver,
            model_state_resolver: ClaudeModelStateResolver,
            seam: IngressSeam,
        ) -> Result<String> {
            run_claude_mutation_scope_from_payload_with_resolver(
                payload,
                logger,
                resolve_git_dir,
                model_state_resolver,
                seam,
            )
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
        fn pre_tool_use_resolves_main_and_subagent_model_state_exactly_at_admission() {
            let git_dir = unique_test_git_dir("model-state-admission");
            let git_dir_resolver = fixed_resolver(git_dir.clone());
            let resolver_calls: RefCell<Vec<(String, String)>> = RefCell::new(Vec::new());
            let model_state_resolver = |_: &Path, session_id: &str, agent_id: &str| {
                resolver_calls
                    .borrow_mut()
                    .push((session_id.to_string(), agent_id.to_string()));
                Ok(Some(if agent_id.is_empty() {
                    "claude/sonnet".to_string()
                } else {
                    "claude/opus".to_string()
                }))
            };
            let starts: RefCell<Vec<Value>> = RefCell::new(Vec::new());
            let seam =
                |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    let value: Value = serde_json::from_str(payload).expect("payload is JSON");
                    if value.get("operation") == Some(&Value::String("start".to_string())) {
                        starts.borrow_mut().push(value);
                    }
                    Ok(String::new())
                };

            let main_payload = pre_tool_use_json(&[]);
            let subagent_payload = pre_tool_use_json(&[
                (
                    TOOL_USE_ID_FIELD,
                    Value::String("toolu_subagent".to_string()),
                ),
                (AGENT_ID_FIELD, Value::String("agent-1".to_string())),
            ]);
            start_with_model_resolver(
                &main_payload,
                None,
                &git_dir_resolver,
                &model_state_resolver,
                &seam,
            )
            .expect("main-agent Start should succeed");
            start_with_model_resolver(
                &subagent_payload,
                None,
                &git_dir_resolver,
                &model_state_resolver,
                &seam,
            )
            .expect("subagent Start should succeed");

            assert_eq!(
                resolver_calls.into_inner(),
                vec![
                    ("cc_session-1".to_string(), String::new()),
                    ("cc_session-1".to_string(), "agent-1".to_string()),
                ]
            );
            let starts = starts.into_inner();
            assert_eq!(starts.len(), 2);
            assert_eq!(
                starts[0]["provenance"],
                json!({"session_id": "cc_session-1", "model_id": "claude/sonnet"})
            );
            assert_eq!(
                starts[1]["provenance"],
                json!({"session_id": "cc_session-1", "model_id": "claude/opus"})
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn subagent_without_exact_model_state_does_not_inherit_main_model() {
            let git_dir = unique_test_git_dir("subagent-model-state-missing");
            let git_dir_resolver = fixed_resolver(git_dir.clone());
            let starts: RefCell<Vec<Value>> = RefCell::new(Vec::new());
            let model_state_resolver = |_: &Path, _: &str, agent_id: &str| {
                Ok(if agent_id.is_empty() {
                    Some("claude/sonnet".to_string())
                } else {
                    None
                })
            };
            let seam =
                |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    starts
                        .borrow_mut()
                        .push(serde_json::from_str(payload).expect("payload is JSON"));
                    Ok(String::new())
                };

            for (tool_use_id, agent_id) in
                [("toolu_main", None), ("toolu_subagent", Some("agent-1"))]
            {
                let overrides = agent_id
                    .map(|agent_id| vec![(AGENT_ID_FIELD, Value::String(agent_id.to_string()))])
                    .unwrap_or_default();
                let mut overrides = overrides;
                overrides.push((TOOL_USE_ID_FIELD, Value::String(tool_use_id.to_string())));
                let payload = pre_tool_use_json(&overrides);
                start_with_model_resolver(
                    &payload,
                    None,
                    &git_dir_resolver,
                    &model_state_resolver,
                    &seam,
                )
                .expect("both Starts should succeed");
            }

            let starts = starts.into_inner();
            assert_eq!(starts.len(), 2);
            assert_eq!(starts[0]["provenance"]["model_id"], "claude/sonnet");
            assert_eq!(starts[1]["provenance"]["session_id"], "cc_session-1");
            assert_eq!(starts[1]["provenance"]["model_id"], Value::Null);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn missing_or_failed_model_resolution_keeps_session_provenance_and_allows_start() {
            let git_dir = unique_test_git_dir("model-state-unavailable");
            let git_dir_resolver = fixed_resolver(git_dir.clone());
            let starts: RefCell<Vec<Value>> = RefCell::new(Vec::new());
            let seam =
                |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    starts
                        .borrow_mut()
                        .push(serde_json::from_str(payload).expect("payload is JSON"));
                    Ok(String::new())
                };
            let response_index = Cell::new(0);
            let model_state_resolver = |_: &Path, _: &str, _: &str| -> Result<Option<String>> {
                let index = response_index.get();
                response_index.set(index + 1);
                if index == 0 {
                    Ok(None)
                } else {
                    Err(anyhow!("local model-state DB is unavailable"))
                }
            };

            for tool_use_id in ["toolu_missing", "toolu_failed"] {
                let payload = pre_tool_use_json(&[(
                    TOOL_USE_ID_FIELD,
                    Value::String(tool_use_id.to_string()),
                )]);
                let output = start_with_model_resolver(
                    &payload,
                    None,
                    &git_dir_resolver,
                    &model_state_resolver,
                    &seam,
                )
                .expect("model unavailability must not fail Start");
                assert_eq!(output, "");
            }

            let starts = starts.into_inner();
            assert_eq!(starts.len(), 2);
            for start in starts {
                assert_eq!(
                    start["provenance"],
                    json!({"session_id": "cc_session-1", "model_id": null})
                );
            }

            remove_test_git_dir(&git_dir);
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

    mod production_regressions {
        use std::fs;
        use std::path::{Path, PathBuf};
        use std::process::Command;

        use super::*;
        use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
        use crate::services::agent_trace_db::{ClaudeModelStateObservation, ObservationKind};
        use crate::services::agent_trace_storage::{
            resolve_agent_trace_storage_at_state_root, AgentTraceStorageContext,
        };
        use crate::services::checkout::{get_or_create_checkout_id, resolve_git_dir};
        use crate::services::mutation_trace::store::decode_revision;

        fn git(dir: &Path, args: &[&str]) -> String {
            let output = Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .expect("git should spawn");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).expect("git output should be UTF-8")
        }

        struct ClaudeRepo {
            temp: tempfile::TempDir,
            root: PathBuf,
            state_root: PathBuf,
        }

        impl ClaudeRepo {
            fn new(label: &str) -> Self {
                let temp = tempfile::Builder::new()
                    .prefix(&format!("sce-claude-mutation-scope-regression-{label}-"))
                    .tempdir()
                    .expect("temp dir should be created");
                let root = temp.path().join("repo");
                fs::create_dir_all(&root).expect("repo dir should be created");
                git(&root, &["init", "-q"]);
                git(&root, &["config", "user.email", "test@example.invalid"]);
                git(&root, &["config", "user.name", "SCE Test"]);
                git(
                    &root,
                    &["remote", "add", "origin", "git@github.com:acme/widgets.git"],
                );
                fs::write(root.join("file.txt"), "one\n").expect("seed file should write");
                git(&root, &["add", "-A"]);
                git(&root, &["commit", "-qm", "base"]);

                let state_root = temp.path().join("state");
                fs::create_dir_all(&state_root).expect("state root should be created");
                resolve_agent_trace_storage_at_state_root(
                    &AgentTraceStorageContext {
                        repository_root: &root,
                        explicit_repository_id: None,
                        repository_remote: "origin",
                    },
                    &state_root,
                )
                .expect("state-root storage should initialize the repository DB");

                Self {
                    temp,
                    root,
                    state_root,
                }
            }

            fn drive(&self, payload: &str) -> Result<String> {
                run_claude_mutation_scope_from_payload_at_state_root(
                    &self.state_root,
                    payload,
                    None,
                )
            }

            fn drive_generic(&self, payload: &str) -> Result<String> {
                crate::services::hooks::mutation_scope::run_mutation_scope_from_payload_at_state_root(
                    &self.root,
                    &self.state_root,
                    payload,
                    None,
                )
            }

            fn drive_flush(&self) -> Result<String> {
                self.drive_generic(&flush_payload())
            }

            fn db(&self) -> RepositoryAgentTraceDb {
                crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                    &self.root,
                    &self.state_root,
                    "claude mutation-scope regression test assertions",
                )
                .expect("assertion DB should open")
            }

            fn cwd(&self) -> String {
                self.root.to_string_lossy().into_owned()
            }

            fn cwd_at(root: &Path) -> String {
                root.to_string_lossy().into_owned()
            }

            fn working_tree_at(root: &Path) -> String {
                git(root, &["add", "-A"]);
                git(root, &["write-tree"]).trim().to_owned()
            }

            fn working_tree(&self) -> String {
                Self::working_tree_at(&self.root)
            }

            fn git_dir_at(root: &Path) -> PathBuf {
                resolve_git_dir(root).expect("git dir should resolve")
            }

            fn git_dir(&self) -> PathBuf {
                Self::git_dir_at(&self.root)
            }

            fn adapter_state_at(root: &Path) -> state::AdapterState {
                state::read_state(&Self::git_dir_at(root))
                    .expect("adapter state should be readable")
            }

            fn adapter_state(&self) -> state::AdapterState {
                Self::adapter_state_at(&self.root)
            }

            fn worktree_id_at(root: &Path) -> String {
                get_or_create_checkout_id(&Self::git_dir_at(root))
                    .expect("checkout id should resolve")
            }

            fn worktree_id(&self) -> String {
                Self::worktree_id_at(&self.root)
            }

            fn add_worktree(&self, name: &str) -> PathBuf {
                let worktree_path = self.temp.path().join(name);
                git(
                    &self.root,
                    &[
                        "worktree",
                        "add",
                        "-q",
                        worktree_path.to_str().expect("utf-8 worktree path"),
                    ],
                );
                worktree_path
            }
        }

        fn count(db: &RepositoryAgentTraceDb, table: &str) -> i64 {
            db.query_map(&format!("SELECT COUNT(*) FROM {table}"), (), |row| {
                row.get::<i64>(0).map_err(anyhow::Error::from)
            })
            .expect("count query should succeed")
            .into_iter()
            .next()
            .expect("a count row should exist")
        }

        fn assert_raw_agent_trace_tables_untouched(db: &RepositoryAgentTraceDb) {
            assert_eq!(count(db, "diff_traces"), 0);
            assert_eq!(count(db, "post_commit_patch_intersections"), 0);
            assert_eq!(count(db, "agent_traces"), 0);
        }

        fn worktree_row(
            db: &RepositoryAgentTraceDb,
            worktree_id: &str,
        ) -> Option<(u64, String, bool)> {
            db.query_map(
                "SELECT revision, cursor_tree, needs_rebaseline FROM mutation_trace_worktrees \
                 WHERE worktree_id = ?1",
                (worktree_id,),
                |row| {
                    let blob: Vec<u8> = row.get(0).map_err(anyhow::Error::from)?;
                    let revision = decode_revision(&blob)?;
                    let cursor_tree = row.get::<String>(1).map_err(anyhow::Error::from)?;
                    let needs_rebaseline = row.get::<i64>(2).map_err(anyhow::Error::from)? != 0;
                    Ok((revision, cursor_tree, needs_rebaseline))
                },
            )
            .expect("worktree-row query should succeed")
            .into_iter()
            .next()
        }

        fn processed_events(db: &RepositoryAgentTraceDb) -> Vec<(String, String)> {
            db.query_map(
                "SELECT scope_id, event_id FROM mutation_trace_processed_events \
                 ORDER BY scope_id, event_id",
                (),
                |row| {
                    let scope_id = row.get::<String>(0).map_err(anyhow::Error::from)?;
                    let event_id = row.get::<String>(1).map_err(anyhow::Error::from)?;
                    Ok((scope_id, event_id))
                },
            )
            .expect("processed-events query should succeed")
        }

        fn scope_status(db: &RepositoryAgentTraceDb, scope_id: &str) -> Option<(String, String)> {
            db.query_map(
                "SELECT actor_kind, status FROM mutation_trace_scopes WHERE scope_id = ?1",
                (scope_id,),
                |row| {
                    let actor_kind = row.get::<String>(0).map_err(anyhow::Error::from)?;
                    let status = row.get::<String>(1).map_err(anyhow::Error::from)?;
                    Ok((actor_kind, status))
                },
            )
            .expect("scope query should succeed")
            .into_iter()
            .next()
        }

        fn scope_provenance(
            db: &RepositoryAgentTraceDb,
            scope_id: &str,
        ) -> Option<(String, Option<String>)> {
            db.query_map(
                "SELECT session_id, model_id FROM mutation_trace_scope_provenance \
                 WHERE scope_id = ?1",
                (scope_id,),
                |row| {
                    let session_id = row.get::<String>(0).map_err(anyhow::Error::from)?;
                    let model_id = row.get::<Option<String>>(1).map_err(anyhow::Error::from)?;
                    Ok((session_id, model_id))
                },
            )
            .expect("scope-provenance query should succeed")
            .into_iter()
            .next()
        }

        fn mutation_events_for(
            db: &RepositoryAgentTraceDb,
            worktree_id: &str,
        ) -> Vec<(String, Option<String>, String)> {
            db.query_map(
                "SELECT attribution_kind, attribution_scope_id, boundary_kind \
                 FROM mutation_trace_events WHERE worktree_id = ?1 ORDER BY revision",
                (worktree_id,),
                |row| {
                    let attribution_kind = row.get::<String>(0).map_err(anyhow::Error::from)?;
                    let attribution_scope_id =
                        row.get::<Option<String>>(1).map_err(anyhow::Error::from)?;
                    let boundary_kind = row.get::<String>(2).map_err(anyhow::Error::from)?;
                    Ok((attribution_kind, attribution_scope_id, boundary_kind))
                },
            )
            .expect("mutation-events query should succeed")
        }

        fn tool_identity_json(
            event_name: &str,
            cwd: &str,
            session_id: &str,
            tool_name: &str,
            tool_use_id: &str,
            agent_id: Option<&str>,
        ) -> String {
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
            object.insert(
                TOOL_NAME_FIELD.to_string(),
                Value::String(tool_name.to_string()),
            );
            object.insert(
                TOOL_USE_ID_FIELD.to_string(),
                Value::String(tool_use_id.to_string()),
            );
            if let Some(agent_id) = agent_id {
                object.insert(
                    AGENT_ID_FIELD.to_string(),
                    Value::String(agent_id.to_string()),
                );
            }
            Value::Object(object).to_string()
        }

        fn pre_tool_use_for(
            cwd: &str,
            session_id: &str,
            tool_name: &str,
            tool_use_id: &str,
            agent_id: Option<&str>,
        ) -> String {
            tool_identity_json(
                HOOK_EVENT_PRE_TOOL_USE,
                cwd,
                session_id,
                tool_name,
                tool_use_id,
                agent_id,
            )
        }

        fn background_pre_tool_use_for(
            cwd: &str,
            session_id: &str,
            tool_use_id: &str,
            agent_id: Option<&str>,
        ) -> String {
            let mut object: serde_json::Map<String, Value> = serde_json::from_str(
                &pre_tool_use_for(cwd, session_id, "Bash", tool_use_id, agent_id),
            )
            .expect("base PreToolUse payload should parse");
            object.insert(
                TOOL_INPUT_FIELD.to_string(),
                json!({ "run_in_background": true }),
            );
            Value::Object(object).to_string()
        }

        fn post_tool_use_for(
            cwd: &str,
            session_id: &str,
            tool_name: &str,
            tool_use_id: &str,
            agent_id: Option<&str>,
        ) -> String {
            tool_identity_json(
                HOOK_EVENT_POST_TOOL_USE,
                cwd,
                session_id,
                tool_name,
                tool_use_id,
                agent_id,
            )
        }

        fn post_tool_use_failure_for(
            cwd: &str,
            session_id: &str,
            tool_name: &str,
            tool_use_id: &str,
            agent_id: Option<&str>,
        ) -> String {
            tool_identity_json(
                HOOK_EVENT_POST_TOOL_USE_FAILURE,
                cwd,
                session_id,
                tool_name,
                tool_use_id,
                agent_id,
            )
        }

        fn permission_denied_for(
            cwd: &str,
            session_id: &str,
            tool_name: &str,
            tool_use_id: &str,
            agent_id: Option<&str>,
        ) -> String {
            tool_identity_json(
                HOOK_EVENT_PERMISSION_DENIED,
                cwd,
                session_id,
                tool_name,
                tool_use_id,
                agent_id,
            )
        }

        fn session_json(event_name: &str, cwd: &str, session_id: &str) -> String {
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

        fn agent_json(event_name: &str, cwd: &str, session_id: &str, agent_id: &str) -> String {
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
            object.insert(
                AGENT_ID_FIELD.to_string(),
                Value::String(agent_id.to_string()),
            );
            Value::Object(object).to_string()
        }

        fn worktree_remove_json(session_id: &str, worktree_path: &str) -> String {
            let mut object = serde_json::Map::new();
            object.insert(
                HOOK_EVENT_NAME_FIELD.to_string(),
                Value::String(HOOK_EVENT_WORKTREE_REMOVE.to_string()),
            );
            object.insert(
                SESSION_ID_FIELD.to_string(),
                Value::String(session_id.to_string()),
            );
            object.insert(
                WORKTREE_PATH_FIELD.to_string(),
                Value::String(worktree_path.to_string()),
            );
            Value::Object(object).to_string()
        }

        #[test]
        fn test1_foreground_write_closes_ai_exclusive() {
            let repo = ClaudeRepo::new("test1-foreground-write");
            let cwd = repo.cwd();

            assert_eq!(
                repo.drive(&pre_tool_use_for(
                    &cwd,
                    "session-1",
                    "Write",
                    "toolu_1",
                    None
                ))
                .expect("PreToolUse should succeed"),
                ""
            );
            let scope_id = repo.adapter_state().attempts[0].scope_id.clone();

            fs::write(repo.root.join("file.txt"), "one\ntwo\n")
                .expect("the tool's own edit should write");

            assert_eq!(
                repo.drive(&post_tool_use_for(
                    &cwd,
                    "session-1",
                    "Write",
                    "toolu_1",
                    None
                ))
                .expect("PostToolUse should succeed"),
                ""
            );

            assert!(
                repo.adapter_state().attempts.is_empty(),
                "the closed attempt must be removed from adapter bookkeeping"
            );

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &scope_id),
                Some(("claude_code".to_string(), "closed".to_string()))
            );
            let worktree_id = repo.worktree_id();
            assert_eq!(
                mutation_events_for(&db, &worktree_id),
                vec![(
                    "ai_exclusive".to_string(),
                    Some(scope_id.clone()),
                    "close".to_string(),
                )]
            );
            assert_eq!(
                worktree_row(&db, &worktree_id).map(|(_, cursor_tree, _)| cursor_tree),
                Some(repo.working_tree())
            );
            assert_eq!(
                processed_events(&db),
                vec![
                    (scope_id.clone(), claude_scope_close_event_id(&scope_id)),
                    (scope_id.clone(), claude_scope_start_event_id(&scope_id)),
                ],
                "rows are ordered by (scope_id, event_id), and 'close' sorts before 'start'"
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test2_failed_bash_partial_write_still_closes_ai_exclusive() {
            let repo = ClaudeRepo::new("test2-failed-bash");
            let cwd = repo.cwd();

            repo.drive(&pre_tool_use_for(
                &cwd,
                "session-1",
                "Bash",
                "toolu_1",
                None,
            ))
            .expect("PreToolUse should succeed");
            let scope_id = repo.adapter_state().attempts[0].scope_id.clone();

            fs::write(repo.root.join("file.txt"), "one\npartial\n")
                .expect("the failed tool's partial edit should write");

            assert_eq!(
                repo.drive(&post_tool_use_failure_for(
                    &cwd,
                    "session-1",
                    "Bash",
                    "toolu_1",
                    None
                ))
                .expect("PostToolUseFailure should succeed"),
                ""
            );

            assert!(repo.adapter_state().attempts.is_empty());

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &scope_id),
                Some(("claude_code".to_string(), "closed".to_string()))
            );
            let worktree_id = repo.worktree_id();
            assert_eq!(
                mutation_events_for(&db, &worktree_id),
                vec![(
                    "ai_exclusive".to_string(),
                    Some(scope_id.clone()),
                    "close".to_string(),
                )]
            );
            assert_eq!(
                worktree_row(&db, &worktree_id).map(|(_, cursor_tree, _)| cursor_tree),
                Some(repo.working_tree())
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test4_duplicate_pre_and_post_replay_has_no_duplicate_transition() {
            let repo = ClaudeRepo::new("test4-duplicate-replay");
            let cwd = repo.cwd();
            let pre = pre_tool_use_for(&cwd, "session-1", "Write", "toolu_1", None);

            repo.drive(&pre).expect("first PreToolUse should succeed");
            assert_eq!(
                repo.drive(&pre)
                    .expect("duplicate PreToolUse should be idempotent"),
                ""
            );
            assert_eq!(
                repo.adapter_state().attempts.len(),
                1,
                "AC4: duplicate PreToolUse delivery must reuse the same attempt"
            );
            let scope_id = repo.adapter_state().attempts[0].scope_id.clone();

            fs::write(repo.root.join("file.txt"), "one\ntwo\n").expect("the edit should write");

            let post = post_tool_use_for(&cwd, "session-1", "Write", "toolu_1", None);
            repo.drive(&post).expect("first PostToolUse should succeed");

            let db = repo.db();
            let (revision_before, events_before, processed_before) = (
                worktree_row(&db, &repo.worktree_id())
                    .map(|(revision, _, _)| revision)
                    .expect("a worktree row should exist"),
                count(&db, "mutation_trace_events"),
                count(&db, "mutation_trace_processed_events"),
            );

            assert_eq!(
                repo.drive(&post)
                    .expect("duplicate PostToolUse delivery must be a safe no-op"),
                ""
            );

            let db = repo.db();
            assert_eq!(
                worktree_row(&db, &repo.worktree_id()).map(|(revision, _, _)| revision),
                Some(revision_before)
            );
            assert_eq!(count(&db, "mutation_trace_events"), events_before);
            assert_eq!(
                count(&db, "mutation_trace_processed_events"),
                processed_before
            );
            assert_eq!(
                processed_events(&db)
                    .into_iter()
                    .filter(|(scope, event)| scope == &scope_id
                        && event == &claude_scope_close_event_id(&scope_id))
                    .count(),
                1
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test5_auto_permission_denied_abandons_and_requires_rebaseline() {
            let repo = ClaudeRepo::new("test5-permission-denied");
            let cwd = repo.cwd();

            repo.drive(&pre_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_1",
                None,
            ))
            .expect("PreToolUse should succeed");
            let scope_id = repo.adapter_state().attempts[0].scope_id.clone();

            assert_eq!(
                repo.drive(&permission_denied_for(
                    &cwd,
                    "session-1",
                    "Write",
                    "toolu_1",
                    None
                ))
                .expect("PermissionDenied should succeed"),
                ""
            );

            assert!(
                repo.adapter_state().attempts.is_empty(),
                "the denied attempt must be retired from adapter bookkeeping"
            );
            assert!(
                repo.adapter_state().recovery_pending,
                "D19: abandonment must arm the recovery barrier"
            );

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &scope_id),
                Some(("claude_code".to_string(), "abandoned".to_string()))
            );
            let worktree_id = repo.worktree_id();
            assert!(
                worktree_row(&db, &worktree_id)
                    .is_some_and(|(_, _, needs_rebaseline)| needs_rebaseline),
                "AC12: a denied execution must leave the worktree needing rebaseline"
            );
            assert_eq!(count(&db, "mutation_trace_events"), 0);

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test3_two_parallel_subagent_tools_produce_ai_contended() {
            let repo = ClaudeRepo::new("test3-parallel-subagents");
            let cwd = repo.cwd();

            repo.drive(&pre_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_a",
                Some("agent-a"),
            ))
            .expect("agent-a PreToolUse should succeed");
            repo.drive(&pre_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_b",
                Some("agent-b"),
            ))
            .expect("agent-b PreToolUse should succeed");
            assert_eq!(repo.adapter_state().attempts.len(), 2);

            fs::write(repo.root.join("file.txt"), "one\ncontended\n")
                .expect("the racing edit should write");

            repo.drive(&post_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_a",
                Some("agent-a"),
            ))
            .expect("agent-a PostToolUse (closing while agent-b is still active) should succeed");
            repo.drive(&post_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_b",
                Some("agent-b"),
            ))
            .expect("agent-b PostToolUse should succeed");

            assert!(repo.adapter_state().attempts.is_empty());

            let db = repo.db();
            let worktree_id = repo.worktree_id();
            assert_eq!(
                mutation_events_for(&db, &worktree_id),
                vec![("ai_contended".to_string(), None, "close".to_string())],
                "AC11: a tree transition observed while two scopes are live must be AiContended"
            );
            assert_eq!(
                worktree_row(&db, &worktree_id).map(|(_, cursor_tree, _)| cursor_tree),
                Some(repo.working_tree())
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test6_other_hook_denial_is_retired_by_stop_cleanup() {
            let repo = ClaudeRepo::new("test6-stop-cleanup");
            let cwd = repo.cwd();

            repo.drive(&pre_tool_use_for(
                &cwd,
                "session-1",
                "Bash",
                "toolu_1",
                None,
            ))
            .expect("PreToolUse should succeed");
            let scope_id = repo.adapter_state().attempts[0].scope_id.clone();

            assert_eq!(
                repo.drive(&session_json(HOOK_EVENT_STOP, &cwd, "session-1"))
                    .expect("Stop should succeed"),
                ""
            );

            assert!(
                repo.adapter_state().attempts.is_empty(),
                "AC13: Stop must retire the stale main-thread attempt"
            );
            assert!(repo.adapter_state().recovery_pending);

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &scope_id),
                Some(("claude_code".to_string(), "abandoned".to_string()))
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test7_interrupted_main_turn_is_retired_by_next_user_prompt_submit() {
            let repo = ClaudeRepo::new("test7-user-prompt-submit-cleanup");
            let cwd = repo.cwd();

            repo.drive(&pre_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_1",
                None,
            ))
            .expect("PreToolUse should succeed");
            let scope_id = repo.adapter_state().attempts[0].scope_id.clone();

            fs::write(repo.root.join("file.txt"), "one\ninterrupted\n")
                .expect("the interrupted edit should write");

            assert_eq!(
                repo.drive(&session_json(
                    HOOK_EVENT_USER_PROMPT_SUBMIT,
                    &cwd,
                    "session-1"
                ))
                .expect("UserPromptSubmit should succeed"),
                ""
            );

            assert!(
                repo.adapter_state().attempts.is_empty(),
                "AC14: UserPromptSubmit must retire the stale main-thread attempt \
                 before another mutation-capable tool can start"
            );

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &scope_id),
                Some(("claude_code".to_string(), "abandoned".to_string()))
            );
            let worktree_id = repo.worktree_id();
            assert!(worktree_row(&db, &worktree_id)
                .is_some_and(|(_, _, needs_rebaseline)| needs_rebaseline));

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test8_resumed_subagent_tool_use_id_gets_a_fresh_scope_id() {
            let repo = ClaudeRepo::new("test8-resumed-subagent");
            let cwd = repo.cwd();

            repo.drive(&pre_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_resumed",
                Some("agent-a"),
            ))
            .expect("first PreToolUse should succeed");
            let first_scope_id = repo.adapter_state().attempts[0].scope_id.clone();

            fs::write(repo.root.join("file.txt"), "one\nfirst\n").expect("first edit should write");
            repo.drive(&post_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_resumed",
                Some("agent-a"),
            ))
            .expect("first PostToolUse should succeed");
            assert!(repo.adapter_state().attempts.is_empty());

            repo.drive(&pre_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_resumed",
                Some("agent-a"),
            ))
            .expect("resumed PreToolUse should succeed");
            let second_scope_id = repo.adapter_state().attempts[0].scope_id.clone();

            assert_ne!(
                first_scope_id, second_scope_id,
                "AC15: a resumed subagent's new tool attempt must receive a fresh ScopeId"
            );

            fs::write(repo.root.join("file.txt"), "one\nfirst\nsecond\n")
                .expect("second edit should write");
            repo.drive(&post_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_resumed",
                Some("agent-a"),
            ))
            .expect("second PostToolUse should succeed");

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &first_scope_id).map(|(_, status)| status),
                Some("closed".to_string())
            );
            assert_eq!(
                scope_status(&db, &second_scope_id).map(|(_, status)| status),
                Some("closed".to_string())
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test9_main_and_subagent_concurrent_mutation_is_ai_contended() {
            let repo = ClaudeRepo::new("test9-main-plus-subagent");
            let cwd = repo.cwd();

            repo.drive(&pre_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_main",
                None,
            ))
            .expect("main-thread PreToolUse should succeed");
            repo.drive(&pre_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_sub",
                Some("agent-a"),
            ))
            .expect("subagent PreToolUse should succeed");
            assert_eq!(repo.adapter_state().attempts.len(), 2);

            fs::write(repo.root.join("file.txt"), "one\nboth-writing\n")
                .expect("the racing edit should write");

            repo.drive(&post_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_main",
                None,
            ))
            .expect("main-thread PostToolUse should succeed");
            repo.drive(&post_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_sub",
                Some("agent-a"),
            ))
            .expect("subagent PostToolUse should succeed");

            let db = repo.db();
            let worktree_id = repo.worktree_id();
            assert_eq!(
                mutation_events_for(&db, &worktree_id),
                vec![("ai_contended".to_string(), None, "close".to_string())],
                "AC11: main + subagent concurrent mutation must be AiContended"
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test10_isolated_subagent_worktree_advances_only_its_own_cursor() {
            let repo = ClaudeRepo::new("test10-isolated-worktree");
            let worktree_path = repo.add_worktree("subagent-worktree");
            let worktree_cwd = ClaudeRepo::cwd_at(&worktree_path);

            let main_worktree_id = repo.worktree_id();
            let sub_worktree_id = ClaudeRepo::worktree_id_at(&worktree_path);
            assert_ne!(
                main_worktree_id, sub_worktree_id,
                "a linked worktree must resolve to a distinct WorktreeId"
            );

            repo.drive_flush()
                .expect("main-checkout baseline flush should succeed");
            let main_cursor_before = worktree_row(&repo.db(), &main_worktree_id)
                .map(|(_, cursor_tree, _)| cursor_tree)
                .expect("main checkout should have a baseline worktree row");

            repo.drive(&pre_tool_use_for(
                &worktree_cwd,
                "session-1",
                "Write",
                "toolu_sub",
                Some("agent-a"),
            ))
            .expect("subagent PreToolUse in the isolated worktree should succeed");
            fs::write(worktree_path.join("file.txt"), "one\nisolated\n")
                .expect("the isolated worktree's own edit should write");
            repo.drive(&post_tool_use_for(
                &worktree_cwd,
                "session-1",
                "Write",
                "toolu_sub",
                Some("agent-a"),
            ))
            .expect("subagent PostToolUse in the isolated worktree should succeed");

            let db = repo.db();
            assert_eq!(
                worktree_row(&db, &main_worktree_id).map(|(_, cursor_tree, _)| cursor_tree),
                Some(main_cursor_before),
                "AC17: the main checkout's mutation cursor must be unchanged"
            );
            let sub_row = worktree_row(&db, &sub_worktree_id).expect("subagent worktree row");
            assert_eq!(
                sub_row.1,
                ClaudeRepo::working_tree_at(&worktree_path),
                "AC16/AC17: the isolated worktree's own cursor must advance"
            );
            assert_eq!(
                mutation_events_for(&db, &sub_worktree_id)
                    .into_iter()
                    .map(|(attribution, _, boundary)| (attribution, boundary))
                    .collect::<Vec<_>>(),
                vec![("ai_exclusive".to_string(), "close".to_string())]
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test11_worktree_remove_cleans_only_that_worktrees_outstanding_attempt() {
            let repo = ClaudeRepo::new("test11-worktree-remove");
            let worktree_path = repo.add_worktree("removed-worktree");
            let worktree_cwd = ClaudeRepo::cwd_at(&worktree_path);
            let main_cwd = repo.cwd();

            repo.drive(&pre_tool_use_for(
                &main_cwd,
                "session-1",
                "Write",
                "toolu_main",
                None,
            ))
            .expect("main-thread PreToolUse should succeed");
            repo.drive(&pre_tool_use_for(
                &worktree_cwd,
                "session-1",
                "Write",
                "toolu_sub",
                Some("agent-a"),
            ))
            .expect("subagent PreToolUse in the isolated worktree should succeed");

            assert_eq!(ClaudeRepo::adapter_state_at(&repo.root).attempts.len(), 1);
            assert_eq!(
                ClaudeRepo::adapter_state_at(&worktree_path).attempts.len(),
                1
            );

            assert_eq!(
                repo.drive(&worktree_remove_json("session-1", &worktree_cwd))
                    .expect("WorktreeRemove should succeed"),
                ""
            );

            assert!(
                ClaudeRepo::adapter_state_at(&worktree_path)
                    .attempts
                    .is_empty(),
                "AC13/D22: WorktreeRemove must retire the outstanding attempt for that worktree"
            );
            assert_eq!(
                ClaudeRepo::adapter_state_at(&repo.root).attempts.len(),
                1,
                "WorktreeRemove for one worktree must not touch the main checkout's attempts"
            );

            assert_raw_agent_trace_tables_untouched(&repo.db());
        }

        #[test]
        fn test12_pending_start_crash_before_start_is_recovered_conservatively() {
            let repo = ClaudeRepo::new("test12-pending-start-crash");
            let cwd = repo.cwd();
            let git_dir = repo.git_dir();

            let key = AttemptKey {
                session_id: "session-1".to_string(),
                agent_id: None,
                tool_use_id: "toolu_crashed".to_string(),
            };
            let allocated = state::allocate_attempt(&git_dir, &key, "Write")
                .expect("allocation should succeed");
            assert_eq!(allocated.attempt.phase, state::AttemptPhase::PendingStart);

            assert_eq!(
                repo.drive(&post_tool_use_for(
                    &cwd,
                    "session-1",
                    "Write",
                    "toolu_crashed",
                    None
                ))
                .expect("D11: PostToolUse on a pending_start attempt must abandon, not late-start"),
                ""
            );

            assert!(
                repo.adapter_state().attempts.is_empty(),
                "the never-started attempt must be retired"
            );
            assert!(repo.adapter_state().recovery_pending);
            let db = repo.db();
            assert_eq!(
                scope_status(&db, &allocated.attempt.scope_id),
                None,
                "a Start that never committed must never appear as a real scope"
            );
            assert_eq!(count(&db, "mutation_trace_events"), 0);

            repo.drive(&pre_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_fresh",
                None,
            ))
            .expect("the next PreToolUse should proceed after the quiescent flush");
            assert!(!repo.adapter_state().recovery_pending);
            assert_eq!(repo.adapter_state().attempts.len(), 1);

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test13_start_committed_before_state_settlement_is_recovered_by_abandonment() {
            let repo = ClaudeRepo::new("test13-start-committed-crash");
            let cwd = repo.cwd();
            let git_dir = repo.git_dir();

            let key = AttemptKey {
                session_id: "session-1".to_string(),
                agent_id: None,
                tool_use_id: "toolu_crashed".to_string(),
            };
            let allocated = state::allocate_attempt(&git_dir, &key, "Write")
                .expect("allocation should succeed");
            let scope_id = allocated.attempt.scope_id.clone();

            repo.drive_generic(&scope_boundary_payload(
                "start",
                &scope_id,
                &claude_scope_start_event_id(&scope_id),
            ))
            .expect("the runtime Start should commit durably");
            assert_eq!(
                state::read_state(&git_dir).unwrap().attempts[0].phase,
                state::AttemptPhase::PendingStart
            );

            assert_eq!(
                repo.drive(&post_tool_use_for(
                    &cwd,
                    "session-1",
                    "Write",
                    "toolu_crashed",
                    None
                ))
                .expect("D11: a pending_start attempt with a committed Start must be abandoned"),
                ""
            );

            assert!(repo.adapter_state().attempts.is_empty());
            assert!(repo.adapter_state().recovery_pending);

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &scope_id),
                Some(("claude_code".to_string(), "abandoned".to_string())),
                "the runtime's own committed Start must settle as a real abandonment"
            );
            let worktree_id = repo.worktree_id();
            assert!(worktree_row(&db, &worktree_id)
                .is_some_and(|(_, _, needs_rebaseline)| needs_rebaseline));

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test14_terminal_runtime_success_before_state_cleanup_is_replay_safe() {
            let repo = ClaudeRepo::new("test14-close-committed-crash");
            let cwd = repo.cwd();
            let git_dir = repo.git_dir();

            repo.drive(&pre_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_1",
                None,
            ))
            .expect("PreToolUse should succeed");
            let scope_id = repo.adapter_state().attempts[0].scope_id.clone();
            fs::write(repo.root.join("file.txt"), "one\ntwo\n").expect("the edit should write");

            repo.drive_generic(&scope_boundary_payload(
                "close",
                &scope_id,
                &claude_scope_close_event_id(&scope_id),
            ))
            .expect("the runtime Close should commit durably");
            assert_eq!(state::read_state(&git_dir).unwrap().attempts.len(), 1);

            let db = repo.db();
            let (revision_before, events_before) = (
                worktree_row(&db, &repo.worktree_id())
                    .map(|(revision, _, _)| revision)
                    .expect("a worktree row should exist"),
                count(&db, "mutation_trace_events"),
            );

            assert_eq!(
                repo.drive(&post_tool_use_for(
                    &cwd,
                    "session-1",
                    "Write",
                    "toolu_1",
                    None
                ))
                .expect("a replayed Close against an already-durable commit must be safe"),
                ""
            );

            assert!(
                repo.adapter_state().attempts.is_empty(),
                "the stale bookkeeping must finally be cleared"
            );

            let db = repo.db();
            assert_eq!(
                worktree_row(&db, &repo.worktree_id()).map(|(revision, _, _)| revision),
                Some(revision_before),
                "a durably completed Close must never be re-applied as a second transition"
            );
            assert_eq!(count(&db, "mutation_trace_events"), events_before);
            assert_eq!(
                scope_status(&db, &scope_id).map(|(_, status)| status),
                Some("closed".to_string())
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test15_explicit_background_bash_is_denied_with_no_scope() {
            let repo = ClaudeRepo::new("test15-explicit-background-bash");
            let cwd = repo.cwd();

            let output = repo
                .drive(&background_pre_tool_use_for(
                    &cwd,
                    "session-1",
                    "toolu_1",
                    None,
                ))
                .expect("an explicit background shell must still return Ok with a deny payload");

            assert_eq!(
                output,
                pre_tool_use_deny_json(EXPLICIT_BACKGROUND_SHELL_DENY_REASON)
            );
            assert!(
                repo.adapter_state().attempts.is_empty(),
                "AC21: an explicit background shell must create no scope"
            );

            let db = repo.db();
            assert_eq!(count(&db, "mutation_trace_scopes"), 0);
            assert_eq!(count(&db, "mutation_trace_events"), 0);

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test18_model_switch_does_not_rewrite_scope_provenance() {
            let repo = ClaudeRepo::new("model-switch-provenance");
            let session_id = "session-model-switch";
            let db = repo.db();
            db.upsert_claude_model_state(ClaudeModelStateObservation {
                session_id: "cc_session-model-switch".to_string(),
                agent_id: String::new(),
                model_id: "claude/sonnet".to_string(),
                observation_kind: ObservationKind::SessionStart,
                source: "test".to_string(),
                observed_at_ms: 1,
            })
            .expect("initial Claude model state should persist");

            let pre_payload =
                pre_tool_use_for(&repo.cwd(), session_id, "Bash", "toolu_model_switch", None);
            repo.drive(&pre_payload)
                .expect("initial PreToolUse should establish a scope");
            let scope_id = repo
                .adapter_state()
                .attempts
                .first()
                .expect("the scope should remain live")
                .scope_id
                .clone();
            let before = scope_provenance(&repo.db(), &scope_id);
            assert_eq!(
                before,
                Some((
                    "cc_session-model-switch".to_string(),
                    Some("claude/sonnet".to_string()),
                ))
            );

            repo.db()
                .upsert_claude_model_state(ClaudeModelStateObservation {
                    session_id: "cc_session-model-switch".to_string(),
                    agent_id: String::new(),
                    model_id: "claude/opus".to_string(),
                    observation_kind: ObservationKind::PostModelSwitch,
                    source: "test".to_string(),
                    observed_at_ms: 2,
                })
                .expect("model switch should persist");
            repo.drive(&pre_payload)
                .expect("replayed PreToolUse should remain idempotent");

            assert_eq!(scope_provenance(&repo.db(), &scope_id), before);
        }

        #[test]
        fn test16_regression_matrix_leaves_raw_agent_trace_tables_untouched() {
            let repo = ClaudeRepo::new("test16-raw-tables-untouched");
            let cwd = repo.cwd();

            let before = {
                let db = repo.db();
                (
                    count(&db, "diff_traces"),
                    count(&db, "post_commit_patch_intersections"),
                    count(&db, "agent_traces"),
                )
            };
            assert_eq!(before, (0, 0, 0));

            repo.drive(&pre_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_1",
                None,
            ))
            .expect("PreToolUse should succeed");
            fs::write(repo.root.join("file.txt"), "one\ntwo\n").expect("the edit should write");
            repo.drive(&post_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_1",
                None,
            ))
            .expect("PostToolUse should succeed");
            repo.drive(&pre_tool_use_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_2",
                None,
            ))
            .expect("second PreToolUse should succeed");
            repo.drive(&permission_denied_for(
                &cwd,
                "session-1",
                "Write",
                "toolu_2",
                None,
            ))
            .expect("PermissionDenied should succeed");

            let db = repo.db();
            let after = (
                count(&db, "diff_traces"),
                count(&db, "post_commit_patch_intersections"),
                count(&db, "agent_traces"),
            );
            assert_eq!(
                after,
                (0, 0, 0),
                "AC20: Claude mutation-scope-only regressions must leave the raw \
                 Agent Trace tables unchanged"
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test17_detached_descendant_write_after_post_tool_use_is_not_folded_into_the_closed_scope(
        ) {
            let repo = ClaudeRepo::new("test17-detached-descendant");
            let cwd = repo.cwd();

            repo.drive(&pre_tool_use_for(
                &cwd,
                "session-1",
                "Bash",
                "toolu_1",
                None,
            ))
            .expect("PreToolUse should succeed");
            let scope_id = repo.adapter_state().attempts[0].scope_id.clone();

            fs::write(repo.root.join("file.txt"), "one\nforeground-output\n")
                .expect("the tool's own foreground write should write");
            let tree_at_close = repo.working_tree();

            repo.drive(&post_tool_use_for(
                &cwd,
                "session-1",
                "Bash",
                "toolu_1",
                None,
            ))
            .expect("PostToolUse should succeed");

            let db = repo.db();
            let worktree_id = repo.worktree_id();
            assert_eq!(
                scope_status(&db, &scope_id).map(|(_, status)| status),
                Some("closed".to_string())
            );
            assert_eq!(
                worktree_row(&db, &worktree_id).map(|(_, cursor_tree, _)| cursor_tree),
                Some(tree_at_close.clone()),
                "the scope must close at the tool's own observed tree"
            );

            fs::write(
                repo.root.join("file.txt"),
                "one\nforeground-output\ndetached-descendant\n",
            )
            .expect("the detached descendant's later write should write");
            let tree_after_descendant = repo.working_tree();
            assert_ne!(tree_after_descendant, tree_at_close);

            let events_before_flush = mutation_events_for(&db, &worktree_id);

            repo.drive_flush()
                .expect("a later recovery/diagnostic flush should succeed");

            let db = repo.db();
            let events_after_flush = mutation_events_for(&db, &worktree_id);
            assert_eq!(
                events_after_flush.len(),
                events_before_flush.len() + 1,
                "the detached descendant's mutation must surface as its own event"
            );
            let (attribution_kind, attribution_scope_id, _) = events_after_flush
                .last()
                .expect("a flush event should exist");
            assert_ne!(
                attribution_scope_id.as_deref(),
                Some(scope_id.as_str()),
                "the detached descendant's mutation must never be attributed to the \
                 already-closed tool scope"
            );
            assert_eq!(attribution_kind, "ineligible_unscoped");
            assert_eq!(
                worktree_row(&db, &worktree_id).map(|(_, cursor_tree, _)| cursor_tree),
                Some(tree_after_descendant)
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }
    }
}
