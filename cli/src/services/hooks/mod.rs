use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, to_string as serialize_to_json, Value};

use crate::services::agent_trace::{
    agent_trace_persisted_url, build_agent_trace, patch_has_touched_lines, patches_have_overlap,
    validate_agent_trace_value, AgentTrace, AgentTraceMetadataInput, AgentTraceVcsType,
};
use crate::services::agent_trace_db::{
    AgentTraceDb, AgentTraceInsert, DiffTraceInsert, InsertMessageInsert, InsertPartInsert,
    MessageRole, PartType, PostCommitPatchIntersectionInsert, RecentDiffTracePatches,
    PAYLOAD_TYPE_PATCH, PAYLOAD_TYPE_STRUCTURED,
};
use crate::services::checkout;
use crate::services::config;
use crate::services::observability::traits::Logger;
use crate::services::patch::{
    combine_patches as combine_patches_fn, intersect_patches as intersect_patches_fn,
    load_patch_from_json, parse_patch as parse_patch_from_text, ParsedPatch,
};
use crate::services::structured_patch::{
    build_claude_post_tool_use_patch, derive_claude_structured_patch,
    ClaudeStructuredPatchDerivationResult, PatchBuildResult,
};
pub mod command;
pub mod lifecycle;

pub const NAME: &str = "hooks";
pub const CANONICAL_SCE_COAUTHOR_TRAILER: &str = "Co-authored-by: SCE <sce@crocoder.dev>";
const MAX_TRACE_FILE_CREATE_ATTEMPTS: u64 = 1_000_000;
const CLAUDE_MODEL_ID_PREFIX: &str = "claude/";
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
    model_id: Option<String>,
    tool_name: String,
    tool_version: Option<String>,
    payload_type: String,
}

/// Either a diff-trace payload to persist or a deterministic no-op result.
#[derive(Clone, Debug, Eq, PartialEq)]
enum DiffTraceParseResult {
    Persist(DiffTracePayload),
    NoOp(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StdinPayloadKind {
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

const CONVERSATION_TRACE_MESSAGE_UPDATED: &str = "message";
const CONVERSATION_TRACE_MESSAGE_PART_UPDATED: &str = "message.part";

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

/// Required `sce hooks diff-trace` STDIN payload shape:
/// `{ sessionID, diff, time, model_id?, tool_name, tool_version }`.
///
/// Validation contract:
/// - `sessionID`, `diff`, and `tool_name` must be non-empty strings.
/// - `model_id` is optional: absent or `null` → `None`, present+non-empty → `Some`, present+empty → error.
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
            run_commit_msg_subcommand_with_trace(repository_root, subcommand, message_file, logger)
        }
        HookSubcommand::PostCommit {
            vcs_type,
            remote_url,
        } => {
            run_post_commit_subcommand_with_trace(repository_root, *vcs_type, remote_url.as_deref())
        }
        HookSubcommand::PostRewrite { rewrite_method } => {
            run_post_rewrite_subcommand_with_trace(repository_root, subcommand, rewrite_method)
        }
        HookSubcommand::DiffTrace => Ok(run_diff_trace_subcommand(repository_root, logger)),
        HookSubcommand::ConversationTrace => {
            Ok(run_conversation_trace_subcommand(repository_root, logger))
        }
    }
}

fn run_conversation_trace_subcommand(
    repository_root: &Path,
    logger: Option<&dyn Logger>,
) -> String {
    let stdin_payload = match read_hook_stdin() {
        Ok(payload) => payload,
        Err(error) => return log_conversation_trace_fail_open(&error, logger),
    };

    match run_conversation_trace_subcommand_from_payload(repository_root, &stdin_payload, logger) {
        Ok(output) => output,
        Err(error) => log_conversation_trace_fail_open(&error, logger),
    }
}

fn run_conversation_trace_subcommand_from_payload(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let payload = parse_conversation_trace_payload(stdin_payload)?;
    persist_conversation_trace_payload_to_agent_trace_db(repository_root, payload, logger)
}

fn log_conversation_trace_fail_open(error: &anyhow::Error, logger: Option<&dyn Logger>) -> String {
    if let Some(log) = logger {
        log.error(
            "sce.hooks.conversation_trace.error",
            &error.to_string(),
            &[],
        );
    }

    String::from("conversation-trace hook intake failed open; error logged.")
}

