use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, to_string as serialize_to_json, Value};

use crate::services::agent_trace::{
    build_agent_trace, validate_agent_trace_value, AgentTrace, AgentTraceMetadataInput,
    AgentTraceVcsType,
};
use crate::services::agent_trace_db::{
    AgentTraceDb, AgentTraceInsert, DiffTraceInsert, InsertMessageInsert, InsertPartInsert,
    MessageRole, PartType, PostCommitPatchIntersectionInsert, RecentDiffTracePatches,
};
use crate::services::config;
use crate::services::observability::traits::Logger;
use crate::services::patch::{
    combine_patches as combine_patches_fn, intersect_patches as intersect_patches_fn,
    parse_patch as parse_patch_from_text, ParsedPatch,
};

pub mod command;
pub mod lifecycle;

pub const NAME: &str = "hooks";
pub const CANONICAL_SCE_COAUTHOR_TRAILER: &str = "Co-authored-by: SCE <sce@crocoder.dev>";
const AGENT_TRACE_URL_PREFIX: &str = "sce.crocoder.dev/trace/";

const MAX_TRACE_FILE_CREATE_ATTEMPTS: u64 = 1_000_000;

type PayloadValidationError = fn(&str) -> String;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HookSubcommand {
    PreCommit,
    CommitMsg {
        message_file: PathBuf,
    },
    PostCommit {
        vcs_type: Option<AgentTraceVcsType>,
        remote_url: Option<String>,
    },
    PostRewrite {
        rewrite_method: String,
    },
    DiffTrace,
    ConversationTrace,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct DiffTracePayload {
    #[serde(rename = "sessionID")]
    session_id: String,
    diff: String,
    time: u64,
    model_id: String,
    tool_name: String,
    tool_version: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConversationTracePayload {
    MessageUpdated(ConversationTraceMessageBatch),
    MessagePartUpdated(ConversationTracePartBatch),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationTraceMessageBatch {
    pub inserts: Vec<InsertMessageInsert>,
    pub skipped: Vec<SkippedConversationTracePayload>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationTracePartBatch {
    pub inserts: Vec<InsertPartInsert>,
    pub skipped: Vec<SkippedConversationTracePayload>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkippedConversationTracePayload {
    pub index: usize,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConversationTracePersistenceSummary {
    event_type: &'static str,
    attempted_count: usize,
    persisted_count: usize,
    skipped_count: usize,
}

impl ConversationTracePersistenceSummary {
    fn render(&self) -> String {
        format!(
            "conversation-trace hook persisted {} payload batch to AgentTraceDb: attempted={}, persisted={}, skipped={}.",
            self.event_type, self.attempted_count, self.persisted_count, self.skipped_count
        )
    }
}

/// Required `sce hooks diff-trace` STDIN payload shape:
/// `{ sessionID, diff, time, model_id, tool_name, tool_version }`.
///
/// Validation contract:
/// - `sessionID`, `diff`, `model_id`, and `tool_name` must be non-empty strings.
/// - `time` must be a `u64` Unix epoch millisecond value.
/// - `tool_version` must be present and either `null` or a non-empty string.
pub fn run_hooks_subcommand(
    subcommand: &HookSubcommand,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let repository_root = std::env::current_dir().with_context(|| {
        format!(
            "Failed to determine current directory for {}.",
            hook_runtime_invocation_name(subcommand)
        )
    })?;

    run_hooks_subcommand_in_repo(&repository_root, subcommand, logger)
}

fn run_hooks_subcommand_in_repo(
    repository_root: &Path,
    subcommand: &HookSubcommand,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    match subcommand {
        HookSubcommand::PreCommit => run_pre_commit_subcommand_with_trace(repository_root),
        HookSubcommand::CommitMsg { message_file } => {
            run_commit_msg_subcommand_with_trace(repository_root, subcommand, message_file)
        }
        HookSubcommand::PostCommit {
            vcs_type,
            remote_url,
        } => run_post_commit_subcommand_with_trace(repository_root, *vcs_type, remote_url.clone()),
        HookSubcommand::PostRewrite { rewrite_method } => {
            run_post_rewrite_subcommand_with_trace(repository_root, subcommand, rewrite_method)
        }
        HookSubcommand::DiffTrace => run_diff_trace_subcommand(repository_root, logger),
        HookSubcommand::ConversationTrace => run_conversation_trace_subcommand(logger),
    }
}

fn run_conversation_trace_subcommand(logger: Option<&dyn Logger>) -> Result<String> {
    let stdin_payload = read_hook_stdin()?;
    let result = run_conversation_trace_subcommand_from_payload(&stdin_payload, logger);
    if let Err(ref error) = result {
        if let Some(log) = logger {
            log.error(
                "sce.hooks.conversation_trace.error",
                &error.to_string(),
                &[],
            );
        }
    }
    result
}

fn run_conversation_trace_subcommand_from_payload(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let payload = parse_conversation_trace_payload(stdin_payload)?;
    persist_conversation_trace_payload_to_agent_trace_db(payload, logger)
}

fn persist_conversation_trace_payload_to_agent_trace_db(
    payload: ConversationTracePayload,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let db = AgentTraceDb::new()
        .context("Failed to open Agent Trace DB for conversation-trace persistence.")?;

    let summary = match payload {
        ConversationTracePayload::MessageUpdated(batch) => {
            persist_message_updated_batch_to_agent_trace_db(&db, batch, logger)
        }
        ConversationTracePayload::MessagePartUpdated(batch) => {
            persist_message_part_updated_batch_to_agent_trace_db(&db, batch, logger)
        }
    };

    Ok(summary.render())
}

fn persist_message_updated_batch_to_agent_trace_db(
    db: &AgentTraceDb,
    batch: ConversationTraceMessageBatch,
    logger: Option<&dyn Logger>,
) -> ConversationTracePersistenceSummary {
    const EVENT_TYPE: &str = "message.updated";

    let attempted_count = batch.inserts.len() + batch.skipped.len();
    let mut skipped_count = batch.skipped.len();

    log_skipped_conversation_trace_payloads(logger, EVENT_TYPE, &batch.skipped);

    let valid_count = batch.inserts.len();
    let persisted_count = if valid_count == 0 {
        0
    } else {
        match db.insert_messages(batch.inserts) {
            Ok(affected_rows) => usize::try_from(affected_rows)
                .unwrap_or(usize::MAX)
                .min(valid_count),
            Err(error) => {
                skipped_count += valid_count;
                log_conversation_trace_batch_insert_failure(
                    logger,
                    EVENT_TYPE,
                    valid_count,
                    &error,
                );
                0
            }
        }
    };

    ConversationTracePersistenceSummary {
        event_type: EVENT_TYPE,
        attempted_count,
        persisted_count,
        skipped_count,
    }
}

fn persist_message_part_updated_batch_to_agent_trace_db(
    db: &AgentTraceDb,
    batch: ConversationTracePartBatch,
    logger: Option<&dyn Logger>,
) -> ConversationTracePersistenceSummary {
    const EVENT_TYPE: &str = "message.part.updated";

    let attempted_count = batch.inserts.len() + batch.skipped.len();
    let mut skipped_count = batch.skipped.len();

    log_skipped_conversation_trace_payloads(logger, EVENT_TYPE, &batch.skipped);

    let valid_count = batch.inserts.len();
    let persisted_count = if valid_count == 0 {
        0
    } else {
        match db.insert_parts(batch.inserts) {
            Ok(affected_rows) => usize::try_from(affected_rows)
                .unwrap_or(usize::MAX)
                .min(valid_count),
            Err(error) => {
                skipped_count += valid_count;
                log_conversation_trace_batch_insert_failure(
                    logger,
                    EVENT_TYPE,
                    valid_count,
                    &error,
                );
                0
            }
        }
    };

    ConversationTracePersistenceSummary {
        event_type: EVENT_TYPE,
        attempted_count,
        persisted_count,
        skipped_count,
    }
}

fn log_skipped_conversation_trace_payloads(
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
        );
    }
}

fn log_conversation_trace_batch_insert_failure(
    logger: Option<&dyn Logger>,
    event_type: &str,
    valid_count: usize,
    error: &anyhow::Error,
) {
    if let Some(log) = logger {
        let count = valid_count.to_string();
        log.warn(
            "sce.hooks.conversation_trace.agent_trace_db_batch_failed",
            &error.to_string(),
            &[("event_type", event_type), ("valid_count", count.as_str())],
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
    let event_type = required_string_field(payload, "type", conversation_trace_validation_error)?;
    let payloads = required_payloads_array(payload)?;

    match event_type.as_str() {
        "message.updated" => parse_message_updated_payloads(payloads),
        "message.part.updated" => parse_message_part_updated_payloads(payloads),
        _ => bail!(conversation_trace_validation_error(
            "field 'type' must be one of 'message.updated' or 'message.part.updated'"
        )),
    }
}

fn required_payloads_array(payload: &serde_json::Map<String, Value>) -> Result<&Vec<Value>> {
    required_field(payload, "payloads", conversation_trace_validation_error)?
        .as_array()
        .ok_or_else(|| {
            anyhow!(conversation_trace_validation_error(
                "field 'payloads' must be an array"
            ))
        })
}

fn parse_message_updated_payloads(payloads: &[Value]) -> Result<ConversationTracePayload> {
    let mut inserts = Vec::new();
    let mut skipped = Vec::new();

    for (index, item) in payloads.iter().enumerate() {
        let Some(item) = conversation_trace_payload_item(item, index, &mut skipped)? else {
            continue;
        };
        match parse_message_updated_item(item) {
            Ok(input) => inserts.push(input),
            Err(error) => skipped.push(SkippedConversationTracePayload {
                index,
                reason: error.to_string(),
            }),
        }
    }

    Ok(ConversationTracePayload::MessageUpdated(
        ConversationTraceMessageBatch { inserts, skipped },
    ))
}

fn parse_message_part_updated_payloads(payloads: &[Value]) -> Result<ConversationTracePayload> {
    let mut inserts = Vec::new();
    let mut skipped = Vec::new();

    for (index, item) in payloads.iter().enumerate() {
        let Some(item) = conversation_trace_payload_item(item, index, &mut skipped)? else {
            continue;
        };
        match parse_message_part_updated_item(item) {
            Ok(input) => inserts.push(input),
            Err(error) => skipped.push(SkippedConversationTracePayload {
                index,
                reason: error.to_string(),
            }),
        }
    }

    Ok(ConversationTracePayload::MessagePartUpdated(
        ConversationTracePartBatch { inserts, skipped },
    ))
}

fn conversation_trace_payload_item<'a>(
    item: &'a Value,
    index: usize,
    skipped: &mut Vec<SkippedConversationTracePayload>,
) -> Result<Option<&'a serde_json::Map<String, Value>>> {
    let Some(payload) = item.as_object() else {
        skipped.push(SkippedConversationTracePayload {
            index,
            reason: conversation_trace_validation_error(&format!(
                "payloads[{index}] must be an object"
            )),
        });
        return Ok(None);
    };

    if payload.contains_key("type") {
        bail!(conversation_trace_validation_error(&format!(
            "payloads[{index}] must not declare its own 'type'; use the top-level 'type' for homogeneous batches"
        )));
    }

    Ok(Some(payload))
}

fn parse_message_updated_item(
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

fn parse_message_part_updated_item(
    payload: &serde_json::Map<String, Value>,
) -> Result<InsertPartInsert> {
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
        part_type: parse_part_type(payload)?,
        text: required_string_field(payload, "text", conversation_trace_validation_error)?,
        generated_at_unix_ms: required_i64_millisecond_field(
            payload,
            "generated_at_unix_ms",
            conversation_trace_validation_error,
        )?,
    })
}

fn parse_message_role(payload: &serde_json::Map<String, Value>) -> Result<MessageRole> {
    match required_string_field(payload, "role", conversation_trace_validation_error)?.as_str() {
        "user" => Ok(MessageRole::User),
        "assistant" => Ok(MessageRole::Assistant),
        _ => bail!(conversation_trace_validation_error(
            "field 'role' must be one of 'user' or 'assistant'"
        )),
    }
}

fn parse_part_type(payload: &serde_json::Map<String, Value>) -> Result<PartType> {
    match required_string_field(payload, "part_type", conversation_trace_validation_error)?.as_str()
    {
        "text" => Ok(PartType::Text),
        "reasoning" => Ok(PartType::Reasoning),
        "patch" => Ok(PartType::Patch),
        _ => bail!(conversation_trace_validation_error(
            "field 'part_type' must be one of 'text', 'reasoning' or 'patch'"
        )),
    }
}

fn conversation_trace_validation_error(detail: &str) -> String {
    format!("Invalid conversation-trace payload from STDIN: {detail}.")
}

fn run_diff_trace_subcommand(
    repository_root: &Path,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let stdin_payload = read_hook_stdin()?;
    let result = run_diff_trace_subcommand_from_payload(repository_root, &stdin_payload, logger);
    if let Err(ref error) = result {
        if let Some(log) = logger {
            log.error("sce.hooks.diff_trace.error", &error.to_string(), &[]);
        }
    }
    result
}

fn run_diff_trace_subcommand_from_payload(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let payload = parse_diff_trace_payload(stdin_payload)?;
    if let Err(error) = diff_trace_db_time_ms(payload.time) {
        if let Some(log) = logger {
            log.warn(
                "sce.hooks.diff_trace.agent_trace_db_time_invalid",
                &error.to_string(),
                &[],
            );
        }
    }
    persist_diff_trace_payload(repository_root, &payload)?;
    let agent_trace_db_result = persist_diff_trace_payload_to_agent_trace_db(&payload);
    let agent_trace_db_persisted = match agent_trace_db_result {
        Ok(()) => true,
        Err(error) => {
            if let Some(log) = logger {
                log.warn(
                    "sce.hooks.diff_trace.agent_trace_db_write_failed",
                    &error.to_string(),
                    &[],
                );
            }
            false
        }
    };

    if agent_trace_db_persisted {
        Ok(String::from(
            "diff-trace hook intake persisted payload to AgentTraceDb and context/tmp.",
        ))
    } else {
        Ok(String::from(
            "diff-trace hook intake persisted payload to context/tmp; AgentTraceDb persistence failed.",
        ))
    }
}

fn parse_diff_trace_payload(stdin_payload: &str) -> Result<DiffTracePayload> {
    let parsed: Value = serde_json::from_str(stdin_payload)
        .context("Invalid diff-trace payload from STDIN: expected valid JSON.")?;
    let payload = parsed
        .as_object()
        .ok_or_else(|| anyhow!(diff_trace_validation_error("expected a JSON object")))?;

    let session_id =
        required_non_empty_string_field(payload, "sessionID", diff_trace_validation_error)?;
    let diff = required_non_empty_string_field(payload, "diff", diff_trace_validation_error)?;
    let time = required_u64_millisecond_field(payload, "time", diff_trace_validation_error)?;
    let model_id =
        required_non_empty_string_field(payload, "model_id", diff_trace_validation_error)?;
    let tool_name =
        required_non_empty_string_field(payload, "tool_name", diff_trace_validation_error)?;
    let tool_version = required_nullable_or_non_empty_string_field(
        payload,
        "tool_version",
        diff_trace_validation_error,
    )?;

    Ok(DiffTracePayload {
        session_id,
        diff,
        time,
        model_id,
        tool_name,
        tool_version,
    })
}

fn required_nullable_or_non_empty_string_field(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
    validation_error: PayloadValidationError,
) -> Result<Option<String>> {
    let raw = required_field(payload, field_name, validation_error)?;

    if raw.is_null() {
        return Ok(None);
    }

    let value = raw.as_str().ok_or_else(|| {
        anyhow!(validation_error(&format!(
            "field '{field_name}' must be null or a non-empty string"
        )))
    })?;

    if value.trim().is_empty() {
        bail!(validation_error(&format!(
            "field '{field_name}' must be null or a non-empty string"
        )));
    }

    Ok(Some(value.to_string()))
}

fn required_non_empty_string_field(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
    validation_error: PayloadValidationError,
) -> Result<String> {
    let value = required_string_field(payload, field_name, validation_error)?;

    if value.trim().is_empty() {
        bail!(validation_error(&format!(
            "field '{field_name}' must be a non-empty string"
        )));
    }

    Ok(value)
}

fn required_string_field(
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
fn required_u64_millisecond_field(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
    validation_error: PayloadValidationError,
) -> Result<u64> {
    let raw = required_field(payload, field_name, validation_error)?;

    if let Some(value) = raw.as_u64() {
        return Ok(value);
    }

    if let Some(value) = raw.as_i64() {
        if value < 0 {
            bail!(validation_error(&format!(
                "field '{field_name}' must be a u64 Unix epoch millisecond value, got a negative number"
            )));
        }
        return Ok(value as u64);
    }

    if let Some(value) = raw.as_f64() {
        if value.fract() != 0.0 {
            bail!(validation_error(&format!(
                "field '{field_name}' must be a u64 Unix epoch millisecond value, got a fractional number"
            )));
        }
        if value < 0.0 {
            bail!(validation_error(&format!(
                "field '{field_name}' must be a u64 Unix epoch millisecond value, got a negative number"
            )));
        }
        if value > u64::MAX as f64 {
            bail!(validation_error(&format!(
                "field '{field_name}' must be a u64 Unix epoch millisecond value"
            )));
        }
        return Ok(value as u64);
    }

    bail!(validation_error(&format!(
        "field '{field_name}' must be a u64 Unix epoch millisecond value"
    )))
}

fn required_i64_millisecond_field(
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

fn required_field<'a>(
    payload: &'a serde_json::Map<String, Value>,
    field_name: &str,
    validation_error: PayloadValidationError,
) -> Result<&'a Value> {
    payload.get(field_name).ok_or_else(|| {
        anyhow!(validation_error(&format!(
            "missing required field '{field_name}'"
        )))
    })
}

fn diff_trace_validation_error(detail: &str) -> String {
    format!("Invalid diff-trace payload from STDIN: {detail}.")
}

fn persist_diff_trace_payload(
    repository_root: &Path,
    payload: &DiffTracePayload,
) -> Result<PathBuf> {
    let trace_directory = repository_root.join("context").join("tmp");
    let serialized = format!(
        "{}\n",
        serde_json::to_string_pretty(payload)
            .context("Failed to serialize diff-trace payload for persistence.")?
    );

    persist_serialized_trace_payload(
        &trace_directory,
        "diff-trace",
        &serialized,
        "diff-trace payload",
    )
}

fn persist_diff_trace_payload_to_agent_trace_db(payload: &DiffTracePayload) -> Result<()> {
    persist_diff_trace_payload_to_agent_trace_db_with(payload, |input| {
        let db = AgentTraceDb::new()
            .context("Failed to open Agent Trace DB for diff-trace persistence.")?;
        db.insert_diff_trace(input)
            .context("Failed to persist diff-trace payload to Agent Trace DB.")?;

        Ok(())
    })
}

fn persist_diff_trace_payload_to_agent_trace_db_with<F>(
    payload: &DiffTracePayload,
    insert_fn: F,
) -> Result<()>
where
    F: FnOnce(DiffTraceInsert<'_>) -> Result<()>,
{
    let time_ms = diff_trace_db_time_ms(payload.time)?;

    insert_fn(DiffTraceInsert {
        time_ms,
        session_id: &payload.session_id,
        patch: &payload.diff,
        model_id: &payload.model_id,
        tool_name: &payload.tool_name,
        tool_version: payload.tool_version.as_deref(),
    })
}

fn diff_trace_db_time_ms(time: u64) -> Result<i64> {
    i64::try_from(time).map_err(|_| {
        anyhow!(diff_trace_validation_error(
            "field 'time' must fit in a signed 64-bit Unix epoch millisecond value for Agent Trace DB storage"
        ))
    })
}

fn persist_serialized_trace_payload(
    trace_directory: &Path,
    trace_name: &str,
    serialized: &str,
    artifact_description: &str,
) -> Result<PathBuf> {
    fs::create_dir_all(trace_directory).with_context(|| {
        format!(
            "Failed to create hook trace directory '{}'.",
            trace_directory.display()
        )
    })?;

    let timestamp = Utc::now();

    for attempt in 0..MAX_TRACE_FILE_CREATE_ATTEMPTS {
        let file_path = trace_directory.join(build_trace_file_name(trace_name, timestamp, attempt));

        match persist_trace_payload_to_file(&file_path, serialized) {
            Ok(()) => return Ok(file_path),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Failed to write {artifact_description} file '{}'.",
                        file_path.display()
                    )
                });
            }
        }
    }

    bail!(
        "Failed to write {artifact_description} file in '{}': exhausted {} collision-safe filename attempts.",
        trace_directory.display(),
        MAX_TRACE_FILE_CREATE_ATTEMPTS
    )
}

