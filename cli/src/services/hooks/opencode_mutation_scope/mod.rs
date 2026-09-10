#![allow(dead_code)]

mod boundary_lock;
mod os_lock;
pub(crate) mod state;

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::services::checkout;
use crate::services::hooks::{
    normalize_opencode_model_id, prefixed_diff_trace_session_id, OPENCODE_TOOL_NAME,
};
use crate::services::observability::traits::Logger;

use boundary_lock::{AdapterBoundaryLock, DEFAULT_BOUNDARY_LOCK_TIMEOUT};
use state::{AdmitDecision, RecoveryFlushCompletion};

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
    pub tool_name: String,
}

impl OpenCodeCallIdentity {
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
        tool_name: required_non_blank_str(object, TOOL_NAME_FIELD)?,
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
    let resolve_git_dir_fn = |cwd: &str| checkout::resolve_git_dir(Path::new(cwd));
    let seam_fn = |repository_root: &Path, payload: &str, logger: Option<&dyn Logger>| {
        super::mutation_scope::run_mutation_scope_from_payload(repository_root, payload, logger)
    };

    run_opencode_mutation_scope_from_payload_with_seams(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &seam_fn,
    )
}

#[cfg(test)]
pub(crate) fn run_opencode_mutation_scope_from_payload_at_state_root(
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

    run_opencode_mutation_scope_from_payload_with_seams(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &seam_fn,
    )
}

type GitDirResolver<'a> = &'a dyn Fn(&str) -> Result<PathBuf>;

type IngressSeam<'a> = &'a dyn Fn(&Path, &str, Option<&dyn Logger>) -> Result<String>;

const FAIL_CLOSED_MESSAGE: &str =
    "SCE could not establish OpenCode mutation attribution for this tool execution.";

const FAIL_CLOSED_EVENT: &str = "sce.hooks.opencode_mutation_scope.start_fail_closed";

fn log_fail_closed(logger: Option<&dyn Logger>, context: &str, error: &anyhow::Error) {
    if let Some(log) = logger {
        log.warn(
            FAIL_CLOSED_EVENT,
            &error.to_string(),
            &[("context", context)],
            None,
        );
    }
}

