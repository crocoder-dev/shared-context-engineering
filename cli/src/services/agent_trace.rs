//! Minimal agent-trace domain model and hunk classification contract.
//!
//! This module defines the domain types for producing a minimal agent-trace
//! JSON payload from patch data. The classification logic computes the
//! intersection of a constructed patch and a post-commit patch
//! (`intersection_patch = intersect_patches(constructed_patch, post_commit_patch)`),
//! then compares `intersection_patch` against `post_commit_patch` hunk by hunk,
//! anchored to `post_commit_patch` as the canonical source of truth.
//!
//! Classification rules:
//! - **exact** line-by-line match between `intersection_patch` and `post_commit_patch` hunk => `ai`
//! - same hunk slot in `post_commit_patch` but not exact line-by-line match => `mixed`
//! - hunk present in `post_commit_patch` but missing from `intersection_patch` => `unknown`

use std::{error::Error, fmt, path::Path, sync::OnceLock};

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset};
use jsonschema::{validator_for, Validator};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::{NoContext, Timestamp, Uuid};

use super::patch::{
    intersect_patches, parse_patch, FileChangeKind, ParsedPatch, PatchFileChange, PatchHunk,
    TouchedLineKind,
};

pub const AGENT_TRACE_VERSION: &str = "0.1.0";
pub const SCE_METADATA_VERSION: &str = env!("CARGO_PKG_VERSION");

const RANGE_CONTENT_HASH_PREFIX: &str = "sha256:";
const RANGE_CONTENT_HASH_INPUT_VERSION: &[u8] = b"sce-agent-trace-range-content-hash-v1\0";
const TOUCHED_LINE_ADDED_TAG: &[u8] = b"added\0";
const TOUCHED_LINE_REMOVED_TAG: &[u8] = b"removed\0";

fn default_agent_trace_version() -> String {
    AGENT_TRACE_VERSION.to_owned()
}

fn default_agent_trace_metadata() -> AgentTraceMetadata {
    AgentTraceMetadata {
        sce: AgentTraceSceMetadata {
            version: SCE_METADATA_VERSION.to_owned(),
        },
    }
}

fn generate_agent_trace_id(commit_time: DateTime<FixedOffset>) -> Result<String> {
    let seconds = u64::try_from(commit_time.timestamp()).with_context(|| {
        format!(
            "Invalid commit timestamp '{}': unix seconds must be non-negative.",
            commit_time.to_rfc3339()
        )
    })?;
    let timestamp = Timestamp::from_unix(NoContext, seconds, commit_time.timestamp_subsec_nanos());

    Ok(Uuid::new_v7(timestamp).to_string())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentTraceMetadataInput<'a> {
    pub commit_timestamp: &'a str,
    pub commit_revision: &'a str,
    pub vcs_type: Option<AgentTraceVcsType>,
    pub tool_name: Option<&'a str>,
    pub tool_version: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTraceVcsType {
    Git,
    Jj,
    Hg,
    Svn,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentTraceVcs {
    #[serde(rename = "type")]
    pub r#type: AgentTraceVcsType,
    pub revision: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentTraceTool {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentTraceMetadata {
    pub sce: AgentTraceSceMetadata,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentTraceSceMetadata {
    pub version: String,
}

fn parse_commit_timestamp(commit_timestamp: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(commit_timestamp).with_context(|| {
        format!("Invalid commit timestamp '{commit_timestamp}': expected RFC 3339 date-time.")
    })
}

#[allow(dead_code)]
const AGENT_TRACE_SCHEMA_PATH: &str = "config/schema/agent-trace.schema.json";
#[allow(dead_code)]
pub(crate) const AGENT_TRACE_SCHEMA_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/schema/agent-trace.schema.json"
));

#[allow(dead_code)]
static AGENT_TRACE_SCHEMA_VALIDATOR: OnceLock<Validator> = OnceLock::new();

#[derive(Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum AgentTraceValidationError {
    FileRead { path: String, message: String },
    InvalidJson { message: String },
    SchemaValidation { errors: Vec<String> },
}

impl fmt::Display for AgentTraceValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileRead { path, message } => {
                write!(f, "Agent Trace JSON file could not be read at '{path}': {message}")
            }
            Self::InvalidJson { message } => {
                write!(f, "Agent Trace JSON must be valid JSON: {message}")
            }
            Self::SchemaValidation { errors } => write!(
                f,
                "Agent Trace JSON failed schema validation against embedded schema '{AGENT_TRACE_SCHEMA_PATH}': {}",
                errors.join(" | ")
            ),
        }
    }
}

