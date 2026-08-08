//! Agent Trace DWH database adapter.
//!
//! `AgentTraceDwhDb` is a dedicated explicit-path Turso adapter for the
//! append-oriented Agent Trace data warehouse destination schema
//! (`cli/migrations/agent-trace-dwh/`), separate from the repository-scoped
//! `agent-trace.db` source schema (`crate::services::agent_trace_db`). Like
//! the repository-scoped adapter, it has no canonical `DbSpec::db_path()`:
//! this plan explicitly excludes a local sync database and provisioning, so
//! there is no canonical production path yet, and callers must resolve an
//! explicit path themselves. This module owns schema initialization and
//! migration readiness only; it does not own a sync URL, credentials, ETL
//! state transitions, bridge locking, or CLI lifecycle behavior.

use std::path::PathBuf;

use anyhow::Result;

use crate::{
    generated_migrations,
    services::db::{DbSpec, TursoDb},
};

const AGENT_TRACE_DWH_SETUP_GUIDANCE: &str =
    "initialize the Agent Trace DWH database through its migration-running constructor";

/// Agent Trace DWH database configuration.
pub struct AgentTraceDwhDbSpec;

impl DbSpec for AgentTraceDwhDbSpec {
    fn db_name() -> &'static str {
        "Agent Trace DWH DB"
    }

    fn db_path() -> Result<PathBuf> {
        anyhow::bail!(
            "Agent Trace DWH DBs have no canonical spec path; resolve an explicit \
             path and use the explicit-path constructors"
        )
    }

    fn migrations() -> &'static [(&'static str, &'static str)] {
        generated_migrations::AGENT_TRACE_DWH_MIGRATIONS
    }

    fn db_config_key() -> &'static str {
        "agent_trace_db"
    }
}

/// Agent Trace DWH Turso database adapter.
pub type AgentTraceDwhDb = TursoDb<AgentTraceDwhDbSpec>;