fn persist_conversation_trace_payload_to_agent_trace_db(
    repository_root: &Path,
    payload: ConversationTracePayload,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let db = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for conversation-trace persistence.",
    )?;

    let summary = persist_conversation_trace_payload_to_agent_trace_db_with(
        payload,
        logger,
        |inserts| db.insert_messages(inserts),
        |inserts| db.insert_parts(inserts),
    );

    Ok(summary.render())
}

fn persist_conversation_trace_payload_to_agent_trace_db_with<IM, IP>(
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

fn open_agent_trace_db_for_hook_runtime(
    repository_root: &Path,
    context_message: &'static str,
) -> Result<AgentTraceDb> {
    checkout::resolve_or_create_agent_trace_db_for_checkout(repository_root)
        .map(|(db, _checkout_id)| db)
        .context(context_message)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConversationTraceEventPersistenceSummary {
    persisted: usize,
    skipped: usize,
}

fn persist_message_updated_batch_to_agent_trace_db_with<I>(
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
                );
                0
            }
        }
    };

    ConversationTraceEventPersistenceSummary { persisted, skipped }
}

fn persist_message_part_updated_batch_to_agent_trace_db_with<I>(
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
                );
                0
            }
        }
    };

    ConversationTraceEventPersistenceSummary { persisted, skipped }
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

    // Classify: Claude raw hook events carry hook_event_name.
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
        return Ok(parse_conversation_trace_payloads(&items));
    }

    let payloads = required_payloads_array(payload)?;

    Ok(parse_conversation_trace_payloads(payloads))
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

fn parse_conversation_trace_payloads(payloads: &[Value]) -> ConversationTracePayload {
    let mut message_inserts = Vec::new();
    let mut message_skipped = Vec::new();
    let mut part_inserts = Vec::new();
    let mut part_skipped = Vec::new();
    let mut skipped = Vec::new();

    for (index, item) in payloads.iter().enumerate() {
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
                    });
                    continue;
                }
            };

        match event_type.as_str() {
            CONVERSATION_TRACE_MESSAGE_UPDATED => match parse_message_updated_item(item) {
                Ok(input) => message_inserts.push(input),
                Err(error) => message_skipped.push(SkippedConversationTracePayload {
                    index,
                    reason: error.to_string(),
                }),
            },
            CONVERSATION_TRACE_MESSAGE_PART_UPDATED => {
                match parse_message_part_updated_item(item) {
                    Ok(input) => part_inserts.push(input),
                    Err(error) => part_skipped.push(SkippedConversationTracePayload {
                        index,
                        reason: error.to_string(),
                    }),
                }
            }
            _ => skipped.push(SkippedConversationTracePayload {
                index,
                reason: conversation_trace_validation_error(
                    "field 'type' must be one of 'message' or 'message.part'",
                ),
            }),
        }
    }

    ConversationTracePayload {
        attempted_count: payloads.len(),
        message_updated: ConversationTraceMessageBatch {
            inserts: message_inserts,
            skipped: message_skipped,
        },
        message_part_updated: ConversationTracePartBatch {
            inserts: part_inserts,
            skipped: part_skipped,
        },
        skipped,
    }
}

fn conversation_trace_payload_item<'a>(
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
        });
        return None;
    };

    Some(payload)
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
    let part_type = parse_part_type(payload)?;
    let raw_text = required_string_field(payload, "text", conversation_trace_validation_error)?;
    let text = match part_type {
        PartType::Patch => {
            // Try JSON first — if payload.text is already a serialized ParsedPatch, use it directly.
            if load_patch_from_json(&raw_text).is_ok() {
                raw_text
            } else {
                // Fall back to raw unified-diff parsing.
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
        "question" => Ok(PartType::Question),
        _ => bail!(conversation_trace_validation_error(
            "field 'part_type' must be one of 'text', 'reasoning', 'patch' or 'question'"
        )),
    }
}

fn validate_question_part_text(raw_text: String) -> Result<String> {
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

fn conversation_trace_validation_error(detail: &str) -> String {
    format!("Invalid conversation-trace payload from STDIN: {detail}.")
}

fn run_diff_trace_subcommand(repository_root: &Path, logger: Option<&dyn Logger>) -> String {
    let stdin_payload = match read_hook_stdin() {
        Ok(payload) => payload,
        Err(error) => return log_diff_trace_fail_open(&error, logger),
    };

    match run_diff_trace_subcommand_from_payload(repository_root, &stdin_payload, logger) {
        Ok(output) => output,
        Err(error) => log_diff_trace_fail_open(&error, logger),
    }
}

fn run_diff_trace_subcommand_from_payload(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let parse_result = parse_diff_trace_payload(stdin_payload)?;
    let payload = match parse_result {
        DiffTraceParseResult::Persist(payload) => payload,
        DiffTraceParseResult::NoOp(message) => return Ok(message),
    };
    run_diff_trace_subcommand_from_payload_with(repository_root, &payload, logger)
}

fn log_diff_trace_fail_open(error: &anyhow::Error, logger: Option<&dyn Logger>) -> String {
    if let Some(log) = logger {
        log.error("sce.hooks.diff_trace.error", &error.to_string(), &[]);
    }

    String::from("diff-trace hook intake failed open; error logged.")
}

fn run_diff_trace_subcommand_from_payload_with(
    repository_root: &Path,
    payload: &DiffTracePayload,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    if let Err(error) = diff_trace_db_time_ms(payload.time) {
        if let Some(log) = logger {
            log.warn(
                "sce.hooks.diff_trace.agent_trace_db_time_invalid",
                &error.to_string(),
                &[],
            );
        }
    }
    let agent_trace_db_result = persist_diff_trace_payload_to_agent_trace_db(
        repository_root,
        payload,
        payload.model_id.as_deref(),
        payload.tool_version.as_deref(),
    );
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
            "diff-trace hook intake persisted payload to AgentTraceDb.",
        ))
    } else {
        Ok(String::from(
            "diff-trace hook intake completed; AgentTraceDb persistence failed.",
        ))
    }
}

