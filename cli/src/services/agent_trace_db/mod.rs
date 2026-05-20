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
const CREATE_AGENT_TRACES_MIGRATION: &str =
    include_str!("../../../migrations/agent-trace/004_create_agent_traces.sql");
const ADD_DIFF_TRACES_MODEL_ID_MIGRATION: &str =
    include_str!("../../../migrations/agent-trace/005_add_diff_traces_model_id.sql");
const ADD_AGENT_TRACES_AGENT_TRACE_ID_MIGRATION: &str =
    include_str!("../../../migrations/agent-trace/006_add_agent_traces_agent_trace_id.sql");
const ADD_DIFF_TRACES_TOOL_METADATA_COLUMNS_MIGRATION: &str =
    include_str!("../../../migrations/agent-trace/007_add_diff_traces_tool_metadata_columns.sql");
const ADD_DIFF_TRACES_TOOL_VERSION_COLUMN_MIGRATION: &str =
    include_str!("../../../migrations/agent-trace/008_add_diff_traces_tool_version_column.sql");

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
    ("004_create_agent_traces", CREATE_AGENT_TRACES_MIGRATION),
    (
        "005_add_diff_traces_model_id",
        ADD_DIFF_TRACES_MODEL_ID_MIGRATION,
    ),
    (
        "006_add_agent_traces_agent_trace_id",
        ADD_AGENT_TRACES_AGENT_TRACE_ID_MIGRATION,
    ),
    (
        "007_add_diff_traces_tool_metadata_columns",
        ADD_DIFF_TRACES_TOOL_METADATA_COLUMNS_MIGRATION,
    ),
    (
        "008_add_diff_traces_tool_version_column",
        ADD_DIFF_TRACES_TOOL_VERSION_COLUMN_MIGRATION,
    ),
];

/// Parameterized SQL for inserting a captured diff trace payload.
pub const INSERT_DIFF_TRACE_SQL: &str =
    "INSERT INTO diff_traces (time_ms, session_id, patch, model_id, tool_name, tool_version) VALUES (?1, ?2, ?3, ?4, ?5, ?6)";

/// Parameterized SQL for retrieving recent captured diff trace patches.
pub const SELECT_RECENT_DIFF_TRACE_PATCHES_SQL: &str =
    "SELECT id, time_ms, session_id, patch, model_id
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
    "INSERT INTO agent_traces (commit_id, commit_time_ms, trace_json, agent_trace_id) VALUES (?1, ?2, ?3, ?4)";

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
    pub model_id: &'a str,
    pub tool_name: &'a str,
    pub tool_version: Option<&'a str>,
}

/// Raw diff trace row read from the agent trace database.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffTracePatchRow {
    pub id: i64,
    pub time_ms: i64,
    pub session_id: String,
    pub patch: String,
    pub model_id: Option<String>,
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

/// Built agent trace payload to persist in the agent trace database.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentTraceInsert<'a> {
    pub commit_id: &'a str,
    pub commit_time_ms: i64,
    pub trace_json: &'a str,
    pub agent_trace_id: &'a str,
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

    /// Insert a built agent trace payload into the `agent_traces` table.
    pub fn insert_agent_trace(&self, input: AgentTraceInsert<'_>) -> Result<u64> {
        self.execute(
            INSERT_AGENT_TRACE_SQL,
            (
                input.commit_id,
                input.commit_time_ms,
                input.trace_json,
                input.agent_trace_id,
            ),
        )
    }

    /// Query and parse recent diff trace patches within the inclusive time window.
    pub fn recent_diff_trace_patches(
        &self,
        cutoff_time_ms: i64,
        end_time_ms: i64,
    ) -> Result<RecentDiffTracePatches> {
        recent_diff_trace_patches_with(self, cutoff_time_ms, end_time_ms)
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
    })
}

fn parse_recent_diff_trace_patch_rows(rows: Vec<DiffTracePatchRow>) -> RecentDiffTracePatches {
    let mut patches = Vec::new();
    let mut skipped = Vec::new();

    for row in rows {
        match parse_patch(&row.patch) {
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
                });
            }
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::OnceLock,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    static TEST_DB_PATH: OnceLock<PathBuf> = OnceLock::new();
    static UPGRADE_TEST_DB_PATH: OnceLock<PathBuf> = OnceLock::new();

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
            AGENT_TRACE_MIGRATIONS
        }
    }

    struct LegacyAgentTraceDbSpec;

    impl DbSpec for LegacyAgentTraceDbSpec {
        fn db_name() -> &'static str {
            "legacy test agent trace DB"
        }

        fn db_path() -> Result<PathBuf> {
            UPGRADE_TEST_DB_PATH
                .get()
                .cloned()
                .context("upgrade test DB path should be initialized")
        }

        fn migrations() -> &'static [(&'static str, &'static str)] {
            &[("001_create_diff_traces", CREATE_DIFF_TRACES_MIGRATION)]
        }
    }

    struct UpgradedAgentTraceDbSpec;

    impl DbSpec for UpgradedAgentTraceDbSpec {
        fn db_name() -> &'static str {
            "upgraded test agent trace DB"
        }

        fn db_path() -> Result<PathBuf> {
            UPGRADE_TEST_DB_PATH
                .get()
                .cloned()
                .context("upgrade test DB path should be initialized")
        }

        fn migrations() -> &'static [(&'static str, &'static str)] {
            AGENT_TRACE_MIGRATIONS
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
                model_id: "test-provider/test-model",
                tool_name: "opencode",
                tool_version: Some("1.2.3"),
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
    fn new_applies_later_agent_trace_migrations_to_existing_database() {
        let db_path = unique_test_db_path();
        UPGRADE_TEST_DB_PATH
            .set(db_path.clone())
            .expect("upgrade test DB path should only be initialized once");

        {
            let legacy_db =
                TursoDb::<LegacyAgentTraceDbSpec>::new().expect("legacy DB should open");
            assert!(sqlite_object_exists(&legacy_db, "table", "diff_traces"));
            assert!(!sqlite_object_exists(
                &legacy_db,
                "table",
                "post_commit_patch_intersections"
            ));
            assert!(!sqlite_object_exists(
                &legacy_db,
                "index",
                "idx_diff_traces_time_ms_id"
            ));
        }

        {
            let upgraded_db =
                TursoDb::<UpgradedAgentTraceDbSpec>::new().expect("upgraded DB should open");

            assert!(sqlite_object_exists(&upgraded_db, "table", "diff_traces"));
            assert!(sqlite_object_exists(
                &upgraded_db,
                "table",
                "post_commit_patch_intersections"
            ));
            assert!(sqlite_object_exists(
                &upgraded_db,
                "index",
                "idx_diff_traces_time_ms_id"
            ));
            assert!(sqlite_object_exists(&upgraded_db, "table", "agent_traces"));
            assert_eq!(
                applied_migration_ids(&upgraded_db),
                vec![
                    "001_create_diff_traces",
                    "002_create_post_commit_patch_intersections",
                    "003_add_diff_traces_time_ms_id_index",
                    "004_create_agent_traces",
                    "005_add_diff_traces_model_id",
                    "006_add_agent_traces_agent_trace_id",
                    "007_add_diff_traces_tool_metadata_columns",
                    "008_add_diff_traces_tool_version_column",
                ]
            );
        }

        if let Some(parent) = db_path.parent() {
            fs::remove_dir_all(parent).expect("test DB directory should be removed");
        }
    }
}