fn persist_trace_payload_to_file(file_path: &Path, serialized: &str) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file_path)?;
    file.write_all(serialized.as_bytes())?;

    Ok(())
}

fn format_trace_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.format("%Y-%m-%dT%H-%M-%S-%3fZ").to_string()
}

fn build_trace_file_name(trace_name: &str, timestamp: DateTime<Utc>, attempt: u64) -> String {
    let safe_name = sanitize_trace_name(trace_name);

    format!(
        "{}-{:06}-{}.json",
        format_trace_timestamp(timestamp),
        attempt,
        safe_name
    )
}

fn sanitize_trace_name(trace_name: &str) -> String {
    trace_name
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' | '-' => character,
            _ => '_',
        })
        .collect()
}

fn run_pre_commit_subcommand_with_trace(repository_root: &Path) -> Result<String> {
    run_pre_commit_subcommand(repository_root)
}

fn run_pre_commit_subcommand(repository_root: &Path) -> Result<String> {
    let runtime = resolve_runtime_state(repository_root)?;

    Ok(format!(
        "pre-commit hook executed with no-op runtime state: {:?}",
        pre_commit_no_op_reason(&runtime)
    ))
}

fn run_commit_msg_subcommand_in_repo(
    repository_root: &Path,
    message_file: &Path,
) -> Result<String> {
    let metadata = fs::metadata(message_file).with_context(|| {
        format!(
            "Invalid commit message file '{}': file does not exist or is not readable.",
            message_file.display()
        )
    })?;

    if !metadata.is_file() {
        bail!(
            "Invalid commit message file '{}': expected a regular file path.",
            message_file.display()
        );
    }

    let runtime = resolve_runtime_state(repository_root)?;
    let original = fs::read_to_string(message_file).with_context(|| {
        format!(
            "Invalid commit message file '{}': failed to read UTF-8 content.",
            message_file.display()
        )
    })?;

    let gate_passed = commit_msg_policy_gate_passed(&runtime);
    let transformed = apply_commit_msg_coauthor_policy(&runtime, &original);
    let trailer_applied = gate_passed && transformed != original;

    if trailer_applied {
        fs::write(message_file, transformed.as_bytes()).with_context(|| {
            format!(
                "Failed to update commit message file '{}' with canonical co-author trailer.",
                message_file.display()
            )
        })?;
    }

    Ok(format!(
        "commit-msg hook processed message file '{}' (policy_gate_passed={}, trailer_applied={}).",
        message_file.display(),
        gate_passed,
        trailer_applied
    ))
}