fn parse_diff_trace_payload(stdin_payload: &str) -> Result<DiffTraceParseResult> {
    let payload_kind = StdinPayloadKind::DiffTrace;
    let parsed: Value = serde_json::from_str(stdin_payload)
        .with_context(|| payload_kind.validation_error("expected valid JSON"))?;
    let payload = parsed
        .as_object()
        .ok_or_else(|| anyhow!(payload_kind.validation_error("expected a JSON object")))?;

    // Classify: Claude structured payloads carry hook_event_name.
    if payload.contains_key("hook_event_name") {
        return parse_claude_diff_trace_payload(payload, stdin_payload, payload_kind);
    }

    // OpenCode normalized payload — unchanged validation.
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
        tool_name,
        tool_version,
        payload_type: PAYLOAD_TYPE_PATCH.to_string(),
    }))
}

/// Parse a Claude structured hook payload into a diff-trace intake result.
///
/// Returns `NoOp` for events without diff traces and unsupported tool usage;
/// only supported `PostToolUse Write` / `Edit` events produce a `Persist` result.
fn parse_claude_diff_trace_payload(
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
                model_id: extract_direct_claude_model_id(payload),
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

fn extract_direct_claude_model_id(payload: &serde_json::Map<String, Value>) -> Option<String> {
    direct_claude_model_id_string(payload, &["model", "model_id", "modelId"])
        .or_else(|| {
            payload
                .get("model")
                .and_then(Value::as_object)
                .and_then(|model| direct_claude_model_id_string(model, &["id", "model", "name"]))
        })
        .and_then(|model| normalize_claude_model_id(&model))
}

fn direct_claude_model_id_string(
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

fn normalize_claude_model_id(model: &str) -> Option<String> {
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

/// Extract a u64 timestamp from a Claude hook event payload, falling back to the
/// current system time when no timestamp field is present.
fn extract_claude_event_time(payload: &serde_json::Map<String, Value>) -> u64 {
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

fn required_nullable_or_non_empty_string_field(
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

fn optional_string_field(
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

fn required_non_empty_string_field(
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
    format_error: impl Fn(&str) -> String,
) -> Result<&'a Value> {
    payload.get(field_name).ok_or_else(|| {
        anyhow!(format_error(&format!(
            "missing required field '{field_name}'"
        )))
    })
}

fn persist_diff_trace_payload_to_agent_trace_db(
    repository_root: &Path,
    payload: &DiffTracePayload,
    model_id: Option<&str>,
    tool_version: Option<&str>,
) -> Result<()> {
    persist_diff_trace_payload_to_agent_trace_db_with(payload, model_id, tool_version, |input| {
        let db = open_agent_trace_db_for_hook_runtime(
            repository_root,
            "Failed to open Agent Trace DB for diff-trace persistence.",
        )?;
        db.insert_diff_trace(input)
            .context("Failed to persist diff-trace payload to Agent Trace DB.")?;

        Ok(())
    })
}

fn persist_diff_trace_payload_to_agent_trace_db_with<F>(
    payload: &DiffTracePayload,
    model_id: Option<&str>,
    tool_version: Option<&str>,
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
        model_id,
        tool_name: &payload.tool_name,
        tool_version,
        payload_type: &payload.payload_type,
    })
}

fn diff_trace_db_time_ms(time: u64) -> Result<i64> {
    i64::try_from(time).map_err(|_| {
        anyhow!(StdinPayloadKind::DiffTrace.validation_error(
            "field 'time' must fit in a signed 64-bit Unix epoch millisecond value for Agent Trace DB storage"
        ))
    })
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
    logger: Option<&dyn Logger>,
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
    let ai_contribution_present = if gate_passed {
        match staged_diff_has_ai_overlap(repository_root, logger) {
            StagedDiffAiOverlapResult::Overlap => true,
            StagedDiffAiOverlapResult::NoOverlap | StagedDiffAiOverlapResult::Error => false,
        }
    } else {
        false
    };
    let transformed =
        apply_commit_msg_coauthor_policy(&runtime, ai_contribution_present, &original);
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
    logger: Option<&dyn Logger>,
) -> Result<String> {
    run_commit_msg_subcommand_in_repo(repository_root, message_file, logger)
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
    repository_root: &Path,
    flow_result: &PostCommitIntersectionFlowResult,
    vcs_type: Option<AgentTraceVcsType>,
    remote_url: &str,
) -> Result<AgentTrace> {
    let db = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for post-commit trace.",
    )?;

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

    let constructed_url = agent_trace_persisted_url(&agent_trace.id);

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
    let db = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for post-commit intersection.",
    )?;

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

/// Result of the staged-diff AI-overlap evidence check.
///
/// Used by the commit-msg hook to decide whether to append the canonical
/// co-author trailer. Errors are collapsed to `NoEvidence` at the policy
/// level (trailer is never appended on error), but the `Error` variant
/// allows the caller to log a diagnostic event distinguishing error
/// paths from honest no-overlap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StagedDiffAiOverlapResult {
    /// Staged diff overlaps with at least one recent AI/editor diff trace.
    Overlap,
    /// No overlap found; staged diff and recent traces were both available
    /// but share no touched lines.
    NoOverlap,
    /// An error occurred (DB open failure, schema not ready, query error,
    /// staged diff read failure, etc.). The trailer must not be appended.
    Error,
}

fn staged_diff_has_ai_overlap(
    repository_root: &Path,
    logger: Option<&dyn Logger>,
) -> StagedDiffAiOverlapResult {
    let db_open_result = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for staged AI-overlap evidence check.",
    );

    let db = match db_open_result {
        Ok(db) => db,
        Err(error) => {
            if let Some(log) = logger {
                log.error(
                    "sce.hooks.commit_msg.ai_overlap_error",
                    &format!("Staged AI-overlap evidence check failed: {error}."),
                    &[],
                );
            }
            return StagedDiffAiOverlapResult::Error;
        }
    };

    let result = staged_diff_has_ai_overlap_with(
        repository_root,
        capture_staged_patch_from_git,
        current_unix_time_ms,
        |cutoff_ms, end_ms| db.recent_diff_trace_patches(cutoff_ms, end_ms),
    );

    if result == StagedDiffAiOverlapResult::Error {
        if let Some(log) = logger {
            log.error(
                "sce.hooks.commit_msg.ai_overlap_error",
                "Staged AI-overlap evidence check failed: error during staged-diff or trace query.",
                &[],
            );
        }
    }

    result
}