impl Error for AgentTraceValidationError {}

#[allow(dead_code)]
fn agent_trace_schema_validator() -> &'static Validator {
    AGENT_TRACE_SCHEMA_VALIDATOR.get_or_init(|| {
        let schema: Value = serde_json::from_str(AGENT_TRACE_SCHEMA_JSON)
            .expect("agent trace schema JSON should parse");
        validator_for(&schema).expect("agent trace schema JSON should compile")
    })
}

#[allow(dead_code)]
pub(crate) fn validate_agent_trace_value(value: &Value) -> Result<(), AgentTraceValidationError> {
    let mut errors = agent_trace_schema_validator()
        .iter_errors(value)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();

    if errors.is_empty() {
        return Ok(());
    }

    errors.sort();

    Err(AgentTraceValidationError::SchemaValidation { errors })
}

#[allow(dead_code)]
pub(crate) fn validate_agent_trace_json(raw: &str) -> Result<(), AgentTraceValidationError> {
    let value: Value =
        serde_json::from_str(raw).map_err(|error| AgentTraceValidationError::InvalidJson {
            message: error.to_string(),
        })?;

    validate_agent_trace_value(&value)
}

#[allow(dead_code)]
pub(crate) fn validate_agent_trace_file(path: &Path) -> Result<(), AgentTraceValidationError> {
    let raw =
        std::fs::read_to_string(path).map_err(|error| AgentTraceValidationError::FileRead {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;

    validate_agent_trace_json(&raw)
}
/// Classification of a single hunk's origin relative to the AI candidate patch.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HunkContributor {
    /// All touched lines in the `post_commit_patch` hunk are present identically
    /// in `intersection_patch`.
    Ai,
    /// The `post_commit_patch` hunk has a corresponding slot in `intersection_patch`
    /// but the content differs.
    Mixed,
    /// The `post_commit_patch` hunk has no corresponding slot in `intersection_patch`
    /// (missing from AI candidate).
    Unknown,
}

/// A single conversation entry derived from one `post_commit_patch` hunk.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Conversation {
    /// Classification of this hunk's origin.
    pub contributor: Contributor,
    /// Line ranges in the new file, derived from the `post_commit_patch` hunk metadata.
    pub ranges: Vec<LineRange>,
}

/// Nested contributor object for a conversation entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Contributor {
    /// Classification of this hunk's origin.
    #[serde(rename = "type")]
    pub kind: HunkContributor,
    /// Model provenance for this contributor; omitted when unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

/// A single line range in the new file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct LineRange {
    pub start_line: u64,
    pub end_line: u64,
}

/// A file-level entry in the minimal agent-trace payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TraceFile {
    /// Post-change file path (from `post_commit_patch`).
    pub path: String,
    /// One conversation per `post_commit_patch` hunk, in `post_commit_patch` hunk order.
    pub conversations: Vec<Conversation>,
}

/// Top-level minimal agent-trace payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentTrace {
    /// Agent-trace payload version.
    #[serde(default = "default_agent_trace_version")]
    pub version: String,
    /// Trace record identifier (`UUIDv7` generated from commit-time metadata).
    #[serde(default)]
    pub id: String,
    /// RFC 3339 timestamp string sourced from caller-provided commit metadata.
    #[serde(default)]
    pub timestamp: String,
    /// Version control metadata sourced from caller-provided commit metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vcs: Option<AgentTraceVcs>,
    /// Optional tool metadata sourced from caller-provided metadata input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<AgentTraceTool>,
    /// Implementation-specific metadata for SCE-generated traces.
    #[serde(default = "default_agent_trace_metadata")]
    pub metadata: AgentTraceMetadata,
    /// File-level trace entries, one per file present in `post_commit_patch`.
    pub files: Vec<TraceFile>,
}

