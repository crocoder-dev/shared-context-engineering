use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    error::Error,
    fmt,
    io::Cursor,
    path::Path,
    sync::OnceLock,
};

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset};
use jsonschema::{validator_for, Validator};
use murmur3::murmur3_32;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::{NoContext, Timestamp, Uuid};

use super::mutation_trace::attribution::{logical_path, PatchLineLocation, PatchLineProvenance};
use super::patch::{
    parse_patch, FileChangeKind, ParsedPatch, PatchFileChange, PatchHunk, TouchedLineKind,
};
use super::version::PACKAGE_VERSION;

mod evidence;
mod repository_path;

pub use evidence::DirectEvidenceSource;
use evidence::{
    accepted_direct_tool, direct_line_evidence, AcceptedDirectLine, DirectLineEvidence,
};
pub(crate) use evidence::{direct_evidence_patch, has_direct_evidence};
pub use repository_path::RepositoryRoot;

pub const AGENT_TRACE_VERSION: &str = "0.1.0";
pub(crate) const SCE_WEB_BASE_URL: &str = "https://sce.crocoder.dev";

pub(crate) fn sce_config_schema_url() -> String {
    format!("{SCE_WEB_BASE_URL}/v{PACKAGE_VERSION}/config.json")
}

const RANGE_CONTENT_HASH_PREFIX: &str = "murmur3:";
const RANGE_CONTENT_HASH_INPUT_VERSION: &[u8] = b"sce-agent-trace-range-content-hash-v1\0";
const TOUCHED_LINE_ADDED_TAG: &[u8] = b"added\0";
const TOUCHED_LINE_REMOVED_TAG: &[u8] = b"removed\0";

pub(crate) fn agent_trace_conversation_url(agent_trace_id: &str) -> String {
    format!("{SCE_WEB_BASE_URL}/conversations/{agent_trace_id}")
}

pub(crate) fn agent_trace_persisted_url(agent_trace_id: &str) -> String {
    format!("{}/trace/{agent_trace_id}", sce_web_host())
}

pub(crate) fn agent_trace_session_url(session_id: &str) -> String {
    format!("{SCE_WEB_BASE_URL}/sessions/{session_id}")
}

fn sce_web_host() -> &'static str {
    SCE_WEB_BASE_URL
        .strip_prefix("https://")
        .unwrap_or(SCE_WEB_BASE_URL)
}

fn default_agent_trace_version() -> String {
    AGENT_TRACE_VERSION.to_owned()
}

fn default_agent_trace_metadata() -> AgentTraceMetadata {
    AgentTraceMetadata {
        sce: AgentTraceSceMetadata {
            version: PACKAGE_VERSION.to_owned(),
            line_changes: LineChangeAttribution::default(),
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentTraceEvidence<'a> {
    pub repository_root: &'a RepositoryRoot,
    pub direct_sources: &'a [DirectEvidenceSource<'a>],
    pub mutation_ai_patch: &'a ParsedPatch,
    pub mutation_non_ai_patch: &'a ParsedPatch,
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
    #[serde(default)]
    pub line_changes: LineChangeAttribution,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct LineChangeCounts {
    pub added: u64,
    pub removed: u64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct LineChangeAttribution {
    #[serde(default)]
    pub ai: LineChangeCounts,
    #[serde(default)]
    pub non_exclusive: LineChangeCounts,
    #[serde(default)]
    pub unknown: LineChangeCounts,
}

fn line_change_attribution(
    patch: &ParsedPatch,
    decisions: &BTreeMap<PatchLineLocation, LineAttribution>,
) -> LineChangeAttribution {
    let mut counts = LineChangeAttribution::default();
    for (file_index, file) in patch.files.iter().enumerate() {
        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            for (line_index, line) in hunk.lines.iter().enumerate() {
                let location = PatchLineLocation {
                    file_index,
                    hunk_index,
                    line_index,
                };
                let bucket = match decisions.get(&location) {
                    Some(LineAttribution::Ai(_)) => &mut counts.ai,
                    Some(LineAttribution::NonExclusive) => &mut counts.non_exclusive,
                    Some(LineAttribution::Unknown) | None => &mut counts.unknown,
                };
                match line.kind {
                    TouchedLineKind::Added => bucket.added += 1,
                    TouchedLineKind::Removed => bucket.removed += 1,
                }
            }
        }
    }
    counts
}

fn parse_commit_timestamp(commit_timestamp: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(commit_timestamp).with_context(|| {
        format!("Invalid commit timestamp '{commit_timestamp}': expected RFC 3339 date-time.")
    })
}

const AGENT_TRACE_SCHEMA_PATH: &str = "config/schema/agent-trace.schema.json";
pub(crate) const AGENT_TRACE_SCHEMA_JSON: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/static/schema/agent-trace.schema.json"
));

