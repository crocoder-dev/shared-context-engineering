#![allow(dead_code)]

mod boundary_lock;
mod os_lock;
mod process_owner;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PiHookEvent {
    ExecutionStart(PiToolIdentity),
    Call(PiToolCall),
    Executed(PiToolIdentity),
    ExecutionEnd(PiToolIdentity),
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

/// D10: every tracked Start admission is a reconciliation opportunity, independent of the
/// incoming key. Repeatedly collects `PendingStart`/`Executed` attempts with a positively dead
/// owner (any session, any prior process) and retires them through the existing D8
/// flush/abandon/flush sequence, grouping every independently-proven-dead scope into one
/// generation per pass. Live and uncertain-owner attempts are left untouched.
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
            Value::String("01a091f4-session".to_string()),
        );
        object.insert(
            TOOL_CALL_ID_FIELD.to_string(),
            Value::String("call_1|fc_1".to_string()),
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

    fn key(session_id: &str, tool_call_id: &str) -> AttemptKey {
        AttemptKey {
            session_id: session_id.to_string(),
            tool_call_id: tool_call_id.to_string(),
        }
    }

    fn tool_call(payload: &str) -> PiToolCall {
        match parse_pi_hook_event(payload).expect("valid ToolCall parses") {
            PiHookEvent::Call(call) => call,
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn empty_payload_is_rejected() {
        let error = parse_pi_hook_event("   ").unwrap_err().to_string();
        assert_eq!(
            error,
            "Invalid Pi hook event payload from STDIN: expected a JSON object, got an empty payload."
        );
    }

    #[test]
    fn non_object_json_is_rejected() {
        for payload in ["[]", "\"ToolCall\"", "42", "null"] {
            let error = parse_pi_hook_event(payload).unwrap_err().to_string();
            assert!(
                error.contains("expected a JSON object"),
                "payload {payload:?} produced {error:?}"
            );
        }
    }

    #[test]
    fn invalid_json_is_rejected() {
        let error = parse_pi_hook_event("{not json").unwrap_err().to_string();
        assert!(
            error.contains("Invalid Pi hook event payload from STDIN: expected valid JSON"),
            "{error:?}"
        );
    }

    #[test]
    fn unsupported_hook_event_name_is_rejected() {
        for name in ["PreToolUse", "tool_call", "chat.params", ""] {
            let payload = tool_event_json(name, &[]);
            let error = parse_pi_hook_event(&payload).unwrap_err().to_string();
            assert!(
                error.contains("hook_event_name"),
                "name {name:?} produced {error:?}"
            );
        }
    }

    #[test]
    fn missing_required_fields_are_rejected_without_fabricating_identity() {
        for field in [
            SESSION_ID_FIELD,
            TOOL_CALL_ID_FIELD,
            CWD_FIELD,
            TOOL_NAME_FIELD,
        ] {
            let mut object: Map<String, Value> =
                serde_json::from_str(&tool_event_json(HOOK_EVENT_TOOL_CALL, &[])).unwrap();
            object.remove(field);
            let payload = Value::Object(object).to_string();

            let error = parse_pi_hook_event(&payload).unwrap_err().to_string();
            assert!(
                error.contains(&format!("'{field}'")),
                "missing {field} produced {error:?}"
            );
        }
    }

    #[test]
    fn blank_required_fields_are_rejected() {
        for field in [
            SESSION_ID_FIELD,
            TOOL_CALL_ID_FIELD,
            CWD_FIELD,
            TOOL_NAME_FIELD,
        ] {
            let payload = tool_event_json(
                HOOK_EVENT_TOOL_CALL,
                &[(field, Value::String("   ".to_string()))],
            );
            let error = parse_pi_hook_event(&payload).unwrap_err().to_string();
            assert!(
                error.contains(&format!("field '{field}' must be a non-blank string")),
                "blank {field} produced {error:?}"
            );
        }
    }

    #[test]
    fn wrong_typed_fields_are_rejected() {
        let payload = tool_event_json(
            HOOK_EVENT_TOOL_CALL,
            &[(TOOL_CALL_ID_FIELD, Value::Bool(true))],
        );
        let error = parse_pi_hook_event(&payload).unwrap_err().to_string();
        assert!(
            error.contains("field 'tool_call_id' must be a string"),
            "{error:?}"
        );
    }

    #[test]
    fn wrong_typed_optional_model_is_rejected() {
        let payload = tool_event_json(HOOK_EVENT_TOOL_CALL, &[(MODEL_FIELD, Value::Bool(false))]);
        let error = parse_pi_hook_event(&payload).unwrap_err().to_string();
        assert!(
            error.contains("field 'model' must be null, absent, or a non-blank string"),
            "{error:?}"
        );
    }

    #[test]
    fn tool_call_parses_identity_and_model() {
        let call = tool_call(&tool_event_json(
            HOOK_EVENT_TOOL_CALL,
            &[
                (TOOL_NAME_FIELD, Value::String("edit".to_string())),
                (
                    MODEL_FIELD,
                    Value::String("openai-codex/gpt-5.5".to_string()),
                ),
            ],
        ));
        assert_eq!(call.identity.session_id, "01a091f4-session");
        assert_eq!(call.identity.tool_call_id, "call_1|fc_1");
        assert_eq!(call.identity.tool_name, "edit");
        assert_eq!(call.model.as_deref(), Some("openai-codex/gpt-5.5"));
        assert_eq!(
            call.identity.classification(),
            ToolClassification::TrackedMutation
        );
    }

    #[test]
    fn tool_call_model_is_optional() {
        let call = tool_call(&tool_event_json(HOOK_EVENT_TOOL_CALL, &[]));
        assert_eq!(call.model, None);
    }

    #[test]
    fn tool_result_and_tool_execution_end_parse_minimal_identity() {
        for name in [HOOK_EVENT_TOOL_RESULT, HOOK_EVENT_TOOL_EXECUTION_END] {
            let event = parse_pi_hook_event(&tool_event_json(name, &[])).unwrap();
            let identity = match event {
                PiHookEvent::Executed(identity) | PiHookEvent::ExecutionEnd(identity) => identity,
                other => panic!("expected a minimal-identity event, got {other:?}"),
            };
            assert_eq!(
                identity.attempt_key(),
                key("01a091f4-session", "call_1|fc_1")
            );
        }
    }

    #[test]
    fn tool_execution_start_parses_and_is_never_evidence() {
        let event =
            parse_pi_hook_event(&tool_event_json(HOOK_EVENT_TOOL_EXECUTION_START, &[])).unwrap();
        let PiHookEvent::ExecutionStart(identity) = event else {
            panic!("expected ToolExecutionStart");
        };
        assert_eq!(identity.tool_call_id, "call_1|fc_1");
    }

    #[test]
    fn classification_table() {
        let cases: &[(&str, ToolClassification)] = &[
            ("bash", ToolClassification::TrackedMutation),
            ("edit", ToolClassification::TrackedMutation),
            ("write", ToolClassification::TrackedMutation),
            ("read", ToolClassification::Untracked),
            ("grep", ToolClassification::Untracked),
            ("find", ToolClassification::Untracked),
            ("ls", ToolClassification::Untracked),
            ("probe_mutate", ToolClassification::Untracked),
            ("Bash", ToolClassification::Untracked),
            ("some_future_pi_builtin", ToolClassification::Untracked),
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
    fn scope_id_embeds_attempt_seq_and_is_length_prefixed() {
        let k = key("01a091f4-session", "call_1|fc_1");
        let scope_id = format_pi_scope_id(&k, 1);
        assert_eq!(
            scope_id,
            "pi-tool-v1|n=1|s=16:01a091f4-session|c=11:call_1|fc_1"
        );
        assert_ne!(format_pi_scope_id(&k, 1), format_pi_scope_id(&k, 2));
        assert_eq!(
            pi_scope_start_event_id(&scope_id),
            format!("{scope_id}|start")
        );
        assert_eq!(
            pi_scope_close_event_id(&scope_id),
            format!("{scope_id}|close")
        );
        assert_ne!(
            pi_scope_start_event_id(&scope_id),
            pi_scope_close_event_id(&scope_id)
        );
    }

    #[test]
    fn length_prefix_disambiguates_delimiter_collisions() {
        let a = key("s|c=1:x", "y");
        let b = key("s", "1:x|y");
        assert_ne!(format_pi_scope_id(&a, 1), format_pi_scope_id(&b, 1));
    }

    #[test]
    fn provenance_canonicalizes_the_session_and_normalizes_the_model() {
        let provenance = pi_scope_provenance("01a091f4-session", Some("openai-codex/gpt-5.5"));
        assert_eq!(provenance.session_id, "pi_01a091f4-session");
        assert_eq!(provenance.model_id.as_deref(), Some("openai-codex/gpt-5.5"));
    }

    #[test]
    fn provenance_keeps_an_already_prefixed_session_id() {
        let provenance = pi_scope_provenance("pi_01a091f4-session", None);
        assert_eq!(provenance.session_id, "pi_01a091f4-session");
    }

    #[test]
    fn provenance_without_model_evidence_is_null() {
        for model in [None, Some(""), Some("   ")] {
            let provenance = pi_scope_provenance("01a091f4-session", model);
            assert_eq!(provenance.model_id, None, "model {model:?}");
        }
    }

    #[test]
    fn run_from_payload_fails_closed_when_a_tracked_start_cannot_resolve_its_checkout() {
        let payload = tool_event_json(
            HOOK_EVENT_TOOL_CALL,
            &[(
                CWD_FIELD,
                Value::String("/nonexistent/sce/pi/checkout".to_string()),
            )],
        );
        let error = run_pi_mutation_scope_from_payload(&payload, None)
            .expect_err("a tracked Start that cannot resolve its checkout must fail closed");
        assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE), "{error:?}");
    }

    #[test]
    fn run_from_payload_is_neutral_for_untracked_events() {
        for tool_name in ["read", "grep", "find", "ls", "probe_mutate"] {
            let payload = tool_event_json(
                HOOK_EVENT_TOOL_CALL,
                &[(TOOL_NAME_FIELD, Value::String(tool_name.to_string()))],
            );
            assert_eq!(
                run_pi_mutation_scope_from_payload(&payload, None).unwrap(),
                String::new()
            );
        }
    }

    #[test]
    fn run_from_payload_surfaces_malformed_input() {
        let error = run_pi_mutation_scope_from_payload("{bad", None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("expected valid JSON"), "{error:?}");
    }

    #[test]
    fn tool_execution_start_is_always_a_no_op_regardless_of_classification() {
        for tool_name in ["bash", "read"] {
            let payload = tool_event_json(
                HOOK_EVENT_TOOL_EXECUTION_START,
                &[(TOOL_NAME_FIELD, Value::String(tool_name.to_string()))],
            );
            assert_eq!(
                run_pi_mutation_scope_from_payload(&payload, None).unwrap(),
                String::new()
            );
        }
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    use serde_json::Value;

    use super::state::{read_state, AdapterAttempt, AdapterState, AttemptPhase, RecoveryState};
    use super::*;

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    fn temp_git_dir(label: &str) -> PathBuf {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "sce-pi-mutation-scope-lifecycle-{label}-{}-{id}",
            std::process::id()
        ))
    }

    const CWD: &str = "/repo/pi-checkout";

    struct RecordingSeam {
        calls: Mutex<Vec<String>>,
        fail_operations: Vec<String>,
        fail_once_operations: Mutex<Vec<String>>,
    }

    impl RecordingSeam {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail_operations: Vec::new(),
                fail_once_operations: Mutex::new(Vec::new()),
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

        fn handle(&self, payload: &str) -> Result<String> {
            let operation = operation_of(payload);
            {
                let mut calls = self.calls.lock().expect("seam mutex");
                calls.push(operation.clone());
            }
            if self.fail_operations.contains(&operation) {
                bail!("seam failure injected by test for '{operation}'");
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
        run_pi_mutation_scope_from_payload_with_seams(payload, None, &resolver, &seam_fn)
    }

    fn tool_call_event(tool_name: &str, tool_call_id: &str) -> String {
        json!({
            "hook_event_name": "ToolCall",
            "session_id": "ses-main",
            "tool_call_id": tool_call_id,
            "cwd": CWD,
            "tool_name": tool_name,
            "model": "openai-codex/gpt-5.5",
        })
        .to_string()
    }

    fn tool_result_event(tool_name: &str, tool_call_id: &str) -> String {
        json!({
            "hook_event_name": "ToolResult",
            "session_id": "ses-main",
            "tool_call_id": tool_call_id,
            "cwd": CWD,
            "tool_name": tool_name,
        })
        .to_string()
    }

    fn tool_execution_end_event(tool_name: &str, tool_call_id: &str) -> String {
        json!({
            "hook_event_name": "ToolExecutionEnd",
            "session_id": "ses-main",
            "tool_call_id": tool_call_id,
            "cwd": CWD,
            "tool_name": tool_name,
        })
        .to_string()
    }

    fn cleanup(git_dir: &Path) {
        let _ = std::fs::remove_dir_all(git_dir);
    }

    fn tool_call_event_for_session(
        tool_name: &str,
        session_id: &str,
        tool_call_id: &str,
    ) -> String {
        json!({
            "hook_event_name": "ToolCall",
            "session_id": session_id,
            "tool_call_id": tool_call_id,
            "cwd": CWD,
            "tool_name": tool_name,
            "model": "openai-codex/gpt-5.5",
        })
        .to_string()
    }

    fn tool_result_event_for_session(
        tool_name: &str,
        session_id: &str,
        tool_call_id: &str,
    ) -> String {
        json!({
            "hook_event_name": "ToolResult",
            "session_id": session_id,
            "tool_call_id": tool_call_id,
            "cwd": CWD,
            "tool_name": tool_name,
        })
        .to_string()
    }

    fn dead_process_owner() -> super::process_owner::ProcessOwner {
        let mut dead_child = std::process::Command::new("true")
            .spawn()
            .expect("spawning 'true' should succeed");
        let dead_pid = i32::try_from(dead_child.id()).expect("pid fits in i32");
        dead_child.wait().expect("child should exit and be reaped");
        super::process_owner::ProcessOwner {
            pid: dead_pid,
            instance_token: None,
        }
    }

    fn attempt_owned_by(state: &AdapterState, session_id: &str) -> AdapterAttempt {
        state
            .attempts
            .iter()
            .find(|attempt| attempt.session_id == session_id)
            .expect("attempt for session must exist")
            .clone()
    }

    #[test]
    fn tool_call_establishes_a_write_ahead_start_and_replays_idempotently() {
        let git_dir = temp_git_dir("write-ahead-start");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_call_event("write", "call_1")).expect("first Start");
        drive(&git_dir, &seam, &tool_call_event("write", "call_1"))
            .expect("duplicate Start is idempotent");

        assert_eq!(seam.operations(), vec!["start", "start"]);
        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].phase, AttemptPhase::PendingStart);
        assert_eq!(state.attempts[0].tool_name, "write");

        cleanup(&git_dir);
    }

    #[test]
    fn concurrent_bash_calls_in_one_session_stay_separate_live_scopes() {
        let git_dir = temp_git_dir("concurrent-bash");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_call_event("bash", "call_a")).expect("A Start");
        drive(&git_dir, &seam, &tool_call_event("bash", "call_b"))
            .expect("B Start must not retire A");

        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 2);
        assert!(state
            .attempts
            .iter()
            .all(|attempt| attempt.phase == AttemptPhase::PendingStart));

        cleanup(&git_dir);
    }

    #[test]
    fn full_success_lifecycle_start_result_close() {
        let git_dir = temp_git_dir("success-lifecycle");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("Start");
        assert_eq!(
            read_state(&git_dir).expect("state readable").attempts[0].phase,
            AttemptPhase::PendingStart
        );

        drive(&git_dir, &seam, &tool_result_event("bash", "call_1")).expect("tool_result");
        assert_eq!(
            read_state(&git_dir).expect("state readable").attempts[0].phase,
            AttemptPhase::Executed,
            "D5/D6: tool_result is the sole Executed-transition evidence"
        );

        drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_1"))
            .expect("tool_execution_end closes an Executed attempt");
        assert!(read_state(&git_dir)
            .expect("state readable")
            .attempts
            .is_empty());
        assert_eq!(seam.operations(), vec!["start", "close"]);

        cleanup(&git_dir);
    }

    #[test]
    fn tool_execution_end_without_a_preceding_tool_result_abandons_never_closes() {
        let git_dir = temp_git_dir("d7-abandon");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("Start");

        drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_1"))
            .expect("D7: a terminal event with no preceding tool_result must abandon");

        assert!(read_state(&git_dir)
            .expect("state readable")
            .attempts
            .is_empty());
        assert!(
            !seam.operations().contains(&"close".to_string()),
            "D7: an unexecuted attempt must never be closed"
        );
        assert!(seam.operations().contains(&"abandon".to_string()));
        assert!(read_state(&git_dir)
            .expect("state readable")
            .recovery
            .is_clear());

        cleanup(&git_dir);
    }

    #[test]
    fn a_failed_close_falls_back_to_abandon_recovery() {
        let git_dir = temp_git_dir("close-failure-falls-back");
        let seam = RecordingSeam::failing_on(&["close"]);

        drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("Start");
        drive(&git_dir, &seam, &tool_result_event("bash", "call_1")).expect("tool_result");

        drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_1"))
            .expect("a Close failure must recover via abandon, not surface an error");

        assert!(read_state(&git_dir)
            .expect("state readable")
            .attempts
            .is_empty());
        assert!(seam.operations().contains(&"abandon".to_string()));

        cleanup(&git_dir);
    }

    #[test]
    fn a_terminal_recovery_flush_failure_leaves_a_pending_recovery_and_denies_new_admission() {
        let git_dir = temp_git_dir("recovery-flush-failure");
        let persistently_failing = RecordingSeam::failing_on(&["flush"]);

        drive(
            &git_dir,
            &persistently_failing,
            &tool_call_event("bash", "call_1"),
        )
        .expect("Start");
        drive(
            &git_dir,
            &persistently_failing,
            &tool_execution_end_event("bash", "call_1"),
        )
        .expect("abandon path swallows the flush failure rather than surfacing an error");

        assert_eq!(
            read_state(&git_dir).expect("state readable").recovery,
            RecoveryState::Pending { generation: 1 },
            "a failed ambiguity flush must leave recovery Pending, not Clear"
        );

        let error = drive(
            &git_dir,
            &persistently_failing,
            &tool_call_event("bash", "call_2"),
        )
        .expect_err("a new admission must fail closed while recovery remains unresolved");
        assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE));

        let recovered = RecordingSeam::new();
        drive(&git_dir, &recovered, &tool_call_event("bash", "call_3"))
            .expect("a new admission must self-heal once recovery can complete");
        assert!(read_state(&git_dir)
            .expect("state readable")
            .recovery
            .is_clear());
        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].tool_call_id, "call_3");

        cleanup(&git_dir);
    }

    #[test]
    fn start_provenance_carries_the_prefixed_session_and_normalized_model_to_the_seam() {
        let git_dir = temp_git_dir("provenance-present");
        let captured: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let resolver = |_cwd: &str| Ok(git_dir.clone());
        let seam_fn = |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| {
            captured
                .lock()
                .expect("capture mutex")
                .push(payload.to_string());
            Ok(String::new())
        };

        run_pi_mutation_scope_from_payload_with_seams(
            &tool_call_event("bash", "call_model"),
            None,
            &resolver,
            &seam_fn,
        )
        .expect("Start should succeed");

        let payloads = captured.into_inner().expect("capture mutex");
        assert_eq!(payloads.len(), 1);
        let sent: Value = serde_json::from_str(&payloads[0]).expect("seam payload is JSON");
        assert_eq!(
            sent["provenance"]["session_id"].as_str(),
            Some("pi_ses-main")
        );
        assert_eq!(
            sent["provenance"]["model_id"].as_str(),
            Some("openai-codex/gpt-5.5")
        );

        cleanup(&git_dir);
    }

    #[test]
    fn start_provenance_is_null_model_when_the_event_carries_no_model() {
        let git_dir = temp_git_dir("provenance-absent");
        let captured: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let resolver = |_cwd: &str| Ok(git_dir.clone());
        let seam_fn = |_root: &Path, payload: &str, _logger: Option<&dyn Logger>| {
            captured
                .lock()
                .expect("capture mutex")
                .push(payload.to_string());
            Ok(String::new())
        };

        let payload = json!({
            "hook_event_name": "ToolCall",
            "session_id": "ses-main",
            "tool_call_id": "call_no_model",
            "cwd": CWD,
            "tool_name": "bash",
        })
        .to_string();

        run_pi_mutation_scope_from_payload_with_seams(&payload, None, &resolver, &seam_fn)
            .expect("Start should succeed");

        let payloads = captured.into_inner().expect("capture mutex");
        let sent: Value = serde_json::from_str(&payloads[0]).expect("seam payload is JSON");
        assert!(sent["provenance"]["model_id"].is_null());

        cleanup(&git_dir);
    }

    #[test]
    fn untracked_tool_call_never_admits_an_attempt() {
        let git_dir = temp_git_dir("untracked-no-admit");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_call_event("read", "call_ro")).expect("untracked is inert");
        assert!(seam.operations().is_empty());
        assert!(read_state(&git_dir)
            .expect("state readable")
            .attempts
            .is_empty());

        drive(&git_dir, &seam, &tool_result_event("read", "call_ro"))
            .expect("untracked result inert");
        drive(
            &git_dir,
            &seam,
            &tool_execution_end_event("read", "call_ro"),
        )
        .expect("untracked terminal inert");
        assert!(seam.operations().is_empty());

        cleanup(&git_dir);
    }

    #[test]
    fn a_pending_start_attempt_owned_by_a_dead_process_is_abandoned_not_replayed() {
        let git_dir = temp_git_dir("d10-dead-owner-abandon");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("first Start");
        let scope_id = read_state(&git_dir).expect("state readable").attempts[0]
            .scope_id
            .clone();

        let mut dead_child = std::process::Command::new("true")
            .spawn()
            .expect("spawning 'true' should succeed");
        let dead_pid = i32::try_from(dead_child.id()).expect("pid fits in i32");
        dead_child.wait().expect("child should exit and be reaped");
        state::set_attempt_owner_for_tests(
            &git_dir,
            &scope_id,
            super::process_owner::ProcessOwner {
                pid: dead_pid,
                instance_token: None,
            },
        );

        drive(&git_dir, &seam, &tool_call_event("bash", "call_1"))
            .expect("a replay whose recorded owner is positively dead must abandon, not reuse");

        assert_eq!(
            seam.operations(),
            vec!["start", "flush", "abandon", "flush", "start"],
            "D10: a dead-owner PendingStart must be abandoned via the existing D8 flush/abandon/\
             flush pattern, then the triggering event admitted as a fresh attempt"
        );
        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_ne!(
            state.attempts[0].scope_id, scope_id,
            "the fresh attempt must never reuse the abandoned attempt's ScopeId"
        );
        assert_eq!(state.attempts[0].attempt_seq, 2);
        assert!(state.recovery.is_clear());

        cleanup(&git_dir);
    }

    #[test]
    fn a_pending_start_attempt_owned_by_a_live_process_is_never_abandoned_by_a_replay() {
        let git_dir = temp_git_dir("d10-live-owner-no-abandon");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("first Start");
        drive(&git_dir, &seam, &tool_call_event("bash", "call_1"))
            .expect("a replay owned by a still-live process must be treated as a normal replay");

        assert_eq!(
            seam.operations(),
            vec!["start", "start"],
            "no TTL and no elapsed time may ever cause an abandon here: the owner is this test \
             process's own live parent for the whole test"
        );
        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].phase, AttemptPhase::PendingStart);

        cleanup(&git_dir);
    }

    #[test]
    fn a_dead_pending_start_attempt_is_recovered_by_an_unrelated_fresh_session_start() {
        let git_dir = temp_git_dir("d10-fresh-session-dead-pending-start");
        let seam = RecordingSeam::new();

        drive(
            &git_dir,
            &seam,
            &tool_call_event_for_session("bash", "sess-a", "call-a"),
        )
        .expect("A's Start");
        let scope_a =
            attempt_owned_by(&read_state(&git_dir).expect("state readable"), "sess-a").scope_id;
        state::set_attempt_owner_for_tests(&git_dir, &scope_a, dead_process_owner());

        drive(
            &git_dir,
            &seam,
            &tool_call_event_for_session("bash", "sess-b", "call-b"),
        )
        .expect(
            "B's Start must recover A's stale owner without ever replaying A's \
             (session_id, tool_call_id) key",
        );

        assert_eq!(
            seam.operations(),
            vec!["start", "flush", "abandon", "flush", "start"],
            "D10: a dead PendingStart owner discovered by an unrelated fresh-session Start must \
             be retired through the existing D8 flush/abandon/flush sequence before the \
             triggering Start is admitted"
        );
        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].session_id, "sess-b");
        assert_ne!(state.attempts[0].scope_id, scope_a);
        assert!(state.recovery.is_clear());

        cleanup(&git_dir);
    }

    #[test]
    fn a_dead_executed_attempt_is_recovered_by_a_fresh_session_start_without_a_synthetic_close() {
        let git_dir = temp_git_dir("d10-fresh-session-dead-executed");
        let seam = RecordingSeam::new();

        drive(
            &git_dir,
            &seam,
            &tool_call_event_for_session("bash", "sess-a", "call-a"),
        )
        .expect("A's Start");
        drive(
            &git_dir,
            &seam,
            &tool_result_event_for_session("bash", "sess-a", "call-a"),
        )
        .expect("A's tool_result marks Executed");
        let state = read_state(&git_dir).expect("state readable");
        let scope_a = attempt_owned_by(&state, "sess-a").scope_id;
        assert_eq!(
            attempt_owned_by(&state, "sess-a").phase,
            AttemptPhase::Executed
        );
        state::set_attempt_owner_for_tests(&git_dir, &scope_a, dead_process_owner());

        drive(
            &git_dir,
            &seam,
            &tool_call_event_for_session("bash", "sess-b", "call-b"),
        )
        .expect("B's Start must recover A's dead Executed attempt");

        assert_eq!(
            seam.operations(),
            vec!["start", "flush", "abandon", "flush", "start"],
            "a dead Executed attempt must be abandoned/rebaselined via D8, never given a \
             synthetic delayed Close"
        );
        assert!(
            !seam.operations().contains(&"close".to_string()),
            "D9: the current Git tree no longer represents the original terminal observation \
             time, so a dead Executed attempt must never be closed"
        );
        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].session_id, "sess-b");
        assert!(state.recovery.is_clear());

        cleanup(&git_dir);
    }

    #[test]
    fn a_dead_owner_scope_is_recovered_while_a_live_owner_sibling_survives_untouched() {
        let git_dir = temp_git_dir("d10-dead-live-sibling-isolation");
        let seam = RecordingSeam::new();

        drive(
            &git_dir,
            &seam,
            &tool_call_event_for_session("bash", "sess-a", "call-a"),
        )
        .expect("A's Start (owner will die)");
        drive(
            &git_dir,
            &seam,
            &tool_call_event_for_session("bash", "sess-b", "call-b"),
        )
        .expect("B's Start (owner stays live)");

        let state = read_state(&git_dir).expect("state readable");
        let scope_a = attempt_owned_by(&state, "sess-a").scope_id;
        let scope_b = attempt_owned_by(&state, "sess-b").scope_id;
        state::set_attempt_owner_for_tests(&git_dir, &scope_a, dead_process_owner());

        drive(
            &git_dir,
            &seam,
            &tool_call_event_for_session("bash", "sess-c", "call-c"),
        )
        .expect("C's Start must recover only A");

        assert_eq!(
            seam.operations(),
            vec!["start", "start", "flush", "abandon", "flush", "start"],
            "exactly one abandon must occur, and only for A's own positively dead owner"
        );
        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 2);
        assert!(!state.attempts.iter().any(|a| a.scope_id == scope_a));
        let b = attempt_owned_by(&state, "sess-b");
        assert_eq!(b.scope_id, scope_b);
        assert_eq!(
            b.phase,
            AttemptPhase::PendingStart,
            "B must survive reconciliation exactly as it was, untouched"
        );
        assert_eq!(
            attempt_owned_by(&state, "sess-c").phase,
            AttemptPhase::PendingStart
        );

        cleanup(&git_dir);
    }

    #[test]
    fn multiple_dead_owner_scopes_are_retired_in_one_recovery_generation_while_a_live_sibling_survives(
    ) {
        let git_dir = temp_git_dir("d10-multiple-dead-owner-scopes");
        let seam = RecordingSeam::new();

        drive(
            &git_dir,
            &seam,
            &tool_call_event_for_session("bash", "sess-a", "call-a"),
        )
        .expect("A's Start");
        drive(
            &git_dir,
            &seam,
            &tool_result_event_for_session("bash", "sess-a", "call-a"),
        )
        .expect("A's tool_result marks Executed");
        drive(
            &git_dir,
            &seam,
            &tool_call_event_for_session("bash", "sess-b", "call-b"),
        )
        .expect("B's Start");
        drive(
            &git_dir,
            &seam,
            &tool_call_event_for_session("bash", "sess-q", "call-q"),
        )
        .expect("Q's Start (owner stays live)");

        let state = read_state(&git_dir).expect("state readable");
        let scope_a = attempt_owned_by(&state, "sess-a").scope_id;
        let scope_b = attempt_owned_by(&state, "sess-b").scope_id;
        let scope_q = attempt_owned_by(&state, "sess-q").scope_id;
        let dead_owner = dead_process_owner();
        state::set_attempt_owner_for_tests(&git_dir, &scope_a, dead_owner);
        state::set_attempt_owner_for_tests(&git_dir, &scope_b, dead_owner);

        drive(
            &git_dir,
            &seam,
            &tool_call_event_for_session("bash", "sess-c", "call-c"),
        )
        .expect("C's Start must recover both A and B, grouped into one recovery generation");

        assert_eq!(
            seam.operations(),
            vec!["start", "start", "start", "flush", "abandon", "abandon", "flush", "start"],
            "a single flush/abandon.../flush recovery generation must retire every \
             independently-proven-dead scope owned by the same dead process together"
        );
        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 2);
        assert!(!state.attempts.iter().any(|a| a.scope_id == scope_a));
        assert!(!state.attempts.iter().any(|a| a.scope_id == scope_b));
        assert_eq!(attempt_owned_by(&state, "sess-q").scope_id, scope_q);
        assert!(state.recovery.is_clear());

        cleanup(&git_dir);
    }

    #[test]
    fn an_owner_that_cannot_be_positively_proven_dead_is_never_abandoned_by_an_unrelated_start() {
        let git_dir = temp_git_dir("d10-uncertain-owner-preserved");
        let seam = RecordingSeam::new();

        drive(
            &git_dir,
            &seam,
            &tool_call_event_for_session("bash", "sess-a", "call-a"),
        )
        .expect("A's Start");
        let scope_a =
            attempt_owned_by(&read_state(&git_dir).expect("state readable"), "sess-a").scope_id;
        state::set_attempt_owner_for_tests(
            &git_dir,
            &scope_a,
            super::process_owner::ProcessOwner {
                pid: std::process::id().cast_signed(),
                instance_token: None,
            },
        );

        drive(
            &git_dir,
            &seam,
            &tool_call_event_for_session("bash", "sess-c", "call-c"),
        )
        .expect(
            "C's Start must proceed without touching A, whose owner cannot be positively \
             proven dead",
        );

        assert_eq!(
            seam.operations(),
            vec!["start", "start"],
            "a live pid with no instance-token evidence must never be converted into proof of \
             death: uncertain identity is conservatively treated as alive"
        );
        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 2);
        assert!(state
            .attempts
            .iter()
            .all(|attempt| attempt.phase == AttemptPhase::PendingStart));
        assert!(state.attempts.iter().any(|a| a.scope_id == scope_a));

        cleanup(&git_dir);
    }

    #[test]
    fn an_interrupted_stale_owner_recovery_remains_pending_and_denies_the_triggering_start_until_resumed(
    ) {
        let git_dir = temp_git_dir("d10-interrupted-stale-recovery");
        let seam = RecordingSeam::new();

        drive(
            &git_dir,
            &seam,
            &tool_call_event_for_session("bash", "sess-a", "call-a"),
        )
        .expect("A's Start");
        let scope_a =
            attempt_owned_by(&read_state(&git_dir).expect("state readable"), "sess-a").scope_id;
        state::set_attempt_owner_for_tests(&git_dir, &scope_a, dead_process_owner());

        let crashing = RecordingSeam::failing_once_on(&["abandon"]);
        let error = drive(
            &git_dir,
            &crashing,
            &tool_call_event_for_session("bash", "sess-b", "call-b"),
        )
        .expect_err(
            "a Start that triggers a stale-owner recovery which fails mid-way must not commit",
        );
        assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE));

        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.recovery, RecoveryState::Pending { generation: 1 });
        assert!(
            !state.attempts.iter().any(|a| a.session_id == "sess-b"),
            "B must never be admitted while A's stale-owner recovery is still pending"
        );
        assert_eq!(
            attempt_owned_by(&state, "sess-a").phase,
            AttemptPhase::PendingAbandon
        );

        drive(
            &git_dir,
            &crashing,
            &tool_call_event_for_session("bash", "sess-b", "call-b"),
        )
        .expect(
            "the next boundary-lock acquisition must resume and complete the pending recovery, \
             and only then admit B",
        );

        let state = read_state(&git_dir).expect("state readable");
        assert!(state.recovery.is_clear());
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].session_id, "sess-b");
        assert!(!state.attempts.iter().any(|a| a.scope_id == scope_a));

        cleanup(&git_dir);
    }

    #[test]
    fn duplicate_tool_result_after_close_is_a_safe_no_op() {
        let git_dir = temp_git_dir("duplicate-tool-result-after-close");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("Start");
        drive(&git_dir, &seam, &tool_result_event("bash", "call_1")).expect("tool_result");
        drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_1")).expect("Close");

        drive(&git_dir, &seam, &tool_result_event("bash", "call_1"))
            .expect("a late duplicate tool_result after Close must be a safe no-op");
        drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_1"))
            .expect("a late duplicate tool_execution_end after Close must be a safe no-op");

        assert_eq!(
            seam.operations(),
            vec!["start", "close"],
            "a resurrected attempt must never re-enter the runtime seam after its own Close"
        );
        assert!(read_state(&git_dir)
            .expect("state readable")
            .attempts
            .is_empty());

        cleanup(&git_dir);
    }

    #[test]
    fn duplicate_tool_execution_end_after_abandon_is_a_safe_no_op() {
        let git_dir = temp_git_dir("duplicate-terminal-after-abandon");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("Start");
        drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_1")).expect("D7 abandon");

        drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_1"))
            .expect("a late duplicate terminal event after abandon must be a safe no-op");

        assert_eq!(
            seam.operations(),
            vec!["start", "flush", "abandon", "flush"],
            "a duplicate terminal delivery for an already-abandoned attempt must never issue a \
             second abandon"
        );

        cleanup(&git_dir);
    }

    #[test]
    fn abandoning_one_sibling_never_touches_a_concurrent_sibling_in_the_same_session() {
        let git_dir = temp_git_dir("sibling-abandon-isolation");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_call_event("bash", "call_a")).expect("A Start");
        drive(&git_dir, &seam, &tool_call_event("bash", "call_b")).expect("B Start");
        drive(&git_dir, &seam, &tool_result_event("bash", "call_b")).expect("B tool_result");

        drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_a"))
            .expect("A's terminal event with no tool_result must abandon only A");

        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(
            state.attempts.len(),
            1,
            "abandoning A must never remove or block sibling B"
        );
        assert_eq!(state.attempts[0].tool_call_id, "call_b");
        assert_eq!(state.attempts[0].phase, AttemptPhase::Executed);

        drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_b"))
            .expect("B must still close normally after A's abandonment and recovery");
        assert!(read_state(&git_dir)
            .expect("state readable")
            .attempts
            .is_empty());
        assert_eq!(
            seam.operations(),
            vec!["start", "start", "flush", "abandon", "flush", "close"]
        );

        cleanup(&git_dir);
    }

    #[test]
    fn a_crash_mid_abandon_loop_is_resumed_and_completed_on_the_next_boundary_lock_acquisition() {
        let git_dir = temp_git_dir("crash-mid-abandon-loop");
        let crashing = RecordingSeam::failing_once_on(&["abandon"]);

        drive(&git_dir, &crashing, &tool_call_event("bash", "call_1")).expect("Start");
        drive(
            &git_dir,
            &crashing,
            &tool_execution_end_event("bash", "call_1"),
        )
        .expect(
            "a transient abandon failure mid-recovery must leave recovery Pending, not surface \
             an error, simulating a crash between marking PendingAbandon and completing the \
             flush/abandon/flush sequence",
        );

        assert_eq!(
            read_state(&git_dir).expect("state readable").recovery,
            RecoveryState::Pending { generation: 1 },
            "the interrupted abandon loop must leave recovery durably Pending for the next \
             boundary-lock acquisition to resume, never Clear and never lost"
        );
        assert_eq!(
            read_state(&git_dir)
                .expect("state readable")
                .attempts
                .first()
                .expect("the doomed attempt must still be recorded")
                .phase,
            AttemptPhase::PendingAbandon
        );

        drive(&git_dir, &crashing, &tool_call_event("bash", "call_2"))
            .expect("recovery must self-heal and complete on the very next invocation");

        assert!(read_state(&git_dir)
            .expect("state readable")
            .recovery
            .is_clear());
        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].tool_call_id, "call_2");

        cleanup(&git_dir);
    }

    #[test]
    fn a_reused_tool_call_id_after_terminal_cleanup_gets_a_distinct_scope_id() {
        let git_dir = temp_git_dir("terminal-scope-id-non-reuse");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("first Start");
        drive(&git_dir, &seam, &tool_result_event("bash", "call_1")).expect("first tool_result");
        drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_1")).expect("first Close");
        assert!(read_state(&git_dir)
            .expect("state readable")
            .attempts
            .is_empty());

        drive(&git_dir, &seam, &tool_call_event("bash", "call_1"))
            .expect("reused toolCallId Start");
        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].attempt_seq, 2);

        cleanup(&git_dir);
    }
}

