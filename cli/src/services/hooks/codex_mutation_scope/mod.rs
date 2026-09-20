#![allow(dead_code)]

mod boundary_lock;
pub(crate) mod health;
mod os_lock;
pub(crate) mod state;

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::services::hooks::codex::bash_policy::{
    bash_command_from_tool_input, evaluate_codex_bash_policy, CodexBashPolicyDecision,
};
use crate::services::hooks::{
    normalize_codex_model_id, prefixed_diff_trace_session_id, CODEX_TOOL_NAME,
};
use crate::services::mutation_trace::runtime::resolve_git_dir;
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
    let resolve_git_dir_fn = |cwd: &str| resolve_git_dir(Path::new(cwd));
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
pub(crate) fn run_codex_mutation_scope_from_payload_at_state_root(
    state_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let resolve_git_dir_fn = |cwd: &str| resolve_git_dir(Path::new(cwd));
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
mod tests;