impl AgentTraceDwhDb {
    /// Verify that the DWH schema baseline already exists and every embedded
    /// migration has been applied. Non-mutating.
    pub fn ensure_dwh_schema_ready(&self) -> Result<()> {
        TursoDb::ensure_schema_ready(self, AGENT_TRACE_DWH_SETUP_GUIDANCE)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn unique_test_db_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "sce-agent-trace-dwh-db-{label}-{}-{nonce}",
                std::process::id()
            ))
            .join("agent-trace-dwh.db")
    }

    fn remove_test_db(db_path: &std::path::Path) {
        if let Some(parent) = db_path.parent() {
            fs::remove_dir_all(parent).expect("test DB directory should be removed");
        }
    }

    fn sqlite_object_exists(db: &AgentTraceDwhDb, object_type: &str, name: &str) -> bool {
        let rows = db
            .query_map(
                "SELECT name FROM sqlite_master WHERE type = ?1 AND name = ?2",
                (object_type, name),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .expect("sqlite_master query should succeed");
        !rows.is_empty()
    }

    fn table_sql(db: &AgentTraceDwhDb, name: &str) -> String {
        db.query_map(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
            (name,),
            |row| row.get::<String>(0).map_err(Into::into),
        )
        .expect("sqlite_master sql query should succeed")
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("table '{name}' should exist"))
    }

    fn insert_repository_lineage(
        db: &AgentTraceDwhDb,
        repository_id: &str,
        source_instance_id: &str,
    ) {
        db.execute(
            "INSERT INTO repositories (repository_id) VALUES (?1)
             ON CONFLICT (repository_id) DO NOTHING",
            (repository_id,),
        )
        .expect("repository dimension insert should succeed");
        db.execute(
            "INSERT INTO source_instances (repository_id, source_instance_id) VALUES (?1, ?2)",
            (repository_id, source_instance_id),
        )
        .expect("source instance dimension insert should succeed");
    }

    fn insert_message(
        db: &AgentTraceDwhDb,
        repository_id: &str,
        source_instance_id: &str,
        session_id: &str,
        message_id: &str,
        generated_at_unix_ms: i64,
    ) -> Result<u64> {
        db.execute(
            "INSERT INTO messages (repository_id, source_instance_id, session_id, message_id, role, generated_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, 'user', ?5)",
            (
                repository_id,
                source_instance_id,
                session_id,
                message_id,
                generated_at_unix_ms,
            ),
        )
    }

    fn insert_message_part(
        db: &AgentTraceDwhDb,
        repository_id: &str,
        source_instance_id: &str,
        session_id: &str,
        message_id: &str,
        source_part_id: i64,
        generated_at_unix_ms: i64,
    ) -> Result<u64> {
        db.execute(
            "INSERT INTO message_parts (repository_id, source_instance_id, session_id, message_id, source_part_id, part_type, text, text_sha256, generated_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, 'text', 'hello', 'deadbeef', ?6)",
            (
                repository_id,
                source_instance_id,
                session_id,
                message_id,
                source_part_id,
                generated_at_unix_ms,
            ),
        )
    }

    fn insert_agent_trace(
        db: &AgentTraceDwhDb,
        repository_id: &str,
        source_instance_id: &str,
        agent_trace_id: &str,
    ) -> Result<u64> {
        db.execute(
            "INSERT INTO agent_traces (repository_id, source_instance_id, agent_trace_id, commit_id, commit_time_ms, trace_json, trace_json_sha256, url)
             VALUES (?1, ?2, ?3, 'commit-1', 1_000, '{}', 'deadbeef', 'https://sce.crocoder.dev/agent-trace/trace-1')",
            (repository_id, source_instance_id, agent_trace_id),
        )
    }

    fn insert_code_change(
        db: &AgentTraceDwhDb,
        repository_id: &str,
        source_instance_id: &str,
        source_diff_trace_id: i64,
    ) -> Result<u64> {
        db.execute(
            "INSERT INTO code_changes (repository_id, source_instance_id, source_diff_trace_id, session_id, time_ms, tool_name, payload_type, files_changed, lines_added, lines_removed, patch_sha256)
             VALUES (?1, ?2, ?3, 'session-1', 1_000, 'opencode', 'patch', 1, 1, 0, 'deadbeef')",
            (repository_id, source_instance_id, source_diff_trace_id),
        )
    }

    #[test]
    fn fresh_dwh_database_initializes_exactly_the_contract_tables_and_records_the_baseline() {
        let db_path = unique_test_db_path("baseline");
        let db = AgentTraceDwhDb::new_at(&db_path).expect("DWH DB should open");

        for table in [
            "repositories",
            "source_instances",
            "etl_watermarks",
            "messages",
            "message_parts",
            "agent_traces",
            "code_changes",
        ] {
            assert!(
                sqlite_object_exists(&db, "table", table),
                "table '{table}' should exist"
            );
        }

        let applied_ids = db
            .query_map(
                "SELECT id FROM __sce_migrations ORDER BY id ASC",
                (),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .expect("migration metadata query should succeed");
        assert_eq!(
            applied_ids,
            vec![String::from("001_dwh_schema")],
            "a fresh DWH DB should report the baseline migration as applied"
        );

        db.ensure_dwh_schema_ready()
            .expect("fresh DWH DB schema should be ready");

        remove_test_db(&db_path);
    }

    #[test]
    fn required_dwh_indexes_exist() {
        let db_path = unique_test_db_path("indexes");
        let db = AgentTraceDwhDb::new_at(&db_path).expect("DWH DB should open");

        for index in [
            "idx_dwh_repositories_repository_id",
            "idx_dwh_source_instances_repository_source",
            "idx_dwh_etl_watermarks_repository_source_table",
            "idx_dwh_messages_logical_identity",
            "idx_dwh_message_parts_source_identity",
            "idx_dwh_message_parts_order",
            "idx_dwh_agent_traces_logical_identity",
            "idx_dwh_code_changes_source_identity",
        ] {
            assert!(
                sqlite_object_exists(&db, "index", index),
                "index '{index}' should exist"
            );
        }

        remove_test_db(&db_path);
    }

    #[test]
    fn dwh_fact_tables_have_no_ingestion_order_foreign_keys() {
        let db_path = unique_test_db_path("no-fk");
        let db = AgentTraceDwhDb::new_at(&db_path).expect("DWH DB should open");

        for table in [
            "repositories",
            "source_instances",
            "etl_watermarks",
            "messages",
            "message_parts",
            "agent_traces",
            "code_changes",
        ] {
            let sql = table_sql(&db, table);
            assert!(
                !sql.to_uppercase().contains("REFERENCES"),
                "table '{table}' must not declare a foreign key: {sql}"
            );
        }

        remove_test_db(&db_path);
    }

    #[test]
    fn message_part_and_code_change_local_ids_coexist_across_source_instances_and_repositories() {
        let db_path = unique_test_db_path("coexisting-local-ids");
        let db = AgentTraceDwhDb::new_at(&db_path).expect("DWH DB should open");

        insert_repository_lineage(&db, "repo-a", "instance-a");
        insert_repository_lineage(&db, "repo-a", "instance-b");
        insert_repository_lineage(&db, "repo-b", "instance-c");

        // message_parts carries no foreign key to messages (no
        // ingestion-order constraint), so a part row for a given
        // session/message identity can be written independently of the
        // corresponding message row.
        //
        // The same local source_part_id (1) coexists across source instances
        // within the same repository, and across repositories.
        insert_message_part(
            &db,
            "repo-a",
            "instance-a",
            "session-1",
            "message-1",
            1,
            1_000,
        )
        .expect("part insert for instance-a should succeed");
        insert_message_part(
            &db,
            "repo-a",
            "instance-b",
            "session-1",
            "message-1",
            1,
            1_000,
        )
        .expect(
            "part insert with overlapping local id in a different source instance should succeed",
        );
        insert_message_part(
            &db,
            "repo-b",
            "instance-c",
            "session-1",
            "message-1",
            1,
            1_000,
        )
        .expect("part insert with overlapping local id in a different repository should succeed");

        // The same local source_diff_trace_id (1) coexists across source
        // instances within the same repository, and across repositories.
        insert_code_change(&db, "repo-a", "instance-a", 1)
            .expect("code change insert for instance-a should succeed");
        insert_code_change(&db, "repo-a", "instance-b", 1)
            .expect("code change insert with overlapping local id in a different source instance should succeed");
        insert_code_change(&db, "repo-b", "instance-c", 1).expect(
            "code change insert with overlapping local id in a different repository should succeed",
        );

        for (table, expected_count) in [("message_parts", 3_i64), ("code_changes", 3)] {
            let count = db
                .query_map(&format!("SELECT COUNT(*) FROM {table}"), (), |row| {
                    row.get::<i64>(0).map_err(Into::into)
                })
                .expect("count query should succeed")
                .into_iter()
                .next()
                .expect("count row should exist");
            assert_eq!(count, expected_count, "unexpected row count for {table}");
        }

        remove_test_db(&db_path);
    }

    #[test]
    fn duplicate_message_logical_identity_is_rejected_regardless_of_source_instance() {
        let db_path = unique_test_db_path("duplicate-message");
        let db = AgentTraceDwhDb::new_at(&db_path).expect("DWH DB should open");

        insert_repository_lineage(&db, "repo-a", "instance-a");
        insert_repository_lineage(&db, "repo-a", "instance-b");

        insert_message(&db, "repo-a", "instance-a", "session-1", "message-1", 1_000)
            .expect("first message insert should succeed");

        let same_instance_error =
            insert_message(&db, "repo-a", "instance-a", "session-1", "message-1", 2_000)
                .expect_err("duplicate message within the same source instance should fail");
        assert!(same_instance_error
            .to_string()
            .to_lowercase()
            .contains("unique"));

        let different_instance_error = insert_message(
            &db,
            "repo-a",
            "instance-b",
            "session-1",
            "message-1",
            2_000,
        )
        .expect_err(
            "duplicate message logical identity from a different source instance should still fail",
        );
        assert!(different_instance_error
            .to_string()
            .to_lowercase()
            .contains("unique"));

        remove_test_db(&db_path);
    }

    #[test]
    fn duplicate_agent_trace_logical_identity_is_rejected_regardless_of_source_instance() {
        let db_path = unique_test_db_path("duplicate-trace");
        let db = AgentTraceDwhDb::new_at(&db_path).expect("DWH DB should open");

        insert_repository_lineage(&db, "repo-a", "instance-a");
        insert_repository_lineage(&db, "repo-a", "instance-b");

        insert_agent_trace(&db, "repo-a", "instance-a", "trace-1")
            .expect("first agent trace insert should succeed");

        let different_instance_error = insert_agent_trace(&db, "repo-a", "instance-b", "trace-1")
            .expect_err(
                "duplicate agent trace logical identity from a different source instance should still fail",
            );
        assert!(different_instance_error
            .to_string()
            .to_lowercase()
            .contains("unique"));

        remove_test_db(&db_path);
    }

    #[test]
    fn watermarks_are_independently_keyed_by_repository_source_instance_and_source_table() {
        let db_path = unique_test_db_path("watermarks");
        let db = AgentTraceDwhDb::new_at(&db_path).expect("DWH DB should open");

        for (repository_id, source_instance_id, source_table) in [
            ("repo-a", "instance-a", "messages"),
            ("repo-a", "instance-a", "diff_traces"),
            ("repo-a", "instance-b", "messages"),
            ("repo-b", "instance-c", "messages"),
        ] {
            db.execute(
                "INSERT INTO etl_watermarks (repository_id, source_instance_id, source_table, last_extracted_source_row_id)
                 VALUES (?1, ?2, ?3, 0)",
                (repository_id, source_instance_id, source_table),
            )
            .unwrap_or_else(|_| panic!("watermark insert should succeed for {repository_id}/{source_instance_id}/{source_table}"));
        }

        let count = db
            .query_map("SELECT COUNT(*) FROM etl_watermarks", (), |row| {
                row.get::<i64>(0).map_err(Into::into)
            })
            .expect("count query should succeed")
            .into_iter()
            .next()
            .expect("count row should exist");
        assert_eq!(
            count, 4,
            "each repository/source/table triple should be independently keyed"
        );

        let duplicate_error = db
            .execute(
                "INSERT INTO etl_watermarks (repository_id, source_instance_id, source_table, last_extracted_source_row_id)
                 VALUES ('repo-a', 'instance-a', 'messages', 5)",
                (),
            )
            .expect_err("duplicate repository/source/table watermark should fail");
        assert!(duplicate_error
            .to_string()
            .to_lowercase()
            .contains("unique"));

        remove_test_db(&db_path);
    }

    #[test]
    fn equal_time_message_parts_query_deterministically_by_source_part_id() {
        let db_path = unique_test_db_path("deterministic-order");
        let db = AgentTraceDwhDb::new_at(&db_path).expect("DWH DB should open");

        insert_repository_lineage(&db, "repo-a", "instance-a");

        // Insert parts sharing the same generated_at_unix_ms in a
        // deliberately out-of-order sequence to prove ordering comes from the
        // index, not insertion order.
        insert_message_part(
            &db,
            "repo-a",
            "instance-a",
            "session-1",
            "message-1",
            3,
            1_000,
        )
        .expect("part 3 insert should succeed");
        insert_message_part(
            &db,
            "repo-a",
            "instance-a",
            "session-1",
            "message-1",
            1,
            1_000,
        )
        .expect("part 1 insert should succeed");
        insert_message_part(
            &db,
            "repo-a",
            "instance-a",
            "session-1",
            "message-1",
            2,
            1_000,
        )
        .expect("part 2 insert should succeed");

        let ordered_source_part_ids = db
            .query_map(
                "SELECT source_part_id FROM message_parts
                 WHERE repository_id = ?1 AND session_id = ?2 AND message_id = ?3
                 ORDER BY repository_id, session_id, message_id, generated_at_unix_ms, source_part_id",
                ("repo-a", "session-1", "message-1"),
                |row| row.get::<i64>(0).map_err(Into::into),
            )
            .expect("ordered query should succeed");

        assert_eq!(
            ordered_source_part_ids,
            vec![1, 2, 3],
            "equal-time parts should be ordered deterministically by source_part_id"
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn spec_path_constructor_is_rejected() {
        let error = AgentTraceDwhDbSpec::db_path()
            .expect_err("DWH DBs must not have a canonical spec path");
        assert!(error.to_string().contains("explicit-path"));
    }
}
