//! Read-only incremental export readers for the Agent Trace capture streams.
//!
//! This module establishes the local read/export boundary: cursor in, owned
//! wire-compatible rows out. It performs no database mutation, holds no local
//! sync cursor, and makes no network calls.

use anyhow::{bail, Result};
use serde::Serialize;

use crate::services::agent_trace_db::MessageRole;

/// Maximum number of rows a single export reader call may return.
pub const AGENT_TRACE_EXPORT_BATCH_SIZE: usize = 500;

/// Largest integer value that round-trips exactly through an IEEE-754 double
/// (`Number.MAX_SAFE_INTEGER`).
pub const JS_MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// Rejects a negative cursor.
pub fn validate_cursor(cursor: i64) -> Result<()> {
    if cursor < 0 {
        bail!("agent trace export cursor must be >= 0, got {cursor}");
    }

    Ok(())
}

/// Rejects a zero limit or a limit above [`AGENT_TRACE_EXPORT_BATCH_SIZE`].
pub fn validate_limit(limit: usize) -> Result<()> {
    if limit == 0 {
        bail!("agent trace export limit must be greater than 0");
    }

    if limit > AGENT_TRACE_EXPORT_BATCH_SIZE {
        bail!(
            "agent trace export limit {limit} exceeds maximum batch size {AGENT_TRACE_EXPORT_BATCH_SIZE}"
        );
    }

    Ok(())
}

/// Rejects a value outside `0..=JS_MAX_SAFE_INTEGER`, the range an exportable
/// numeric field must stay within to survive JSON round-trip without
/// truncation or casting.
pub fn validate_js_safe_integer(value: i64) -> Result<()> {
    if !(0..=JS_MAX_SAFE_INTEGER).contains(&value) {
        bail!("agent trace export value {value} is outside the JS-safe-integer range 0..={JS_MAX_SAFE_INTEGER}");
    }

    Ok(())
}

/// Owned, wire-compatible export row for the `messages` capture stream.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceMessageExportRow {
    pub source_row_id: i64,
    pub session_id: String,
    pub message_id: String,
    pub role: MessageRole,
    pub generated_at_unix_ms: i64,
}

/// Owned, wire-compatible export row for the `parts` capture stream.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTracePartExportRow {
    pub source_row_id: i64,
    pub session_id: String,
    pub message_id: String,
    #[serde(rename = "type")]
    pub part_type: String,
    pub text: String,
    pub generated_at_unix_ms: i64,
}

/// Owned, wire-compatible export row for the `diff_traces` capture stream.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceDiffTraceExportRow {
    pub source_row_id: i64,
    pub session_id: String,
    pub time_ms: i64,
    pub patch: String,
    pub model_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_version: Option<String>,
    pub payload_type: String,
}