fn run_opencode_mutation_scope_from_payload_with_seams(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> Result<String> {
    let event = parse_opencode_hook_event(stdin_payload)?;
    dispatch_opencode_hook_event(event, logger, resolve_git_dir, seam)
}

fn dispatch_opencode_hook_event(
    event: OpenCodeHookEvent,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> Result<String> {
    match event {
        OpenCodeHookEvent::ToolExecuteBefore(execution) => {
            match execution.identity.classification() {
                ToolClassification::TrackedMutation => {
                    if execution.identity.tool_name == OPENCODE_TRACKED_TOOL_BASH {
                        return Ok(String::new());
                    }
                    let provenance = opencode_scope_provenance(
                        &execution.identity.session_id,
                        execution.model.as_deref(),
                    );
                    establish_tracked_start(
                        &execution.identity.cwd,
                        &execution.identity.attempt_key(),
                        &execution.identity.tool_name,
                        &provenance,
                        logger,
                        resolve_git_dir,
                        seam,
                    )
                }
                ToolClassification::Delegation | ToolClassification::Untracked => Ok(String::new()),
            }
        }
        OpenCodeHookEvent::ShellEnv(shell) => {
            let provenance = opencode_scope_provenance(&shell.session_id, shell.model.as_deref());
            establish_tracked_start(
                &shell.cwd,
                &shell.attempt_key(),
                OPENCODE_TRACKED_TOOL_BASH,
                &provenance,
                logger,
                resolve_git_dir,
                seam,
            )
        }
        OpenCodeHookEvent::ToolExecuteAfter(identity) => {
            if !matches!(
                identity.classification(),
                ToolClassification::TrackedMutation
            ) {
                return Ok(String::new());
            }
            let git_dir = resolve_git_dir(&identity.cwd)?;
            let repository_root = Path::new(&identity.cwd);
            let key = identity.attempt_key();
            with_boundary_lock(&git_dir, || {
                state::normalize_recovery_after_boundary_lock_acquired(&git_dir)?;
                handle_close(&git_dir, repository_root, &key, logger, seam)
            })
        }
        OpenCodeHookEvent::ToolError(call) => {
            if !matches!(call.classification(), ToolClassification::TrackedMutation) {
                return Ok(String::new());
            }
            let git_dir = resolve_git_dir(&call.cwd)?;
            let repository_root = Path::new(&call.cwd);
            let key = call.attempt_key();
            with_boundary_lock(&git_dir, || {
                state::normalize_recovery_after_boundary_lock_acquired(&git_dir)?;
                abandon_and_consume(&git_dir, repository_root, logger, seam, |attempt| {
                    attempt.session_id == key.session_id && attempt.call_id == key.call_id
                })
            })
        }
        OpenCodeHookEvent::SessionIdle(_)
        | OpenCodeHookEvent::SessionError(_)
        | OpenCodeHookEvent::SessionDeleted(_)
        | OpenCodeHookEvent::ServerDisposed(_) => Ok(String::new()),
    }
}

fn with_boundary_lock<T>(git_dir: &Path, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    let _boundary = AdapterBoundaryLock::acquire(git_dir, DEFAULT_BOUNDARY_LOCK_TIMEOUT)
        .map_err(|error| anyhow!("Failed to acquire adapter boundary lock: {error}"))?;
    operation()
}

enum Admission {
    Admitted(state::AllocatedAttempt),
    Denied,
}

enum StartOutcome {
    Established,
    Denied,
}

fn establish_tracked_start(
    cwd: &str,
    key: &AttemptKey,
    tool_name: &str,
    provenance: &OpenCodeScopeProvenance,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> Result<String> {
    let git_dir = match resolve_git_dir(cwd) {
        Ok(git_dir) => git_dir,
        Err(error) => {
            log_fail_closed(logger, "resolve_git_dir", &error);
            return Err(error.context(FAIL_CLOSED_MESSAGE));
        }
    };
    let repository_root = Path::new(cwd);

    let outcome = with_boundary_lock(&git_dir, || {
        state::normalize_recovery_after_boundary_lock_acquired(&git_dir)?;

        match admit_or_recover(&git_dir, repository_root, key, tool_name, logger, seam)? {
            Admission::Admitted(allocated) => {
                establish_start(
                    &git_dir,
                    repository_root,
                    &allocated,
                    provenance,
                    logger,
                    seam,
                )?;
                Ok(StartOutcome::Established)
            }
            Admission::Denied => Ok(StartOutcome::Denied),
        }
    });

    match outcome {
        Ok(StartOutcome::Established) => Ok(String::new()),
        Ok(StartOutcome::Denied) => bail!(FAIL_CLOSED_MESSAGE),
        Err(error) => {
            log_fail_closed(logger, "establish_tracked_start", &error);
            Err(error.context(FAIL_CLOSED_MESSAGE))
        }
    }
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
        AdmitDecision::Admitted(allocated) => Ok(Admission::Admitted(allocated)),
        AdmitDecision::RecoveryBlocked
        | AdmitDecision::UncertainAttemptBlocked
        | AdmitDecision::TerminalAttemptBlocked => Ok(Admission::Denied),
        AdmitDecision::FlushClaimed { generation } => {
            match resolve_recovery(git_dir, repository_root, generation, logger, seam)? {
                RecoveryResolution::Cleared => readmit_after_flush(git_dir, key, tool_name),
                RecoveryResolution::Unresolved => Ok(Admission::Denied),
            }
        }
    }
}

fn readmit_after_flush(git_dir: &Path, key: &AttemptKey, tool_name: &str) -> Result<Admission> {
    match state::admit_tracked_attempt(git_dir, key, tool_name)? {
        AdmitDecision::Admitted(allocated) => Ok(Admission::Admitted(allocated)),
        AdmitDecision::FlushClaimed { generation } => {
            state::relinquish_recovery_flush(git_dir, generation)?;
            Ok(Admission::Denied)
        }
        AdmitDecision::RecoveryBlocked
        | AdmitDecision::UncertainAttemptBlocked
        | AdmitDecision::TerminalAttemptBlocked => Ok(Admission::Denied),
    }
}

fn establish_start(
    git_dir: &Path,
    repository_root: &Path,
    allocated: &state::AllocatedAttempt,
    provenance: &OpenCodeScopeProvenance,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<()> {
    let scope_id = &allocated.attempt.scope_id;

    if allocated.reused && allocated.attempt.phase == state::AttemptPhase::Active {
        return Ok(());
    }

    let start_payload = scope_start_payload(
        scope_id,
        &opencode_scope_start_event_id(scope_id),
        provenance,
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
        .find(|attempt| attempt.session_id == key.session_id && attempt.call_id == key.call_id)
        .cloned()
    else {
        return Ok(String::new());
    };

    let doomed_scope_id = attempt.scope_id.clone();

    if matches!(
        attempt.phase,
        state::AttemptPhase::PendingStart | state::AttemptPhase::PendingAbandon
    ) {
        return abandon_and_consume(git_dir, repository_root, logger, seam, move |candidate| {
            candidate.scope_id == doomed_scope_id
        });
    }

    let close_payload = scope_boundary_payload(
        "close",
        &attempt.scope_id,
        &opencode_scope_close_event_id(&attempt.scope_id),
    );

    if seam(repository_root, &close_payload, logger).is_ok() {
        state::remove_attempt(git_dir, &attempt.scope_id)?;
        Ok(String::new())
    } else {
        abandon_and_consume(git_dir, repository_root, logger, seam, move |candidate| {
            candidate.scope_id == doomed_scope_id
        })
    }
}

enum RecoveryResolution {
    Cleared,
    Unresolved,
}

fn abandon_and_consume(
    git_dir: &Path,
    repository_root: &Path,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
    doomed: impl Fn(&state::AdapterAttempt) -> bool,
) -> Result<String> {
    let doomed_scope_ids: Vec<String> = state::read_state(git_dir)?
        .attempts
        .into_iter()
        .filter(|attempt| doomed(attempt))
        .map(|attempt| attempt.scope_id)
        .collect();
    if doomed_scope_ids.is_empty() {
        return Ok(String::new());
    }

    let generation = state::begin_terminal_cleanup(git_dir, &doomed_scope_ids)?;
    resolve_recovery(git_dir, repository_root, generation, logger, seam)?;
    Ok(String::new())
}

fn resolve_recovery(
    git_dir: &Path,
    repository_root: &Path,
    generation: u64,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<RecoveryResolution> {
    let pending_abandon: Vec<state::AdapterAttempt> = state::read_state(git_dir)?
        .attempts
        .into_iter()
        .filter(|attempt| attempt.phase == state::AttemptPhase::PendingAbandon)
        .collect();

    if let Err(error) = seam(repository_root, &flush_payload(), logger) {
        log_fail_closed(logger, "recovery_ambiguity_flush", &error);
        state::relinquish_recovery_flush(git_dir, generation)?;
        return Ok(RecoveryResolution::Unresolved);
    }

    for attempt in &pending_abandon {
        if let Err(error) = seam(repository_root, &abandon_payload(&attempt.scope_id), logger) {
            log_fail_closed(logger, "recovery_abandon", &error);
            state::relinquish_recovery_flush(git_dir, generation)?;
            return Ok(RecoveryResolution::Unresolved);
        }
        state::remove_attempt(git_dir, &attempt.scope_id)?;
    }

    if let Err(error) = seam(repository_root, &flush_payload(), logger) {
        log_fail_closed(logger, "recovery_rebaseline_flush", &error);
        state::relinquish_recovery_flush(git_dir, generation)?;
        return Ok(RecoveryResolution::Unresolved);
    }

    match state::complete_recovery_flush(git_dir, generation)? {
        RecoveryFlushCompletion::Cleared => Ok(RecoveryResolution::Cleared),
        RecoveryFlushCompletion::Superseded => Ok(RecoveryResolution::Unresolved),
    }
}

fn scope_boundary_payload(operation: &str, scope_id: &str, event_id: &str) -> String {
    json!({
        "operation": operation,
        "scope_id": scope_id,
        "event_id": event_id,
        "actor_kind": ACTOR_KIND_OPENCODE,
    })
    .to_string()
}

fn scope_start_payload(
    scope_id: &str,
    event_id: &str,
    provenance: &OpenCodeScopeProvenance,
) -> String {
    json!({
        "operation": "start",
        "scope_id": scope_id,
        "event_id": event_id,
        "actor_kind": ACTOR_KIND_OPENCODE,
        "provenance": {
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
        object.insert(
            TOOL_NAME_FIELD.to_string(),
            Value::String("write".to_string()),
        );
        let OpenCodeHookEvent::ToolError(identity) =
            parse_opencode_hook_event(&Value::Object(object).to_string()).unwrap()
        else {
            panic!("expected ToolError");
        };
        assert_eq!(identity.attempt_key(), key("ses_main", "call_1"));
        assert_eq!(
            identity.classification(),
            ToolClassification::TrackedMutation
        );

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
    fn run_from_payload_fails_closed_when_a_tracked_start_cannot_resolve_its_checkout() {
        let payload = tool_event_json(
            HOOK_EVENT_TOOL_EXECUTE_BEFORE,
            &[
                (TOOL_NAME_FIELD, Value::String("write".to_string())),
                (
                    CWD_FIELD,
                    Value::String("/nonexistent/sce/opencode/checkout".to_string()),
                ),
            ],
        );
        let error = run_opencode_mutation_scope_from_payload(&payload, None)
            .expect_err("a tracked Start that cannot resolve its checkout must fail closed");
        assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE), "{error:?}");
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

#[cfg(test)]
mod lifecycle_tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    use serde_json::Value;

    use super::state::{read_state, AttemptPhase, RecoveryState};
    use super::*;

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    fn temp_git_dir(label: &str) -> PathBuf {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "sce-opencode-mutation-scope-lifecycle-{label}-{}-{id}",
            std::process::id()
        ))
    }

    const CWD: &str = "/repo/opencode-checkout";

    struct RecordingSeam {
        calls: Mutex<Vec<String>>,
        fail_operations: Vec<String>,
        fail_once_operations: Mutex<Vec<String>>,
        fail_operation_occurrence: Option<(String, usize)>,
    }

    impl RecordingSeam {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail_operations: Vec::new(),
                fail_once_operations: Mutex::new(Vec::new()),
                fail_operation_occurrence: None,
            }
        }

        fn failing_on(operations: &[&str]) -> Self {
            Self {
                fail_operations: operations.iter().map(|op| (*op).to_string()).collect(),
                ..Self::new()
            }
        }

        fn failing_once_on(operations: &[&str]) -> Self {
            Self {
                fail_once_operations: Mutex::new(
                    operations.iter().map(|op| (*op).to_string()).collect(),
                ),
                ..Self::new()
            }
        }

        fn failing_on_nth_occurrence(operation: &str, occurrence: usize) -> Self {
            Self {
                fail_operation_occurrence: Some((operation.to_string(), occurrence)),
                ..Self::new()
            }
        }

        fn handle(&self, payload: &str) -> Result<String> {
            let operation = operation_of(payload);
            let occurrence = {
                let mut calls = self.calls.lock().expect("seam mutex");
                calls.push(operation.clone());
                calls
                    .iter()
                    .filter(|candidate| *candidate == &operation)
                    .count()
            };
            if self.fail_operations.contains(&operation) {
                bail!("seam failure injected by test for '{operation}'");
            }
            if let Some((target, target_occurrence)) = &self.fail_operation_occurrence {
                if target == &operation && *target_occurrence == occurrence {
                    bail!(
                        "seam failure injected by test for '{operation}' occurrence {occurrence}"
                    );
                }
            }
            {
                let mut once = self.fail_once_operations.lock().expect("seam mutex");
                if let Some(position) = once.iter().position(|candidate| candidate == &operation) {
                    once.remove(position);
                    bail!("transient seam failure injected once by test for '{operation}'");
                }
            }
            Ok(String::new())
        }

        fn operations(&self) -> Vec<String> {
            self.calls.lock().expect("seam mutex").clone()
        }
    }

    fn operation_of(payload: &str) -> String {
        let value: Value = serde_json::from_str(payload).expect("seam payload is JSON");
        value
            .get("operation")
            .and_then(Value::as_str)
            .expect("seam payload has an operation")
            .to_string()
    }

    fn drive(git_dir: &Path, seam: &RecordingSeam, payload: &str) -> Result<String> {
        let resolver = |_cwd: &str| Ok(git_dir.to_path_buf());
        let seam_fn =
            |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| seam.handle(payload);
        run_opencode_mutation_scope_from_payload_with_seams(payload, None, &resolver, &seam_fn)
    }

    fn tool_before(tool_name: &str, call_id: &str) -> String {
        json!({
            "hook_event_name": "ToolExecuteBefore",
            "session_id": "ses_main",
            "call_id": call_id,
            "cwd": CWD,
            "tool_name": tool_name,
            "model": "opencode/big-pickle",
        })
        .to_string()
    }

    fn shell_env(call_id: &str) -> String {
        json!({
            "hook_event_name": "ShellEnv",
            "session_id": "ses_main",
            "call_id": call_id,
            "cwd": CWD,
            "model": "opencode/big-pickle",
        })
        .to_string()
    }

    fn tool_after(tool_name: &str, call_id: &str) -> String {
        json!({
            "hook_event_name": "ToolExecuteAfter",
            "session_id": "ses_main",
            "call_id": call_id,
            "cwd": CWD,
            "tool_name": tool_name,
        })
        .to_string()
    }

    fn tool_error(tool_name: &str, call_id: &str) -> String {
        json!({
            "hook_event_name": "ToolError",
            "session_id": "ses_main",
            "call_id": call_id,
            "cwd": CWD,
            "tool_name": tool_name,
        })
        .to_string()
    }

    fn session_event(hook_event_name: &str, session_id: &str) -> String {
        json!({
            "hook_event_name": hook_event_name,
            "session_id": session_id,
            "cwd": CWD,
        })
        .to_string()
    }

    fn server_disposed() -> String {
        json!({ "hook_event_name": "ServerDisposed", "cwd": CWD }).to_string()
    }

    fn cleanup(git_dir: &Path) {
        let _ = std::fs::remove_dir_all(git_dir);
    }

    #[test]
    fn file_tool_before_establishes_a_write_ahead_start_and_replays_idempotently() {
        let git_dir = temp_git_dir("write-ahead-start");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_before("write", "call_1")).expect("first Start");
        drive(&git_dir, &seam, &tool_before("write", "call_1"))
            .expect("duplicate Start is a no-op");

        assert_eq!(seam.operations(), vec!["start"]);
        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].phase, AttemptPhase::Active);
        assert_eq!(state.attempts[0].tool_name, "write");

        cleanup(&git_dir);
    }

    #[test]
    fn bash_start_is_anchored_to_shell_env_not_tool_execute_before() {
        let git_dir = temp_git_dir("bash-shell-env");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_before("bash", "call_bash")).expect("bash before is inert");
        assert!(seam.operations().is_empty());
        assert!(read_state(&git_dir)
            .expect("state readable")
            .attempts
            .is_empty());

        drive(&git_dir, &seam, &shell_env("call_bash")).expect("shell.env establishes Start");
        assert_eq!(seam.operations(), vec!["start"]);
        assert_eq!(
            read_state(&git_dir).expect("state readable").attempts[0].phase,
            AttemptPhase::Active,
        );

        cleanup(&git_dir);
    }

    #[test]
    fn concurrent_bash_calls_in_one_session_stay_separate_live_scopes() {
        let git_dir = temp_git_dir("concurrent-bash");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &shell_env("call_a")).expect("A Start");
        drive(&git_dir, &seam, &shell_env("call_b")).expect("B Start must not retire A");

        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 2);
        assert!(state
            .attempts
            .iter()
            .all(|a| a.phase == AttemptPhase::Active));
        assert_eq!(seam.operations(), vec!["start", "start"]);

        cleanup(&git_dir);
    }

    #[test]
    fn successful_after_closes_exactly_that_attempt_and_replays_as_a_no_op() {
        let git_dir = temp_git_dir("close-replay");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_before("edit", "call_1")).expect("Start");
        drive(&git_dir, &seam, &tool_after("edit", "call_1")).expect("Close");
        drive(&git_dir, &seam, &tool_after("edit", "call_1")).expect("duplicate Close is a no-op");

        assert_eq!(seam.operations(), vec!["start", "close"]);
        assert!(read_state(&git_dir)
            .expect("state readable")
            .attempts
            .is_empty());

        cleanup(&git_dir);
    }

    #[test]
    fn tool_error_retires_the_named_attempt_and_consumes_the_ambiguous_interval() {
        let git_dir = temp_git_dir("tool-error-consume");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &shell_env("call_a")).expect("A Start");
        drive(&git_dir, &seam, &shell_env("call_b")).expect("B Start");
        drive(&git_dir, &seam, &tool_error("bash", "call_a")).expect("A terminal failure");

        assert_eq!(
            seam.operations(),
            vec!["start", "start", "flush", "abandon", "flush"],
            "the ambiguous interval is flushed before A is abandoned, then the abandon \
             rebaseline is flushed away so B keeps its future intervals",
        );
        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].call_id, "call_b");
        assert_eq!(state.attempts[0].phase, AttemptPhase::Active);
        assert!(
            state.recovery.is_clear(),
            "a successful ambiguity flush clears the recovery barrier while B stays live",
        );

        cleanup(&git_dir);
    }

    #[test]
    fn exact_error_retires_only_the_named_sibling() {
        let git_dir = temp_git_dir("exact-error-siblings");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &shell_env("call_a")).expect("A Start");
        drive(&git_dir, &seam, &shell_env("call_b")).expect("B Start");
        drive(&git_dir, &seam, &shell_env("call_c")).expect("C Start");
        drive(&git_dir, &seam, &tool_error("bash", "call_b")).expect("B terminal failure");

        let state = read_state(&git_dir).expect("state readable");
        let mut remaining: Vec<&str> = state
            .attempts
            .iter()
            .map(|attempt| attempt.call_id.as_str())
            .collect();
        remaining.sort_unstable();
        assert_eq!(remaining, vec!["call_a", "call_c"]);
        assert!(state
            .attempts
            .iter()
            .all(|a| a.phase == AttemptPhase::Active));
        assert!(state.recovery.is_clear());
        assert_eq!(
            seam.operations()
                .iter()
                .filter(|op| *op == "abandon")
                .count(),
            1,
            "abandoning B must not sweep A or C",
        );

        cleanup(&git_dir);
    }

    #[test]
    fn a_close_before_start_confirmation_consumes_rather_than_closes() {
        let git_dir = temp_git_dir("pending-start-close");
        let seam = RecordingSeam::failing_on(&["start"]);

        let error = drive(&git_dir, &seam, &tool_before("write", "call_1"))
            .expect_err("a failed Start seam must fail closed");
        assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE));
        assert_eq!(
            read_state(&git_dir).expect("state readable").attempts[0].phase,
            AttemptPhase::PendingStart,
        );

        let ok_seam = RecordingSeam::new();
        drive(&git_dir, &ok_seam, &tool_after("write", "call_1")).expect("After on a PendingStart");
        assert_eq!(ok_seam.operations(), vec!["flush", "abandon", "flush"]);
        assert!(read_state(&git_dir)
            .expect("state readable")
            .attempts
            .is_empty());

        cleanup(&git_dir);
    }

    #[test]
    fn session_idle_is_non_destructive() {
        let git_dir = temp_git_dir("session-idle-noop");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &shell_env("call_a")).expect("A Start");
        drive(
            &git_dir,
            &seam,
            &json!({
                "hook_event_name": "ShellEnv",
                "session_id": "ses_other",
                "call_id": "call_c",
                "cwd": CWD,
            })
            .to_string(),
        )
        .expect("other-session Start");

        for name in ["SessionIdle", "SessionError", "SessionDeleted"] {
            drive(&git_dir, &seam, &session_event(name, "ses_main")).expect("broad event is inert");
        }

        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(
            state.attempts.len(),
            2,
            "no attempt is retired by a broad event"
        );
        assert!(state
            .attempts
            .iter()
            .all(|a| a.phase == AttemptPhase::Active));
        assert!(state.recovery.is_clear());
        assert_eq!(seam.operations(), vec!["start", "start"]);

        cleanup(&git_dir);
    }

    #[test]
    fn delayed_session_idle_cannot_retire_a_newer_call() {
        let git_dir = temp_git_dir("delayed-session-idle");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &shell_env("call_b")).expect("newer call B Start");
        drive(&git_dir, &seam, &session_event("SessionIdle", "ses_main"))
            .expect("a delayed SessionIdle for the same session arrives after B started");

        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].call_id, "call_b");
        assert_eq!(state.attempts[0].phase, AttemptPhase::Active);
        assert!(state.recovery.is_clear());
        assert!(!seam.operations().iter().any(|op| op == "abandon"));

        cleanup(&git_dir);
    }

    #[test]
    fn server_disposed_cannot_sweep_another_processes_attempt() {
        let git_dir = temp_git_dir("server-disposed-noop");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &shell_env("call_a")).expect("process P1 owns call A");
        drive(
            &git_dir,
            &seam,
            &json!({
                "hook_event_name": "ShellEnv",
                "session_id": "ses_other",
                "call_id": "call_b",
                "cwd": CWD,
            })
            .to_string(),
        )
        .expect("process P2 owns call B in the same checkout");

        drive(&git_dir, &seam, &server_disposed()).expect("P1 server disposal is inert");

        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(
            state.attempts.len(),
            2,
            "one process's disposal must not retire another process's live attempt",
        );
        assert!(state.recovery.is_clear());
        assert_eq!(seam.operations(), vec!["start", "start"]);

        cleanup(&git_dir);
    }

    #[test]
    fn a_failed_sibling_does_not_retire_survivors_or_block_new_starts() {
        let git_dir = temp_git_dir("failed-sibling-survivors");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &shell_env("call_a")).expect("A Start");
        drive(&git_dir, &seam, &shell_env("call_b")).expect("B Start");
        drive(&git_dir, &seam, &tool_error("bash", "call_a")).expect("A fails");

        drive(&git_dir, &seam, &shell_env("call_c"))
            .expect("a new Start is admitted normally once the ambiguity flush cleared recovery");

        let state = read_state(&git_dir).expect("state readable");
        let mut remaining: Vec<&str> = state
            .attempts
            .iter()
            .map(|attempt| attempt.call_id.as_str())
            .collect();
        remaining.sort_unstable();
        assert_eq!(remaining, vec!["call_b", "call_c"]);
        assert!(state
            .attempts
            .iter()
            .all(|a| a.phase == AttemptPhase::Active));
        assert!(state.recovery.is_clear());

        cleanup(&git_dir);
    }

    #[test]
    fn a_terminal_failure_consumes_the_interval_and_the_next_start_proceeds() {
        let git_dir = temp_git_dir("terminal-then-start");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_before("write", "call_1")).expect("Start");
        drive(&git_dir, &seam, &tool_error("write", "call_1")).expect("terminal failure");
        assert!(
            read_state(&git_dir)
                .expect("state readable")
                .recovery
                .is_clear(),
            "the ambiguity flush is done at abandon time, not deferred to the next Start",
        );

        drive(&git_dir, &seam, &tool_before("write", "call_2")).expect("Start after consume");

        assert_eq!(
            seam.operations(),
            vec!["start", "flush", "abandon", "flush", "start"],
        );
        let state = read_state(&git_dir).expect("state readable");
        assert!(state.recovery.is_clear());
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].call_id, "call_2");

        cleanup(&git_dir);
    }

    #[test]
    fn a_failed_ambiguity_flush_stays_recovery_pending_and_fails_closed_starts() {
        let git_dir = temp_git_dir("failed-ambiguity-flush");

        {
            let ok_seam = RecordingSeam::new();
            drive(&git_dir, &ok_seam, &tool_before("write", "call_1")).expect("Start");
        }

        let failing = RecordingSeam::failing_on(&["flush"]);
        drive(&git_dir, &failing, &tool_error("write", "call_1"))
            .expect("a terminal failure whose flush fails still returns (best-effort)");
        assert_eq!(
            read_state(&git_dir).expect("state readable").recovery,
            RecoveryState::Pending { generation: 1 },
            "a failed ambiguity flush retains a recovery-required state",
        );

        let error = drive(&git_dir, &failing, &tool_before("write", "call_2"))
            .expect_err("a new Start while recovery is unresolved must stay fail-closed");
        assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE));
        assert_eq!(
            read_state(&git_dir).expect("state readable").recovery,
            RecoveryState::Pending { generation: 1 },
        );

        cleanup(&git_dir);
    }

    #[test]
    fn untracked_and_delegation_events_are_zero_footprint() {
        let git_dir = temp_git_dir("zero-footprint");
        let seam = RecordingSeam::new();

        for payload in [
            tool_before("read", "call_r"),
            tool_before("task", "call_t"),
            tool_before("some_future_tool", "call_f"),
            tool_after("read", "call_r"),
            tool_error("read", "call_r"),
            tool_error("task", "call_t"),
        ] {
            drive(&git_dir, &seam, &payload).expect("untracked event is neutral");
        }

        assert!(seam.operations().is_empty());
        assert!(
            !git_dir.join("sce").exists(),
            "no state directory is created"
        );

        cleanup(&git_dir);
    }

    #[test]
    fn close_seam_failure_falls_back_to_consume() {
        let git_dir = temp_git_dir("close-seam-failure");

        {
            let ok_seam = RecordingSeam::new();
            drive(&git_dir, &ok_seam, &tool_before("edit", "call_1")).expect("Start");
        }

        let failing = RecordingSeam::failing_on(&["close"]);
        drive(&git_dir, &failing, &tool_after("edit", "call_1")).expect("Close seam failure");

        assert_eq!(
            failing.operations(),
            vec!["close", "flush", "abandon", "flush"],
        );
        let state = read_state(&git_dir).expect("state readable");
        assert!(state.attempts.is_empty());
        assert!(state.recovery.is_clear());

        cleanup(&git_dir);
    }

    #[test]
    fn regression_a_abandon_failure_preserves_terminal_intent_then_recovers() {
        let git_dir = temp_git_dir("regression-a-abandon-failure");

        {
            let ok_seam = RecordingSeam::new();
            drive(&git_dir, &ok_seam, &tool_before("write", "call_1")).expect("A Start");
        }

        let failing = RecordingSeam::failing_on(&["abandon"]);
        drive(&git_dir, &failing, &tool_error("write", "call_1"))
            .expect("a terminal failure whose abandon fails still returns best-effort");

        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1, "A is not forgotten");
        assert_eq!(state.attempts[0].call_id, "call_1");
        assert_eq!(
            state.attempts[0].phase,
            AttemptPhase::PendingAbandon,
            "the exact terminal attempt is durably marked PendingAbandon",
        );
        assert!(
            !state.recovery.is_clear(),
            "recovery stays unresolved while Abandon has not succeeded",
        );
        assert_eq!(
            failing.operations(),
            vec!["flush", "abandon"],
            "the ambiguity flush ran, then the abandon that failed; no rebaseline flush, \
             no removal",
        );

        let healthy = RecordingSeam::new();
        drive(&git_dir, &healthy, &tool_before("write", "call_2"))
            .expect("a healthy retry boundary resolves recovery and admits the new Start");

        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].call_id, "call_2");
        assert!(state.recovery.is_clear());
        assert_eq!(
            healthy.operations(),
            vec!["flush", "abandon", "flush", "start"],
            "recovery replays flush + abandon + rebaseline flush before the new Start",
        );

        cleanup(&git_dir);
    }

    #[test]
    fn regression_b_ambiguity_flush_failure_blocks_new_starts_then_recovers() {
        let git_dir = temp_git_dir("regression-b-ambiguity-flush-failure");
        let live = RecordingSeam::new();

        drive(&git_dir, &live, &shell_env("call_a")).expect("A Start");
        drive(&git_dir, &live, &shell_env("call_b")).expect("B Start");

        let failing = RecordingSeam::failing_on(&["flush"]);
        drive(&git_dir, &failing, &tool_error("bash", "call_a")).expect("A terminal failure");

        let state = read_state(&git_dir).expect("state readable");
        let pending_abandon: Vec<&str> = state
            .attempts
            .iter()
            .filter(|attempt| attempt.phase == AttemptPhase::PendingAbandon)
            .map(|attempt| attempt.call_id.as_str())
            .collect();
        let active: Vec<&str> = state
            .attempts
            .iter()
            .filter(|attempt| attempt.phase == AttemptPhase::Active)
            .map(|attempt| attempt.call_id.as_str())
            .collect();
        assert_eq!(pending_abandon, vec!["call_a"], "A is PendingAbandon");
        assert_eq!(active, vec!["call_b"], "B stays Active");
        assert!(!state.recovery.is_clear(), "recovery pending");

        let error = drive(&git_dir, &failing, &shell_env("call_c"))
            .expect_err("a new tracked Start must fail closed while recovery is unresolved");
        assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE));
        assert!(read_state(&git_dir)
            .expect("state readable")
            .attempts
            .iter()
            .all(|attempt| attempt.call_id != "call_c"));

        let healthy = RecordingSeam::new();
        drive(&git_dir, &healthy, &shell_env("call_c"))
            .expect("once recovery succeeds a new Start is admitted normally");

        let state = read_state(&git_dir).expect("state readable");
        let mut remaining: Vec<&str> = state
            .attempts
            .iter()
            .map(|attempt| attempt.call_id.as_str())
            .collect();
        remaining.sort_unstable();
        assert_eq!(
            remaining,
            vec!["call_b", "call_c"],
            "A removed, B kept, C admitted"
        );
        assert!(state
            .attempts
            .iter()
            .all(|attempt| attempt.phase == AttemptPhase::Active));
        assert!(state.recovery.is_clear());

        cleanup(&git_dir);
    }

    #[test]
    fn regression_c_rebaseline_flush_failure_is_recoverable_without_poison() {
        let git_dir = temp_git_dir("regression-c-rebaseline-flush-failure");

        {
            let ok_seam = RecordingSeam::new();
            drive(&git_dir, &ok_seam, &tool_before("edit", "call_1")).expect("Start");
        }

        let rebaseline_failing = RecordingSeam::failing_on_nth_occurrence("flush", 2);
        drive(&git_dir, &rebaseline_failing, &tool_error("edit", "call_1"))
            .expect("terminal failure whose rebaseline flush fails still returns");

        assert_eq!(
            rebaseline_failing.operations(),
            vec!["flush", "abandon", "flush"],
            "the ambiguity flush and the abandon succeeded; the rebaseline flush failed",
        );
        let state = read_state(&git_dir).expect("state readable");
        assert!(
            state.attempts.is_empty(),
            "the abandon succeeded so the attempt is removed",
        );
        assert!(
            !state.recovery.is_clear(),
            "recovery state still carries the outstanding rebaseline",
        );

        let healthy = RecordingSeam::new();
        drive(&git_dir, &healthy, &tool_before("edit", "call_2")).expect("retry admits new work");

        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].call_id, "call_2");
        assert!(state.recovery.is_clear());

        cleanup(&git_dir);
    }

    #[test]
    fn regression_e_duplicate_tool_error_is_idempotent() {
        let git_dir = temp_git_dir("regression-e-duplicate-tool-error");

        {
            let ok_seam = RecordingSeam::new();
            drive(&git_dir, &ok_seam, &tool_before("write", "call_1")).expect("Start");
        }

        let stuck = RecordingSeam::failing_on(&["abandon"]);
        drive(&git_dir, &stuck, &tool_error("write", "call_1")).expect("first terminal failure");
        drive(&git_dir, &stuck, &tool_error("write", "call_1"))
            .expect("a duplicate ToolError while PendingAbandon is idempotent");

        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1, "no second attempt is created");
        assert_eq!(state.attempts[0].phase, AttemptPhase::PendingAbandon);
        assert_eq!(
            state.next_recovery_generation, 2,
            "the recovery generation is not incremented by the duplicate",
        );

        cleanup(&git_dir);
    }

    #[test]
    fn regression_f_start_replay_for_a_pending_abandon_identity_never_reactivates() {
        let git_dir = temp_git_dir("regression-f-start-replay-pending-abandon");

        {
            let ok_seam = RecordingSeam::new();
            drive(&git_dir, &ok_seam, &tool_before("write", "call_1")).expect("Start");
        }

        let stuck = RecordingSeam::failing_on(&["abandon"]);
        drive(&git_dir, &stuck, &tool_error("write", "call_1")).expect("terminal failure");

        let error = drive(&git_dir, &stuck, &tool_before("write", "call_1"))
            .expect_err("a replayed Start for a PendingAbandon identity must fail closed");
        assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE));

        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(
            state.attempts[0].phase,
            AttemptPhase::PendingAbandon,
            "the replayed Start does not return the attempt to Active",
        );

        cleanup(&git_dir);
    }

    #[test]
    fn regression_g_late_tool_error_after_close_is_a_harmless_no_op() {
        let git_dir = temp_git_dir("regression-g-late-tool-error");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_before("write", "call_1")).expect("Start");
        drive(&git_dir, &seam, &tool_after("write", "call_1")).expect("Close");
        drive(&git_dir, &seam, &tool_error("write", "call_1"))
            .expect("a late ToolError after a completed Close is inert");

        assert_eq!(seam.operations(), vec!["start", "close"]);
        let state = read_state(&git_dir).expect("state readable");
        assert!(state.attempts.is_empty());
        assert!(state.recovery.is_clear());

        cleanup(&git_dir);
    }

    #[test]
    fn regression_h_siblings_stay_active_through_a_transient_cleanup_failure() {
        let git_dir = temp_git_dir("regression-h-siblings-preserved");
        let live = RecordingSeam::new();

        drive(&git_dir, &live, &shell_env("call_a")).expect("A Start");
        drive(&git_dir, &live, &shell_env("call_b")).expect("B Start");
        drive(&git_dir, &live, &shell_env("call_c")).expect("C Start");

        let transient = RecordingSeam::failing_once_on(&["abandon"]);
        drive(&git_dir, &transient, &tool_error("bash", "call_b"))
            .expect("B terminal failure with a transient abandon failure");

        let snapshot = read_state(&git_dir).expect("state readable");
        let mut siblings: Vec<&str> = snapshot
            .attempts
            .iter()
            .filter(|attempt| attempt.phase == AttemptPhase::Active)
            .map(|attempt| attempt.call_id.as_str())
            .collect();
        siblings.sort_unstable();
        assert_eq!(
            siblings,
            vec!["call_a", "call_c"],
            "A and C stay Active mid-failure"
        );

        let healthy = RecordingSeam::new();
        drive(&git_dir, &healthy, &shell_env("call_d"))
            .expect("the retry boundary resolves recovery and admits D");

        let state = read_state(&git_dir).expect("state readable");
        let mut remaining: Vec<&str> = state
            .attempts
            .iter()
            .map(|attempt| attempt.call_id.as_str())
            .collect();
        remaining.sort_unstable();
        assert_eq!(
            remaining,
            vec!["call_a", "call_c", "call_d"],
            "B gone, A/C/D active"
        );
        assert!(state
            .attempts
            .iter()
            .all(|attempt| attempt.phase == AttemptPhase::Active));
        assert!(state.recovery.is_clear());

        cleanup(&git_dir);
    }
}