static AGENT_TRACE_SCHEMA_VALIDATOR: OnceLock<Validator> = OnceLock::new();

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum AgentTraceValidationError {
    SchemaValidation { errors: Vec<String> },
}

impl fmt::Display for AgentTraceValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SchemaValidation { errors } => write!(
                f,
                "Agent Trace JSON failed schema validation against embedded schema '{AGENT_TRACE_SCHEMA_PATH}': {}",
                errors.join(" | ")
            ),
        }
    }
}

impl Error for AgentTraceValidationError {}

fn agent_trace_schema_validator() -> &'static Validator {
    AGENT_TRACE_SCHEMA_VALIDATOR.get_or_init(|| {
        let schema: Value = serde_json::from_str(AGENT_TRACE_SCHEMA_JSON)
            .expect("agent trace schema JSON should parse");
        validator_for(&schema).expect("agent trace schema JSON should compile")
    })
}

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
    /// Absolute lookup URL for the generated top-level agent trace.
    pub url: String,
    /// Classification of this hunk's origin.
    pub contributor: Contributor,
    /// Line ranges in the new file, derived from the `post_commit_patch` hunk metadata.
    pub ranges: Vec<LineRange>,
    /// Optional related resources for this conversation entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related: Option<Vec<ConversationRelated>>,
}

/// A related resource for a conversation entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ConversationRelated {
    /// Free-form related resource type.
    #[serde(rename = "type")]
    pub kind: String,
    /// Related resource URL.
    pub url: String,
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
    pub content_hash: String,
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

