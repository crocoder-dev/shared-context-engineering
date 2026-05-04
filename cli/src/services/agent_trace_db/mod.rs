//! Agent trace Turso database adapter.

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::services::{
    db::{DbSpec, TursoDb},
    default_paths::agent_trace_db_path,
    patch::{parse_patch, ParseError, ParsedPatch},
};

pub mod lifecycle;

const CREATE_DIFF_TRACES_MIGRATION: &str =
    include_str!("../../../migrations/agent-trace/001_create_diff_traces.sql");
const CREATE_POST_COMMIT_PATCH_INTERSECTIONS_MIGRATION: &str =
    include_str!("../../../migrations/agent-trace/002_create_post_commit_patch_intersections.sql");
const ADD_DIFF_TRACES_TIME_MS_ID_INDEX_MIGRATION: &str =
    include_str!("../../../migrations/agent-trace/003_add_diff_traces_time_ms_id_index.sql");

const AGENT_TRACE_MIGRATIONS: &[(&str, &str)] = &[
    ("001_create_diff_traces", CREATE_DIFF_TRACES_MIGRATION),
    (
        "002_create_post_commit_patch_intersections",
        CREATE_POST_COMMIT_PATCH_INTERSECTIONS_MIGRATION,
    ),
    (
        "003_add_diff_traces_time_ms_id_index",
        ADD_DIFF_TRACES_TIME_MS_ID_INDEX_MIGRATION,
    ),
];

/// Parameterized SQL for inserting a captured diff trace payload.
pub const INSERT_DIFF_TRACE_SQL: &str =
    "INSERT INTO diff_traces (time_ms, session_id, patch) VALUES (?1, ?2, ?3)";

/// Parameterized SQL for retrieving recent captured diff trace patches.
pub const SELECT_RECENT_DIFF_TRACE_PATCHES_SQL: &str = "SELECT id, time_ms, session_id, patch
FROM diff_traces
WHERE time_ms >= ?1
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
        AGENT_TRACE_MIGRATIONS
    }
}

/// Agent trace Turso database adapter.
pub type AgentTraceDb = TursoDb<AgentTraceDbSpec>;

/// Diff trace payload to persist in the agent trace database.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiffTraceInsert<'a> {
    pub time_ms: i64,
    pub session_id: &'a str,
    pub patch: &'a str,
}

/// Raw diff trace row read from the agent trace database.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffTracePatchRow {
    pub id: i64,
    pub time_ms: i64,
    pub session_id: String,
    pub patch: String,
}

/// Parsed recent diff trace patch ready for comparison flows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedDiffTracePatch {
    pub id: i64,
    pub time_ms: i64,
    pub session_id: String,
    pub patch: ParsedPatch,
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

impl AgentTraceDb {
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

    /// Query and parse recent diff trace patches with `time_ms >= cutoff_time_ms`.
    pub fn recent_diff_trace_patches(&self, cutoff_time_ms: i64) -> Result<RecentDiffTracePatches> {
        recent_diff_trace_patches_with(self, cutoff_time_ms)
    }
}

fn insert_diff_trace_with<M: DbSpec>(db: &TursoDb<M>, input: DiffTraceInsert<'_>) -> Result<u64> {
    db.execute(
        INSERT_DIFF_TRACE_SQL,
        (input.time_ms, input.session_id, input.patch),
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

fn recent_diff_trace_patches_with<M: DbSpec>(
    db: &TursoDb<M>,
    cutoff_time_ms: i64,
) -> Result<RecentDiffTracePatches> {
    let rows = db.query_map(
        SELECT_RECENT_DIFF_TRACE_PATCHES_SQL,
        (cutoff_time_ms,),
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
    })
}

fn parse_recent_diff_trace_patch_rows(rows: Vec<DiffTracePatchRow>) -> RecentDiffTracePatches {
    let mut patches = Vec::new();
    let mut skipped = Vec::new();

    for row in rows {
        match parse_patch(&row.patch) {
            Ok(patch) => patches.push(ParsedDiffTracePatch {
                id: row.id,
                time_ms: row.time_ms,
                session_id: row.session_id,
                patch,
            }),
            Err(error) => skipped.push(SkippedDiffTracePatch {
                id: row.id,
                time_ms: row.time_ms,
                session_id: row.session_id,
                reason: skipped_diff_trace_patch_reason(&error),
            }),
        }
    }

    RecentDiffTracePatches { patches, skipped }
}

fn skipped_diff_trace_patch_reason(error: &ParseError) -> String {
    error.to_string()
}
