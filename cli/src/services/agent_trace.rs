#![allow(dead_code)]

use std::collections::BTreeMap;

pub const TRACE_VERSION: &str = "0.1.0";
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

#[cfg(test)]
mod tests {
    use super::{
        adapt_trace_payload, ContributorInput, ContributorType, ConversationInput,
        FileAttributionInput, QualityStatus, RangeInput, RewriteInfo, TraceAdapterInput,
        METADATA_CONTENT_TYPE, METADATA_IDEMPOTENCY_KEY, METADATA_NOTES_REF,
        METADATA_QUALITY_STATUS, METADATA_REWRITE_CONFIDENCE, METADATA_REWRITE_FROM,
        METADATA_REWRITE_METHOD,
    };

    #[test]
    fn adapter_maps_required_fields_and_vcs_contract() {
        let record = adapt_trace_payload(TraceAdapterInput {
            record_id: "f8cabb2a-18e4-4e52-a6df-cf5bf8c0fbe7".to_string(),
            timestamp_rfc3339: "2026-03-04T10:11:12Z".to_string(),
            commit_sha: "abc123def456".to_string(),
            files: vec![FileAttributionInput {
                path: "cli/src/services/agent_trace.rs".to_string(),
                conversations: vec![ConversationInput {
                    url: "https://example.test/conversation/123".to_string(),
                    related: vec![],
                    ranges: vec![RangeInput {
                        start_line: 1,
                        end_line: 3,
                        contributor: ContributorInput {
                            kind: ContributorType::Human,
                            model_id: None,
                        },
                    }],
                }],
            }],
            quality_status: QualityStatus::Final,
            rewrite: None,
            idempotency_key: None,
        });

        assert_eq!(record.version, "0.1.0");
        assert_eq!(record.id, "f8cabb2a-18e4-4e52-a6df-cf5bf8c0fbe7");
        assert_eq!(record.timestamp, "2026-03-04T10:11:12Z");
        assert_eq!(record.vcs.r#type, "git");
        assert_eq!(record.vcs.revision, "abc123def456");
        assert_eq!(record.files.len(), 1);
    }

    #[test]
    fn adapter_places_extension_metadata_in_reserved_reverse_domain_keys() {
        let record = adapt_trace_payload(TraceAdapterInput {
            record_id: "f8cabb2a-18e4-4e52-a6df-cf5bf8c0fbe7".to_string(),
            timestamp_rfc3339: "2026-03-04T10:11:12Z".to_string(),
            commit_sha: "abc123def456".to_string(),
            files: vec![FileAttributionInput {
                path: "README.md".to_string(),
                conversations: vec![],
            }],
            quality_status: QualityStatus::Partial,
            rewrite: Some(RewriteInfo {
                from_sha: "oldsha".to_string(),
                method: "rebase".to_string(),
                confidence: "0.91".to_string(),
            }),
            idempotency_key: Some("repo:oldsha:newsha".to_string()),
        });

        assert_eq!(
            record.metadata.get(METADATA_QUALITY_STATUS),
            Some(&"partial".to_string())
        );
        assert_eq!(
            record.metadata.get(METADATA_NOTES_REF),
            Some(&"refs/notes/agent-trace".to_string())
        );
        assert_eq!(
            record.metadata.get(METADATA_CONTENT_TYPE),
            Some(&"application/vnd.agent-trace.record+json".to_string())
        );
        assert_eq!(
            record.metadata.get(METADATA_REWRITE_FROM),
            Some(&"oldsha".to_string())
        );
        assert_eq!(
            record.metadata.get(METADATA_REWRITE_METHOD),
            Some(&"rebase".to_string())
        );
        assert_eq!(
            record.metadata.get(METADATA_REWRITE_CONFIDENCE),
            Some(&"0.91".to_string())
        );
        assert_eq!(
            record.metadata.get(METADATA_IDEMPOTENCY_KEY),
            Some(&"repo:oldsha:newsha".to_string())
        );
    }

    #[test]
    fn adapter_maps_contributor_types_and_optional_model_ids() {
        let record = adapt_trace_payload(TraceAdapterInput {
            record_id: "f8cabb2a-18e4-4e52-a6df-cf5bf8c0fbe7".to_string(),
            timestamp_rfc3339: "2026-03-04T10:11:12Z".to_string(),
            commit_sha: "abc123def456".to_string(),
            files: vec![FileAttributionInput {
                path: "src/lib.rs".to_string(),
                conversations: vec![ConversationInput {
                    url: "https://example.test/c/1".to_string(),
                    related: vec!["https://example.test/c/2".to_string()],
                    ranges: vec![
                        RangeInput {
                            start_line: 4,
                            end_line: 9,
                            contributor: ContributorInput {
                                kind: ContributorType::Ai,
                                model_id: Some("openai/gpt-5.3-codex".to_string()),
                            },
                        },
                        RangeInput {
                            start_line: 10,
                            end_line: 10,
                            contributor: ContributorInput {
                                kind: ContributorType::Mixed,
                                model_id: None,
                            },
                        },
                        RangeInput {
                            start_line: 11,
                            end_line: 12,
                            contributor: ContributorInput {
                                kind: ContributorType::Unknown,
                                model_id: None,
                            },
                        },
                    ],
                }],
            }],
            quality_status: QualityStatus::NeedsReview,
            rewrite: None,
            idempotency_key: None,
        });

        let ranges = &record.files[0].conversations[0].ranges;
        assert_eq!(ranges[0].contributor.r#type, "ai");
        assert_eq!(
            ranges[0].contributor.model_id,
            Some("openai/gpt-5.3-codex".to_string())
        );
        assert_eq!(ranges[1].contributor.r#type, "mixed");
        assert_eq!(ranges[1].contributor.model_id, None);
        assert_eq!(ranges[2].contributor.r#type, "unknown");
        assert_eq!(ranges[2].contributor.model_id, None);
        assert_eq!(
            record.files[0].conversations[0].related,
            vec!["https://example.test/c/2".to_string()]
        );
    }
}
