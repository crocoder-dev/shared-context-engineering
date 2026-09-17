#![allow(dead_code)]

mod boundary_lock;
mod os_lock;
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
        HOOK_EVENT_TOOL_EXECUTION_END => {
            parse_tool_identity(object).map(PiHookEvent::ExecutionEnd)
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
                establish_start(&git_dir, repository_root, &allocated, provenance, logger, seam)?;
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
    _git_dir: &Path,
    repository_root: &Path,
    allocated: &state::AllocatedAttempt,
    provenance: &PiScopeProvenance,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<()> {
    let scope_id = &allocated.attempt.scope_id;

    let start_payload = scope_start_payload(scope_id, &pi_scope_start_event_id(scope_id), provenance);

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
        .find(|attempt| attempt.session_id == key.session_id && attempt.tool_call_id == key.tool_call_id)
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
                PiHookEvent::Executed(identity) | PiHookEvent::ExecutionEnd(identity) => {
                    identity
                }
                other => panic!("expected a minimal-identity event, got {other:?}"),
            };
            assert_eq!(identity.attempt_key(), key("01a091f4-session", "call_1|fc_1"));
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

    use super::state::{read_state, AttemptPhase, RecoveryState};
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
        drive(&git_dir, &seam, &tool_call_event("bash", "call_b")).expect("B Start must not retire A");

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
        assert!(read_state(&git_dir).expect("state readable").attempts.is_empty());
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

        assert!(read_state(&git_dir).expect("state readable").attempts.is_empty());
        assert!(
            !seam.operations().contains(&"close".to_string()),
            "D7: an unexecuted attempt must never be closed"
        );
        assert!(seam.operations().contains(&"abandon".to_string()));
        assert!(read_state(&git_dir).expect("state readable").recovery.is_clear());

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

        assert!(read_state(&git_dir).expect("state readable").attempts.is_empty());
        assert!(seam.operations().contains(&"abandon".to_string()));

        cleanup(&git_dir);
    }

    #[test]
    fn a_terminal_recovery_flush_failure_leaves_a_pending_recovery_and_denies_new_admission() {
        let git_dir = temp_git_dir("recovery-flush-failure");
        let persistently_failing = RecordingSeam::failing_on(&["flush"]);

        drive(&git_dir, &persistently_failing, &tool_call_event("bash", "call_1")).expect("Start");
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

        let error = drive(&git_dir, &persistently_failing, &tool_call_event("bash", "call_2"))
            .expect_err("a new admission must fail closed while recovery remains unresolved");
        assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE));

        let recovered = RecordingSeam::new();
        drive(&git_dir, &recovered, &tool_call_event("bash", "call_3"))
            .expect("a new admission must self-heal once recovery can complete");
        assert!(read_state(&git_dir).expect("state readable").recovery.is_clear());
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
            captured.lock().expect("capture mutex").push(payload.to_string());
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
            captured.lock().expect("capture mutex").push(payload.to_string());
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
        assert!(read_state(&git_dir).expect("state readable").attempts.is_empty());

        drive(&git_dir, &seam, &tool_result_event("read", "call_ro")).expect("untracked result inert");
        drive(&git_dir, &seam, &tool_execution_end_event("read", "call_ro"))
            .expect("untracked terminal inert");
        assert!(seam.operations().is_empty());

        cleanup(&git_dir);
    }

    #[test]
    fn a_reused_tool_call_id_after_terminal_cleanup_gets_a_distinct_scope_id() {
        let git_dir = temp_git_dir("terminal-scope-id-non-reuse");
        let seam = RecordingSeam::new();

        drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("first Start");
        drive(&git_dir, &seam, &tool_result_event("bash", "call_1")).expect("first tool_result");
        drive(&git_dir, &seam, &tool_execution_end_event("bash", "call_1")).expect("first Close");
        assert!(read_state(&git_dir).expect("state readable").attempts.is_empty());

        drive(&git_dir, &seam, &tool_call_event("bash", "call_1")).expect("reused toolCallId Start");
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
