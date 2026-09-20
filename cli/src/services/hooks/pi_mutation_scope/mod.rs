#![allow(dead_code)]

mod boundary_lock;
pub(crate) mod health;
mod os_lock;
pub(crate) mod process_owner;
pub(crate) mod state;

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::services::hooks::{normalize_pi_model_id, prefixed_diff_trace_session_id, PI_TOOL_NAME};
use crate::services::mutation_trace::runtime::resolve_git_dir;
use crate::services::observability::traits::Logger;

use boundary_lock::{AdapterBoundaryLock, DEFAULT_BOUNDARY_LOCK_TIMEOUT};
use state::{AdmitDecision, RecoveryFlushCompletion};

const HOOK_EVENT_NAME_FIELD: &str = "hook_event_name";
const SESSION_ID_FIELD: &str = "session_id";
const TOOL_CALL_ID_FIELD: &str = "tool_call_id";
const CWD_FIELD: &str = "cwd";
const TOOL_NAME_FIELD: &str = "tool_name";
const MODEL_FIELD: &str = "model";

const HOOK_EVENT_TOOL_EXECUTION_START: &str = "ToolExecutionStart";
const HOOK_EVENT_TOOL_CALL: &str = "ToolCall";
const HOOK_EVENT_TOOL_RESULT: &str = "ToolResult";
const HOOK_EVENT_TOOL_EXECUTION_END: &str = "ToolExecutionEnd";
const HOOK_EVENT_TOOL_EXECUTION_ABANDON: &str = "ToolExecutionAbandon";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PiHookEvent {
    ExecutionStart(PiToolIdentity),
    Call(PiToolCall),
    Executed(PiToolIdentity),
    ExecutionEnd(PiToolIdentity),
    ExecutionAbandon(PiToolIdentity),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PiToolIdentity {
    pub session_id: String,
    pub tool_call_id: String,
    pub cwd: String,
    pub tool_name: String,
}

impl PiToolIdentity {
    pub(crate) fn attempt_key(&self) -> AttemptKey {
        AttemptKey {
            session_id: self.session_id.clone(),
            tool_call_id: self.tool_call_id.clone(),
        }
    }

    pub(crate) fn classification(&self) -> ToolClassification {
        classify_tool(&self.tool_name)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PiToolCall {
    pub identity: PiToolIdentity,
    pub model: Option<String>,
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct AttemptKey {
    pub session_id: String,
    pub tool_call_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolClassification {
    TrackedMutation,
    Untracked,
}

const TRACKED_MUTATION_TOOL_NAMES: &[&str] = &["bash", "edit", "write"];

pub(crate) fn classify_tool(tool_name: &str) -> ToolClassification {
    if TRACKED_MUTATION_TOOL_NAMES.contains(&tool_name) {
        ToolClassification::TrackedMutation
    } else {
        ToolClassification::Untracked
    }
}

const PI_SCOPE_ID_SCHEME: &str = "pi-tool-v1";

pub(crate) fn format_pi_scope_id(key: &AttemptKey, attempt_seq: u64) -> String {
    format!(
        "{PI_SCOPE_ID_SCHEME}|n={attempt_seq}|s={}:{}|c={}:{}",
        key.session_id.len(),
        key.session_id,
        key.tool_call_id.len(),
        key.tool_call_id,
    )
}

pub(crate) fn pi_scope_start_event_id(scope_id: &str) -> String {
    format!("{scope_id}|start")
}

pub(crate) fn pi_scope_close_event_id(scope_id: &str) -> String {
    format!("{scope_id}|close")
}

const ACTOR_KIND_PI: &str = "pi";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PiScopeProvenance {
    pub session_id: String,
    pub model_id: Option<String>,
}

pub(crate) fn pi_scope_provenance(session_id: &str, model: Option<&str>) -> PiScopeProvenance {
    PiScopeProvenance {
        session_id: prefixed_diff_trace_session_id(PI_TOOL_NAME, session_id),
        model_id: model.and_then(normalize_pi_model_id),
    }
}

pub(crate) fn parse_pi_hook_event(stdin_payload: &str) -> Result<PiHookEvent> {
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
        HOOK_EVENT_TOOL_EXECUTION_START => {
            parse_tool_identity(object).map(PiHookEvent::ExecutionStart)
        }
        HOOK_EVENT_TOOL_CALL => parse_tool_call(object).map(PiHookEvent::Call),
        HOOK_EVENT_TOOL_RESULT => parse_tool_identity(object).map(PiHookEvent::Executed),
        HOOK_EVENT_TOOL_EXECUTION_END => parse_tool_identity(object).map(PiHookEvent::ExecutionEnd),
        HOOK_EVENT_TOOL_EXECUTION_ABANDON => {
            parse_tool_identity(object).map(PiHookEvent::ExecutionAbandon)
        }
        other => bail!(validation_error(&format!(
            "unsupported hook_event_name '{other}'"
        ))),
    }
}

fn parse_tool_identity(object: &Map<String, Value>) -> Result<PiToolIdentity> {
    Ok(PiToolIdentity {
        session_id: required_non_blank_str(object, SESSION_ID_FIELD)?,
        tool_call_id: required_non_blank_str(object, TOOL_CALL_ID_FIELD)?,
        cwd: required_non_blank_str(object, CWD_FIELD)?,
        tool_name: required_non_blank_str(object, TOOL_NAME_FIELD)?,
    })
}

fn parse_tool_call(object: &Map<String, Value>) -> Result<PiToolCall> {
    Ok(PiToolCall {
        identity: parse_tool_identity(object)?,
        model: optional_non_blank_str(object, MODEL_FIELD)?,
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
    format!("Invalid Pi hook event payload from STDIN: {detail}.")
}

pub(crate) fn run_pi_mutation_scope_subcommand(logger: Option<&dyn Logger>) -> Result<String> {
    let stdin_payload = super::read_hook_stdin()?;
    run_pi_mutation_scope_from_payload(&stdin_payload, logger)
}

pub(crate) fn run_pi_mutation_scope_from_payload(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let resolve_git_dir_fn = |cwd: &str| resolve_git_dir(Path::new(cwd));
    let seam_fn = |repository_root: &Path, payload: &str, logger: Option<&dyn Logger>| {
        super::mutation_scope::run_mutation_scope_from_payload(repository_root, payload, logger)
    };

    run_pi_mutation_scope_from_payload_with_seams(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &seam_fn,
    )
}

#[cfg(test)]
pub(crate) fn run_pi_mutation_scope_from_payload_at_state_root(
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

    run_pi_mutation_scope_from_payload_with_seams(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &seam_fn,
    )
}

type GitDirResolver<'a> = &'a dyn Fn(&str) -> Result<PathBuf>;

type IngressSeam<'a> = &'a dyn Fn(&Path, &str, Option<&dyn Logger>) -> Result<String>;

const FAIL_CLOSED_MESSAGE: &str =
    "SCE could not establish Pi mutation attribution for this tool execution.";

const FAIL_CLOSED_EVENT: &str = "sce.hooks.pi_mutation_scope.start_fail_closed";

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

fn run_pi_mutation_scope_from_payload_with_seams(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> Result<String> {
    let event = parse_pi_hook_event(stdin_payload)?;
    dispatch_pi_hook_event(event, logger, resolve_git_dir, seam)
}

fn dispatch_pi_hook_event(
    event: PiHookEvent,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> Result<String> {
    match event {
        PiHookEvent::ExecutionStart(_identity) => Ok(String::new()),
        PiHookEvent::Call(call) => match call.identity.classification() {
            ToolClassification::TrackedMutation => {
                let provenance =
                    pi_scope_provenance(&call.identity.session_id, call.model.as_deref());
                establish_tracked_start(
                    &call.identity.cwd,
                    &call.identity.attempt_key(),
                    &call.identity.tool_name,
                    &provenance,
                    logger,
                    resolve_git_dir,
                    seam,
                )
            }
            ToolClassification::Untracked => Ok(String::new()),
        },
        PiHookEvent::Executed(identity) => {
            if !matches!(
                identity.classification(),
                ToolClassification::TrackedMutation
            ) {
                return Ok(String::new());
            }
            let git_dir = resolve_git_dir(&identity.cwd)?;
            state::mark_executed(&git_dir, &identity.attempt_key())?;
            Ok(String::new())
        }
        PiHookEvent::ExecutionEnd(identity) => {
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
                handle_tool_execution_end(&git_dir, repository_root, &key, logger, seam)
            })
        }
        PiHookEvent::ExecutionAbandon(identity) => {
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
                force_abandon_attempt(&git_dir, repository_root, &key, logger, seam)
            })
        }
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
    provenance: &PiScopeProvenance,
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
    match reconcile_stale_owners(git_dir, repository_root, logger, seam)? {
        RecoveryResolution::Cleared => {}
        RecoveryResolution::Unresolved => return Ok(Admission::Denied),
    }

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

fn reconcile_stale_owners(
    git_dir: &Path,
    repository_root: &Path,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<RecoveryResolution> {
    loop {
        let dead_scope_ids = state::find_definitely_dead_attempts(git_dir)?;
        if dead_scope_ids.is_empty() {
            return Ok(RecoveryResolution::Cleared);
        }

        let generation = state::begin_terminal_cleanup(git_dir, &dead_scope_ids)?;
        if matches!(
            resolve_recovery(git_dir, repository_root, generation, logger, seam)?,
            RecoveryResolution::Unresolved
        ) {
            return Ok(RecoveryResolution::Unresolved);
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
    _git_dir: &Path,
    repository_root: &Path,
    allocated: &state::AllocatedAttempt,
    provenance: &PiScopeProvenance,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<()> {
    let scope_id = &allocated.attempt.scope_id;

    let start_payload =
        scope_start_payload(scope_id, &pi_scope_start_event_id(scope_id), provenance);

    seam(repository_root, &start_payload, logger)?;
    Ok(())
}

fn handle_tool_execution_end(
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
        .find(|attempt| {
            attempt.session_id == key.session_id && attempt.tool_call_id == key.tool_call_id
        })
        .cloned()
    else {
        return Ok(String::new());
    };

    let doomed_scope_id = attempt.scope_id.clone();

    if !matches!(attempt.phase, state::AttemptPhase::Executed) {
        return abandon_and_consume(git_dir, repository_root, logger, seam, move |candidate| {
            candidate.scope_id == doomed_scope_id
        });
    }

    let close_payload = scope_boundary_payload(
        "close",
        &attempt.scope_id,
        &pi_scope_close_event_id(&attempt.scope_id),
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

fn force_abandon_attempt(
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
        .find(|attempt| {
            attempt.session_id == key.session_id && attempt.tool_call_id == key.tool_call_id
        })
        .cloned()
    else {
        return Ok(String::new());
    };

    let doomed_scope_id = attempt.scope_id.clone();
    abandon_and_consume(git_dir, repository_root, logger, seam, move |candidate| {
        candidate.scope_id == doomed_scope_id
    })
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
        "actor_kind": ACTOR_KIND_PI,
    })
    .to_string()
}

fn scope_start_payload(scope_id: &str, event_id: &str, provenance: &PiScopeProvenance) -> String {
    json!({
        "operation": "start",
        "scope_id": scope_id,
        "event_id": event_id,
        "actor_kind": ACTOR_KIND_PI,
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
pub(crate) fn force_attempt_owner_dead_for_tests(git_dir: &Path, scope_id: &str) {
    let mut dead_child = std::process::Command::new("true")
        .spawn()
        .expect("spawning 'true' should succeed");
    let dead_pid = i32::try_from(dead_child.id()).expect("pid fits in i32");
    dead_child.wait().expect("child should exit and be reaped");
    state::set_attempt_owner_for_tests(
        git_dir,
        scope_id,
        process_owner::ProcessOwner {
            pid: dead_pid,
            instance_token: None,
        },
    );
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod runtime_seam_tests;

#[cfg(all(unix, test))]
mod guard_reconciliation_tests;
