//! Agent trace Turso database adapter.

use std::path::PathBuf;

use anyhow::{Context, Result};
use turso::Value as TursoValue;

use crate::{
    generated_migrations,
    services::{
        db::{DbSpec, TursoDb},
        default_paths::agent_trace_db_path,
        patch::{parse_patch, ParseError, ParsedPatch},
        structured_patch::{derive_claude_structured_patch, ClaudeStructuredPatchDerivationResult},
    },
};

use serde_json::Value;

pub mod lifecycle;

const AGENT_TRACE_SCHEMA_SETUP_GUIDANCE: &str = "Run 'sce setup'.";

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
#[allow(dead_code)]
pub const INSERT_MESSAGE_SQL: &str =
    "INSERT INTO messages (session_id, message_id, role, generated_at_unix_ms)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT (session_id, message_id) DO NOTHING";

/// Parameterized SQL for inserting a part row (append-only, no upsert).
#[allow(dead_code)]
pub const INSERT_PART_SQL: &str =
    "INSERT INTO parts (type, text, message_id, session_id, generated_at_unix_ms)
VALUES (?1, ?2, ?3, ?4, ?5)";

/// Parameterized SQL for upserting editor session model attribution.
pub const UPSERT_SESSION_MODEL_SQL: &str = "INSERT INTO session_models (
    tool_name,
    session_id,
    model_id,
    tool_version,
    session_start_time_ms
) VALUES (?1, ?2, ?3, ?4, ?5)
ON CONFLICT(tool_name, session_id) DO UPDATE SET
    model_id = excluded.model_id,
    tool_version = excluded.tool_version,
    session_start_time_ms = excluded.session_start_time_ms,
    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

/// Parameterized SQL for retrieving editor session model attribution.
pub const SELECT_SESSION_MODEL_SQL: &str = "SELECT
    tool_name,
    session_id,
    model_id,
    tool_version,
    session_start_time_ms
FROM session_models
WHERE tool_name = ?1 AND session_id = ?2
LIMIT 1";

/// Agent trace database configuration.
pub struct AgentTraceDbSpec;

impl DbSpec for AgentTraceDbSpec {
    fn db_name() -> &'static str {
        "agent trace DB"
    }

    fn db_path() -> Result<PathBuf> {
        agent_trace_db_path()
    }

    fn migrations() -> &'static [(&'static str, &'static str)] {
        generated_migrations::AGENT_TRACE_MIGRATIONS
    }

    fn db_config_key() -> &'static str {
        "agent_trace_db"
    }
}

/// Agent trace Turso database adapter.
pub type AgentTraceDb = TursoDb<AgentTraceDbSpec>;

impl AgentTraceDb {
    /// Open or create an Agent Trace database at an explicit path and run all
    /// embedded migrations.
    pub fn open_at(path: impl AsRef<std::path::Path>) -> Result<Self> {
        TursoDb::<AgentTraceDbSpec>::new_at(path)
    }

    /// Open or create an Agent Trace database at an explicit path without
    /// running migrations.
    pub fn open_for_hooks_without_migrations_at(path: impl AsRef<std::path::Path>) -> Result<Self> {
        TursoDb::<AgentTraceDbSpec>::open_without_migrations_at(path)
    }
}

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

/// Session model attribution payload to upsert into the agent trace database.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionModelUpsert<'a> {
    pub tool_name: &'a str,
    pub session_id: &'a str,
    pub model_id: &'a str,
    pub tool_version: Option<&'a str>,
    pub session_start_time_ms: i64,
}

/// Durable session model attribution row read from the agent trace database.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionModelAttribution {
    pub tool_name: String,
    pub session_id: String,
    pub model_id: String,
    pub tool_version: Option<String>,
    pub session_start_time_ms: i64,
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
#[allow(dead_code)]
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
#[allow(dead_code)]
pub struct InsertMessageInsert {
    pub session_id: String,
    pub message_id: String,
    pub role: MessageRole,
    pub generated_at_unix_ms: i64,
}

/// Part type constraint for the `parts` table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
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
#[allow(dead_code)]
pub struct InsertPartInsert {
    pub part_type: PartType,
    pub text: String,
    pub session_id: String,
    pub message_id: String,
    pub generated_at_unix_ms: i64,
}

