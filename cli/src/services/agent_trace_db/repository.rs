//! Repository-scoped Agent Trace database adapter.
//!
//! One logical Git repository maps to one database at
//! `<state-root>/sce/repos/<repository-id>/agent-trace.db`. The schema
//! baseline is one fresh schema SQL file (`agent-trace-repository`
//! migrations), because repository-scoped databases are always created new;
//! there is no incremental chain and no migration path from legacy
//! checkout-scoped databases. Trace tables carry no `checkout_id` columns.

use std::path::PathBuf;

use anyhow::Result;

use crate::{
    generated_migrations,
    services::db::{DbSpec, TursoDb},
};

const REPOSITORY_AGENT_TRACE_SCHEMA_SETUP_GUIDANCE: &str = "Run 'sce setup'.";

const SELECT_REPOSITORY_METADATA_SQL: &str =
    "SELECT repository_id FROM repository_metadata WHERE id = 1";

/// Seeds the single metadata row on first initialization; concurrent first
/// opens race safely because the conflicting insert is ignored and the stored
/// value is validated afterwards.
const INSERT_REPOSITORY_METADATA_SQL: &str =
    "INSERT INTO repository_metadata (id, repository_id) VALUES (1, ?1)
ON CONFLICT (id) DO NOTHING";

/// Repository-scoped Agent Trace database configuration.
pub struct RepositoryAgentTraceDbSpec;

impl DbSpec for RepositoryAgentTraceDbSpec {
    fn db_name() -> &'static str {
        "repository Agent Trace DB"
    }

    fn db_path() -> Result<PathBuf> {
        anyhow::bail!(
            "repository Agent Trace DBs have no canonical spec path; resolve the \
             repository-scoped path and use the explicit-path constructors"
        )
    }

    fn migrations() -> &'static [(&'static str, &'static str)] {
        generated_migrations::AGENT_TRACE_REPOSITORY_MIGRATIONS
    }

    fn db_config_key() -> &'static str {
        "agent_trace_db"
    }
}

/// Repository-scoped Agent Trace Turso database adapter.
pub type RepositoryAgentTraceDb = TursoDb<RepositoryAgentTraceDbSpec>;

impl RepositoryAgentTraceDb {
    /// Verify that the repository-scoped schema baseline already exists.
    pub fn ensure_schema_ready_for_hooks(&self) -> Result<()> {
        self.ensure_schema_ready(REPOSITORY_AGENT_TRACE_SCHEMA_SETUP_GUIDANCE)
    }