fn run_commit_msg_subcommand_with_trace(
    repository_root: &Path,
    _: &HookSubcommand,
    message_file: &Path,
) -> Result<String> {
    run_commit_msg_subcommand_in_repo(repository_root, message_file)
}

fn run_post_commit_subcommand(
    repository_root: &Path,
    vcs_type: Option<AgentTraceVcsType>,
    remote_url: &str,
) -> Result<String> {
    run_post_commit_subcommand_with(
        repository_root,
        vcs_type,
        remote_url,
        run_post_commit_intersection_flow,
        run_post_commit_agent_trace_flow,
    )
}

fn run_post_commit_subcommand_with<F, B>(
    repository_root: &Path,
    vcs_type: Option<AgentTraceVcsType>,
    remote_url: &str,
    run_intersection_flow: F,
    run_agent_trace_flow: B,
) -> Result<String>
where
    F: FnOnce(&Path) -> Result<PostCommitIntersectionFlowResult>,
    B: FnOnce(
        &Path,
        &PostCommitIntersectionFlowResult,
        Option<AgentTraceVcsType>,
        &str,
    ) -> Result<AgentTrace>,
{
    let result = run_intersection_flow(repository_root)?;
    let _agent_trace = run_agent_trace_flow(repository_root, &result, vcs_type, remote_url)?;

    Ok(format!(
        "post-commit hook processed intersection: commit={}, intersection_files={}",
        result.post_commit_data.commit_oid,
        result.combined_recent_patch.files.len()
    ))
}

