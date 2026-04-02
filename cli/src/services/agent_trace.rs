#![allow(dead_code)]

use std::collections::BTreeMap;

pub const TRACE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const VCS_TYPE_GIT: &str = "git";
pub const NOTES_REF: &str = "refs/notes/agent-trace";
pub const TRACE_CONTENT_TYPE: &str = "application/vnd.agent-trace.record+json";

pub const METADATA_QUALITY_STATUS: &str = "dev.crocoder.sce.quality_status";
pub const METADATA_REWRITE_FROM: &str = "dev.crocoder.sce.rewrite_from";
pub const METADATA_REWRITE_METHOD: &str = "dev.crocoder.sce.rewrite_method";
pub const METADATA_REWRITE_CONFIDENCE: &str = "dev.crocoder.sce.rewrite_confidence";
pub const METADATA_IDEMPOTENCY_KEY: &str = "dev.crocoder.sce.idempotency_key";
pub const METADATA_NOTES_REF: &str = "dev.crocoder.sce.notes_ref";
pub const METADATA_CONTENT_TYPE: &str = "dev.crocoder.sce.content_type";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceAdapterInput {
    pub record_id: String,
    pub timestamp_rfc3339: String,
    pub commit_sha: String,
    pub files: Vec<FileAttributionInput>,
    pub quality_status: QualityStatus,
    pub rewrite: Option<RewriteInfo>,
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileAttributionInput {
    pub path: String,
    pub conversations: Vec<ConversationInput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationInput {
    pub url: String,
    pub related: Vec<String>,
    pub ranges: Vec<RangeInput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RangeInput {
    pub start_line: u32,
    pub end_line: u32,
    pub contributor: ContributorInput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContributorInput {
    pub kind: ContributorType,
    pub model_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewriteInfo {
    pub from_sha: String,
    pub method: String,
    pub confidence: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QualityStatus {
    Final,
    Partial,
    NeedsReview,
}

impl QualityStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Final => "final",
            Self::Partial => "partial",
            Self::NeedsReview => "needs_review",
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContributorType {
    Human,
    Ai,
    Mixed,
    Unknown,
}

impl ContributorType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Ai => "ai",
            Self::Mixed => "mixed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentTraceRecord {
    pub version: String,
    pub id: String,
    pub timestamp: String,
    pub vcs: AgentTraceVcs,
    pub files: Vec<AgentTraceFile>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentTraceVcs {
    pub r#type: String,
    pub revision: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentTraceFile {
    pub path: String,
    pub conversations: Vec<AgentTraceConversation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentTraceConversation {
    pub url: String,
    pub related: Vec<String>,
    pub ranges: Vec<AgentTraceRange>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentTraceRange {
    pub start_line: u32,
    pub end_line: u32,
    pub contributor: AgentTraceContributor,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentTraceContributor {
    pub r#type: String,
    pub model_id: Option<String>,
}

pub fn build_trace_payload(input: TraceAdapterInput) -> AgentTraceRecord {
    let mut record = adapt_trace_payload(input);
    normalize_record_model_ids(&mut record);
    record
}

fn normalize_record_model_ids(record: &mut AgentTraceRecord) {
    for file in &mut record.files {
        for conversation in &mut file.conversations {
            for range in &mut conversation.ranges {
                if range.contributor.r#type == "ai" {
                    range.contributor.model_id =
                        normalize_model_id(range.contributor.model_id.take());
                }
            }
        }
    }
}

fn normalize_model_id(model_id: Option<String>) -> Option<String> {
    let raw = model_id?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let canonical = trimmed
        .replace(':', "/")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");

    if canonical.is_empty() {
        return None;
    }

    let mut segments = canonical.split('/');
    let provider = segments.next();
    let model = segments.next();
    let has_more = segments.next().is_some();
    if !has_more {
        if let (Some(provider), Some(model)) = (provider, model) {
            if !provider.is_empty() && !model.is_empty() {
                return Some(format!(
                    "{}/{}",
                    provider.to_ascii_lowercase(),
                    model.to_ascii_lowercase()
                ));
            }
        }
    }

    Some(canonical)
}

pub fn adapt_trace_payload(input: TraceAdapterInput) -> AgentTraceRecord {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        METADATA_QUALITY_STATUS.to_string(),
        input.quality_status.as_str().to_string(),
    );
    metadata.insert(METADATA_NOTES_REF.to_string(), NOTES_REF.to_string());
    metadata.insert(
        METADATA_CONTENT_TYPE.to_string(),
        TRACE_CONTENT_TYPE.to_string(),
    );

    if let Some(rewrite) = input.rewrite {
        metadata.insert(METADATA_REWRITE_FROM.to_string(), rewrite.from_sha);
        metadata.insert(METADATA_REWRITE_METHOD.to_string(), rewrite.method);
        metadata.insert(METADATA_REWRITE_CONFIDENCE.to_string(), rewrite.confidence);
    }

    if let Some(idempotency_key) = input.idempotency_key {
        metadata.insert(METADATA_IDEMPOTENCY_KEY.to_string(), idempotency_key);
    }

    let files = input
        .files
        .into_iter()
        .map(|file| AgentTraceFile {
            path: file.path,
            conversations: file
                .conversations
                .into_iter()
                .map(|conversation| AgentTraceConversation {
                    url: conversation.url,
                    related: conversation.related,
                    ranges: conversation
                        .ranges
                        .into_iter()
                        .map(|range| AgentTraceRange {
                            start_line: range.start_line,
                            end_line: range.end_line,
                            contributor: AgentTraceContributor {
                                r#type: range.contributor.kind.as_str().to_string(),
                                model_id: range.contributor.model_id,
                            },
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect();

    AgentTraceRecord {
        version: TRACE_VERSION.to_string(),
        id: input.record_id,
        timestamp: input.timestamp_rfc3339,
        vcs: AgentTraceVcs {
            r#type: VCS_TYPE_GIT.to_string(),
            revision: input.commit_sha,
        },
        files,
        metadata,
    }
}
