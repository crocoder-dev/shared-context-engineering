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
    agent_trace_persisted_url, build_agent_trace_from_evidence, patch_has_touched_lines,
    patches_have_overlap, validate_agent_trace_value, AgentTrace, AgentTraceEvidence,
    AgentTraceMetadataInput, AgentTraceVcsType,
};
use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_db::{
    AgentTraceInsert, DiffTraceInsert, InsertMessageInsert, InsertPartInsert, MessageRole,
    PartType, PostCommitPatchIntersectionInsert, RecentDiffTracePatches, PAYLOAD_TYPE_PATCH,
    PAYLOAD_TYPE_STRUCTURED,
};
#[cfg(test)]
use crate::services::agent_trace_storage::{
    resolve_agent_trace_storage_at_state_root,
    resolve_agent_trace_storage_for_hook_runtime_at_state_root,
};
use crate::services::agent_trace_storage::{
    resolve_agent_trace_storage_for_hook_runtime, AgentTraceStorageContext,
};
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
use crate::services::sync::auto_sync;
pub mod claude_model_state;
pub mod claude_transcript;
pub mod codex;
pub mod command;
pub mod lifecycle;
pub mod mutation_scope;

pub const NAME: &str = "hooks";
pub const CANONICAL_SCE_COAUTHOR_TRAILER: &str = "Co-authored-by: SCE <sce@crocoder.dev>";
const CLAUDE_MODEL_ID_PREFIX: &str = "claude/";
pub(crate) const DIFF_TRACE_OPENCODE_SESSION_ID_PREFIX: &str = "oc_";
pub(crate) const DIFF_TRACE_CLAUDE_SESSION_ID_PREFIX: &str = "cc_";
pub(crate) const DIFF_TRACE_PI_SESSION_ID_PREFIX: &str = "pi_";
pub(crate) const DIFF_TRACE_CODEX_SESSION_ID_PREFIX: &str = "cx_";
const OPENCODE_TOOL_NAME: &str = "opencode";
const CLAUDE_TOOL_NAME: &str = "claude";
const PI_TOOL_NAME: &str = "pi";
const CODEX_TOOL_NAME: &str = "codex";
const NORMALIZED_CONVERSATION_TRACE_TOOL_NAMES: &[&str] = &[OPENCODE_TOOL_NAME, PI_TOOL_NAME];
type PayloadValidationError = fn(&str) -> String;

pub(crate) fn prefixed_diff_trace_session_id(tool_name: &str, raw_session_id: &str) -> String {
    prefixed_session_id(tool_name, raw_session_id)
}

fn prefixed_conversation_trace_session_id(tool_name: &str, raw_session_id: &str) -> String {
    prefixed_session_id(tool_name, raw_session_id)
}

fn prefixed_session_id(tool_name: &str, raw_session_id: &str) -> String {
    let prefix = match tool_name {
        OPENCODE_TOOL_NAME => DIFF_TRACE_OPENCODE_SESSION_ID_PREFIX,
        CLAUDE_TOOL_NAME => DIFF_TRACE_CLAUDE_SESSION_ID_PREFIX,
        PI_TOOL_NAME => DIFF_TRACE_PI_SESSION_ID_PREFIX,
        CODEX_TOOL_NAME => DIFF_TRACE_CODEX_SESSION_ID_PREFIX,
        _ => return raw_session_id.to_string(),
    };

    if raw_session_id.starts_with(prefix) {
        raw_session_id.to_string()
    } else {
        format!("{prefix}{raw_session_id}")
    }
}

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
    Codex,
    ClaudeModelState,
    MutationScope,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct DiffTracePayload {
    #[serde(rename = "sessionID")]
    session_id: String,
    diff: String,
    time: u64,
    model_id: Option<String>,
    #[serde(skip)]
    agent_id: Option<String>,
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
        } => run_post_commit_subcommand_with_trace(
            repository_root,
            *vcs_type,
            remote_url.as_deref(),
            logger,
        ),
        HookSubcommand::PostRewrite { rewrite_method } => {
            run_post_rewrite_subcommand_with_trace(repository_root, subcommand, rewrite_method)
        }
        HookSubcommand::DiffTrace => Ok(run_diff_trace_subcommand(repository_root, logger)),
        HookSubcommand::ConversationTrace => {
            Ok(run_conversation_trace_subcommand(repository_root, logger))
        }
        HookSubcommand::Codex => Ok(codex::run_codex_subcommand(repository_root, logger)),
        HookSubcommand::ClaudeModelState => Ok(
            claude_model_state::run_claude_model_state_subcommand(repository_root, logger),
        ),
        HookSubcommand::MutationScope => {
            mutation_scope::run_mutation_scope_subcommand(repository_root, logger)
        }
    }
}