fn run_post_commit_agent_trace_flow(
    _repository_root: &Path,
    flow_result: &PostCommitIntersectionFlowResult,
    vcs_type: Option<AgentTraceVcsType>,
    remote_url: &str,
) -> Result<AgentTrace> {
    let db = AgentTraceDb::new().context("Failed to open Agent Trace DB for post-commit trace.")?;

    run_post_commit_agent_trace_flow_with(
        flow_result,
        vcs_type,
        remote_url,
        |trace_value| {
            validate_agent_trace_value(trace_value)
                .map_err(|error| anyhow!(error.to_string()))
                .context("Failed to verify built post-commit Agent Trace payload.")?;

            Ok(())
        },
        |insert_input| {
            db.insert_agent_trace(insert_input)
                .context("Failed to persist built post-commit Agent Trace payload.")?;

            Ok(())
        },
    )
}

fn run_post_commit_agent_trace_flow_with<V, I>(
    flow_result: &PostCommitIntersectionFlowResult,
    vcs_type: Option<AgentTraceVcsType>,
    remote_url: &str,
    validate_agent_trace: V,
    persist_agent_trace: I,
) -> Result<AgentTrace>
where
    V: FnOnce(&Value) -> Result<()>,
    I: for<'a> FnOnce(AgentTraceInsert<'a>) -> Result<()>,
{
    let commit_timestamp =
        DateTime::<Utc>::from_timestamp_millis(flow_result.post_commit_data.commit_time_ms)
            .ok_or_else(|| {
                anyhow!(
            "Invalid post-commit timestamp '{}': expected a valid Unix epoch millisecond value.",
            flow_result.post_commit_data.commit_time_ms
        )
            })?
            .to_rfc3339();

    let agent_trace = build_agent_trace(
        &flow_result.combined_recent_patch,
        &flow_result.post_commit_data.parsed_patch,
        AgentTraceMetadataInput {
            commit_timestamp: &commit_timestamp,
            commit_revision: &flow_result.post_commit_data.commit_oid,
            vcs_type,
            tool_name: flow_result.tool_name.as_deref(),
            tool_version: flow_result.tool_version.as_deref(),
        },
    )
    .context("Failed to build Agent Trace payload from post-commit intersection flow result.")?;

    let agent_trace_value = serde_json::to_value(&agent_trace)
        .context("Failed to serialize post-commit Agent Trace payload for validation.")?;
    validate_agent_trace(&agent_trace_value)
        .context("Failed to validate built post-commit Agent Trace payload.")?;

    let serialized = format!(
        "{}\n",
        serde_json::to_string_pretty(&agent_trace)
            .context("Failed to serialize post-commit Agent Trace payload for persistence.")?
    );

    let constructed_url = format!("{AGENT_TRACE_URL_PREFIX}{}", agent_trace.id);

    let insert_input = AgentTraceInsert {
        commit_id: &flow_result.post_commit_data.commit_oid,
        commit_time_ms: flow_result.post_commit_data.commit_time_ms,
        trace_json: &serialized,
        agent_trace_id: &agent_trace.id,
        url: &constructed_url,
        remote_url,
    };
    persist_agent_trace(insert_input)?;

    Ok(agent_trace)
}