impl AgentTraceDb {
    /// Open the Agent Trace DB for high-frequency hook runtime paths without
    /// running embedded migrations.
    ///
    /// Setup/lifecycle initialization must continue to use [`AgentTraceDb::new`]
    /// so schema migrations remain explicitly owned by setup flows. Hook callers
    /// must verify schema readiness before reading or writing through this DB.
    #[allow(dead_code)]
    pub fn open_for_hooks_without_migrations() -> Result<Self> {
        TursoDb::<AgentTraceDbSpec>::open_without_migrations()
    }

    /// Verify that the Agent Trace DB schema needed by hook runtime readers and
    /// writers already exists.
    ///
    /// This check is intentionally non-mutating. Missing or incomplete schema is
    /// reported with setup guidance instead of running migrations from a
    /// high-frequency hook path.
    pub fn ensure_schema_ready_for_hooks(&self) -> Result<()> {
        self.ensure_schema_ready(AGENT_TRACE_SCHEMA_SETUP_GUIDANCE)
    }

    /// Insert a diff trace payload into the `diff_traces` table.
    pub fn insert_diff_trace(&self, input: DiffTraceInsert<'_>) -> Result<u64> {
        insert_diff_trace_with(self, input)
    }

    /// Insert a post-commit patch intersection result into the
    /// `post_commit_patch_intersections` table.
    pub fn insert_post_commit_patch_intersection(
        &self,
        input: PostCommitPatchIntersectionInsert<'_>,
    ) -> Result<u64> {
        insert_post_commit_patch_intersection_with(self, input)
    }

    /// Insert a built agent trace payload into the `agent_traces` table.
    pub fn insert_agent_trace(&self, input: AgentTraceInsert<'_>) -> Result<u64> {
        insert_agent_trace_with(self, input)
    }

    /// Upsert editor session model attribution into the `session_models` table.
    pub fn upsert_session_model(&self, input: SessionModelUpsert<'_>) -> Result<u64> {
        upsert_session_model_with(self, input)
    }

    /// Retrieve editor session model attribution by `(tool_name, session_id)`.
    pub fn session_model_by_tool_and_session(
        &self,
        tool_name: &str,
        session_id: &str,
    ) -> Result<Option<SessionModelAttribution>> {
        session_model_by_tool_and_session_with(self, tool_name, session_id)
    }

    /// Query and parse recent diff trace patches within the inclusive time window.
    pub fn recent_diff_trace_patches(
        &self,
        cutoff_time_ms: i64,
        end_time_ms: i64,
    ) -> Result<RecentDiffTracePatches> {
        recent_diff_trace_patches_with(self, cutoff_time_ms, end_time_ms)
    }

    /// Insert a message row, ignoring duplicate `(session_id, message_id)` rows.
    #[allow(dead_code)]
    pub fn insert_message(&self, input: InsertMessageInsert) -> Result<u64> {
        insert_message_with(self, input)
    }

    /// Insert message rows with one multi-row statement, ignoring duplicate
    /// `(session_id, message_id)` rows.
    pub fn insert_messages(&self, inputs: Vec<InsertMessageInsert>) -> Result<u64> {
        insert_messages_with(self, inputs)
    }

    /// Append a part row (no upsert; multiple rows per message allowed).
    #[allow(dead_code)]
    pub fn insert_part(&self, input: InsertPartInsert) -> Result<u64> {
        insert_part_with(self, input)
    }