fn run_conversation_trace_subcommand(
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

fn run_conversation_trace_subcommand_from_payload(
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

fn log_conversation_trace_fail_open(
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

fn persist_conversation_trace_payload_to_agent_trace_db(
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
) -> Result<RepositoryAgentTraceDb> {
    let storage_config = config::resolve_agent_trace_storage_runtime_config(repository_root)
        .context("Failed to resolve Agent Trace repository storage config.")?;
    let storage_context = AgentTraceStorageContext {
        repository_root,
        explicit_repository_id: storage_config.repository_id.as_deref(),
        repository_remote: &storage_config.repository_remote,
    };

    resolve_agent_trace_storage_for_hook_runtime(&storage_context)
        .map(|storage| storage.db)
        .context(context_message)
}

#[cfg(test)]
pub(crate) fn open_agent_trace_db_for_hook_runtime_at_state_root(
    repository_root: &Path,
    state_root: &Path,
    context_message: &'static str,
) -> Result<RepositoryAgentTraceDb> {
    let storage_config = config::resolve_agent_trace_storage_runtime_config(repository_root)
        .context("Failed to resolve Agent Trace repository storage config.")?;
    let storage_context = AgentTraceStorageContext {
        repository_root,
        explicit_repository_id: storage_config.repository_id.as_deref(),
        repository_remote: &storage_config.repository_remote,
    };

    resolve_agent_trace_storage_for_hook_runtime_at_state_root(&storage_context, state_root)
        .map(|storage| storage.db)
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
            skipped.session_id.as_deref(),
        );
    }
}

fn log_conversation_trace_batch_insert_failure(
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

fn required_payloads_array(payload: &serde_json::Map<String, Value>) -> Result<&Vec<Value>> {
    required_field(payload, "payloads", conversation_trace_validation_error)?
        .as_array()
        .ok_or_else(|| {
            anyhow!(conversation_trace_validation_error(
                "field 'payloads' must be an array"
            ))
        })
}

fn parse_conversation_trace_payloads(
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
            session_id: None,
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

fn conversation_trace_fail_open_session_id(stdin_payload: &str) -> Option<String> {
    let payload: Value = serde_json::from_str(stdin_payload).ok()?;
    let payload = payload.as_object()?;

    if payload.contains_key("hook_event_name") {
        return non_empty_string(payload.get("session_id")).map(str::to_owned);
    }

    let first_payload = payload.get("payloads")?.as_array()?.first()?;
    non_empty_string(first_payload.get("session_id")).map(str::to_owned)
}

fn diff_trace_fail_open_session_id(stdin_payload: &str) -> Option<String> {
    let payload: Value = serde_json::from_str(stdin_payload).ok()?;
    let payload = payload.as_object()?;
    let field_name = if payload.contains_key("hook_event_name") {
        "session_id"
    } else {
        "sessionID"
    };

    non_empty_string(payload.get(field_name)).map(str::to_owned)
}

fn non_empty_string(value: Option<&Value>) -> Option<&str> {
    value?.as_str().filter(|value| !value.trim().is_empty())
}

fn run_diff_trace_subcommand(repository_root: &Path, logger: Option<&dyn Logger>) -> String {
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
    Ok(run_diff_trace_subcommand_from_payload_with(
        repository_root,
        &payload,
        logger,
    ))
}

fn log_diff_trace_fail_open(
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

fn run_diff_trace_subcommand_from_payload_with(
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
        agent_id: None,
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
                model_id: resolve_claude_model_id(payload),
                agent_id: extract_claude_agent_id(payload)?,
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

fn extract_claude_agent_id(payload: &serde_json::Map<String, Value>) -> Result<Option<String>> {
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

fn resolve_claude_model_id(payload: &serde_json::Map<String, Value>) -> Option<String> {
    resolve_claude_model_id_with(payload, claude_transcript::extract_claude_transcript_model)
}

fn resolve_claude_model_id_with<F>(
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

fn normalize_codex_model_id(model: &str) -> Option<String> {
    let normalized = model.trim();
    if normalized.is_empty() {
        return None;
    }

    Some(normalized.to_string())
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

fn persist_diff_trace_payload_to_agent_trace_db_with_db(
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

fn resolve_diff_trace_model_id(
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
    Ok(db
        .claude_model_state_by_session_and_agent(&session_id, agent_id)?
        .map(|state| state.model_id))
}

#[cfg(test)]
fn persist_diff_trace_payload_to_agent_trace_db_with<F, T>(
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
    logger: Option<&dyn Logger>,
) -> Result<String> {
    run_post_commit_subcommand_with(
        repository_root,
        vcs_type,
        remote_url,
        run_post_commit_intersection_flow,
        run_post_commit_agent_trace_flow,
        |root| {
            config::resolve_hook_runtime_config(root).map(|runtime| runtime.agent_trace_auto_sync)
        },
        |root| {
            auto_sync::launch(root);
            Ok(())
        },
        run_post_commit_passive_checkpoint,
        logger,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_post_commit_subcommand_with<F, B, C, L, K>(
    repository_root: &Path,
    vcs_type: Option<AgentTraceVcsType>,
    remote_url: &str,
    run_intersection_flow: F,
    run_agent_trace_flow: B,
    resolve_auto_sync: C,
    launch_auto_sync: L,
    run_passive_checkpoint: K,
    logger: Option<&dyn Logger>,
) -> Result<String>
where
    F: FnOnce(&Path) -> Result<PostCommitIntersectionFlowResult>,
    B: FnOnce(
        &Path,
        &PostCommitIntersectionFlowResult,
        Option<AgentTraceVcsType>,
        &str,
    ) -> Result<AgentTrace>,
    C: FnOnce(&Path) -> Result<bool>,
    L: FnOnce(&Path) -> Result<()>,
    K: FnOnce(&Path) -> Result<()>,
{
    let result = run_intersection_flow(repository_root)?;
    let _agent_trace = run_agent_trace_flow(repository_root, &result, vcs_type, remote_url)?;

    if let Err(error) = run_passive_checkpoint(repository_root) {
        if let Some(log) = logger {
            log.warn(
                "sce.agent_trace_db.passive_checkpoint_failed",
                &error.to_string(),
                &[],
                None,
            );
        }
    }

    if resolve_auto_sync(repository_root)? {
        let _ = launch_auto_sync(repository_root);
    }

    Ok(format!(
        "post-commit hook processed intersection: commit={}, intersection_files={}",
        result.post_commit_data.commit_oid,
        result.combined_recent_patch.files.len()
    ))
}

fn run_post_commit_passive_checkpoint(repository_root: &Path) -> Result<()> {
    let db = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for post-commit checkpoint.",
    )?;

    db.passive_checkpoint()
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

    // Direct evidence is resolved first with the existing intersection, then the
    // committed lines it does not cover are offered to bounded mutation history
    // (read-only, current-worktree-only, direct-only fallback on absent identity).
    let direct_intersection = intersect_patches_fn(
        &flow_result.combined_recent_patch,
        &flow_result.post_commit_data.parsed_patch,
    );
    let mutation_ai_patch =
        crate::services::mutation_trace::runtime::resolve_post_commit_mutation_ai_patch(
            repository_root,
            &db,
            &direct_intersection,
            &flow_result.post_commit_data.parsed_patch,
        );

    run_post_commit_agent_trace_flow_with(
        flow_result,
        vcs_type,
        remote_url,
        &mutation_ai_patch,
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
    mutation_ai_patch: &ParsedPatch,
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

    let agent_trace = build_agent_trace_from_evidence(
        AgentTraceEvidence {
            direct_patch: &flow_result.combined_recent_patch,
            mutation_ai_patch,
        },
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
                    None,
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
                None,
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
    logger: Option<&dyn Logger>,
) -> Result<String> {
    run_post_commit_subcommand(
        repository_root,
        vcs_type,
        remote_url.unwrap_or_default(),
        logger,
    )
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
        HookSubcommand::Codex => "codex runtime invocation",
        HookSubcommand::ClaudeModelState => "Claude model-state runtime invocation",
        HookSubcommand::MutationScope => "mutation-scope runtime invocation",
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
    use std::{
        cell::RefCell,
        fs,
        path::{Path, PathBuf},
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::services::agent_trace_db::{
        ClaudeModelStateObservation, ObservationKind, ParsedDiffTracePatch, SkippedDiffTracePatch,
    };

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
            "tool_name": "opencode",
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
        assert_eq!(message.session_id, "oc_session-1");
        assert_eq!(message.message_id, "message-1");
        assert_eq!(message.role, MessageRole::Assistant);
        assert_eq!(message.generated_at_unix_ms, 1_800_000_000_000_i64);

        assert_eq!(parsed.message_part_updated.inserts.len(), 3);
        let reasoning_part = &parsed.message_part_updated.inserts[0];
        assert_eq!(reasoning_part.session_id, "oc_session-1");
        assert_eq!(reasoning_part.message_id, "message-1");
        assert_eq!(reasoning_part.part_type, PartType::Reasoning);
        assert_eq!(reasoning_part.text, "thinking through validation");
        assert_eq!(reasoning_part.generated_at_unix_ms, 1_800_000_000_001_i64);

        let patch_part = &parsed.message_part_updated.inserts[1];
        assert_eq!(patch_part.session_id, "oc_session-1");
        assert_eq!(patch_part.message_id, "message-1");
        assert_eq!(patch_part.part_type, PartType::Patch);
        assert_eq!(
            patch_part.text,
            serialize_to_json(&valid_patch("src/lib.rs", "let answer = 42;"))
                .expect("test patch should serialize")
        );
        assert_eq!(patch_part.generated_at_unix_ms, 1_800_000_000_002_i64);

        let question_part = &parsed.message_part_updated.inserts[2];
        assert_eq!(question_part.session_id, "oc_session-1");
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
            "tool_name": "opencode",
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

    fn normalized_conversation_trace_message_payload(tool_name: &str, session_id: &str) -> String {
        serde_json::json!({
            "tool_name": tool_name,
            "payloads": [
                {
                    "type": "message",
                    "session_id": session_id,
                    "message_id": "message-1",
                    "role": "assistant",
                    "generated_at_unix_ms": 1_800_000_000_000_i64
                }
            ]
        })
        .to_string()
    }

    #[test]
    fn conversation_trace_normalized_payload_accepts_pi_tool_name_with_prefixed_session_id() {
        let stdin_payload = normalized_conversation_trace_message_payload("pi", "session-1");

        let parsed = parse_conversation_trace_payload(&stdin_payload)
            .expect("Pi normalized conversation-trace payload should parse");

        assert_eq!(parsed.message_updated.inserts.len(), 1);
        assert_eq!(parsed.message_updated.inserts[0].session_id, "pi_session-1");
    }

    #[test]
    fn conversation_trace_normalized_payload_rejects_unsupported_tool_name() {
        let stdin_payload = normalized_conversation_trace_message_payload("cursor", "session-1");

        let error = parse_conversation_trace_payload(&stdin_payload)
            .expect_err("unsupported tool_name should be rejected");

        assert!(error.to_string().contains("unsupported tool_name 'cursor'"));
        assert!(error.to_string().contains("'opencode'"));
        assert!(error.to_string().contains("'pi'"));
    }

    #[test]
    fn conversation_trace_normalized_payload_rejects_empty_tool_name() {
        let stdin_payload = normalized_conversation_trace_message_payload("", "session-1");

        let error = parse_conversation_trace_payload(&stdin_payload)
            .expect_err("empty tool_name should be rejected");

        assert!(error
            .to_string()
            .contains("field 'tool_name' must be a non-empty string"));
    }

    #[test]
    fn conversation_trace_normalized_payload_rejects_missing_tool_name() {
        let stdin_payload = serde_json::json!({
            "payloads": [
                {
                    "type": "message",
                    "session_id": "session-1",
                    "message_id": "message-1",
                    "role": "assistant",
                    "generated_at_unix_ms": 1_800_000_000_000_i64
                }
            ]
        })
        .to_string();

        let error = parse_conversation_trace_payload(&stdin_payload)
            .expect_err("missing tool_name should be rejected");

        assert!(error
            .to_string()
            .contains("missing required field 'tool_name'"));
    }

    #[test]
    fn conversation_trace_normalized_payload_keeps_already_prefixed_session_id() {
        let stdin_payload =
            normalized_conversation_trace_message_payload("opencode", "oc_session-1");

        let parsed = parse_conversation_trace_payload(&stdin_payload)
            .expect("already-prefixed OpenCode session ID should parse");

        assert_eq!(parsed.message_updated.inserts[0].session_id, "oc_session-1");
    }

    #[test]
    fn conversation_trace_raw_claude_event_uses_claude_identity_with_cc_prefixed_session_id() {
        let stdin_payload = serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "session-1",
            "prompt": "hello"
        })
        .to_string();

        let parsed = parse_conversation_trace_payload(&stdin_payload)
            .expect("raw Claude UserPromptSubmit event should parse");

        assert_eq!(parsed.message_updated.inserts.len(), 1);
        assert_eq!(parsed.message_updated.inserts[0].session_id, "cc_session-1");
    }

    fn diff_trace_payload(model_id: Option<&str>, tool_version: Option<&str>) -> DiffTracePayload {
        diff_trace_payload_with(
            "claude",
            "session-123",
            PAYLOAD_TYPE_STRUCTURED,
            model_id,
            tool_version,
        )
    }

    fn diff_trace_payload_with(
        tool_name: &str,
        session_id: &str,
        payload_type: &str,
        model_id: Option<&str>,
        tool_version: Option<&str>,
    ) -> DiffTracePayload {
        DiffTracePayload {
            session_id: String::from(session_id),
            diff: String::from("diff text"),
            time: 1_800_000_000_000_u64,
            model_id: model_id.map(String::from),
            agent_id: None,
            tool_name: String::from(tool_name),
            tool_version: tool_version.map(String::from),
            payload_type: String::from(payload_type),
        }
    }

    fn claude_model_test_event(transcript_path: &Path, tool_use_id: &str) -> Value {
        json!({
            "hook_event_name": "PostToolUse",
            "session_id": "session-123",
            "tool_name": "Write",
            "tool_use_id": tool_use_id,
            "transcript_path": transcript_path,
            "tool_input": {
                "file_path": "docs/status.md",
                "content": "# Status\n\nThe new state is complete.\n"
            },
            "tool_response": {
                "originalFile": "# Status\n\nThe old state is pending.\n",
                "structuredPatch": {
                    "hunks": [{
                        "oldStart": 1,
                        "oldCount": 3,
                        "newStart": 1,
                        "newCount": 3,
                        "lines": [
                            " # Status",
                            " ",
                            "-The old state is pending.",
                            "+The new state is complete."
                        ]
                    }]
                }
            }
        })
    }

    fn parsed_claude_model_id(event: &Value) -> Option<String> {
        match parse_diff_trace_payload(&event.to_string())
            .expect("Claude PostToolUse diff-trace payload should parse")
        {
            DiffTraceParseResult::Persist(payload) => payload.model_id,
            DiffTraceParseResult::NoOp(message) => {
                panic!("Claude Write payload should persist, got no-op: {message}")
            }
        }
    }

    fn parsed_claude_diff_trace(event: &Value) -> DiffTracePayload {
        match parse_diff_trace_payload(&event.to_string())
            .expect("Claude PostToolUse diff-trace payload should parse")
        {
            DiffTraceParseResult::Persist(payload) => payload,
            DiffTraceParseResult::NoOp(message) => {
                panic!("Claude Write payload should persist, got no-op: {message}")
            }
        }
    }

    fn unique_attribution_db_path(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("sce-claude-model-attribution-{label}-{suffix}"))
            .join("agent-trace.db")
    }

    fn resolved_claude_model_id_with<F>(event: &Value, transcript_lookup: F) -> Option<String>
    where
        F: FnOnce(&Path, &str) -> Option<String>,
    {
        resolve_claude_model_id_with(
            event.as_object().expect("test event should be an object"),
            transcript_lookup,
        )
    }

    fn run_attribution_git(repo_root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .output()
            .expect("git should start");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_attribution_git_repo(label: &str) -> PathBuf {
        let repo_root = unique_attribution_db_path(label)
            .parent()
            .expect("test repository should have a parent")
            .to_path_buf();
        fs::create_dir_all(&repo_root).expect("test repository directory should be created");
        run_attribution_git(&repo_root, &["init", "-q"]);
        run_attribution_git(
            &repo_root,
            &["remote", "add", "origin", "git@github.com:acme/widgets.git"],
        );
        repo_root
    }

    fn model_less_claude_diff_event(
        session_id: &str,
        tool_use_id: &str,
        agent_id: Option<&str>,
    ) -> Value {
        let mut event = claude_model_test_event(Path::new("/virtual/missing.jsonl"), tool_use_id);
        let object = event
            .as_object_mut()
            .expect("Claude test event should be an object");
        object.insert("session_id".to_string(), json!(session_id));
        object.remove("transcript_path");
        object.remove("tool_use_id");
        if let Some(agent_id) = agent_id {
            object.insert("agent_id".to_string(), json!(agent_id));
        }
        event
    }

    fn persisted_model_ids(db: &RepositoryAgentTraceDb) -> Vec<Option<String>> {
        db.query_map(
            "SELECT model_id FROM diff_traces ORDER BY id ASC",
            (),
            |row| row.get::<Option<String>>(0).map_err(Into::into),
        )
        .expect("persisted model IDs should be readable")
    }

    #[test]
    fn claude_model_direct_nested_metadata_wins_over_transcript_without_double_prefixing() {
        let transcript_path = Path::new("/unused/direct-precedence.jsonl");
        let mut event = claude_model_test_event(transcript_path, "tool-123");
        event
            .as_object_mut()
            .expect("test event should be an object")
            .insert("model".to_string(), json!({ "id": "claude/direct-model" }));

        let model_id = resolved_claude_model_id_with(&event, |_, _| {
            panic!("transcript lookup must not run when direct metadata is present")
        });

        assert_eq!(model_id.as_deref(), Some("claude/direct-model"));
        assert_eq!(parsed_claude_model_id(&event), model_id);
    }

    #[test]
    fn claude_model_falls_back_to_matching_transcript_and_normalizes_model() {
        let transcript_path = Path::new("/virtual/transcript-fallback.jsonl");
        let event = claude_model_test_event(transcript_path, "tool-123");

        let model_id = resolved_claude_model_id_with(&event, |path, tool_use_id| {
            assert_eq!(path, transcript_path);
            assert_eq!(tool_use_id, "tool-123");
            Some(String::from("claude/claude-opus-4-1"))
        });

        assert_eq!(model_id.as_deref(), Some("claude/claude-opus-4-1"));
    }

    #[test]
    fn claude_model_remains_none_when_transcript_lookup_cannot_succeed() {
        let event = claude_model_test_event(Path::new("/virtual/missing.jsonl"), "tool-123");
        assert_eq!(resolved_claude_model_id_with(&event, |_, _| None), None);

        let mut event_without_lookup_fields = event;
        let payload = event_without_lookup_fields
            .as_object_mut()
            .expect("test event should be an object");
        payload.remove("transcript_path");
        payload.remove("tool_use_id");
        assert_eq!(
            resolved_claude_model_id_with(&event_without_lookup_fields, |_, _| {
                panic!("lookup must not run without transcript event metadata")
            }),
            None
        );
    }

    #[test]
    fn claude_diff_trace_parser_keeps_agent_id_ephemeral_and_storage_free() {
        let mut event = claude_model_test_event(Path::new("/virtual/missing.jsonl"), "tool-123");
        event
            .as_object_mut()
            .expect("test event should be an object")
            .insert("agent_id".to_string(), json!(" agent-1 "));

        let payload = parsed_claude_diff_trace(&event);

        assert_eq!(payload.agent_id.as_deref(), Some("agent-1"));
        assert!(serde_json::to_value(&payload)
            .expect("internal payload should serialize")
            .get("agent_id")
            .is_none());
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn claude_model_attribution_end_to_end_persists_lifecycle_fallback_precedence_and_scope() {
        let repo_root = init_attribution_git_repo("end-to-end");
        let state_root = unique_attribution_db_path("end-to-end-state")
            .parent()
            .expect("test state should have a parent")
            .to_path_buf();
        let storage = resolve_agent_trace_storage_at_state_root(
            &AgentTraceStorageContext {
                repository_root: &repo_root,
                explicit_repository_id: None,
                repository_remote: "origin",
            },
            &state_root,
        )
        .expect("setup path should initialize the test repository DB");
        drop(storage);

        let session_start = json!({
            "hook_event_name": "SessionStart",
            "session_id": "session-123",
            "model": "model-a",
            "source": "startup"
        });
        assert_eq!(
            claude_model_state::run_claude_model_state_from_payload_at_state_root(
                &repo_root,
                &state_root,
                &session_start.to_string(),
                None,
                || Ok(10),
            ),
            ""
        );

        let db = open_agent_trace_db_for_hook_runtime_at_state_root(
            &repo_root,
            &state_root,
            "test DB should open after SessionStart",
        )
        .expect("test DB should open after SessionStart");
        assert_eq!(
            db.claude_model_state_by_session_and_agent("cc_session-123", "")
                .expect("SessionStart state should be readable")
                .expect("SessionStart should seed state")
                .model_id,
            "claude/model-a"
        );
        let session_start_event = model_less_claude_diff_event("session-123", "tool-a", None);
        persist_diff_trace_payload_to_agent_trace_db_with_db(
            &db,
            &parsed_claude_diff_trace(&session_start_event),
        )
        .expect("SessionStart state should attribute the next diff trace");
        drop(db);

        let post_model_switch = json!({
            "hook_event_name": "PostModelSwitch",
            "session_id": "session-123",
            "from_model": "model-a",
            "to_model": "model-b",
            "source": "picker"
        });
        assert_eq!(
            claude_model_state::run_claude_model_state_from_payload_at_state_root(
                &repo_root,
                &state_root,
                &post_model_switch.to_string(),
                None,
                || Ok(20),
            ),
            ""
        );

        let db = open_agent_trace_db_for_hook_runtime_at_state_root(
            &repo_root,
            &state_root,
            "test DB should open after PostModelSwitch",
        )
        .expect("test DB should open after PostModelSwitch");
        assert_eq!(
            db.claude_model_state_by_session_and_agent("cc_session-123", "")
                .expect("PostModelSwitch state should be readable")
                .expect("PostModelSwitch should update state")
                .model_id,
            "claude/model-b"
        );
        let switched_event = model_less_claude_diff_event("session-123", "tool-b", None);
        persist_diff_trace_payload_to_agent_trace_db_with_db(
            &db,
            &parsed_claude_diff_trace(&switched_event),
        )
        .expect("PostModelSwitch state should attribute the next diff trace");

        let mut direct_event = model_less_claude_diff_event("session-123", "tool-direct", None);
        direct_event
            .as_object_mut()
            .expect("Claude test event should be an object")
            .insert("model".to_string(), json!("model-c"));
        persist_diff_trace_payload_to_agent_trace_db_with_db(
            &db,
            &parsed_claude_diff_trace(&direct_event),
        )
        .expect("direct model attribution should persist");

        let transcript_path = state_root.join("transcript.jsonl");
        fs::write(
            &transcript_path,
            concat!(
                r#"{"type":"assistant","message":{"role":"assistant","model":"model-c","content":[{"type":"tool_use","id":"tool-transcript"}]}}"#,
                "\n"
            ),
        )
        .expect("transcript fixture should be written");
        let transcript_event = claude_model_test_event(&transcript_path, "tool-transcript");
        persist_diff_trace_payload_to_agent_trace_db_with_db(
            &db,
            &parsed_claude_diff_trace(&transcript_event),
        )
        .expect("transcript model attribution should persist");

        let no_state_event =
            model_less_claude_diff_event("session-without-state", "tool-none", None);
        persist_diff_trace_payload_to_agent_trace_db_with_db(
            &db,
            &parsed_claude_diff_trace(&no_state_event),
        )
        .expect("an attribution-less diff trace should still persist");

        let subagent_event =
            model_less_claude_diff_event("session-123", "tool-subagent", Some("subagent-1"));
        persist_diff_trace_payload_to_agent_trace_db_with_db(
            &db,
            &parsed_claude_diff_trace(&subagent_event),
        )
        .expect("a subagent diff trace should persist");

        assert_eq!(
            persisted_model_ids(&db),
            vec![
                Some(String::from("claude/model-a")),
                Some(String::from("claude/model-b")),
                Some(String::from("claude/model-c")),
                Some(String::from("claude/model-c")),
                None,
                None,
            ]
        );

        drop(db);
        fs::remove_file(transcript_path).expect("transcript fixture should be removed");
        fs::remove_dir_all(repo_root).expect("test repository should be removed");
        fs::remove_dir_all(state_root).expect("test state should be removed");
    }

    #[test]
    fn claude_diff_trace_persistence_uses_state_only_after_direct_and_transcript() {
        let db_path = unique_attribution_db_path("precedence");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        db.upsert_claude_model_state(ClaudeModelStateObservation {
            session_id: String::from("cc_session-123"),
            agent_id: String::new(),
            model_id: String::from("claude/state-model"),
            observation_kind: ObservationKind::SessionStart,
            source: String::from("startup"),
            observed_at_ms: 1,
        })
        .expect("state should be seeded");

        let mut state_event = claude_model_test_event(Path::new("/virtual/missing.jsonl"), "state");
        let state_object = state_event
            .as_object_mut()
            .expect("test event should be an object");
        state_object.remove("transcript_path");
        state_object.remove("tool_use_id");
        let state_payload = parsed_claude_diff_trace(&state_event);
        persist_diff_trace_payload_to_agent_trace_db_with_db(&db, &state_payload)
            .expect("state fallback should persist");

        let mut direct_event = state_event.clone();
        direct_event
            .as_object_mut()
            .expect("test event should be an object")
            .insert("model".to_string(), json!("direct-model"));
        let direct_payload = parsed_claude_diff_trace(&direct_event);
        persist_diff_trace_payload_to_agent_trace_db_with_db(&db, &direct_payload)
            .expect("direct attribution should persist");

        let transcript_path = db_path.with_extension("jsonl");
        fs::write(
            &transcript_path,
            concat!(
                r#"{"type":"assistant","message":{"role":"assistant","model":"transcript-model","content":[{"type":"tool_use","id":"transcript"}]}}"#,
                "\n"
            ),
        )
        .expect("transcript fixture should be written");
        let transcript_event = claude_model_test_event(&transcript_path, "transcript");
        let transcript_payload = parsed_claude_diff_trace(&transcript_event);
        persist_diff_trace_payload_to_agent_trace_db_with_db(&db, &transcript_payload)
            .expect("transcript attribution should persist");

        let models = db
            .query_map(
                "SELECT model_id FROM diff_traces ORDER BY id ASC",
                (),
                |row| row.get::<Option<String>>(0).map_err(Into::into),
            )
            .expect("persisted models should be readable");
        assert_eq!(
            models,
            vec![
                Some(String::from("claude/state-model")),
                Some(String::from("claude/direct-model")),
                Some(String::from("claude/transcript-model")),
            ]
        );

        drop(db);
        fs::remove_file(transcript_path).expect("transcript fixture should be removed");
        fs::remove_dir_all(db_path.parent().expect("test DB should have a parent"))
            .expect("test DB directory should be removed");
    }

    #[test]
    fn normalized_claude_tool_name_does_not_use_claude_state_fallback() {
        let db_path = unique_attribution_db_path("normalized-claude");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        db.upsert_claude_model_state(ClaudeModelStateObservation {
            session_id: String::from("cc_session-123"),
            agent_id: String::new(),
            model_id: String::from("claude/parent-model"),
            observation_kind: ObservationKind::SessionStart,
            source: String::from("startup"),
            observed_at_ms: 1,
        })
        .expect("parent state should be seeded");

        let payload = diff_trace_payload_with(
            CLAUDE_TOOL_NAME,
            "session-123",
            PAYLOAD_TYPE_PATCH,
            None,
            None,
        );
        persist_diff_trace_payload_to_agent_trace_db_with_db(&db, &payload)
            .expect("normalized Claude payload should persist");

        let model = db
            .query_map("SELECT model_id FROM diff_traces LIMIT 1", (), |row| {
                row.get::<Option<String>>(0).map_err(Into::into)
            })
            .expect("persisted model should be readable")
            .into_iter()
            .next()
            .expect("diff trace row should exist");
        assert_eq!(model, None);

        drop(db);
        fs::remove_dir_all(db_path.parent().expect("test DB should have a parent"))
            .expect("test DB directory should be removed");
    }

    #[test]
    fn claude_diff_trace_state_lookup_isolated_to_exact_subagent_scope() {
        let db_path = unique_attribution_db_path("subagent");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        db.upsert_claude_model_state(ClaudeModelStateObservation {
            session_id: String::from("cc_session-123"),
            agent_id: String::new(),
            model_id: String::from("claude/parent-model"),
            observation_kind: ObservationKind::SessionStart,
            source: String::from("startup"),
            observed_at_ms: 1,
        })
        .expect("parent state should be seeded");

        let mut event = claude_model_test_event(Path::new("/virtual/missing.jsonl"), "subagent");
        let event_object = event
            .as_object_mut()
            .expect("test event should be an object");
        event_object.remove("transcript_path");
        event_object.remove("tool_use_id");
        event_object.insert("agent_id".to_string(), json!("subagent-1"));
        let payload = parsed_claude_diff_trace(&event);
        persist_diff_trace_payload_to_agent_trace_db_with_db(&db, &payload)
            .expect("subagent diff trace should persist");

        let model = db
            .query_map("SELECT model_id FROM diff_traces LIMIT 1", (), |row| {
                row.get::<Option<String>>(0).map_err(Into::into)
            })
            .expect("persisted model should be readable")
            .into_iter()
            .next()
            .expect("diff trace row should exist");
        assert_eq!(model, None);

        drop(db);
        fs::remove_dir_all(db_path.parent().expect("test DB should have a parent"))
            .expect("test DB directory should be removed");
    }

    #[test]
    fn prefixed_diff_trace_session_id_prefixes_fresh_pi_session_id() {
        assert_eq!(
            prefixed_diff_trace_session_id("pi", "session-123"),
            "pi_session-123"
        );
    }

    #[test]
    fn prefixed_diff_trace_session_id_keeps_already_prefixed_pi_session_id() {
        assert_eq!(
            prefixed_diff_trace_session_id("pi", "pi_session-123"),
            "pi_session-123"
        );
    }

    #[test]
    fn prefixed_diff_trace_session_id_prefixes_fresh_codex_session_id() {
        assert_eq!(
            prefixed_diff_trace_session_id("codex", "session-123"),
            "cx_session-123"
        );
    }

    #[test]
    fn prefixed_diff_trace_session_id_keeps_already_prefixed_codex_session_id() {
        assert_eq!(
            prefixed_diff_trace_session_id("codex", "cx_session-123"),
            "cx_session-123"
        );
    }

    #[test]
    fn prefixed_diff_trace_session_id_adding_codex_does_not_affect_other_tool_prefixes() {
        assert_eq!(
            prefixed_diff_trace_session_id("opencode", "session-123"),
            "oc_session-123"
        );
        assert_eq!(
            prefixed_diff_trace_session_id("claude", "session-123"),
            "cc_session-123"
        );
        assert_eq!(
            prefixed_diff_trace_session_id("pi", "session-123"),
            "pi_session-123"
        );
    }

    #[test]
    fn normalize_codex_model_id_preserves_fresh_model_id() {
        assert_eq!(
            normalize_codex_model_id("gpt-5.6-codex").as_deref(),
            Some("gpt-5.6-codex")
        );
    }

    #[test]
    fn normalize_codex_model_id_preserves_qualified_model_ids() {
        for model in ["openai/gpt-x", "qualified/custom-provider/model"] {
            assert_eq!(normalize_codex_model_id(model).as_deref(), Some(model));
        }
    }

    #[test]
    fn normalize_codex_model_id_preserves_unqualified_model_ids() {
        assert_eq!(
            normalize_codex_model_id("custom-codex-model").as_deref(),
            Some("custom-codex-model")
        );
    }

    #[test]
    fn normalize_codex_model_id_returns_none_for_blank_model_ids() {
        assert_eq!(normalize_codex_model_id("   "), None);
    }

    #[test]
    fn pi_normalized_diff_trace_payload_persists_with_pi_prefixed_session_id() {
        let stdin_payload = serde_json::json!({
            "sessionID": "session-123",
            "diff": "diff text",
            "time": 1_800_000_000_000_u64,
            "model_id": "anthropic/claude-opus-4",
            "tool_name": "pi",
            "tool_version": null
        })
        .to_string();

        let parsed = parse_diff_trace_payload(&stdin_payload)
            .expect("normalized Pi diff-trace payload should parse");
        let payload = match parsed {
            DiffTraceParseResult::Persist(payload) => payload,
            DiffTraceParseResult::NoOp(message) => {
                panic!("Pi payload should persist, got no-op: {message}")
            }
        };

        assert_eq!(payload.tool_name, "pi");
        assert_eq!(payload.model_id.as_deref(), Some("anthropic/claude-opus-4"));
        assert_eq!(payload.tool_version, None);

        persist_diff_trace_payload_to_agent_trace_db_with(
            &payload,
            payload.model_id.as_deref(),
            payload.tool_version.as_deref(),
            |input| {
                assert_eq!(input.time_ms, 1_800_000_000_000_i64);
                assert_eq!(input.session_id, "pi_session-123");
                assert_eq!(input.model_id, Some("anthropic/claude-opus-4"));
                assert_eq!(input.tool_name, "pi");
                assert_eq!(input.tool_version, None);
                assert_eq!(input.payload_type, PAYLOAD_TYPE_PATCH);

                Ok(())
            },
        )
        .expect("Pi diff-trace payload should be persisted");
    }

    #[test]
    fn post_commit_intersection_flow_preserves_pi_provenance() {
        let now_ms = 1_800_000_000_000_i64;
        let commit_time_ms = now_ms - 1_000;

        let output = run_post_commit_intersection_flow_with(
            Path::new("/repo"),
            |_| {
                Ok(PostCommitPatchData {
                    commit_oid: String::from("def456"),
                    commit_time_ms,
                    parsed_patch: valid_patch("src/lib.rs", "shared line"),
                })
            },
            || Ok(now_ms),
            |_, _| {
                Ok(RecentDiffTracePatches {
                    patches: vec![ParsedDiffTracePatch {
                        id: 9,
                        time_ms: now_ms - 500,
                        session_id: String::from("pi_valid-session"),
                        patch: valid_patch("src/lib.rs", "shared line"),
                        tool_name: Some(String::from("pi")),
                        tool_version: None,
                        payload_type: String::from(PAYLOAD_TYPE_PATCH),
                    }],
                    skipped: vec![],
                })
            },
            |_| Ok(()),
        )
        .expect("post-commit intersection flow should succeed");

        assert_eq!(output.combined_recent_patch.files.len(), 1);
        assert_eq!(output.tool_name, Some(String::from("pi")));
        assert_eq!(output.tool_version, None);
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
                assert_eq!(input.session_id, "cc_session-123");
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
                        session_id: String::from("oc_valid-session"),
                        patch: valid_patch("src/lib.rs", "shared line"),
                        tool_name: Some(String::from("opencode")),
                        tool_version: Some(String::from("1.2.3")),
                        payload_type: String::from(PAYLOAD_TYPE_PATCH),
                    }],
                    skipped: vec![SkippedDiffTracePatch {
                        id: 8,
                        time_ms: now_ms - 250,
                        session_id: String::from("oc_malformed-session"),
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

    fn post_commit_flow_result() -> PostCommitIntersectionFlowResult {
        PostCommitIntersectionFlowResult {
            combined_recent_patch: valid_patch("src/lib.rs", "shared line"),
            post_commit_data: PostCommitPatchData {
                commit_oid: String::from("abc123"),
                commit_time_ms: 1_800_000_000_000,
                parsed_patch: valid_patch("src/lib.rs", "shared line"),
            },
            tool_name: None,
            tool_version: None,
        }
    }

    fn minimal_agent_trace() -> AgentTrace {
        serde_json::from_value(json!({ "files": [] }))
            .expect("minimal Agent Trace should deserialize")
    }

    #[test]
    fn post_commit_auto_sync_launches_after_successful_persistence_when_enabled() {
        let events = RefCell::new(Vec::new());

        let output = run_post_commit_subcommand_with(
            Path::new("/repo"),
            None,
            "",
            |_| {
                events.borrow_mut().push("intersection");
                Ok(post_commit_flow_result())
            },
            |_, _, _, _| {
                events.borrow_mut().push("persistence");
                Ok(minimal_agent_trace())
            },
            |_| {
                events.borrow_mut().push("config");
                Ok(true)
            },
            |_| {
                events.borrow_mut().push("launch");
                Ok(())
            },
            |_| {
                events.borrow_mut().push("checkpoint");
                Ok(())
            },
            None,
        )
        .expect("successful post-commit should remain successful");

        assert!(output.contains("post-commit hook processed intersection"));
        assert_eq!(
            events.into_inner(),
            vec![
                "intersection",
                "persistence",
                "checkpoint",
                "config",
                "launch"
            ]
        );
    }

    #[test]
    fn post_commit_validation_failure_does_not_resolve_or_launch_auto_sync() {
        let validation_called = RefCell::new(false);
        let config_called = RefCell::new(false);
        let launch_called = RefCell::new(false);

        let error = run_post_commit_subcommand_with(
            Path::new("/repo"),
            None,
            "",
            |_| Ok(post_commit_flow_result()),
            |_, flow_result, vcs_type, remote_url| {
                run_post_commit_agent_trace_flow_with(
                    flow_result,
                    vcs_type,
                    remote_url,
                    &ParsedPatch { files: Vec::new() },
                    |_| {
                        *validation_called.borrow_mut() = true;
                        Err(anyhow!("Agent Trace validation failed"))
                    },
                    |_| panic!("Agent Trace persistence must not run after validation failure"),
                )
            },
            |_| {
                *config_called.borrow_mut() = true;
                Ok(true)
            },
            |_| {
                *launch_called.borrow_mut() = true;
                Ok(())
            },
            |_| panic!("checkpoint must not run after persistence failure"),
            None,
        )
        .expect_err("validation failure should be returned");

        assert!(*validation_called.borrow());
        assert!(!error.to_string().is_empty());
        assert!(!*config_called.borrow());
        assert!(!*launch_called.borrow());
    }

    fn post_commit_flow_result_for(
        direct: ParsedPatch,
        committed: ParsedPatch,
    ) -> PostCommitIntersectionFlowResult {
        PostCommitIntersectionFlowResult {
            combined_recent_patch: direct,
            post_commit_data: PostCommitPatchData {
                commit_oid: String::from("abc123"),
                commit_time_ms: 1_800_000_000_000,
                parsed_patch: committed,
            },
            tool_name: Some(String::from("claude")),
            tool_version: Some(String::from("9.9.9")),
        }
    }

    fn persisted_post_commit_trace(
        flow_result: &PostCommitIntersectionFlowResult,
        mutation_ai_patch: &ParsedPatch,
    ) -> Value {
        let persisted = RefCell::new(None);

        run_post_commit_agent_trace_flow_with(
            flow_result,
            Some(AgentTraceVcsType::Git),
            "",
            mutation_ai_patch,
            |_| Ok(()),
            |insert| {
                *persisted.borrow_mut() = Some(insert.trace_json.to_string());
                Ok(())
            },
        )
        .expect("post-commit Agent Trace flow should build and persist");

        serde_json::from_str(
            persisted
                .into_inner()
                .expect("trace should have been persisted")
                .as_str(),
        )
        .expect("persisted trace JSON should parse")
    }

    #[test]
    fn post_commit_agent_trace_flow_attributes_mutation_only_lines_as_ai_without_provenance() {
        let flow_result = post_commit_flow_result_for(
            ParsedPatch { files: Vec::new() },
            valid_patch("src/lib.rs", "mutated line"),
        );
        let mutation_ai_patch = valid_patch("src/lib.rs", "mutated line");

        let trace = persisted_post_commit_trace(&flow_result, &mutation_ai_patch);

        assert_eq!(
            trace["metadata"]["sce"]["line_changes"]["ai"]["added"],
            json!(1)
        );
        assert_eq!(
            trace["metadata"]["sce"]["line_changes"]["unknown"]["added"],
            json!(0)
        );
        assert!(
            trace.get("tool").is_none(),
            "mutation-only coverage fabricates no tool provenance"
        );
        let contributor = &trace["files"][0]["conversations"][0]["contributor"];
        assert_eq!(contributor["type"], json!("ai"));
        assert!(
            contributor.get("model_id").is_none(),
            "mutation-only coverage carries no model provenance"
        );
        assert!(
            trace["files"][0]["conversations"][0]
                .get("related")
                .is_none(),
            "mutation-only coverage carries no session provenance"
        );
    }

    #[test]
    fn post_commit_agent_trace_flow_keeps_direct_provenance_when_direct_covers_the_line() {
        let flow_result = post_commit_flow_result_for(
            valid_patch("src/lib.rs", "shared line"),
            valid_patch("src/lib.rs", "shared line"),
        );

        let trace = persisted_post_commit_trace(&flow_result, &ParsedPatch { files: Vec::new() });

        assert_eq!(
            trace["metadata"]["sce"]["line_changes"]["ai"]["added"],
            json!(1)
        );
        assert_eq!(
            trace["tool"],
            json!({ "name": "claude", "version": "9.9.9" })
        );
        assert_eq!(
            trace["files"][0]["conversations"][0]["contributor"]["type"],
            json!("ai")
        );
    }

    #[test]
    fn post_commit_agent_trace_flow_with_empty_mutation_patch_leaves_uncovered_lines_unknown() {
        let flow_result = post_commit_flow_result_for(
            ParsedPatch { files: Vec::new() },
            valid_patch("src/lib.rs", "human line"),
        );

        let trace = persisted_post_commit_trace(&flow_result, &ParsedPatch { files: Vec::new() });

        assert_eq!(
            trace["metadata"]["sce"]["line_changes"]["unknown"]["added"],
            json!(1)
        );
        assert_eq!(
            trace["metadata"]["sce"]["line_changes"]["ai"]["added"],
            json!(0)
        );
        assert!(trace.get("tool").is_none());
        assert_eq!(
            trace["files"][0]["conversations"][0]["contributor"]["type"],
            json!("unknown")
        );
    }

    mod mutation_attribution_e2e {
        use super::*;
        use crate::services::checkout::{get_or_create_checkout_id, resolve_git_dir};
        use crate::services::mutation_trace::runtime::resolve_post_commit_mutation_ai_patch;
        use crate::services::mutation_trace::store::encode_revision;

        fn git(repo: &Path, args: &[&str]) -> String {
            let output = Command::new("git")
                .args(args)
                .current_dir(repo)
                .output()
                .expect("git should spawn");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).expect("git output should be UTF-8")
        }

        fn commit_all(repo: &Path, message: &str) {
            git(repo, &["add", "-A"]);
            git(
                repo,
                &[
                    "-c",
                    "user.name=SCE Test",
                    "-c",
                    "user.email=sce@example.invalid",
                    "commit",
                    "-qm",
                    message,
                ],
            );
        }

        struct E2eRepo {
            _temp: tempfile::TempDir,
            root: PathBuf,
            db_path: PathBuf,
        }

        impl E2eRepo {
            fn new(label: &str) -> Self {
                let temp = tempfile::Builder::new()
                    .prefix(&format!("sce-mutation-attr-e2e-{label}-"))
                    .tempdir()
                    .expect("temp dir should be created");
                let root = temp.path().join("repo");
                fs::create_dir_all(&root).expect("repo dir should be created");
                git(&root, &["init", "-q"]);
                git(
                    &root,
                    &["remote", "add", "origin", "git@github.com:acme/widgets.git"],
                );
                fs::write(root.join("file.rs"), "one\n").expect("seed file should write");
                commit_all(&root, "base");
                let db_path = temp.path().join("agent-trace.db");
                RepositoryAgentTraceDb::new_at(&db_path)
                    .expect("repository DB should open with schema");
                Self {
                    _temp: temp,
                    root,
                    db_path,
                }
            }

            fn db(&self) -> RepositoryAgentTraceDb {
                RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&self.db_path)
                    .expect("repository DB should reopen")
            }

            fn head_tree(&self) -> String {
                git(&self.root, &["rev-parse", "HEAD^{tree}"])
                    .trim()
                    .to_owned()
            }

            fn parent_tree(&self) -> String {
                git(&self.root, &["rev-parse", "HEAD~1^{tree}"])
                    .trim()
                    .to_owned()
            }

            fn checkout_id(&self) -> String {
                let git_dir = resolve_git_dir(&self.root).expect("git dir should resolve");
                get_or_create_checkout_id(&git_dir).expect("checkout identity should resolve")
            }
        }

        fn seed_event(
            db: &RepositoryAgentTraceDb,
            worktree_id: &str,
            revision: u64,
            before_tree: &str,
            after_tree: &str,
            attribution_kind: &str,
            attribution_scope_id: Option<&str>,
        ) {
            db.execute(
                "INSERT INTO mutation_trace_events
                    (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
                     attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id,
                     boundary_event_id)
                 VALUES (?1, ?2, ?3, ?4, 0, 'healthy', ?5, ?6, 'flush', NULL, NULL)",
                (
                    worktree_id,
                    encode_revision(revision).as_slice(),
                    before_tree,
                    after_tree,
                    attribution_kind,
                    attribution_scope_id,
                ),
            )
            .expect("mutation event insert should succeed");
        }

        fn row_count(db: &RepositoryAgentTraceDb, table: &str) -> i64 {
            db.query_map(&format!("SELECT COUNT(*) FROM {table}"), (), |row| {
                row.get::<i64>(0).map_err(anyhow::Error::from)
            })
            .expect("count query should succeed")
            .into_iter()
            .next()
            .expect("count row should exist")
        }

        fn touched_line_count(patch: &ParsedPatch) -> usize {
            patch
                .files
                .iter()
                .flat_map(|file| file.hunks.iter())
                .map(|hunk| hunk.lines.len())
                .sum()
        }

        fn flow_result_for(
            repo: &E2eRepo,
            direct: ParsedPatch,
        ) -> PostCommitIntersectionFlowResult {
            let post_commit_data = capture_post_commit_patch_from_git(&repo.root)
                .expect("capturing the post-commit patch should succeed");
            PostCommitIntersectionFlowResult {
                combined_recent_patch: direct,
                post_commit_data,
                tool_name: None,
                tool_version: None,
            }
        }

        fn resolve_mutation_ai(
            repo: &E2eRepo,
            db: &RepositoryAgentTraceDb,
            flow_result: &PostCommitIntersectionFlowResult,
        ) -> ParsedPatch {
            let direct_intersection = intersect_patches_fn(
                &flow_result.combined_recent_patch,
                &flow_result.post_commit_data.parsed_patch,
            );
            resolve_post_commit_mutation_ai_patch(
                &repo.root,
                db,
                &direct_intersection,
                &flow_result.post_commit_data.parsed_patch,
            )
        }

        fn persist_trace(
            flow_result: &PostCommitIntersectionFlowResult,
            db: &RepositoryAgentTraceDb,
            mutation_ai_patch: &ParsedPatch,
        ) -> Value {
            let persisted = RefCell::new(None);
            run_post_commit_agent_trace_flow_with(
                flow_result,
                Some(AgentTraceVcsType::Git),
                "git@github.com:acme/widgets.git",
                mutation_ai_patch,
                |value| {
                    validate_agent_trace_value(value).map_err(|error| anyhow!(error.to_string()))
                },
                |insert| {
                    *persisted.borrow_mut() = Some(insert.trace_json.to_string());
                    db.insert_agent_trace(insert).map(|_| ())
                },
            )
            .expect("the post-commit Agent Trace flow should build, validate, and persist");

            serde_json::from_str(
                persisted
                    .into_inner()
                    .expect("a trace should have been persisted")
                    .as_str(),
            )
            .expect("the persisted trace JSON should parse")
        }

        #[test]
        fn a_mutation_only_line_persists_as_ai_without_fabricated_provenance() {
            let repo = E2eRepo::new("mutation-only");
            fs::write(repo.root.join("file.rs"), "one\ntwo\n").expect("the edit should write");
            commit_all(&repo.root, "add two");

            let db = repo.db();
            seed_event(
                &db,
                &repo.checkout_id(),
                1,
                &repo.parent_tree(),
                &repo.head_tree(),
                "ai_exclusive",
                Some("scope-x"),
            );

            let flow_result = flow_result_for(&repo, ParsedPatch { files: Vec::new() });
            let mutation_ai_patch = resolve_mutation_ai(&repo, &db, &flow_result);
            assert_eq!(
                touched_line_count(&mutation_ai_patch),
                1,
                "a healthy untainted exclusive event covers the committed line"
            );

            let trace = persist_trace(&flow_result, &db, &mutation_ai_patch);
            assert_eq!(
                trace["metadata"]["sce"]["line_changes"]["ai"]["added"],
                json!(1)
            );
            assert_eq!(
                trace["metadata"]["sce"]["line_changes"]["unknown"]["added"],
                json!(0)
            );
            assert!(
                trace.get("tool").is_none(),
                "mutation-only coverage fabricates no tool provenance"
            );
            let contributor = &trace["files"][0]["conversations"][0]["contributor"];
            assert_eq!(contributor["type"], json!("ai"));
            assert!(
                contributor.get("model_id").is_none(),
                "mutation-only coverage carries no model provenance"
            );

            assert_eq!(
                row_count(&db, "diff_traces"),
                0,
                "mutation evidence is never inserted into diff_traces"
            );
            assert_eq!(
                row_count(&db, "post_commit_patch_intersections"),
                0,
                "the direct-only intersection table is untouched by this flow"
            );
            assert_eq!(row_count(&db, "agent_traces"), 1);
        }

        #[test]
        fn direct_plus_mutation_evidence_completes_hunk_coverage_and_keeps_direct_provenance() {
            let repo = E2eRepo::new("direct-plus-mutation");
            fs::write(repo.root.join("file.rs"), "one\ntwo\nthree\n")
                .expect("the edit should write");
            commit_all(&repo.root, "add two and three");

            let db = repo.db();
            seed_event(
                &db,
                &repo.checkout_id(),
                1,
                &repo.parent_tree(),
                &repo.head_tree(),
                "ai_exclusive",
                Some("scope-x"),
            );

            let direct = parse_patch_from_text(
                "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1,1 +1,2 @@\n one\n+two\n",
                None,
            )
            .expect("the direct patch should parse");
            let mut flow_result = flow_result_for(&repo, direct);
            flow_result.tool_name = Some(String::from("claude"));
            flow_result.tool_version = Some(String::from("9.9.9"));

            let mutation_ai_patch = resolve_mutation_ai(&repo, &db, &flow_result);
            assert_eq!(
                touched_line_count(&mutation_ai_patch),
                1,
                "only the line direct evidence did not cover is resolved from mutation history"
            );

            let trace = persist_trace(&flow_result, &db, &mutation_ai_patch);
            assert_eq!(
                trace["metadata"]["sce"]["line_changes"]["ai"]["added"],
                json!(2),
                "the union of direct and mutation coverage classifies the hunk ai"
            );
            assert_eq!(
                trace["metadata"]["sce"]["line_changes"]["unknown"]["added"],
                json!(0)
            );
            assert_eq!(
                trace["tool"],
                json!({ "name": "claude", "version": "9.9.9" })
            );
        }

        #[test]
        fn a_newer_nonexclusive_event_keeps_the_line_non_ai() {
            let repo = E2eRepo::new("newer-nonexclusive");
            fs::write(repo.root.join("file.rs"), "one\ntwo\n").expect("the edit should write");
            commit_all(&repo.root, "add two");

            let db = repo.db();
            let worktree = repo.checkout_id();
            seed_event(
                &db,
                &worktree,
                1,
                &repo.parent_tree(),
                &repo.head_tree(),
                "ai_exclusive",
                Some("scope-old"),
            );
            seed_event(
                &db,
                &worktree,
                2,
                &repo.parent_tree(),
                &repo.head_tree(),
                "ai_contended",
                None,
            );

            let flow_result = flow_result_for(&repo, ParsedPatch { files: Vec::new() });
            let mutation_ai_patch = resolve_mutation_ai(&repo, &db, &flow_result);
            assert_eq!(
                touched_line_count(&mutation_ai_patch),
                0,
                "the newer contended match resolves the line and blocks the older exclusive event"
            );

            let trace = persist_trace(&flow_result, &db, &mutation_ai_patch);
            assert_eq!(
                trace["metadata"]["sce"]["line_changes"]["unknown"]["added"],
                json!(1)
            );
            assert_eq!(
                trace["metadata"]["sce"]["line_changes"]["ai"]["added"],
                json!(0)
            );
            assert_eq!(
                trace["files"][0]["conversations"][0]["contributor"]["type"],
                json!("unknown")
            );
        }

        #[test]
        fn an_adversarial_foreign_worktree_event_cannot_block_the_current_worktrees_exclusive_event(
        ) {
            let repo = E2eRepo::new("adversarial-linked");

            let linked_root = repo
                .root
                .parent()
                .expect("the repo should have a parent directory")
                .join("linked");
            git(
                &repo.root,
                &[
                    "worktree",
                    "add",
                    "-q",
                    linked_root.to_str().expect("worktree path should be UTF-8"),
                ],
            );

            fs::write(repo.root.join("file.rs"), "one\ntwo\n").expect("the edit should write");
            commit_all(&repo.root, "add two");

            let db = repo.db();
            let current_worktree = repo.checkout_id();
            let linked_git_dir =
                resolve_git_dir(&linked_root).expect("the linked git dir should resolve");
            let foreign_worktree = get_or_create_checkout_id(&linked_git_dir)
                .expect("the linked worktree's checkout identity should resolve");
            assert_ne!(
                current_worktree, foreign_worktree,
                "the linked worktree must derive its own distinct identity"
            );

            seed_event(
                &db,
                &current_worktree,
                1,
                &repo.parent_tree(),
                &repo.head_tree(),
                "ai_exclusive",
                Some("scope-current"),
            );
            seed_event(
                &db,
                &foreign_worktree,
                2,
                &repo.parent_tree(),
                &repo.head_tree(),
                "ai_contended",
                None,
            );

            let flow_result = flow_result_for(&repo, ParsedPatch { files: Vec::new() });
            let mutation_ai_patch = resolve_mutation_ai(&repo, &db, &flow_result);
            assert_eq!(
                touched_line_count(&mutation_ai_patch),
                1,
                "only the current worktree's history is eligible, so the older exclusive event contributes"
            );

            let trace = persist_trace(&flow_result, &db, &mutation_ai_patch);
            assert_eq!(
                trace["metadata"]["sce"]["line_changes"]["ai"]["added"],
                json!(1),
                "worktree isolation lets the current worktree's exclusive event classify the target ai"
            );
            assert_eq!(
                trace["files"][0]["conversations"][0]["contributor"]["type"],
                json!("ai")
            );
            assert!(trace.get("tool").is_none());
        }

        fn touched_contents(patch: &ParsedPatch) -> Vec<String> {
            patch
                .files
                .iter()
                .flat_map(|file| file.hunks.iter())
                .flat_map(|hunk| hunk.lines.iter())
                .map(|line| line.content.clone())
                .collect()
        }

        #[test]
        #[allow(clippy::too_many_lines)]
        fn persistence_boundaries_stay_separated_across_diff_traces_intersection_and_agent_trace() {
            let repo = E2eRepo::new("persistence-boundary");

            fs::write(repo.root.join("file.rs"), "one\ntwo\n")
                .expect("the direct edit should write");
            git(&repo.root, &["add", "-A"]);
            let intermediate_tree = git(&repo.root, &["write-tree"]).trim().to_owned();

            fs::write(repo.root.join("file.rs"), "one\ntwo\nthree\n")
                .expect("the mutation edit should write");
            commit_all(&repo.root, "add two and three");

            let base_tree = repo.parent_tree();
            let final_tree = repo.head_tree();
            assert_ne!(
                base_tree, intermediate_tree,
                "the direct edit must move the tree"
            );
            assert_ne!(
                intermediate_tree, final_tree,
                "the mutation edit must move the tree again"
            );

            let db = repo.db();

            let now_ms = current_unix_time_ms().expect("the clock should resolve");
            db.insert_diff_trace(DiffTraceInsert {
                time_ms: now_ms - 60_000,
                session_id: "cc_session-direct",
                patch: "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1,1 +1,2 @@\n one\n+two\n",
                model_id: Some("claude/model-direct"),
                tool_name: "claude",
                tool_version: Some("9.9.9"),
                payload_type: PAYLOAD_TYPE_PATCH,
            })
            .expect("the direct diff_traces row should insert");

            seed_event(
                &db,
                &repo.checkout_id(),
                1,
                &intermediate_tree,
                &final_tree,
                "ai_exclusive",
                Some("scope-mutation"),
            );

            let flow_result = run_post_commit_intersection_flow_with(
                &repo.root,
                capture_post_commit_patch_from_git,
                current_unix_time_ms,
                |cutoff_ms, end_ms| db.recent_diff_trace_patches(cutoff_ms, end_ms),
                |insert| db.insert_post_commit_patch_intersection(insert).map(|_| ()),
            )
            .expect("the real post-commit intersection flow should run");
            assert_eq!(
                touched_contents(&flow_result.combined_recent_patch),
                vec!["two".to_owned()],
                "the combined recent patch comes from the real diff_traces query, not an in-memory patch"
            );

            let mutation_ai_patch = resolve_mutation_ai(&repo, &db, &flow_result);
            assert_eq!(
                touched_contents(&mutation_ai_patch),
                vec!["three".to_owned()],
                "mutation history resolves only the committed line direct evidence missed"
            );

            persist_trace(&flow_result, &db, &mutation_ai_patch);

            assert_eq!(
                row_count(&db, "diff_traces"),
                1,
                "mutation attribution must not create another diff_traces row"
            );
            let stored_direct_patch: String = db
                .query_map("SELECT patch FROM diff_traces", (), |row| {
                    row.get::<String>(0).map_err(anyhow::Error::from)
                })
                .expect("diff_traces query should succeed")
                .into_iter()
                .next()
                .expect("one diff_traces row should exist");
            let stored_direct = parse_patch_from_text(&stored_direct_patch, None)
                .expect("the stored direct patch should parse");
            assert_eq!(
                touched_contents(&stored_direct),
                vec!["two".to_owned()],
                "the direct diff_traces row contains 'two' and never 'three'"
            );

            assert_eq!(
                row_count(&db, "post_commit_patch_intersections"),
                1,
                "the intersection flow persists exactly one direct-only row"
            );
            let stored_intersection_json: String = db
                .query_map(
                    "SELECT intersection_patch FROM post_commit_patch_intersections",
                    (),
                    |row| row.get::<String>(0).map_err(anyhow::Error::from),
                )
                .expect("intersection query should succeed")
                .into_iter()
                .next()
                .expect("one intersection row should exist");
            let stored_intersection = load_patch_from_json(&stored_intersection_json)
                .expect("the persisted intersection patch should reconstruct");
            assert_eq!(
                touched_contents(&stored_intersection),
                vec!["two".to_owned()],
                "post_commit_patch_intersections stays direct-only; the mutation line 'three' \
                 must never contaminate this table"
            );

            assert_eq!(row_count(&db, "agent_traces"), 1);
            let stored_trace_json: String = db
                .query_map("SELECT trace_json FROM agent_traces", (), |row| {
                    row.get::<String>(0).map_err(anyhow::Error::from)
                })
                .expect("agent_traces query should succeed")
                .into_iter()
                .next()
                .expect("one Agent Trace row should exist");
            let trace: Value = serde_json::from_str(&stored_trace_json)
                .expect("the persisted Agent Trace JSON should parse");
            validate_agent_trace_value(&trace).expect(
                "the persisted agent_traces.trace_json validates against the embedded Agent Trace schema",
            );
            assert_eq!(
                trace["metadata"]["sce"]["line_changes"]["ai"]["added"],
                json!(2),
                "direct + mutation coverage classifies both committed added lines as ai"
            );
            assert_eq!(
                trace["metadata"]["sce"]["line_changes"]["unknown"]["added"],
                json!(0)
            );
            assert_eq!(
                trace["files"][0]["conversations"][0]["contributor"]["type"],
                json!("ai")
            );

            assert_eq!(
                trace["tool"],
                json!({ "name": "claude", "version": "9.9.9" })
            );

            assert_eq!(
                row_count(&db, "mutation_trace_events"),
                1,
                "attribution performs no mutation-cursor write"
            );
        }
    }

    #[test]
    fn post_commit_auto_sync_does_not_launch_when_disabled() {
        let launch_called = RefCell::new(false);

        run_post_commit_subcommand_with(
            Path::new("/repo"),
            None,
            "",
            |_| Ok(post_commit_flow_result()),
            |_, _, _, _| Ok(minimal_agent_trace()),
            |_| Ok(false),
            |_| {
                *launch_called.borrow_mut() = true;
                Ok(())
            },
            |_| Ok(()),
            None,
        )
        .expect("disabled auto-sync should not affect post-commit success");

        assert!(!*launch_called.borrow());
    }

    #[test]
    fn post_commit_persistence_failure_does_not_launch_auto_sync() {
        let launch_called = RefCell::new(false);

        let error = run_post_commit_subcommand_with(
            Path::new("/repo"),
            None,
            "",
            |_| Ok(post_commit_flow_result()),
            |_, _, _, _| Err(anyhow!("Agent Trace persistence failed")),
            |_| panic!("auto-sync config must not be resolved after persistence failure"),
            |_| {
                *launch_called.borrow_mut() = true;
                Ok(())
            },
            |_| panic!("checkpoint must not run after persistence failure"),
            None,
        )
        .expect_err("persistence failure should be returned");

        assert!(error.to_string().contains("persistence failed"));
        assert!(!*launch_called.borrow());
    }

    #[test]
    fn post_commit_auto_sync_launcher_failure_is_fail_open() {
        let output = run_post_commit_subcommand_with(
            Path::new("/repo"),
            None,
            "",
            |_| Ok(post_commit_flow_result()),
            |_, _, _, _| Ok(minimal_agent_trace()),
            |_| Ok(true),
            |_| Err(anyhow!("spawn unavailable")),
            |_| Ok(()),
            None,
        )
        .expect("launcher failure must not affect post-commit success");

        assert!(output.contains("post-commit hook processed intersection"));
    }

    #[derive(Default)]
    struct RecordingLogger {
        warnings: std::sync::Mutex<Vec<(String, String)>>,
    }

    impl Logger for RecordingLogger {
        fn info(
            &self,
            _event_id: &str,
            _message: &str,
            _fields: &[(&str, &str)],
            _session_id: Option<&str>,
        ) {
        }

        fn debug(
            &self,
            _event_id: &str,
            _message: &str,
            _fields: &[(&str, &str)],
            _session_id: Option<&str>,
        ) {
        }

        fn warn(
            &self,
            event_id: &str,
            message: &str,
            _fields: &[(&str, &str)],
            _session_id: Option<&str>,
        ) {
            self.warnings
                .lock()
                .expect("warnings mutex should not be poisoned")
                .push((event_id.to_string(), message.to_string()));
        }

        fn error(
            &self,
            _event_id: &str,
            _message: &str,
            _fields: &[(&str, &str)],
            _session_id: Option<&str>,
        ) {
        }

        fn log_cli_error(
            &self,
            _error: &crate::services::error::CliError,
            _session_id: Option<&str>,
        ) {
        }
    }

    #[test]
    fn post_commit_checkpoint_runs_once_after_successful_persistence() {
        let events = RefCell::new(Vec::new());

        let output = run_post_commit_subcommand_with(
            Path::new("/repo"),
            None,
            "",
            |_| Ok(post_commit_flow_result()),
            |_, _, _, _| {
                events.borrow_mut().push("persistence");
                Ok(minimal_agent_trace())
            },
            |_| Ok(false),
            |_| Ok(()),
            |_| {
                events.borrow_mut().push("checkpoint");
                Ok(())
            },
            None,
        )
        .expect("successful checkpoint should not affect post-commit success");

        assert!(output.contains("post-commit hook processed intersection"));
        assert_eq!(events.into_inner(), vec!["persistence", "checkpoint"]);
    }

    #[test]
    fn post_commit_checkpoint_failure_is_fail_open_and_logs_warning() {
        let logger = RecordingLogger::default();
        let persisted = RefCell::new(false);

        let output = run_post_commit_subcommand_with(
            Path::new("/repo"),
            None,
            "",
            |_| Ok(post_commit_flow_result()),
            |_, _, _, _| {
                *persisted.borrow_mut() = true;
                Ok(minimal_agent_trace())
            },
            |_| Ok(false),
            |_| Ok(()),
            |_| Err(anyhow!("checkpoint failed")),
            Some(&logger),
        )
        .expect("checkpoint failure must not affect post-commit success");

        assert!(output.contains("post-commit hook processed intersection"));
        assert!(*persisted.borrow());
        assert_eq!(
            logger
                .warnings
                .into_inner()
                .expect("warnings mutex should not be poisoned"),
            vec![(
                String::from("sce.agent_trace_db.passive_checkpoint_failed"),
                String::from("checkpoint failed")
            )]
        );
    }
}