/// Duration for looking up recent diff traces: 7 days in milliseconds.
const RECENT_DAYS_MILLIS: i64 = 7 * 24 * 60 * 60 * 1000;

fn run_post_commit_intersection_flow(
    repository_root: &Path,
) -> Result<PostCommitIntersectionFlowResult> {
    let db = AgentTraceDb::new()
        .context("Failed to open Agent Trace DB for post-commit intersection.")?;

    run_post_commit_intersection_flow_with(
        repository_root,
        capture_post_commit_patch_from_git,
        current_unix_time_ms,
        |cutoff_ms, end_ms| {
            db.recent_diff_trace_patches(cutoff_ms, end_ms)
                .context("Failed to query recent diff trace patches.")
        },
        |insert_input| {
            db.insert_post_commit_patch_intersection(insert_input)
                .context("Failed to persist post-commit patch intersection.")?;

            Ok(())
        },
    )
}

fn run_post_commit_intersection_flow_with<C, N, Q, P>(
    repository_root: &Path,
    capture_post_commit_patch: C,
    now_ms: N,
    query_recent_patches: Q,
    persist_intersection: P,
) -> Result<PostCommitIntersectionFlowResult>
where
    C: FnOnce(&Path) -> Result<PostCommitPatchData>,
    N: FnOnce() -> Result<i64>,
    Q: FnOnce(i64, i64) -> Result<RecentDiffTracePatches>,
    P: for<'a> FnOnce(PostCommitPatchIntersectionInsert<'a>) -> Result<()>,
{
    let post_commit_data = capture_post_commit_patch(repository_root)?;

    let now_ms = now_ms()?;
    let cutoff_ms = now_ms - RECENT_DAYS_MILLIS;

    let recent_patches = query_recent_patches(cutoff_ms, now_ms)?;

    #[allow(clippy::cast_possible_wrap)]
    let loaded_count = recent_patches.loaded_count() as i64;
    #[allow(clippy::cast_possible_wrap)]
    let skipped_count = recent_patches.skipped_count() as i64;

    let last_patch = recent_patches.patches.last();
    let tool_name = last_patch.and_then(|patch| patch.tool_name.clone());
    let tool_version = last_patch.and_then(|patch| patch.tool_version.clone());

    let recent_patches_slice: Vec<ParsedPatch> = recent_patches
        .patches
        .into_iter()
        .map(|p| p.patch)
        .collect();

    let combined_recent_patch = combine_patches_fn(&recent_patches_slice);

    let intersection_patch =
        intersect_patches_fn(&combined_recent_patch, &post_commit_data.parsed_patch);

    let serialized_intersection = serialize_to_json(&intersection_patch)
        .context("Failed to serialize intersection patch.")?;

    let insert_input = PostCommitPatchIntersectionInsert {
        commit_id: &post_commit_data.commit_oid,
        post_commit_time_ms: post_commit_data.commit_time_ms,
        recent_window_cutoff_ms: cutoff_ms,
        recent_window_end_ms: now_ms,
        loaded_diff_trace_count: loaded_count,
        skipped_diff_trace_count: skipped_count,
        intersection_patch: &serialized_intersection,
    };

    persist_intersection(insert_input)?;

    Ok(PostCommitIntersectionFlowResult {
        combined_recent_patch,
        post_commit_data,
        tool_name,
        tool_version,
    })
}

