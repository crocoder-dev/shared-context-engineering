//! Read-only incremental export readers for the Agent Trace capture streams.
//!
//! This module establishes the local read/export boundary: cursor in, owned
//! wire-compatible rows out. It performs no database mutation, holds no local
//! sync cursor, and makes no network calls.

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::services::agent_trace_db::{repository::RepositoryAgentTraceDb, MessageRole};

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

const SELECT_MESSAGES_AFTER_SQL: &str =
    "SELECT id, session_id, message_id, role, generated_at_unix_ms
FROM messages
WHERE id > ?1
ORDER BY id ASC
LIMIT ?2";

const SELECT_PARTS_AFTER_SQL: &str =
    "SELECT id, session_id, message_id, type, text, generated_at_unix_ms
FROM parts
WHERE id > ?1
ORDER BY id ASC
LIMIT ?2";

const SELECT_DIFF_TRACES_AFTER_SQL: &str =
    "SELECT id, session_id, time_ms, patch, model_id, tool_name, tool_version, payload_type
FROM diff_traces
WHERE id > ?1
ORDER BY id ASC
LIMIT ?2";

const SELECT_AGENT_TRACES_AFTER_SQL: &str =
    "SELECT id, agent_trace_id, commit_id, commit_time_ms, trace_json, url, remote_url
FROM agent_traces
WHERE id > ?1
ORDER BY id ASC
LIMIT ?2";

/// Read-only incremental export reader over one repository-scoped Agent Trace
/// database. Holds no local cursor, performs no mutation, and makes no
/// network calls; the caller supplies the last server-accepted `id` as
/// `cursor` on every call.
pub struct AgentTraceExportReader<'a> {
    db: &'a RepositoryAgentTraceDb,
}

impl<'a> AgentTraceExportReader<'a> {
    pub fn new(db: &'a RepositoryAgentTraceDb) -> Self {
        Self { db }
    }

    /// Read `messages` rows with `id > cursor`, ordered by `id ASC`, capped
    /// at `limit`.
    pub fn read_messages_after(
        &self,
        cursor: i64,
        limit: usize,
    ) -> Result<Vec<AgentTraceMessageExportRow>> {
        validate_cursor(cursor)?;
        validate_limit(limit)?;

        let rows = self.db.query_map(
            SELECT_MESSAGES_AFTER_SQL,
            (cursor, limit_as_i64(limit)),
            message_export_row_from_turso,
        )?;

        for row in &rows {
            validate_js_safe_integer(row.source_row_id)?;
            validate_js_safe_integer(row.generated_at_unix_ms)?;
        }

        Ok(rows)
    }

    /// Read `parts` rows with `id > cursor`, ordered by `id ASC`, capped at
    /// `limit`.
    pub fn read_parts_after(
        &self,
        cursor: i64,
        limit: usize,
    ) -> Result<Vec<AgentTracePartExportRow>> {
        validate_cursor(cursor)?;
        validate_limit(limit)?;

        let rows = self.db.query_map(
            SELECT_PARTS_AFTER_SQL,
            (cursor, limit_as_i64(limit)),
            part_export_row_from_turso,
        )?;

        for row in &rows {
            validate_js_safe_integer(row.source_row_id)?;
            validate_js_safe_integer(row.generated_at_unix_ms)?;
        }

        Ok(rows)
    }

    /// Read `diff_traces` rows with `id > cursor`, ordered by `id ASC`,
    /// capped at `limit`. `patch` and `payload_type` are returned raw and
    /// unmodified: no patch parsing or normalization is performed.
    pub fn read_diff_traces_after(
        &self,
        cursor: i64,
        limit: usize,
    ) -> Result<Vec<AgentTraceDiffTraceExportRow>> {
        validate_cursor(cursor)?;
        validate_limit(limit)?;

        let rows = self.db.query_map(
            SELECT_DIFF_TRACES_AFTER_SQL,
            (cursor, limit_as_i64(limit)),
            diff_trace_export_row_from_turso,
        )?;

        for row in &rows {
            validate_js_safe_integer(row.source_row_id)?;
            validate_js_safe_integer(row.time_ms)?;
        }

        Ok(rows)
    }

