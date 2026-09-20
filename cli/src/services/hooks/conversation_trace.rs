use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{to_string as serialize_to_json, Value};

use crate::services::agent_trace_db::{
    InsertMessageInsert, InsertPartInsert, MessageRole, PartType,
};
use crate::services::observability::traits::Logger;
use crate::services::patch::{load_patch_from_json, parse_patch as parse_patch_from_text};

use super::claude_transforms::{
    transform_claude_post_tool_use, transform_claude_stop, transform_claude_user_prompt_submit,
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
pub(crate) fn run_conversation_trace_subcommand(
    repository_root: &Path,
    logger: Option<&dyn Logger>,
) -> String {
    let stdin_payload = match read_hook_stdin() {
        Ok(payload) => payload,
        Err(error) => return log_conversation_trace_fail_open(&error, logger, None),
    };
    let session_id = conversation_trace_fail_open_session_id(&stdin_payload);

    match run_conversation_trace_subcommand_from_payload(
        repository_root,
        &stdin_payload,
        logger,
        session_id.as_deref(),
    ) {
        Ok(output) => output,
        Err(error) => log_conversation_trace_fail_open(&error, logger, session_id.as_deref()),
    }
}

pub(crate) fn run_conversation_trace_subcommand_from_payload(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    session_id: Option<&str>,
) -> Result<String> {
    let payload = parse_conversation_trace_payload(stdin_payload)?;
    Ok(persist_conversation_trace_payload_to_agent_trace_db(
        repository_root,
        payload,
        logger,
        session_id,
    ))
}

pub(crate) fn log_conversation_trace_fail_open(
    error: &anyhow::Error,
    logger: Option<&dyn Logger>,
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

pub(crate) fn persist_conversation_trace_payload_to_agent_trace_db(
    repository_root: &Path,
    payload: ConversationTracePayload,
    logger: Option<&dyn Logger>,
    session_id: Option<&str>,
) -> String {
    let db = match open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for conversation-trace persistence.",
    ) {
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

            return String::from("conversation-trace hook intake failed open; error logged.");
        }
    };

    let summary = persist_conversation_trace_payload_to_agent_trace_db_with(
        payload,
        logger,
        |inserts| db.insert_messages(inserts),
        |inserts| db.insert_parts(inserts),
    );

    summary.render()
}
pub(crate) fn persist_conversation_trace_payload_to_agent_trace_db_with<IM, IP>(
    payload: ConversationTracePayload,
    logger: Option<&dyn Logger>,
    insert_messages: IM,
    insert_parts: IP,
) -> ConversationTracePersistenceSummary
where
    IM: FnOnce(Vec<InsertMessageInsert>) -> Result<u64>,
    IP: FnOnce(Vec<InsertPartInsert>) -> Result<u64>,
{
    log_skipped_conversation_trace_payloads(logger, "unsupported", &payload.skipped);

    let message_summary = persist_message_updated_batch_to_agent_trace_db_with(
        payload.message_updated,
        logger,
        insert_messages,
    );
    let part_summary = persist_message_part_updated_batch_to_agent_trace_db_with(
        payload.message_part_updated,
        logger,
        insert_parts,
    );

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

pub(crate) fn persist_message_updated_batch_to_agent_trace_db_with<I>(
    batch: ConversationTraceMessageBatch,
    logger: Option<&dyn Logger>,
    insert_messages: I,
) -> ConversationTraceEventPersistenceSummary
where
    I: FnOnce(Vec<InsertMessageInsert>) -> Result<u64>,
{
    const EVENT_TYPE: &str = "message";

    let mut skipped = batch.skipped.len();

    log_skipped_conversation_trace_payloads(logger, EVENT_TYPE, &batch.skipped);

    let valid_count = batch.inserts.len();
    let session_id = batch.diagnostic_session_id;
    let persisted = if valid_count == 0 {
        0
    } else {
        match insert_messages(batch.inserts) {
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

pub(crate) fn persist_message_part_updated_batch_to_agent_trace_db_with<I>(
    batch: ConversationTracePartBatch,
    logger: Option<&dyn Logger>,
    insert_parts: I,
) -> ConversationTraceEventPersistenceSummary
where
    I: FnOnce(Vec<InsertPartInsert>) -> Result<u64>,
{
    const EVENT_TYPE: &str = "message.part";

    let mut skipped = batch.skipped.len();

    log_skipped_conversation_trace_payloads(logger, EVENT_TYPE, &batch.skipped);

    let valid_count = batch.inserts.len();
    let session_id = batch.diagnostic_session_id;
    let persisted = if valid_count == 0 {
        0
    } else {
        match insert_parts(batch.inserts) {
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

pub(crate) fn log_skipped_conversation_trace_payloads(
    logger: Option<&dyn Logger>,
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

pub(crate) fn log_conversation_trace_batch_insert_failure(
    logger: Option<&dyn Logger>,
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

pub fn parse_conversation_trace_payload(stdin_payload: &str) -> Result<ConversationTracePayload> {
    let parsed: Value = serde_json::from_str(stdin_payload)
        .context("Invalid conversation-trace payload from STDIN: expected valid JSON.")?;
    let payload = parsed.as_object().ok_or_else(|| {
        anyhow!(conversation_trace_validation_error(
            "expected a JSON object"
        ))
    })?;

    if payload.contains_key("hook_event_name") {
        let event_name = required_non_empty_string_field(
            payload,
            "hook_event_name",
            conversation_trace_validation_error,
        )?;

        let items = match event_name.as_str() {
            "UserPromptSubmit" => transform_claude_user_prompt_submit(payload)?,
            "Stop" => transform_claude_stop(payload)?,
            "PostToolUse" => transform_claude_post_tool_use(payload)?,
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