    /// Append part rows with one multi-row statement.
    pub fn insert_parts(&self, inputs: Vec<InsertPartInsert>) -> Result<u64> {
        insert_parts_with(self, inputs)
    }
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

fn upsert_session_model_with<M: DbSpec>(
    db: &TursoDb<M>,
    input: SessionModelUpsert<'_>,
) -> Result<u64> {
    db.execute(
        UPSERT_SESSION_MODEL_SQL,
        (
            input.tool_name,
            input.session_id,
            input.model_id,
            input.tool_version,
            input.session_start_time_ms,
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

fn session_model_by_tool_and_session_with<M: DbSpec>(
    db: &TursoDb<M>,
    tool_name: &str,
    session_id: &str,
) -> Result<Option<SessionModelAttribution>> {
    let rows = db.query_map(
        SELECT_SESSION_MODEL_SQL,
        (tool_name, session_id),
        session_model_attribution_from_turso,
    )?;

    Ok(rows.into_iter().next())
}

fn session_model_attribution_from_turso(row: &turso::Row) -> Result<SessionModelAttribution> {
    Ok(SessionModelAttribution {
        tool_name: row
            .get(0)
            .context("failed to read session_models.tool_name")?,
        session_id: row
            .get(1)
            .context("failed to read session_models.session_id")?,
        model_id: row
            .get(2)
            .context("failed to read session_models.model_id")?,
        tool_version: row
            .get(3)
            .context("failed to read session_models.tool_version")?,
        session_start_time_ms: row
            .get(4)
            .context("failed to read session_models.session_start_time_ms")?,
    })
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
        sync::OnceLock,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::services::agent_trace;

    static TEST_DB_PATH: OnceLock<PathBuf> = OnceLock::new();
    static BASELINE_TEST_DB_PATH: OnceLock<PathBuf> = OnceLock::new();

    struct TestAgentTraceDbSpec;

    impl DbSpec for TestAgentTraceDbSpec {
        fn db_name() -> &'static str {
            "test agent trace DB"
        }

        fn db_path() -> Result<PathBuf> {
            TEST_DB_PATH
                .get()
                .cloned()
                .context("test DB path should be initialized")
        }

        fn migrations() -> &'static [(&'static str, &'static str)] {
            generated_migrations::AGENT_TRACE_MIGRATIONS
        }

        fn db_config_key() -> &'static str {
            "agent_trace_db"
        }
    }

    struct BaselineAgentTraceDbSpec;

    impl DbSpec for BaselineAgentTraceDbSpec {
        fn db_name() -> &'static str {
            "baseline test agent trace DB"
        }

        fn db_path() -> Result<PathBuf> {
            BASELINE_TEST_DB_PATH
                .get()
                .cloned()
                .context("baseline test DB path should be initialized")
        }

        fn migrations() -> &'static [(&'static str, &'static str)] {
            generated_migrations::AGENT_TRACE_MIGRATIONS
        }

