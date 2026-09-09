#![allow(dead_code)]

mod boundary_lock;
mod os_lock;
pub(crate) mod state;

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::services::checkout;
use crate::services::hooks::codex::bash_policy::{
    bash_command_from_tool_input, evaluate_codex_bash_policy, CodexBashPolicyDecision,
};
use crate::services::hooks::{
    normalize_codex_model_id, prefixed_diff_trace_session_id, CODEX_TOOL_NAME,
};
use crate::services::observability::traits::Logger;

use boundary_lock::{AdapterBoundaryLock, DEFAULT_BOUNDARY_LOCK_TIMEOUT};

const HOOK_EVENT_NAME_FIELD: &str = "hook_event_name";
const SESSION_ID_FIELD: &str = "session_id";
const TURN_ID_FIELD: &str = "turn_id";
const CWD_FIELD: &str = "cwd";
const AGENT_ID_FIELD: &str = "agent_id";
const AGENT_TYPE_FIELD: &str = "agent_type";
const MODEL_FIELD: &str = "model";
const PROVENANCE_FIELD: &str = "provenance";
const TOOL_NAME_FIELD: &str = "tool_name";
const TOOL_USE_ID_FIELD: &str = "tool_use_id";
const TOOL_INPUT_FIELD: &str = "tool_input";

const CODEX_TRACKED_TOOL_BASH: &str = "Bash";

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
        model: tolerated_model(object),
        tool_input: object.get(TOOL_INPUT_FIELD).cloned(),
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

fn tolerated_model(object: &Map<String, Value>) -> Option<String> {
    object
        .get(MODEL_FIELD)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn validation_error(detail: &str) -> String {
    format!("Invalid Codex hook event payload from STDIN: {detail}.")
}

type GitDirResolver<'a> = &'a dyn Fn(&str) -> Result<PathBuf>;

type IngressSeam<'a> = &'a dyn Fn(&Path, &str, Option<&dyn Logger>) -> Result<String>;

type BashPolicyEvaluator<'a> = &'a dyn Fn(&Path, &str) -> Result<CodexBashPolicyDecision>;

const ACTOR_KIND_CODEX: &str = "codex";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexScopeProvenance {
    pub session_id: String,
    pub model_id: Option<String>,
}

fn codex_scope_provenance(execution: &CodexToolExecution) -> CodexScopeProvenance {
    CodexScopeProvenance {
        session_id: prefixed_diff_trace_session_id(CODEX_TOOL_NAME, &execution.identity.session_id),
        model_id: execution
            .model
            .as_deref()
            .and_then(normalize_codex_model_id),
    }
}

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
    let bash_policy_fn = |repository_root: &Path, command: &str| {
        evaluate_codex_bash_policy(repository_root, command)
    };

    run_codex_mutation_scope_from_payload_with_seams(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &seam_fn,
        &bash_policy_fn,
    )
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
    let bash_policy_fn = |repository_root: &Path, command: &str| {
        evaluate_codex_bash_policy(repository_root, command)
    };

    run_codex_mutation_scope_from_payload_with_seams(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &seam_fn,
        &bash_policy_fn,
    )
}

#[cfg(test)]
fn run_codex_mutation_scope_from_payload_with(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> Result<String> {
    let allow_all = |_repository_root: &Path, _command: &str| Ok(CodexBashPolicyDecision::Allowed);
    run_codex_mutation_scope_from_payload_with_seams(
        stdin_payload,
        logger,
        resolve_git_dir,
        seam,
        &allow_all,
    )
}

#[cfg(test)]
fn run_codex_mutation_scope_from_payload_with_bash_policy(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
    evaluate_bash_policy: BashPolicyEvaluator,
) -> Result<String> {
    run_codex_mutation_scope_from_payload_with_seams(
        stdin_payload,
        logger,
        resolve_git_dir,
        seam,
        evaluate_bash_policy,
    )
}

fn run_codex_mutation_scope_from_payload_with_seams(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
    evaluate_bash_policy: BashPolicyEvaluator,
) -> Result<String> {
    let event = parse_codex_hook_event(stdin_payload)?;
    dispatch_codex_hook_event(event, logger, resolve_git_dir, seam, evaluate_bash_policy)
}

fn dispatch_codex_hook_event(
    event: CodexHookEvent,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
    evaluate_bash_policy: BashPolicyEvaluator,
) -> Result<String> {
    match event {
        CodexHookEvent::PreToolUse(execution) => Ok(handle_pre_tool_use(
            &execution,
            logger,
            resolve_git_dir,
            seam,
            evaluate_bash_policy,
        )),
        CodexHookEvent::PostToolUse(identity) => {
            if !matches!(
                classify_tool(&identity.tool_name),
                ToolClassification::TrackedMutation
            ) {
                return Ok(String::new());
            }

            let git_dir = resolve_git_dir(&identity.cwd)?;
            let repository_root = Path::new(&identity.cwd);
            with_boundary_lock(&git_dir, || {
                handle_close(
                    &git_dir,
                    repository_root,
                    &identity.attempt_key(),
                    logger,
                    seam,
                )
            })
        }
        CodexHookEvent::Stop(turn) => {
            let git_dir = resolve_git_dir(&turn.cwd)?;
            let repository_root = Path::new(&turn.cwd);
            let session_id = turn.session_id.clone();
            with_boundary_lock(&git_dir, || {
                cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                    attempt.session_id == session_id && attempt.agent_id.is_none()
                })
            })
        }
        CodexHookEvent::Interrupt(turn) => {
            let git_dir = resolve_git_dir(&turn.cwd)?;
            let repository_root = Path::new(&turn.cwd);
            let session_id = turn.session_id.clone();
            with_boundary_lock(&git_dir, || {
                cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                    attempt.session_id == session_id
                })
            })
        }
        CodexHookEvent::SubagentStop(agent) => {
            let git_dir = resolve_git_dir(&agent.cwd)?;
            let repository_root = Path::new(&agent.cwd);
            let session_id = agent.session_id.clone();
            let agent_id = agent.agent_id.clone();
            with_boundary_lock(&git_dir, || {
                cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                    attempt.session_id == session_id
                        && attempt.agent_id.as_deref() == Some(&agent_id)
                })
            })
        }
        CodexHookEvent::SessionEnd(session) => {
            let git_dir = resolve_git_dir(&session.cwd)?;
            let repository_root = Path::new(&session.cwd);
            let session_id = session.session_id.clone();
            with_boundary_lock(&git_dir, || {
                cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                    attempt.session_id == session_id
                })
            })
        }
    }
}

fn with_boundary_lock<T>(git_dir: &Path, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    let _boundary = AdapterBoundaryLock::acquire(git_dir, DEFAULT_BOUNDARY_LOCK_TIMEOUT)
        .map_err(|error| anyhow!("Failed to acquire adapter boundary lock: {error}"))?;
    operation()
}

fn handle_pre_tool_use(
    execution: &CodexToolExecution,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
    evaluate_bash_policy: BashPolicyEvaluator,
) -> String {
    let identity = &execution.identity;

    if !matches!(
        classify_tool(&identity.tool_name),
        ToolClassification::TrackedMutation
    ) {
        return String::new();
    }

    let repository_root = Path::new(&identity.cwd);

    if identity.tool_name == CODEX_TRACKED_TOOL_BASH {
        match codex_bash_policy_preflight(repository_root, execution, evaluate_bash_policy) {
            BashPolicyPreflight::Allowed => {}
            BashPolicyPreflight::Blocked(response) => return response,
            BashPolicyPreflight::EvaluationFailed(error) => {
                log_pre_tool_use_fail_closed(logger, "bash_policy_preflight", &error);
                return pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON);
            }
        }
    }

    let git_dir = match resolve_git_dir(&identity.cwd) {
        Ok(git_dir) => git_dir,
        Err(error) => {
            log_pre_tool_use_fail_closed(logger, "resolve_git_dir", &error);
            return pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON);
        }
    };

    let key = identity.attempt_key();
    let turn_id = identity.turn_id.as_str();
    let provenance = codex_scope_provenance(execution);
    let outcome = with_boundary_lock(&git_dir, || {
        state::normalize_recovery_after_boundary_lock_acquired(&git_dir)?;

        sweep_stale_lane_predecessors(&git_dir, repository_root, &key, turn_id, logger, seam)?;

        match admit_or_recover(
            &git_dir,
            repository_root,
            &key,
            turn_id,
            &identity.tool_name,
            logger,
            seam,
        )? {
            Admission::Admitted(allocated) => {
                establish_start(
                    &git_dir,
                    repository_root,
                    &allocated,
                    &provenance,
                    logger,
                    seam,
                )?;
                Ok(PreToolUseOutcome::Continue)
            }
            Admission::Denied => Ok(PreToolUseOutcome::Deny),
        }
    });

    match outcome {
        Ok(PreToolUseOutcome::Continue) => String::new(),
        Ok(PreToolUseOutcome::Deny) => pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON),
        Err(error) => {
            log_pre_tool_use_fail_closed(logger, "codex_mutation_scope_pre_tool_use", &error);
            pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON)
        }
    }
}

enum PreToolUseOutcome {
    Continue,
    Deny,
}

enum BashPolicyPreflight {
    Allowed,
    Blocked(String),
    EvaluationFailed(anyhow::Error),
}

fn codex_bash_policy_preflight(
    repository_root: &Path,
    execution: &CodexToolExecution,
    evaluate_bash_policy: BashPolicyEvaluator,
) -> BashPolicyPreflight {
    let command = match bash_command_from_tool_input(execution.tool_input.as_ref()) {
        Ok(command) => command,
        Err(error) => return BashPolicyPreflight::EvaluationFailed(error),
    };

    match evaluate_bash_policy(repository_root, command) {
        Ok(CodexBashPolicyDecision::Allowed) => BashPolicyPreflight::Allowed,
        Ok(CodexBashPolicyDecision::Blocked(response)) => BashPolicyPreflight::Blocked(response),
        Err(error) => BashPolicyPreflight::EvaluationFailed(error),
    }
}

enum Admission {
    Admitted(state::AllocatedAttempt),
    Denied,
}