type MutationLineKey<'a> = (&'a str, TouchedLineKind, u64, &'a str);

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResolvedLine {
    provenance: PatchLineProvenance,
    direct_sources: BTreeSet<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum LineAttribution {
    Ai(ResolvedLine),
    NonExclusive,
    Unknown,
}

impl LineAttribution {
    fn accepted_ai(&self) -> Option<&ResolvedLine> {
        match self {
            Self::Ai(line) => Some(line),
            Self::NonExclusive | Self::Unknown => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum MutationLineEvidence {
    Ai(PatchLineProvenance),
    NonAi,
    Unresolved,
}

struct MutationEvidenceIndex<'a> {
    ai: HashMap<MutationLineKey<'a>, PatchLineProvenance>,
    non_ai: HashSet<MutationLineKey<'a>>,
}

impl<'a> MutationEvidenceIndex<'a> {
    fn new(mutation_ai_patch: &'a ParsedPatch, mutation_non_ai_patch: &'a ParsedPatch) -> Self {
        let mut ai = HashMap::new();
        for file in &mutation_ai_patch.files {
            for hunk in &file.hunks {
                for line in &hunk.lines {
                    ai.insert(
                        (
                            logical_path(file),
                            line.kind,
                            line.line_number,
                            line.content.as_str(),
                        ),
                        PatchLineProvenance {
                            session_id: line.session_id.clone(),
                            model_id: hunk.model_id.clone(),
                        },
                    );
                }
            }
        }
        let non_ai = mutation_non_ai_patch
            .files
            .iter()
            .flat_map(|file| {
                file.hunks.iter().flat_map(move |hunk| {
                    hunk.lines.iter().map(move |line| {
                        (
                            logical_path(file),
                            line.kind,
                            line.line_number,
                            line.content.as_str(),
                        )
                    })
                })
            })
            .collect();
        MutationEvidenceIndex { ai, non_ai }
    }

    fn lookup(&self, key: &MutationLineKey<'_>) -> MutationLineEvidence {
        if let Some(provenance) = self.ai.get(key) {
            MutationLineEvidence::Ai(provenance.clone())
        } else if self.non_ai.contains(key) {
            MutationLineEvidence::NonAi
        } else {
            MutationLineEvidence::Unresolved
        }
    }
}

fn resolve_line(
    mutation: MutationLineEvidence,
    direct: Option<&DirectLineEvidence>,
) -> LineAttribution {
    match (mutation, direct) {
        (
            MutationLineEvidence::Ai(mutation),
            Some(DirectLineEvidence::Accepted(AcceptedDirectLine {
                provenance: direct,
                sources,
            })),
        ) if direct.session_id == mutation.session_id => LineAttribution::Ai(ResolvedLine {
            provenance: PatchLineProvenance {
                session_id: mutation.session_id,
                model_id: mutation.model_id.or_else(|| direct.model_id.clone()),
            },
            direct_sources: sources.clone(),
        }),
        (MutationLineEvidence::Ai(mutation), _) => LineAttribution::Ai(ResolvedLine {
            provenance: mutation,
            direct_sources: BTreeSet::new(),
        }),
        (MutationLineEvidence::NonAi, _) => LineAttribution::NonExclusive,
        (MutationLineEvidence::Unresolved, Some(DirectLineEvidence::Accepted(accepted))) => {
            LineAttribution::Ai(ResolvedLine {
                provenance: accepted.provenance.clone(),
                direct_sources: accepted.sources.clone(),
            })
        }
        (MutationLineEvidence::Unresolved, Some(DirectLineEvidence::Conflicting) | None) => {
            LineAttribution::Unknown
        }
    }
}

fn resolve_patch_lines(
    target: &ParsedPatch,
    direct_sources: &[DirectEvidenceSource<'_>],
    repository_root: &RepositoryRoot,
    mutation: &MutationEvidenceIndex<'_>,
) -> BTreeMap<PatchLineLocation, LineAttribution> {
    let direct = direct_line_evidence(direct_sources, target, repository_root);
    let mut resolved = BTreeMap::new();
    for (file_index, file) in target.files.iter().enumerate() {
        let path = logical_path(file);
        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            for (line_index, line) in hunk.lines.iter().enumerate() {
                let location = PatchLineLocation {
                    file_index,
                    hunk_index,
                    line_index,
                };
                let key = (path, line.kind, line.line_number, line.content.as_str());
                resolved.insert(
                    location,
                    resolve_line(mutation.lookup(&key), direct.get(&location)),
                );
            }
        }
    }
    resolved
}

fn hunk_resolved_lines(
    resolved: &BTreeMap<PatchLineLocation, LineAttribution>,
    file_index: usize,
    hunk_index: usize,
) -> impl Iterator<Item = &ResolvedLine> {
    resolved
        .range(
            PatchLineLocation {
                file_index,
                hunk_index,
                line_index: 0,
            }..=PatchLineLocation {
                file_index,
                hunk_index,
                line_index: usize::MAX,
            },
        )
        .filter_map(|(_, line)| line.accepted_ai())
}

fn classify_hunk(hunk: &PatchHunk, covered: usize) -> HunkContributor {
    let total = hunk.lines.len();
    if total > 0 && covered == total {
        HunkContributor::Ai
    } else if covered > 0 {
        HunkContributor::Mixed
    } else {
        HunkContributor::Unknown
    }
}

fn agreed_model_id<'r>(lines: impl Iterator<Item = &'r ResolvedLine>) -> Option<String> {
    let mut model: Option<&str> = None;
    for line in lines {
        let candidate = line.provenance.model_id.as_deref()?;
        match model {
            None => model = Some(candidate),
            Some(existing) if existing == candidate => {}
            Some(_) => return None,
        }
    }
    model.map(ToOwned::to_owned)
}

pub(crate) fn patch_has_touched_lines(patch: &ParsedPatch) -> bool {
    patch
        .files
        .iter()
        .any(|file| file.hunks.iter().any(|hunk| !hunk.lines.is_empty()))
}

pub(crate) fn range_content_hash(hunk: &PatchHunk) -> String {
    let mut input = Vec::new();
    input.extend_from_slice(RANGE_CONTENT_HASH_INPUT_VERSION);

    for line in &hunk.lines {
        let kind_tag = match line.kind {
            TouchedLineKind::Added => TOUCHED_LINE_ADDED_TAG,
            TouchedLineKind::Removed => TOUCHED_LINE_REMOVED_TAG,
        };
        let content = line.content.as_bytes();

        input.extend_from_slice(kind_tag);
        input.extend_from_slice(&(content.len() as u64).to_be_bytes());
        input.extend_from_slice(content);
        input.push(0);
    }

    let hash = murmur3_32(&mut Cursor::new(input), 0)
        .expect("murmur3 hashing from in-memory cursor should not fail");
    format!("{RANGE_CONTENT_HASH_PREFIX}{hash:08x}")
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
        content_hash: range_content_hash(hunk),
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

    let parsed_patch = parse_patch(&embedded_patch, None).ok()?;
    (!parsed_patch.files.is_empty()).then_some(parsed_patch)
}

fn build_trace_file(
    post_commit_file: &PatchFileChange,
    file_index: usize,
    resolved: &BTreeMap<PatchLineLocation, LineAttribution>,
    conversation_url: &str,
) -> Option<TraceFile> {
    if post_commit_file.hunks.is_empty() {
        return None;
    }

    let conversations = post_commit_file
        .hunks
        .iter()
        .enumerate()
        .map(|(hunk_index, post_commit_hunk)| {
            let covered = hunk_resolved_lines(resolved, file_index, hunk_index).count();
            let contributor_kind = classify_hunk(post_commit_hunk, covered);
            let contributor_model_id = match contributor_kind {
                HunkContributor::Ai | HunkContributor::Mixed => {
                    agreed_model_id(hunk_resolved_lines(resolved, file_index, hunk_index))
                }
                HunkContributor::Unknown => None,
            };
            let related_session_ids = hunk_resolved_lines(resolved, file_index, hunk_index)
                .filter_map(|line| line.provenance.session_id.as_deref())
                .filter(|session_id| !session_id.is_empty())
                .collect::<BTreeSet<_>>();
            let related = (!related_session_ids.is_empty()).then(|| {
                related_session_ids
                    .into_iter()
                    .map(|session_id| ConversationRelated {
                        kind: String::from("session"),
                        url: agent_trace_session_url(session_id),
                    })
                    .collect()
            });

            Conversation {
                url: conversation_url.to_owned(),
                contributor: Contributor {
                    kind: contributor_kind,
                    model_id: contributor_model_id,
                },
                ranges: vec![line_range_from_hunk(post_commit_file, post_commit_hunk)],
                related,
            }
        })
        .collect();

    Some(TraceFile {
        path: trace_path(post_commit_file).to_string(),
        conversations,
    })
}

pub fn build_agent_trace_from_evidence(
    evidence: AgentTraceEvidence<'_>,
    post_commit_patch: &ParsedPatch,
    metadata: AgentTraceMetadataInput<'_>,
) -> Result<AgentTrace> {
    let AgentTraceEvidence {
        repository_root,
        direct_sources,
        mutation_ai_patch,
        mutation_non_ai_patch,
    } = evidence;

    let commit_time = parse_commit_timestamp(metadata.commit_timestamp)?;
    let id = generate_agent_trace_id(commit_time)?;
    let conversation_url = agent_trace_conversation_url(&id);
    let timestamp = metadata.commit_timestamp.to_owned();
    let mutation = MutationEvidenceIndex::new(mutation_ai_patch, mutation_non_ai_patch);
    let resolved = resolve_patch_lines(
        post_commit_patch,
        direct_sources,
        repository_root,
        &mutation,
    );
    let line_changes = line_change_attribution(post_commit_patch, &resolved);
    let empty_patch = ParsedPatch { files: Vec::new() };
    let no_mutation = MutationEvidenceIndex::new(&empty_patch, &empty_patch);

    let mut files = Vec::new();

    for (file_index, post_commit_file) in post_commit_patch.files.iter().enumerate() {
        if let Some(embedded_patch) = parse_embedded_deleted_patch(post_commit_file) {
            let embedded_resolved = resolve_patch_lines(
                &embedded_patch,
                direct_sources,
                repository_root,
                &no_mutation,
            );
            files.extend(embedded_patch.files.iter().enumerate().filter_map(
                |(embedded_index, embedded_file)| {
                    build_trace_file(
                        embedded_file,
                        embedded_index,
                        &embedded_resolved,
                        &conversation_url,
                    )
                },
            ));
            continue;
        }

        if let Some(trace_file) =
            build_trace_file(post_commit_file, file_index, &resolved, &conversation_url)
        {
            files.push(trace_file);
        }
    }

    let accepted_sources: BTreeSet<usize> = resolved
        .values()
        .filter_map(LineAttribution::accepted_ai)
        .flat_map(|line| line.direct_sources.iter().copied())
        .collect();
    let tool = accepted_direct_tool(direct_sources, &accepted_sources);

    Ok(AgentTrace {
        version: default_agent_trace_version(),
        id,
        timestamp,
        vcs: metadata.vcs_type.map(|vcs_type| AgentTraceVcs {
            r#type: vcs_type,
            revision: metadata.commit_revision.to_owned(),
        }),
        tool,
        metadata: AgentTraceMetadata {
            sce: AgentTraceSceMetadata {
                version: PACKAGE_VERSION.to_owned(),
                line_changes,
            },
        },
        files,
    })
}

#[cfg(test)]
mod tests;