fn current_unix_time_ms() -> Result<i64> {
    i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())
        .context("Current time exceeds i64 range for post-commit intersection.")
}

fn run_post_commit_subcommand_with_trace(
    repository_root: &Path,
    vcs_type: Option<AgentTraceVcsType>,
    remote_url: Option<String>,
) -> Result<String> {
    let remote_url_value = remote_url.clone().unwrap_or_default();
    let subcommand = HookSubcommand::PostCommit {
        vcs_type,
        remote_url,
    };
    let input = build_hook_trace_input_for_post_commit(repository_root);
    let outcome = run_post_commit_subcommand(repository_root, vcs_type, &remote_url_value);

    let _ = persist_hook_trace(repository_root, &subcommand, &input, &outcome);

    outcome
}

fn run_post_rewrite_subcommand(repository_root: &Path, rewrite_method: &str) -> Result<String> {
    let runtime = resolve_runtime_state(repository_root)?;

    Ok(format!(
        "post-rewrite hook executed with no-op runtime state: {:?} (rewrite_method='{}')",
        post_rewrite_no_op_reason(&runtime),
        rewrite_method.trim()
    ))
}

fn run_post_rewrite_subcommand_with_trace(
    repository_root: &Path,
    _: &HookSubcommand,
    rewrite_method: &str,
) -> Result<String> {
    let stdin_payload = read_hook_stdin();
    stdin_payload.and_then(|_| run_post_rewrite_subcommand(repository_root, rewrite_method))
}

fn hook_runtime_invocation_name(subcommand: &HookSubcommand) -> &'static str {
    match subcommand {
        HookSubcommand::PreCommit => "pre-commit runtime invocation",
        HookSubcommand::CommitMsg { .. } => "commit-msg runtime invocation",
        HookSubcommand::PostCommit { .. } => "post-commit runtime invocation",
        HookSubcommand::PostRewrite { .. } => "post-rewrite runtime invocation",
        HookSubcommand::DiffTrace => "diff-trace runtime invocation",
        HookSubcommand::ConversationTrace => "conversation-trace runtime invocation",
    }
}

fn persist_hook_trace(
    repository_root: &Path,
    subcommand: &HookSubcommand,
    input: &Value,
    outcome: &Result<String>,
) -> Result<()> {
    let trace_directory = repository_root.join("context").join("tmp");
    let body = match outcome {
        Ok(output) => json!({
            "input": input,
            "output": output,
        }),
        Err(error) => json!({
            "input": input,
            "error": error.to_string(),
        }),
    };

    let serialized = format!(
        "{}\n",
        serde_json::to_string_pretty(&body).context("Failed to serialize hook trace.")?
    );
    persist_serialized_trace_payload(
        &trace_directory,
        hook_trace_name(subcommand),
        &serialized,
        "hook trace",
    )?;

    Ok(())
}

fn hook_trace_name(subcommand: &HookSubcommand) -> &'static str {
    match subcommand {
        HookSubcommand::PreCommit => "pre-commit",
        HookSubcommand::CommitMsg { .. } => "commit-msg",
        HookSubcommand::PostCommit { .. } => "post-commit",
        HookSubcommand::PostRewrite { .. } => "post-rewrite",
        HookSubcommand::DiffTrace => "diff-trace",
        HookSubcommand::ConversationTrace => "conversation-trace",
    }
}

fn build_hook_trace_input_for_post_commit(repository_root: &Path) -> Value {
    let mut input = build_base_hook_trace_input("post-commit");
    insert_head_commit_from_git(repository_root, &mut input);
    Value::Object(input)
}

fn build_base_hook_trace_input(hook_name: &str) -> serde_json::Map<String, Value> {
    let mut input = serde_json::Map::new();
    input.insert("hook".to_string(), Value::String(hook_name.to_string()));
    input.insert(
        "git_env".to_string(),
        Value::Object(
            collect_git_environment()
                .into_iter()
                .map(|(key, value)| (key, Value::String(value)))
                .collect(),
        ),
    );
    input
}

fn collect_git_environment() -> BTreeMap<String, String> {
    std::env::vars()
        .filter(|(key, _)| key.starts_with("GIT_"))
        .collect()
}

fn read_hook_stdin() -> Result<String> {
    let mut stdin_payload = String::new();
    io::stdin()
        .read_to_string(&mut stdin_payload)
        .context("Failed to read hook input from STDIN.")?;
    Ok(stdin_payload)
}

fn insert_head_commit_from_git(repository_root: &Path, input: &mut serde_json::Map<String, Value>) {
    insert_git_output(
        repository_root,
        &["rev-parse", "HEAD"],
        "Failed to capture HEAD revision from git.",
        input,
        "head_oid_from_git",
        "head_oid_from_git_read_error",
    );
    insert_git_output(
        repository_root,
        &["show", "--format=", "--patch", "--no-ext-diff", "HEAD"],
        "Failed to capture HEAD patch from git.",
        input,
        "head_patch_from_git",
        "head_patch_from_git_read_error",
    );
}

fn insert_git_output(
    repository_root: &Path,
    args: &[&str],
    context_message: &str,
    input: &mut serde_json::Map<String, Value>,
    output_key: &str,
    error_key: &str,
) {
    match run_git_command_capture_stdout(repository_root, args, context_message) {
        Ok(stdout) => {
            input.insert(output_key.to_string(), Value::String(stdout));
        }
        Err(error) => {
            input.insert(error_key.to_string(), Value::String(error.to_string()));
        }
    }
}

