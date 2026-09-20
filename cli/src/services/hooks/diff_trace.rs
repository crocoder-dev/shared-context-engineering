use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::Value;

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_db::{
    ClaudeModelStateObservation, DiffTraceInsert, ObservationKind, PAYLOAD_TYPE_PATCH,
    PAYLOAD_TYPE_STRUCTURED,
};
use crate::services::observability::traits::Logger;
use crate::services::structured_patch::{
    derive_claude_structured_patch, ClaudeStructuredPatchDerivationResult,
};

use super::claude_model_state;
use super::claude_transcript;
use super::conversation_trace::{
    diff_trace_fail_open_session_id, non_empty_string, required_field,
    required_non_empty_string_field,
};
use super::runtime::{
    current_unix_time_ms, open_agent_trace_db_for_hook_runtime, prefixed_diff_trace_session_id,
    read_hook_stdin, CLAUDE_MODEL_ID_PREFIX, CLAUDE_TOOL_NAME,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct DiffTracePayload {
    #[serde(rename = "sessionID")]
    pub(crate) session_id: String,
    pub(crate) diff: String,
    pub(crate) time: u64,
    pub(crate) model_id: Option<String>,
    #[serde(skip)]
    pub(crate) agent_id: Option<String>,
    #[serde(skip)]
    pub(crate) transcript_path: Option<String>,
    pub(crate) tool_name: String,
    pub(crate) tool_version: Option<String>,
    pub(crate) payload_type: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DiffTraceParseResult {
    Persist(DiffTracePayload),
    NoOp(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StdinPayloadKind {
    DiffTrace,
}

impl StdinPayloadKind {
    fn label(self) -> &'static str {
        match self {
            Self::DiffTrace => "diff-trace",
        }
    }

    fn validation_error(self, detail: &str) -> String {
        format!("Invalid {} payload from STDIN: {detail}.", self.label())
    }
}
pub(crate) fn run_diff_trace_subcommand(
    repository_root: &Path,
    logger: Option<&dyn Logger>,
) -> String {
    let stdin_payload = match read_hook_stdin() {
        Ok(payload) => payload,
        Err(error) => return log_diff_trace_fail_open(&error, logger, None),
    };
    let session_id = diff_trace_fail_open_session_id(&stdin_payload);

    match run_diff_trace_subcommand_from_payload(repository_root, &stdin_payload, logger) {
        Ok(output) => output,
        Err(error) => log_diff_trace_fail_open(&error, logger, session_id.as_deref()),
    }
}

pub(crate) fn run_diff_trace_subcommand_from_payload(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let parse_result = parse_diff_trace_payload(stdin_payload)?;
    let payload = match parse_result {
        DiffTraceParseResult::Persist(payload) => payload,
        DiffTraceParseResult::NoOp(message) => return Ok(message),
    };
    Ok(run_diff_trace_subcommand_from_payload_with(
        repository_root,
        &payload,
        logger,
    ))
}

pub(crate) fn log_diff_trace_fail_open(
    error: &anyhow::Error,
    logger: Option<&dyn Logger>,
    session_id: Option<&str>,
) -> String {
    if let Some(log) = logger {
        log.error(
            "sce.hooks.diff_trace.error",
            &error.to_string(),
            &[],
            session_id,
        );
    }

    String::from("diff-trace hook intake failed open; error logged.")
}

pub(crate) fn run_diff_trace_subcommand_from_payload_with(
    repository_root: &Path,
    payload: &DiffTracePayload,
    logger: Option<&dyn Logger>,
) -> String {
    if let Err(error) = diff_trace_db_time_ms(payload.time) {
        if let Some(log) = logger {
            log.warn(
                "sce.hooks.diff_trace.agent_trace_db_time_invalid",
                &error.to_string(),
                &[],
                Some(&payload.session_id),
            );
        }
    }
    let agent_trace_db_persisted =
        match persist_diff_trace_payload_to_agent_trace_db(repository_root, payload, logger) {
            Ok(persisted) => persisted,
            Err(error) => {
                if let Some(log) = logger {
                    log.warn(
                        "sce.hooks.diff_trace.agent_trace_db_write_failed",
                        &error.to_string(),
                        &[],
                        Some(&payload.session_id),
                    );
                }
                false
            }
        };

    if agent_trace_db_persisted {
        String::from("diff-trace hook intake persisted payload to AgentTraceDb.")
    } else {
        String::from("diff-trace hook intake completed; AgentTraceDb persistence failed.")
    }
}

pub(crate) fn parse_diff_trace_payload(stdin_payload: &str) -> Result<DiffTraceParseResult> {
    let payload_kind = StdinPayloadKind::DiffTrace;
    let parsed: Value = serde_json::from_str(stdin_payload)
        .with_context(|| payload_kind.validation_error("expected valid JSON"))?;
    let payload = parsed
        .as_object()
        .ok_or_else(|| anyhow!(payload_kind.validation_error("expected a JSON object")))?;

    if payload.contains_key("hook_event_name") {
        return parse_claude_diff_trace_payload(payload, stdin_payload, payload_kind);
    }

    let session_id = required_non_empty_string_field(payload, "sessionID", |d| {
        payload_kind.validation_error(d)
    })?;
    let diff =
        required_non_empty_string_field(payload, "diff", |d| payload_kind.validation_error(d))?;
    let time = required_u64_millisecond_field(payload, "time", payload_kind)?;
    let model_id = optional_string_field(payload, "model_id", payload_kind)?;
    let tool_name = required_non_empty_string_field(payload, "tool_name", |d| {
        payload_kind.validation_error(d)
    })?;
    let tool_version =
        required_nullable_or_non_empty_string_field(payload, "tool_version", payload_kind)?;

    Ok(DiffTraceParseResult::Persist(DiffTracePayload {
        session_id,
        diff,
        time,
        model_id,
        agent_id: None,
        transcript_path: None,
        tool_name,
        tool_version,
        payload_type: PAYLOAD_TYPE_PATCH.to_string(),
    }))
}

pub(crate) fn parse_claude_diff_trace_payload(
    payload: &serde_json::Map<String, Value>,
    stdin_payload: &str,
    payload_kind: StdinPayloadKind,
) -> Result<DiffTraceParseResult> {
    let event_name = required_non_empty_string_field(payload, "hook_event_name", |d| {
        payload_kind.validation_error(d)
    })?;

    if event_name != "PostToolUse" {
        return Ok(DiffTraceParseResult::NoOp(format!(
            "diff-trace hook intake: Claude '{event_name}' event has no diff trace; no-op."
        )));
    }

    let time = extract_claude_event_time(payload);

    match derive_claude_structured_patch(&event_name, &Value::Object(payload.clone()), time, None) {
        ClaudeStructuredPatchDerivationResult::Derived(patch) => {
            Ok(DiffTraceParseResult::Persist(DiffTracePayload {
                session_id: patch.session_id,
                diff: stdin_payload.to_string(),
                time: patch.time,
                model_id: resolve_claude_model_id(payload),
                agent_id: extract_claude_agent_id(payload)?,
                transcript_path: non_empty_string(payload.get("transcript_path"))
                    .map(str::to_string),
                tool_name: patch.tool_name,
                tool_version: patch.tool_version,
                payload_type: PAYLOAD_TYPE_STRUCTURED.to_string(),
            }))
        }
        ClaudeStructuredPatchDerivationResult::Skipped(reason) => {
            Ok(DiffTraceParseResult::NoOp(format!(
                "diff-trace hook intake: Claude PostToolUse event skipped ({reason:?}); no-op."
            )))
        }
    }
}

pub(crate) fn extract_claude_agent_id(
    payload: &serde_json::Map<String, Value>,
) -> Result<Option<String>> {
    let Some(value) = payload.get("agent_id") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }

    let value = value.as_str().ok_or_else(|| {
        anyhow!(StdinPayloadKind::DiffTrace
            .validation_error("field 'agent_id' must be null or a non-empty string"))
    })?;
    let value = value.trim();
    if value.is_empty() {
        bail!(StdinPayloadKind::DiffTrace
            .validation_error("field 'agent_id' must be null or a non-empty string"));
    }

    Ok(Some(value.to_string()))
}

pub(crate) fn resolve_claude_model_id(payload: &serde_json::Map<String, Value>) -> Option<String> {
    resolve_claude_model_id_with(payload, claude_transcript::extract_claude_transcript_model)
}

pub(crate) fn resolve_claude_model_id_with<F>(
    payload: &serde_json::Map<String, Value>,
    transcript_lookup: F,
) -> Option<String>
where
    F: FnOnce(&Path, &str) -> Option<String>,
{
    extract_direct_claude_model_id(payload).or_else(|| {
        let transcript_path = non_empty_string(payload.get("transcript_path"))?;
        let tool_use_id = non_empty_string(payload.get("tool_use_id"))?;

        transcript_lookup(Path::new(transcript_path), tool_use_id)
            .and_then(|model| normalize_claude_model_id(&model))
    })
}

pub(crate) fn extract_direct_claude_model_id(
    payload: &serde_json::Map<String, Value>,
) -> Option<String> {
    direct_claude_model_id_string(payload, &["model", "model_id", "modelId"])
        .or_else(|| {
            payload
                .get("model")
                .and_then(Value::as_object)
                .and_then(|model| direct_claude_model_id_string(model, &["id", "model", "name"]))
        })
        .and_then(|model| normalize_claude_model_id(&model))
}

pub(crate) fn direct_claude_model_id_string(
    payload: &serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<String> {
    keys.iter().find_map(|key| {
        payload
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

pub(crate) fn normalize_claude_model_id(model: &str) -> Option<String> {
    let normalized = model.trim();
    if normalized.is_empty() {
        return None;
    }

    if normalized.starts_with(CLAUDE_MODEL_ID_PREFIX) {
        Some(normalized.to_string())
    } else {
        Some(format!("{CLAUDE_MODEL_ID_PREFIX}{normalized}"))
    }
}

pub(crate) fn normalize_codex_model_id(model: &str) -> Option<String> {
    let normalized = model.trim();
    if normalized.is_empty() {
        return None;
    }

    Some(normalized.to_string())
}

pub(crate) fn normalize_opencode_model_id(model: &str) -> Option<String> {
    let normalized = model.trim();
    if normalized.is_empty() {
        return None;
    }

    Some(normalized.to_string())
}

pub(crate) fn normalize_pi_model_id(model: &str) -> Option<String> {
    let normalized = model.trim();
    if normalized.is_empty() {
        return None;
    }

    Some(normalized.to_string())
}

pub(crate) fn extract_claude_event_time(payload: &serde_json::Map<String, Value>) -> u64 {
    for key in &["time", "timestamp"] {
        if let Some(time_value) = payload.get(*key) {
            if let Some(time) = time_value.as_u64() {
                return time;
            }
            if let Some(time) = time_value.as_i64() {
                if time >= 0 {
                    #[allow(clippy::cast_sign_loss)]
                    return time as u64;
                }
            }
            if let Some(time) = time_value.as_f64() {
                #[allow(
                    clippy::cast_sign_loss,
                    clippy::cast_possible_truncation,
                    clippy::cast_precision_loss
                )]
                if time >= 0.0 && time.fract() == 0.0 && time <= u64::MAX as f64 {
                    return time as u64;
                }
            }
        }
    }
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

pub(crate) fn required_nullable_or_non_empty_string_field(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
    payload_kind: StdinPayloadKind,
) -> Result<Option<String>> {
    let raw = required_field(payload, field_name, |d| payload_kind.validation_error(d))?;

    if raw.is_null() {
        return Ok(None);
    }

    let value = raw.as_str().ok_or_else(|| {
        anyhow!(payload_kind.validation_error(&format!(
            "field '{field_name}' must be null or a non-empty string"
        )))
    })?;

    if value.trim().is_empty() {
        bail!(payload_kind.validation_error(&format!(
            "field '{field_name}' must be null or a non-empty string"
        )));
    }

    Ok(Some(value.to_string()))
}

pub(crate) fn optional_string_field(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
    payload_kind: StdinPayloadKind,
) -> Result<Option<String>> {
    let Some(raw) = payload.get(field_name) else {
        return Ok(None);
    };

    if raw.is_null() {
        return Ok(None);
    }

    let value = raw.as_str().ok_or_else(|| {
        anyhow!(payload_kind.validation_error(&format!(
            "field '{field_name}' must be null, absent, or a non-empty string"
        )))
    })?;

    if value.trim().is_empty() {
        bail!(payload_kind.validation_error(&format!(
            "field '{field_name}' must be null, absent, or a non-empty string"
        )));
    }

    Ok(Some(value.to_string()))
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub(crate) fn required_u64_millisecond_field(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
    payload_kind: StdinPayloadKind,
) -> Result<u64> {
    let raw = required_field(payload, field_name, |d| payload_kind.validation_error(d))?;

    if let Some(value) = raw.as_u64() {
        return Ok(value);
    }

    if let Some(value) = raw.as_i64() {
        if value < 0 {
            bail!(payload_kind.validation_error(&format!(
                "field '{field_name}' must be a u64 Unix epoch millisecond value, got a negative number"
            )));
        }
        return Ok(value as u64);
    }

    if let Some(value) = raw.as_f64() {
        if value.fract() != 0.0 {
            bail!(payload_kind.validation_error(&format!(
                "field '{field_name}' must be a u64 Unix epoch millisecond value, got a fractional number"
            )));
        }
        if value < 0.0 {
            bail!(payload_kind.validation_error(&format!(
                "field '{field_name}' must be a u64 Unix epoch millisecond value, got a negative number"
            )));
        }
        if value > u64::MAX as f64 {
            bail!(payload_kind.validation_error(&format!(
                "field '{field_name}' must be a u64 Unix epoch millisecond value"
            )));
        }
        return Ok(value as u64);
    }

    bail!(payload_kind.validation_error(&format!(
        "field '{field_name}' must be a u64 Unix epoch millisecond value"
    )))
}

pub(crate) fn persist_diff_trace_payload_to_agent_trace_db(
    repository_root: &Path,
    payload: &DiffTracePayload,
    logger: Option<&dyn Logger>,
) -> Result<bool> {
    let db = match open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for diff-trace persistence.",
    ) {
        Ok(db) => db,
        Err(error) => {
            if let Some(log) = logger {
                log.error(
                    "sce.hooks.diff_trace.agent_trace_db_open_failed",
                    &error.to_string(),
                    &[],
                    Some(&payload.session_id),
                );
            }

            return Ok(false);
        }
    };

    persist_diff_trace_payload_to_agent_trace_db_with_db(&db, payload)?;
    Ok(true)
}

pub(crate) fn persist_diff_trace_payload_to_agent_trace_db_with_db(
    db: &RepositoryAgentTraceDb,
    payload: &DiffTracePayload,
) -> Result<()> {
    let model_id = resolve_diff_trace_model_id(db, payload)?;
    db.insert_diff_trace(DiffTraceInsert {
        time_ms: diff_trace_db_time_ms(payload.time)?,
        session_id: &prefixed_diff_trace_session_id(&payload.tool_name, &payload.session_id),
        patch: &payload.diff,
        model_id: model_id.as_deref(),
        tool_name: &payload.tool_name,
        tool_version: payload.tool_version.as_deref(),
        payload_type: &payload.payload_type,
    })
    .context("Failed to persist diff-trace payload to Agent Trace DB.")?;

    Ok(())
}

pub(crate) fn resolve_diff_trace_model_id(
    db: &RepositoryAgentTraceDb,
    payload: &DiffTracePayload,
) -> Result<Option<String>> {
    if payload.model_id.is_some()
        || payload.tool_name != CLAUDE_TOOL_NAME
        || payload.payload_type != PAYLOAD_TYPE_STRUCTURED
    {
        return Ok(payload.model_id.clone());
    }

    let session_id = prefixed_diff_trace_session_id(CLAUDE_TOOL_NAME, &payload.session_id);
    let agent_id = payload.agent_id.as_deref().unwrap_or("");
    if let Some(state) = db.claude_model_state_by_session_and_agent(&session_id, agent_id)? {
        return Ok(Some(state.model_id));
    }

    Ok(seed_diff_trace_model_from_bridge_chain(
        db,
        payload,
        &session_id,
        agent_id,
    ))
}

pub(crate) fn seed_diff_trace_model_from_bridge_chain(
    db: &RepositoryAgentTraceDb,
    payload: &DiffTracePayload,
    session_id: &str,
    agent_id: &str,
) -> Option<String> {
    if !agent_id.is_empty() {
        return None;
    }

    let transcript_path = payload.transcript_path.as_deref()?;
    let model_id = claude_model_state::newest_bridge_chain_model(db, Path::new(transcript_path))?;

    let observed_at_ms = current_unix_time_ms().ok()?;
    match db.upsert_claude_model_state(ClaudeModelStateObservation {
        session_id: session_id.to_string(),
        agent_id: String::new(),
        model_id: model_id.clone(),
        observation_kind: ObservationKind::SessionStart,
        source: String::from("bridge_inherited"),
        observed_at_ms,
    }) {
        Ok(_) => Some(model_id),
        Err(_) => None,
    }
}

#[cfg(test)]
pub(crate) fn persist_diff_trace_payload_to_agent_trace_db_with<F, T>(
    payload: &DiffTracePayload,
    model_id: Option<&str>,
    tool_version: Option<&str>,
    insert_fn: F,
) -> Result<T>
where
    F: FnOnce(DiffTraceInsert<'_>) -> Result<T>,
{
    let time_ms = diff_trace_db_time_ms(payload.time)?;
    let session_id = prefixed_diff_trace_session_id(&payload.tool_name, &payload.session_id);

    insert_fn(DiffTraceInsert {
        time_ms,
        session_id: &session_id,
        patch: &payload.diff,
        model_id,
        tool_name: &payload.tool_name,
        tool_version,
        payload_type: &payload.payload_type,
    })
}

pub(crate) fn diff_trace_db_time_ms(time: u64) -> Result<i64> {
    i64::try_from(time).map_err(|_| {
        anyhow!(StdinPayloadKind::DiffTrace.validation_error(
            "field 'time' must fit in a signed 64-bit Unix epoch millisecond value for Agent Trace DB storage"
        ))
    })
}
