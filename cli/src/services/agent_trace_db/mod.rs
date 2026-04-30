//! Agent trace Turso database adapter.
#![allow(dead_code)]

use std::path::PathBuf;

use anyhow::Result;

use crate::services::{
    db::{DbSpec, TursoDb},
    default_paths::agent_trace_db_path,
};

pub mod lifecycle;

const CREATE_DIFF_TRACES_MIGRATION: &str =
    include_str!("../../../migrations/agent-trace/001_create_diff_traces.sql");

const AGENT_TRACE_MIGRATIONS: &[(&str, &str)] =
    &[("001_create_diff_traces", CREATE_DIFF_TRACES_MIGRATION)];

/// Parameterized SQL for inserting a captured diff trace payload.
pub const INSERT_DIFF_TRACE_SQL: &str =
    "INSERT INTO diff_traces (time_ms, session_id, patch) VALUES (?1, ?2, ?3)";

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

impl AgentTraceDb {
    /// Insert a diff trace payload into the `diff_traces` table.
    pub fn insert_diff_trace(&self, input: DiffTraceInsert<'_>) -> Result<u64> {
        self.execute(
            INSERT_DIFF_TRACE_SQL,
            (input.time_ms, input.session_id, input.patch),
        )
    }
}