fn staged_diff_has_ai_overlap_with<C, N, Q>(
    repository_root: &Path,
    capture_staged_patch: C,
    now_ms: N,
    query_recent_patches: Q,
) -> StagedDiffAiOverlapResult
where
    C: FnOnce(&Path) -> Result<ParsedPatch>,
    N: FnOnce() -> Result<i64>,
    Q: FnOnce(i64, i64) -> Result<RecentDiffTracePatches>,
{
    let Ok(staged_patch) = capture_staged_patch(repository_root) else {
        return StagedDiffAiOverlapResult::Error;
    };

    if !patch_has_touched_lines(&staged_patch) {
        return StagedDiffAiOverlapResult::NoOverlap;
    }

    let Ok(now_ms) = now_ms() else {
        return StagedDiffAiOverlapResult::Error;
    };
    let cutoff_ms = now_ms - RECENT_DAYS_MILLIS;

    let Ok(recent_patches) = query_recent_patches(cutoff_ms, now_ms) else {
        return StagedDiffAiOverlapResult::Error;
    };

    let has_overlap = recent_patches.patches.into_iter().any(|recent_patch| {
        let combined_recent_patch = combine_patches_fn(&[recent_patch.patch]);
        patches_have_overlap(&combined_recent_patch, &staged_patch)
    });

    if has_overlap {
        StagedDiffAiOverlapResult::Overlap
    } else {
        StagedDiffAiOverlapResult::NoOverlap
    }
}