fn run_git_command_capture_stdout(
    repository_root: &Path,
    args: &[&str],
    context_message: &str,
) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository_root)
        .output()
        .with_context(|| {
            format!(
                "{} (directory: '{}')",
                context_message,
                repository_root.display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let diagnostic = if stderr.is_empty() {
            String::from("git command exited with a non-zero status")
        } else {
            stderr
        };
        bail!("{context_message} {diagnostic}");
    }

    String::from_utf8(output.stdout).context("git command output contained invalid UTF-8")
}

fn resolve_runtime_state(repository_root: &Path) -> Result<HookRuntimeState> {
    Ok(HookRuntimeState {
        sce_disabled: env_flag_is_truthy("SCE_DISABLED"),
        attribution_hooks_enabled: config::resolve_hook_runtime_config(repository_root)?
            .attribution_hooks_enabled,
    })
}

fn env_flag_is_truthy(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| env_value_is_truthy(&value))
}

fn env_value_is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn commit_msg_policy_gate_passed(runtime: &HookRuntimeState) -> bool {
    !runtime.sce_disabled && runtime.attribution_hooks_enabled
}

fn pre_commit_no_op_reason(runtime: &HookRuntimeState) -> HookNoOpReason {
    if runtime.sce_disabled {
        HookNoOpReason::Disabled
    } else {
        HookNoOpReason::AttributionOnlyCommitMsgMode
    }
}

fn post_rewrite_no_op_reason(runtime: &HookRuntimeState) -> HookNoOpReason {
    if runtime.sce_disabled {
        HookNoOpReason::Disabled
    } else {
        HookNoOpReason::AttributionOnlyCommitMsgMode
    }
}

pub fn apply_commit_msg_coauthor_policy(
    runtime: &HookRuntimeState,
    commit_message: &str,
) -> String {
    if !commit_msg_policy_gate_passed(runtime) {
        return commit_message.to_string();
    }

    let mut lines: Vec<&str> = commit_message.lines().collect();
    lines.retain(|line| *line != CANONICAL_SCE_COAUTHOR_TRAILER);

    if !lines.is_empty() && !lines.last().is_some_and(|line| line.is_empty()) {
        lines.push("");
    }
    lines.push(CANONICAL_SCE_COAUTHOR_TRAILER);

    let mut normalized = lines.join("\n");
    if commit_message.ends_with('\n') {
        normalized.push('\n');
    }

    normalized
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HookRuntimeState {
    pub sce_disabled: bool,
    pub attribution_hooks_enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookNoOpReason {
    Disabled,
    AttributionOnlyCommitMsgMode,
}

/// Post-commit patch data captured from git for intersection flows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostCommitPatchData {
    pub commit_oid: String,
    pub commit_time_ms: i64,
    pub parsed_patch: ParsedPatch,
}

/// Structured post-commit intersection flow result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostCommitIntersectionFlowResult {
    pub combined_recent_patch: ParsedPatch,
    pub post_commit_data: PostCommitPatchData,
    pub tool_name: Option<String>,
    pub tool_version: Option<String>,
}

/// Capture and parse the current commit patch.
pub fn capture_post_commit_patch_from_git(repository_root: &Path) -> Result<PostCommitPatchData> {
    let commit_oid = capture_head_oid_from_git(repository_root)?;
    let commit_time_ms = capture_head_timestamp_from_git(repository_root)?;
    let patch_text = capture_head_patch_from_git(repository_root)?;
    let parsed_patch = parse_patch_from_text(&patch_text, None).map_err(|e| {
        anyhow!(post_commit_patch_error(
            "failed to parse post-commit patch",
            &e.to_string()
        ))
    })?;

    Ok(PostCommitPatchData {
        commit_oid,
        commit_time_ms,
        parsed_patch,
    })
}

fn capture_head_oid_from_git(repository_root: &Path) -> Result<String> {
    let output = run_git_command_capture_stdout(
        repository_root,
        &["rev-parse", "HEAD"],
        "Failed to capture HEAD commit OID from git.",
    )?;
    Ok(output.trim().to_string())
}

fn capture_head_timestamp_from_git(repository_root: &Path) -> Result<i64> {
    let output = run_git_command_capture_stdout(
        repository_root,
        &["show", "--format=%ct", "--no-patch", "HEAD"],
        "Failed to capture HEAD commit timestamp from git.",
    )?;
    let timestamp_str = output.trim();
    let timestamp_seconds: i64 = timestamp_str.parse().map_err(|_| {
        anyhow!(post_commit_patch_error(
            "failed to parse HEAD timestamp",
            timestamp_str,
        ))
    })?;
    let timestamp_ms = timestamp_seconds.checked_mul(1000).ok_or_else(|| {
        anyhow!(post_commit_patch_error(
            "failed to parse HEAD timestamp",
            timestamp_str,
        ))
    })?;
    Ok(timestamp_ms)
}

fn capture_head_patch_from_git(repository_root: &Path) -> Result<String> {
    run_git_command_capture_stdout(
        repository_root,
        &["show", "--format=", "--patch", "--no-ext-diff", "HEAD"],
        "Failed to capture HEAD patch from git.",
    )
}