    /// Read `agent_traces` rows with `id > cursor`, ordered by `id ASC`,
    /// capped at `limit`. `trace_json` is returned as the exact raw string
    /// from `SQLite`: no parse/reserialize is performed.
    pub fn read_agent_traces_after(
        &self,
        cursor: i64,
        limit: usize,
    ) -> Result<Vec<AgentTraceAgentTraceExportRow>> {
        validate_cursor(cursor)?;
        validate_limit(limit)?;

        let rows = self.db.query_map(
            SELECT_AGENT_TRACES_AFTER_SQL,
            (cursor, limit_as_i64(limit)),
            agent_trace_export_row_from_turso,
        )?;

        for row in &rows {
            validate_js_safe_integer(row.source_row_id)?;
            validate_js_safe_integer(row.commit_time_ms)?;
        }

        Ok(rows)
    }
}

fn message_export_row_from_turso(row: &turso::Row) -> Result<AgentTraceMessageExportRow> {
    Ok(AgentTraceMessageExportRow {
        source_row_id: row.get(0).context("failed to read messages.id")?,
        session_id: row.get(1).context("failed to read messages.session_id")?,
        message_id: row.get(2).context("failed to read messages.message_id")?,
        role: message_role_from_column(
            row.get::<String>(3)
                .context("failed to read messages.role")?
                .as_str(),
        )?,
        generated_at_unix_ms: row
            .get(4)
            .context("failed to read messages.generated_at_unix_ms")?,
    })
}

fn part_export_row_from_turso(row: &turso::Row) -> Result<AgentTracePartExportRow> {
    Ok(AgentTracePartExportRow {
        source_row_id: row.get(0).context("failed to read parts.id")?,
        session_id: row.get(1).context("failed to read parts.session_id")?,
        message_id: row.get(2).context("failed to read parts.message_id")?,
        part_type: row.get(3).context("failed to read parts.type")?,
        text: row.get(4).context("failed to read parts.text")?,
        generated_at_unix_ms: row
            .get(5)
            .context("failed to read parts.generated_at_unix_ms")?,
    })
}

fn diff_trace_export_row_from_turso(row: &turso::Row) -> Result<AgentTraceDiffTraceExportRow> {
    Ok(AgentTraceDiffTraceExportRow {
        source_row_id: row.get(0).context("failed to read diff_traces.id")?,
        session_id: row
            .get(1)
            .context("failed to read diff_traces.session_id")?,
        time_ms: row.get(2).context("failed to read diff_traces.time_ms")?,
        patch: row.get(3).context("failed to read diff_traces.patch")?,
        model_id: row.get(4).context("failed to read diff_traces.model_id")?,
        tool_name: row.get(5).context("failed to read diff_traces.tool_name")?,
        tool_version: row
            .get(6)
            .context("failed to read diff_traces.tool_version")?,
        payload_type: row
            .get(7)
            .context("failed to read diff_traces.payload_type")?,
    })
}

fn agent_trace_export_row_from_turso(row: &turso::Row) -> Result<AgentTraceAgentTraceExportRow> {
    Ok(AgentTraceAgentTraceExportRow {
        source_row_id: row.get(0).context("failed to read agent_traces.id")?,
        agent_trace_id: row
            .get(1)
            .context("failed to read agent_traces.agent_trace_id")?,
        commit_id: row
            .get(2)
            .context("failed to read agent_traces.commit_id")?,
        commit_time_ms: row
            .get(3)
            .context("failed to read agent_traces.commit_time_ms")?,
        trace_json: row
            .get(4)
            .context("failed to read agent_traces.trace_json")?,
        url: row.get(5).context("failed to read agent_traces.url")?,
        remote_url: row
            .get(6)
            .context("failed to read agent_traces.remote_url")?,
    })
}

fn message_role_from_column(value: &str) -> Result<MessageRole> {
    match value {
        "user" => Ok(MessageRole::User),
        "assistant" => Ok(MessageRole::Assistant),
        other => bail!("agent trace export encountered unknown messages.role value: {other}"),
    }
}

