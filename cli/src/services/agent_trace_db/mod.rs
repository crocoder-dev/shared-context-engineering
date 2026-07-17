//! Agent trace Turso database adapter.

use anyhow::{Context, Result};
use turso::Value as TursoValue;

use crate::services::{
    db::{DbSpec, TursoDb},
    patch::{parse_patch, ParseError, ParsedPatch},
    structured_patch::{derive_claude_structured_patch, ClaudeStructuredPatchDerivationResult},
};

use serde_json::Value;

pub mod lifecycle;
pub mod repository;

/// Payload type discriminator for diff trace source payloads.
///
/// `OpenCode` normalized diff-trace payloads use [`PAYLOAD_TYPE_PATCH`].
/// `Claude` structured `PostToolUse` payloads use [`PAYLOAD_TYPE_STRUCTURED`].
pub const PAYLOAD_TYPE_PATCH: &str = "patch";
pub const PAYLOAD_TYPE_STRUCTURED: &str = "structured";

/// Parameterized SQL for inserting a captured diff trace payload.
pub const INSERT_DIFF_TRACE_SQL: &str =
    "INSERT INTO diff_traces (time_ms, session_id, patch, model_id, tool_name, tool_version, payload_type) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

/// Parameterized SQL for retrieving recent captured diff trace patches.
pub const SELECT_RECENT_DIFF_TRACE_PATCHES_SQL: &str =
    "SELECT id, time_ms, session_id, patch, model_id, tool_name, tool_version, payload_type
FROM diff_traces
WHERE time_ms >= ?1 AND time_ms <= ?2
ORDER BY time_ms ASC, id ASC";

/// Parameterized SQL for inserting a post-commit patch intersection result.
pub const INSERT_POST_COMMIT_PATCH_INTERSECTION_SQL: &str =
    "INSERT INTO post_commit_patch_intersections (
    commit_id,
    post_commit_time_ms,
    recent_window_cutoff_ms,
    recent_window_end_ms,
    loaded_diff_trace_count,
    skipped_diff_trace_count,
    intersection_patch
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

/// Parameterized SQL for inserting a built agent trace payload.
pub const INSERT_AGENT_TRACE_SQL: &str =
    "INSERT INTO agent_traces (commit_id, commit_time_ms, trace_json, agent_trace_id, url, remote_url) VALUES (?1, ?2, ?3, ?4, ?5, ?6)";

/// Parameterized SQL for inserting a message row. Duplicate
/// `(session_id, message_id)` writes are ignored.
pub const INSERT_MESSAGE_SQL: &str =
    "INSERT INTO messages (session_id, message_id, role, generated_at_unix_ms)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT (session_id, message_id) DO NOTHING";

/// Parameterized SQL for inserting a part row (append-only, no upsert).
pub const INSERT_PART_SQL: &str =
    "INSERT INTO parts (type, text, message_id, session_id, generated_at_unix_ms)
VALUES (?1, ?2, ?3, ?4, ?5)";

/// Diff trace payload to persist in the agent trace database.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiffTraceInsert<'a> {
    pub time_ms: i64,
    pub session_id: &'a str,
    pub patch: &'a str,
    pub model_id: Option<&'a str>,
    pub tool_name: &'a str,
    pub tool_version: Option<&'a str>,
    pub payload_type: &'a str,
}

/// Raw diff trace row read from the agent trace database.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffTracePatchRow {
    pub id: i64,
    pub time_ms: i64,
    pub session_id: String,
    pub patch: String,
    pub model_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_version: Option<String>,
    pub payload_type: String,
}

/// Parsed recent diff trace patch ready for comparison flows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedDiffTracePatch {
    pub id: i64,
    pub time_ms: i64,
    pub session_id: String,
    pub patch: ParsedPatch,
    pub tool_name: Option<String>,
    pub tool_version: Option<String>,
    pub payload_type: String,
}

/// Deterministic skipped-row report for invalid recent diff trace patches.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkippedDiffTracePatch {
    pub id: i64,
    pub time_ms: i64,
    pub session_id: String,
    pub reason: String,
}

/// Parsed recent diff trace query result with accounting for valid and skipped rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecentDiffTracePatches {
    pub patches: Vec<ParsedDiffTracePatch>,
    pub skipped: Vec<SkippedDiffTracePatch>,
}

impl RecentDiffTracePatches {
    pub fn loaded_count(&self) -> usize {
        self.patches.len()
    }

    pub fn skipped_count(&self) -> usize {
        self.skipped.len()
    }
}