    /// Seed repository metadata on first initialization and validate it on
    /// every open.
    ///
    /// The stored `repository_id` must match the resolved repository ID for
    /// this database path; a mismatch means the file does not belong to the
    /// resolved repository and is an error rather than a write target.
    pub fn verify_or_initialize_repository_metadata(&self, repository_id: &str) -> Result<()> {
        self.execute(INSERT_REPOSITORY_METADATA_SQL, (repository_id,))?;

        let stored = self.query_map(SELECT_REPOSITORY_METADATA_SQL, (), |row| {
            row.get::<String>(0).map_err(Into::into)
        })?;

        match stored.first() {
            Some(stored_repository_id) if stored_repository_id == repository_id => Ok(()),
            Some(stored_repository_id) => anyhow::bail!(
                "repository Agent Trace DB metadata mismatch: stored repository ID \
                 {stored_repository_id} does not match resolved repository ID {repository_id}"
            ),
            None => anyhow::bail!(
                "repository Agent Trace DB metadata is missing its repository ID row. \
                 {REPOSITORY_AGENT_TRACE_SCHEMA_SETUP_GUIDANCE}"
            ),
        }
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
                "sce-repo-agent-trace-db-{label}-{}-{nonce}",
                std::process::id()
            ))
            .join("agent-trace.db")
    }

    fn remove_test_db(db_path: &std::path::Path) {
        if let Some(parent) = db_path.parent() {
            fs::remove_dir_all(parent).expect("test DB directory should be removed");
        }
    }

    fn sqlite_object_exists(db: &RepositoryAgentTraceDb, object_type: &str, name: &str) -> bool {
        let rows = db
            .query_map(
                "SELECT name FROM sqlite_master WHERE type = ?1 AND name = ?2",
                (object_type, name),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .expect("sqlite_master query should succeed");
        !rows.is_empty()
    }

    fn table_sql(db: &RepositoryAgentTraceDb, name: &str) -> String {
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

    #[test]
    fn open_at_initializes_the_full_schema_from_one_migration() {
        let db_path = unique_test_db_path("baseline");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        for table in [
            "repository_metadata",
            "diff_traces",
            "post_commit_patch_intersections",
            "agent_traces",
            "messages",
            "parts",
        ] {
            assert!(
                sqlite_object_exists(&db, "table", table),
                "table '{table}' should exist"
            );
        }
        for index in [
            "idx_diff_traces_time_ms_id",
            "idx_agent_traces_agent_trace_id",
            "idx_agent_traces_remote_url",
            "idx_messages_session_message",
            "idx_messages_session_order",
            "idx_parts_session_message_order",
        ] {
            assert!(
                sqlite_object_exists(&db, "index", index),
                "index '{index}' should exist"
            );
        }
        for trigger in ["trg_messages_updated_at", "trg_parts_updated_at"] {
            assert!(
                sqlite_object_exists(&db, "trigger", trigger),
                "trigger '{trigger}' should exist"
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
            vec![String::from("001_repository_schema")],
            "repository DBs should be initialized from exactly one schema file"
        );

        db.ensure_schema_ready_for_hooks()
            .expect("fresh repository DB schema should be ready");

        remove_test_db(&db_path);
    }

    #[test]
    fn trace_tables_have_no_checkout_id_columns() {
        let db_path = unique_test_db_path("no-checkout-id");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        for table in [
            "diff_traces",
            "post_commit_patch_intersections",
            "agent_traces",
            "messages",
            "parts",
        ] {
            let sql = table_sql(&db, table);
            assert!(
                !sql.contains("checkout_id"),
                "table '{table}' must not have a checkout_id column: {sql}"
            );
        }

        remove_test_db(&db_path);
    }

    #[test]
    fn repository_metadata_is_seeded_once_and_validated_on_reopen() {
        let db_path = unique_test_db_path("metadata");
        let repository_id = "a".repeat(64);

        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        db.verify_or_initialize_repository_metadata(&repository_id)
            .expect("first metadata initialization should succeed");
        db.verify_or_initialize_repository_metadata(&repository_id)
            .expect("repeated validation with the same repository ID should succeed");
        drop(db);

        let reopened = RepositoryAgentTraceDb::open_without_migrations_at(&db_path)
            .expect("repository DB should reopen");
        reopened
            .verify_or_initialize_repository_metadata(&repository_id)
            .expect("reopen validation with the matching repository ID should succeed");

        remove_test_db(&db_path);
    }

    #[test]
    fn mismatched_repository_metadata_errors_on_open() {
        let db_path = unique_test_db_path("mismatch");
        let stored_repository_id = "a".repeat(64);
        let other_repository_id = "b".repeat(64);

        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        db.verify_or_initialize_repository_metadata(&stored_repository_id)
            .expect("first metadata initialization should succeed");

        let error = db
            .verify_or_initialize_repository_metadata(&other_repository_id)
            .expect_err("mismatched repository ID should fail validation");
        let message = error.to_string();
        assert!(
            message.contains("metadata mismatch"),
            "unexpected error: {message}"
        );
        assert!(message.contains(&stored_repository_id));
        assert!(message.contains(&other_repository_id));

        remove_test_db(&db_path);
    }

    #[test]
    fn spec_path_constructor_is_rejected() {
        let error = RepositoryAgentTraceDbSpec::db_path()
            .expect_err("repository DBs must not have a canonical spec path");
        assert!(error.to_string().contains("explicit-path"));
    }
}