fn post_commit_patch_error(detail: &str, context: &str) -> String {
    format!("Post-commit patch capture error: {detail} ({context}).")
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, path::Path};

    use super::*;
    use crate::services::agent_trace_db::{ParsedDiffTracePatch, SkippedDiffTracePatch};

    #[derive(Debug, Eq, PartialEq)]
    struct CapturedPostCommitIntersectionInsert {
        commit_id: String,
        post_commit_time_ms: i64,
        recent_window_cutoff_ms: i64,
        recent_window_end_ms: i64,
        loaded_diff_trace_count: i64,
        skipped_diff_trace_count: i64,
        intersection_patch: String,
    }

    fn valid_patch(path: &str, content: &str) -> ParsedPatch {
        let patch_text = format!(
            "Index: {path}\n===================================================================\n--- {path}\n+++ {path}\n@@ -0,0 +1,1 @@\n+{content}\n"
        );

        parse_patch_from_text(&patch_text, None).expect("test patch should parse")
    }

    #[test]
    fn conversation_trace_message_updated_payload_maps_to_message_insert_input() {
        let payload = serde_json::json!({
            "type": "message.updated",
            "payloads": [
                {
                    "session_id": "session-1",
                    "message_id": "message-1",
                    "role": "assistant",
                    "generated_at_unix_ms": 1_800_000_000_000_i64
                },
                {
                    "session_id": "session-2",
                    "message_id": "message-2",
                    "role": "system",
                    "generated_at_unix_ms": 1_800_000_000_002_i64
                }
            ]
        });

        let parsed = parse_conversation_trace_payload(&payload.to_string())
            .expect("conversation-trace message.updated payload should parse");

        let ConversationTracePayload::MessageUpdated(batch) = parsed else {
            panic!("expected message.updated payload");
        };

        assert_eq!(batch.inserts.len(), 1);
        assert_eq!(batch.skipped.len(), 1);
        assert_eq!(batch.skipped[0].index, 1);
        assert!(batch.skipped[0].reason.contains("field 'role'"));
        let input = &batch.inserts[0];
        assert_eq!(input.session_id, "session-1");
        assert_eq!(input.message_id, "message-1");
        assert_eq!(input.role, MessageRole::Assistant);
        assert_eq!(input.generated_at_unix_ms, 1_800_000_000_000_i64);
    }

    #[test]
    fn conversation_trace_message_part_updated_payload_maps_to_part_insert_input() {
        let payload = serde_json::json!({
            "type": "message.part.updated",
            "payloads": [
                {
                    "session_id": "session-1",
                    "message_id": "message-1",
                    "part_type": "reasoning",
                    "text": "thinking through validation",
                    "generated_at_unix_ms": 1_800_000_000_001_i64
                },
                {
                    "session_id": "session-2",
                    "message_id": "message-2",
                    "part_type": "text",
                    "generated_at_unix_ms": 1_800_000_000_002_i64
                }
            ]
        });

        let parsed = parse_conversation_trace_payload(&payload.to_string())
            .expect("conversation-trace message.part.updated payload should parse");

        let ConversationTracePayload::MessagePartUpdated(batch) = parsed else {
            panic!("expected message.part.updated payload");
        };

        assert_eq!(batch.inserts.len(), 1);
        assert_eq!(batch.skipped.len(), 1);
        assert_eq!(batch.skipped[0].index, 1);
        assert!(batch.skipped[0]
            .reason
            .contains("missing required field 'text'"));
        let input = &batch.inserts[0];
        assert_eq!(input.session_id, "session-1");
        assert_eq!(input.message_id, "message-1");
        assert_eq!(input.part_type, PartType::Reasoning);
        assert_eq!(input.text, "thinking through validation");
        assert_eq!(input.generated_at_unix_ms, 1_800_000_000_001_i64);
    }

    #[test]
    fn post_commit_intersection_flow_uses_same_window_end_for_query_and_persistence() {
        let now_ms = 1_800_000_000_000_i64;
        let commit_time_ms = now_ms - 1_000;
        let expected_cutoff_ms = now_ms - RECENT_DAYS_MILLIS;
        let query_window = RefCell::new(None);
        let persisted = RefCell::new(None);

        let output = run_post_commit_intersection_flow_with(
            Path::new("/repo"),
            |_| {
                Ok(PostCommitPatchData {
                    commit_oid: String::from("abc123"),
                    commit_time_ms,
                    parsed_patch: valid_patch("src/lib.rs", "shared line"),
                })
            },
            || Ok(now_ms),
            |cutoff_ms, end_ms| {
                *query_window.borrow_mut() = Some((cutoff_ms, end_ms));

                Ok(RecentDiffTracePatches {
                    patches: vec![ParsedDiffTracePatch {
                        id: 7,
                        time_ms: now_ms - 500,
                        session_id: String::from("valid-session"),
                        patch: valid_patch("src/lib.rs", "shared line"),
                        tool_name: Some(String::from("opencode")),
                        tool_version: Some(String::from("1.2.3")),
                    }],
                    skipped: vec![SkippedDiffTracePatch {
                        id: 8,
                        time_ms: now_ms - 250,
                        session_id: String::from("malformed-session"),
                        reason: String::from("invalid hunk header"),
                    }],
                })
            },
            |insert_input| {
                *persisted.borrow_mut() = Some(CapturedPostCommitIntersectionInsert {
                    commit_id: insert_input.commit_id.to_string(),
                    post_commit_time_ms: insert_input.post_commit_time_ms,
                    recent_window_cutoff_ms: insert_input.recent_window_cutoff_ms,
                    recent_window_end_ms: insert_input.recent_window_end_ms,
                    loaded_diff_trace_count: insert_input.loaded_diff_trace_count,
                    skipped_diff_trace_count: insert_input.skipped_diff_trace_count,
                    intersection_patch: insert_input.intersection_patch.to_string(),
                });

                Ok(())
            },
        )
        .expect("post-commit intersection flow should succeed");

        assert_eq!(
            query_window.into_inner(),
            Some((expected_cutoff_ms, now_ms))
        );

        let persisted = persisted
            .into_inner()
            .expect("intersection row should be persisted");
        assert_eq!(persisted.commit_id, "abc123");
        assert_eq!(persisted.post_commit_time_ms, commit_time_ms);
        assert_eq!(persisted.recent_window_cutoff_ms, expected_cutoff_ms);
        assert_eq!(persisted.recent_window_end_ms, now_ms);
        assert_eq!(persisted.loaded_diff_trace_count, 1);
        assert_eq!(persisted.skipped_diff_trace_count, 1);

        let intersection: ParsedPatch = serde_json::from_str(&persisted.intersection_patch)
            .expect("persisted intersection patch should deserialize");
        assert_eq!(intersection.files.len(), 1);
        assert_eq!(intersection.files[0].new_path, "src/lib.rs");
        assert_eq!(intersection.files[0].hunks[0].lines.len(), 1);
        assert_eq!(
            intersection.files[0].hunks[0].lines[0].content,
            "shared line"
        );

        assert_eq!(output.post_commit_data.commit_oid, "abc123");
        assert_eq!(output.post_commit_data.commit_time_ms, commit_time_ms);
        assert_eq!(output.combined_recent_patch.files.len(), 1);
        assert_eq!(output.combined_recent_patch.files[0].new_path, "src/lib.rs");
        assert_eq!(output.tool_name, Some(String::from("opencode")));
        assert_eq!(output.tool_version, Some(String::from("1.2.3")));
    }
}