/// Post-commit patch intersection result to persist in the agent trace database.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PostCommitPatchIntersectionInsert<'a> {
    pub commit_id: &'a str,
    pub post_commit_time_ms: i64,
    pub recent_window_cutoff_ms: i64,
    pub recent_window_end_ms: i64,
    pub loaded_diff_trace_count: i64,
    pub skipped_diff_trace_count: i64,
    pub intersection_patch: &'a str,
}

/// Built agent trace payload to persist in the agent trace database.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentTraceInsert<'a> {
    pub commit_id: &'a str,
    pub commit_time_ms: i64,
    pub trace_json: &'a str,
    pub agent_trace_id: &'a str,
    pub url: &'a str,
    pub remote_url: &'a str,
}

/// Message role constraint for the `messages` table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
}

impl std::fmt::Display for MessageRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::Assistant => write!(f, "assistant"),
        }
    }
}

/// Message insert payload for the `messages` table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InsertMessageInsert {
    pub session_id: String,
    pub message_id: String,
    pub role: MessageRole,
    pub generated_at_unix_ms: i64,
}

/// Part type constraint for the `parts` table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PartType {
    Text,
    Reasoning,
    Patch,
    Question,
}

impl std::fmt::Display for PartType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text => write!(f, "text"),
            Self::Reasoning => write!(f, "reasoning"),
            Self::Patch => write!(f, "patch"),
            Self::Question => write!(f, "question"),
        }
    }
}

/// Part insert payload for the `parts` table (append-only, no upsert).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InsertPartInsert {
    pub part_type: PartType,
    pub text: String,
    pub session_id: String,
    pub message_id: String,
    pub generated_at_unix_ms: i64,
}

fn insert_diff_trace_with<M: DbSpec>(db: &TursoDb<M>, input: DiffTraceInsert<'_>) -> Result<u64> {
    db.execute(
        INSERT_DIFF_TRACE_SQL,
        (
            input.time_ms,
            input.session_id,
            input.patch,
            input.model_id,
            input.tool_name,
            input.tool_version,
            input.payload_type,
        ),
    )
}

fn insert_post_commit_patch_intersection_with<M: DbSpec>(
    db: &TursoDb<M>,
    input: PostCommitPatchIntersectionInsert<'_>,
) -> Result<u64> {
    db.execute(
        INSERT_POST_COMMIT_PATCH_INTERSECTION_SQL,
        (
            input.commit_id,
            input.post_commit_time_ms,
            input.recent_window_cutoff_ms,
            input.recent_window_end_ms,
            input.loaded_diff_trace_count,
            input.skipped_diff_trace_count,
            input.intersection_patch,
        ),
    )
}

fn insert_agent_trace_with<M: DbSpec>(db: &TursoDb<M>, input: AgentTraceInsert<'_>) -> Result<u64> {
    db.execute(
        INSERT_AGENT_TRACE_SQL,
        (
            input.commit_id,
            input.commit_time_ms,
            input.trace_json,
            input.agent_trace_id,
            input.url,
            input.remote_url,
        ),
    )
}

#[allow(dead_code)]
fn insert_message_with<M: DbSpec>(db: &TursoDb<M>, input: InsertMessageInsert) -> Result<u64> {
    db.execute(
        INSERT_MESSAGE_SQL,
        (
            input.session_id,
            input.message_id,
            input.role.to_string(),
            input.generated_at_unix_ms,
        ),
    )
}

fn insert_messages_with<M: DbSpec>(
    db: &TursoDb<M>,
    inputs: Vec<InsertMessageInsert>,
) -> Result<u64> {
    if inputs.is_empty() {
        return Ok(0);
    }

    let mut params = Vec::with_capacity(inputs.len() * 4);
    let mut rows = Vec::with_capacity(inputs.len());

    for (row_index, input) in inputs.into_iter().enumerate() {
        let param_start = row_index * 4 + 1;
        rows.push(numbered_placeholders(param_start, 4));
        params.push(TursoValue::Text(input.session_id));
        params.push(TursoValue::Text(input.message_id));
        params.push(TursoValue::Text(input.role.to_string()));
        params.push(TursoValue::Integer(input.generated_at_unix_ms));
    }

    let sql = format!(
        "INSERT INTO messages (session_id, message_id, role, generated_at_unix_ms)\nVALUES {}\nON CONFLICT (session_id, message_id) DO NOTHING",
        rows.join(", ")
    );

    db.execute(&sql, params)
}

