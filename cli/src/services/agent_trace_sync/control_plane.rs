//! Wire-contract DTOs for the control-plane Agent Trace ingestion API.
//!
//! These types define the request/response shapes for
//! `POST /agent-trace/ingestion/state` and `POST /agent-trace/ingestion/batch`.
//! They perform no HTTP I/O and hold no cursor state themselves.

use serde::{Deserialize, Serialize};

/// One of the four independent Agent Trace capture streams, identified by its
/// literal wire value. These values are always the `snake_case` stream
/// identifiers (`messages`, `parts`, `diff_traces`, `agent_traces`), distinct
/// from the `camelCase` field names (`diffTraces`, `agentTraces`) used in
/// [`AgentTraceCursors`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestionStream {
    Messages,
    Parts,
    DiffTraces,
    AgentTraces,
}

/// Request body for `POST /agent-trace/ingestion/state`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceIngestionStateRequest {
    pub repository_id: String,
    pub source_instance_id: String,
}

/// Authoritative server-side cursor for each of the four capture streams, as
/// returned by `/state`. Each cursor is the last `source_row_id` the control
/// plane has accepted for that stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceCursors {
    pub messages: i64,
    pub parts: i64,
    pub diff_traces: i64,
    pub agent_traces: i64,
}

/// Response body for `POST /agent-trace/ingestion/state`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceIngestionStateResponse {
    pub cursors: AgentTraceCursors,
}

/// Request body for `POST /agent-trace/ingestion/batch`, generic over the
/// PR #198 export row type carried by the stream being uploaded.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceIngestionBatchRequest<T> {
    pub repository_id: String,
    pub source_instance_id: String,
    pub stream: IngestionStream,
    pub expected_cursor: i64,
    pub rows: Vec<T>,
}

/// Response body for `POST /agent-trace/ingestion/batch`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceIngestionBatchResponse {
    pub accepted: usize,
    pub cursor: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::agent_trace_db::MessageRole;
    use crate::services::agent_trace_export::{
        AgentTraceAgentTraceExportRow, AgentTraceDiffTraceExportRow, AgentTraceMessageExportRow,
        AgentTracePartExportRow,
    };
    use serde_json::json;

    #[test]
    fn ingestion_stream_serializes_to_exact_snake_case_wire_values() {
        assert_eq!(
            serde_json::to_value(IngestionStream::Messages).unwrap(),
            json!("messages")
        );
        assert_eq!(
            serde_json::to_value(IngestionStream::Parts).unwrap(),
            json!("parts")
        );
        assert_eq!(
            serde_json::to_value(IngestionStream::DiffTraces).unwrap(),
            json!("diff_traces")
        );
        assert_eq!(
            serde_json::to_value(IngestionStream::AgentTraces).unwrap(),
            json!("agent_traces")
        );
    }

    #[test]
    fn state_request_serializes_to_camel_case_fields() {
        let request = AgentTraceIngestionStateRequest {
            repository_id: "acme-monorepo".to_string(),
            source_instance_id: "src-123".to_string(),
        };

        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({
                "repositoryId": "acme-monorepo",
                "sourceInstanceId": "src-123",
            })
        );
    }

    #[test]
    fn state_response_deserializes_camel_case_cursor_fields() {
        let response: AgentTraceIngestionStateResponse = serde_json::from_value(json!({
            "cursors": {
                "messages": 10,
                "parts": 20,
                "diffTraces": 30,
                "agentTraces": 40,
            }
        }))
        .unwrap();

        assert_eq!(
            response.cursors,
            AgentTraceCursors {
                messages: 10,
                parts: 20,
                diff_traces: 30,
                agent_traces: 40,
            }
        );
    }

    #[test]
    fn batch_response_deserializes_accepted_and_cursor() {
        let response: AgentTraceIngestionBatchResponse = serde_json::from_value(json!({
            "accepted": 3,
            "cursor": 13,
        }))
        .unwrap();

        assert_eq!(
            response,
            AgentTraceIngestionBatchResponse {
                accepted: 3,
                cursor: 13,
            }
        );
    }

    #[test]
    fn batch_request_composes_with_message_export_rows() {
        let request = AgentTraceIngestionBatchRequest {
            repository_id: "acme-monorepo".to_string(),
            source_instance_id: "src-123".to_string(),
            stream: IngestionStream::Messages,
            expected_cursor: 10,
            rows: vec![AgentTraceMessageExportRow {
                source_row_id: 11,
                session_id: "session-1".to_string(),
                message_id: "message-1".to_string(),
                role: MessageRole::User,
                generated_at_unix_ms: 1_700_000_000_000,
            }],
        };

        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["repositoryId"], json!("acme-monorepo"));
        assert_eq!(value["sourceInstanceId"], json!("src-123"));
        assert_eq!(value["stream"], json!("messages"));
        assert_eq!(value["expectedCursor"], json!(10));
        assert_eq!(value["rows"][0]["sourceRowId"], json!(11));
    }

    #[test]
    fn batch_request_composes_with_part_export_rows() {
        let request = AgentTraceIngestionBatchRequest {
            repository_id: "acme-monorepo".to_string(),
            source_instance_id: "src-123".to_string(),
            stream: IngestionStream::Parts,
            expected_cursor: 0,
            rows: vec![AgentTracePartExportRow {
                source_row_id: 1,
                session_id: "session-1".to_string(),
                message_id: "message-1".to_string(),
                part_type: "text".to_string(),
                text: "hello".to_string(),
                generated_at_unix_ms: 1_700_000_000_000,
            }],
        };

        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["stream"], json!("parts"));
        assert_eq!(value["rows"][0]["type"], json!("text"));
    }

    #[test]
    fn batch_request_composes_with_diff_trace_export_rows() {
        let request = AgentTraceIngestionBatchRequest {
            repository_id: "acme-monorepo".to_string(),
            source_instance_id: "src-123".to_string(),
            stream: IngestionStream::DiffTraces,
            expected_cursor: 0,
            rows: vec![AgentTraceDiffTraceExportRow {
                source_row_id: 1,
                session_id: "session-1".to_string(),
                time_ms: 1_700_000_000_000,
                patch: "diff --git a b".to_string(),
                model_id: None,
                tool_name: None,
                tool_version: None,
                payload_type: "patch".to_string(),
            }],
        };

        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["stream"], json!("diff_traces"));
        assert_eq!(value["rows"][0]["payloadType"], json!("patch"));
    }

    #[test]
    fn batch_request_composes_with_agent_trace_export_rows() {
        let request = AgentTraceIngestionBatchRequest {
            repository_id: "acme-monorepo".to_string(),
            source_instance_id: "src-123".to_string(),
            stream: IngestionStream::AgentTraces,
            expected_cursor: 0,
            rows: vec![AgentTraceAgentTraceExportRow {
                source_row_id: 1,
                agent_trace_id: "agent-trace-1".to_string(),
                commit_id: "abc123".to_string(),
                commit_time_ms: 1_700_000_000_000,
                trace_json: "{}".to_string(),
                url: "https://example.com/trace".to_string(),
                remote_url: None,
            }],
        };

        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["stream"], json!("agent_traces"));
        assert_eq!(value["rows"][0]["agentTraceId"], json!("agent-trace-1"));
    }
}
