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

use super::{
    insert_agent_trace_with, insert_diff_trace_with, insert_message_with, insert_messages_with,
    insert_part_with, insert_parts_with, insert_post_commit_patch_intersection_with,
    recent_diff_trace_patches_with, AgentTraceInsert, DiffTraceInsert, InsertMessageInsert,
    InsertPartInsert, PostCommitPatchIntersectionInsert, RecentDiffTracePatches,
};

const REPOSITORY_AGENT_TRACE_SCHEMA_SETUP_GUIDANCE: &str = "Run 'sce setup'.";

const SELECT_REPOSITORY_METADATA_SQL: &str =
    "SELECT repository_id FROM repository_metadata WHERE id = 1";
const SELECT_SQLITE_OBJECT_SQL: &str =
    "SELECT name FROM sqlite_master WHERE type = ?1 AND name = ?2 LIMIT 1";
const RECORD_REPOSITORY_SCHEMA_MIGRATION_SQL: &str =
    "INSERT OR IGNORE INTO __sce_migrations (id) VALUES ('001_repository_schema')";
const REQUIRED_REPOSITORY_SCHEMA_TABLES: &[&str] = &[
    "repository_metadata",
    "diff_traces",
    "post_commit_patch_intersections",
    "agent_traces",
    "messages",
    "parts",
];

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
    /// Open a repository-scoped Agent Trace database at an explicit path without
    /// running migrations, for read-only hook/runtime paths that must not
    /// migrate from a high-frequency caller.
    pub fn open_for_hooks_without_migrations_at(path: impl AsRef<std::path::Path>) -> Result<Self> {
        TursoDb::<RepositoryAgentTraceDbSpec>::open_without_migrations_at(path)
    }

    /// Verify that the repository-scoped schema baseline already exists.
    pub fn ensure_schema_ready_for_hooks(&self) -> Result<()> {
        self.ensure_schema_ready(REPOSITORY_AGENT_TRACE_SCHEMA_SETUP_GUIDANCE)
    }

    /// Repair the narrow concurrent-initialization case where the one-file
    /// schema batch completed but recording `__sce_migrations` raced with
    /// another first opener. This never creates trace tables; it only records
    /// the baseline migration after all required repository tables already
    /// exist.
    pub fn repair_missing_repository_schema_migration_metadata(&self) -> Result<()> {
        for table in REQUIRED_REPOSITORY_SCHEMA_TABLES {
            if !self.sqlite_object_exists("table", table)? {
                anyhow::bail!(
                    "repository Agent Trace DB schema is incomplete; missing table {table}. \
                     {REPOSITORY_AGENT_TRACE_SCHEMA_SETUP_GUIDANCE}"
                );
            }
        }

        self.execute(RECORD_REPOSITORY_SCHEMA_MIGRATION_SQL, ())?;
        self.ensure_schema_ready_for_hooks()
    }

    fn sqlite_object_exists(&self, object_type: &str, name: &str) -> Result<bool> {
        let rows = self.query_map(SELECT_SQLITE_OBJECT_SQL, (object_type, name), |row| {
            row.get::<String>(0).map_err(Into::into)
        })?;
        Ok(!rows.is_empty())
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

    /// Insert a diff trace payload into the repository-scoped `diff_traces`
    /// table. Rows remain repository-level; no checkout provenance is stored.
    pub fn insert_diff_trace(&self, input: DiffTraceInsert<'_>) -> Result<u64> {
        insert_diff_trace_with(self, input)
    }

    /// Insert a post-commit patch intersection into the repository-scoped
    /// `post_commit_patch_intersections` table.
    pub fn insert_post_commit_patch_intersection(
        &self,
        input: PostCommitPatchIntersectionInsert<'_>,
    ) -> Result<u64> {
        insert_post_commit_patch_intersection_with(self, input)
    }

    /// Insert a built Agent Trace payload into the repository-scoped
    /// `agent_traces` table.
    pub fn insert_agent_trace(&self, input: AgentTraceInsert<'_>) -> Result<u64> {
        insert_agent_trace_with(self, input)
    }

    /// Query and parse recent diff trace patches within the inclusive time
    /// window for this repository-scoped database. Rows remain repository-level;
    /// no checkout filter or checkout provenance is applied.
    pub fn recent_diff_trace_patches(
        &self,
        cutoff_time_ms: i64,
        end_time_ms: i64,
    ) -> Result<RecentDiffTracePatches> {
        recent_diff_trace_patches_with(self, cutoff_time_ms, end_time_ms)
    }

    /// Insert a message row, ignoring duplicate `(session_id, message_id)`
    /// rows.
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::services::agent_trace_db::{MessageRole, PartType, PAYLOAD_TYPE_PATCH};

    fn valid_patch(path: &str, content: &str) -> String {
        format!(
            "Index: {path}\n===================================================================\n--- {path}\n+++ {path}\n@@ -0,0 +1,1 @@\n+{content}\n"
        )
    }

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
    fn repository_scoped_write_methods_insert_all_agent_trace_rows() {
        let db_path = unique_test_db_path("writes");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        db.insert_diff_trace(DiffTraceInsert {
            time_ms: 1_000,
            session_id: "oc_session-1",
            patch: "Index: notes.md\n===================================================================\n--- notes.md\n+++ notes.md\n@@ -0,0 +1,1 @@\n+hello\n",
            model_id: Some("provider/model"),
            tool_name: "opencode",
            tool_version: Some("1.2.3"),
            payload_type: PAYLOAD_TYPE_PATCH,
        })
        .expect("diff trace insert should succeed");

        db.insert_post_commit_patch_intersection(PostCommitPatchIntersectionInsert {
            commit_id: "abc123",
            post_commit_time_ms: 2_000,
            recent_window_cutoff_ms: 1_000,
            recent_window_end_ms: 2_000,
            loaded_diff_trace_count: 1,
            skipped_diff_trace_count: 0,
            intersection_patch: "Index: notes.md\n===================================================================\n--- notes.md\n+++ notes.md\n@@ -0,0 +1,1 @@\n+hello\n",
        })
        .expect("post-commit intersection insert should succeed");

        db.insert_agent_trace(AgentTraceInsert {
            commit_id: "abc123",
            commit_time_ms: 2_000,
            trace_json: r#"{"id":"trace-1"}"#,
            agent_trace_id: "trace-1",
            url: "https://sce.crocoder.dev/agent-trace/trace-1",
            remote_url: "https://github.com/acme/widgets",
        })
        .expect("agent trace insert should succeed");

        db.insert_message(InsertMessageInsert {
            session_id: "oc_session-1".to_string(),
            message_id: "message-1".to_string(),
            role: MessageRole::Assistant,
            generated_at_unix_ms: 1_000,
        })
        .expect("message insert should succeed");
        db.insert_messages(vec![InsertMessageInsert {
            session_id: "oc_session-1".to_string(),
            message_id: "message-2".to_string(),
            role: MessageRole::User,
            generated_at_unix_ms: 1_001,
        }])
        .expect("batch message insert should succeed");

        db.insert_part(InsertPartInsert {
            part_type: PartType::Text,
            text: "hello".to_string(),
            session_id: "oc_session-1".to_string(),
            message_id: "message-1".to_string(),
            generated_at_unix_ms: 1_000,
        })
        .expect("part insert should succeed");
        db.insert_parts(vec![InsertPartInsert {
            part_type: PartType::Patch,
            text: "patch text".to_string(),
            session_id: "oc_session-1".to_string(),
            message_id: "message-2".to_string(),
            generated_at_unix_ms: 1_001,
        }])
        .expect("batch part insert should succeed");

        for (table, expected_count) in [
            ("diff_traces", 1_i64),
            ("post_commit_patch_intersections", 1),
            ("agent_traces", 1),
            ("messages", 2),
            ("parts", 2),
        ] {
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
    fn recent_diff_trace_reads_all_repository_rows_without_checkout_filter() {
        let db_path = unique_test_db_path("recent-repository-level");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        db.insert_diff_trace(DiffTraceInsert {
            time_ms: 999,
            session_id: "oc_before-cutoff",
            patch: &valid_patch("notes/before.md", "before"),
            model_id: Some("provider/model"),
            tool_name: "opencode",
            tool_version: Some("1.2.3"),
            payload_type: PAYLOAD_TYPE_PATCH,
        })
        .expect("before-cutoff diff trace insert should succeed");
        db.insert_diff_trace(DiffTraceInsert {
            time_ms: 1_000,
            session_id: "oc_checkout-a-session",
            patch: &valid_patch("notes/a.md", "same repository checkout a"),
            model_id: Some("provider/model-a"),
            tool_name: "opencode",
            tool_version: Some("1.2.3"),
            payload_type: PAYLOAD_TYPE_PATCH,
        })
        .expect("checkout-a diff trace insert should succeed");
        db.insert_diff_trace(DiffTraceInsert {
            time_ms: 1_500,
            session_id: "pi_checkout-b-session",
            patch: &valid_patch("notes/b.md", "same repository checkout b"),
            model_id: Some("provider/model-b"),
            tool_name: "pi",
            tool_version: None,
            payload_type: PAYLOAD_TYPE_PATCH,
        })
        .expect("checkout-b diff trace insert should succeed");
        db.insert_diff_trace(DiffTraceInsert {
            time_ms: 2_001,
            session_id: "oc_after-end",
            patch: &valid_patch("notes/after.md", "after"),
            model_id: Some("provider/model"),
            tool_name: "opencode",
            tool_version: Some("1.2.3"),
            payload_type: PAYLOAD_TYPE_PATCH,
        })
        .expect("after-end diff trace insert should succeed");

        let recent = db
            .recent_diff_trace_patches(1_000, 2_000)
            .expect("recent repository diff traces should load");

        assert_eq!(recent.loaded_count(), 2);
        assert_eq!(recent.skipped_count(), 0);
        assert_eq!(
            recent
                .patches
                .iter()
                .map(|patch| (
                    patch.time_ms,
                    patch.session_id.as_str(),
                    patch.tool_name.as_deref()
                ))
                .collect::<Vec<_>>(),
            vec![
                (1_000, "oc_checkout-a-session", Some("opencode")),
                (1_500, "pi_checkout-b-session", Some("pi")),
            ]
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn recent_diff_trace_reads_are_isolated_by_repository_db_path() {
        let first_db_path = unique_test_db_path("recent-repo-one");
        let second_db_path = unique_test_db_path("recent-repo-two");
        let first_db = RepositoryAgentTraceDb::new_at(&first_db_path)
            .expect("first repository DB should open");
        let second_db = RepositoryAgentTraceDb::new_at(&second_db_path)
            .expect("second repository DB should open");

        first_db
            .insert_diff_trace(DiffTraceInsert {
                time_ms: 1_000,
                session_id: "oc_first-repo",
                patch: &valid_patch("notes/first.md", "first repository"),
                model_id: Some("provider/first"),
                tool_name: "opencode",
                tool_version: Some("1.2.3"),
                payload_type: PAYLOAD_TYPE_PATCH,
            })
            .expect("first repository diff trace insert should succeed");
        second_db
            .insert_diff_trace(DiffTraceInsert {
                time_ms: 1_000,
                session_id: "oc_second-repo",
                patch: &valid_patch("notes/second.md", "second repository"),
                model_id: Some("provider/second"),
                tool_name: "opencode",
                tool_version: Some("1.2.3"),
                payload_type: PAYLOAD_TYPE_PATCH,
            })
            .expect("second repository diff trace insert should succeed");

        let first_recent = first_db
            .recent_diff_trace_patches(0, 2_000)
            .expect("first repository recent traces should load");
        let second_recent = second_db
            .recent_diff_trace_patches(0, 2_000)
            .expect("second repository recent traces should load");

        assert_eq!(first_recent.loaded_count(), 1);
        assert_eq!(second_recent.loaded_count(), 1);
        assert_eq!(first_recent.patches[0].session_id, "oc_first-repo");
        assert_eq!(second_recent.patches[0].session_id, "oc_second-repo");
        assert_ne!(
            first_recent.patches[0].session_id,
            second_recent.patches[0].session_id
        );

        remove_test_db(&first_db_path);
        remove_test_db(&second_db_path);
    }

    #[test]
    fn spec_path_constructor_is_rejected() {
        let error = RepositoryAgentTraceDbSpec::db_path()
            .expect_err("repository DBs must not have a canonical spec path");
        assert!(error.to_string().contains("explicit-path"));
    }
}
