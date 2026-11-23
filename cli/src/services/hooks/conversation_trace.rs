use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{to_string as serialize_to_json, Value};

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_db::{
    InsertMessageInsert, InsertPartInsert, MessageRole, PartType,
};
use crate::services::patch::{load_patch_from_json, parse_patch as parse_patch_from_text};

use super::claude_model_resolver;
use super::claude_transforms::{
    claude_patch_evidence, transform_claude_post_tool_use, transform_claude_stop,
    transform_claude_user_prompt_submit, ClaudePatchEnrichment, ClaudePatchEvidence,
};
use super::runtime::{
    open_agent_trace_db_for_hook_runtime, prefixed_conversation_trace_session_id, read_hook_stdin,
    PayloadValidationError, CLAUDE_TOOL_NAME, NORMALIZED_CONVERSATION_TRACE_TOOL_NAMES,
};

pub(crate) const CONVERSATION_TRACE_MESSAGE_UPDATED: &str = "message";
pub(crate) const CONVERSATION_TRACE_MESSAGE_PART_UPDATED: &str = "message.part";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationTracePayload {
    pub attempted_count: usize,
    pub message_updated: ConversationTraceMessageBatch,
    pub message_part_updated: ConversationTracePartBatch,
    pub skipped: Vec<SkippedConversationTracePayload>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationTraceMessageBatch {
    pub inserts: Vec<InsertMessageInsert>,
    pub skipped: Vec<SkippedConversationTracePayload>,
    diagnostic_session_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationTracePartBatch {
    pub inserts: Vec<InsertPartInsert>,
    pub skipped: Vec<SkippedConversationTracePayload>,
    diagnostic_session_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkippedConversationTracePayload {
    pub index: usize,
    pub reason: String,
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConversationTracePersistenceSummary {
    attempted: usize,
    persisted_messages: usize,
    persisted_parts: usize,
    skipped: usize,
}

impl ConversationTracePersistenceSummary {
    fn render(&self) -> String {
        format!(
            "conversation-trace hook persisted mixed payload batch to AgentTraceDb: attempted={}, persisted_messages={}, persisted_parts={}, skipped={}.",
            self.attempted, self.persisted_messages, self.persisted_parts, self.skipped
        )
    }
}
pub(crate) async fn run_conversation_trace_subcommand<
    L: crate::services::observability::traits::Logger,
>(
    repository_root: &Path,
    logger: Option<&L>,
) -> String {
    crate::services::managed_telemetry::trace_export::record_claude_hook_command(
        crate::services::managed_telemetry::trace_export::ClaudeHookCommand::ConversationTrace,
    );
    let stdin_payload = match read_hook_stdin() {
        Ok(payload) => payload,
        Err(error) => return log_conversation_trace_fail_open(&error, logger, None),
    };
    crate::services::managed_telemetry::trace_export::record_claude_hook_payload(&stdin_payload);
    let session_id = conversation_trace_fail_open_session_id(&stdin_payload);

    match run_conversation_trace_subcommand_from_payload(
        repository_root,
        &stdin_payload,
        logger,
        session_id.as_deref(),
    )
    .await
    {
        Ok(output) => output,
        Err(error) => log_conversation_trace_fail_open(&error, logger, session_id.as_deref()),
    }
}

pub(crate) async fn run_conversation_trace_subcommand_from_payload<
    L: crate::services::observability::traits::Logger,
>(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&L>,
    session_id: Option<&str>,
) -> Result<String> {
    let request = parse_conversation_trace_request(stdin_payload)?;
    let db = match open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for conversation-trace persistence.",
    )
    .await
    {
        Ok(db) => db,
        Err(error) => {
            if let Some(log) = logger {
                log.error(
                    "sce.hooks.conversation_trace.agent_trace_db_open_failed",
                    &error.to_string(),
                    &[],
                    session_id,
                );
            }

            return Ok(String::from(
                "conversation-trace hook intake failed open; error logged.",
            ));
        }
    };

    let payload = conversation_trace_payload_with_db(&db, &request, logger).await?;
    let summary = persist_conversation_trace_payload_to_agent_trace_db_with(
        payload,
        logger,
        async |inserts| db.insert_messages(inserts).await,
        async |inserts| db.insert_parts(inserts).await,
    )
    .await;

    Ok(summary.render())
}

pub(crate) async fn conversation_trace_payload_with_db<
    L: crate::services::observability::traits::Logger,
>(
    db: &RepositoryAgentTraceDb,
    request: &serde_json::Map<String, Value>,
    logger: Option<&L>,
) -> Result<ConversationTracePayload> {
    let claude_patch_enrichment = match claude_patch_evidence(request) {
        Some(evidence) => Some(resolve_claude_patch_enrichment(db, &evidence, logger).await),
        None => None,
    };
    conversation_trace_payload_from_request(request, claude_patch_enrichment)
}

async fn resolve_claude_patch_enrichment<L: crate::services::observability::traits::Logger>(
    db: &RepositoryAgentTraceDb,
    evidence: &ClaudePatchEvidence,
    logger: Option<&L>,
) -> ClaudePatchEnrichment {
    let model_id = match claude_model_resolver::resolve_claude_model(db, &evidence.model).await {
        Ok(model_id) => model_id,
        Err(error) => {
            if let Some(log) = logger {
                log.warn(
                    "sce.hooks.conversation_trace.claude_model_resolution_failed",
                    &error.to_string(),
                    &[],
                    Some(&evidence.model.session_id),
                );
            }
            None
        }
    };

    ClaudePatchEnrichment {
        model_id,
        attribution: claude_model_resolver::resolve_claude_patch_attribution(
            &evidence.model,
            evidence.agent_type.as_deref(),
        ),
    }
}

pub(crate) fn log_conversation_trace_fail_open<
    L: crate::services::observability::traits::Logger,
>(
    error: &anyhow::Error,
    logger: Option<&L>,
    session_id: Option<&str>,
) -> String {
    if let Some(log) = logger {
        log.error(
            "sce.hooks.conversation_trace.error",
            &error.to_string(),
            &[],
            session_id,
        );
    }

    String::from("conversation-trace hook intake failed open; error logged.")
}

pub(crate) async fn persist_conversation_trace_payload_to_agent_trace_db_with<
    L: crate::services::observability::traits::Logger,
    IM,
    IP,
>(
    payload: ConversationTracePayload,
    logger: Option<&L>,
    insert_messages: IM,
    insert_parts: IP,
) -> ConversationTracePersistenceSummary
where
    IM: std::ops::AsyncFnOnce(Vec<InsertMessageInsert>) -> Result<u64>,
    IP: std::ops::AsyncFnOnce(Vec<InsertPartInsert>) -> Result<u64>,
{
    log_skipped_conversation_trace_payloads(logger, "unsupported", &payload.skipped);

    let message_summary = persist_message_updated_batch_to_agent_trace_db_with(
        payload.message_updated,
        logger,
        insert_messages,
    )
    .await;
    let part_summary = persist_message_part_updated_batch_to_agent_trace_db_with(
        payload.message_part_updated,
        logger,
        insert_parts,
    )
    .await;

    ConversationTracePersistenceSummary {
        attempted: payload.attempted_count,
        persisted_messages: message_summary.persisted,
        persisted_parts: part_summary.persisted,
        skipped: payload.skipped.len() + message_summary.skipped + part_summary.skipped,
    }
}

pub(crate) struct ConversationTraceEventPersistenceSummary {
    persisted: usize,
    skipped: usize,
}

pub(crate) async fn persist_message_updated_batch_to_agent_trace_db_with<
    L: crate::services::observability::traits::Logger,
    I,
>(
    batch: ConversationTraceMessageBatch,
    logger: Option<&L>,
    insert_messages: I,
) -> ConversationTraceEventPersistenceSummary
where
    I: std::ops::AsyncFnOnce(Vec<InsertMessageInsert>) -> Result<u64>,
{
    const EVENT_TYPE: &str = "message";

    let mut skipped = batch.skipped.len();

    log_skipped_conversation_trace_payloads(logger, EVENT_TYPE, &batch.skipped);

    let valid_count = batch.inserts.len();
    let session_id = batch.diagnostic_session_id;
    let persisted = if valid_count == 0 {
        0
    } else {
        match insert_messages(batch.inserts).await {
            Ok(affected_rows) => usize::try_from(affected_rows)
                .unwrap_or(usize::MAX)
                .min(valid_count),
            Err(error) => {
                skipped += valid_count;
                log_conversation_trace_batch_insert_failure(
                    logger,
                    EVENT_TYPE,
                    valid_count,
                    &error,
                    session_id.as_deref(),
                );
                0
            }
        }
    };

    ConversationTraceEventPersistenceSummary { persisted, skipped }
}

pub(crate) async fn persist_message_part_updated_batch_to_agent_trace_db_with<
    L: crate::services::observability::traits::Logger,
    I,
>(
    batch: ConversationTracePartBatch,
    logger: Option<&L>,
    insert_parts: I,
) -> ConversationTraceEventPersistenceSummary
where
    I: std::ops::AsyncFnOnce(Vec<InsertPartInsert>) -> Result<u64>,
{
    const EVENT_TYPE: &str = "message.part";

    let mut skipped = batch.skipped.len();

    log_skipped_conversation_trace_payloads(logger, EVENT_TYPE, &batch.skipped);

    let valid_count = batch.inserts.len();
    let session_id = batch.diagnostic_session_id;
    let persisted = if valid_count == 0 {
        0
    } else {
        match insert_parts(batch.inserts).await {
            Ok(affected_rows) => usize::try_from(affected_rows)
                .unwrap_or(usize::MAX)
                .min(valid_count),
            Err(error) => {
                skipped += valid_count;
                log_conversation_trace_batch_insert_failure(
                    logger,
                    EVENT_TYPE,
                    valid_count,
                    &error,
                    session_id.as_deref(),
                );
                0
            }
        }
    };

    ConversationTraceEventPersistenceSummary { persisted, skipped }
}

pub(crate) fn log_skipped_conversation_trace_payloads<
    L: crate::services::observability::traits::Logger,
>(
    logger: Option<&L>,
    event_type: &str,
    skipped_payloads: &[SkippedConversationTracePayload],
) {
    let Some(log) = logger else {
        return;
    };

    for skipped in skipped_payloads {
        let index = skipped.index.to_string();
        log.warn(
            "sce.hooks.conversation_trace.payload_skipped",
            &skipped.reason,
            &[
                ("event_type", event_type),
                ("payload_index", index.as_str()),
            ],
            skipped.session_id.as_deref(),
        );
    }
}

pub(crate) fn log_conversation_trace_batch_insert_failure<
    L: crate::services::observability::traits::Logger,
>(
    logger: Option<&L>,
    event_type: &str,
    valid_count: usize,
    error: &anyhow::Error,
    session_id: Option<&str>,
) {
    if let Some(log) = logger {
        let count = valid_count.to_string();
        log.warn(
            "sce.hooks.conversation_trace.agent_trace_db_batch_failed",
            &error.to_string(),
            &[("event_type", event_type), ("valid_count", count.as_str())],
            session_id,
        );
    }
}

pub(crate) fn parse_conversation_trace_request(
    stdin_payload: &str,
) -> Result<serde_json::Map<String, Value>> {
    let parsed: Value = serde_json::from_str(stdin_payload)
        .context("Invalid conversation-trace payload from STDIN: expected valid JSON.")?;
    match parsed {
        Value::Object(payload) => Ok(payload),
        _ => bail!(conversation_trace_validation_error(
            "expected a JSON object"
        )),
    }
}

pub(crate) fn conversation_trace_payload_from_request(
    payload: &serde_json::Map<String, Value>,
    claude_patch_enrichment: Option<ClaudePatchEnrichment>,
) -> Result<ConversationTracePayload> {
    if payload.contains_key("hook_event_name") {
        let event_name = required_non_empty_string_field(
            payload,
            "hook_event_name",
            conversation_trace_validation_error,
        )?;

        let items = match event_name.as_str() {
            "UserPromptSubmit" => transform_claude_user_prompt_submit(payload)?,
            "Stop" => transform_claude_stop(payload)?,
            "PostToolUse" => {
                transform_claude_post_tool_use(payload, claude_patch_enrichment.unwrap_or_default())?
            },
            _ => bail!(conversation_trace_validation_error(&format!(
                "unsupported Claude hook event '{event_name}': supported events are 'UserPromptSubmit', 'Stop' and 'PostToolUse'"
            ))),
        };
        return Ok(parse_conversation_trace_payloads(&items, CLAUDE_TOOL_NAME));
    }

    let tool_name =
        required_non_empty_string_field(payload, "tool_name", conversation_trace_validation_error)?;
    if !NORMALIZED_CONVERSATION_TRACE_TOOL_NAMES.contains(&tool_name.as_str()) {
        bail!(conversation_trace_validation_error(&format!(
            "unsupported tool_name '{tool_name}': supported producers are 'opencode' and 'pi'"
        )));
    }
    let payloads = required_payloads_array(payload)?;

    Ok(parse_conversation_trace_payloads(payloads, &tool_name))
}

pub(crate) fn required_payloads_array(
    payload: &serde_json::Map<String, Value>,
) -> Result<&Vec<Value>> {
    required_field(payload, "payloads", conversation_trace_validation_error)?
        .as_array()
        .ok_or_else(|| {
            anyhow!(conversation_trace_validation_error(
                "field 'payloads' must be an array"
            ))
        })
}

pub(crate) fn parse_conversation_trace_payloads(
    payloads: &[Value],
    tool_name: &str,
) -> ConversationTracePayload {
    let mut message_inserts = Vec::new();
    let mut message_skipped = Vec::new();
    let mut part_inserts = Vec::new();
    let mut part_skipped = Vec::new();
    let mut skipped = Vec::new();
    let mut message_diagnostic_session_id = None;
    let mut part_diagnostic_session_id = None;

    for (index, item) in payloads.iter().enumerate() {
        let session_id = non_empty_string(item.get("session_id")).map(str::to_owned);
        let Some(item) = conversation_trace_payload_item(item, index, &mut skipped) else {
            continue;
        };

        let event_type =
            match required_string_field(item, "type", conversation_trace_validation_error) {
                Ok(event_type) => event_type,
                Err(error) => {
                    skipped.push(SkippedConversationTracePayload {
                        index,
                        reason: error.to_string(),
                        session_id: session_id.clone(),
                    });
                    continue;
                }
            };

        match event_type.as_str() {
            CONVERSATION_TRACE_MESSAGE_UPDATED => match parse_message_updated_item(item) {
                Ok(mut input) => {
                    if message_diagnostic_session_id.is_none() {
                        message_diagnostic_session_id.clone_from(&session_id);
                    }
                    input.session_id =
                        prefixed_conversation_trace_session_id(tool_name, &input.session_id);
                    message_inserts.push(input);
                }
                Err(error) => message_skipped.push(SkippedConversationTracePayload {
                    index,
                    reason: error.to_string(),
                    session_id: session_id.clone(),
                }),
            },
            CONVERSATION_TRACE_MESSAGE_PART_UPDATED => {
                match parse_message_part_updated_item(item) {
                    Ok(mut input) => {
                        if part_diagnostic_session_id.is_none() {
                            part_diagnostic_session_id.clone_from(&session_id);
                        }
                        input.session_id =
                            prefixed_conversation_trace_session_id(tool_name, &input.session_id);
                        part_inserts.push(input);
                    }
                    Err(error) => part_skipped.push(SkippedConversationTracePayload {
                        index,
                        reason: error.to_string(),
                        session_id: session_id.clone(),
                    }),
                }
            }
            _ => skipped.push(SkippedConversationTracePayload {
                index,
                reason: conversation_trace_validation_error(
                    "field 'type' must be one of 'message' or 'message.part'",
                ),
                session_id,
            }),
        }
    }

    ConversationTracePayload {
        attempted_count: payloads.len(),
        message_updated: ConversationTraceMessageBatch {
            inserts: message_inserts,
            skipped: message_skipped,
            diagnostic_session_id: message_diagnostic_session_id,
        },
        message_part_updated: ConversationTracePartBatch {
            inserts: part_inserts,
            skipped: part_skipped,
            diagnostic_session_id: part_diagnostic_session_id,
        },
        skipped,
    }
}

pub(crate) fn conversation_trace_payload_item<'a>(
    item: &'a Value,
    index: usize,
    skipped: &mut Vec<SkippedConversationTracePayload>,
) -> Option<&'a serde_json::Map<String, Value>> {
    let Some(payload) = item.as_object() else {
        skipped.push(SkippedConversationTracePayload {
            index,
            reason: conversation_trace_validation_error(&format!(
                "payloads[{index}] must be an object"
            )),
            session_id: None,
        });
        return None;
    };

    Some(payload)
}

pub(crate) fn parse_message_updated_item(
    payload: &serde_json::Map<String, Value>,
) -> Result<InsertMessageInsert> {
    Ok(InsertMessageInsert {
        session_id: required_non_empty_string_field(
            payload,
            "session_id",
            conversation_trace_validation_error,
        )?,
        message_id: required_non_empty_string_field(
            payload,
            "message_id",
            conversation_trace_validation_error,
        )?,
        role: parse_message_role(payload)?,
        generated_at_unix_ms: required_i64_millisecond_field(
            payload,
            "generated_at_unix_ms",
            conversation_trace_validation_error,
        )?,
    })
}

pub(crate) fn parse_message_part_updated_item(
    payload: &serde_json::Map<String, Value>,
) -> Result<InsertPartInsert> {
    let part_type = parse_part_type(payload)?;
    let raw_text = required_string_field(payload, "text", conversation_trace_validation_error)?;
    let text = match part_type {
        PartType::Patch => {
            if load_patch_from_json(&raw_text).is_ok() {
                raw_text
            } else {
                match parse_patch_from_text(&raw_text, None) {
                    Ok(parsed_patch) => serialize_to_json(&parsed_patch).map_err(|error| {
                        anyhow!(conversation_trace_validation_error(&format!(
                            "failed to serialize parsed patch for conversation-trace patch part: {error}"
                        )))
                    })?,
                    Err(diff_error) => {
                        bail!(conversation_trace_validation_error(&format!(
                            "field 'text' for patch part is neither valid patch-JSON nor a valid patch: {diff_error}"
                        )));
                    }
                }
            }
        }
        PartType::Text | PartType::Reasoning => raw_text,
        PartType::Question => validate_question_part_text(raw_text)?,
    };

    Ok(InsertPartInsert {
        session_id: required_non_empty_string_field(
            payload,
            "session_id",
            conversation_trace_validation_error,
        )?,
        message_id: required_non_empty_string_field(
            payload,
            "message_id",
            conversation_trace_validation_error,
        )?,
        part_type,
        text,
        generated_at_unix_ms: required_i64_millisecond_field(
            payload,
            "generated_at_unix_ms",
            conversation_trace_validation_error,
        )?,
    })
}

pub(crate) fn parse_message_role(payload: &serde_json::Map<String, Value>) -> Result<MessageRole> {
    match required_string_field(payload, "role", conversation_trace_validation_error)?.as_str() {
        "user" => Ok(MessageRole::User),
        "assistant" => Ok(MessageRole::Assistant),
        _ => bail!(conversation_trace_validation_error(
            "field 'role' must be one of 'user' or 'assistant'"
        )),
    }
}

pub(crate) fn parse_part_type(payload: &serde_json::Map<String, Value>) -> Result<PartType> {
    match required_string_field(payload, "part_type", conversation_trace_validation_error)?.as_str()
    {
        "text" => Ok(PartType::Text),
        "reasoning" => Ok(PartType::Reasoning),
        "patch" => Ok(PartType::Patch),
        "question" => Ok(PartType::Question),
        _ => bail!(conversation_trace_validation_error(
            "field 'part_type' must be one of 'text', 'reasoning', 'patch' or 'question'"
        )),
    }
}

pub(crate) fn validate_question_part_text(raw_text: String) -> Result<String> {
    let parsed: Value = serde_json::from_str(&raw_text).map_err(|_| {
        anyhow!(conversation_trace_validation_error(
            "field 'text' for question part must be a JSON array of objects with string 'question' and 'answer' fields"
        ))
    })?;

    let items = parsed.as_array().ok_or_else(|| {
        anyhow!(conversation_trace_validation_error(
            "field 'text' for question part must be a JSON array of objects with string 'question' and 'answer' fields"
        ))
    })?;

    if items.iter().all(|item| {
        item.as_object().is_some_and(|object| {
            object.get("question").is_some_and(Value::is_string)
                && object.get("answer").is_some_and(Value::is_string)
        })
    }) {
        return Ok(raw_text);
    }

    bail!(conversation_trace_validation_error(
        "field 'text' for question part must be a JSON array of objects with string 'question' and 'answer' fields"
    ))
}

pub(crate) fn conversation_trace_validation_error(detail: &str) -> String {
    format!("Invalid conversation-trace payload from STDIN: {detail}.")
}

pub(crate) fn conversation_trace_fail_open_session_id(stdin_payload: &str) -> Option<String> {
    let payload: Value = serde_json::from_str(stdin_payload).ok()?;
    let payload = payload.as_object()?;

    if payload.contains_key("hook_event_name") {
        return non_empty_string(payload.get("session_id")).map(str::to_owned);
    }

    let first_payload = payload.get("payloads")?.as_array()?.first()?;
    non_empty_string(first_payload.get("session_id")).map(str::to_owned)
}

pub(crate) fn diff_trace_fail_open_session_id(stdin_payload: &str) -> Option<String> {
    let payload: Value = serde_json::from_str(stdin_payload).ok()?;
    let payload = payload.as_object()?;
    let field_name = if payload.contains_key("hook_event_name") {
        "session_id"
    } else {
        "sessionID"
    };

    non_empty_string(payload.get(field_name)).map(str::to_owned)
}

pub(crate) fn non_empty_string(value: Option<&Value>) -> Option<&str> {
    value?.as_str().filter(|value| !value.trim().is_empty())
}

pub(crate) fn required_non_empty_string_field(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
    format_error: impl Fn(&str) -> String,
) -> Result<String> {
    let raw = required_field(payload, field_name, &format_error)?;

    let value = raw.as_str().ok_or_else(|| {
        anyhow!(format_error(&format!(
            "field '{field_name}' must be a non-empty string"
        )))
    })?;

    if value.trim().is_empty() {
        bail!(format_error(&format!(
            "field '{field_name}' must be a non-empty string"
        )));
    }

    Ok(value.to_string())
}

pub(crate) fn required_string_field(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
    validation_error: PayloadValidationError,
) -> Result<String> {
    let raw = required_field(payload, field_name, validation_error)?;

    raw.as_str().map(ToString::to_string).ok_or_else(|| {
        anyhow!(validation_error(&format!(
            "field '{field_name}' must be a string"
        )))
    })
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub(crate) fn required_i64_millisecond_field(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
    validation_error: PayloadValidationError,
) -> Result<i64> {
    let raw = required_field(payload, field_name, validation_error)?;

    if let Some(value) = raw.as_i64() {
        if value < 0 {
            bail!(validation_error(&format!(
                "field '{field_name}' must be a non-negative signed 64-bit Unix epoch millisecond value"
            )));
        }
        return Ok(value);
    }

    if let Some(value) = raw.as_u64() {
        return i64::try_from(value).map_err(|_| {
            anyhow!(validation_error(&format!(
                "field '{field_name}' must fit in a signed 64-bit Unix epoch millisecond value for Agent Trace DB storage"
            )))
        });
    }

    if raw.as_f64().is_some_and(|value| value.fract() != 0.0) {
        bail!(validation_error(&format!(
            "field '{field_name}' must be a non-negative signed 64-bit Unix epoch millisecond value, got a fractional number"
        )));
    }

    bail!(validation_error(&format!(
        "field '{field_name}' must be a non-negative signed 64-bit Unix epoch millisecond value"
    )))
}

pub(crate) fn required_field<'a>(
    payload: &'a serde_json::Map<String, Value>,
    field_name: &str,
    format_error: impl Fn(&str) -> String,
) -> Result<&'a Value> {
    payload.get(field_name).ok_or_else(|| {
        anyhow!(format_error(&format!(
            "missing required field '{field_name}'"
        )))
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use serde_json::Value;

    use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
    use crate::services::agent_trace_db::{ClaudeModelStateObservation, ObservationKind};
    use crate::services::observability::traits::NoopLogger;

    use super::{
        conversation_trace_payload_with_db, parse_conversation_trace_request,
        persist_conversation_trace_payload_to_agent_trace_db_with,
    };

    const SESSION: &str = "3c9d7e1f-1111-4222-8333-944445555666";
    const CANONICAL_SESSION: &str = "cc_3c9d7e1f-1111-4222-8333-944445555666";
    const AGENT: &str = "a7f3c2e1b9d8";
    const TOOL_USE: &str = "toolu_write_7";
    const PARENT_TOOL_USE: &str = "toolu_agent_parent";

    struct Fixture {
        _dir: tempfile::TempDir,
        project_dir: PathBuf,
        db: RepositoryAgentTraceDb,
    }

    impl Fixture {
        async fn new() -> Self {
            let dir = tempfile::tempdir().expect("temp dir");
            let project_dir = dir.path().join("project");
            std::fs::create_dir_all(project_dir.join(SESSION).join("subagents"))
                .expect("subagent dir");
            let db = RepositoryAgentTraceDb::new_at(dir.path().join("agent-trace.db"))
                .await
                .expect("db");
            Self {
                _dir: dir,
                project_dir,
                db,
            }
        }

        fn main_transcript(&self) -> PathBuf {
            self.project_dir.join(format!("{SESSION}.jsonl"))
        }

        fn subagent_file(&self, extension: &str) -> PathBuf {
            self.project_dir
                .join(SESSION)
                .join("subagents")
                .join(format!("agent-{AGENT}.{extension}"))
        }

        async fn seed_main_agent_state(&self, model_id: &str) {
            self.db
                .upsert_claude_model_state(ClaudeModelStateObservation {
                    session_id: CANONICAL_SESSION.to_string(),
                    agent_id: String::new(),
                    model_id: model_id.to_string(),
                    observation_kind: ObservationKind::SessionStart,
                    source: String::from("startup"),
                    observed_at_ms: 1_000,
                })
                .await
                .expect("seed state");
        }

        async fn patch_part(&self, overrides: &[(&str, &str)]) -> Value {
            let mut payload: Value = serde_json::from_str(include_str!(
                "../structured_patch/fixtures/write_create_simple/claude-post-tool-use.json"
            ))
            .expect("fixture JSON");
            let object = payload.as_object_mut().expect("fixture object");
            object.insert("session_id".into(), SESSION.into());
            object.insert("tool_use_id".into(), TOOL_USE.into());
            object.insert(
                "transcript_path".into(),
                self.main_transcript().display().to_string().into(),
            );
            for (key, value) in overrides {
                object.insert((*key).into(), (*value).into());
            }

            let request = parse_conversation_trace_request(&payload.to_string()).unwrap();
            let trace = conversation_trace_payload_with_db(&self.db, &request, None::<&NoopLogger>)
                .await
                .expect("Claude PostToolUse builds a conversation trace");
            let parts = trace.message_part_updated.inserts.clone();
            assert_eq!(parts.len(), 1, "one patch part per Write");
            assert_eq!(parts[0].session_id, CANONICAL_SESSION);

            let summary = persist_conversation_trace_payload_to_agent_trace_db_with(
                trace,
                None::<&NoopLogger>,
                async |inserts| self.db.insert_messages(inserts).await,
                async |inserts| self.db.insert_parts(inserts).await,
            )
            .await;
            assert!(
                summary.render().contains("persisted_parts=1"),
                "patch part must persist: {}",
                summary.render()
            );

            serde_json::from_str(&parts[0].text).expect("patch part text is JSON")
        }
    }

    fn write(path: &Path, contents: &str) {
        std::fs::write(path, contents).expect("write fixture file");
    }

    fn tool_use_record(tool_use_id: &str, model: &str) -> String {
        format!(
            "{}\n",
            serde_json::json!({
                "type": "assistant",
                "message": {
                    "role": "assistant",
                    "model": model,
                    "content": [{"type": "tool_use", "id": tool_use_id, "name": "Write"}]
                }
            })
        )
    }

    fn meta(agent_type: &str, parent_tool_use_id: &str) -> String {
        serde_json::json!({"agentType": agent_type, "toolUseId": parent_tool_use_id}).to_string()
    }

    fn hunk_models(patch: &Value) -> Vec<Value> {
        patch["files"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|file| file["hunks"].as_array().unwrap().iter())
            .map(|hunk| hunk["model_id"].clone())
            .collect()
    }

    fn line_sessions(patch: &Value) -> Vec<Value> {
        patch["files"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|file| file["hunks"].as_array().unwrap().iter())
            .flat_map(|hunk| hunk["lines"].as_array().unwrap().iter())
            .map(|line| line["session_id"].clone())
            .collect()
    }

    fn assert_no_agent_fields_below_patch(patch: &Value) {
        for file in patch["files"].as_array().unwrap() {
            for hunk in file["hunks"].as_array().unwrap() {
                assert!(hunk.get("agent_id").is_none() && hunk.get("attribution").is_none());
                for line in hunk["lines"].as_array().unwrap() {
                    assert!(line.get("agent_id").is_none() && line.get("attribution").is_none());
                }
            }
        }
    }

    #[tokio::test]
    async fn conversation_trace_claude_main_agent_patch_carries_session_model_and_null_agent() {
        let fixture = Fixture::new().await;
        fixture
            .seed_main_agent_state("claude/claude-opus-5-5")
            .await;
        write(
            &fixture.main_transcript(),
            &tool_use_record(TOOL_USE, "claude-haiku-5-5"),
        );

        let patch = fixture.patch_part(&[]).await;

        let sessions = line_sessions(&patch);
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().all(|session| session == CANONICAL_SESSION));
        assert_eq!(
            hunk_models(&patch),
            vec![Value::from("claude/claude-haiku-5-5")]
        );
        assert_eq!(
            patch["attribution"],
            serde_json::json!({
                "agent_id": null,
                "agent_type": null,
                "tool_use_id": TOOL_USE,
                "parent_tool_use_id": null
            })
        );
        assert_no_agent_fields_below_patch(&patch);
    }

    #[tokio::test]
    async fn conversation_trace_claude_subagent_patch_uses_subagent_record_and_meta() {
        let fixture = Fixture::new().await;
        fixture
            .seed_main_agent_state("claude/claude-opus-5-5")
            .await;
        write(
            &fixture.main_transcript(),
            &tool_use_record(TOOL_USE, "claude-opus-5-5"),
        );
        write(
            &fixture.subagent_file("jsonl"),
            &tool_use_record(TOOL_USE, "claude-sonnet-5-5"),
        );
        write(
            &fixture.subagent_file("meta.json"),
            &meta("general-purpose", PARENT_TOOL_USE),
        );

        let patch = fixture.patch_part(&[("agent_id", AGENT)]).await;

        assert!(line_sessions(&patch)
            .iter()
            .all(|session| session == CANONICAL_SESSION));
        assert_eq!(
            hunk_models(&patch),
            vec![Value::from("claude/claude-sonnet-5-5")]
        );
        assert_eq!(
            patch["attribution"],
            serde_json::json!({
                "agent_id": AGENT,
                "agent_type": "general-purpose",
                "tool_use_id": TOOL_USE,
                "parent_tool_use_id": PARENT_TOOL_USE
            })
        );
        assert_no_agent_fields_below_patch(&patch);
    }

    #[tokio::test]
    async fn conversation_trace_claude_subagent_event_agent_type_wins_over_meta() {
        let fixture = Fixture::new().await;
        write(
            &fixture.subagent_file("meta.json"),
            &meta("general-purpose", PARENT_TOOL_USE),
        );

        let patch = fixture
            .patch_part(&[("agent_id", AGENT), ("agent_type", "Explore")])
            .await;

        assert_eq!(patch["attribution"]["agent_type"], "Explore");
        assert_eq!(patch["attribution"]["parent_tool_use_id"], PARENT_TOOL_USE);
    }

    #[tokio::test]
    async fn conversation_trace_claude_subagent_missing_files_yield_nulls_and_still_persist() {
        let fixture = Fixture::new().await;
        fixture
            .seed_main_agent_state("claude/claude-opus-5-5")
            .await;

        let patch = fixture.patch_part(&[("agent_id", AGENT)]).await;

        assert_eq!(hunk_models(&patch), vec![Value::Null]);
        assert!(line_sessions(&patch)
            .iter()
            .all(|session| session == CANONICAL_SESSION));
        assert_eq!(
            patch["attribution"],
            serde_json::json!({
                "agent_id": AGENT,
                "agent_type": null,
                "tool_use_id": TOOL_USE,
                "parent_tool_use_id": null
            })
        );
    }
}