fn sweep_stale_lane_predecessors(
    git_dir: &Path,
    repository_root: &Path,
    key: &AttemptKey,
    turn_id: &str,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<()> {
    loop {
        let current = state::read_state(git_dir)?;
        let Some(stale) = current
            .attempts
            .iter()
            .find(|attempt| {
                attempt.in_builtin_lane(&key.session_id, turn_id)
                    && !attempt_matches_key(attempt, key)
            })
            .cloned()
        else {
            return Ok(());
        };
        abandon_attempt(git_dir, repository_root, &stale, logger, seam)?;
    }
}

fn admit_or_recover(
    git_dir: &Path,
    repository_root: &Path,
    key: &AttemptKey,
    turn_id: &str,
    tool_name: &str,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<Admission> {
    match state::admit_tracked_attempt(git_dir, key, turn_id, tool_name)? {
        state::AdmitDecision::Admitted(allocated) => Ok(Admission::Admitted(allocated)),
        state::AdmitDecision::RecoveryBlocked
        | state::AdmitDecision::UncertainAttemptBlocked
        | state::AdmitDecision::StalePredecessorBlocked => Ok(Admission::Denied),
        state::AdmitDecision::FlushClaimed { generation } => {
            match seam(repository_root, &flush_payload(), logger) {
                Ok(_) => match state::complete_recovery_flush(git_dir, generation)? {
                    state::RecoveryFlushCompletion::Cleared => {
                        readmit_after_flush(git_dir, key, turn_id, tool_name)
                    }
                    state::RecoveryFlushCompletion::Superseded => Ok(Admission::Denied),
                },
                Err(error) => {
                    log_pre_tool_use_fail_closed(logger, "recovery_flush", &error);
                    state::relinquish_recovery_flush(git_dir, generation)?;
                    Ok(Admission::Denied)
                }
            }
        }
    }
}

fn readmit_after_flush(
    git_dir: &Path,
    key: &AttemptKey,
    turn_id: &str,
    tool_name: &str,
) -> Result<Admission> {
    match state::admit_tracked_attempt(git_dir, key, turn_id, tool_name)? {
        state::AdmitDecision::Admitted(allocated) => Ok(Admission::Admitted(allocated)),
        state::AdmitDecision::FlushClaimed { generation } => {
            state::relinquish_recovery_flush(git_dir, generation)?;
            Ok(Admission::Denied)
        }
        state::AdmitDecision::RecoveryBlocked
        | state::AdmitDecision::UncertainAttemptBlocked
        | state::AdmitDecision::StalePredecessorBlocked => Ok(Admission::Denied),
    }
}

fn establish_start(
    git_dir: &Path,
    repository_root: &Path,
    allocated: &state::AllocatedAttempt,
    provenance: &CodexScopeProvenance,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<()> {
    let scope_id = &allocated.attempt.scope_id;

    if allocated.reused && allocated.attempt.phase == state::AttemptPhase::Active {
        return Ok(());
    }

    let start_payload =
        scope_start_payload(scope_id, &codex_scope_start_event_id(scope_id), provenance);

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
    state::arm_recovery(git_dir)?;

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

fn scope_start_payload(
    scope_id: &str,
    event_id: &str,
    provenance: &CodexScopeProvenance,
) -> String {
    json!({
        "operation": "start",
        "scope_id": scope_id,
        "event_id": event_id,
        "actor_kind": ACTOR_KIND_CODEX,
        PROVENANCE_FIELD: {
            "session_id": provenance.session_id,
            "model_id": provenance.model_id,
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
        object.insert(TOOL_INPUT_FIELD.to_string(), json!({"command": "true"}));
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
    fn ac3_pre_tool_use_fixtures_retain_the_codex_model() {
        for fixture in [
            PROBE01_SHELL_PRE,
            PROBE01_APPLY_PATCH_PRE,
            PROBE05_SHELL_PRE,
            PROBE08_AGENT_APPLY_PATCH_PRE,
        ] {
            assert_eq!(pre_tool_use(fixture).model.as_deref(), Some("gpt-5.6-sol"));
        }
    }

    #[test]
    fn ac3_scope_provenance_canonicalizes_the_session_and_normalizes_the_model() {
        let provenance = codex_scope_provenance(&pre_tool_use(PROBE01_SHELL_PRE));
        assert_eq!(
            provenance.session_id,
            "cx_01a07c1e-e08e-7172-8032-cb9d62af21d9"
        );
        assert_eq!(provenance.model_id.as_deref(), Some("gpt-5.6-sol"));

        let apply_patch = codex_scope_provenance(&pre_tool_use(PROBE01_APPLY_PATCH_PRE));
        assert_eq!(apply_patch, provenance);
    }

    #[test]
    fn ac3_scope_provenance_keeps_an_already_prefixed_session_id() {
        let execution = pre_tool_use(&pre_tool_use_json(&[(
            SESSION_ID_FIELD,
            Value::String("cx_session-1".to_string()),
        )]));
        assert_eq!(
            codex_scope_provenance(&execution).session_id,
            "cx_session-1"
        );
    }

    #[test]
    fn ac3_an_unusable_model_yields_no_model_id_without_rejecting_the_event() {
        for model in [
            Value::Null,
            Value::String(String::new()),
            Value::String("   ".to_string()),
            Value::Bool(true),
            json!(7),
            json!({ "id": "gpt-5.6-sol" }),
        ] {
            let payload = pre_tool_use_json(&[(MODEL_FIELD, model.clone())]);
            let execution = pre_tool_use(&payload);
            let provenance = codex_scope_provenance(&execution);
            assert_eq!(provenance.model_id, None, "model {model:?}");
            assert_eq!(provenance.session_id, "cx_session-1", "model {model:?}");
        }

        let absent = pre_tool_use(&pre_tool_use_json(&[]));
        assert_eq!(absent.model, None);
        assert_eq!(codex_scope_provenance(&absent).model_id, None);
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
        use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
        use std::sync::mpsc;
        use std::sync::{Arc, Mutex};
        use std::thread;
        use std::time::Duration;

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

        const BLOCKED_BASH_POLICY_RESPONSE: &str = concat!(
            r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","#,
            r#""permissionDecision":"deny","#,
            r#""permissionDecisionReason":"Blocked by SCE bash-tool policy 'no-danger': danger is not allowed"}}"#,
        );

        #[allow(clippy::unnecessary_wraps)]
        fn blocking_bash_policy(_root: &Path, _command: &str) -> Result<CodexBashPolicyDecision> {
            Ok(CodexBashPolicyDecision::Blocked(
                BLOCKED_BASH_POLICY_RESPONSE.to_string(),
            ))
        }

        #[allow(clippy::unnecessary_wraps)]
        fn allow_bash_policy(_root: &Path, _command: &str) -> Result<CodexBashPolicyDecision> {
            Ok(CodexBashPolicyDecision::Allowed)
        }

        fn failing_bash_policy(_root: &Path, _command: &str) -> Result<CodexBashPolicyDecision> {
            Err(anyhow!(
                "repository Bash policy configuration is invalid and could not be evaluated"
            ))
        }

        fn unreachable_bash_policy(_root: &Path, command: &str) -> Result<CodexBashPolicyDecision> {
            panic!("the Bash policy preflight must not run for this event (command: {command})");
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

        fn recording_seam(
            log: Arc<Mutex<Vec<String>>>,
        ) -> impl Fn(&Path, &str, Option<&dyn Logger>) -> Result<String> {
            move |_root, payload, _logger| {
                log.lock()
                    .expect("recording seam mutex")
                    .push(payload.to_string());
                Ok(String::new())
            }
        }

        struct SeamGate {
            entered: mpsc::Receiver<()>,
            release: mpsc::Sender<()>,
        }

        impl SeamGate {
            fn wait_until_entered(&self) {
                self.entered
                    .recv_timeout(Duration::from_secs(5))
                    .expect("gated seam should be entered");
            }

            fn release(&self) {
                let _ = self.release.send(());
            }
        }

        #[allow(clippy::type_complexity)]
        fn gated_seam(
            operation: &'static str,
            calls: Arc<Mutex<Vec<String>>>,
        ) -> (
            impl Fn(&Path, &str, Option<&dyn Logger>) -> Result<String> + Send,
            SeamGate,
        ) {
            let (entered_tx, entered_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let release_rx = Mutex::new(release_rx);
            let seam = move |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| {
                calls
                    .lock()
                    .expect("gated seam mutex")
                    .push(payload.to_string());
                if payload.contains(&format!(r#""operation":"{operation}""#)) {
                    entered_tx.send(()).expect("gate entry signal");
                    release_rx
                        .lock()
                        .expect("gate release mutex")
                        .recv()
                        .expect("gate release signal");
                }
                Ok(String::new())
            };
            (
                seam,
                SeamGate {
                    entered: entered_rx,
                    release: release_tx,
                },
            )
        }

        fn fixed_resolver(git_dir: PathBuf) -> impl Fn(&str) -> Result<PathBuf> + Send + Clone {
            move |_cwd| Ok(git_dir.clone())
        }

        fn panicking_resolver(_cwd: &str) -> Result<PathBuf> {
            panic!("resolve_git_dir must not be called for a non-tracked tool")
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

        const DRIVER_TURN: &str = "turn-1";

        fn seed_attempt(
            git_dir: &Path,
            session_id: &str,
            agent_id: Option<&str>,
            tool_use_id: &str,
            phase: state::AttemptPhase,
        ) -> state::AdapterAttempt {
            seed_attempt_in_turn(
                git_dir,
                session_id,
                DRIVER_TURN,
                agent_id,
                tool_use_id,
                phase,
            )
        }

        fn seed_attempt_in_turn(
            git_dir: &Path,
            session_id: &str,
            turn_id: &str,
            agent_id: Option<&str>,
            tool_use_id: &str,
            phase: state::AttemptPhase,
        ) -> state::AdapterAttempt {
            state::seed_attempt_for_tests(
                git_dir,
                &AttemptKey {
                    session_id: session_id.to_string(),
                    agent_id: agent_id.map(str::to_string),
                    tool_use_id: tool_use_id.to_string(),
                },
                turn_id,
                "Bash",
                phase,
            )
        }

        fn drive(
            payload: &str,
            resolver: &(impl Fn(&str) -> Result<PathBuf> + ?Sized),
            seam: &(impl Fn(&Path, &str, Option<&dyn Logger>) -> Result<String> + ?Sized),
        ) -> String {
            run_codex_mutation_scope_from_payload_with(payload, None, &resolver, &seam)
                .expect("driver should return Ok")
        }

        #[test]
        fn untracked_mcp_pre_tool_use_creates_no_scope_and_never_touches_seam_or_git_dir() {
            let payload = pre_tool_use_json(&[(
                TOOL_NAME_FIELD,
                Value::String("mcp__probe__mutate_success".to_string()),
            )]);
            let output = drive(&payload, &panicking_resolver, &unreachable_seam);
            assert_eq!(output, "");
        }

        #[test]
        fn unknown_and_delegation_pre_tool_use_create_no_scope_ac3() {
            for tool in [
                "some_future_codex_tool",
                "collaborationspawn_agent",
                "collaborationwait_agent",
            ] {
                let payload =
                    pre_tool_use_json(&[(TOOL_NAME_FIELD, Value::String(tool.to_string()))]);
                assert_eq!(drive(&payload, &panicking_resolver, &unreachable_seam), "");
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
                drive(&payload, &resolver, &ok_seam);
            }

            assert!(read_state(&git_dir).attempts.is_empty());
            assert!(read_state(&git_dir).recovery.is_clear());

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
                        "AC6: Start driven while attempt is PendingStart"
                    );
                    Ok(String::new())
                };

            let output = drive(&pre_tool_use_json(&[]), &resolver, &seam);
            assert_eq!(output, "");

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

        fn boundary_payload_field(payload: &str, field: &str) -> Option<Value> {
            let object: Map<String, Value> =
                serde_json::from_str(payload).expect("a boundary payload is a JSON object");
            object.get(field).cloned()
        }

        fn start_provenance(payload: &str) -> Value {
            assert_eq!(
                boundary_payload_field(payload, "operation"),
                Some(Value::String("start".to_string()))
            );
            boundary_payload_field(payload, PROVENANCE_FIELD)
                .expect("a Codex start payload carries provenance")
        }

        fn drive_recording_start(label: &str, payload: &str) -> Vec<String> {
            let git_dir = unique_test_git_dir(label);
            let resolver = fixed_resolver(git_dir.clone());
            let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

            drive(payload, &resolver, &recording_seam(Arc::clone(&recorded)));

            let calls = recorded.lock().expect("recording seam mutex").clone();
            remove_test_git_dir(&git_dir);
            calls
        }

        #[test]
        fn ac3_tracked_start_carries_scope_provenance_for_both_tracked_tools() {
            for tool in TRACKED_MUTATION_TOOL_NAMES {
                let payload = pre_tool_use_json(&[
                    (TOOL_NAME_FIELD, Value::String((*tool).to_string())),
                    (MODEL_FIELD, Value::String("gpt-5.6-sol".to_string())),
                ]);
                let calls = drive_recording_start(&format!("provenance-{tool}"), &payload);

                assert_eq!(calls.len(), 1, "{tool} should drive exactly one boundary");
                assert_eq!(
                    start_provenance(&calls[0]),
                    json!({ "session_id": "cx_session-1", "model_id": "gpt-5.6-sol" }),
                    "AC3: {tool} must carry its canonical session and normalized model"
                );
            }
        }

        #[test]
        fn ac3_a_start_without_a_usable_model_still_carries_its_session() {
            let cases: [(&str, Option<Value>); 4] = [
                ("absent", None),
                ("null", Some(Value::Null)),
                ("blank", Some(Value::String("   ".to_string()))),
                ("non-string", Some(Value::Bool(true))),
            ];

            for (label, model) in cases {
                let overrides = model.map_or_else(Vec::new, |value| vec![(MODEL_FIELD, value)]);
                let payload = pre_tool_use_json(&overrides);
                let calls = drive_recording_start(&format!("provenance-model-{label}"), &payload);

                assert_eq!(calls.len(), 1, "{label} should drive exactly one boundary");
                assert_eq!(
                    start_provenance(&calls[0]),
                    json!({ "session_id": "cx_session-1", "model_id": Value::Null }),
                    "AC3: a {label} model records no model without losing the session"
                );
            }
        }

        #[test]
        fn ac3_only_the_start_boundary_carries_provenance() {
            let git_dir = unique_test_git_dir("provenance-start-only");
            let resolver = fixed_resolver(git_dir.clone());
            let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let seam = recording_seam(Arc::clone(&recorded));

            drive(&pre_tool_use_json(&[]), &resolver, &seam);
            drive(&post_tool_use_json(&[]), &resolver, &seam);

            let calls = recorded.lock().expect("recording seam mutex").clone();
            assert_eq!(calls.len(), 2);
            assert!(boundary_payload_field(&calls[0], PROVENANCE_FIELD).is_some());
            assert_eq!(
                boundary_payload_field(&calls[1], "operation"),
                Some(Value::String("close".to_string()))
            );
            assert_eq!(boundary_payload_field(&calls[1], PROVENANCE_FIELD), None);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn duplicate_pre_tool_use_reuses_the_same_scope_id_ac4_test_e() {
            let git_dir = unique_test_git_dir("duplicate-pre");
            let resolver = fixed_resolver(git_dir.clone());

            drive(&pre_tool_use_json(&[]), &resolver, &ok_seam);
            let scope_id = read_state(&git_dir).attempts[0].scope_id.clone();

            drive(&pre_tool_use_json(&[]), &resolver, &ok_seam);

            let attempts = read_state(&git_dir).attempts;
            assert_eq!(
                attempts.len(),
                1,
                "AC4/Test E: a replay must not fork a new attempt"
            );
            assert_eq!(attempts[0].scope_id, scope_id);

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
            assert!(!output.contains("boom"));
            assert!(!output.contains("allow"));

            let warnings = logger.warnings();
            assert_eq!(warnings.len(), 1);
            assert_eq!(warnings[0].0, PRE_TOOL_USE_FAIL_CLOSED_EVENT);
            assert!(warnings[0].1.contains("boom"));
        }

        #[test]
        fn start_seam_failure_denies_and_leaves_the_pending_start_attempt_as_a_barrier_ac7() {
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
            assert!(!logger.warnings().is_empty());

            let final_state = read_state(&git_dir);
            assert_eq!(final_state.attempts.len(), 1);
            assert_eq!(
                final_state.attempts[0].phase,
                state::AttemptPhase::PendingStart
            );

            let successor = pre_tool_use_json(&[
                (
                    TOOL_USE_ID_FIELD,
                    Value::String("exec-successor".to_string()),
                ),
                (TURN_ID_FIELD, Value::String("turn-2".to_string())),
            ]);
            assert_eq!(
                run_codex_mutation_scope_from_payload_with(
                    &successor,
                    None,
                    &resolver,
                    &unreachable_seam,
                )
                .expect("successor must return Ok with a deny payload"),
                pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON),
                "I5: an unresolved PendingStart in another lane must block a successor Start",
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
                assert_eq!(
                    run_codex_mutation_scope_from_payload_with(
                        &payload,
                        None,
                        &resolver,
                        &unreachable_seam,
                    )
                    .expect("non-tracked PreToolUse should succeed"),
                    "",
                );
            }
        }

        #[test]
        fn successful_close_removes_the_attempt_ac8() {
            let git_dir = unique_test_git_dir("close-success");
            let resolver = fixed_resolver(git_dir.clone());

            drive(&pre_tool_use_json(&[]), &resolver, &ok_seam);

            let seen: RefCell<Vec<String>> = RefCell::new(Vec::new());
            let seam =
                |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    seen.borrow_mut().push(payload.to_string());
                    Ok(String::new())
                };
            assert_eq!(drive(&post_tool_use_json(&[]), &resolver, &seam), "");

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
            seed_attempt(
                &git_dir,
                "session-1",
                None,
                "exec-1",
                state::AttemptPhase::PendingStart,
            );

            let seen: RefCell<Vec<String>> = RefCell::new(Vec::new());
            let seam =
                |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    seen.borrow_mut().push(payload.to_string());
                    Ok(String::new())
                };
            drive(&post_tool_use_json(&[]), &resolver, &seam);

            let calls = seen.into_inner();
            assert_eq!(calls.len(), 1);
            assert!(calls[0].contains(r#""operation":"abandon""#));

            let final_state = read_state(&git_dir);
            assert!(final_state.attempts.is_empty());
            assert!(!final_state.recovery.is_clear());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn failed_close_abandons_and_arms_recovery_ac13() {
            let git_dir = unique_test_git_dir("failed-close");
            let resolver = fixed_resolver(git_dir.clone());

            drive(&pre_tool_use_json(&[]), &resolver, &ok_seam);

            let seam = seam_failing_on("close");
            drive(&post_tool_use_json(&[]), &resolver, &seam);

            let final_state = read_state(&git_dir);
            assert!(final_state.attempts.is_empty());
            assert!(
                !final_state.recovery.is_clear(),
                "D11: a failed Close arms recovery"
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn failed_close_and_failed_abandon_keep_the_attempt_tracked_and_recovery_armed_d11() {
            let git_dir = unique_test_git_dir("failed-close-and-abandon");
            let resolver = fixed_resolver(git_dir.clone());

            drive(&pre_tool_use_json(&[]), &resolver, &ok_seam);

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
            assert_eq!(final_state.attempts.len(), 1);
            assert!(!final_state.recovery.is_clear());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn post_tool_use_with_no_matching_attempt_is_a_noop() {
            let git_dir = unique_test_git_dir("close-no-attempt");
            let resolver = fixed_resolver(git_dir.clone());

            assert_eq!(
                drive(
                    &post_tool_use_json(&[(TOOL_NAME_FIELD, Value::String("Bash".to_string()),)]),
                    &resolver,
                    &unreachable_seam,
                ),
                "",
            );
            assert!(read_state(&git_dir).attempts.is_empty());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn stop_sweeps_only_main_thread_attempts_d12() {
            let git_dir = unique_test_git_dir("stop-sweep");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt(
                &git_dir,
                "session-1",
                None,
                "exec-main",
                state::AttemptPhase::Active,
            );
            seed_attempt(
                &git_dir,
                "session-1",
                Some("agent-1"),
                "exec-agent",
                state::AttemptPhase::Active,
            );

            drive(
                &turn_scoped_payload(HOOK_EVENT_STOP, "session-1", "turn-1"),
                &resolver,
                &ok_seam,
            );

            let attempts = read_state(&git_dir).attempts;
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].tool_use_id, "exec-agent");

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn interrupt_sweeps_every_attempt_for_the_session_d12() {
            let git_dir = unique_test_git_dir("interrupt-sweep");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt(
                &git_dir,
                "session-1",
                None,
                "exec-main",
                state::AttemptPhase::Active,
            );
            seed_attempt(
                &git_dir,
                "session-1",
                Some("agent-1"),
                "exec-agent",
                state::AttemptPhase::Active,
            );
            seed_attempt(
                &git_dir,
                "session-2",
                None,
                "exec-other",
                state::AttemptPhase::Active,
            );

            drive(
                &turn_scoped_payload(HOOK_EVENT_INTERRUPT, "session-1", "turn-1"),
                &resolver,
                &ok_seam,
            );

            let attempts = read_state(&git_dir).attempts;
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].session_id, "session-2");

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn subagent_stop_sweeps_only_the_matching_agent_d12() {
            let git_dir = unique_test_git_dir("subagent-stop-sweep");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt(
                &git_dir,
                "session-1",
                Some("agent-a"),
                "exec-a",
                state::AttemptPhase::Active,
            );
            seed_attempt(
                &git_dir,
                "session-1",
                Some("agent-b"),
                "exec-b",
                state::AttemptPhase::Active,
            );
            seed_attempt(
                &git_dir,
                "session-1",
                None,
                "exec-main",
                state::AttemptPhase::Active,
            );

            drive(
                &subagent_stop_payload("session-1", "turn-1", "agent-a"),
                &resolver,
                &ok_seam,
            );

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
            seed_attempt(
                &git_dir,
                "session-1",
                None,
                "exec-main",
                state::AttemptPhase::Active,
            );
            seed_attempt(
                &git_dir,
                "session-1",
                Some("agent-1"),
                "exec-agent",
                state::AttemptPhase::Active,
            );

            drive(&session_end_payload("session-1"), &resolver, &ok_seam);

            assert!(read_state(&git_dir).attempts.is_empty());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn lifecycle_cleanup_with_a_failed_abandon_keeps_the_attempt_tracked_d12() {
            let git_dir = unique_test_git_dir("sweep-failed-abandon");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt(
                &git_dir,
                "session-1",
                None,
                "exec-main",
                state::AttemptPhase::Active,
            );

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
            assert!(!final_state.recovery.is_clear());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn recovery_barrier_denies_new_tracked_pre_tool_use_while_attempts_remain_ac12() {
            let git_dir = unique_test_git_dir("barrier-attempts-remain");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt_in_turn(
                &git_dir,
                "session-1",
                "turn-other",
                None,
                "exec-live",
                state::AttemptPhase::Active,
            );
            state::arm_recovery(&git_dir).expect("arming the barrier should succeed");

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
        fn recovery_barrier_does_not_affect_untracked_pre_tool_use_ac12_test_f() {
            let git_dir = unique_test_git_dir("barrier-untracked-unaffected");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt(
                &git_dir,
                "session-1",
                None,
                "exec-live",
                state::AttemptPhase::Active,
            );
            state::arm_recovery(&git_dir).expect("arming the barrier should succeed");

            for tool in [
                "mcp__probe__mutate_success",
                "some_future_codex_tool",
                "collaborationspawn_agent",
            ] {
                let payload =
                    pre_tool_use_json(&[(TOOL_NAME_FIELD, Value::String(tool.to_string()))]);
                assert_eq!(
                    run_codex_mutation_scope_from_payload_with(
                        &payload,
                        None,
                        &resolver,
                        &unreachable_seam,
                    )
                    .expect("an untracked PreToolUse ignores the barrier"),
                    "",
                    "Test F: recovery must never deny an untracked tool",
                );
            }

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn recovery_barrier_flushes_once_quiescent_then_starts_ac12() {
            let git_dir = unique_test_git_dir("barrier-flush-success");
            std::fs::create_dir_all(&git_dir).expect("git dir should be created");
            let resolver = fixed_resolver(git_dir.clone());
            state::arm_recovery(&git_dir).expect("arming the barrier should succeed");

            let seen: RefCell<Vec<String>> = RefCell::new(Vec::new());
            let seam =
                |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| -> Result<String> {
                    seen.borrow_mut().push(payload.to_string());
                    Ok(String::new())
                };

            let output = drive(
                &pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("exec-new".to_string()))]),
                &resolver,
                &seam,
            );
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
                final_state.recovery.is_clear(),
                "a successful flush clears the barrier"
            );
            assert_eq!(final_state.attempts.len(), 1);
            assert_eq!(final_state.attempts[0].phase, state::AttemptPhase::Active);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn recovery_barrier_stays_closed_when_flush_fails_ac12() {
            let git_dir = unique_test_git_dir("barrier-flush-failure");
            std::fs::create_dir_all(&git_dir).expect("git dir should be created");
            let resolver = fixed_resolver(git_dir.clone());
            let generation =
                state::arm_recovery(&git_dir).expect("arming the barrier should succeed");

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
            assert_eq!(
                final_state.recovery,
                state::RecoveryState::Pending { generation },
                "a failed flush hands the generation back as Pending so a later PreToolUse retries",
            );
            assert!(final_state.attempts.is_empty());

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
            drive(&mcp_pre, &resolver, &unreachable_seam);
            assert!(read_state(&git_dir).attempts.is_empty());

            drive(
                &turn_scoped_payload(HOOK_EVENT_STOP, "session-1", "turn-1"),
                &resolver,
                &ok_seam,
            );
            drive(&session_end_payload("session-1"), &resolver, &ok_seam);

            let final_state = read_state(&git_dir);
            assert!(final_state.attempts.is_empty());
            assert!(
                final_state.recovery.is_clear(),
                "AC9c: no Start => no abandon => recovery stays clear"
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

            drive(&mcp_a, &resolver, &unreachable_seam);
            drive(&bash_b, &resolver, &ok_seam);

            let attempts = read_state(&git_dir).attempts;
            assert_eq!(attempts.len(), 1);
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
                drive(&payload, &resolver, &unreachable_seam);
                assert!(read_state(&git_dir).attempts.is_empty());
            }

            let final_state = read_state(&git_dir);
            assert!(final_state.attempts.is_empty());
            assert!(final_state.recovery.is_clear());

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

            drive(&predecessor_pre, &resolver, &ok_seam);
            drive(&predecessor_post, &resolver, &ok_seam);
            drive(&successor_pre, &resolver, &ok_seam);

            let attempts = read_state(&git_dir).attempts;
            assert_eq!(attempts.len(), 1);
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

        fn spawn_pre_tool_use(
            git_dir: &Path,
            tool_use_id: &'static str,
            seam: impl Fn(&Path, &str, Option<&dyn Logger>) -> Result<String> + Send + 'static,
        ) -> (thread::JoinHandle<String>, mpsc::Receiver<()>) {
            spawn_pre_tool_use_in_turn(git_dir, tool_use_id, DRIVER_TURN, seam)
        }

        fn spawn_pre_tool_use_in_turn(
            git_dir: &Path,
            tool_use_id: &'static str,
            turn_id: &'static str,
            seam: impl Fn(&Path, &str, Option<&dyn Logger>) -> Result<String> + Send + 'static,
        ) -> (thread::JoinHandle<String>, mpsc::Receiver<()>) {
            let (done_tx, done_rx) = mpsc::channel();
            let resolver = fixed_resolver(git_dir.to_path_buf());
            let handle = thread::spawn(move || {
                let output = run_codex_mutation_scope_from_payload_with(
                    &pre_tool_use_json(&[
                        (TOOL_USE_ID_FIELD, Value::String(tool_use_id.to_string())),
                        (TURN_ID_FIELD, Value::String(turn_id.to_string())),
                    ]),
                    None,
                    &resolver,
                    &seam,
                )
                .expect("PreToolUse should return Ok");
                let _ = done_tx.send(());
                output
            });
            (handle, done_rx)
        }

        fn assert_still_blocked(done_rx: &mpsc::Receiver<()>, context: &str) {
            assert!(
                done_rx.recv_timeout(Duration::from_millis(250)).is_err(),
                "{context}: the operation must still be blocked on the boundary lock",
            );
        }

        fn first_index_of(recorded: &[String], operation: &str) -> Option<usize> {
            recorded
                .iter()
                .position(|payload| payload.contains(&format!(r#""operation":"{operation}""#)))
        }

        #[test]
        fn test_h_cleanup_owning_the_boundary_lock_blocks_admission_until_recovery_is_processed() {
            let git_dir = unique_test_git_dir("test-h-cleanup-owns-boundary");
            std::fs::create_dir_all(&git_dir).expect("git dir should be created");
            seed_attempt(
                &git_dir,
                "session-1",
                None,
                "exec-a",
                state::AttemptPhase::Active,
            );

            let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let (abandon_seam, gate) = gated_seam("abandon", Arc::clone(&recorded));

            let sweeper = {
                let resolver = fixed_resolver(git_dir.clone());
                thread::spawn(move || {
                    run_codex_mutation_scope_from_payload_with(
                        &session_end_payload("session-1"),
                        None,
                        &resolver,
                        &abandon_seam,
                    )
                    .expect("SessionEnd cleanup should succeed")
                })
            };

            gate.wait_until_entered();
            assert_eq!(
                read_state(&git_dir).recovery,
                state::RecoveryState::Pending { generation: 1 },
                "cleanup arms recovery while it owns the boundary lock",
            );

            let (b_handle, b_done) =
                spawn_pre_tool_use(&git_dir, "exec-b", recording_seam(Arc::clone(&recorded)));
            assert_still_blocked(&b_done, "Test H");
            assert!(
                first_index_of(&recorded.lock().unwrap(), "start").is_none(),
                "Test H: B must not reach Start while cleanup owns the boundary lock",
            );

            gate.release();
            sweeper.join().expect("sweeper thread should not panic");

            let b_output = b_handle.join().expect("B thread should not panic");
            assert_eq!(
                b_output, "",
                "Test H: once recovery is processed B proceeds"
            );

            let recorded = recorded.lock().unwrap().clone();
            let abandon_at =
                first_index_of(&recorded, "abandon").expect("cleanup abandoned exec-a");
            let flush_at = first_index_of(&recorded, "flush").expect("B drove the quiescent flush");
            let start_at = first_index_of(&recorded, "start").expect("B reached Start");
            assert!(
                abandon_at < flush_at && flush_at < start_at,
                "Test H: the serialized order must be abandon -> flush -> start, got {recorded:?}",
            );

            let final_state = read_state(&git_dir);
            assert!(final_state.recovery.is_clear());
            assert_eq!(final_state.attempts.len(), 1);
            assert_eq!(final_state.attempts[0].tool_use_id, "exec-b");
            assert_eq!(final_state.attempts[0].phase, state::AttemptPhase::Active);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn test_g_admission_completed_recovery_cannot_arm_before_start() {
            let git_dir = unique_test_git_dir("test-g-admit-before-start");
            std::fs::create_dir_all(&git_dir).expect("git dir should be created");

            let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let (start_seam, gate) = gated_seam("start", Arc::clone(&recorded));

            let p1 = {
                let resolver = fixed_resolver(git_dir.clone());
                thread::spawn(move || {
                    run_codex_mutation_scope_from_payload_with(
                        &pre_tool_use_json(&[(
                            TOOL_USE_ID_FIELD,
                            Value::String("exec-b".to_string()),
                        )]),
                        None,
                        &resolver,
                        &start_seam,
                    )
                    .expect("P1 PreToolUse should return Ok")
                })
            };

            gate.wait_until_entered();
            let mid = read_state(&git_dir);
            assert_eq!(mid.attempts.len(), 1);
            assert_eq!(mid.attempts[0].phase, state::AttemptPhase::PendingStart);
            assert!(
                mid.recovery.is_clear(),
                "recovery must still be Clear while P1 holds the boundary lock pre-Start",
            );

            let (p2_handle, p2_done) = spawn_pre_tool_use(
                &git_dir,
                "exec-cleanup-trigger",
                recording_seam(Arc::clone(&recorded)),
            );

            let sweeper = {
                let resolver = fixed_resolver(git_dir.clone());
                let recorded = Arc::clone(&recorded);
                thread::spawn(move || {
                    let seam = recording_seam(recorded);
                    run_codex_mutation_scope_from_payload_with(
                        &session_end_payload("session-1"),
                        None,
                        &resolver,
                        &seam,
                    )
                    .expect("SessionEnd cleanup should return Ok")
                })
            };

            assert_still_blocked(&p2_done, "Test G");
            assert!(
                read_state(&git_dir).recovery.is_clear(),
                "Test G: no concurrent process may arm recovery between admit(B) and Start(B)",
            );

            gate.release();
            assert_eq!(p1.join().expect("P1 should not panic"), "");
            sweeper.join().expect("sweeper should not panic");
            p2_handle.join().expect("P2 should not panic");

            let recorded = recorded.lock().unwrap().clone();
            let start_at = first_index_of(&recorded, "start").expect("P1 drove Start(B)");
            if let Some(abandon_at) = first_index_of(&recorded, "abandon") {
                assert!(
                    start_at < abandon_at,
                    "Test G: Start(B) must be serialized before any later abandon, got {recorded:?}",
                );
            }

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn test_j_a_live_flush_owner_is_never_reclaimed_by_a_blocked_process() {
            let git_dir = unique_test_git_dir("test-j-live-flush-owner");
            std::fs::create_dir_all(&git_dir).expect("git dir should be created");
            state::arm_recovery(&git_dir).expect("arm recovery");

            let flush_count = Arc::new(AtomicUsize::new(0));
            let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let (owner_gated, gate) = gated_seam("flush", Arc::clone(&recorded));

            let owner = {
                let resolver = fixed_resolver(git_dir.clone());
                let flush_count = Arc::clone(&flush_count);
                thread::spawn(move || {
                    let seam = move |root: &Path,
                                     payload: &str,
                                     logger: Option<&dyn Logger>|
                          -> Result<String> {
                        if payload.contains(r#""operation":"flush""#) {
                            flush_count.fetch_add(1, Ordering::SeqCst);
                        }
                        owner_gated(root, payload, logger)
                    };
                    run_codex_mutation_scope_from_payload_with(
                        &pre_tool_use_json(&[(
                            TOOL_USE_ID_FIELD,
                            Value::String("exec-owner".to_string()),
                        )]),
                        None,
                        &resolver,
                        &seam,
                    )
                    .expect("owner PreToolUse should return Ok")
                })
            };

            gate.wait_until_entered();
            assert_eq!(
                read_state(&git_dir).recovery,
                state::RecoveryState::Flushing { generation: 1 },
            );

            let flush_count_p2 = Arc::clone(&flush_count);
            let (p2_handle, p2_done) =
                spawn_pre_tool_use_in_turn(&git_dir, "exec-2", "turn-2", move |_r, payload, _l| {
                    if payload.contains(r#""operation":"flush""#) {
                        flush_count_p2.fetch_add(1, Ordering::SeqCst);
                    }
                    Ok(String::new())
                });

            assert_still_blocked(&p2_done, "Test J");
            assert_eq!(
                read_state(&git_dir).recovery,
                state::RecoveryState::Flushing { generation: 1 },
                "Test J: a blocked process must not reclaim the live owner's Flushing(g)",
            );

            gate.release();
            assert_eq!(owner.join().expect("owner should not panic"), "");
            assert_eq!(p2_handle.join().expect("P2 should not panic"), "");

            assert_eq!(
                flush_count.load(Ordering::SeqCst),
                1,
                "Test J: exactly one Flush ran — the live owner's, never a reclaim",
            );
            let final_state = read_state(&git_dir);
            assert!(final_state.recovery.is_clear());
            assert_eq!(final_state.attempts.len(), 2);
            assert!(final_state
                .attempts
                .iter()
                .all(|a| a.phase == state::AttemptPhase::Active));

            remove_test_git_dir(&git_dir);
        }

        fn seed_orphaned_flushing(git_dir: &Path) -> u64 {
            let generation = state::arm_recovery(git_dir).expect("arm recovery to seed");
            match state::admit_tracked_attempt(
                git_dir,
                &key("seed", None, "seed"),
                "seed-turn",
                "Bash",
            )
            .expect("seeding admit should not error")
            {
                state::AdmitDecision::FlushClaimed {
                    generation: claimed,
                } => {
                    assert_eq!(claimed, generation);
                }
                other => panic!("expected FlushClaimed while seeding, got {other:?}"),
            }
            assert_eq!(
                read_state(git_dir).recovery,
                state::RecoveryState::Flushing { generation },
                "seed left durable Flushing(g) with no live boundary-lock owner",
            );
            generation
        }

        #[test]
        fn test_i_orphaned_flushing_is_reclaimed_and_flush_is_retried_once() {
            let git_dir = unique_test_git_dir("test-i-orphaned-flushing");
            std::fs::create_dir_all(&git_dir).expect("git dir should be created");
            let generation = seed_orphaned_flushing(&git_dir);
            let next_generation_before = read_state(&git_dir).next_recovery_generation;

            let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let resolver = fixed_resolver(git_dir.clone());
            let output = drive(
                &pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("exec-x".to_string()))]),
                &resolver,
                &recording_seam(Arc::clone(&recorded)),
            );
            assert_eq!(output, "");

            let ops = recorded.lock().unwrap().clone();
            assert_eq!(
                ops.iter()
                    .filter(|p| p.contains(r#""operation":"flush""#))
                    .count(),
                1,
                "Test I: exactly one retry Flush for the reclaimed generation, got {ops:?}",
            );
            assert!(
                first_index_of(&ops, "flush").unwrap() < first_index_of(&ops, "start").unwrap()
            );

            let final_state = read_state(&git_dir);
            assert!(
                final_state.recovery.is_clear(),
                "Test I: no permanent RecoveryBlocked"
            );
            assert_eq!(
                final_state.next_recovery_generation, next_generation_before,
                "Test I: reclaiming Flushing(g) preserves the generation, never bumps it",
            );
            assert_eq!(final_state.attempts.len(), 1);
            assert_eq!(final_state.attempts[0].tool_use_id, "exec-x");
            assert_eq!(final_state.attempts[0].phase, state::AttemptPhase::Active);
            let _ = generation;

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn test_k_crash_after_durable_flush_before_completion_write_converges() {
            let git_dir = unique_test_git_dir("test-k-crash-after-flush");
            std::fs::create_dir_all(&git_dir).expect("git dir should be created");
            seed_orphaned_flushing(&git_dir);

            let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let resolver = fixed_resolver(git_dir.clone());

            let first = drive(
                &pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("exec-1".to_string()))]),
                &resolver,
                &recording_seam(Arc::clone(&recorded)),
            );
            assert_eq!(first, "");
            assert!(read_state(&git_dir).recovery.is_clear());

            let second = drive(
                &pre_tool_use_json(&[
                    (TOOL_USE_ID_FIELD, Value::String("exec-2".to_string())),
                    (TURN_ID_FIELD, Value::String("turn-2".to_string())),
                ]),
                &resolver,
                &recording_seam(Arc::clone(&recorded)),
            );
            assert_eq!(second, "");

            let ops = recorded.lock().unwrap().clone();
            assert_eq!(
                ops.iter()
                    .filter(|p| p.contains(r#""operation":"flush""#))
                    .count(),
                1,
                "Test K: the recovery retry Flush runs exactly once across convergence, got {ops:?}",
            );
            let final_state = read_state(&git_dir);
            assert!(final_state.recovery.is_clear());
            assert_eq!(final_state.attempts.len(), 2);
            assert!(final_state
                .attempts
                .iter()
                .all(|a| a.phase == state::AttemptPhase::Active));

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn test_l_duplicate_active_delivery_drives_no_second_start() {
            let git_dir = unique_test_git_dir("test-l-duplicate-active");
            std::fs::create_dir_all(&git_dir).expect("git dir should be created");
            let resolver = fixed_resolver(git_dir.clone());
            let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

            let first = drive(
                &pre_tool_use_json(&[]),
                &resolver,
                &recording_seam(Arc::clone(&recorded)),
            );
            assert_eq!(first, "");
            let scope_id = read_state(&git_dir).attempts[0].scope_id.clone();
            assert_eq!(
                read_state(&git_dir).attempts[0].phase,
                state::AttemptPhase::Active
            );

            let duplicate = drive(
                &pre_tool_use_json(&[]),
                &resolver,
                &recording_seam(Arc::clone(&recorded)),
            );
            assert_eq!(duplicate, "");

            let ops = recorded.lock().unwrap().clone();
            assert_eq!(
                ops.iter()
                    .filter(|p| p.contains(r#""operation":"start""#))
                    .count(),
                1,
                "Test L: duplicate delivery of an Active execution drives no second Start, got {ops:?}",
            );
            let attempts = read_state(&git_dir).attempts;
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].scope_id, scope_id);
            assert_eq!(read_state(&git_dir).next_attempt_seq, 2);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn test_m_untracked_tools_never_touch_the_boundary_lock() {
            let git_dir = unique_test_git_dir("test-m-untracked-no-boundary");
            let resolver = fixed_resolver(git_dir.clone());

            for tool in [
                "mcp__probe__mutate_success",
                "some_future_codex_tool",
                "collaborationspawn_agent",
                "collaborationwait_agent",
            ] {
                let payload =
                    pre_tool_use_json(&[(TOOL_NAME_FIELD, Value::String(tool.to_string()))]);
                assert_eq!(
                    run_codex_mutation_scope_from_payload_with(
                        &payload,
                        None,
                        &panicking_resolver,
                        &unreachable_seam,
                    )
                    .expect("an untracked PreToolUse is neutral"),
                    "",
                );
                assert_eq!(drive(&payload, &resolver, &unreachable_seam), "");
            }

            assert!(
                !crate::services::hooks::codex_mutation_scope::boundary_lock::boundary_lock_path(
                    &git_dir
                )
                .exists(),
                "Test M: no untracked tool may create the adapter boundary lock",
            );
            assert!(
                !state::adapter_state_dir(&git_dir).exists(),
                "Test M: an untracked tool resolves no git dir and touches no adapter state",
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn untracked_post_tool_use_never_touches_mutation_scope_machinery() {
            let git_dir = unique_test_git_dir("untracked-post-no-footprint");
            let resolver = fixed_resolver(git_dir.clone());

            for tool in [
                "mcp__probe__mutate_success",
                "some_future_codex_tool",
                "collaborationspawn_agent",
                "collaborationwait_agent",
            ] {
                let payload =
                    post_tool_use_json(&[(TOOL_NAME_FIELD, Value::String(tool.to_string()))]);

                assert_eq!(
                    run_codex_mutation_scope_from_payload_with(
                        &payload,
                        None,
                        &panicking_resolver,
                        &unreachable_seam,
                    )
                    .expect("an untracked PostToolUse is neutral"),
                    "",
                    "untracked PostToolUse for {tool:?} must return neutral",
                );
                assert_eq!(
                    drive(&payload, &resolver, &unreachable_seam),
                    "",
                    "untracked PostToolUse for {tool:?} must not call the ingress seam",
                );
            }

            assert!(
                !state::adapter_state_dir(&git_dir).exists(),
                "an untracked PostToolUse resolves no git dir and creates no adapter state directory",
            );
            assert!(
                !crate::services::hooks::codex_mutation_scope::boundary_lock::boundary_lock_path(
                    &git_dir
                )
                .exists(),
                "an untracked PostToolUse must not create the adapter boundary lock",
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn a_complete_successful_mcp_lifecycle_leaves_zero_adapter_footprint() {
            let git_dir = unique_test_git_dir("mcp-lifecycle-no-footprint");
            let resolver = fixed_resolver(git_dir.clone());

            let mcp = &[(
                TOOL_NAME_FIELD,
                Value::String("mcp__probe__mutate_success".to_string()),
            )];

            assert_eq!(
                run_codex_mutation_scope_from_payload_with(
                    &pre_tool_use_json(mcp),
                    None,
                    &panicking_resolver,
                    &unreachable_seam,
                )
                .expect("MCP PreToolUse is neutral"),
                "",
            );
            assert_eq!(
                run_codex_mutation_scope_from_payload_with(
                    &post_tool_use_json(mcp),
                    None,
                    &panicking_resolver,
                    &unreachable_seam,
                )
                .expect("MCP PostToolUse is neutral"),
                "",
            );

            assert_eq!(
                drive(&pre_tool_use_json(mcp), &resolver, &unreachable_seam),
                ""
            );
            assert_eq!(
                drive(&post_tool_use_json(mcp), &resolver, &unreachable_seam),
                ""
            );

            let state = read_state(&git_dir);
            assert!(state.attempts.is_empty(), "no attempts recorded");
            assert!(state.recovery.is_clear(), "recovery stays Clear");

            assert!(
                !state::adapter_state_dir(&git_dir).exists(),
                "a complete successful MCP lifecycle creates no adapter state directory, \
                 state lock, or boundary lock",
            );
            assert!(
                !crate::services::hooks::codex_mutation_scope::boundary_lock::boundary_lock_path(
                    &git_dir
                )
                .exists(),
                "a complete successful MCP lifecycle creates no boundary lock",
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn test_c_recovery_rearmed_while_flush_in_flight_survives_the_stale_completion() {
            let git_dir = unique_test_git_dir("race-rearm-during-flush");
            std::fs::create_dir_all(&git_dir).expect("git dir should be created");
            state::arm_recovery(&git_dir).expect("arm g1");

            let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let (flush_seam, gate) = gated_seam("flush", Arc::clone(&calls));

            let flusher = {
                let git_dir = git_dir.clone();
                let resolver = fixed_resolver(git_dir.clone());
                thread::spawn(move || {
                    run_codex_mutation_scope_from_payload_with(
                        &pre_tool_use_json(&[(
                            TOOL_USE_ID_FIELD,
                            Value::String("exec-flusher".to_string()),
                        )]),
                        None,
                        &resolver,
                        &flush_seam,
                    )
                    .expect("flusher PreToolUse should return Ok")
                })
            };

            gate.wait_until_entered();
            assert_eq!(
                read_state(&git_dir).recovery,
                state::RecoveryState::Flushing { generation: 1 },
            );

            let second_generation = state::arm_recovery(&git_dir).expect("re-arm to g2");
            assert_eq!(second_generation, 2);

            gate.release();
            let flusher_output = flusher.join().expect("flusher thread should not panic");
            assert_eq!(
                flusher_output,
                pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON),
                "Test C: the flusher denies because recovery was re-armed under it",
            );

            assert_eq!(
                read_state(&git_dir).recovery,
                state::RecoveryState::Pending { generation: 2 },
                "Test C: the stale Flush(g1) completion must not clear Pending(g2)",
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn test_d_start_succeeds_but_mark_active_fails_blocks_a_successor_until_recovery() {
            let git_dir = unique_test_git_dir("start-then-mark-active-fails");
            let resolver = fixed_resolver(git_dir.clone());

            state::arm_mark_active_failure_for_tests();
            let logger = RecordingLogger::default();
            let output = run_codex_mutation_scope_from_payload_with(
                &pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("exec-a".to_string()))]),
                Some(&logger),
                &resolver,
                &ok_seam,
            )
            .expect("a mark_active failure still returns Ok with a deny payload");
            assert_eq!(output, pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON));

            let after_start = read_state(&git_dir);
            assert_eq!(after_start.attempts.len(), 1);
            assert_eq!(
                after_start.attempts[0].phase,
                state::AttemptPhase::PendingStart
            );
            assert!(after_start.recovery.is_clear());

            let successor = pre_tool_use_json(&[
                (TOOL_USE_ID_FIELD, Value::String("exec-b".to_string())),
                (TURN_ID_FIELD, Value::String("turn-2".to_string())),
            ]);
            assert_eq!(
                run_codex_mutation_scope_from_payload_with(
                    &successor,
                    None,
                    &resolver,
                    &unreachable_seam,
                )
                .expect("successor returns a deny payload"),
                pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON),
                "Test D: an uncertain PendingStart in another lane blocks a successor Start",
            );

            drive(&session_end_payload("session-1"), &resolver, &ok_seam);
            assert!(read_state(&git_dir).attempts.is_empty());
            assert!(!read_state(&git_dir).recovery.is_clear());

            let recording: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let seam = recording_seam(Arc::clone(&recording));
            let recovered = drive(&successor, &resolver, &seam);
            assert_eq!(recovered, "");

            let ops = recording.lock().expect("recording mutex").clone();
            assert_eq!(ops.len(), 2, "expected flush then start, got {ops:?}");
            assert!(ops[0].contains(r#""operation":"flush""#));
            assert!(ops[1].contains(r#""operation":"start""#));

            let final_state = read_state(&git_dir);
            assert!(final_state.recovery.is_clear());
            assert_eq!(final_state.attempts.len(), 1);
            assert_eq!(final_state.attempts[0].phase, state::AttemptPhase::Active);
            assert_eq!(final_state.attempts[0].tool_use_id, "exec-b");

            remove_test_git_dir(&git_dir);
        }

        fn boundary_lock_exists(git_dir: &Path) -> bool {
            crate::services::hooks::codex_mutation_scope::boundary_lock::boundary_lock_path(git_dir)
                .exists()
        }

        #[test]
        fn policy_blocked_bash_pre_tool_use_creates_no_mutation_scope_state() {
            let git_dir = unique_test_git_dir("policy-blocked-no-scope");

            let output = run_codex_mutation_scope_from_payload_with_bash_policy(
                &pre_tool_use_json(&[(TOOL_INPUT_FIELD, json!({"command": "danger --now"}))]),
                None,
                &panicking_resolver,
                &unreachable_seam,
                &blocking_bash_policy,
            )
            .expect("a policy-blocked Bash PreToolUse still returns Ok");

            assert_eq!(
                output, BLOCKED_BASH_POLICY_RESPONSE,
                "a policy block returns the Codex-native policy denial verbatim, \
                 never the generic mutation-scope deny",
            );
            assert!(!output.contains(FAIL_CLOSED_DENY_REASON));
            assert!(
                !state::adapter_state_dir(&git_dir).exists(),
                "a policy block must leave no adapter state: no PendingStart, Active, \
                 Start, Abandon, Flush, or recovery",
            );
            assert!(
                !boundary_lock_exists(&git_dir),
                "a policy block must not even acquire the boundary lock",
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn policy_allowed_bash_pre_tool_use_follows_the_normal_write_ahead_start_path() {
            let git_dir = unique_test_git_dir("policy-allowed-start");
            let resolver = fixed_resolver(git_dir.clone());
            let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

            let output = run_codex_mutation_scope_from_payload_with_bash_policy(
                &pre_tool_use_json(&[]),
                None,
                &resolver,
                &recording_seam(Arc::clone(&recorded)),
                &allow_bash_policy,
            )
            .expect("an allowed Bash PreToolUse returns Ok");
            assert_eq!(output, "");

            let ops = recorded.lock().unwrap().clone();
            assert_eq!(
                ops.iter()
                    .filter(|payload| payload.contains(r#""operation":"start""#))
                    .count(),
                1,
                "an allowed Bash still drives exactly one write-ahead Start, got {ops:?}",
            );
            let attempts = read_state(&git_dir).attempts;
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].phase, state::AttemptPhase::Active);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn policy_evaluation_failure_is_fail_closed_with_no_mutation_scope_state() {
            let git_dir = unique_test_git_dir("policy-eval-failure");
            let logger = RecordingLogger::default();

            let output = run_codex_mutation_scope_from_payload_with_bash_policy(
                &pre_tool_use_json(&[]),
                Some(&logger),
                &panicking_resolver,
                &unreachable_seam,
                &failing_bash_policy,
            )
            .expect("a Bash policy evaluation failure still returns Ok");

            assert_eq!(
                output,
                pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON),
                "a policy evaluation failure fails closed with the generic mutation-scope deny",
            );
            let warnings = logger.warnings();
            assert_eq!(warnings.len(), 1);
            assert_eq!(warnings[0].0, PRE_TOOL_USE_FAIL_CLOSED_EVENT);
            assert!(warnings[0].1.contains("could not be evaluated"));
            assert!(
                !state::adapter_state_dir(&git_dir).exists(),
                "a fail-closed policy evaluation must leave no adapter state",
            );

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn apply_patch_pre_tool_use_never_evaluates_bash_policy() {
            let git_dir = unique_test_git_dir("apply-patch-no-policy");
            let resolver = fixed_resolver(git_dir.clone());

            let output = run_codex_mutation_scope_from_payload_with_bash_policy(
                &pre_tool_use_json(&[
                    (TOOL_NAME_FIELD, Value::String("apply_patch".to_string())),
                    (TOOL_INPUT_FIELD, Value::Null),
                ]),
                None,
                &resolver,
                &ok_seam,
                &unreachable_bash_policy,
            )
            .expect("an apply_patch PreToolUse returns Ok");
            assert_eq!(output, "");

            let attempts = read_state(&git_dir).attempts;
            assert_eq!(attempts.len(), 1, "apply_patch still establishes a scope");
            assert_eq!(attempts[0].phase, state::AttemptPhase::Active);

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn malformed_bash_tool_input_is_fail_closed_before_the_policy_evaluator_runs() {
            let git_dir = unique_test_git_dir("malformed-bash-tool-input");
            let logger = RecordingLogger::default();

            let output = run_codex_mutation_scope_from_payload_with_bash_policy(
                &pre_tool_use_json(&[(TOOL_INPUT_FIELD, json!({"not_command": "x"}))]),
                Some(&logger),
                &panicking_resolver,
                &unreachable_seam,
                &unreachable_bash_policy,
            )
            .expect("a malformed Bash tool_input still returns Ok");

            assert_eq!(output, pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON));
            let warnings = logger.warnings();
            assert_eq!(warnings.len(), 1);
            assert_eq!(warnings[0].0, PRE_TOOL_USE_FAIL_CLOSED_EVENT);
            assert!(
                warnings[0].1.contains("tool_input.command"),
                "extraction reuses the shared bash_command_from_tool_input semantics",
            );
            assert!(!state::adapter_state_dir(&git_dir).exists());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn production_bash_policy_evaluator_blocks_a_repo_denied_command_before_any_start() {
            let repo = unique_test_git_dir("prod-policy-regression");
            std::fs::create_dir_all(repo.join(".sce")).expect("create .sce dir");
            std::fs::write(
                repo.join(".sce").join("config.json"),
                concat!(
                    r#"{"policies":{"bash":{"custom":[{"id":"no-rm","#,
                    r#""match":{"argv_prefix":["rm"]},"#,
                    r#""message":"rm is blocked in this repository"}]}}}"#,
                ),
            )
            .expect("write repo bash policy config");

            let real_evaluator =
                |root: &Path, command: &str| evaluate_codex_bash_policy(root, command);

            let blocked = run_codex_mutation_scope_from_payload_with_bash_policy(
                &pre_tool_use_json(&[
                    (
                        CWD_FIELD,
                        Value::String(repo.to_string_lossy().into_owned()),
                    ),
                    (TOOL_INPUT_FIELD, json!({"command": "rm -rf build"})),
                ]),
                None,
                &panicking_resolver,
                &unreachable_seam,
                &real_evaluator,
            )
            .expect("a repo-denied Bash command still returns Ok");

            assert!(blocked.contains(r#""permissionDecision":"deny""#));
            assert!(blocked.contains("no-rm"));
            assert!(blocked.contains("rm is blocked in this repository"));
            assert!(
                !blocked.contains(FAIL_CLOSED_DENY_REASON),
                "a real policy block keeps the policy-specific UX, not the generic deny",
            );

            let allowed = run_codex_mutation_scope_from_payload_with_bash_policy(
                &pre_tool_use_json(&[
                    (
                        CWD_FIELD,
                        Value::String(repo.to_string_lossy().into_owned()),
                    ),
                    (TOOL_INPUT_FIELD, json!({"command": "echo ok > ok.txt"})),
                    (TOOL_USE_ID_FIELD, Value::String("exec-allowed".to_string())),
                ]),
                None,
                &fixed_resolver(repo.join(".git")),
                &ok_seam,
                &real_evaluator,
            )
            .expect("an allowed Bash command still returns Ok");
            assert_eq!(
                allowed, "",
                "the same repo config lets a non-denied command through to the normal path",
            );

            remove_test_git_dir(&repo);
        }

        fn operations(recorded: &[String]) -> Vec<String> {
            recorded
                .iter()
                .filter_map(|payload| {
                    for op in ["start", "close", "abandon", "flush"] {
                        if payload.contains(&format!(r#""operation":"{op}""#)) {
                            return Some(op.to_string());
                        }
                    }
                    None
                })
                .collect()
        }

        fn pre(tool_use_id: &str, tool_name: &str, overrides: &[(&str, Value)]) -> String {
            let mut merged: Vec<(&str, Value)> = vec![
                (TOOL_USE_ID_FIELD, Value::String(tool_use_id.to_string())),
                (TOOL_NAME_FIELD, Value::String(tool_name.to_string())),
            ];
            if tool_name != CODEX_TRACKED_TOOL_BASH {
                merged.push((TOOL_INPUT_FIELD, Value::Null));
            }
            merged.extend(
                overrides
                    .iter()
                    .map(|(field, value)| (*field, value.clone())),
            );
            pre_tool_use_json(&merged)
        }

        fn assert_zombie_then_successor_sweep(
            a_tool: &str,
            b_tool: &str,
            b_overrides: &[(&str, Value)],
        ) {
            let git_dir = unique_test_git_dir(&format!("zombie-successor-{a_tool}-{b_tool}"));
            let resolver = fixed_resolver(git_dir.clone());
            let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

            let started = drive(
                &pre("exec-a", a_tool, &[]),
                &resolver,
                &recording_seam(Arc::clone(&recorded)),
            );
            assert_eq!(started, "");
            assert_eq!(
                read_state(&git_dir).attempts[0].phase,
                state::AttemptPhase::Active,
            );

            let successor = drive(
                &pre("exec-b", b_tool, b_overrides),
                &resolver,
                &recording_seam(Arc::clone(&recorded)),
            );
            assert_eq!(successor, "");

            let ops = operations(&recorded.lock().unwrap());
            assert_eq!(
                ops,
                vec![
                    "start".to_string(),
                    "abandon".to_string(),
                    "flush".to_string(),
                    "start".to_string(),
                ],
                "successor sequence must be Start(A) -> Abandon(A) -> Flush -> Start(B), never Start(A) -> Start(B) -> Abandon(A)",
            );

            let final_state = read_state(&git_dir);
            assert_eq!(final_state.attempts.len(), 1);
            assert_eq!(final_state.attempts[0].tool_use_id, "exec-b");
            assert_eq!(final_state.attempts[0].phase, state::AttemptPhase::Active);
            assert!(final_state.recovery.is_clear());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn regression1_arbitrary_blocker_zombie_then_tracked_successor() {
            assert_zombie_then_successor_sweep("Bash", "Bash", &[]);
        }

        #[test]
        fn regression2_apply_patch_successor_variants() {
            assert_zombie_then_successor_sweep("Bash", "apply_patch", &[]);
            assert_zombie_then_successor_sweep("apply_patch", "Bash", &[]);
            assert_zombie_then_successor_sweep("apply_patch", "apply_patch", &[]);
        }

        #[test]
        fn regression3_parent_then_subagent_same_lane_is_swept() {
            assert_zombie_then_successor_sweep(
                "Bash",
                "Bash",
                &[(AGENT_ID_FIELD, Value::String("agent-1".to_string()))],
            );
        }

        #[test]
        fn regression3_subagent_then_parent_same_lane_is_swept() {
            let git_dir = unique_test_git_dir("subagent-then-parent");
            let resolver = fixed_resolver(git_dir.clone());
            let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

            drive(
                &pre(
                    "exec-a",
                    "Bash",
                    &[(AGENT_ID_FIELD, Value::String("agent-1".to_string()))],
                ),
                &resolver,
                &recording_seam(Arc::clone(&recorded)),
            );
            drive(
                &pre("exec-b", "Bash", &[]),
                &resolver,
                &recording_seam(Arc::clone(&recorded)),
            );

            assert_eq!(
                operations(&recorded.lock().unwrap()),
                vec![
                    "start".to_string(),
                    "abandon".to_string(),
                    "flush".to_string(),
                    "start".to_string(),
                ],
            );
            let final_state = read_state(&git_dir);
            assert_eq!(final_state.attempts.len(), 1);
            assert_eq!(final_state.attempts[0].tool_use_id, "exec-b");
            assert!(final_state.attempts[0].agent_id.is_none());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn regression4_different_session_is_not_swept() {
            let git_dir = unique_test_git_dir("different-session-not-swept");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt_in_turn(
                &git_dir,
                "session-1",
                "turn-1",
                None,
                "exec-a",
                state::AttemptPhase::Active,
            );
            let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

            let output = drive(
                &pre(
                    "exec-b",
                    "Bash",
                    &[(SESSION_ID_FIELD, Value::String("session-2".to_string()))],
                ),
                &resolver,
                &recording_seam(Arc::clone(&recorded)),
            );
            assert_eq!(output, "");

            assert_eq!(
                operations(&recorded.lock().unwrap()),
                vec!["start".to_string()]
            );
            let final_state = read_state(&git_dir);
            assert_eq!(final_state.attempts.len(), 2);
            assert!(final_state
                .attempts
                .iter()
                .any(|attempt| attempt.tool_use_id == "exec-a"));
            assert!(final_state.recovery.is_clear());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn regression5_different_turn_is_not_swept_by_case_b_inference() {
            let git_dir = unique_test_git_dir("different-turn-not-swept");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt_in_turn(
                &git_dir,
                "session-1",
                "turn-1",
                None,
                "exec-a",
                state::AttemptPhase::Active,
            );
            let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

            let output = drive(
                &pre(
                    "exec-b",
                    "Bash",
                    &[(TURN_ID_FIELD, Value::String("turn-2".to_string()))],
                ),
                &resolver,
                &recording_seam(Arc::clone(&recorded)),
            );
            assert_eq!(output, "");

            assert_eq!(
                operations(&recorded.lock().unwrap()),
                vec!["start".to_string()]
            );
            let final_state = read_state(&git_dir);
            assert_eq!(final_state.attempts.len(), 2);
            assert!(final_state
                .attempts
                .iter()
                .any(|attempt| attempt.tool_use_id == "exec-a"));

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn regression6_duplicate_same_attempt_key_is_not_swept() {
            let git_dir = unique_test_git_dir("duplicate-not-swept");
            let resolver = fixed_resolver(git_dir.clone());
            let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

            drive(
                &pre("exec-a", "Bash", &[]),
                &resolver,
                &recording_seam(Arc::clone(&recorded)),
            );
            let scope_id = read_state(&git_dir).attempts[0].scope_id.clone();
            let next_seq = read_state(&git_dir).next_attempt_seq;

            drive(
                &pre("exec-a", "Bash", &[]),
                &resolver,
                &recording_seam(Arc::clone(&recorded)),
            );

            assert_eq!(
                operations(&recorded.lock().unwrap()),
                vec!["start".to_string()]
            );
            let final_state = read_state(&git_dir);
            assert_eq!(final_state.attempts.len(), 1);
            assert_eq!(final_state.attempts[0].scope_id, scope_id);
            assert_eq!(final_state.next_attempt_seq, next_seq);
            assert!(final_state.recovery.is_clear());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn regression7_pending_start_predecessor_is_swept() {
            let git_dir = unique_test_git_dir("pending-start-predecessor-swept");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt_in_turn(
                &git_dir,
                "session-1",
                "turn-1",
                None,
                "exec-a",
                state::AttemptPhase::PendingStart,
            );
            let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

            let output = drive(
                &pre("exec-b", "Bash", &[]),
                &resolver,
                &recording_seam(Arc::clone(&recorded)),
            );
            assert_eq!(output, "");

            assert_eq!(
                operations(&recorded.lock().unwrap()),
                vec![
                    "abandon".to_string(),
                    "flush".to_string(),
                    "start".to_string(),
                ],
            );
            let final_state = read_state(&git_dir);
            assert_eq!(final_state.attempts.len(), 1);
            assert_eq!(final_state.attempts[0].tool_use_id, "exec-b");
            assert!(final_state.recovery.is_clear());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn regression8_abandon_failure_during_sweep_is_fail_closed() {
            let git_dir = unique_test_git_dir("sweep-abandon-failure");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt_in_turn(
                &git_dir,
                "session-1",
                "turn-1",
                None,
                "exec-a",
                state::AttemptPhase::Active,
            );

            let output = run_codex_mutation_scope_from_payload_with(
                &pre("exec-b", "Bash", &[]),
                None,
                &resolver,
                &seam_failing_on("abandon"),
            )
            .expect("a failed sweep abandon still returns Ok with a deny payload");
            assert_eq!(output, pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON));

            let final_state = read_state(&git_dir);
            assert_eq!(final_state.attempts.len(), 1);
            assert_eq!(final_state.attempts[0].tool_use_id, "exec-a");
            assert!(!final_state.recovery.is_clear());

            remove_test_git_dir(&git_dir);
        }

        #[test]
        fn regression9_flush_failure_after_sweep_is_fail_closed() {
            let git_dir = unique_test_git_dir("sweep-flush-failure");
            let resolver = fixed_resolver(git_dir.clone());
            seed_attempt_in_turn(
                &git_dir,
                "session-1",
                "turn-1",
                None,
                "exec-a",
                state::AttemptPhase::Active,
            );

            let output = run_codex_mutation_scope_from_payload_with(
                &pre("exec-b", "Bash", &[]),
                None,
                &resolver,
                &seam_failing_on("flush"),
            )
            .expect("a failed post-sweep flush still returns Ok with a deny payload");
            assert_eq!(output, pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON));

            let final_state = read_state(&git_dir);
            assert!(final_state
                .attempts
                .iter()
                .all(|attempt| attempt.tool_use_id != "exec-b"));
            assert!(!final_state.recovery.is_clear());

            remove_test_git_dir(&git_dir);
        }
    }

    mod production_regressions {
        use std::fs;
        use std::path::{Path, PathBuf};
        use std::process::Command;

        use super::*;
        use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
        use crate::services::agent_trace_storage::{
            resolve_agent_trace_storage_at_state_root, AgentTraceStorageContext,
        };
        use crate::services::checkout::{get_or_create_checkout_id, resolve_git_dir};
        use crate::services::mutation_trace::store::decode_revision;

        const PROBE01_APPLY_PATCH_POST: &str = include_str!(
            "fixtures/probe01-apply-patch-and-shell-success.apply_patch.post_tool_use.json"
        );
        const PROBE02_FAILED_SHELL_PRE: &str = include_str!(
            "fixtures/probe02-shell-partial-write-then-nonzero-exit.pre_tool_use.json"
        );
        const PROBE06_APPLY_PATCH_FAILURE_PRE: &str = include_str!(
            "fixtures/probe06-apply-patch-verification-failure-no-post.pre_tool_use.json"
        );
        const PROBE09_DETACHED_PRE: &str =
            include_str!("fixtures/probe09-self-detaching-descendant.pre_tool_use.json");
        const PROBE09_DETACHED_POST: &str =
            include_str!("fixtures/probe09-self-detaching-descendant.post_tool_use.json");
        const PROBE11_INTERRUPT_PRE: &str =
            include_str!("fixtures/probe11-interrupt-event-on-sigint.pre_tool_use.json");
        const PROBE13_MCP_STOP: &str =
            include_str!("fixtures/probe13-mcp-mutate-then-error.stop.json");
        const PROBE14_MCP_FAILED_PRE: &str =
            include_str!("fixtures/probe14-mcp-failed-then-successor.failed.pre_tool_use.json");
        const PROBE14_MCP_SUCCESSOR_PRE: &str =
            include_str!("fixtures/probe14-mcp-failed-then-successor.successor.pre_tool_use.json");
        const PROBE14_MCP_SUCCESSOR_POST: &str =
            include_str!("fixtures/probe14-mcp-failed-then-successor.successor.post_tool_use.json");
        const PROBE16_MCP_PARALLEL_A_PRE: &str =
            include_str!("fixtures/probe16-mcp-parallel-server-optin.a.pre_tool_use.json");
        const PROBE16_MCP_PARALLEL_A_POST: &str =
            include_str!("fixtures/probe16-mcp-parallel-server-optin.a.post_tool_use.json");
        const PROBE16_MCP_PARALLEL_B_PRE: &str =
            include_str!("fixtures/probe16-mcp-parallel-server-optin.b.pre_tool_use.json");
        const PROBE16_MCP_PARALLEL_B_POST: &str =
            include_str!("fixtures/probe16-mcp-parallel-server-optin.b.post_tool_use.json");

        const OTHER_HARNESS_ACTOR_KIND: &str = "claude_code";

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

        struct CodexRepo {
            temp: tempfile::TempDir,
            root: PathBuf,
            state_root: PathBuf,
        }

        impl CodexRepo {
            fn new(label: &str) -> Self {
                let temp = tempfile::Builder::new()
                    .prefix(&format!("sce-codex-mutation-scope-regression-{label}-"))
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
                run_codex_mutation_scope_from_payload_at_state_root(&self.state_root, payload, None)
            }

            fn drive_generic(&self, payload: &str) -> Result<String> {
                self.drive_generic_at(&self.root, payload)
            }

            fn drive_generic_at(&self, repository_root: &Path, payload: &str) -> Result<String> {
                crate::services::hooks::mutation_scope::run_mutation_scope_from_payload_at_state_root(
                    repository_root,
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
                    "codex mutation-scope regression test assertions",
                )
                .expect("assertion DB should open")
            }

            fn cwd(&self) -> String {
                Self::cwd_at(&self.root)
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

            fn adapter_state_file_exists(&self) -> bool {
                state::adapter_state_dir(&self.git_dir())
                    .join("codex-mutation-scope-state.json")
                    .exists()
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

            fn write(&self, name: &str, contents: &str) {
                fs::write(self.root.join(name), contents).expect("regression write should succeed");
            }

            fn live_scope_id(&self) -> String {
                let state = self.adapter_state();
                assert_eq!(state.attempts.len(), 1, "exactly one live attempt expected");
                state.attempts[0].scope_id.clone()
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

        fn raw_agent_trace_row_counts(db: &RepositoryAgentTraceDb) -> [i64; 5] {
            [
                count(db, "diff_traces"),
                count(db, "post_commit_patch_intersections"),
                count(db, "agent_traces"),
                count(db, "messages"),
                count(db, "parts"),
            ]
        }

        fn assert_raw_agent_trace_tables_untouched(db: &RepositoryAgentTraceDb) {
            assert_eq!(
                raw_agent_trace_row_counts(db),
                [0, 0, 0, 0, 0],
                "AC20: the mutation-scope adapter must never write the raw Agent Trace tables"
            );
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

        fn active_scopes_for(db: &RepositoryAgentTraceDb, worktree_id: &str) -> Vec<String> {
            db.query_map(
                "SELECT scope_id FROM mutation_trace_event_active_scopes \
                 WHERE worktree_id = ?1 ORDER BY revision, scope_id",
                (worktree_id,),
                |row| row.get::<String>(0).map_err(anyhow::Error::from),
            )
            .expect("active-scopes query should succeed")
        }

        fn fixture_at(fixture: &str, cwd: &str) -> String {
            let mut object: Map<String, Value> =
                serde_json::from_str(fixture).expect("a fixture payload is a JSON object");
            object.insert(CWD_FIELD.to_string(), Value::String(cwd.to_string()));
            Value::Object(object).to_string()
        }

        struct ToolEvent<'a> {
            event_name: &'a str,
            cwd: &'a str,
            session_id: &'a str,
            turn_id: &'a str,
            tool_name: &'a str,
            tool_use_id: &'a str,
            agent_id: Option<&'a str>,
        }

        fn tool_event_json(event: &ToolEvent, tool_input: Option<Value>) -> String {
            let mut object = Map::new();
            object.insert(
                HOOK_EVENT_NAME_FIELD.to_string(),
                Value::String(event.event_name.to_string()),
            );
            object.insert(
                SESSION_ID_FIELD.to_string(),
                Value::String(event.session_id.to_string()),
            );
            object.insert(
                TURN_ID_FIELD.to_string(),
                Value::String(event.turn_id.to_string()),
            );
            object.insert(CWD_FIELD.to_string(), Value::String(event.cwd.to_string()));
            object.insert(
                TOOL_NAME_FIELD.to_string(),
                Value::String(event.tool_name.to_string()),
            );
            object.insert(
                TOOL_USE_ID_FIELD.to_string(),
                Value::String(event.tool_use_id.to_string()),
            );
            if let Some(agent_id) = event.agent_id {
                object.insert(
                    AGENT_ID_FIELD.to_string(),
                    Value::String(agent_id.to_string()),
                );
            }
            if let Some(tool_input) = tool_input {
                object.insert(TOOL_INPUT_FIELD.to_string(), tool_input);
            }
            Value::Object(object).to_string()
        }

        struct TrackedCall<'a> {
            cwd: &'a str,
            session_id: &'a str,
            turn_id: &'a str,
            tool_name: &'a str,
            tool_use_id: &'a str,
            agent_id: Option<&'a str>,
        }

        impl TrackedCall<'_> {
            fn pre(&self) -> String {
                tool_event_json(
                    &ToolEvent {
                        event_name: HOOK_EVENT_PRE_TOOL_USE,
                        cwd: self.cwd,
                        session_id: self.session_id,
                        turn_id: self.turn_id,
                        tool_name: self.tool_name,
                        tool_use_id: self.tool_use_id,
                        agent_id: self.agent_id,
                    },
                    Some(json!({ "command": "echo regression >> file.txt" })),
                )
            }

            fn post(&self) -> String {
                tool_event_json(
                    &ToolEvent {
                        event_name: HOOK_EVENT_POST_TOOL_USE,
                        cwd: self.cwd,
                        session_id: self.session_id,
                        turn_id: self.turn_id,
                        tool_name: self.tool_name,
                        tool_use_id: self.tool_use_id,
                        agent_id: self.agent_id,
                    },
                    None,
                )
            }
        }

        fn bash_call<'a>(
            cwd: &'a str,
            session_id: &'a str,
            tool_use_id: &'a str,
        ) -> TrackedCall<'a> {
            TrackedCall {
                cwd,
                session_id,
                turn_id: "turn-1",
                tool_name: CODEX_TRACKED_TOOL_BASH,
                tool_use_id,
                agent_id: None,
            }
        }

        fn turn_event_json(event_name: &str, cwd: &str, session_id: &str, turn_id: &str) -> String {
            json!({
                HOOK_EVENT_NAME_FIELD: event_name,
                SESSION_ID_FIELD: session_id,
                TURN_ID_FIELD: turn_id,
                CWD_FIELD: cwd,
            })
            .to_string()
        }

        fn session_end_json(cwd: &str, session_id: &str) -> String {
            json!({
                HOOK_EVENT_NAME_FIELD: HOOK_EVENT_SESSION_END,
                SESSION_ID_FIELD: session_id,
                CWD_FIELD: cwd,
            })
            .to_string()
        }

        fn other_harness_payload(operation: &str, scope_id: &str) -> String {
            json!({
                "operation": operation,
                "scope_id": scope_id,
                "event_id": format!("{scope_id}|{operation}"),
                "actor_kind": OTHER_HARNESS_ACTOR_KIND,
            })
            .to_string()
        }

        #[test]
        fn test1_tracked_bash_success_closes_ai_exclusive_ac8() {
            let repo = CodexRepo::new("test1-bash-success");
            let cwd = repo.cwd();
            let pre = fixture_at(PROBE01_SHELL_PRE, &cwd);
            let post = fixture_at(PROBE01_SHELL_POST, &cwd);

            assert_eq!(
                repo.drive(&pre).expect("PreToolUse should succeed"),
                "",
                "a tracked PreToolUse that established Start returns the neutral response"
            );
            let scope_id = repo.live_scope_id();

            repo.write("file.txt", "one\ntwo\n");

            assert_eq!(repo.drive(&post).expect("PostToolUse should succeed"), "");
            assert!(
                repo.adapter_state().attempts.is_empty(),
                "the closed attempt must be removed from adapter bookkeeping"
            );

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &scope_id),
                Some(("codex".to_string(), "closed".to_string()))
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
                    (scope_id.clone(), codex_scope_close_event_id(&scope_id)),
                    (scope_id.clone(), codex_scope_start_event_id(&scope_id)),
                ],
                "rows are ordered by (scope_id, event_id), and 'close' sorts before 'start'"
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test2_failed_bash_partial_write_still_closes_ai_exclusive_ac9() {
            let repo = CodexRepo::new("test2-failed-bash");
            let cwd = repo.cwd();

            repo.drive(&fixture_at(PROBE02_FAILED_SHELL_PRE, &cwd))
                .expect("PreToolUse should succeed");
            let scope_id = repo.live_scope_id();

            repo.write("file.txt", "one\npartial\n");

            assert_eq!(
                repo.drive(&fixture_at(PROBE02_FAILED_SHELL_POST, &cwd))
                    .expect("a non-zero-exit shell still fires PostToolUse"),
                ""
            );
            assert!(repo.adapter_state().attempts.is_empty());

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &scope_id),
                Some(("codex".to_string(), "closed".to_string())),
                "D10: a Bash tool that partially mutated then exited non-zero closes its scope"
            );
            let worktree_id = repo.worktree_id();
            assert_eq!(
                mutation_events_for(&db, &worktree_id),
                vec![(
                    "ai_exclusive".to_string(),
                    Some(scope_id),
                    "close".to_string(),
                )],
                "the partial mutation is attributed to the failed tool's own scope"
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test3_apply_patch_success_closes_ai_exclusive_ac8() {
            let repo = CodexRepo::new("test3-apply-patch-success");
            let cwd = repo.cwd();

            repo.drive(&fixture_at(PROBE01_APPLY_PATCH_PRE, &cwd))
                .expect("PreToolUse should succeed");
            let scope_id = repo.live_scope_id();

            repo.write("alpha.txt", "alpha one\n");

            repo.drive(&fixture_at(PROBE01_APPLY_PATCH_POST, &cwd))
                .expect("PostToolUse should succeed");

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &scope_id),
                Some(("codex".to_string(), "closed".to_string()))
            );
            let worktree_id = repo.worktree_id();
            assert_eq!(
                mutation_events_for(&db, &worktree_id),
                vec![(
                    "ai_exclusive".to_string(),
                    Some(scope_id),
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
        fn test4_apply_patch_verification_failure_mutates_nothing_and_is_swept_ac9() {
            let repo = CodexRepo::new("test4-apply-patch-failure");
            let cwd = repo.cwd();
            let pre = fixture_at(PROBE06_APPLY_PATCH_FAILURE_PRE, &cwd);
            let tree_before = repo.working_tree();

            repo.drive(&pre).expect("PreToolUse should succeed");
            let scope_id = repo.live_scope_id();

            let stop = {
                let execution = pre_tool_use(&pre);
                turn_event_json(
                    HOOK_EVENT_STOP,
                    &cwd,
                    &execution.identity.session_id,
                    &execution.identity.turn_id,
                )
            };
            repo.drive(&stop)
                .expect("Stop should retire the attempt that never received PostToolUse");

            assert!(repo.adapter_state().attempts.is_empty());
            assert!(!repo.adapter_state().recovery.is_clear());
            assert_eq!(
                repo.working_tree(),
                tree_before,
                "D10: apply_patch verification failure never touches the working tree"
            );

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &scope_id),
                Some(("codex".to_string(), "abandoned".to_string()))
            );
            let worktree_id = repo.worktree_id();
            assert_eq!(
                mutation_events_for(&db, &worktree_id),
                vec![],
                "no mutation happened, so nothing is attributed"
            );
            assert!(worktree_row(&db, &worktree_id)
                .is_some_and(|(_, _, needs_rebaseline)| needs_rebaseline));

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test5_duplicate_tracked_lifecycle_is_idempotent_ac4() {
            let repo = CodexRepo::new("test5-duplicate-lifecycle");
            let cwd = repo.cwd();
            let pre = fixture_at(PROBE01_SHELL_PRE, &cwd);
            let post = fixture_at(PROBE01_SHELL_POST, &cwd);

            repo.drive(&pre).expect("first PreToolUse should succeed");
            let scope_id = repo.live_scope_id();
            assert_eq!(
                repo.drive(&pre)
                    .expect("duplicate PreToolUse should be idempotent"),
                ""
            );
            assert_eq!(
                repo.live_scope_id(),
                scope_id,
                "AC4: duplicate delivery of a live PreToolUse reuses the same ScopeId"
            );

            repo.write("file.txt", "one\ntwo\n");
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
                        && event == &codex_scope_close_event_id(&scope_id))
                    .count(),
                1
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test6_interrupted_tracked_execution_is_retired_by_interrupt_ac11() {
            let repo = CodexRepo::new("test6-interrupt");
            let cwd = repo.cwd();

            repo.drive(&fixture_at(PROBE11_INTERRUPT_PRE, &cwd))
                .expect("PreToolUse should succeed");
            let scope_id = repo.live_scope_id();

            repo.write("file.txt", "one\ninterrupted\n");

            assert_eq!(
                repo.drive(&fixture_at(PROBE11_INTERRUPT, &cwd))
                    .expect("Interrupt should succeed"),
                ""
            );

            assert!(
                repo.adapter_state().attempts.is_empty(),
                "AC11: Interrupt is a proven cleanup signal for the interrupted turn"
            );

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &scope_id),
                Some(("codex".to_string(), "abandoned".to_string()))
            );
            let worktree_id = repo.worktree_id();
            assert!(worktree_row(&db, &worktree_id)
                .is_some_and(|(_, _, needs_rebaseline)| needs_rebaseline));
            assert_eq!(mutation_events_for(&db, &worktree_id), vec![]);

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test6b_session_end_is_the_load_bearing_backstop_ac11() {
            let repo = CodexRepo::new("test6b-session-end");
            let cwd = repo.cwd();

            repo.drive(&fixture_at(PROBE01_SHELL_PRE, &cwd))
                .expect("PreToolUse should succeed");
            let scope_id = repo.live_scope_id();

            repo.write("file.txt", "one\nstranded\n");

            assert_eq!(
                repo.drive(&fixture_at(PROBE01_SESSION_END, &cwd))
                    .expect("SessionEnd should succeed"),
                ""
            );

            assert!(
                repo.adapter_state().attempts.is_empty(),
                "D12: SessionEnd is the load-bearing whole-session backstop"
            );

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &scope_id),
                Some(("codex".to_string(), "abandoned".to_string()))
            );
            assert!(worktree_row(&db, &repo.worktree_id())
                .is_some_and(|(_, _, needs_rebaseline)| needs_rebaseline));

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test7_subagent_tracked_tool_gets_its_own_scope_identity() {
            let repo = CodexRepo::new("test7-subagent");
            let cwd = repo.cwd();
            let subagent_pre = fixture_at(PROBE08_AGENT_APPLY_PATCH_PRE, &cwd);
            let subagent_identity = pre_tool_use(&subagent_pre).identity;
            let agent_id = subagent_identity
                .agent_id
                .clone()
                .expect("probe08 carries a delegated-agent identity");
            let main_thread = TrackedCall {
                cwd: &cwd,
                session_id: &subagent_identity.session_id,
                turn_id: "main-turn",
                tool_name: CODEX_TRACKED_TOOL_BASH,
                tool_use_id: "exec-main-thread",
                agent_id: None,
            };

            repo.drive(&main_thread.pre())
                .expect("main-thread PreToolUse should succeed");
            repo.drive(&subagent_pre)
                .expect("subagent PreToolUse should succeed");

            let state = repo.adapter_state();
            assert_eq!(state.attempts.len(), 2);
            let subagent_scope_id = state
                .attempts
                .iter()
                .find(|attempt| attempt.agent_id.as_deref() == Some(agent_id.as_str()))
                .map(|attempt| attempt.scope_id.clone())
                .expect("the subagent attempt carries its agent_id");
            let main_scope_id = state
                .attempts
                .iter()
                .find(|attempt| attempt.agent_id.is_none())
                .map(|attempt| attempt.scope_id.clone())
                .expect("the main-thread attempt has no agent_id");
            assert_ne!(subagent_scope_id, main_scope_id);
            assert!(subagent_scope_id.contains(&agent_id));

            repo.drive(&fixture_at(PROBE08_SUBAGENT_STOP, &cwd))
                .expect("SubagentStop should succeed");

            let remaining = repo.adapter_state();
            assert_eq!(
                remaining
                    .attempts
                    .iter()
                    .map(|attempt| attempt.scope_id.clone())
                    .collect::<Vec<_>>(),
                vec![main_scope_id.clone()],
                "D12: SubagentStop sweeps only the ending agent's attempts"
            );

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &subagent_scope_id).map(|(_, status)| status),
                Some("abandoned".to_string())
            );
            assert_eq!(
                scope_status(&db, &main_scope_id).map(|(_, status)| status),
                Some("active".to_string())
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test8_linked_worktree_advances_only_its_own_cursor_ac14() {
            let repo = CodexRepo::new("test8-linked-worktree");
            let worktree_path = repo.add_worktree("codex-worktree");
            let worktree_cwd = CodexRepo::cwd_at(&worktree_path);

            let main_worktree_id = repo.worktree_id();
            let linked_worktree_id = CodexRepo::worktree_id_at(&worktree_path);
            assert_ne!(main_worktree_id, linked_worktree_id);

            repo.drive_flush()
                .expect("main-checkout baseline flush should succeed");
            let main_cursor_before = worktree_row(&repo.db(), &main_worktree_id)
                .map(|(_, cursor_tree, _)| cursor_tree)
                .expect("main checkout should have a baseline worktree row");

            let pre = fixture_at(PROBE10_WORKTREE_PRE, &worktree_cwd);
            let post = {
                let identity = pre_tool_use(&pre).identity;
                tool_event_json(
                    &ToolEvent {
                        event_name: HOOK_EVENT_POST_TOOL_USE,
                        cwd: &worktree_cwd,
                        session_id: &identity.session_id,
                        turn_id: &identity.turn_id,
                        tool_name: &identity.tool_name,
                        tool_use_id: &identity.tool_use_id,
                        agent_id: None,
                    },
                    None,
                )
            };

            repo.drive(&pre)
                .expect("linked-worktree PreToolUse should succeed");
            let scope_id = CodexRepo::adapter_state_at(&worktree_path).attempts[0]
                .scope_id
                .clone();
            fs::write(worktree_path.join("wt.txt"), "worktree-write\n")
                .expect("the linked worktree's own write should succeed");
            repo.drive(&post)
                .expect("linked-worktree PostToolUse should succeed");

            let db = repo.db();
            assert_eq!(
                worktree_row(&db, &main_worktree_id).map(|(_, cursor_tree, _)| cursor_tree),
                Some(main_cursor_before),
                "AC14: the main checkout's cursor must not move"
            );
            let linked_row =
                worktree_row(&db, &linked_worktree_id).expect("linked worktree row should exist");
            assert_eq!(
                linked_row.1,
                CodexRepo::working_tree_at(&worktree_path),
                "AC14: the linked worktree's own cursor advances"
            );
            assert_eq!(
                mutation_events_for(&db, &linked_worktree_id),
                vec![(
                    "ai_exclusive".to_string(),
                    Some(scope_id),
                    "close".to_string(),
                )]
            );
            assert_eq!(mutation_events_for(&db, &main_worktree_id), vec![]);

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test9_successful_mcp_lifecycle_creates_no_mutation_scope_ac9b() {
            let repo = CodexRepo::new("test9-mcp-success");
            let cwd = repo.cwd();

            assert_eq!(
                repo.drive(&fixture_at(PROBE12_MCP_PRE, &cwd))
                    .expect("an MCP PreToolUse is allowed"),
                "",
                "AC9b: an Untracked tool gets the Codex-neutral continue response"
            );
            repo.write("mcp_a.txt", "written by the MCP server\n");
            assert_eq!(
                repo.drive(&fixture_at(PROBE12_MCP_POST, &cwd))
                    .expect("an MCP PostToolUse is ignored"),
                ""
            );

            assert!(
                !repo.adapter_state_file_exists(),
                "AC9b: an Untracked lifecycle writes no adapter bookkeeping at all"
            );

            let db = repo.db();
            assert_eq!(count(&db, "mutation_trace_scopes"), 0);
            assert_eq!(count(&db, "mutation_trace_events"), 0);
            assert_eq!(count(&db, "mutation_trace_processed_events"), 0);
            assert_eq!(count(&db, "mutation_trace_worktrees"), 0);

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test10_mcp_mutate_then_error_leaves_no_zombie_state_ac9c() {
            let repo = CodexRepo::new("test10-mcp-mutate-then-error");
            let cwd = repo.cwd();

            repo.drive(&fixture_at(PROBE13_MCP_MUTATE_THEN_ERROR_PRE, &cwd))
                .expect("an MCP PreToolUse is allowed");
            repo.write("mcp_b.txt", "mutated before the MCP error\n");

            repo.drive(&fixture_at(PROBE13_MCP_STOP, &cwd))
                .expect("Stop should find nothing to retire");
            repo.drive(&fixture_at(PROBE13_MCP_SESSION_END, &cwd))
                .expect("SessionEnd should find nothing to retire");

            assert!(
                !repo.adapter_state_file_exists(),
                "AC9c: no Start occurred, so there is no stale attempt and no recovery to arm"
            );

            let db = repo.db();
            assert_eq!(count(&db, "mutation_trace_scopes"), 0);
            assert_eq!(count(&db, "mutation_trace_events"), 0);

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test11_failed_mcp_then_tracked_successor_starts_clean_ac9d() {
            let repo = CodexRepo::new("test11-mcp-then-tracked");
            let cwd = repo.cwd();
            let failed_mcp = fixture_at(PROBE14_MCP_FAILED_PRE, &cwd);
            let failed_identity = pre_tool_use(&failed_mcp).identity;

            repo.drive(&failed_mcp)
                .expect("the failing MCP PreToolUse is allowed");
            repo.write("mcp_c1.txt", "mutated by the failing MCP tool\n");
            repo.drive(&fixture_at(PROBE14_MCP_SUCCESSOR_PRE, &cwd))
                .expect("the MCP successor is also Untracked");
            repo.drive(&fixture_at(PROBE14_MCP_SUCCESSOR_POST, &cwd))
                .expect("the MCP successor's PostToolUse is ignored");

            let tracked_successor = TrackedCall {
                cwd: &cwd,
                session_id: &failed_identity.session_id,
                turn_id: &failed_identity.turn_id,
                tool_name: CODEX_TRACKED_TOOL_BASH,
                tool_use_id: "exec-tracked-successor",
                agent_id: None,
            };
            repo.drive(&tracked_successor.pre())
                .expect("the tracked successor should Start normally");
            let scope_id = repo.live_scope_id();
            assert!(
                repo.adapter_state().recovery.is_clear(),
                "AC9d: no MCP attempt existed, so no successor barrier runs"
            );

            repo.write("file.txt", "one\ntracked-successor\n");
            repo.drive(&tracked_successor.post())
                .expect("the tracked successor's PostToolUse should close its scope");

            let db = repo.db();
            let worktree_id = repo.worktree_id();
            assert_eq!(count(&db, "mutation_trace_scopes"), 1);
            assert_eq!(
                mutation_events_for(&db, &worktree_id),
                vec![(
                    "ai_exclusive".to_string(),
                    Some(scope_id),
                    "close".to_string(),
                )],
                "AC9d: the tracked successor is the only live scope — no false AiContended"
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test12_parallel_mcp_executions_create_no_scopes_ac9e() {
            let repo = CodexRepo::new("test12-parallel-mcp");
            let cwd = repo.cwd();

            repo.drive(&fixture_at(PROBE16_MCP_PARALLEL_A_PRE, &cwd))
                .expect("parallel MCP A is allowed");
            repo.drive(&fixture_at(PROBE16_MCP_PARALLEL_B_PRE, &cwd))
                .expect("parallel MCP B is allowed");
            assert!(
                !repo.adapter_state_file_exists(),
                "AC9e: neither overlapping MCP execution creates adapter state"
            );

            repo.write("mcp_parallel_a.txt", "a\n");
            repo.write("mcp_parallel_b.txt", "b\n");

            repo.drive(&fixture_at(PROBE16_MCP_PARALLEL_A_POST, &cwd))
                .expect("parallel MCP A PostToolUse is ignored");
            repo.drive(&fixture_at(PROBE16_MCP_PARALLEL_B_POST, &cwd))
                .expect("parallel MCP B PostToolUse is ignored");

            assert!(!repo.adapter_state_file_exists());

            let db = repo.db();
            assert_eq!(
                count(&db, "mutation_trace_scopes"),
                0,
                "AC9e: overlapping MCP executions produce no scopes and therefore no AiContended"
            );
            assert_eq!(count(&db, "mutation_trace_events"), 0);

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test13_tracked_scope_overlapping_an_mcp_mutation_is_tracked_exclusivity_ac9f() {
            let repo = CodexRepo::new("test13-tracked-plus-mcp");
            let cwd = repo.cwd();

            repo.drive(&fixture_at(PROBE01_SHELL_PRE, &cwd))
                .expect("the tracked Bash PreToolUse should Start");
            let scope_id = repo.live_scope_id();

            repo.drive(&fixture_at(PROBE12_MCP_PRE, &cwd))
                .expect("the overlapping MCP call is allowed and untracked");
            repo.write("mcp_a.txt", "written by the MCP server, not by Bash\n");
            repo.drive(&fixture_at(PROBE12_MCP_POST, &cwd))
                .expect("the MCP PostToolUse is ignored");

            repo.drive(&fixture_at(PROBE01_SHELL_POST, &cwd))
                .expect("the tracked Bash PostToolUse should close its scope");

            let db = repo.db();
            let worktree_id = repo.worktree_id();
            assert_eq!(count(&db, "mutation_trace_scopes"), 1);
            assert_eq!(
                mutation_events_for(&db, &worktree_id),
                vec![(
                    "ai_exclusive".to_string(),
                    Some(scope_id),
                    "close".to_string(),
                )],
                "AC9f: ai_exclusive means exactly one TRACKED scope was live in the interval, \
                 not that the tracked scope authored every mutation — the MCP call did mutate \
                 mcp_a.txt inside this interval and remains unattributed (D14/D23)"
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test14_unknown_tool_is_allowed_untracked_ac9b() {
            let repo = CodexRepo::new("test14-unknown-tool");
            let cwd = repo.cwd();
            let unknown = TrackedCall {
                cwd: &cwd,
                session_id: "session-unknown",
                turn_id: "turn-1",
                tool_name: "some_future_codex_tool",
                tool_use_id: "exec-unknown",
                agent_id: None,
            };

            assert_eq!(
                repo.drive(&unknown.pre())
                    .expect("an unknown tool is never denied for being untracked"),
                ""
            );
            repo.write("unknown_tool_output.txt", "the unknown tool mutated\n");
            assert_eq!(
                repo.drive(&unknown.post())
                    .expect("an unknown tool's PostToolUse is ignored"),
                ""
            );

            assert!(!repo.adapter_state_file_exists());

            let db = repo.db();
            assert_eq!(count(&db, "mutation_trace_scopes"), 0);
            assert_eq!(count(&db, "mutation_trace_events"), 0);

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test15_regression_matrix_leaves_raw_agent_trace_tables_untouched_ac20() {
            let repo = CodexRepo::new("test15-raw-tables");
            let cwd = repo.cwd();

            let before = raw_agent_trace_row_counts(&repo.db());
            assert_eq!(before, [0, 0, 0, 0, 0]);

            repo.drive(&fixture_at(PROBE01_SHELL_PRE, &cwd))
                .expect("tracked PreToolUse should succeed");
            repo.write("file.txt", "one\ntwo\n");
            repo.drive(&fixture_at(PROBE01_SHELL_POST, &cwd))
                .expect("tracked PostToolUse should succeed");
            repo.drive(&fixture_at(PROBE12_MCP_PRE, &cwd))
                .expect("MCP PreToolUse should succeed");
            repo.write("mcp_a.txt", "mcp\n");
            repo.drive(&fixture_at(PROBE12_MCP_POST, &cwd))
                .expect("MCP PostToolUse should succeed");
            repo.drive(&fixture_at(PROBE01_APPLY_PATCH_PRE, &cwd))
                .expect("apply_patch PreToolUse should succeed");
            repo.drive(&fixture_at(PROBE01_STOP, &cwd))
                .expect("Stop should succeed");

            let db = repo.db();
            assert_eq!(
                raw_agent_trace_row_counts(&db),
                before,
                "AC20: mutation-scope-only regressions leave diff_traces, \
                 post_commit_patch_intersections, agent_traces, messages and parts unchanged"
            );
            assert!(count(&db, "mutation_trace_scopes") > 0);
            assert!(
                state::adapter_state_dir(&repo.git_dir()).starts_with(repo.git_dir()),
                "AC20: adapter state lives only below <git-dir>/sce/"
            );
        }

        #[test]
        fn test16_arbitrary_blocker_zombie_then_same_lane_successor_ac9a() {
            let repo = CodexRepo::new("test16-zombie-successor");
            let cwd = repo.cwd();
            let zombie = bash_call(&cwd, "session-lane", "exec-zombie");
            let successor = bash_call(&cwd, "session-lane", "exec-successor");

            repo.drive(&zombie.pre())
                .expect("the first tracked PreToolUse should Start");
            let zombie_scope_id = repo.live_scope_id();
            repo.write("file.txt", "one\nzombie-partial\n");

            repo.drive(&successor.pre())
                .expect("the same-lane successor should sweep, flush, then Start");
            let successor_scope_id = repo.live_scope_id();
            assert_ne!(zombie_scope_id, successor_scope_id);
            assert!(
                repo.adapter_state().recovery.is_clear(),
                "the quiescent flush must clear the barrier before Start(B)"
            );

            repo.write("file.txt", "one\nzombie-partial\nsuccessor\n");
            repo.drive(&successor.post())
                .expect("the successor's PostToolUse should close its scope");

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &zombie_scope_id),
                Some(("codex".to_string(), "abandoned".to_string())),
                "AC9a: the stale same-lane predecessor is abandoned, never closed"
            );
            assert_eq!(
                scope_status(&db, &successor_scope_id),
                Some(("codex".to_string(), "closed".to_string()))
            );
            let worktree_id = repo.worktree_id();
            let events = mutation_events_for(&db, &worktree_id);
            assert!(
                events.iter().all(|(kind, scope, _)| kind != "ai_contended"
                    && scope.as_deref() != Some(zombie_scope_id.as_str())),
                "AC9a: no false AiContended and nothing attributed to the zombie: {events:?}"
            );
            assert_eq!(
                events.last(),
                Some(&(
                    "ai_exclusive".to_string(),
                    Some(successor_scope_id),
                    "close".to_string()
                )),
                "the successor is the only live scope at its own Close"
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test17_crash_before_start_commit_is_recovered_conservatively_ac21a() {
            let repo = CodexRepo::new("test17-crash-before-start");
            let cwd = repo.cwd();
            let git_dir = repo.git_dir();
            let crashed = bash_call(&cwd, "session-crash", "exec-crashed");
            let key = key("session-crash", None, "exec-crashed");

            let attempt = state::seed_attempt_for_tests(
                &git_dir,
                &key,
                "turn-1",
                CODEX_TRACKED_TOOL_BASH,
                state::AttemptPhase::PendingStart,
            );

            repo.drive(&crashed.post())
                .expect("D11: a pending_start attempt must abandon, not late-Start");

            assert!(repo.adapter_state().attempts.is_empty());
            assert!(!repo.adapter_state().recovery.is_clear());

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &attempt.scope_id),
                None,
                "AC21a: a Start that never committed must never appear as a real scope"
            );
            assert_eq!(count(&db, "mutation_trace_events"), 0);

            let fresh = bash_call(&cwd, "session-crash", "exec-fresh");
            repo.drive(&fresh.pre())
                .expect("the next tracked PreToolUse proceeds after the quiescent flush");
            assert!(repo.adapter_state().recovery.is_clear());
            assert_eq!(repo.adapter_state().attempts.len(), 1);

            assert_raw_agent_trace_tables_untouched(&repo.db());
        }

        #[test]
        fn test18_start_committed_before_state_settlement_is_abandoned_ac21b() {
            let repo = CodexRepo::new("test18-crash-after-start");
            let cwd = repo.cwd();
            let git_dir = repo.git_dir();
            let crashed = bash_call(&cwd, "session-crash", "exec-crashed");
            let key = key("session-crash", None, "exec-crashed");

            let attempt = state::seed_attempt_for_tests(
                &git_dir,
                &key,
                "turn-1",
                CODEX_TRACKED_TOOL_BASH,
                state::AttemptPhase::PendingStart,
            );
            let scope_id = attempt.scope_id.clone();

            repo.drive_generic(&scope_boundary_payload(
                "start",
                &scope_id,
                &codex_scope_start_event_id(&scope_id),
            ))
            .expect("the runtime Start should commit durably");
            assert_eq!(
                repo.adapter_state().attempts[0].phase,
                state::AttemptPhase::PendingStart
            );

            repo.drive(&crashed.post())
                .expect("D11: a committed Start with unsettled bookkeeping must be abandoned");

            assert!(repo.adapter_state().attempts.is_empty());
            assert!(!repo.adapter_state().recovery.is_clear());

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &scope_id),
                Some(("codex".to_string(), "abandoned".to_string())),
                "AC21b: the committed Start settles as a real abandonment, not a late Start"
            );
            assert!(worktree_row(&db, &repo.worktree_id())
                .is_some_and(|(_, _, needs_rebaseline)| needs_rebaseline));

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test19_close_committed_before_state_cleanup_is_replay_safe_ac21c() {
            let repo = CodexRepo::new("test19-crash-after-close");
            let cwd = repo.cwd();
            let call = bash_call(&cwd, "session-close", "exec-close");

            repo.drive(&call.pre()).expect("PreToolUse should succeed");
            let scope_id = repo.live_scope_id();
            repo.write("file.txt", "one\ntwo\n");

            repo.drive_generic(&scope_boundary_payload(
                "close",
                &scope_id,
                &codex_scope_close_event_id(&scope_id),
            ))
            .expect("the runtime Close should commit durably");
            assert_eq!(repo.adapter_state().attempts.len(), 1);

            let db = repo.db();
            let (revision_before, events_before) = (
                worktree_row(&db, &repo.worktree_id())
                    .map(|(revision, _, _)| revision)
                    .expect("a worktree row should exist"),
                count(&db, "mutation_trace_events"),
            );

            repo.drive(&call.post())
                .expect("a replayed Close against an already-durable commit must be safe");

            assert!(
                repo.adapter_state().attempts.is_empty(),
                "the stale bookkeeping is finally cleared"
            );

            let db = repo.db();
            assert_eq!(
                worktree_row(&db, &repo.worktree_id()).map(|(revision, _, _)| revision),
                Some(revision_before),
                "AC21c: a durably completed Close is never re-applied as a second transition"
            );
            assert_eq!(count(&db, "mutation_trace_events"), events_before);
            assert_eq!(
                scope_status(&db, &scope_id).map(|(_, status)| status),
                Some("closed".to_string())
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test20_recovery_pending_blocks_a_tracked_successor_until_recovery_succeeds_ac12() {
            let repo = CodexRepo::new("test20-recovery-barrier");
            let cwd = repo.cwd();
            let first = bash_call(&cwd, "session-a", "exec-a");
            let second = bash_call(&cwd, "session-b", "exec-b");
            let blocked = bash_call(&cwd, "session-c", "exec-c");

            repo.drive(&first.pre()).expect("session-a Start");
            repo.drive(&second.pre()).expect("session-b Start");
            assert_eq!(repo.adapter_state().attempts.len(), 2);

            repo.write("file.txt", "one\nabandoned\n");
            repo.drive(&turn_event_json(
                HOOK_EVENT_INTERRUPT,
                &cwd,
                "session-a",
                "turn-1",
            ))
            .expect("Interrupt should retire session-a's attempt");
            assert!(!repo.adapter_state().recovery.is_clear());
            assert_eq!(repo.adapter_state().attempts.len(), 1);

            assert_eq!(
                repo.drive(&blocked.pre())
                    .expect("a barred PreToolUse still returns Ok with a deny payload"),
                pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON),
                "AC12: while recovery is armed and attempts remain, a tracked successor is denied"
            );
            assert!(
                repo.adapter_state()
                    .attempts
                    .iter()
                    .all(|attempt| attempt.tool_use_id != "exec-c"),
                "the denied successor must never be admitted"
            );

            repo.drive(&session_end_json(&cwd, "session-b"))
                .expect("SessionEnd should retire session-b's attempt");
            assert!(repo.adapter_state().attempts.is_empty());
            assert!(!repo.adapter_state().recovery.is_clear());

            repo.drive(&blocked.pre())
                .expect("once quiescent, the flush runs and the successor Starts");
            assert!(
                repo.adapter_state().recovery.is_clear(),
                "AC12: recovery_pending clears only on durable flush success"
            );
            let scope_id = repo.live_scope_id();

            let db = repo.db();
            assert_eq!(
                scope_status(&db, &scope_id).map(|(_, status)| status),
                Some("active".to_string())
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test21_reused_tool_use_id_after_terminal_gets_a_fresh_scope_id_ac5() {
            let repo = CodexRepo::new("test21-reused-identifier");
            let cwd = repo.cwd();
            let call = bash_call(&cwd, "session-reuse", "exec-reused");

            repo.drive(&call.pre()).expect("first PreToolUse");
            let first_scope_id = repo.live_scope_id();
            repo.write("file.txt", "one\nfirst\n");
            repo.drive(&call.post()).expect("first PostToolUse");
            assert!(repo.adapter_state().attempts.is_empty());

            repo.drive(&call.pre())
                .expect("a later attempt reusing the same tool_use_id");
            let second_scope_id = repo.live_scope_id();
            assert_ne!(
                first_scope_id, second_scope_id,
                "AC5: a terminal ScopeId is never reused"
            );

            repo.write("file.txt", "one\nfirst\nsecond\n");
            repo.drive(&call.post()).expect("second PostToolUse");

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
        fn test22_self_detaching_descendant_write_is_not_folded_into_the_closed_scope_ac15() {
            let repo = CodexRepo::new("test22-detached-descendant");
            let cwd = repo.cwd();

            repo.drive(&fixture_at(PROBE09_DETACHED_PRE, &cwd))
                .expect("PreToolUse should succeed");
            let scope_id = repo.live_scope_id();

            repo.write("file.txt", "one\nforeground\n");
            let tree_at_close = repo.working_tree();
            repo.drive(&fixture_at(PROBE09_DETACHED_POST, &cwd))
                .expect("PostToolUse should succeed");

            let db = repo.db();
            let worktree_id = repo.worktree_id();
            assert_eq!(
                worktree_row(&db, &worktree_id).map(|(_, cursor_tree, _)| cursor_tree),
                Some(tree_at_close.clone())
            );
            let events_before_flush = mutation_events_for(&db, &worktree_id);

            repo.write("file.txt", "one\nforeground\ndetached-descendant\n");
            let tree_after_descendant = repo.working_tree();
            assert_ne!(tree_after_descendant, tree_at_close);

            repo.drive_flush()
                .expect("a later diagnostic flush should succeed");

            let db = repo.db();
            let events_after_flush = mutation_events_for(&db, &worktree_id);
            assert_eq!(events_after_flush.len(), events_before_flush.len() + 1);
            let (attribution_kind, attribution_scope_id, _) = events_after_flush
                .last()
                .expect("a flush event should exist");
            assert_eq!(
                attribution_kind, "ineligible_unscoped",
                "AC15/D16: SCE does not supervise self-detaching descendants; \
                 a post-terminal write is never folded into the closed tool scope"
            );
            assert_ne!(attribution_scope_id.as_deref(), Some(scope_id.as_str()));

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test23_denied_tracked_execution_leaves_no_untracked_start_ac7() {
            let repo = CodexRepo::new("test23-policy-denied");
            let cwd = repo.cwd();
            fs::create_dir_all(repo.root.join(".sce")).expect(".sce dir should be created");
            fs::write(
                repo.root.join(".sce").join("config.json"),
                concat!(
                    r#"{"policies":{"bash":{"custom":[{"id":"no-rm","#,
                    r#""match":{"argv_prefix":["rm"]},"#,
                    r#""message":"rm is blocked in this repository"}]}}}"#,
                ),
            )
            .expect("repo bash policy config should write");

            let denied = tool_event_json(
                &ToolEvent {
                    event_name: HOOK_EVENT_PRE_TOOL_USE,
                    cwd: &cwd,
                    session_id: "session-denied",
                    turn_id: "turn-1",
                    tool_name: CODEX_TRACKED_TOOL_BASH,
                    tool_use_id: "exec-denied",
                    agent_id: None,
                },
                Some(json!({ "command": "rm -rf build" })),
            );

            let response = repo
                .drive(&denied)
                .expect("a policy-denied Bash command still returns Ok with a deny payload");
            assert!(response.contains(r#""permissionDecision":"deny""#));

            assert!(
                !repo.adapter_state_file_exists(),
                "AC7: a denied tracked execution must never leave an untracked Start behind"
            );

            let db = repo.db();
            assert_eq!(count(&db, "mutation_trace_scopes"), 0);
            assert_eq!(count(&db, "mutation_trace_events"), 0);

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test24_cross_harness_overlap_at_a_non_confirming_boundary_is_ineligible_ac10() {
            let repo = CodexRepo::new("test24-cross-harness-ineligible");
            let cwd = repo.cwd();
            let codex_call = bash_call(&cwd, "session-cross", "exec-codex");
            let other_scope_id = "claude-scope-1";

            repo.drive_generic(&other_harness_payload("start", other_scope_id))
                .expect("the other harness's Start should commit");
            repo.drive(&codex_call.pre())
                .expect("the Codex PreToolUse should Start");
            let codex_scope_id = repo.live_scope_id();

            repo.write("file.txt", "one\ncontended\n");

            repo.drive_generic(&other_harness_payload("close", other_scope_id))
                .expect("the other harness's Close should commit");

            let db = repo.db();
            let worktree_id = repo.worktree_id();
            assert_eq!(
                mutation_events_for(&db, &worktree_id),
                vec![("ineligible_unscoped".to_string(), None, "close".to_string())],
                "AC10/D14: an unconfirmed live Codex scope forces IneligibleUnscoped at a \
                 boundary that does not confirm it — never AiContended"
            );
            let mut active = active_scopes_for(&db, &worktree_id);
            active.sort();
            let mut expected = vec![codex_scope_id, other_scope_id.to_string()];
            expected.sort();
            assert_eq!(
                active, expected,
                "active_scopes still records the complete live set; only eligibility changes"
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test25_cross_harness_overlap_at_the_codex_close_is_contended_ac10() {
            let repo = CodexRepo::new("test25-cross-harness-contended");
            let cwd = repo.cwd();
            let codex_call = bash_call(&cwd, "session-cross", "exec-codex");
            let other_scope_id = "claude-scope-1";

            repo.drive_generic(&other_harness_payload("start", other_scope_id))
                .expect("the other harness's Start should commit");
            repo.drive(&codex_call.pre())
                .expect("the Codex PreToolUse should Start");
            let codex_scope_id = repo.live_scope_id();

            repo.write("file.txt", "one\ncontended\n");

            repo.drive(&codex_call.post())
                .expect("the Codex PostToolUse should close its scope");

            let db = repo.db();
            let worktree_id = repo.worktree_id();
            assert_eq!(
                mutation_events_for(&db, &worktree_id),
                vec![("ai_contended".to_string(), None, "close".to_string())],
                "AC10/D14: the Codex scope's own Close confirms it, so the overlap with the \
                 other harness's live scope is attributed AiContended"
            );
            assert_eq!(
                scope_status(&db, &codex_scope_id).map(|(_, status)| status),
                Some("closed".to_string())
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test26_a_second_unconfirmed_codex_scope_suppresses_contention_ac10() {
            let repo = CodexRepo::new("test26-second-codex-scope");
            let cwd = repo.cwd();
            let confirmed = bash_call(&cwd, "session-one", "exec-one");
            let unconfirmed = bash_call(&cwd, "session-two", "exec-two");
            let other_scope_id = "claude-scope-1";

            repo.drive_generic(&other_harness_payload("start", other_scope_id))
                .expect("the other harness's Start should commit");
            repo.drive(&confirmed.pre())
                .expect("the first Codex PreToolUse should Start");
            repo.drive(&unconfirmed.pre())
                .expect("a second Codex lane's PreToolUse should Start");
            assert_eq!(repo.adapter_state().attempts.len(), 2);

            repo.write("file.txt", "one\ncontended\n");

            repo.drive(&confirmed.post())
                .expect("the first Codex scope's Close should commit");

            let db = repo.db();
            let worktree_id = repo.worktree_id();
            assert_eq!(
                mutation_events_for(&db, &worktree_id),
                vec![("ineligible_unscoped".to_string(), None, "close".to_string())],
                "AC10/D14: a second unconfirmed live Codex scope suppresses attribution back to \
                 IneligibleUnscoped even at a confirming Codex Close"
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test27_tracked_fixtures_persist_scope_provenance_ac3() {
            for (label, fixture) in [
                ("bash", PROBE01_SHELL_PRE),
                ("apply-patch", PROBE01_APPLY_PATCH_PRE),
            ] {
                let repo = CodexRepo::new(&format!("test27-provenance-{label}"));
                let cwd = repo.cwd();

                repo.drive(&fixture_at(fixture, &cwd))
                    .expect("a tracked PreToolUse should Start");
                let scope_id = repo.live_scope_id();

                let db = repo.db();
                assert_eq!(
                    scope_provenance(&db, &scope_id),
                    Some((
                        "cx_01a07c1e-e08e-7172-8032-cb9d62af21d9".to_string(),
                        Some("gpt-5.6-sol".to_string())
                    )),
                    "AC3: the {label} fixture must persist its cx_ session and normalized model"
                );
                assert_eq!(count(&db, "mutation_trace_scope_provenance"), 1);
                assert_eq!(
                    scope_status(&db, &scope_id),
                    Some(("codex".to_string(), "active".to_string()))
                );

                assert_raw_agent_trace_tables_untouched(&db);
            }
        }

        #[test]
        fn test28_a_tracked_execution_without_a_model_persists_a_null_model_ac3() {
            let repo = CodexRepo::new("test28-provenance-no-model");
            let cwd = repo.cwd();
            let call = bash_call(&cwd, "session-no-model", "exec-no-model");

            repo.drive(&call.pre())
                .expect("a tracked PreToolUse without a model should still Start");
            let scope_id = repo.live_scope_id();

            let db = repo.db();
            assert_eq!(
                scope_provenance(&db, &scope_id),
                Some(("cx_session-no-model".to_string(), None)),
                "AC3: a missing model records model_id = NULL without losing the session"
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[test]
        fn test29_untracked_and_delegation_tools_persist_no_provenance_ac3() {
            let repo = CodexRepo::new("test29-untracked-no-provenance");
            let cwd = repo.cwd();

            for fixture in [
                PROBE12_MCP_PRE,
                PROBE08_SPAWN_AGENT_PRE,
                PROBE08_WAIT_AGENT_PRE,
            ] {
                assert_eq!(
                    repo.drive(&fixture_at(fixture, &cwd))
                        .expect("an untracked or delegation PreToolUse should succeed"),
                    ""
                );
            }

            let db = repo.db();
            assert_eq!(count(&db, "mutation_trace_scopes"), 0);
            assert_eq!(count(&db, "mutation_trace_scope_provenance"), 0);

            assert_raw_agent_trace_tables_untouched(&db);
        }
    }
}