/// Owned, wire-compatible export row for the `agent_traces` capture stream.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceAgentTraceExportRow {
    pub source_row_id: i64,
    pub agent_trace_id: String,
    pub commit_id: String,
    pub commit_time_ms: i64,
    pub trace_json: String,
    pub url: String,
    pub remote_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_export_row_serializes_to_camel_case_contract() {
        let row = AgentTraceMessageExportRow {
            source_row_id: 1,
            session_id: "sess-1".to_string(),
            message_id: "msg-1".to_string(),
            role: MessageRole::Assistant,
            generated_at_unix_ms: 1_700_000_000_000,
        };

        assert_eq!(
            serde_json::to_value(&row).expect("row should serialize"),
            serde_json::json!({
                "sourceRowId": 1,
                "sessionId": "sess-1",
                "messageId": "msg-1",
                "role": "assistant",
                "generatedAtUnixMs": 1_700_000_000_000_i64,
            })
        );
    }

    #[test]
    fn message_export_row_serializes_user_role_lowercase() {
        let row = AgentTraceMessageExportRow {
            source_row_id: 2,
            session_id: "sess-2".to_string(),
            message_id: "msg-2".to_string(),
            role: MessageRole::User,
            generated_at_unix_ms: 1_700_000_000_001,
        };

        let value = serde_json::to_value(&row).expect("row should serialize");
        assert_eq!(value["role"], serde_json::json!("user"));
    }

    #[test]
    fn part_export_row_serializes_to_camel_case_contract() {
        let row = AgentTracePartExportRow {
            source_row_id: 5,
            session_id: "sess-1".to_string(),
            message_id: "msg-1".to_string(),
            part_type: "text".to_string(),
            text: "hello".to_string(),
            generated_at_unix_ms: 1_700_000_000_002,
        };

        assert_eq!(
            serde_json::to_value(&row).expect("row should serialize"),
            serde_json::json!({
                "sourceRowId": 5,
                "sessionId": "sess-1",
                "messageId": "msg-1",
                "type": "text",
                "text": "hello",
                "generatedAtUnixMs": 1_700_000_000_002_i64,
            })
        );
    }

    #[test]
    fn diff_trace_export_row_serializes_with_all_fields_populated() {
        let row = AgentTraceDiffTraceExportRow {
            source_row_id: 7,
            session_id: "sess-3".to_string(),
            time_ms: 1_700_000_000_003,
            patch: "Index: a\n".to_string(),
            model_id: Some("test-provider/test-model".to_string()),
            tool_name: Some("opencode".to_string()),
            tool_version: Some("1.2.3".to_string()),
            payload_type: "patch".to_string(),
        };

        assert_eq!(
            serde_json::to_value(&row).expect("row should serialize"),
            serde_json::json!({
                "sourceRowId": 7,
                "sessionId": "sess-3",
                "timeMs": 1_700_000_000_003_i64,
                "patch": "Index: a\n",
                "modelId": "test-provider/test-model",
                "toolName": "opencode",
                "toolVersion": "1.2.3",
                "payloadType": "patch",
            })
        );
    }

    #[test]
    fn diff_trace_export_row_serializes_nullable_fields_as_null() {
        let row = AgentTraceDiffTraceExportRow {
            source_row_id: 8,
            session_id: "sess-4".to_string(),
            time_ms: 1_700_000_000_004,
            patch: "Index: b\n".to_string(),
            model_id: None,
            tool_name: None,
            tool_version: None,
            payload_type: "structured".to_string(),
        };

        assert_eq!(
            serde_json::to_value(&row).expect("row should serialize"),
            serde_json::json!({
                "sourceRowId": 8,
                "sessionId": "sess-4",
                "timeMs": 1_700_000_000_004_i64,
                "patch": "Index: b\n",
                "modelId": null,
                "toolName": null,
                "toolVersion": null,
                "payloadType": "structured",
            })
        );
    }

    #[test]
    fn agent_trace_export_row_serializes_with_remote_url_null() {
        let row = AgentTraceAgentTraceExportRow {
            source_row_id: 9,
            agent_trace_id: "trace-1".to_string(),
            commit_id: "abc123".to_string(),
            commit_time_ms: 1_700_000_000_005,
            trace_json: "{\"steps\":[]}".to_string(),
            url: "https://example.com/trace/1".to_string(),
            remote_url: None,
        };

        assert_eq!(
            serde_json::to_value(&row).expect("row should serialize"),
            serde_json::json!({
                "sourceRowId": 9,
                "agentTraceId": "trace-1",
                "commitId": "abc123",
                "commitTimeMs": 1_700_000_000_005_i64,
                "traceJson": "{\"steps\":[]}",
                "url": "https://example.com/trace/1",
                "remoteUrl": null,
            })
        );
    }

    #[test]
    fn agent_trace_export_row_serializes_with_remote_url_populated() {
        let row = AgentTraceAgentTraceExportRow {
            source_row_id: 10,
            agent_trace_id: "trace-2".to_string(),
            commit_id: "def456".to_string(),
            commit_time_ms: 1_700_000_000_006,
            trace_json: "{\"steps\":[1]}".to_string(),
            url: "https://example.com/trace/2".to_string(),
            remote_url: Some("https://github.com/org/repo/commit/def456".to_string()),
        };

        let value = serde_json::to_value(&row).expect("row should serialize");
        assert_eq!(
            value["remoteUrl"],
            serde_json::json!("https://github.com/org/repo/commit/def456")
        );
    }

    #[test]
    fn validate_cursor_rejects_negative() {
        let error = validate_cursor(-1).expect_err("negative cursor should error");
        assert!(error.to_string().contains("cursor"));
    }

    #[test]
    fn validate_cursor_accepts_zero_and_positive() {
        assert!(validate_cursor(0).is_ok());
        assert!(validate_cursor(42).is_ok());
    }

    #[test]
    fn validate_limit_rejects_zero() {
        let error = validate_limit(0).expect_err("zero limit should error");
        assert!(error.to_string().contains("limit"));
    }

    #[test]
    fn validate_limit_rejects_above_batch_size() {
        let error = validate_limit(AGENT_TRACE_EXPORT_BATCH_SIZE + 1)
            .expect_err("limit above batch size should error");
        assert!(error.to_string().contains("limit"));
    }

    #[test]
    fn validate_limit_accepts_batch_size() {
        assert!(validate_limit(AGENT_TRACE_EXPORT_BATCH_SIZE).is_ok());
    }

    #[test]
    fn validate_limit_accepts_one() {
        assert!(validate_limit(1).is_ok());
    }

    #[test]
    fn validate_js_safe_integer_accepts_zero() {
        assert!(validate_js_safe_integer(0).is_ok());
    }

    #[test]
    fn validate_js_safe_integer_accepts_max_safe_integer() {
        assert!(validate_js_safe_integer(JS_MAX_SAFE_INTEGER).is_ok());
    }

    #[test]
    fn validate_js_safe_integer_rejects_above_max_safe_integer() {
        let error = validate_js_safe_integer(JS_MAX_SAFE_INTEGER + 1)
            .expect_err("value above max safe integer should error");
        assert!(error.to_string().contains("JS-safe-integer"));
    }

    #[test]
    fn validate_js_safe_integer_rejects_negative() {
        let error = validate_js_safe_integer(-1).expect_err("negative value should error");
        assert!(error.to_string().contains("JS-safe-integer"));
    }
}