#[allow(dead_code)]
fn insert_part_with<M: DbSpec>(db: &TursoDb<M>, input: InsertPartInsert) -> Result<u64> {
    db.execute(
        INSERT_PART_SQL,
        (
            input.part_type.to_string(),
            input.text,
            input.message_id,
            input.session_id,
            input.generated_at_unix_ms,
        ),
    )
}

fn insert_parts_with<M: DbSpec>(db: &TursoDb<M>, inputs: Vec<InsertPartInsert>) -> Result<u64> {
    if inputs.is_empty() {
        return Ok(0);
    }

    let mut params = Vec::with_capacity(inputs.len() * 5);
    let mut rows = Vec::with_capacity(inputs.len());

    for (row_index, input) in inputs.into_iter().enumerate() {
        let param_start = row_index * 5 + 1;
        rows.push(numbered_placeholders(param_start, 5));
        params.push(TursoValue::Text(input.part_type.to_string()));
        params.push(TursoValue::Text(input.text));
        params.push(TursoValue::Text(input.message_id));
        params.push(TursoValue::Text(input.session_id));
        params.push(TursoValue::Integer(input.generated_at_unix_ms));
    }

    let sql = format!(
        "INSERT INTO parts (type, text, message_id, session_id, generated_at_unix_ms)\nVALUES {}",
        rows.join(", ")
    );

    db.execute(&sql, params)
}

fn numbered_placeholders(start: usize, count: usize) -> String {
    let placeholders = (start..start + count)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");

    format!("({placeholders})")
}

fn recent_diff_trace_patches_with<M: DbSpec>(
    db: &TursoDb<M>,
    cutoff_time_ms: i64,
    end_time_ms: i64,
) -> Result<RecentDiffTracePatches> {
    let rows = db.query_map(
        SELECT_RECENT_DIFF_TRACE_PATCHES_SQL,
        (cutoff_time_ms, end_time_ms),
        diff_trace_patch_row_from_turso,
    )?;

    Ok(parse_recent_diff_trace_patch_rows(rows))
}