/// Converts a validated `limit` (already bounded by [`validate_limit`] to
/// `1..=AGENT_TRACE_EXPORT_BATCH_SIZE`) into the `i64` the SQL `LIMIT`
/// parameter requires.
fn limit_as_i64(limit: usize) -> i64 {
    i64::try_from(limit).expect("validated limit should fit in i64")
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

    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::services::agent_trace_db::{
        AgentTraceInsert, DiffTraceInsert, InsertMessageInsert, InsertPartInsert, PartType,
        PAYLOAD_TYPE_PATCH, PAYLOAD_TYPE_STRUCTURED,
    };

    fn unique_test_db_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "sce-agent-trace-export-{label}-{}-{nonce}",
                std::process::id()
            ))
            .join("agent-trace.db")
    }

    fn remove_test_db(db_path: &std::path::Path) {
        if let Some(parent) = db_path.parent() {
            fs::remove_dir_all(parent).expect("test DB directory should be removed");
        }
    }

    fn row_count(db: &RepositoryAgentTraceDb, table: &str) -> i64 {
        db.query_map(&format!("SELECT COUNT(*) FROM {table}"), (), |row| {
            row.get::<i64>(0).map_err(Into::into)
        })
        .expect("count query should succeed")
        .into_iter()
        .next()
        .expect("count row should exist")
    }

    fn insert_message_row_with_id(db: &RepositoryAgentTraceDb, id: i64, message_id: &str) {
        db.execute(
            "INSERT INTO messages (id, session_id, message_id, role, generated_at_unix_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            (id, "sess-1", message_id, "assistant", 1_000 + id),
        )
        .expect("direct message insert should succeed");
    }

    fn insert_part_row_with_id(db: &RepositoryAgentTraceDb, id: i64, message_id: &str) {
        db.execute(
            "INSERT INTO parts (id, type, text, message_id, session_id, generated_at_unix_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (id, "text", "hello", message_id, "sess-1", 1_000 + id),
        )
        .expect("direct part insert should succeed");
    }

    fn insert_diff_trace_row_with_id(db: &RepositoryAgentTraceDb, id: i64, session_id: &str) {
        db.execute(
            "INSERT INTO diff_traces (id, time_ms, session_id, patch, model_id, tool_name, tool_version, payload_type) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                id,
                1_000 + id,
                session_id,
                "Index: a\n",
                Option::<&str>::None,
                Option::<&str>::None,
                Option::<&str>::None,
                PAYLOAD_TYPE_PATCH,
            ),
        )
        .expect("direct diff_trace insert should succeed");
    }

    fn insert_agent_trace_row_with_id(db: &RepositoryAgentTraceDb, id: i64, agent_trace_id: &str) {
        db.execute(
            "INSERT INTO agent_traces (id, commit_id, commit_time_ms, trace_json, agent_trace_id, url, remote_url) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                id,
                "abc123",
                1_000 + id,
                "{\"steps\":[]}",
                agent_trace_id,
                "https://example.com/trace",
                Option::<&str>::None,
            ),
        )
        .expect("direct agent_trace insert should succeed");
    }

    #[test]
    fn read_messages_after_returns_rows_after_cursor_in_order() {
        let db_path = unique_test_db_path("messages-basic");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        db.insert_messages(vec![
            InsertMessageInsert {
                session_id: "sess-1".to_string(),
                message_id: "msg-1".to_string(),
                role: MessageRole::User,
                generated_at_unix_ms: 1_001,
            },
            InsertMessageInsert {
                session_id: "sess-1".to_string(),
                message_id: "msg-2".to_string(),
                role: MessageRole::Assistant,
                generated_at_unix_ms: 1_002,
            },
            InsertMessageInsert {
                session_id: "sess-1".to_string(),
                message_id: "msg-3".to_string(),
                role: MessageRole::Assistant,
                generated_at_unix_ms: 1_003,
            },
        ])
        .expect("seed messages should insert");

        let reader = AgentTraceExportReader::new(&db);
        let rows = reader
            .read_messages_after(1, 500)
            .expect("read after cursor should succeed");

        assert_eq!(
            rows.iter().map(|row| row.source_row_id).collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(rows[0].message_id, "msg-2");
        assert_eq!(rows[0].role, MessageRole::Assistant);
        assert_eq!(rows[0].generated_at_unix_ms, 1_002);

        remove_test_db(&db_path);
    }

    #[test]
    fn read_messages_after_returns_non_contiguous_ids() {
        let db_path = unique_test_db_path("messages-gap");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        for (id, message_id) in [
            (11, "msg-11"),
            (15, "msg-15"),
            (19, "msg-19"),
            (30, "msg-30"),
        ] {
            insert_message_row_with_id(&db, id, message_id);
        }

        let reader = AgentTraceExportReader::new(&db);
        let rows = reader
            .read_messages_after(10, 500)
            .expect("read after cursor should succeed");

        assert_eq!(
            rows.iter().map(|row| row.source_row_id).collect::<Vec<_>>(),
            vec![11, 15, 19, 30]
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn read_messages_after_limit_truncates_and_follow_up_continues() {
        let db_path = unique_test_db_path("messages-limit");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        for id in 1..=10 {
            insert_message_row_with_id(&db, id, &format!("msg-{id}"));
        }

        let reader = AgentTraceExportReader::new(&db);
        let first_batch = reader
            .read_messages_after(0, 3)
            .expect("first limited read should succeed");
        assert_eq!(
            first_batch
                .iter()
                .map(|row| row.source_row_id)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );

        let next_cursor = first_batch
            .last()
            .expect("first batch non-empty")
            .source_row_id;
        let second_batch = reader
            .read_messages_after(next_cursor, 3)
            .expect("follow-up read should succeed");
        assert_eq!(
            second_batch
                .iter()
                .map(|row| row.source_row_id)
                .collect::<Vec<_>>(),
            vec![4, 5, 6]
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn read_messages_after_returns_empty_at_or_beyond_max_id() {
        let db_path = unique_test_db_path("messages-empty-tail");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        insert_message_row_with_id(&db, 1, "msg-1");

        let reader = AgentTraceExportReader::new(&db);
        assert!(reader
            .read_messages_after(1, 500)
            .expect("read at max id should succeed")
            .is_empty());
        assert!(reader
            .read_messages_after(100, 500)
            .expect("read beyond max id should succeed")
            .is_empty());

        remove_test_db(&db_path);
    }

    #[test]
    fn read_messages_after_rejects_invalid_cursor_and_limit() {
        let db_path = unique_test_db_path("messages-invalid");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        let reader = AgentTraceExportReader::new(&db);

        let cursor_error = reader
            .read_messages_after(-1, 500)
            .expect_err("negative cursor should error");
        assert!(cursor_error.to_string().contains("cursor"));

        let zero_limit_error = reader
            .read_messages_after(0, 0)
            .expect_err("zero limit should error");
        assert!(zero_limit_error.to_string().contains("limit"));

        let excess_limit_error = reader
            .read_messages_after(0, AGENT_TRACE_EXPORT_BATCH_SIZE + 1)
            .expect_err("excess limit should error");
        assert!(excess_limit_error.to_string().contains("limit"));

        remove_test_db(&db_path);
    }

    #[test]
    fn read_messages_after_rejects_row_above_safe_integer_bound() {
        let db_path = unique_test_db_path("messages-unsafe-integer");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        db.execute(
            "INSERT INTO messages (id, session_id, message_id, role, generated_at_unix_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            (1_i64, "sess-1", "msg-1", "assistant", JS_MAX_SAFE_INTEGER + 1),
        )
        .expect("direct message insert should succeed");

        let reader = AgentTraceExportReader::new(&db);
        let error = reader
            .read_messages_after(0, 500)
            .expect_err("row above safe-integer bound should error");
        assert!(error.to_string().contains("JS-safe-integer"));

        remove_test_db(&db_path);
    }

    #[test]
    fn read_messages_after_performs_no_mutation() {
        let db_path = unique_test_db_path("messages-no-mutation");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        insert_message_row_with_id(&db, 1, "msg-1");
        insert_message_row_with_id(&db, 2, "msg-2");

        let messages_before = row_count(&db, "messages");
        let metadata_before = row_count(&db, "repository_metadata");

        let reader = AgentTraceExportReader::new(&db);
        reader
            .read_messages_after(0, 500)
            .expect("read should succeed");

        assert_eq!(row_count(&db, "messages"), messages_before);
        assert_eq!(row_count(&db, "repository_metadata"), metadata_before);

        remove_test_db(&db_path);
    }

    #[test]
    fn read_parts_after_returns_rows_after_cursor_in_order() {
        let db_path = unique_test_db_path("parts-basic");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        db.insert_parts(vec![
            InsertPartInsert {
                part_type: PartType::Text,
                text: "first".to_string(),
                session_id: "sess-1".to_string(),
                message_id: "msg-1".to_string(),
                generated_at_unix_ms: 1_001,
            },
            InsertPartInsert {
                part_type: PartType::Patch,
                text: "second".to_string(),
                session_id: "sess-1".to_string(),
                message_id: "msg-2".to_string(),
                generated_at_unix_ms: 1_002,
            },
        ])
        .expect("seed parts should insert");

        let reader = AgentTraceExportReader::new(&db);
        let rows = reader
            .read_parts_after(0, 500)
            .expect("read after cursor should succeed");

        assert_eq!(
            rows.iter().map(|row| row.source_row_id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(rows[1].part_type, "patch");
        assert_eq!(rows[1].text, "second");
        assert_eq!(rows[1].message_id, "msg-2");
        assert_eq!(rows[1].generated_at_unix_ms, 1_002);

        remove_test_db(&db_path);
    }

    #[test]
    fn read_parts_after_returns_non_contiguous_ids() {
        let db_path = unique_test_db_path("parts-gap");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        for (id, message_id) in [
            (11, "msg-11"),
            (15, "msg-15"),
            (19, "msg-19"),
            (30, "msg-30"),
        ] {
            insert_part_row_with_id(&db, id, message_id);
        }

        let reader = AgentTraceExportReader::new(&db);
        let rows = reader
            .read_parts_after(10, 500)
            .expect("read after cursor should succeed");

        assert_eq!(
            rows.iter().map(|row| row.source_row_id).collect::<Vec<_>>(),
            vec![11, 15, 19, 30]
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn read_parts_after_limit_truncates_and_follow_up_continues() {
        let db_path = unique_test_db_path("parts-limit");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        for id in 1..=10 {
            insert_part_row_with_id(&db, id, &format!("msg-{id}"));
        }

        let reader = AgentTraceExportReader::new(&db);
        let first_batch = reader
            .read_parts_after(0, 3)
            .expect("first limited read should succeed");
        assert_eq!(
            first_batch
                .iter()
                .map(|row| row.source_row_id)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );

        let next_cursor = first_batch
            .last()
            .expect("first batch non-empty")
            .source_row_id;
        let second_batch = reader
            .read_parts_after(next_cursor, 3)
            .expect("follow-up read should succeed");
        assert_eq!(
            second_batch
                .iter()
                .map(|row| row.source_row_id)
                .collect::<Vec<_>>(),
            vec![4, 5, 6]
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn read_parts_after_returns_empty_at_or_beyond_max_id() {
        let db_path = unique_test_db_path("parts-empty-tail");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        insert_part_row_with_id(&db, 1, "msg-1");

        let reader = AgentTraceExportReader::new(&db);
        assert!(reader
            .read_parts_after(1, 500)
            .expect("read at max id should succeed")
            .is_empty());
        assert!(reader
            .read_parts_after(100, 500)
            .expect("read beyond max id should succeed")
            .is_empty());

        remove_test_db(&db_path);
    }

    #[test]
    fn read_parts_after_rejects_invalid_cursor_and_limit() {
        let db_path = unique_test_db_path("parts-invalid");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        let reader = AgentTraceExportReader::new(&db);

        let cursor_error = reader
            .read_parts_after(-1, 500)
            .expect_err("negative cursor should error");
        assert!(cursor_error.to_string().contains("cursor"));

        let zero_limit_error = reader
            .read_parts_after(0, 0)
            .expect_err("zero limit should error");
        assert!(zero_limit_error.to_string().contains("limit"));

        let excess_limit_error = reader
            .read_parts_after(0, AGENT_TRACE_EXPORT_BATCH_SIZE + 1)
            .expect_err("excess limit should error");
        assert!(excess_limit_error.to_string().contains("limit"));

        remove_test_db(&db_path);
    }

    #[test]
    fn read_parts_after_rejects_row_above_safe_integer_bound() {
        let db_path = unique_test_db_path("parts-unsafe-integer");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        db.execute(
            "INSERT INTO parts (id, type, text, message_id, session_id, generated_at_unix_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (1_i64, "text", "hello", "msg-1", "sess-1", JS_MAX_SAFE_INTEGER + 1),
        )
        .expect("direct part insert should succeed");

        let reader = AgentTraceExportReader::new(&db);
        let error = reader
            .read_parts_after(0, 500)
            .expect_err("row above safe-integer bound should error");
        assert!(error.to_string().contains("JS-safe-integer"));

        remove_test_db(&db_path);
    }

    #[test]
    fn read_parts_after_performs_no_mutation() {
        let db_path = unique_test_db_path("parts-no-mutation");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        insert_part_row_with_id(&db, 1, "msg-1");
        insert_part_row_with_id(&db, 2, "msg-2");

        let parts_before = row_count(&db, "parts");
        let metadata_before = row_count(&db, "repository_metadata");

        let reader = AgentTraceExportReader::new(&db);
        reader
            .read_parts_after(0, 500)
            .expect("read should succeed");

        assert_eq!(row_count(&db, "parts"), parts_before);
        assert_eq!(row_count(&db, "repository_metadata"), metadata_before);

        remove_test_db(&db_path);
    }

    #[test]
    fn read_diff_traces_after_returns_rows_after_cursor_in_order() {
        let db_path = unique_test_db_path("diff-traces-basic");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        db.insert_diff_trace(DiffTraceInsert {
            time_ms: 1_001,
            session_id: "sess-1",
            patch: "Index: a\n",
            model_id: Some("test-provider/test-model"),
            tool_name: "opencode",
            tool_version: Some("1.2.3"),
            payload_type: PAYLOAD_TYPE_PATCH,
        })
        .expect("seed diff_trace should insert");
        db.insert_diff_trace(DiffTraceInsert {
            time_ms: 1_002,
            session_id: "sess-1",
            patch: "{\"tool\":\"Edit\"}",
            model_id: None,
            tool_name: "claude",
            tool_version: None,
            payload_type: PAYLOAD_TYPE_STRUCTURED,
        })
        .expect("seed diff_trace should insert");

        let reader = AgentTraceExportReader::new(&db);
        let rows = reader
            .read_diff_traces_after(1, 500)
            .expect("read after cursor should succeed");

        assert_eq!(
            rows.iter().map(|row| row.source_row_id).collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(rows[0].patch, "{\"tool\":\"Edit\"}");
        assert_eq!(rows[0].payload_type, "structured");
        assert_eq!(rows[0].model_id, None);
        assert_eq!(rows[0].tool_name, Some("claude".to_string()));
        assert_eq!(rows[0].tool_version, None);
        assert_eq!(rows[0].time_ms, 1_002);

        remove_test_db(&db_path);
    }

    #[test]
    fn read_diff_traces_after_preserves_populated_nullable_fields() {
        let db_path = unique_test_db_path("diff-traces-nullable-populated");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        db.insert_diff_trace(DiffTraceInsert {
            time_ms: 1_001,
            session_id: "sess-1",
            patch: "Index: a\n",
            model_id: Some("test-provider/test-model"),
            tool_name: "opencode",
            tool_version: Some("1.2.3"),
            payload_type: PAYLOAD_TYPE_PATCH,
        })
        .expect("seed diff_trace should insert");

        let reader = AgentTraceExportReader::new(&db);
        let rows = reader
            .read_diff_traces_after(0, 500)
            .expect("read after cursor should succeed");

        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].model_id,
            Some("test-provider/test-model".to_string())
        );
        assert_eq!(rows[0].tool_name, Some("opencode".to_string()));
        assert_eq!(rows[0].tool_version, Some("1.2.3".to_string()));
        assert_eq!(rows[0].patch, "Index: a\n");

        remove_test_db(&db_path);
    }

    #[test]
    fn read_diff_traces_after_returns_non_contiguous_ids() {
        let db_path = unique_test_db_path("diff-traces-gap");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        for (id, session_id) in [
            (11, "sess-11"),
            (15, "sess-15"),
            (19, "sess-19"),
            (30, "sess-30"),
        ] {
            insert_diff_trace_row_with_id(&db, id, session_id);
        }

        let reader = AgentTraceExportReader::new(&db);
        let rows = reader
            .read_diff_traces_after(10, 500)
            .expect("read after cursor should succeed");

        assert_eq!(
            rows.iter().map(|row| row.source_row_id).collect::<Vec<_>>(),
            vec![11, 15, 19, 30]
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn read_diff_traces_after_limit_truncates_and_follow_up_continues() {
        let db_path = unique_test_db_path("diff-traces-limit");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        for id in 1..=10 {
            insert_diff_trace_row_with_id(&db, id, &format!("sess-{id}"));
        }

        let reader = AgentTraceExportReader::new(&db);
        let first_batch = reader
            .read_diff_traces_after(0, 3)
            .expect("first limited read should succeed");
        assert_eq!(
            first_batch
                .iter()
                .map(|row| row.source_row_id)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );

        let next_cursor = first_batch
            .last()
            .expect("first batch non-empty")
            .source_row_id;
        let second_batch = reader
            .read_diff_traces_after(next_cursor, 3)
            .expect("follow-up read should succeed");
        assert_eq!(
            second_batch
                .iter()
                .map(|row| row.source_row_id)
                .collect::<Vec<_>>(),
            vec![4, 5, 6]
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn read_diff_traces_after_returns_empty_at_or_beyond_max_id() {
        let db_path = unique_test_db_path("diff-traces-empty-tail");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        insert_diff_trace_row_with_id(&db, 1, "sess-1");

        let reader = AgentTraceExportReader::new(&db);
        assert!(reader
            .read_diff_traces_after(1, 500)
            .expect("read at max id should succeed")
            .is_empty());
        assert!(reader
            .read_diff_traces_after(100, 500)
            .expect("read beyond max id should succeed")
            .is_empty());

        remove_test_db(&db_path);
    }

    #[test]
    fn read_diff_traces_after_rejects_invalid_cursor_and_limit() {
        let db_path = unique_test_db_path("diff-traces-invalid");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        let reader = AgentTraceExportReader::new(&db);

        let cursor_error = reader
            .read_diff_traces_after(-1, 500)
            .expect_err("negative cursor should error");
        assert!(cursor_error.to_string().contains("cursor"));

        let zero_limit_error = reader
            .read_diff_traces_after(0, 0)
            .expect_err("zero limit should error");
        assert!(zero_limit_error.to_string().contains("limit"));

        let excess_limit_error = reader
            .read_diff_traces_after(0, AGENT_TRACE_EXPORT_BATCH_SIZE + 1)
            .expect_err("excess limit should error");
        assert!(excess_limit_error.to_string().contains("limit"));

        remove_test_db(&db_path);
    }

    #[test]
    fn read_diff_traces_after_rejects_row_above_safe_integer_bound() {
        let db_path = unique_test_db_path("diff-traces-unsafe-integer");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        db.execute(
            "INSERT INTO diff_traces (id, time_ms, session_id, patch, model_id, tool_name, tool_version, payload_type) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                1_i64,
                JS_MAX_SAFE_INTEGER + 1,
                "sess-1",
                "Index: a\n",
                Option::<&str>::None,
                Option::<&str>::None,
                Option::<&str>::None,
                PAYLOAD_TYPE_PATCH,
            ),
        )
        .expect("direct diff_trace insert should succeed");

        let reader = AgentTraceExportReader::new(&db);
        let error = reader
            .read_diff_traces_after(0, 500)
            .expect_err("row above safe-integer bound should error");
        assert!(error.to_string().contains("JS-safe-integer"));

        remove_test_db(&db_path);
    }

    #[test]
    fn read_diff_traces_after_performs_no_mutation() {
        let db_path = unique_test_db_path("diff-traces-no-mutation");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        insert_diff_trace_row_with_id(&db, 1, "sess-1");
        insert_diff_trace_row_with_id(&db, 2, "sess-2");

        let diff_traces_before = row_count(&db, "diff_traces");
        let metadata_before = row_count(&db, "repository_metadata");

        let reader = AgentTraceExportReader::new(&db);
        reader
            .read_diff_traces_after(0, 500)
            .expect("read should succeed");

        assert_eq!(row_count(&db, "diff_traces"), diff_traces_before);
        assert_eq!(row_count(&db, "repository_metadata"), metadata_before);

        remove_test_db(&db_path);
    }

    #[test]
    fn read_agent_traces_after_returns_rows_after_cursor_in_order() {
        let db_path = unique_test_db_path("agent-traces-basic");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        db.insert_agent_trace(AgentTraceInsert {
            commit_id: "abc123",
            commit_time_ms: 1_001,
            trace_json: "{\"steps\":[]}",
            agent_trace_id: "trace-1",
            url: "https://example.com/trace/1",
            remote_url: "",
        })
        .expect("seed agent_trace should insert");
        db.insert_agent_trace(AgentTraceInsert {
            commit_id: "def456",
            commit_time_ms: 1_002,
            trace_json: "{\"steps\":[1]}",
            agent_trace_id: "trace-2",
            url: "https://example.com/trace/2",
            remote_url: "https://github.com/org/repo/commit/def456",
        })
        .expect("seed agent_trace should insert");

        let reader = AgentTraceExportReader::new(&db);
        let rows = reader
            .read_agent_traces_after(1, 500)
            .expect("read after cursor should succeed");

        assert_eq!(
            rows.iter().map(|row| row.source_row_id).collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(rows[0].agent_trace_id, "trace-2");
        assert_eq!(rows[0].commit_id, "def456");
        assert_eq!(rows[0].commit_time_ms, 1_002);
        assert_eq!(rows[0].trace_json, "{\"steps\":[1]}");
        assert_eq!(rows[0].url, "https://example.com/trace/2");
        assert_eq!(
            rows[0].remote_url,
            Some("https://github.com/org/repo/commit/def456".to_string())
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn read_agent_traces_after_preserves_null_remote_url() {
        let db_path = unique_test_db_path("agent-traces-remote-url-null");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        insert_agent_trace_row_with_id(&db, 1, "trace-null");

        let reader = AgentTraceExportReader::new(&db);
        let rows = reader
            .read_agent_traces_after(0, 500)
            .expect("read after cursor should succeed");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].remote_url, None);

        remove_test_db(&db_path);
    }

    #[test]
    fn read_agent_traces_after_returns_non_contiguous_ids() {
        let db_path = unique_test_db_path("agent-traces-gap");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        for (id, agent_trace_id) in [
            (11, "trace-11"),
            (15, "trace-15"),
            (19, "trace-19"),
            (30, "trace-30"),
        ] {
            insert_agent_trace_row_with_id(&db, id, agent_trace_id);
        }

        let reader = AgentTraceExportReader::new(&db);
        let rows = reader
            .read_agent_traces_after(10, 500)
            .expect("read after cursor should succeed");

        assert_eq!(
            rows.iter().map(|row| row.source_row_id).collect::<Vec<_>>(),
            vec![11, 15, 19, 30]
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn read_agent_traces_after_limit_truncates_and_follow_up_continues() {
        let db_path = unique_test_db_path("agent-traces-limit");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        for id in 1..=10 {
            insert_agent_trace_row_with_id(&db, id, &format!("trace-{id}"));
        }

        let reader = AgentTraceExportReader::new(&db);
        let first_batch = reader
            .read_agent_traces_after(0, 3)
            .expect("first limited read should succeed");
        assert_eq!(
            first_batch
                .iter()
                .map(|row| row.source_row_id)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );

        let next_cursor = first_batch
            .last()
            .expect("first batch non-empty")
            .source_row_id;
        let second_batch = reader
            .read_agent_traces_after(next_cursor, 3)
            .expect("follow-up read should succeed");
        assert_eq!(
            second_batch
                .iter()
                .map(|row| row.source_row_id)
                .collect::<Vec<_>>(),
            vec![4, 5, 6]
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn read_agent_traces_after_returns_empty_at_or_beyond_max_id() {
        let db_path = unique_test_db_path("agent-traces-empty-tail");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        insert_agent_trace_row_with_id(&db, 1, "trace-1");

        let reader = AgentTraceExportReader::new(&db);
        assert!(reader
            .read_agent_traces_after(1, 500)
            .expect("read at max id should succeed")
            .is_empty());
        assert!(reader
            .read_agent_traces_after(100, 500)
            .expect("read beyond max id should succeed")
            .is_empty());

        remove_test_db(&db_path);
    }

    #[test]
    fn read_agent_traces_after_rejects_invalid_cursor_and_limit() {
        let db_path = unique_test_db_path("agent-traces-invalid");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        let reader = AgentTraceExportReader::new(&db);

        let cursor_error = reader
            .read_agent_traces_after(-1, 500)
            .expect_err("negative cursor should error");
        assert!(cursor_error.to_string().contains("cursor"));

        let zero_limit_error = reader
            .read_agent_traces_after(0, 0)
            .expect_err("zero limit should error");
        assert!(zero_limit_error.to_string().contains("limit"));

        let excess_limit_error = reader
            .read_agent_traces_after(0, AGENT_TRACE_EXPORT_BATCH_SIZE + 1)
            .expect_err("excess limit should error");
        assert!(excess_limit_error.to_string().contains("limit"));

        remove_test_db(&db_path);
    }

    #[test]
    fn read_agent_traces_after_rejects_row_above_safe_integer_bound() {
        let db_path = unique_test_db_path("agent-traces-unsafe-integer");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        db.execute(
            "INSERT INTO agent_traces (id, commit_id, commit_time_ms, trace_json, agent_trace_id, url, remote_url) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                1_i64,
                "abc123",
                JS_MAX_SAFE_INTEGER + 1,
                "{\"steps\":[]}",
                "trace-unsafe",
                "https://example.com/trace",
                Option::<&str>::None,
            ),
        )
        .expect("direct agent_trace insert should succeed");

        let reader = AgentTraceExportReader::new(&db);
        let error = reader
            .read_agent_traces_after(0, 500)
            .expect_err("row above safe-integer bound should error");
        assert!(error.to_string().contains("JS-safe-integer"));

        remove_test_db(&db_path);
    }

    #[test]
    fn read_agent_traces_after_performs_no_mutation() {
        let db_path = unique_test_db_path("agent-traces-no-mutation");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");
        insert_agent_trace_row_with_id(&db, 1, "trace-1");
        insert_agent_trace_row_with_id(&db, 2, "trace-2");

        let agent_traces_before = row_count(&db, "agent_traces");
        let metadata_before = row_count(&db, "repository_metadata");

        let reader = AgentTraceExportReader::new(&db);
        reader
            .read_agent_traces_after(0, 500)
            .expect("read should succeed");

        assert_eq!(row_count(&db, "agent_traces"), agent_traces_before);
        assert_eq!(row_count(&db, "repository_metadata"), metadata_before);

        remove_test_db(&db_path);
    }
}