#[cfg(test)]
mod runtime_seam_tests {
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
    use crate::services::agent_trace_storage::{
        resolve_agent_trace_storage_at_state_root, AgentTraceStorageContext,
    };

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

    struct PiRepo {
        _temp: tempfile::TempDir,
        root: PathBuf,
        state_root: PathBuf,
    }

    impl PiRepo {
        fn new(label: &str) -> Self {
            let temp = tempfile::Builder::new()
                .prefix(&format!("sce-pi-mutation-scope-seam-{label}-"))
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
            run_pi_mutation_scope_from_payload_at_state_root(&self.state_root, payload, None)
        }

        fn write(&self, name: &str, contents: &str) {
            fs::write(self.root.join(name), contents).expect("write should succeed");
        }

        fn db(&self) -> RepositoryAgentTraceDb {
            crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                &self.root,
                &self.state_root,
                "Pi mutation-scope seam test assertions",
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

        fn scope_provenance(&self, scope_id: &str) -> Option<(String, Option<String>)> {
            self.db()
                .query_map(
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

    fn tool_call(repo: &PiRepo, tool_name: &str, tool_call_id: &str) -> String {
        json!({
            "hook_event_name": "ToolCall",
            "session_id": "01a091f4-seam-session",
            "tool_call_id": tool_call_id,
            "cwd": repo.cwd(),
            "tool_name": tool_name,
            "model": "openai-codex/gpt-5.5",
        })
        .to_string()
    }

    fn tool_result(repo: &PiRepo, tool_name: &str, tool_call_id: &str) -> String {
        json!({
            "hook_event_name": "ToolResult",
            "session_id": "01a091f4-seam-session",
            "tool_call_id": tool_call_id,
            "cwd": repo.cwd(),
            "tool_name": tool_name,
        })
        .to_string()
    }

    fn tool_execution_end(repo: &PiRepo, tool_name: &str, tool_call_id: &str) -> String {
        json!({
            "hook_event_name": "ToolExecutionEnd",
            "session_id": "01a091f4-seam-session",
            "tool_call_id": tool_call_id,
            "cwd": repo.cwd(),
            "tool_name": tool_name,
        })
        .to_string()
    }

    #[test]
    fn a_write_start_result_close_lands_a_real_ai_exclusive_event_with_pi_provenance() {
        let repo = PiRepo::new("real-lifecycle");
        let key = AttemptKey {
            session_id: "01a091f4-seam-session".to_string(),
            tool_call_id: "call_write".to_string(),
        };
        let scope_id = format_pi_scope_id(&key, 1);

        repo.drive(&tool_call(&repo, "write", "call_write"))
            .expect("Start should reach the real runtime");
        assert_eq!(
            repo.scope_status(&scope_id),
            Some(("pi".to_string(), "active".to_string()))
        );
        assert_eq!(
            repo.scope_provenance(&scope_id),
            Some((
                "pi_01a091f4-seam-session".to_string(),
                Some("openai-codex/gpt-5.5".to_string())
            ))
        );

        repo.write("file.txt", "one\ntwo\n");
        repo.drive(&tool_result(&repo, "write", "call_write"))
            .expect("tool_result should mark Executed");

        repo.drive(&tool_execution_end(&repo, "write", "call_write"))
            .expect("Close should reach the real runtime");

        assert_eq!(
            repo.scope_status(&scope_id),
            Some(("pi".to_string(), "closed".to_string()))
        );
        assert_eq!(
            repo.mutation_events(),
            vec![("ai_exclusive".to_string(), Some(scope_id))]
        );
        assert!(
            state::read_state(&resolve_git_dir(&repo.root).expect("git dir resolves"))
                .expect("state readable")
                .attempts
                .is_empty()
        );
    }

    #[test]
    fn a_start_followed_by_no_execution_abandons_through_the_real_runtime() {
        let repo = PiRepo::new("real-abandon");
        let key = AttemptKey {
            session_id: "01a091f4-seam-session".to_string(),
            tool_call_id: "call_blocked".to_string(),
        };
        let scope_id = format_pi_scope_id(&key, 1);

        repo.drive(&tool_call(&repo, "bash", "call_blocked"))
            .expect("Start should reach the real runtime");

        repo.drive(&tool_execution_end(&repo, "bash", "call_blocked"))
            .expect("the terminal event must resolve via abandon, not surface an error");

        assert_eq!(
            repo.scope_status(&scope_id),
            Some(("pi".to_string(), "abandoned".to_string()))
        );
        assert!(repo.mutation_events().is_empty());
    }
}

#[cfg(all(unix, test))]
mod guard_reconciliation_tests {
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::mpsc;
    use std::time::Duration;

    use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
    use crate::services::agent_trace_storage::{
        resolve_agent_trace_storage_at_state_root, AgentTraceStorageContext,
    };
    use crate::services::mutation_trace::runtime::{
        coordinate, run_external_mutation_guard, GuardRequest, RuntimeBoundary,
    };
    use crate::services::mutation_trace::types::{ActorKind, EventId, ScopeId};

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

    struct GuardRepo {
        _temp: tempfile::TempDir,
        root: PathBuf,
        state_root: PathBuf,
    }

    impl GuardRepo {
        fn new(label: &str) -> Self {
            let temp = tempfile::Builder::new()
                .prefix(&format!("sce-pi-guard-reconciliation-{label}-"))
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

        fn drive(&self, payload: &str) -> Result<String> {
            run_pi_mutation_scope_from_payload_at_state_root(&self.state_root, payload, None)
        }

        fn open_db(&self) -> anyhow::Result<RepositoryAgentTraceDb> {
            crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                &self.root,
                &self.state_root,
                "Pi guard-reconciliation test assertions",
            )
        }

        fn db(&self) -> RepositoryAgentTraceDb {
            self.open_db().expect("assertion DB should open")
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
    }

    fn tool_call(repo: &GuardRepo, tool_call_id: &str, session_id: &str) -> String {
        json!({
            "hook_event_name": "ToolCall",
            "session_id": session_id,
            "tool_call_id": tool_call_id,
            "cwd": repo.root.to_string_lossy(),
            "tool_name": "bash",
            "model": "openai-codex/gpt-5.5",
        })
        .to_string()
    }

    fn tool_execution_end(repo: &GuardRepo, tool_call_id: &str, session_id: &str) -> String {
        json!({
            "hook_event_name": "ToolExecutionEnd",
            "session_id": session_id,
            "tool_call_id": tool_call_id,
            "cwd": repo.root.to_string_lossy(),
            "tool_name": "bash",
        })
        .to_string()
    }

    #[test]
    fn a_guard_triggered_worktree_abandonment_reconciles_with_the_pi_adapters_own_state() {
        let repo = GuardRepo::new("reconcile");
        let session = "01a091f4-guard-session";
        let key_a = AttemptKey {
            session_id: session.to_string(),
            tool_call_id: "call_a".to_string(),
        };
        let key_b = AttemptKey {
            session_id: session.to_string(),
            tool_call_id: "call_b".to_string(),
        };
        let scope_a = format_pi_scope_id(&key_a, 1);
        let scope_b = format_pi_scope_id(&key_b, 2);

        repo.drive(&tool_call(&repo, "call_a", session))
            .expect("A's Start should reach the real runtime");
        repo.drive(&tool_call(&repo, "call_b", session))
            .expect("B's Start should reach the real runtime");
        assert_eq!(
            repo.scope_status(&scope_a),
            Some(("pi".to_string(), "active".to_string()))
        );
        assert_eq!(
            repo.scope_status(&scope_b),
            Some(("pi".to_string(), "active".to_string()))
        );

        let (_cancel_tx, cancel_rx) = mpsc::channel();
        let root = repo.root.clone();
        let outcome = run_external_mutation_guard(
            &root,
            &GuardRequest {
                command: "printf changed >> file.txt".to_string(),
                cwd: None,
                env: Vec::new(),
            },
            || repo.open_db(),
            |_event| {},
            cancel_rx,
        )
        .expect("the guard should finish successfully");
        assert_eq!(outcome.exit_code, Some(0));
        assert!(!outcome.marker_clear_failed);

        assert_eq!(
            repo.scope_status(&scope_a),
            Some(("pi".to_string(), "abandoned".to_string())),
            "the guard's finish-time forced recovery must abandon every scope live during the \
             guarded interval, regardless of which harness's boundary happened to observe \
             user_bash"
        );
        assert_eq!(
            repo.scope_status(&scope_b),
            Some(("pi".to_string(), "abandoned".to_string()))
        );

        repo.drive(&tool_execution_end(&repo, "call_a", session))
            .expect(
                "the adapter's next interaction for an already-abandoned scope must reconcile \
             safely (falling back through the existing Close-failure-to-abandon path) rather \
             than erroring or resurrecting the scope",
            );
        repo.drive(&tool_execution_end(&repo, "call_b", session))
            .expect("the same reconciliation must hold for every sibling abandoned by the guard");

        assert!(
            state::read_state(&resolve_git_dir(&repo.root).expect("git dir resolves"))
                .expect("state readable")
                .attempts
                .is_empty(),
            "the Pi adapter's own durable local attempt state must converge to empty once it \
             observes the terminal event for a scope the generic runtime already abandoned out \
             from under it"
        );
    }

    #[test]
    fn a_guard_abandons_a_live_pi_scope_alongside_a_live_scope_from_another_harness() {
        let repo = GuardRepo::new("cross-harness");
        let key = AttemptKey {
            session_id: "01a091f4-guard-cross-session".to_string(),
            tool_call_id: "call_pi".to_string(),
        };
        let pi_scope_id = format_pi_scope_id(&key, 1);
        let claude_scope = ScopeId("claude-scope-under-guard".to_string());

        repo.drive(&tool_call(&repo, "call_pi", "01a091f4-guard-cross-session"))
            .expect("Pi's Start should reach the real runtime");
        coordinate(
            &repo.root,
            &RuntimeBoundary::Start {
                scope: claude_scope.clone(),
                event: EventId("claude-evt-start".to_string()),
                actor_kind: ActorKind::ClaudeCode,
                provenance: None,
            },
            || repo.open_db(),
        )
        .expect("Claude's Start should reach the real runtime");

        assert_eq!(
            repo.scope_status(&pi_scope_id),
            Some(("pi".to_string(), "active".to_string()))
        );
        assert_eq!(
            repo.scope_status(&claude_scope.0),
            Some(("claude_code".to_string(), "active".to_string()))
        );

        let (_cancel_tx, cancel_rx) = mpsc::channel();
        let root = repo.root.clone();
        run_external_mutation_guard(
            &root,
            &GuardRequest {
                command: "printf changed >> file.txt".to_string(),
                cwd: None,
                env: Vec::new(),
            },
            || repo.open_db(),
            |_event| {},
            cancel_rx,
        )
        .expect("the guard should finish successfully");

        assert_eq!(
            repo.scope_status(&pi_scope_id),
            Some(("pi".to_string(), "abandoned".to_string())),
            "the guard's forced recovery must abandon the Pi scope even though a different \
             harness's boundary is the one that happened to observe user_bash"
        );
        assert_eq!(
            repo.scope_status(&claude_scope.0),
            Some(("claude_code".to_string(), "abandoned".to_string())),
            "the guard's forced recovery must abandon every live scope on the worktree \
             regardless of which harness owns it"
        );

        repo.drive(&tool_execution_end(
            &repo,
            "call_pi",
            "01a091f4-guard-cross-session",
        ))
        .expect(
            "the Pi adapter must still reconcile cleanly with its own scope even when a \
                 sibling scope belonging to a different harness was abandoned by the same guard",
        );
        assert!(
            state::read_state(&resolve_git_dir(&repo.root).expect("git dir resolves"))
                .expect("state readable")
                .attempts
                .is_empty()
        );
    }

    #[test]
    fn a_foreign_pi_start_racing_an_active_guard_fails_closed_touching_no_state_then_succeeds_on_retry(
    ) {
        let repo = GuardRepo::new("race");
        let ready = repo.root.join("ready");
        let release = repo.root.join("release");
        let command = format!(
            "touch '{}'; while [ ! -f '{}' ]; do sleep 0.02; done",
            ready.display(),
            release.display(),
        );

        let root = repo.root.clone();
        let state_root = repo.state_root.clone();
        let guard_thread = std::thread::spawn(move || {
            let (_cancel_tx, cancel_rx) = mpsc::channel();
            run_external_mutation_guard(
                &root,
                &GuardRequest {
                    command,
                    cwd: None,
                    env: Vec::new(),
                },
                || {
                    crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                        &root,
                        &state_root,
                        "guard race test",
                    )
                },
                |_event| {},
                cancel_rx,
            )
        });

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "the guarded shell never reported ready"
            );
            std::thread::sleep(Duration::from_millis(20));
        }

        let key = AttemptKey {
            session_id: "01a091f4-guard-race-session".to_string(),
            tool_call_id: "call_race".to_string(),
        };
        let scope_id = format_pi_scope_id(&key, 1);

        let error = repo
            .drive(&tool_call(
                &repo,
                "call_race",
                "01a091f4-guard-race-session",
            ))
            .expect_err(
                "a Pi Start racing an active external-mutation guard must block then fail \
                 closed with CoordinateError::LockAcquisition, never proceed",
            );
        assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE));
        assert!(
            repo.scope_status(&scope_id).is_none(),
            "a boundary that fails closed on lock acquisition must touch no protocol state"
        );

        fs::write(&release, "go").expect("release file should write");
        let outcome = guard_thread
            .join()
            .expect("guard thread should not panic")
            .expect("the guard should finish successfully once released");
        assert_eq!(outcome.exit_code, Some(0));

        repo.drive(&tool_call(
            &repo,
            "call_race",
            "01a091f4-guard-race-session",
        ))
        .expect("retrying the same Start after the guard finishes must succeed normally");
        assert_eq!(
            repo.scope_status(&scope_id),
            Some(("pi".to_string(), "active".to_string()))
        );
    }
}