fn diff_trace_patch_row_from_turso(row: &turso::Row) -> Result<DiffTracePatchRow> {
    Ok(DiffTracePatchRow {
        id: row.get(0).context("failed to read diff_traces.id")?,
        time_ms: row.get(1).context("failed to read diff_traces.time_ms")?,
        session_id: row
            .get(2)
            .context("failed to read diff_traces.session_id")?,
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

fn parse_recent_diff_trace_patch_rows(rows: Vec<DiffTracePatchRow>) -> RecentDiffTracePatches {
    let mut patches = Vec::new();
    let mut skipped = Vec::new();

    for row in rows {
        let parse_result = match row.payload_type.as_str() {
            PAYLOAD_TYPE_PATCH => parse_patch(&row.patch, Some(row.session_id.as_str()))
                .map_err(|error| skipped_diff_trace_patch_reason(&error)),
            PAYLOAD_TYPE_STRUCTURED => match serde_json::from_str::<Value>(&row.patch) {
                Ok(payload) => match derive_claude_structured_patch(
                    "PostToolUse",
                    &payload,
                    u64::try_from(row.time_ms).expect("diff trace time_ms should be non-negative"),
                    row.tool_version.as_deref(),
                ) {
                    ClaudeStructuredPatchDerivationResult::Derived(derived) => Ok(derived.patch),
                    ClaudeStructuredPatchDerivationResult::Skipped(reason) => {
                        Err(reason.to_string())
                    }
                },
                Err(error) => Err(format!("invalid structured payload JSON: {error}")),
            },
            other => Err(format!("unsupported diff-trace payload_type: {other}")),
        };

        match parse_result {
            Ok(mut patch) => {
                for file in &mut patch.files {
                    for hunk in &mut file.hunks {
                        hunk.model_id.clone_from(&row.model_id);
                    }
                }

                patches.push(ParsedDiffTracePatch {
                    id: row.id,
                    time_ms: row.time_ms,
                    session_id: row.session_id,
                    patch,
                    tool_name: row.tool_name,
                    tool_version: row.tool_version,
                    payload_type: row.payload_type,
                });
            }
            Err(reason) => skipped.push(SkippedDiffTracePatch {
                id: row.id,
                time_ms: row.time_ms,
                session_id: row.session_id,
                reason,
            }),
        }
    }

    RecentDiffTracePatches { patches, skipped }
}

fn skipped_diff_trace_patch_reason(error: &ParseError) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::repository::RepositoryAgentTraceDb;
    use super::*;

    fn unique_test_db_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "sce-agent-trace-db-test-{}-{nonce}",
                std::process::id()
            ))
            .join("agent-trace.db")
    }

    fn valid_patch(path: &str, content: &str) -> String {
        format!(
            "Index: {path}\n===================================================================\n--- {path}\n+++ {path}\n@@ -0,0 +1,1 @@\n+{content}\n"
        )
    }

    fn insert_test_diff_trace(
        db: &RepositoryAgentTraceDb,
        time_ms: i64,
        session_id: &str,
        patch: &str,
    ) {
        db.insert_diff_trace(DiffTraceInsert {
            time_ms,
            session_id,
            patch,
            model_id: Some("test-provider/test-model"),
            tool_name: "opencode",
            tool_version: Some("1.2.3"),
            payload_type: PAYLOAD_TYPE_PATCH,
        })
        .expect("diff trace insert should succeed");
    }

    #[test]
    fn recent_diff_trace_patches_applies_bounded_window_ordering_and_parse_accounting() {
        let db_path = unique_test_db_path();
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test DB should open");

        let before_cutoff_patch = valid_patch("notes/before.md", "before cutoff");
        let cutoff_patch = valid_patch("notes/cutoff.md", "at cutoff");
        let first_same_time_patch = valid_patch("notes/same-a.md", "same time first");
        let second_same_time_patch = valid_patch("notes/same-b.md", "same time second");
        let end_patch = valid_patch("notes/end.md", "at end");
        let after_end_patch = valid_patch("notes/after.md", "after end");

        insert_test_diff_trace(&db, 999, "oc_before-cutoff", &before_cutoff_patch);
        insert_test_diff_trace(&db, 1000, "oc_at-cutoff", &cutoff_patch);
        insert_test_diff_trace(
            &db,
            1500,
            "oc_malformed",
            "Index: notes/malformed.md\n===================================================================\n--- notes/malformed.md\n+++ notes/malformed.md\n@@ malformed @@\n+bad\n",
        );
        insert_test_diff_trace(&db, 1500, "oc_same-time-a", &first_same_time_patch);
        insert_test_diff_trace(&db, 1500, "oc_same-time-b", &second_same_time_patch);
        insert_test_diff_trace(&db, 2000, "oc_at-end", &end_patch);
        insert_test_diff_trace(&db, 2001, "oc_after-end", &after_end_patch);

        let result = recent_diff_trace_patches_with(&db, 1000, 2000)
            .expect("recent diff trace patches should load");

        assert_eq!(result.loaded_count(), 4);
        assert_eq!(result.skipped_count(), 1);
        assert_eq!(
            result
                .patches
                .iter()
                .map(|patch| (patch.id, patch.time_ms, patch.session_id.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (2, 1000, "oc_at-cutoff"),
                (4, 1500, "oc_same-time-a"),
                (5, 1500, "oc_same-time-b"),
                (6, 2000, "oc_at-end"),
            ]
        );
        assert_eq!(
            result
                .patches
                .iter()
                .map(|patch| { (patch.tool_name.as_deref(), patch.tool_version.as_deref(),) })
                .collect::<Vec<_>>(),
            vec![
                (Some("opencode"), Some("1.2.3")),
                (Some("opencode"), Some("1.2.3")),
                (Some("opencode"), Some("1.2.3")),
                (Some("opencode"), Some("1.2.3")),
            ]
        );
        assert_eq!(
            result
                .patches
                .iter()
                .map(|patch| patch.payload_type.as_str())
                .collect::<Vec<_>>(),
            vec![
                PAYLOAD_TYPE_PATCH,
                PAYLOAD_TYPE_PATCH,
                PAYLOAD_TYPE_PATCH,
                PAYLOAD_TYPE_PATCH
            ]
        );
        assert_eq!(
            result
                .patches
                .iter()
                .map(|patch| patch.patch.files[0].new_path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "notes/cutoff.md",
                "notes/same-a.md",
                "notes/same-b.md",
                "notes/end.md",
            ]
        );
        assert_eq!(result.skipped[0].id, 3);
        assert_eq!(result.skipped[0].time_ms, 1500);
        assert_eq!(result.skipped[0].session_id, "oc_malformed");
        assert!(
            result.skipped[0].reason.contains("invalid hunk header"),
            "unexpected skipped reason: {}",
            result.skipped[0].reason
        );

        drop(db);
        if let Some(parent) = db_path.parent() {
            fs::remove_dir_all(parent).expect("test DB directory should be removed");
        }
    }
}
