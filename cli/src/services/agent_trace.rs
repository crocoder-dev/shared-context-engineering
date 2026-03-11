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

#[cfg(test)]
mod tests {
    use jsonschema::draft202012;
    use serde_json::{Map, Value};

    use super::{
        adapt_trace_payload, build_trace_payload, ContributorInput, ContributorType,
        ConversationInput, FileAttributionInput, QualityStatus, RangeInput, RewriteInfo,
        TraceAdapterInput, METADATA_CONTENT_TYPE, METADATA_IDEMPOTENCY_KEY, METADATA_NOTES_REF,
        METADATA_QUALITY_STATUS, METADATA_REWRITE_CONFIDENCE, METADATA_REWRITE_FROM,
        METADATA_REWRITE_METHOD,
    };

    const AGENT_TRACE_SCHEMA: &str = r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://agent-trace.dev/schemas/v1/trace-record.json",
  "title": "Agent Trace Record",
  "type": "object",
  "required": ["version", "id", "timestamp", "files"],
  "properties": {
    "version": {
      "type": "string",
      "pattern": "^[0-9]+\\.[0-9]+$"
    },
    "id": {
      "type": "string",
      "format": "uuid"
    },
    "timestamp": {
      "type": "string",
      "format": "date-time"
    },
    "vcs": {
      "$ref": "#/$defs/vcs"
    },
    "tool": {
      "$ref": "#/$defs/tool"
    },
    "files": {
      "type": "array",
      "items": {
        "$ref": "#/$defs/file"
      }
    },
    "metadata": {
      "type": "object"
    }
  },
  "$defs": {
    "vcs": {
      "type": "object",
      "required": ["type", "revision"],
      "properties": {
        "type": {
          "type": "string",
          "enum": ["git", "jj", "hg", "svn"]
        },
        "revision": {
          "type": "string"
        }
      }
    },
    "tool": {
      "type": "object",
      "properties": {
        "name": { "type": "string" },
        "version": { "type": "string" }
      }
    },
    "file": {
      "type": "object",
      "required": ["path", "conversations"],
      "properties": {
        "path": {
          "type": "string"
        },
        "conversations": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/conversation"
          }
        }
      }
    },
    "contributor": {
      "type": "object",
      "required": ["type"],
      "properties": {
        "type": {
          "type": "string",
          "enum": ["human", "ai", "mixed", "unknown"]
        },
        "model_id": {
          "type": "string",
          "maxLength": 250
        }
      }
    },
    "conversation": {
      "type": "object",
      "required": ["ranges"],
      "properties": {
        "url": {
          "type": "string",
          "format": "uri"
        },
        "contributor": {
          "$ref": "#/$defs/contributor"
        },
        "ranges": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/range"
          }
        },
        "related": {
          "type": "array",
          "items": {
            "type": "object",
            "required": ["type", "url"],
            "properties": {
              "type": { "type": "string" },
              "url": { "type": "string", "format": "uri" }
            }
          }
        }
      }
    },
    "range": {
      "type": "object",
      "required": ["start_line", "end_line"],
      "properties": {
        "start_line": { "type": "integer", "minimum": 1 },
        "end_line": { "type": "integer", "minimum": 1 },
        "content_hash": {
          "type": "string"
        },
        "contributor": {
          "$ref": "#/$defs/contributor"
        }
      }
    }
  }
}"##;

    fn range_to_json_value(range: &super::AgentTraceRange) -> Value {
        let mut range_obj = Map::new();
        range_obj.insert(
            "start_line".to_string(),
            Value::Number(range.start_line.into()),
        );
        range_obj.insert("end_line".to_string(), Value::Number(range.end_line.into()));

        let mut contributor_obj = Map::new();
        contributor_obj.insert(
            "type".to_string(),
            Value::String(range.contributor.r#type.clone()),
        );
        if let Some(model_id) = &range.contributor.model_id {
            contributor_obj.insert("model_id".to_string(), Value::String(model_id.clone()));
        }
        range_obj.insert("contributor".to_string(), Value::Object(contributor_obj));

        Value::Object(range_obj)
    }

    fn related_to_json_value(url: &str) -> Value {
        let mut related_obj = Map::new();
        related_obj.insert("type".to_string(), Value::String("related".to_string()));
        related_obj.insert("url".to_string(), Value::String(url.to_string()));
        Value::Object(related_obj)
    }

    fn conversation_to_json_value(conversation: &super::AgentTraceConversation) -> Value {
        let mut conv_obj = Map::new();
        conv_obj.insert("url".to_string(), Value::String(conversation.url.clone()));

        let ranges = conversation
            .ranges
            .iter()
            .map(range_to_json_value)
            .collect::<Vec<_>>();
        conv_obj.insert("ranges".to_string(), Value::Array(ranges));

        if !conversation.related.is_empty() {
            conv_obj.insert(
                "related".to_string(),
                Value::Array(
                    conversation
                        .related
                        .iter()
                        .map(|url| related_to_json_value(url))
                        .collect::<Vec<_>>(),
                ),
            );
        }

        Value::Object(conv_obj)
    }

    fn file_to_json_value(file: &super::AgentTraceFile) -> Value {
        let mut file_obj = Map::new();
        file_obj.insert("path".to_string(), Value::String(file.path.clone()));
        file_obj.insert(
            "conversations".to_string(),
            Value::Array(
                file.conversations
                    .iter()
                    .map(conversation_to_json_value)
                    .collect::<Vec<_>>(),
            ),
        );

        Value::Object(file_obj)
    }

    fn record_to_json_value(record: &super::AgentTraceRecord) -> Value {
        let mut root = Map::new();
        root.insert("version".to_string(), Value::String(record.version.clone()));
        root.insert("id".to_string(), Value::String(record.id.clone()));
        root.insert(
            "timestamp".to_string(),
            Value::String(record.timestamp.clone()),
        );

        let mut vcs = Map::new();
        vcs.insert("type".to_string(), Value::String(record.vcs.r#type.clone()));
        vcs.insert(
            "revision".to_string(),
            Value::String(record.vcs.revision.clone()),
        );
        root.insert("vcs".to_string(), Value::Object(vcs));

        let files = record
            .files
            .iter()
            .map(file_to_json_value)
            .collect::<Vec<_>>();
        root.insert("files".to_string(), Value::Array(files));

        if !record.metadata.is_empty() {
            let metadata = record
                .metadata
                .iter()
                .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                .collect::<Map<_, _>>();
            root.insert("metadata".to_string(), Value::Object(metadata));
        }

        Value::Object(root)
    }

    fn patched_agent_trace_schema() -> Value {
        let mut schema: Value =
            serde_json::from_str(AGENT_TRACE_SCHEMA).expect("published schema JSON should parse");
        if let Some(version_pattern) = schema.pointer_mut("/properties/version/pattern") {
            *version_pattern = Value::String("^[0-9]+\\.[0-9]+(?:\\.[0-9]+)?$".to_string());
        }
        schema
    }

    fn schema_validator() -> jsonschema::Validator {
        draft202012::options()
            .should_validate_formats(true)
            .build(&patched_agent_trace_schema())
            .expect("schema compilation should work")
    }

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

    #[test]
    fn builder_normalizes_ai_model_id_to_provider_model_when_possible() {
        let record = build_trace_payload(TraceAdapterInput {
            record_id: "f8cabb2a-18e4-4e52-a6df-cf5bf8c0fbe7".to_string(),
            timestamp_rfc3339: "2026-03-04T10:11:12Z".to_string(),
            commit_sha: "abc123def456".to_string(),
            files: vec![FileAttributionInput {
                path: "src/lib.rs".to_string(),
                conversations: vec![ConversationInput {
                    url: "https://example.test/c/1".to_string(),
                    related: vec![],
                    ranges: vec![RangeInput {
                        start_line: 1,
                        end_line: 3,
                        contributor: ContributorInput {
                            kind: ContributorType::Ai,
                            model_id: Some(" OpenAI:GPT-5.3-CODEX ".to_string()),
                        },
                    }],
                }],
            }],
            quality_status: QualityStatus::Final,
            rewrite: None,
            idempotency_key: None,
        });

        assert_eq!(
            record.files[0].conversations[0].ranges[0]
                .contributor
                .model_id,
            Some("openai/gpt-5.3-codex".to_string())
        );
    }

    #[test]
    fn builder_serialization_is_deterministic_for_identical_input() {
        let input = TraceAdapterInput {
            record_id: "f8cabb2a-18e4-4e52-a6df-cf5bf8c0fbe7".to_string(),
            timestamp_rfc3339: "2026-03-04T10:11:12Z".to_string(),
            commit_sha: "abc123def456".to_string(),
            files: vec![FileAttributionInput {
                path: "src/lib.rs".to_string(),
                conversations: vec![ConversationInput {
                    url: "https://example.test/c/1".to_string(),
                    related: vec!["https://example.test/c/2".to_string()],
                    ranges: vec![RangeInput {
                        start_line: 1,
                        end_line: 2,
                        contributor: ContributorInput {
                            kind: ContributorType::Ai,
                            model_id: Some("openai/gpt-5.3-codex".to_string()),
                        },
                    }],
                }],
            }],
            quality_status: QualityStatus::Final,
            rewrite: Some(RewriteInfo {
                from_sha: "oldsha".to_string(),
                method: "rebase".to_string(),
                confidence: "0.95".to_string(),
            }),
            idempotency_key: Some("repo:old:new".to_string()),
        };

        let first =
            serde_json::to_string(&record_to_json_value(&build_trace_payload(input.clone())))
                .expect("first JSON serialization should succeed");
        let second = serde_json::to_string(&record_to_json_value(&build_trace_payload(input)))
            .expect("second JSON serialization should succeed");

        assert_eq!(first, second);
    }

    #[test]
    fn builder_output_passes_agent_trace_schema_validation() {
        let record = build_trace_payload(TraceAdapterInput {
            record_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            timestamp_rfc3339: "2026-03-04T10:11:12Z".to_string(),
            commit_sha: "abc123def456".to_string(),
            files: vec![FileAttributionInput {
                path: "src/lib.rs".to_string(),
                conversations: vec![ConversationInput {
                    url: "https://example.test/conversation/1".to_string(),
                    related: vec!["https://example.test/session/1".to_string()],
                    ranges: vec![
                        RangeInput {
                            start_line: 1,
                            end_line: 5,
                            contributor: ContributorInput {
                                kind: ContributorType::Ai,
                                model_id: Some("openai/gpt-5.3-codex".to_string()),
                            },
                        },
                        RangeInput {
                            start_line: 6,
                            end_line: 8,
                            contributor: ContributorInput {
                                kind: ContributorType::Human,
                                model_id: None,
                            },
                        },
                    ],
                }],
            }],
            quality_status: QualityStatus::Final,
            rewrite: None,
            idempotency_key: None,
        });

        let payload = record_to_json_value(&record);
        let validator = schema_validator();
        let validation = validator.iter_errors(&payload).collect::<Vec<_>>();

        assert!(
            validation.is_empty(),
            "schema validation errors: {:?}",
            validation
                .into_iter()
                .map(|err| err.to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn builder_output_rejects_invalid_uri_and_timestamp_formats() {
        let invalid_payload = record_to_json_value(&build_trace_payload(TraceAdapterInput {
            record_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            timestamp_rfc3339: "not-a-timestamp".to_string(),
            commit_sha: "abc123def456".to_string(),
            files: vec![FileAttributionInput {
                path: "src/lib.rs".to_string(),
                conversations: vec![ConversationInput {
                    url: "not-a-uri".to_string(),
                    related: vec!["still-not-a-uri".to_string()],
                    ranges: vec![RangeInput {
                        start_line: 1,
                        end_line: 2,
                        contributor: ContributorInput {
                            kind: ContributorType::Ai,
                            model_id: Some("openai/gpt-5.3-codex".to_string()),
                        },
                    }],
                }],
            }],
            quality_status: QualityStatus::Final,
            rewrite: None,
            idempotency_key: None,
        }));

        let validator = schema_validator();
        let errors = validator
            .iter_errors(&invalid_payload)
            .map(|err| err.to_string())
            .collect::<Vec<_>>();

        assert!(!errors.is_empty());
        assert!(
            errors.iter().any(|err| err.contains("date-time")),
            "expected date-time format error, got: {errors:?}"
        );
        assert!(
            errors.iter().any(|err| err.contains("uri")),
            "expected uri format error, got: {errors:?}"
        );
    }
}
