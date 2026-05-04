//! Agent trace Turso database adapter.
#![allow(dead_code)]

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::services::{
    db::{DbSpec, TursoDb},
    default_paths::agent_trace_db_path,
};

pub mod lifecycle;

const CREATE_DIFF_TRACES_MIGRATION: &str =
    include_str!("../../../migrations/agent-trace/001_create_diff_traces.sql");
const CREATE_PATCH_INTERSECTIONS_MIGRATION: &str =
    include_str!("../../../migrations/agent-trace/002_create_patch_intersections.sql");

const AGENT_TRACE_MIGRATIONS: &[(&str, &str)] = &[
    ("001_create_diff_traces", CREATE_DIFF_TRACES_MIGRATION),
    (
        "002_create_patch_intersections",
        CREATE_PATCH_INTERSECTIONS_MIGRATION,
    ),
];

/// Parameterized SQL for inserting a captured diff trace payload.
pub const INSERT_DIFF_TRACE_SQL: &str =
    "INSERT INTO diff_traces (time_ms, session_id, patch) VALUES (?1, ?2, ?3)";

/// Query for selecting the latest diff trace session by deterministic row order.
pub const SELECT_LATEST_DIFF_TRACE_SESSION_SQL: &str =
    "SELECT session_id FROM diff_traces ORDER BY time_ms DESC, id DESC LIMIT 1";

/// Query for loading all raw diff patches for one session in deterministic order.
pub const SELECT_DIFF_TRACE_PATCHES_FOR_SESSION_SQL: &str =
    "SELECT id, patch FROM diff_traces WHERE session_id = ?1 ORDER BY time_ms ASC, id ASC";

/// Parameterized SQL for inserting a patch intersection payload.
pub const INSERT_PATCH_INTERSECTION_SQL: &str = "INSERT INTO patch_intersections \
    (commit_sha, source_diff_trace_ids, intersection_json) \
    VALUES (?1, ?2, ?3)";

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

/// Raw diff trace row loaded for a selected session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffTracePatchRow {
    pub id: i64,
    pub patch: String,
}

/// Patch intersection payload to persist in the agent trace database.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PatchIntersectionInsert<'a> {
    pub commit_sha: &'a str,
    pub source_diff_trace_ids: &'a str,
    pub intersection_json: &'a str,
}

impl AgentTraceDb {
    /// Insert a diff trace payload into the `diff_traces` table.
    pub fn insert_diff_trace(&self, input: DiffTraceInsert<'_>) -> Result<u64> {
        self.execute(
            INSERT_DIFF_TRACE_SQL,
            (input.time_ms, input.session_id, input.patch),
        )
    }

    /// Return the latest diff trace session by `time_ms DESC, id DESC`.
    pub fn latest_diff_trace_session_id(&self) -> Result<Option<String>> {
        let sessions = self.query_map(SELECT_LATEST_DIFF_TRACE_SESSION_SQL, (), |row| {
            row.get(0)
                .context("failed to decode latest diff trace session_id")
        })?;

        Ok(sessions.into_iter().next())
    }

    /// Load all raw diff patches for one session by `time_ms ASC, id ASC`.
    pub fn diff_trace_patches_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<DiffTracePatchRow>> {
        self.query_map(
            SELECT_DIFF_TRACE_PATCHES_FOR_SESSION_SQL,
            (session_id,),
            |row| {
                Ok(DiffTracePatchRow {
                    id: row.get(0).context("failed to decode diff trace id")?,
                    patch: row.get(1).context("failed to decode diff trace patch")?,
                })
            },
        )
    }

    /// Insert a patch intersection payload into the `patch_intersections` table.
    pub fn insert_patch_intersection(&self, input: PatchIntersectionInsert<'_>) -> Result<u64> {
        self.execute(
            INSERT_PATCH_INTERSECTION_SQL,
            (
                input.commit_sha,
                input.source_diff_trace_ids,
                input.intersection_json,
            ),
        )
    }
}