        fn db_config_key() -> &'static str {
            "agent_trace_db"
        }
    }

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
        db: &TursoDb<TestAgentTraceDbSpec>,
        time_ms: i64,
        session_id: &str,
        patch: &str,
    ) {
        insert_diff_trace_with(
            db,
            DiffTraceInsert {
                time_ms,
                session_id,
                patch,
                model_id: Some("test-provider/test-model"),
                tool_name: "opencode",
                tool_version: Some("1.2.3"),
                payload_type: PAYLOAD_TYPE_PATCH,
            },
        )
        .expect("diff trace insert should succeed");
    }

    fn sqlite_object_exists<M: DbSpec>(db: &TursoDb<M>, object_type: &str, name: &str) -> bool {
        let rows = db
            .query_map(
                "SELECT name FROM sqlite_master WHERE type = ?1 AND name = ?2",
                (object_type, name),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .expect("sqlite_master query should succeed");
        !rows.is_empty()
    }

    fn applied_migration_ids<M: DbSpec>(db: &TursoDb<M>) -> Vec<String> {
        db.query_map(
            "SELECT id FROM __sce_migrations ORDER BY id ASC",
            (),
            |row| row.get::<String>(0).map_err(Into::into),
        )
        .expect("migration metadata query should succeed")
    }

    #[test]
    fn recent_diff_trace_patches_applies_bounded_window_ordering_and_parse_accounting() {
        let db_path = unique_test_db_path();
        TEST_DB_PATH
            .set(db_path.clone())
            .expect("test DB path should only be initialized once");
        let db = TursoDb::<TestAgentTraceDbSpec>::new().expect("test DB should open");

        let before_cutoff_patch = valid_patch("notes/before.md", "before cutoff");
        let cutoff_patch = valid_patch("notes/cutoff.md", "at cutoff");
        let first_same_time_patch = valid_patch("notes/same-a.md", "same time first");
        let second_same_time_patch = valid_patch("notes/same-b.md", "same time second");
        let end_patch = valid_patch("notes/end.md", "at end");
        let after_end_patch = valid_patch("notes/after.md", "after end");

        insert_test_diff_trace(&db, 999, "before-cutoff", &before_cutoff_patch);
        insert_test_diff_trace(&db, 1000, "at-cutoff", &cutoff_patch);
        insert_test_diff_trace(
            &db,
            1500,
            "malformed",
            "Index: notes/malformed.md\n===================================================================\n--- notes/malformed.md\n+++ notes/malformed.md\n@@ malformed @@\n+bad\n",
        );
        insert_test_diff_trace(&db, 1500, "same-time-a", &first_same_time_patch);
        insert_test_diff_trace(&db, 1500, "same-time-b", &second_same_time_patch);
        insert_test_diff_trace(&db, 2000, "at-end", &end_patch);
        insert_test_diff_trace(&db, 2001, "after-end", &after_end_patch);

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
                (2, 1000, "at-cutoff"),
                (4, 1500, "same-time-a"),
                (5, 1500, "same-time-b"),
                (6, 2000, "at-end"),
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
        assert_eq!(result.skipped[0].session_id, "malformed");
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

    #[test]
    fn new_applies_baseline_agent_trace_migration_and_indexes() {
        let db_path = unique_test_db_path();
        BASELINE_TEST_DB_PATH
            .set(db_path.clone())
            .expect("baseline test DB path should only be initialized once");

        let db = TursoDb::<BaselineAgentTraceDbSpec>::new().expect("baseline test DB should open");

        assert!(sqlite_object_exists(&db, "table", "diff_traces"));
        assert!(sqlite_object_exists(
            &db,
            "table",
            "post_commit_patch_intersections"
        ));
        assert!(sqlite_object_exists(&db, "table", "agent_traces"));
        assert!(sqlite_object_exists(
            &db,
            "index",
            "idx_diff_traces_time_ms_id"
        ));
        assert!(sqlite_object_exists(
            &db,
            "index",
            "idx_agent_traces_agent_trace_id"
        ));
        assert!(sqlite_object_exists(
            &db,
            "index",
            "idx_agent_traces_remote_url"
        ));
        assert!(sqlite_object_exists(&db, "table", "messages"));
        assert!(sqlite_object_exists(&db, "table", "parts"));
        assert!(sqlite_object_exists(&db, "table", "session_models"));
        assert!(sqlite_object_exists(
            &db,
            "index",
            "idx_messages_session_message"
        ));
        assert!(sqlite_object_exists(
            &db,
            "index",
            "idx_messages_session_order"
        ));
        assert!(sqlite_object_exists(
            &db,
            "index",
            "idx_parts_session_message_order"
        ));
        assert!(sqlite_object_exists(
            &db,
            "trigger",
            "trg_messages_updated_at"
        ));
        assert!(sqlite_object_exists(&db, "trigger", "trg_parts_updated_at"));
        let applied_ids = applied_migration_ids(&db);
        assert_eq!(
            applied_ids.len(),
            generated_migrations::AGENT_TRACE_MIGRATIONS.len(),
            "applied migration count should match generated migration count"
        );
        assert!(
            applied_ids.windows(2).all(|w| w[0] < w[1]),
            "applied migration IDs should be sorted ascending: {applied_ids:?}"
        );
        for id in &applied_ids {
            assert!(
                id.len() > 4
                    && id.chars().take(3).all(|c| c.is_ascii_digit())
                    && id.chars().nth(3) == Some('_'),
                "migration ID '{id}' should match NNN_... pattern"
            );
        }

        let trace_url = agent_trace::agent_trace_persisted_url("trace-1");

        let duplicate_insert = insert_agent_trace_with(
            &db,
            AgentTraceInsert {
                commit_id: "abc123",
                commit_time_ms: 123,
                trace_json: r#"{"id":"trace-1"}"#,
                agent_trace_id: "trace-1",
                url: &trace_url,
                remote_url: "https://github.com/test/repo",
            },
        );
        assert!(duplicate_insert.is_ok());

        let duplicate_insert = insert_agent_trace_with(
            &db,
            AgentTraceInsert {
                commit_id: "abc124",
                commit_time_ms: 124,
                trace_json: r#"{"id":"trace-1"}"#,
                agent_trace_id: "trace-1",
                url: &trace_url,
                remote_url: "https://github.com/test/repo",
            },
        );
        assert!(duplicate_insert.is_err());

        if let Some(parent) = db_path.parent() {
            fs::remove_dir_all(parent).expect("test DB directory should be removed");
        }
    }
}