/// Classify a single `post_commit_patch` hunk against the corresponding
/// `intersection_patch` hunk (if any).
///
/// Two hunks correspond when they share the same `old_start` value within the
/// same file. This is the slot-matching rule that aligns `intersection_patch`
/// hunks to `post_commit_patch` hunks for comparison.
///
/// Returns:
/// - `HunkContributor::Ai` when the `intersection_patch` hunk exists and its
///   touched lines match the `post_commit_patch` hunk's touched lines exactly
///   (same count, same kind, same `line_number`, same content, in the same order).
/// - `HunkContributor::Mixed` when the `intersection_patch` hunk exists but its
///   touched lines differ from the `post_commit_patch` hunk's touched lines.
/// - `HunkContributor::Unknown` when no `intersection_patch` hunk with the same
///   `old_start` exists for this `post_commit_patch` hunk.
pub fn classify_hunk(
    post_commit_hunk: &PatchHunk,
    intersection_hunks: &[PatchHunk],
) -> HunkContributor {
    let Some(intersection_hunk) = intersection_hunks
        .iter()
        .find(|h| h.old_start == post_commit_hunk.old_start)
    else {
        return HunkContributor::Unknown;
    };

    if hunks_match_exactly(post_commit_hunk, intersection_hunk) {
        HunkContributor::Ai
    } else {
        HunkContributor::Mixed
    }
}

/// Check whether two hunks have identical touched lines in the same order.
fn hunks_match_exactly(left: &PatchHunk, right: &PatchHunk) -> bool {
    if left.lines.len() != right.lines.len() {
        return false;
    }
    left.lines.iter().zip(right.lines.iter()).all(|(ll, rl)| {
        ll.kind == rl.kind && ll.line_number == rl.line_number && ll.content == rl.content
    })
}

#[allow(dead_code)]
pub(crate) fn range_content_hash(hunk: &PatchHunk) -> String {
    let mut hasher = Sha256::new();
    hasher.update(RANGE_CONTENT_HASH_INPUT_VERSION);

    for line in &hunk.lines {
        let kind_tag = match line.kind {
            TouchedLineKind::Added => TOUCHED_LINE_ADDED_TAG,
            TouchedLineKind::Removed => TOUCHED_LINE_REMOVED_TAG,
        };
        let content = line.content.as_bytes();

        hasher.update(kind_tag);
        hasher.update((content.len() as u64).to_be_bytes());
        hasher.update(content);
        hasher.update(b"\0");
    }

    let digest = hasher.finalize();
    format!("{RANGE_CONTENT_HASH_PREFIX}{}", hex_lowercase(&digest))
}