fn capture_staged_patch_from_git(repository_root: &Path) -> Result<ParsedPatch> {
    let patch_text = capture_staged_diff_from_git(repository_root)?;

    if patch_text.trim().is_empty() {
        return Ok(ParsedPatch { files: Vec::new() });
    }

    parse_patch_from_text(&patch_text, None).map_err(|error| {
        anyhow!(staged_patch_error(
            "failed to parse staged patch",
            &error.to_string()
        ))
    })
}

fn capture_staged_diff_from_git(repository_root: &Path) -> Result<String> {
    run_git_command_capture_stdout(
        repository_root,
        &["diff", "--cached", "--patch", "--no-ext-diff"],
        "Failed to capture staged patch from git.",
    )
}

fn staged_patch_error(detail: &str, context: &str) -> String {
    format!("Staged patch capture error: {detail} ({context}).")
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
    remote_url: Option<&str>,
) -> Result<String> {
    run_post_commit_subcommand(repository_root, vcs_type, remote_url.unwrap_or_default())
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

fn read_hook_stdin() -> Result<String> {
    let mut stdin_payload = String::new();
    io::stdin()
        .read_to_string(&mut stdin_payload)
        .context("Failed to read hook input from STDIN.")?;
    Ok(stdin_payload)
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
    ai_contribution_present: bool,
    commit_message: &str,
) -> String {
    if !commit_msg_policy_gate_passed(runtime) || !ai_contribution_present {
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

/// Transform a validated raw Claude `UserPromptSubmit` event payload into the two
/// normalized `serde_json::Value` items expected by `parse_conversation_trace_payloads`.
///
/// Returns one `message` item and one `message.part` item sharing
/// the same generated `UUIDv7` `message_id` and the event's `session_id`.
///
/// Supported events:
/// - `UserPromptSubmit`: produces two items (parent user message + text part).
///
/// Any other `hook_event_name` value produces a validation error.
/// Missing or empty required fields (`session_id`, `prompt`) produce a validation error.
fn transform_claude_user_prompt_submit(
    payload: &serde_json::Map<String, Value>,
) -> Result<Vec<Value>> {
    transform_claude_user_prompt_submit_with(
        payload,
        || {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let ts = uuid::Timestamp::from_unix(uuid::NoContext, now.as_secs(), now.subsec_nanos());
            uuid::Uuid::new_v7(ts)
        },
        || current_unix_time_ms().unwrap_or(0),
    )
}

/// Injectable counterpart of `transform_claude_user_prompt_submit` for deterministic testing.
fn transform_claude_user_prompt_submit_with<G, T>(
    payload: &serde_json::Map<String, Value>,
    generate_message_id: G,
    generate_timestamp_ms: T,
) -> Result<Vec<Value>>
where
    G: FnOnce() -> uuid::Uuid,
    T: FnOnce() -> i64,
{
    let event_name = required_non_empty_string_field(
        payload,
        "hook_event_name",
        conversation_trace_validation_error,
    )?;

    if event_name != "UserPromptSubmit" {
        let raw_content = serde_json::to_string(payload).unwrap_or_default();
        bail!(conversation_trace_validation_error(&format!(
            "unsupported Claude hook event '{event_name}': only 'UserPromptSubmit' is supported. Raw event: {raw_content}"
        )));
    }

    let session_id = required_non_empty_string_field(
        payload,
        "session_id",
        conversation_trace_validation_error,
    )?;
    let prompt =
        required_non_empty_string_field(payload, "prompt", conversation_trace_validation_error)?;

    let message_id = generate_message_id().to_string();
    let generated_at_unix_ms = generate_timestamp_ms();

    Ok(vec![
        json!({
            "type": CONVERSATION_TRACE_MESSAGE_UPDATED,
            "session_id": session_id,
            "message_id": message_id,
            "role": "user",
            "generated_at_unix_ms": generated_at_unix_ms,
        }),
        json!({
            "type": CONVERSATION_TRACE_MESSAGE_PART_UPDATED,
            "session_id": session_id,
            "message_id": message_id,
            "part_type": "text",
            "text": prompt,
            "generated_at_unix_ms": generated_at_unix_ms,
        }),
    ])
}

/// Transform a raw Claude `Stop` hook event into two normalized conversation-trace
/// payload items.
///
/// Returns one `message` item and one `message.part` item sharing
/// the same generated `UUIDv7` `message_id` and the event's `session_id`.
///
/// Supported events:
/// - `Stop`: produces two items (assistant parent message + text part).
///
/// Any other `hook_event_name` value produces a validation error.
/// Missing or empty required fields (`session_id`, `last_assistant_message`) produce
/// a validation error.
fn transform_claude_stop(payload: &serde_json::Map<String, Value>) -> Result<Vec<Value>> {
    transform_claude_stop_with(
        payload,
        || {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let ts = uuid::Timestamp::from_unix(uuid::NoContext, now.as_secs(), now.subsec_nanos());
            uuid::Uuid::new_v7(ts)
        },
        || current_unix_time_ms().unwrap_or(0),
    )
}

/// Injectable counterpart of `transform_claude_stop` for deterministic testing.
fn transform_claude_stop_with<G, T>(
    payload: &serde_json::Map<String, Value>,
    generate_message_id: G,
    generate_timestamp_ms: T,
) -> Result<Vec<Value>>
where
    G: FnOnce() -> uuid::Uuid,
    T: FnOnce() -> i64,
{
    let event_name = required_non_empty_string_field(
        payload,
        "hook_event_name",
        conversation_trace_validation_error,
    )?;

    if event_name != "Stop" {
        let raw_content = serde_json::to_string(payload).unwrap_or_default();
        bail!(conversation_trace_validation_error(&format!(
            "unsupported Claude hook event '{event_name}': only 'Stop' is supported. Raw event: {raw_content}"
        )));
    }

    let session_id = required_non_empty_string_field(
        payload,
        "session_id",
        conversation_trace_validation_error,
    )?;
    let last_assistant_message = required_non_empty_string_field(
        payload,
        "last_assistant_message",
        conversation_trace_validation_error,
    )?;

    let message_id = generate_message_id().to_string();
    let generated_at_unix_ms = generate_timestamp_ms();

    Ok(vec![
        json!({
            "type": CONVERSATION_TRACE_MESSAGE_UPDATED,
            "session_id": session_id,
            "message_id": message_id,
            "role": "assistant",
            "generated_at_unix_ms": generated_at_unix_ms,
        }),
        json!({
            "type": CONVERSATION_TRACE_MESSAGE_PART_UPDATED,
            "session_id": session_id,
            "message_id": message_id,
            "part_type": "text",
            "text": last_assistant_message,
            "generated_at_unix_ms": generated_at_unix_ms,
        }),
    ])
}
fn transform_claude_post_tool_use(payload: &serde_json::Map<String, Value>) -> Result<Vec<Value>> {
    transform_claude_post_tool_use_with(
        payload,
        || {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let ts = uuid::Timestamp::from_unix(uuid::NoContext, now.as_secs(), now.subsec_nanos());
            uuid::Uuid::new_v7(ts)
        },
        || current_unix_time_ms().unwrap_or(0),
    )
}

/// Injectable counterpart of `transform_claude_post_tool_use` for deterministic testing.
fn transform_claude_post_tool_use_with<G, T>(
    payload: &serde_json::Map<String, Value>,
    generate_message_id: G,
    generate_timestamp_ms: T,
) -> Result<Vec<Value>>
where
    G: FnOnce() -> uuid::Uuid,
    T: FnOnce() -> i64,
{
    let event_name = required_non_empty_string_field(
        payload,
        "hook_event_name",
        conversation_trace_validation_error,
    )?;

    if event_name != "PostToolUse" {
        let raw_content = serde_json::to_string(payload).unwrap_or_default();
        bail!(conversation_trace_validation_error(&format!(
            "unsupported Claude hook event '{event_name}': only 'PostToolUse' is supported. Raw event: {raw_content}"
        )));
    }

    // Silently skip PostToolUse events for non-Write/Edit tools
    let tool_name = payload
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if tool_name != "Write" && tool_name != "Edit" {
        return Ok(vec![]);
    }

    let session_id = required_non_empty_string_field(
        payload,
        "session_id",
        conversation_trace_validation_error,
    )?;

    let message_id = generate_message_id().to_string();
    let generated_at_unix_ms = generate_timestamp_ms();

    match build_claude_post_tool_use_patch(payload) {
        PatchBuildResult::Built(parsed_patch) => {
            let text = serde_json::to_string(&parsed_patch)?;
            let items = vec![
                json!({
                    "type": CONVERSATION_TRACE_MESSAGE_UPDATED,
                    "session_id": session_id,
                    "message_id": message_id,
                    "role": "assistant",
                    "generated_at_unix_ms": generated_at_unix_ms,
                }),
                json!({
                    "type": CONVERSATION_TRACE_MESSAGE_PART_UPDATED,
                    "session_id": session_id,
                    "message_id": message_id,
                    "part_type": "patch",
                    "text": text,
                    "generated_at_unix_ms": generated_at_unix_ms,
                }),
            ];
            Ok(items)
        }
        PatchBuildResult::Skipped(_) => Ok(vec![]),
    }
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

    fn valid_patch_text(path: &str, content: &str) -> String {
        format!(
            "Index: {path}\n===================================================================\n--- {path}\n+++ {path}\n@@ -0,0 +1,1 @@\n+{content}\n"
        )
    }

    fn valid_patch(path: &str, content: &str) -> ParsedPatch {
        let patch_text = valid_patch_text(path, content);

        parse_patch_from_text(&patch_text, None).expect("test patch should parse")
    }

    #[test]
    fn conversation_trace_mixed_payload_maps_to_message_and_part_insert_inputs() {
        let patch_text = valid_patch_text("src/lib.rs", "let answer = 42;");
        let question_text = serde_json::json!([
            {
                "question": "Proceed?",
                "answer": "Yes"
            }
        ])
        .to_string();
        let payload = serde_json::json!({
            "payloads": [
                {
                    "type": "message",
                    "session_id": "session-1",
                    "message_id": "message-1",
                    "role": "assistant",
                    "generated_at_unix_ms": 1_800_000_000_000_i64
                },
                {
                    "type": "message.part",
                    "session_id": "session-1",
                    "message_id": "message-1",
                    "part_type": "reasoning",
                    "text": "thinking through validation",
                    "generated_at_unix_ms": 1_800_000_000_001_i64
                },
                {
                    "type": "message.part",
                    "session_id": "session-1",
                    "message_id": "message-1",
                    "part_type": "patch",
                    "text": patch_text,
                    "generated_at_unix_ms": 1_800_000_000_002_i64
                },
                {
                    "type": "message.part",
                    "session_id": "session-1",
                    "message_id": "message-1",
                    "part_type": "question",
                    "text": question_text,
                    "generated_at_unix_ms": 1_800_000_000_003_i64
                }
            ]
        });

        let parsed = parse_conversation_trace_payload(&payload.to_string())
            .expect("conversation-trace mixed payload should parse");

        assert_eq!(parsed.attempted_count, 4);
        assert!(parsed.skipped.is_empty());
        assert!(parsed.message_updated.skipped.is_empty());
        assert!(parsed.message_part_updated.skipped.is_empty());

        assert_eq!(parsed.message_updated.inserts.len(), 1);
        let message = &parsed.message_updated.inserts[0];
        assert_eq!(message.session_id, "session-1");
        assert_eq!(message.message_id, "message-1");
        assert_eq!(message.role, MessageRole::Assistant);
        assert_eq!(message.generated_at_unix_ms, 1_800_000_000_000_i64);

        assert_eq!(parsed.message_part_updated.inserts.len(), 3);
        let reasoning_part = &parsed.message_part_updated.inserts[0];
        assert_eq!(reasoning_part.session_id, "session-1");
        assert_eq!(reasoning_part.message_id, "message-1");
        assert_eq!(reasoning_part.part_type, PartType::Reasoning);
        assert_eq!(reasoning_part.text, "thinking through validation");
        assert_eq!(reasoning_part.generated_at_unix_ms, 1_800_000_000_001_i64);

        let patch_part = &parsed.message_part_updated.inserts[1];
        assert_eq!(patch_part.session_id, "session-1");
        assert_eq!(patch_part.message_id, "message-1");
        assert_eq!(patch_part.part_type, PartType::Patch);
        assert_eq!(
            patch_part.text,
            serialize_to_json(&valid_patch("src/lib.rs", "let answer = 42;"))
                .expect("test patch should serialize")
        );
        assert_eq!(patch_part.generated_at_unix_ms, 1_800_000_000_002_i64);

        let question_part = &parsed.message_part_updated.inserts[2];
        assert_eq!(question_part.session_id, "session-1");
        assert_eq!(question_part.message_id, "message-1");
        assert_eq!(question_part.part_type, PartType::Question);
        assert_eq!(question_part.text, question_text);
        assert_eq!(question_part.generated_at_unix_ms, 1_800_000_000_003_i64);
    }

    #[test]
    fn conversation_trace_mixed_payload_skips_malformed_sibling_items() {
        let invalid_question_text = serde_json::json!({
            "question": "Proceed?",
            "answer": "Yes"
        })
        .to_string();
        let payload = serde_json::json!({
            "payloads": [
                {
                    "type": "message",
                    "session_id": "session-1",
                    "message_id": "message-1",
                    "role": "assistant",
                    "generated_at_unix_ms": 1_800_000_000_000_i64
                },
                {
                    "type": "message",
                    "session_id": "session-2",
                    "message_id": "message-2",
                    "role": "system",
                    "generated_at_unix_ms": 1_800_000_000_002_i64
                },
                {
                    "type": "message.part",
                    "session_id": "session-3",
                    "message_id": "message-3",
                    "part_type": "text",
                    "generated_at_unix_ms": 1_800_000_000_003_i64
                },
                {
                    "type": "message.part",
                    "session_id": "session-4",
                    "message_id": "message-4",
                    "part_type": "patch",
                    "text": "--- src/main.rs",
                    "generated_at_unix_ms": 1_800_000_000_004_i64
                },
                {
                    "type": "message.part",
                    "session_id": "session-5",
                    "message_id": "message-5",
                    "part_type": "question",
                    "text": invalid_question_text,
                    "generated_at_unix_ms": 1_800_000_000_005_i64
                },
                {
                    "type": "session.started",
                    "session_id": "session-6"
                },
                42,
                {
                    "type": null,
                    "session_id": "session-7"
                }
            ]
        });

        let parsed = parse_conversation_trace_payload(&payload.to_string())
            .expect("conversation-trace mixed payload should parse with skipped items");

        assert_eq!(parsed.attempted_count, 8);
        assert_eq!(parsed.message_updated.inserts.len(), 1);
        assert_eq!(parsed.message_updated.skipped.len(), 1);
        assert_eq!(parsed.message_updated.skipped[0].index, 1);
        assert!(parsed.message_updated.skipped[0]
            .reason
            .contains("field 'role'"));
        assert_eq!(parsed.message_part_updated.inserts.len(), 0);
        assert_eq!(parsed.message_part_updated.skipped.len(), 3);
        assert_eq!(parsed.message_part_updated.skipped[0].index, 2);
        assert!(parsed.message_part_updated.skipped[0]
            .reason
            .contains("missing required field 'text'"));
        assert_eq!(parsed.message_part_updated.skipped[1].index, 3);
        assert!(parsed.message_part_updated.skipped[1]
            .reason
            .contains("neither valid patch-JSON nor a valid patch"));
        assert_eq!(parsed.message_part_updated.skipped[2].index, 4);
        assert!(parsed.message_part_updated.skipped[2]
            .reason
            .contains("question part must be a JSON array"));
        assert_eq!(parsed.skipped.len(), 3);
        assert_eq!(parsed.skipped[0].index, 5);
        assert!(parsed.skipped[0].reason.contains("field 'type'"));
        assert_eq!(parsed.skipped[1].index, 6);
        assert!(parsed.skipped[1]
            .reason
            .contains("payloads[6] must be an object"));
        assert_eq!(parsed.skipped[2].index, 7);
        assert!(parsed.skipped[2]
            .reason
            .contains("field 'type' must be a string"));
    }

    fn diff_trace_payload(model_id: Option<&str>, tool_version: Option<&str>) -> DiffTracePayload {
        DiffTracePayload {
            session_id: String::from("session-123"),
            diff: String::from("diff text"),
            time: 1_800_000_000_000_u64,
            model_id: model_id.map(String::from),
            tool_name: String::from("claude"),
            tool_version: tool_version.map(String::from),
            payload_type: String::from(PAYLOAD_TYPE_STRUCTURED),
        }
    }

    #[test]
    fn diff_trace_db_persistence_uses_direct_payload_model_and_tool_version() {
        let payload = diff_trace_payload(Some("direct-model"), None);

        persist_diff_trace_payload_to_agent_trace_db_with(
            &payload,
            Some("direct-model"),
            Some("Claude Code 1.2.3"),
            |input| {
                assert_eq!(input.time_ms, 1_800_000_000_000_i64);
                assert_eq!(input.session_id, "session-123");
                assert_eq!(input.model_id, Some("direct-model"));
                assert_eq!(input.tool_name, "claude");
                assert_eq!(input.tool_version, Some("Claude Code 1.2.3"));
                assert_eq!(input.payload_type, PAYLOAD_TYPE_STRUCTURED);

                Ok(())
            },
        )
        .expect("direct diff-trace attribution should be persisted");
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
                        payload_type: String::from(PAYLOAD_TYPE_PATCH),
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
