#![allow(dead_code)]

mod boundary_lock;
mod os_lock;
pub(crate) mod state;

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::services::checkout;
use crate::services::observability::traits::Logger;

use boundary_lock::{AdapterBoundaryLock, DEFAULT_BOUNDARY_LOCK_TIMEOUT};

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

    let key = identity.attempt_key();
    let outcome = with_boundary_lock(&git_dir, || {
        state::normalize_recovery_after_boundary_lock_acquired(&git_dir)?;

        match admit_or_recover(
            &git_dir,
            repository_root,
            &key,
            &identity.tool_name,
            logger,
            seam,
        )? {
            Admission::Admitted(allocated) => {
                establish_start(&git_dir, repository_root, &allocated, logger, seam)?;
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

enum Admission {
    Admitted(state::AllocatedAttempt),
    Denied,
}

fn admit_or_recover(
    git_dir: &Path,
    repository_root: &Path,
    key: &AttemptKey,
    tool_name: &str,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<Admission> {
    match state::admit_tracked_attempt(git_dir, key, tool_name)? {
        state::AdmitDecision::Admitted(allocated) => Ok(Admission::Admitted(allocated)),
        state::AdmitDecision::RecoveryBlocked | state::AdmitDecision::UncertainAttemptBlocked => {
            Ok(Admission::Denied)
        }
        state::AdmitDecision::FlushClaimed { generation } => {
            match seam(repository_root, &flush_payload(), logger) {
                Ok(_) => match state::complete_recovery_flush(git_dir, generation)? {
                    state::RecoveryFlushCompletion::Cleared => {
                        readmit_after_flush(git_dir, key, tool_name)
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

fn readmit_after_flush(git_dir: &Path, key: &AttemptKey, tool_name: &str) -> Result<Admission> {
    match state::admit_tracked_attempt(git_dir, key, tool_name)? {
        state::AdmitDecision::Admitted(allocated) => Ok(Admission::Admitted(allocated)),
        state::AdmitDecision::FlushClaimed { generation } => {
            state::relinquish_recovery_flush(git_dir, generation)?;
            Ok(Admission::Denied)
        }
        state::AdmitDecision::RecoveryBlocked | state::AdmitDecision::UncertainAttemptBlocked => {
            Ok(Admission::Denied)
        }
    }
}

fn establish_start(
    git_dir: &Path,
    repository_root: &Path,
    allocated: &state::AllocatedAttempt,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<()> {
    let scope_id = &allocated.attempt.scope_id;

    if allocated.reused && allocated.attempt.phase == state::AttemptPhase::Active {
        return Ok(());
    }

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

        fn seed_attempt(
            git_dir: &Path,
            session_id: &str,
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

            let successor = pre_tool_use_json(&[(
                TOOL_USE_ID_FIELD,
                Value::String("exec-successor".to_string()),
            )]);
            assert_eq!(
                run_codex_mutation_scope_from_payload_with(
                    &successor,
                    None,
                    &resolver,
                    &unreachable_seam,
                )
                .expect("successor must return Ok with a deny payload"),
                pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON),
                "I5: an unresolved PendingStart must block a successor Start",
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
                    &post_tool_use_json(&[(
                        TOOL_NAME_FIELD,
                        Value::String("mcp__probe__mutate_success".to_string()),
                    )]),
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
            seed_attempt(
                &git_dir,
                "session-1",
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
            let (done_tx, done_rx) = mpsc::channel();
            let resolver = fixed_resolver(git_dir.to_path_buf());
            let handle = thread::spawn(move || {
                let output = run_codex_mutation_scope_from_payload_with(
                    &pre_tool_use_json(&[(
                        TOOL_USE_ID_FIELD,
                        Value::String(tool_use_id.to_string()),
                    )]),
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
                spawn_pre_tool_use(&git_dir, "exec-2", move |_r, payload, _l| {
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
            match state::admit_tracked_attempt(git_dir, &key("seed", None, "seed"), "Bash")
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
                &pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("exec-2".to_string()))]),
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

            let successor =
                pre_tool_use_json(&[(TOOL_USE_ID_FIELD, Value::String("exec-b".to_string()))]);
            assert_eq!(
                run_codex_mutation_scope_from_payload_with(
                    &successor,
                    None,
                    &resolver,
                    &unreachable_seam,
                )
                .expect("successor returns a deny payload"),
                pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON),
                "Test D: an uncertain PendingStart blocks a successor Start",
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
    }
}
