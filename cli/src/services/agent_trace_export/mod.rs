//! Read-only incremental export readers for the Agent Trace capture streams.
//!
//! This module establishes the local read/export boundary: cursor in, owned
//! wire-compatible rows out. It performs no database mutation, holds no local
//! sync cursor, and makes no network calls.

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::services::agent_trace_db::{repository::RepositoryAgentTraceDb, MessageRole};

/// Maximum number of rows a single export reader call may return.
pub const AGENT_TRACE_EXPORT_BATCH_SIZE: usize = 100;

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
    pub async fn read_messages_after(
        &self,
        cursor: i64,
        limit: usize,
    ) -> Result<Vec<AgentTraceMessageExportRow>> {
        validate_cursor(cursor)?;
        validate_limit(limit)?;

        let rows = self
            .db
            .query_map(
                SELECT_MESSAGES_AFTER_SQL,
                (cursor, limit_as_i64(limit)),
                message_export_row_from_turso,
            )
            .await?;

        for row in &rows {
            validate_js_safe_integer(row.source_row_id)?;
            validate_js_safe_integer(row.generated_at_unix_ms)?;
        }

        Ok(rows)
    }

    /// Read `parts` rows with `id > cursor`, ordered by `id ASC`, capped at
    /// `limit`.
    pub async fn read_parts_after(
        &self,
        cursor: i64,
        limit: usize,
    ) -> Result<Vec<AgentTracePartExportRow>> {
        validate_cursor(cursor)?;
        validate_limit(limit)?;

        let rows = self
            .db
            .query_map(
                SELECT_PARTS_AFTER_SQL,
                (cursor, limit_as_i64(limit)),
                part_export_row_from_turso,
            )
            .await?;

        for row in &rows {
            validate_js_safe_integer(row.source_row_id)?;
            validate_js_safe_integer(row.generated_at_unix_ms)?;
        }

        Ok(rows)
    }

    /// Read `diff_traces` rows with `id > cursor`, ordered by `id ASC`,
    /// capped at `limit`. `patch` and `payload_type` are returned raw and
    /// unmodified: no patch parsing or normalization is performed.
    pub async fn read_diff_traces_after(
        &self,
        cursor: i64,
        limit: usize,
    ) -> Result<Vec<AgentTraceDiffTraceExportRow>> {
        validate_cursor(cursor)?;
        validate_limit(limit)?;

        let rows = self
            .db
            .query_map(
                SELECT_DIFF_TRACES_AFTER_SQL,
                (cursor, limit_as_i64(limit)),
                diff_trace_export_row_from_turso,
            )
            .await?;

        for row in &rows {
            validate_js_safe_integer(row.source_row_id)?;
            validate_js_safe_integer(row.time_ms)?;
        }

        Ok(rows)
    }

    /// Read `agent_traces` rows with `id > cursor`, ordered by `id ASC`,
    /// capped at `limit`. `trace_json` is returned as the exact raw string
    /// from `SQLite`: no parse/reserialize is performed.
    pub async fn read_agent_traces_after(
        &self,
        cursor: i64,
        limit: usize,
    ) -> Result<Vec<AgentTraceAgentTraceExportRow>> {
        validate_cursor(cursor)?;
        validate_limit(limit)?;

        let rows = self
            .db
            .query_map(
                SELECT_AGENT_TRACES_AFTER_SQL,
                (cursor, limit_as_i64(limit)),
                agent_trace_export_row_from_turso,
            )
            .await?;

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