fn hex_lowercase(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn line_range_from_hunk(file: &PatchFileChange, hunk: &PatchHunk) -> LineRange {
    let (start_line, line_count) = match file.kind {
        FileChangeKind::Deleted if hunk.new_count == 0 => (hunk.old_start, hunk.old_count),
        _ => (hunk.new_start, hunk.new_count),
    };
    let end_line = start_line.saturating_add(line_count.saturating_sub(1));

    LineRange {
        start_line,
        end_line,
    }
}

fn trace_path(file: &PatchFileChange) -> &str {
    if file.new_path.is_empty() {
        &file.old_path
    } else {
        &file.new_path
    }
}

fn parse_embedded_deleted_patch(file: &PatchFileChange) -> Option<ParsedPatch> {
    if file.kind != FileChangeKind::Deleted
        || !Path::new(&file.old_path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("patch"))
    {
        return None;
    }

    let embedded_patch = file
        .hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .filter(|line| line.kind == TouchedLineKind::Removed)
        .map(|line| line.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    let parsed_patch = parse_patch(&embedded_patch).ok()?;
    (!parsed_patch.files.is_empty()).then_some(parsed_patch)
}

fn build_trace_file(
    post_commit_file: &PatchFileChange,
    intersection_patch: &ParsedPatch,
) -> Option<TraceFile> {
    if post_commit_file.hunks.is_empty() {
        return None;
    }

    let intersection_file = intersection_patch
        .files
        .iter()
        .find(|ifile| ifile.new_path == post_commit_file.new_path);

    let conversations = post_commit_file
        .hunks
        .iter()
        .map(|post_commit_hunk| {
            let (contributor_kind, contributor_model_id) = match intersection_file {
                Some(ifile) => {
                    let contributor_kind = classify_hunk(post_commit_hunk, &ifile.hunks);
                    let matched_intersection_hunk = ifile
                        .hunks
                        .iter()
                        .find(|h| h.old_start == post_commit_hunk.old_start);
                    let contributor_model_id = match contributor_kind {
                        HunkContributor::Ai | HunkContributor::Mixed => {
                            matched_intersection_hunk.and_then(|hunk| hunk.model_id.clone())
                        }
                        HunkContributor::Unknown => None,
                    };
                    (contributor_kind, contributor_model_id)
                }
                None => (HunkContributor::Unknown, None),
            };
            Conversation {
                contributor: Contributor {
                    kind: contributor_kind,
                    model_id: contributor_model_id,
                },
                ranges: vec![line_range_from_hunk(post_commit_file, post_commit_hunk)],
            }
        })
        .collect();

    Some(TraceFile {
        path: trace_path(post_commit_file).to_string(),
        conversations,
    })
}

/// Build the minimal agent-trace payload from two patches.
///
/// Computes `intersection_patch = intersect_patches(constructed_patch, post_commit_patch)`,
/// then iterates over `post_commit_patch`'s files and hunks to classify each hunk
/// against `intersection_patch`. Deleted `.patch` files whose removed contents are
/// themselves valid patch text are expanded into trace entries for the embedded
/// patch's files. Metadata-only entries with no hunks are omitted. The output
/// preserves the surrounding `post_commit_patch` file ordering and per-file hunk
/// ordering.
///
/// Files in `post_commit_patch` that have no corresponding file in
/// `intersection_patch` still appear in the output with all hunks classified
/// as `Unknown`.
#[allow(dead_code)]
pub fn build_agent_trace(
    constructed_patch: &ParsedPatch,
    post_commit_patch: &ParsedPatch,
    metadata: AgentTraceMetadataInput<'_>,
) -> Result<AgentTrace> {
    let commit_time = parse_commit_timestamp(metadata.commit_timestamp)?;
    let timestamp = metadata.commit_timestamp.to_owned();
    let intersection_patch = intersect_patches(constructed_patch, post_commit_patch);

    let mut files = Vec::new();

    for post_commit_file in &post_commit_patch.files {
        if let Some(embedded_patch) = parse_embedded_deleted_patch(post_commit_file) {
            let embedded_intersection = intersect_patches(constructed_patch, &embedded_patch);
            files.extend(embedded_patch.files.iter().filter_map(|embedded_file| {
                build_trace_file(embedded_file, &embedded_intersection)
            }));
            continue;
        }

        if let Some(trace_file) = build_trace_file(post_commit_file, &intersection_patch) {
            files.push(trace_file);
        }
    }

    let tool = if !intersection_patch.files.is_empty()
        && (metadata.tool_name.is_some() || metadata.tool_version.is_some())
    {
        Some(AgentTraceTool {
            name: metadata.tool_name.map(ToOwned::to_owned),
            version: metadata.tool_version.map(ToOwned::to_owned),
        })
    } else {
        None
    };

    Ok(AgentTrace {
        version: default_agent_trace_version(),
        id: generate_agent_trace_id(commit_time)?,
        timestamp,
        vcs: metadata.vcs_type.map(|vcs_type| AgentTraceVcs {
            r#type: vcs_type,
            revision: metadata.commit_revision.to_owned(),
        }),
        tool,
        metadata: default_agent_trace_metadata(),
        files,
    })
}

#[cfg(test)]
mod tests;