#[cfg(test)]
mod runtime_seam_tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
    use crate::services::agent_trace_storage::{
        resolve_agent_trace_storage_at_state_root, AgentTraceStorageContext,
    };
    use crate::services::checkout::resolve_git_dir;

    use super::state::read_state;
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
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
    }

    struct OpenCodeRepo {
        _temp: tempfile::TempDir,
        root: PathBuf,
        state_root: PathBuf,
    }

    impl OpenCodeRepo {
        fn new(label: &str) -> Self {
            let temp = tempfile::Builder::new()
                .prefix(&format!("sce-opencode-mutation-scope-seam-{label}-"))
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
                _temp: temp,
                root,
                state_root,
            }
        }

        fn cwd(&self) -> String {
            self.root.to_string_lossy().into_owned()
        }

        fn drive(&self, payload: &str) -> Result<String> {
            run_opencode_mutation_scope_from_payload_at_state_root(&self.state_root, payload, None)
        }

        fn drive_failing_seam_operation_once(
            &self,
            payload: &str,
            fail_operation: &str,
            remaining_failures: &std::cell::Cell<u32>,
        ) -> Result<String> {
            let resolver = |cwd: &str| resolve_git_dir(Path::new(cwd));
            let seam_fn = |root: &Path, seam_payload: &str, logger: Option<&dyn Logger>| {
                let operation = serde_json::from_str::<serde_json::Value>(seam_payload)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("operation")
                            .and_then(|op| op.as_str())
                            .map(str::to_owned)
                    })
                    .unwrap_or_default();
                if operation == fail_operation && remaining_failures.get() > 0 {
                    remaining_failures.set(remaining_failures.get() - 1);
                    return Err(anyhow!("injected transient '{operation}' seam failure"));
                }
                crate::services::hooks::mutation_scope::run_mutation_scope_from_payload_at_state_root(
                    root,
                    &self.state_root,
                    seam_payload,
                    logger,
                )
            };
            run_opencode_mutation_scope_from_payload_with_seams(payload, None, &resolver, &seam_fn)
        }

        fn write(&self, name: &str, contents: &str) {
            fs::write(self.root.join(name), contents).expect("write should succeed");
        }

        fn git_dir(&self) -> PathBuf {
            resolve_git_dir(&self.root).expect("git dir should resolve")
        }

        fn db(&self) -> RepositoryAgentTraceDb {
            crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                &self.root,
                &self.state_root,
                "opencode mutation-scope seam test assertions",
            )
            .expect("assertion DB should open")
        }

        fn scope_status(&self, scope_id: &str) -> Option<(String, String)> {
            self.db()
                .query_map(
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

        fn scope_count(&self) -> i64 {
            self.db()
                .query_map("SELECT COUNT(*) FROM mutation_trace_scopes", (), |row| {
                    row.get::<i64>(0).map_err(anyhow::Error::from)
                })
                .expect("count query should succeed")
                .into_iter()
                .next()
                .expect("a count row should exist")
        }

        fn mutation_events(&self) -> Vec<(String, Option<String>)> {
            self.db()
                .query_map(
                    "SELECT attribution_kind, attribution_scope_id \
                     FROM mutation_trace_events ORDER BY revision",
                    (),
                    |row| {
                        let attribution_kind = row.get::<String>(0).map_err(anyhow::Error::from)?;
                        let attribution_scope_id =
                            row.get::<Option<String>>(1).map_err(anyhow::Error::from)?;
                        Ok((attribution_kind, attribution_scope_id))
                    },
                )
                .expect("mutation-events query should succeed")
        }
    }

    fn error(repo: &OpenCodeRepo, tool_name: &str, call_id: &str) -> String {
        json!({
            "hook_event_name": "ToolError",
            "session_id": "ses_seam",
            "call_id": call_id,
            "cwd": repo.cwd(),
            "tool_name": tool_name,
        })
        .to_string()
    }

    fn before(repo: &OpenCodeRepo, tool_name: &str, call_id: &str) -> String {
        json!({
            "hook_event_name": "ToolExecuteBefore",
            "session_id": "ses_seam",
            "call_id": call_id,
            "cwd": repo.cwd(),
            "tool_name": tool_name,
            "model": "opencode/big-pickle",
        })
        .to_string()
    }

    fn after(repo: &OpenCodeRepo, tool_name: &str, call_id: &str) -> String {
        json!({
            "hook_event_name": "ToolExecuteAfter",
            "session_id": "ses_seam",
            "call_id": call_id,
            "cwd": repo.cwd(),
            "tool_name": tool_name,
        })
        .to_string()
    }

    #[test]
    fn a_write_start_then_after_closes_the_scope_through_the_real_runtime() {
        let repo = OpenCodeRepo::new("write-start-close");

        assert_eq!(
            repo.drive(&before(&repo, "write", "call_1"))
                .expect("Start"),
            ""
        );
        let scope_id = {
            let state = read_state(&repo.git_dir()).expect("adapter state readable");
            assert_eq!(state.attempts.len(), 1);
            state.attempts[0].scope_id.clone()
        };

        repo.write("file.txt", "one\ntwo\n");
        assert_eq!(
            repo.drive(&after(&repo, "write", "call_1")).expect("Close"),
            ""
        );

        assert!(read_state(&repo.git_dir())
            .expect("adapter state readable")
            .attempts
            .is_empty());
        assert_eq!(
            repo.scope_status(&scope_id),
            Some(("opencode".to_string(), "closed".to_string())),
        );
    }

    #[test]
    fn a_tool_error_abandons_the_scope_through_the_real_runtime() {
        let repo = OpenCodeRepo::new("tool-error-abandon");

        repo.drive(&before(&repo, "edit", "call_1")).expect("Start");
        let scope_id = read_state(&repo.git_dir())
            .expect("adapter state readable")
            .attempts[0]
            .scope_id
            .clone();

        repo.write("file.txt", "one\nabandoned-a\n");
        repo.drive(&error(&repo, "edit", "call_1"))
            .expect("terminal failure");

        let state = read_state(&repo.git_dir()).expect("adapter state readable");
        assert!(state.attempts.is_empty());
        assert!(
            state.recovery.is_clear(),
            "a successful ambiguity flush clears the recovery barrier",
        );
        assert_eq!(
            repo.scope_status(&scope_id).map(|(_, status)| status),
            Some("abandoned".to_string()),
        );
    }

    #[test]
    fn regression_a_failed_concurrent_scope_cannot_contaminate_a_survivor() {
        let repo = OpenCodeRepo::new("regression-a-no-contamination");

        repo.drive(&before(&repo, "write", "call_a"))
            .expect("A Start");
        repo.drive(&before(&repo, "write", "call_b"))
            .expect("B Start");
        let scope_b = read_state(&repo.git_dir())
            .expect("adapter state readable")
            .attempts
            .iter()
            .find(|attempt| attempt.call_id == "call_b")
            .expect("B is tracked")
            .scope_id
            .clone();

        repo.write("file_a.txt", "a mutated\n");
        repo.write("file_b.txt", "b mutated\n");

        repo.drive(&error(&repo, "write", "call_a"))
            .expect("A terminal failure");

        let state = read_state(&repo.git_dir()).expect("adapter state readable");
        assert_eq!(state.attempts.len(), 1, "B stays tracked after A's cleanup");
        assert_eq!(state.attempts[0].call_id, "call_b");
        assert_eq!(state.attempts[0].phase, super::state::AttemptPhase::Active);

        repo.drive(&after(&repo, "write", "call_b"))
            .expect("B Close");

        let events = repo.mutation_events();
        assert!(
            events.iter().any(|(kind, _)| kind == "ineligible_unscoped"),
            "the ambiguous interval containing A's mutations is consumed as \
             IneligibleUnscoped: {events:?}",
        );
        assert!(
            !events.iter().any(|(kind, scope)| kind == "ai_exclusive"
                && scope.as_deref() == Some(scope_b.as_str())),
            "B must never be attributed the interval that could contain A's changes: {events:?}",
        );
        assert_eq!(
            repo.scope_status(&scope_b).map(|(_, status)| status),
            Some("closed".to_string()),
        );
    }

    #[test]
    fn regression_b_survivor_still_attributes_its_later_mutations() {
        let repo = OpenCodeRepo::new("regression-b-survivor-liveness");

        repo.drive(&before(&repo, "write", "call_a"))
            .expect("A Start");
        repo.drive(&before(&repo, "write", "call_b"))
            .expect("B Start");
        let scope_b = read_state(&repo.git_dir())
            .expect("adapter state readable")
            .attempts
            .iter()
            .find(|attempt| attempt.call_id == "call_b")
            .expect("B is tracked")
            .scope_id
            .clone();

        repo.write("file_a.txt", "a mutated\n");
        repo.drive(&error(&repo, "write", "call_a"))
            .expect("A terminal failure consumes the ambiguous interval");

        repo.write("file_c.txt", "b's own later work\n");
        repo.drive(&after(&repo, "write", "call_b"))
            .expect("B Close");

        let events = repo.mutation_events();
        assert!(
            events.iter().any(|(kind, scope)| kind == "ai_exclusive"
                && scope.as_deref() == Some(scope_b.as_str())),
            "after the ambiguity flush, B may legitimately attribute its own later \
             mutations: {events:?}",
        );
        assert_eq!(
            repo.scope_status(&scope_b).map(|(_, status)| status),
            Some("closed".to_string()),
        );
    }

    #[test]
    fn regression_c_exact_error_does_not_sweep_siblings_through_the_real_runtime() {
        let repo = OpenCodeRepo::new("regression-c-no-sibling-sweep");

        repo.drive(&before(&repo, "write", "call_a"))
            .expect("A Start");
        repo.drive(&before(&repo, "write", "call_b"))
            .expect("B Start");
        repo.drive(&before(&repo, "write", "call_c"))
            .expect("C Start");

        repo.write("file.txt", "one\nmutated\n");
        repo.drive(&error(&repo, "write", "call_b"))
            .expect("B terminal failure");

        let state = read_state(&repo.git_dir()).expect("adapter state readable");
        let mut remaining: Vec<&str> = state
            .attempts
            .iter()
            .map(|attempt| attempt.call_id.as_str())
            .collect();
        remaining.sort_unstable();
        assert_eq!(remaining, vec!["call_a", "call_c"], "A and C remain active");
        assert!(state
            .attempts
            .iter()
            .all(|a| a.phase == super::state::AttemptPhase::Active));
        assert!(state.recovery.is_clear());
    }

    #[test]
    fn regression_d_delayed_session_idle_cannot_kill_a_newer_call() {
        let repo = OpenCodeRepo::new("regression-d-delayed-session-idle");

        repo.drive(&before(&repo, "write", "call_b"))
            .expect("newer call B Start");

        repo.drive(
            &json!({
                "hook_event_name": "SessionIdle",
                "session_id": "ses_seam",
                "cwd": repo.cwd(),
            })
            .to_string(),
        )
        .expect("a delayed SessionIdle for the same session is inert");

        let state = read_state(&repo.git_dir()).expect("adapter state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].call_id, "call_b");
        assert_eq!(state.attempts[0].phase, super::state::AttemptPhase::Active);
        assert!(state.recovery.is_clear());
    }

    #[test]
    fn regression_e_server_disposed_cannot_sweep_another_process() {
        let repo = OpenCodeRepo::new("regression-e-server-disposed");

        repo.drive(&before(&repo, "write", "call_a"))
            .expect("P1 owns call A");
        repo.drive(
            &json!({
                "hook_event_name": "ToolExecuteBefore",
                "session_id": "ses_p2",
                "call_id": "call_b",
                "cwd": repo.cwd(),
                "tool_name": "write",
            })
            .to_string(),
        )
        .expect("P2 owns call B in the same checkout");

        repo.drive(&json!({ "hook_event_name": "ServerDisposed", "cwd": repo.cwd() }).to_string())
            .expect("P1 server disposal is inert");

        let state = read_state(&repo.git_dir()).expect("adapter state readable");
        assert_eq!(
            state.attempts.len(),
            2,
            "one process's disposal must not retire another process's live attempt",
        );
        assert!(state.recovery.is_clear());
    }

    #[test]
    fn regression_f_untracked_tool_error_is_zero_footprint() {
        let repo = OpenCodeRepo::new("regression-f-untracked-tool-error");

        for tool_name in ["read", "task", "brave-search_brave_web_search"] {
            repo.drive(&error(&repo, tool_name, "call_x"))
                .expect("an untracked ToolError is neutral");
        }

        assert_eq!(repo.scope_count(), 0);
        assert!(!super::state::adapter_state_dir(&repo.git_dir())
            .join("opencode-mutation-scope-state.json")
            .exists());
    }

    #[test]
    fn untracked_events_never_reach_the_runtime_or_touch_adapter_state() {
        let repo = OpenCodeRepo::new("untracked-zero-footprint");

        assert_eq!(
            repo.drive(&before(&repo, "read", "call_r")).expect("read"),
            ""
        );
        assert_eq!(
            repo.drive(&before(&repo, "task", "call_t")).expect("task"),
            ""
        );

        assert_eq!(repo.scope_count(), 0);
        assert!(!super::state::adapter_state_dir(&repo.git_dir())
            .join("opencode-mutation-scope-state.json")
            .exists());
    }

    #[test]
    fn regression_d_concurrent_survivor_stays_usable_after_a_transient_cleanup_failure() {
        let repo = OpenCodeRepo::new("regression-d-transient-cleanup-failure");

        repo.drive(&before(&repo, "write", "call_a"))
            .expect("A Start");
        repo.drive(&before(&repo, "write", "call_b"))
            .expect("B Start");
        let scoped = read_state(&repo.git_dir()).expect("adapter state readable");
        let scope_a = scoped
            .attempts
            .iter()
            .find(|attempt| attempt.call_id == "call_a")
            .expect("A is tracked")
            .scope_id
            .clone();
        let scope_b = scoped
            .attempts
            .iter()
            .find(|attempt| attempt.call_id == "call_b")
            .expect("B is tracked")
            .scope_id
            .clone();

        repo.write("file_a.txt", "a mutated\n");
        repo.write("file_b.txt", "b mutated\n");

        let remaining_failures = std::cell::Cell::new(1_u32);
        repo.drive_failing_seam_operation_once(
            &error(&repo, "write", "call_a"),
            "abandon",
            &remaining_failures,
        )
        .expect("A terminal failure with a transient abandon failure returns best-effort");

        let state = read_state(&repo.git_dir()).expect("adapter state readable");
        assert_eq!(
            state
                .attempts
                .iter()
                .find(|attempt| attempt.call_id == "call_a")
                .map(|attempt| attempt.phase),
            Some(super::state::AttemptPhase::PendingAbandon),
            "A's terminal intent survives the transient failure",
        );
        assert_eq!(
            state
                .attempts
                .iter()
                .find(|attempt| attempt.call_id == "call_b")
                .map(|attempt| attempt.phase),
            Some(super::state::AttemptPhase::Active),
            "B is untouched",
        );
        assert!(!state.recovery.is_clear());

        repo.drive(&error(&repo, "write", "call_a"))
            .expect("a healthy duplicate ToolError retries and completes cleanup");

        let state = read_state(&repo.git_dir()).expect("adapter state readable");
        let remaining: Vec<&str> = state
            .attempts
            .iter()
            .map(|attempt| attempt.call_id.as_str())
            .collect();
        assert_eq!(remaining, vec!["call_b"], "A retired, B still live");
        assert_eq!(state.attempts[0].phase, super::state::AttemptPhase::Active);
        assert!(state.recovery.is_clear());
        assert_eq!(
            repo.scope_status(&scope_a).map(|(_, status)| status),
            Some("abandoned".to_string()),
        );

        repo.write("file_c.txt", "b's own later work\n");
        repo.drive(&after(&repo, "write", "call_b"))
            .expect("B Close");

        let events = repo.mutation_events();
        assert!(
            events.iter().any(|(kind, _)| kind == "ineligible_unscoped"),
            "the ambiguous A/B interval is consumed as IneligibleUnscoped: {events:?}",
        );
        assert!(
            events.iter().any(|(kind, scope)| kind == "ai_exclusive"
                && scope.as_deref() == Some(scope_b.as_str())),
            "B still attributes its own later mutation once recovery completed: {events:?}",
        );
    }
}
