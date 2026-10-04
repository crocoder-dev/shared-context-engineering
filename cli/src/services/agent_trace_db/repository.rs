use std::path::PathBuf;

use anyhow::Result;
use uuid::Uuid;

use crate::{
    generated_migrations,
    services::db::{DbSpec, TursoDb},
};

use super::{
    insert_agent_trace_with, insert_conversation_text_event_with, insert_diff_trace_with,
    insert_message_with, insert_messages_with, insert_part_with, insert_parts_with,
    insert_post_commit_patch_intersection_with, recent_diff_trace_patches_with,
    upsert_claude_model_state_with, AgentTraceInsert, ClaudeModelStateObservation, DiffTraceInsert,
    InsertMessageInsert, InsertPartInsert, PostCommitPatchIntersectionInsert,
    RecentDiffTracePatches,
};

const REPOSITORY_AGENT_TRACE_SCHEMA_SETUP_GUIDANCE: &str = "Run 'sce setup'.";

const SELECT_REPOSITORY_METADATA_SQL: &str =
    "SELECT repository_id, source_instance_id FROM repository_metadata WHERE id = 1";
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

/// Atomically claims `source_instance_id` for the single metadata row: it
/// only ever replaces an empty placeholder, so a losing racer's candidate is
/// silently discarded (the `UPDATE` affects zero rows) and an already-valid
/// stored value is never overwritten.
const CLAIM_SOURCE_INSTANCE_ID_SQL: &str =
    "UPDATE repository_metadata SET source_instance_id = ?1 WHERE id = 1 AND source_instance_id = ''";

/// Physical database identity for one repository-scoped Agent Trace DB,
/// alongside the existing logical repository identity.
///
/// `repository_id` identifies the logical Git repository this database is
/// scoped to. `source_instance_id` identifies this one physical database
/// file's lineage: it is generated once per physical database, independent
/// of `repository_id`, remote URL, checkout ID, filesystem path, hostname, or
/// user/workspace identity, and stays stable across reopen and repeated
/// `sce setup` runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryMetadata {
    pub repository_id: String,
    pub source_instance_id: String,
}

/// Generate a new candidate source-instance identity.
///
/// Callers must not assume the result is UUID-shaped; `is_valid_source_instance_id`
/// only requires a non-empty (once trimmed) value.
pub fn generate_source_instance_id() -> String {
    Uuid::new_v4().to_string()
}

/// Whether a stored or candidate value is a usable source-instance identity.
pub fn is_valid_source_instance_id(value: &str) -> bool {
    !value.trim().is_empty()
}

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

    /// Seed repository metadata on first initialization, atomically claim a
    /// `source_instance_id` for this physical database if none is set yet,
    /// and validate the result on every open.
    ///
    /// The stored `repository_id` must match the resolved repository ID for
    /// this database path; a mismatch means the file does not belong to the
    /// resolved repository and is an error rather than a write target.
    /// `source_instance_id` is claimed with a concurrency-safe `UPDATE ...
    /// WHERE source_instance_id = ''`: concurrent first opens generate their
    /// own candidate, but only one claim can affect the row, so every caller
    /// re-reads the row afterward and returns whichever value actually won.
    pub fn verify_or_initialize_repository_metadata(
        &self,
        repository_id: &str,
    ) -> Result<RepositoryMetadata> {
        self.execute_idempotent_write(INSERT_REPOSITORY_METADATA_SQL, (repository_id,))?;

        let Some((stored_repository_id, source_instance_id)) =
            self.select_repository_metadata_row()?
        else {
            anyhow::bail!(
                "repository Agent Trace DB metadata is missing its repository ID row. \
                 {REPOSITORY_AGENT_TRACE_SCHEMA_SETUP_GUIDANCE}"
            );
        };

        if stored_repository_id != repository_id {
            anyhow::bail!(
                "repository Agent Trace DB metadata mismatch: stored repository ID \
                 {stored_repository_id} does not match resolved repository ID {repository_id}"
            );
        }

        if is_valid_source_instance_id(&source_instance_id) {
            return Ok(RepositoryMetadata {
                repository_id: stored_repository_id,
                source_instance_id,
            });
        }

        let candidate = generate_source_instance_id();
        self.execute_idempotent_write(CLAIM_SOURCE_INSTANCE_ID_SQL, (candidate.as_str(),))?;

        let (final_repository_id, final_source_instance_id) =
            self.select_repository_metadata_row()?.ok_or_else(|| {
                anyhow::anyhow!(
                    "repository Agent Trace DB metadata row disappeared after \
                     source-instance-id claim"
                )
            })?;

        if !is_valid_source_instance_id(&final_source_instance_id) {
            anyhow::bail!("repository Agent Trace DB failed to establish a source-instance ID");
        }

        Ok(RepositoryMetadata {
            repository_id: final_repository_id,
            source_instance_id: final_source_instance_id,
        })
    }

    fn select_repository_metadata_row(&self) -> Result<Option<(String, String)>> {
        let rows = self.query_map(SELECT_REPOSITORY_METADATA_SQL, (), |row| {
            Ok((row.get::<String>(0)?, row.get::<String>(1)?))
        })?;
        Ok(rows.into_iter().next())
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

    pub fn upsert_claude_model_state(&self, input: ClaudeModelStateObservation) -> Result<u64> {
        upsert_claude_model_state_with(self, input)
    }

    pub fn claude_model_state_by_session_and_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<ClaudeModelStateObservation>> {
        super::claude_model_state_by_session_and_agent_with(self, session_id, agent_id)
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

    /// Atomically insert one conversation `messages` row and its one
    /// `parts` row: if `(message.session_id, message.message_id)` already
    /// exists, this is a no-op (`Ok(false)`); otherwise both rows insert
    /// together in one transaction (`Ok(true)`). Used by conversation
    /// text-event handlers (e.g. Codex `UserPromptSubmit`/`Stop`) in place
    /// of separate `insert_messages`/`insert_parts` calls, so a replayed or
    /// concurrent duplicate delivery never produces an orphaned `parts` row.
    pub fn insert_conversation_text_event(
        &self,
        message: InsertMessageInsert,
        part: InsertPartInsert,
    ) -> Result<bool> {
        insert_conversation_text_event_with(self, message, part, false)
    }

    /// Test-only counterpart of [`insert_conversation_text_event`] that
    /// forces the transaction to fail after the message insert and before
    /// the part insert, proving both statements roll back together.
    #[cfg(test)]
    pub(crate) fn insert_conversation_text_event_with_injected_failure(
        &self,
        message: InsertMessageInsert,
        part: InsertPartInsert,
    ) -> Result<bool> {
        insert_conversation_text_event_with(self, message, part, true)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use super::pre_health_invariant_fixture::seed_pre_health_invariant_fixture;
    use super::*;
    use crate::services::agent_trace_db::{
        MessageRole, ObservationKind, PartType, PAYLOAD_TYPE_PATCH,
    };

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

    fn row_count(db: &RepositoryAgentTraceDb, table: &str) -> i64 {
        db.query_map(&format!("SELECT COUNT(*) FROM {table}"), (), |row| {
            row.get::<i64>(0).map_err(Into::into)
        })
        .expect("count query should succeed")
        .into_iter()
        .next()
        .expect("count row should exist")
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

    // These tests verify convergence/idempotency under concurrent writers, not the
    // global production lock-retry budget. Retry exhausted lock contention at the
    // test boundary so scheduler/load variance does not make those semantic tests
    // flaky. Production retry behavior is intentionally unchanged and tracked
    // separately.
    const DATABASE_LOCKED_ERROR: &str = "database is locked";
    const CONCURRENT_WRITE_DEADLINE: Duration = Duration::from_secs(30);
    const CONCURRENT_WRITE_RETRY_BACKOFF: Duration = Duration::from_millis(10);

    fn retry_while_database_locked<T>(mut write: impl FnMut() -> Result<T>) -> Result<T> {
        let deadline = Instant::now() + CONCURRENT_WRITE_DEADLINE;
        loop {
            match write() {
                Err(error)
                    if error.to_string().contains(DATABASE_LOCKED_ERROR)
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(CONCURRENT_WRITE_RETRY_BACKOFF);
                }
                outcome => return outcome,
            }
        }
    }

    fn claude_observation(
        model_id: &str,
        observation_kind: ObservationKind,
        source: &str,
        observed_at_ms: i64,
    ) -> ClaudeModelStateObservation {
        ClaudeModelStateObservation {
            session_id: String::from("cc_session-1"),
            agent_id: String::new(),
            model_id: String::from(model_id),
            observation_kind,
            source: String::from(source),
            observed_at_ms,
        }
    }

    #[test]
    fn open_at_initializes_the_full_repository_schema() {
        let db_path = unique_test_db_path("baseline");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        for table in [
            "repository_metadata",
            "diff_traces",
            "post_commit_patch_intersections",
            "agent_traces",
            "messages",
            "parts",
            "claude_model_state",
            "mutation_trace_worktrees",
            "mutation_trace_scopes",
            "mutation_trace_processed_events",
            "mutation_trace_events",
            "mutation_trace_event_active_scopes",
            "mutation_trace_scope_provenance",
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
            "idx_mutation_trace_scopes_worktree",
            "idx_mutation_trace_scopes_worktree_status",
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
            vec![
                String::from("001_repository_schema"),
                String::from("002_repository_source_instance_id"),
                String::from("003_claude_model_state"),
                String::from("004_mutation_trace_protocol"),
                String::from("005_mutation_scope_provenance"),
                String::from("006_mutation_trace_health_invariant"),
            ],
            "repository DBs should be initialized from the baseline schema plus \
             its additive source-instance-id, Claude model-state, \
             mutation-trace-protocol, mutation-scope-provenance, and \
             mutation-trace-health-invariant migrations"
        );

        db.ensure_schema_ready_for_hooks()
            .expect("fresh repository DB schema should be ready");

        remove_test_db(&db_path);
    }

    #[test]
    fn pre_claude_model_state_database_is_upgraded_by_the_additive_migration() {
        let db_path = unique_test_db_path("claude-model-state-upgrade");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        db.execute("DROP TABLE claude_model_state", ())
            .expect("test should remove the post-003 table");
        db.execute(
            "DELETE FROM __sce_migrations WHERE id = '003_claude_model_state'",
            (),
        )
        .expect("test should remove the post-003 migration record");
        drop(db);

        let upgraded =
            RepositoryAgentTraceDb::new_at(&db_path).expect("pre-003 database should upgrade");
        assert!(sqlite_object_exists(
            &upgraded,
            "table",
            "claude_model_state"
        ));
        let applied_ids = upgraded
            .query_map(
                "SELECT id FROM __sce_migrations ORDER BY id ASC",
                (),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .expect("migration metadata query should succeed");
        assert_eq!(
            applied_ids,
            vec![
                String::from("001_repository_schema"),
                String::from("002_repository_source_instance_id"),
                String::from("003_claude_model_state"),
                String::from("004_mutation_trace_protocol"),
                String::from("005_mutation_scope_provenance"),
                String::from("006_mutation_trace_health_invariant"),
            ]
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn claude_model_state_has_exact_scope_and_guarded_deterministic_updates() {
        let db_path = unique_test_db_path("claude-model-state");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        let initial = claude_observation("claude/A", ObservationKind::SessionStart, "startup", 100);
        assert_eq!(
            db.upsert_claude_model_state(initial.clone())
                .expect("initial write"),
            1
        );
        assert_eq!(
            db.claude_model_state_by_session_and_agent("cc_session-1", "")
                .expect("state lookup")
                .expect("state should exist"),
            initial
        );
        assert!(db
            .claude_model_state_by_session_and_agent("cc_session-1", "subagent")
            .expect("subagent lookup")
            .is_none());
        assert!(db
            .claude_model_state_by_session_and_agent("cc_other", "")
            .expect("other session lookup")
            .is_none());

        assert_eq!(
            db.upsert_claude_model_state(claude_observation(
                "claude/older",
                ObservationKind::PostModelSwitch,
                "picker",
                99,
            ))
            .expect("older write should be guarded"),
            0
        );
        assert_eq!(
            db.claude_model_state_by_session_and_agent("cc_session-1", "")
                .expect("state lookup")
                .expect("state should remain")
                .model_id,
            "claude/A"
        );

        let switched =
            claude_observation("claude/B", ObservationKind::PostModelSwitch, "picker", 101);
        assert_eq!(
            db.upsert_claude_model_state(switched.clone())
                .expect("newer write"),
            1
        );
        assert_eq!(
            db.upsert_claude_model_state(switched.clone())
                .expect("identical replay should be harmless"),
            0
        );

        assert_eq!(
            db.upsert_claude_model_state(claude_observation(
                "claude/C",
                ObservationKind::SessionStart,
                "resume",
                101,
            ))
            .expect("equal-time lower-priority write should be guarded"),
            0
        );
        assert_eq!(
            db.claude_model_state_by_session_and_agent("cc_session-1", "")
                .expect("state lookup")
                .expect("state should remain")
                .model_id,
            "claude/B"
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn equal_time_same_kind_observations_use_a_stable_tie_break_and_concurrent_writes_converge() {
        let db_path = unique_test_db_path("claude-model-state-concurrent");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        drop(db);

        let db_path = std::sync::Arc::new(db_path);
        let handles: Vec<_> = (0..8)
            .map(|index| {
                let db_path = std::sync::Arc::clone(&db_path);
                std::thread::spawn(move || {
                    let db = RepositoryAgentTraceDb::open_without_migrations_at(&*db_path)
                        .expect("repository DB should reopen for concurrent state write");
                    retry_while_database_locked(|| {
                        db.upsert_claude_model_state(claude_observation(
                            &format!("claude/model-{index}"),
                            ObservationKind::PostModelSwitch,
                            "picker",
                            500,
                        ))
                    })
                    .expect("concurrent state writer should eventually complete")
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("state writer should not panic");
        }

        let db = RepositoryAgentTraceDb::open_without_migrations_at(&*db_path)
            .expect("repository DB should reopen for verification");
        let state = db
            .claude_model_state_by_session_and_agent("cc_session-1", "")
            .expect("state lookup")
            .expect("concurrent writes should leave one state row");
        assert_eq!(state.model_id, "claude/model-7");
        assert_eq!(state.observation_kind, ObservationKind::PostModelSwitch);
        assert_eq!(state.observed_at_ms, 500);

        remove_test_db(&db_path);
    }

    #[test]
    fn mutation_trace_worktrees_revision_must_be_a_blob_not_matching_length_text() {
        let db_path = unique_test_db_path("mutation-trace-revision-blob");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        let text_revision_error = db
            .execute(
                "INSERT INTO mutation_trace_worktrees
                    (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline)
                 VALUES ('wt-1', 'tree-0', '12345678', 0, 'healthy', 0)",
                (),
            )
            .expect_err(
                "an 8-byte TEXT value must still be rejected by the typeof(revision) = 'blob' check",
            );
        assert!(
            text_revision_error.to_string().contains("CHECK"),
            "unexpected error: {text_revision_error}"
        );

        db.execute(
            "INSERT INTO mutation_trace_worktrees
                (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline)
             VALUES ('wt-1', 'tree-0', X'0000000000000000', 0, 'healthy', 0)",
            (),
        )
        .expect("an 8-byte BLOB revision should be accepted");

        remove_test_db(&db_path);
    }

    #[test]
    fn mutation_trace_events_ai_exclusive_attribution_requires_a_scope_id() {
        let db_path = unique_test_db_path("mutation-trace-attribution-check");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        let missing_scope_error = db
            .execute(
                "INSERT INTO mutation_trace_events
                    (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
                     attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id)
                 VALUES ('wt-1', X'0000000000000001', 'tree-0', 'tree-1', 0, 'healthy',
                         'ai_exclusive', NULL, 'flush', NULL, NULL)",
                (),
            )
            .expect_err("ai_exclusive attribution with a NULL attribution_scope_id must be rejected");
        assert!(
            missing_scope_error.to_string().contains("CHECK"),
            "unexpected error: {missing_scope_error}"
        );

        db.execute(
            "INSERT INTO mutation_trace_events
                (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
                 attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id)
             VALUES ('wt-1', X'0000000000000001', 'tree-0', 'tree-1', 0, 'healthy',
                     'ai_exclusive', 'scope-1', 'start', 'scope-1', 'event-1')",
            (),
        )
        .expect("ai_exclusive attribution with a scope ID should be accepted");

        remove_test_db(&db_path);
    }

    #[test]
    fn mutation_trace_processed_events_identity_is_scope_and_event_only() {
        let db_path = unique_test_db_path("mutation-trace-processed-events-identity");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        let sql = table_sql(&db, "mutation_trace_processed_events");
        assert!(
            !sql.contains("worktree_id"),
            "mutation_trace_processed_events must not have a worktree_id column: {sql}"
        );

        db.execute(
            "INSERT INTO mutation_trace_processed_events (scope_id, event_id)
             VALUES ('scope-1', 'event-1')",
            (),
        )
        .expect("first (scope_id, event_id) insert should succeed");

        let duplicate_error = db
            .execute(
                "INSERT INTO mutation_trace_processed_events (scope_id, event_id)
                 VALUES ('scope-1', 'event-1')",
                (),
            )
            .expect_err("a duplicate (scope_id, event_id) pair must be rejected");
        assert!(
            duplicate_error.to_string().contains("UNIQUE")
                || duplicate_error.to_string().contains("PRIMARY KEY"),
            "unexpected error: {duplicate_error}"
        );

        db.execute(
            "INSERT INTO mutation_trace_processed_events (scope_id, event_id)
             VALUES ('scope-2', 'event-1')",
            (),
        )
        .expect("the same event_id under a different scope_id should be allowed");

        db.execute(
            "INSERT INTO mutation_trace_processed_events (scope_id, event_id)
             VALUES ('scope-1', 'event-2')",
            (),
        )
        .expect("the same scope_id with a different event_id should be allowed");

        assert_eq!(row_count(&db, "mutation_trace_processed_events"), 3);

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
        let first = db
            .verify_or_initialize_repository_metadata(&repository_id)
            .expect("first metadata initialization should succeed");
        assert_eq!(first.repository_id, repository_id);
        assert!(
            is_valid_source_instance_id(&first.source_instance_id),
            "freshly initialized metadata should have a non-empty source-instance ID"
        );

        let repeated = db
            .verify_or_initialize_repository_metadata(&repository_id)
            .expect("repeated validation with the same repository ID should succeed");
        assert_eq!(
            repeated.source_instance_id, first.source_instance_id,
            "repeated initialization must not regenerate the source-instance ID"
        );
        drop(db);

        let reopened = RepositoryAgentTraceDb::open_without_migrations_at(&db_path)
            .expect("repository DB should reopen");
        let reopened_metadata = reopened
            .verify_or_initialize_repository_metadata(&repository_id)
            .expect("reopen validation with the matching repository ID should succeed");
        assert_eq!(
            reopened_metadata.source_instance_id, first.source_instance_id,
            "source-instance ID must be stable across reopen"
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn source_instance_id_is_not_derived_from_repository_id_and_diverges_across_independent_dbs() {
        let first_db_path = unique_test_db_path("source-instance-first");
        let second_db_path = unique_test_db_path("source-instance-second");
        let repository_id = "a".repeat(64);

        let first_db = RepositoryAgentTraceDb::new_at(&first_db_path)
            .expect("first repository DB should open");
        let second_db = RepositoryAgentTraceDb::new_at(&second_db_path)
            .expect("second repository DB should open");

        let first_metadata = first_db
            .verify_or_initialize_repository_metadata(&repository_id)
            .expect("first DB metadata initialization should succeed");
        let second_metadata = second_db
            .verify_or_initialize_repository_metadata(&repository_id)
            .expect("second DB metadata initialization should succeed");

        assert_eq!(first_metadata.repository_id, second_metadata.repository_id);
        assert_ne!(
            first_metadata.source_instance_id, second_metadata.source_instance_id,
            "two independently created databases for the same logical repository \
             must diverge in source-instance ID"
        );
        assert_ne!(
            first_metadata.source_instance_id, repository_id,
            "source-instance ID must not be derived from repository_id"
        );

        remove_test_db(&first_db_path);
        remove_test_db(&second_db_path);
    }

    #[test]
    fn concurrent_initialization_converges_on_one_source_instance_id() {
        use std::sync::Arc;

        let db_path = unique_test_db_path("concurrent-source-instance");
        let repository_id = "a".repeat(64);

        // Create the schema up front so both threads race only on the
        // metadata claim, not schema creation.
        RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        let db_path = Arc::new(db_path);
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let db_path = Arc::clone(&db_path);
                let repository_id = repository_id.clone();
                std::thread::spawn(move || {
                    let db = RepositoryAgentTraceDb::open_without_migrations_at(&*db_path)
                        .expect("repository DB should reopen for concurrent claim");
                    retry_while_database_locked(|| {
                        db.verify_or_initialize_repository_metadata(&repository_id)
                    })
                    .expect("concurrent metadata initialization worker should eventually complete")
                })
            })
            .collect();

        let results: Vec<RepositoryMetadata> = handles
            .into_iter()
            .map(|handle| handle.join().expect("worker thread should not panic"))
            .collect();

        let winning_id = results[0].source_instance_id.clone();
        for result in &results {
            assert_eq!(
                result.source_instance_id, winning_id,
                "all concurrent initializations must converge on one persisted \
                 source-instance ID"
            );
        }

        remove_test_db(&db_path);
    }

    #[test]
    fn mismatched_repository_metadata_errors_on_open() {
        let db_path = unique_test_db_path("mismatch");
        let stored_repository_id = "a".repeat(64);
        let other_repository_id = "b".repeat(64);

        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let stored = db
            .verify_or_initialize_repository_metadata(&stored_repository_id)
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

        let unchanged = db
            .verify_or_initialize_repository_metadata(&stored_repository_id)
            .expect("re-validating with the original repository ID should still succeed");
        assert_eq!(
            unchanged.source_instance_id, stored.source_instance_id,
            "a rejected mismatched claim must not alter the stored source-instance ID"
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn baseline_only_fixture_migrates_and_gets_a_stable_source_instance_id() {
        let db_path = unique_test_db_path("baseline-only-fixture");
        let repository_id = "a".repeat(64);

        // Simulate a database created before migration 002 existed: build the
        // pre-002 `repository_metadata` shape (no `source_instance_id`
        // column) and record only migration 001 as applied, so opening with
        // the current embedded migration set exercises the real 001-applied,
        // 002-pending upgrade path.
        let baseline_only = RepositoryAgentTraceDb::open_without_migrations_at(&db_path)
            .expect("baseline-only repository DB should open");
        baseline_only
            .execute(
                "CREATE TABLE IF NOT EXISTS __sce_migrations (
    id TEXT PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
)",
                (),
            )
            .expect("migration metadata table should create");
        baseline_only
            .execute(
                "CREATE TABLE IF NOT EXISTS repository_metadata (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    repository_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
)",
                (),
            )
            .expect("baseline repository_metadata table should create");
        baseline_only
            .execute(
                "INSERT INTO __sce_migrations (id) VALUES ('001_repository_schema')",
                (),
            )
            .expect("baseline migration record should insert");
        baseline_only
            .execute(
                "INSERT INTO repository_metadata (id, repository_id) VALUES (1, ?1)",
                (repository_id.as_str(),),
            )
            .expect("baseline metadata row should seed");
        drop(baseline_only);

        let migrated =
            RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should migrate to 002");
        let metadata = migrated
            .verify_or_initialize_repository_metadata(&repository_id)
            .expect("metadata initialization after migrating from baseline should succeed");
        assert_eq!(metadata.repository_id, repository_id);
        assert!(
            is_valid_source_instance_id(&metadata.source_instance_id),
            "migrating from a baseline-only fixture should populate a valid source-instance ID"
        );
        drop(migrated);

        let reopened = RepositoryAgentTraceDb::open_without_migrations_at(&db_path)
            .expect("migrated repository DB should reopen");
        let reopened_metadata = reopened
            .verify_or_initialize_repository_metadata(&repository_id)
            .expect("reopen after migration should succeed");
        assert_eq!(
            reopened_metadata.source_instance_id, metadata.source_instance_id,
            "source-instance ID populated during migration must remain stable across reopen"
        );

        remove_test_db(&db_path);
    }

    fn seed_001_and_002_only_fixture(db_path: &std::path::Path, repository_id: &str) {
        let fixture = RepositoryAgentTraceDb::open_without_migrations_at(db_path)
            .expect("001+002-only fixture DB should open");
        fixture
            .execute(
                "CREATE TABLE IF NOT EXISTS __sce_migrations (
    id TEXT PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
)",
                (),
            )
            .expect("migration metadata table should create");
        fixture
            .execute(
                "CREATE TABLE IF NOT EXISTS repository_metadata (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    repository_id TEXT NOT NULL,
    source_instance_id TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
)",
                (),
            )
            .expect("post-002 repository_metadata table should create");
        for (table, ddl) in [
            (
                "diff_traces",
                "CREATE TABLE IF NOT EXISTS diff_traces (id INTEGER PRIMARY KEY)",
            ),
            (
                "post_commit_patch_intersections",
                "CREATE TABLE IF NOT EXISTS post_commit_patch_intersections (id INTEGER PRIMARY KEY)",
            ),
            (
                "agent_traces",
                "CREATE TABLE IF NOT EXISTS agent_traces (id INTEGER PRIMARY KEY)",
            ),
            (
                "messages",
                "CREATE TABLE IF NOT EXISTS messages (id INTEGER PRIMARY KEY)",
            ),
            (
                "parts",
                "CREATE TABLE IF NOT EXISTS parts (id INTEGER PRIMARY KEY)",
            ),
        ] {
            fixture
                .execute(ddl, ())
                .unwrap_or_else(|error| panic!("{table} table should create: {error}"));
        }
        fixture
            .execute(
                "INSERT INTO __sce_migrations (id) VALUES ('001_repository_schema')",
                (),
            )
            .expect("001 migration record should insert");
        fixture
            .execute(
                "INSERT INTO __sce_migrations (id) VALUES ('002_repository_source_instance_id')",
                (),
            )
            .expect("002 migration record should insert");
        fixture
            .execute(
                "INSERT INTO repository_metadata (id, repository_id) VALUES (1, ?1)",
                (repository_id,),
            )
            .expect("repository_metadata row should seed");
        drop(fixture);
    }

    #[test]
    fn baseline_and_source_instance_fixture_migrates_to_mutation_trace_protocol_through_setup() {
        let db_path = unique_test_db_path("baseline-and-source-instance-fixture");
        let repository_id = "c".repeat(64);

        seed_001_and_002_only_fixture(&db_path, &repository_id);

        let migrated = RepositoryAgentTraceDb::new_at(&db_path)
            .expect("repository DB should migrate a 001+002-only fixture through 004");

        let applied_ids = migrated
            .query_map(
                "SELECT id FROM __sce_migrations ORDER BY id ASC",
                (),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .expect("migration metadata query should succeed");
        assert_eq!(
            applied_ids,
            vec![
                String::from("001_repository_schema"),
                String::from("002_repository_source_instance_id"),
                String::from("003_claude_model_state"),
                String::from("004_mutation_trace_protocol"),
                String::from("005_mutation_scope_provenance"),
                String::from("006_mutation_trace_health_invariant"),
            ],
            "an existing 001+002 database should get 003, 004, 005, and 006 applied on top through the setup/lifecycle path, without reapplying 001/002"
        );

        for table in [
            "mutation_trace_worktrees",
            "mutation_trace_scopes",
            "mutation_trace_processed_events",
            "mutation_trace_events",
            "mutation_trace_event_active_scopes",
            "mutation_trace_scope_provenance",
        ] {
            assert!(
                sqlite_object_exists(&migrated, "table", table),
                "table '{table}' should exist after migrating a 001+002-only fixture"
            );
        }

        migrated
            .ensure_schema_ready_for_hooks()
            .expect("migrated repository DB schema should be ready for hooks");

        let metadata = migrated
            .verify_or_initialize_repository_metadata(&repository_id)
            .expect("metadata initialization on a migrated 001+002-only fixture should succeed");
        assert_eq!(metadata.repository_id, repository_id);

        remove_test_db(&db_path);
    }

    #[test]
    fn hook_runtime_path_never_applies_mutation_trace_protocol_migration() {
        let db_path = unique_test_db_path("hook-runtime-no-migration");
        let repository_id = "d".repeat(64);

        seed_001_and_002_only_fixture(&db_path, &repository_id);

        let db = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
            .expect("hook-runtime open without migrations should succeed on an existing DB file");

        let readiness_error = db.ensure_schema_ready_for_hooks().expect_err(
            "a 001+002-only DB should not be schema-ready for the mutation-trace store",
        );
        assert!(
            readiness_error
                .to_string()
                .contains(REPOSITORY_AGENT_TRACE_SCHEMA_SETUP_GUIDANCE),
            "unexpected error: {readiness_error}"
        );

        let repair_error = db
            .repair_missing_repository_schema_migration_metadata()
            .expect_err("the base-table repair path must not silently mark 003 as applied");
        assert!(
            repair_error
                .to_string()
                .contains(REPOSITORY_AGENT_TRACE_SCHEMA_SETUP_GUIDANCE),
            "unexpected error: {repair_error}"
        );

        let applied_ids = db
            .query_map(
                "SELECT id FROM __sce_migrations ORDER BY id ASC",
                (),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .expect("migration metadata query should succeed");
        assert_eq!(
            applied_ids,
            vec![
                String::from("001_repository_schema"),
                String::from("002_repository_source_instance_id"),
            ],
            "the no-migration hook-runtime path must never record or apply 003, 004, 005, or 006"
        );

        for table in [
            "mutation_trace_worktrees",
            "mutation_trace_scopes",
            "mutation_trace_processed_events",
            "mutation_trace_events",
            "mutation_trace_event_active_scopes",
            "mutation_trace_scope_provenance",
        ] {
            assert!(
                !sqlite_object_exists(&db, "table", table),
                "table '{table}' should not exist; the hook-runtime path must not create mutation-trace tables"
            );
        }

        drop(db);
        remove_test_db(&db_path);
    }

    const TABLE_REBUILD_PROBE_SQL: &str = "BEGIN IMMEDIATE;
CREATE TABLE probe_health (
    tainted INTEGER NOT NULL CHECK (tainted IN (0, 1)),
    failure_kind TEXT NOT NULL,
    CHECK (tainted = CASE WHEN failure_kind = 'healthy' THEN 0 ELSE 1 END)
);
INSERT INTO probe_health (tainted, failure_kind) VALUES (0, 'healthy');
CREATE TABLE probe_health_v006 (
    tainted INTEGER NOT NULL CHECK (tainted IN (0, 1)),
    failure_kind TEXT NOT NULL,
    CHECK (tainted = CASE WHEN failure_kind = 'healthy' THEN 0 ELSE 1 END)
);
INSERT INTO probe_health_v006 (tainted, failure_kind)
SELECT tainted, failure_kind FROM probe_health;
DROP TABLE probe_health;
ALTER TABLE probe_health_v006 RENAME TO probe_health;
COMMIT;";

    const TABLE_REBUILD_PROBE_MIGRATIONS: &[(&str, &str)] =
        &[("probe_table_rebuild", TABLE_REBUILD_PROBE_SQL)];

    struct TableRebuildProbeDbSpec;

    impl DbSpec for TableRebuildProbeDbSpec {
        fn db_name() -> &'static str {
            "table rebuild probe DB"
        }

        fn db_path() -> Result<PathBuf> {
            anyhow::bail!("table rebuild probe DB has no canonical path")
        }

        fn migrations() -> &'static [(&'static str, &'static str)] {
            TABLE_REBUILD_PROBE_MIGRATIONS
        }

        fn db_config_key() -> &'static str {
            "agent_trace_db"
        }
    }

    #[test]
    fn turso_supports_the_transactional_table_rebuild_used_by_migration_006() {
        let db_path = unique_test_db_path("table-rebuild-probe");
        let probe = TursoDb::<TableRebuildProbeDbSpec>::new_at(&db_path).expect(
            "turso should run BEGIN IMMEDIATE, DROP TABLE, ALTER TABLE RENAME and COMMIT in one batch",
        );

        let tables = probe
            .query_map(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'probe_health%'",
                (),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .expect("sqlite_master query should succeed");
        assert_eq!(tables, vec![String::from("probe_health")]);

        let rows = probe
            .query_map(
                "SELECT tainted, failure_kind FROM probe_health",
                (),
                |row| Ok((row.get::<i64>(0)?, row.get::<String>(1)?)),
            )
            .expect("renamed table should be readable");
        assert_eq!(rows, vec![(0, String::from("healthy"))]);

        probe
            .execute(
                "INSERT INTO probe_health (tainted, failure_kind) VALUES (1, 'snapshot_failure')",
                (),
            )
            .expect("a consistent pair should satisfy the CASE check");
        for (tainted, failure_kind) in [(0, "snapshot_failure"), (1, "healthy")] {
            let error = probe
                .execute(
                    "INSERT INTO probe_health (tainted, failure_kind) VALUES (?1, ?2)",
                    (tainted, failure_kind),
                )
                .expect_err("an inconsistent pair should violate the CASE check");
            assert!(
                error.to_string().contains("CHECK"),
                "unexpected error: {error}"
            );
        }

        drop(probe);
        remove_test_db(&db_path);
    }

    const HEALTH_INVARIANT_MIGRATION_ID: &str = "006_mutation_trace_health_invariant";
    const HEALTH_INVARIANT_TABLES: [&str; 2] =
        ["mutation_trace_worktrees", "mutation_trace_events"];

    const WORKTREE_ROWS_SNAPSHOT_SQL: &str = "SELECT worktree_id, quote(cursor_tree) || '|' || quote(revision) || '|' || \
         quote(failure_kind) || '|' || quote(needs_rebaseline) || '|' || quote(created_at) || '|' || \
         quote(updated_at) FROM mutation_trace_worktrees ORDER BY worktree_id";
    const EVENT_ROWS_SNAPSHOT_SQL: &str = "SELECT worktree_id || ':' || hex(revision), quote(before_tree) || '|' || \
         quote(after_tree) || '|' || quote(failure_kind) || '|' || quote(attribution_kind) || '|' || \
         quote(attribution_scope_id) || '|' || quote(boundary_kind) || '|' || quote(boundary_scope_id) || '|' || \
         quote(boundary_event_id) || '|' || quote(created_at) FROM mutation_trace_events \
         ORDER BY worktree_id, revision";
    const WORKTREE_TAINT_SQL: &str =
        "SELECT worktree_id, tainted FROM mutation_trace_worktrees ORDER BY worktree_id";
    const EVENT_TAINT_SQL: &str =
        "SELECT worktree_id || ':' || hex(revision), tainted FROM mutation_trace_events \
         ORDER BY worktree_id, revision";
    const UNTOUCHED_TABLE_SNAPSHOT_SQL: [&str; 4] = [
        "SELECT scope_id, quote(worktree_id) || '|' || quote(actor_kind) || '|' || quote(status) || '|' || \
         quote(created_at) || '|' || quote(updated_at) FROM mutation_trace_scopes ORDER BY scope_id",
        "SELECT scope_id || ':' || event_id, quote(created_at) FROM mutation_trace_processed_events \
         ORDER BY scope_id, event_id",
        "SELECT worktree_id || ':' || hex(revision) || ':' || scope_id, '' \
         FROM mutation_trace_event_active_scopes ORDER BY worktree_id, revision, scope_id",
        "SELECT scope_id, quote(session_id) || '|' || quote(model_id) || '|' || quote(created_at) \
         FROM mutation_trace_scope_provenance ORDER BY scope_id",
    ];

    const LEGACY_HEALTH_ROWS_SQL: [&str; 12] = [
        "INSERT INTO mutation_trace_worktrees
            (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline, created_at, updated_at)
         VALUES ('wt-healthy-tainted', 'tree-a', X'0000000000000001', 1, 'healthy', 1,
                 '2026-01-01T00:00:00.000Z', '2026-01-02T00:00:00.000Z')",
        "INSERT INTO mutation_trace_worktrees
            (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline, created_at, updated_at)
         VALUES ('wt-healthy-untainted', 'tree-b', X'0000000000000002', 0, 'healthy', 0,
                 '2026-01-03T00:00:00.000Z', '2026-01-04T00:00:00.000Z')",
        "INSERT INTO mutation_trace_worktrees
            (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline, created_at, updated_at)
         VALUES ('wt-snapshot-failure-tainted', 'tree-c', X'0000000000000003', 1, 'snapshot_failure', 0,
                 '2026-01-05T00:00:00.000Z', '2026-01-06T00:00:00.000Z')",
        "INSERT INTO mutation_trace_worktrees
            (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline, created_at, updated_at)
         VALUES ('wt-snapshot-failure-untainted', 'tree-d', X'00000000000000FF', 0, 'snapshot_failure', 1,
                 '2026-01-07T00:00:00.000Z', '2026-01-08T00:00:00.000Z')",
        "INSERT INTO mutation_trace_events
            (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
             attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id, created_at)
         VALUES ('wt-events', X'0000000000000001', 'tree-0', 'tree-1', 1, 'healthy',
                 'ai_exclusive', 'scope-1', 'start', 'scope-1', 'event-1', '2026-02-01T00:00:00.000Z')",
        "INSERT INTO mutation_trace_events
            (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
             attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id, created_at)
         VALUES ('wt-events', X'0000000000000002', 'tree-1', 'tree-2', 0, 'healthy',
                 'ai_contended', NULL, 'advance', 'scope-1', 'event-2', '2026-02-02T00:00:00.000Z')",
        "INSERT INTO mutation_trace_events
            (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
             attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id, created_at)
         VALUES ('wt-events', X'0000000000000003', 'tree-2', 'tree-3', 1, 'snapshot_failure',
                 'ineligible_unscoped', NULL, 'flush', NULL, NULL, '2026-02-03T00:00:00.000Z')",
        "INSERT INTO mutation_trace_events
            (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
             attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id, created_at)
         VALUES ('wt-events', X'0000000000000004', 'tree-3', 'tree-4', 0, 'snapshot_failure',
                 'ineligible_unscoped', NULL, 'close', 'scope-1', 'event-3', '2026-02-04T00:00:00.000Z')",
        "INSERT INTO mutation_trace_scopes (scope_id, worktree_id, actor_kind, status, created_at, updated_at)
         VALUES ('scope-1', 'wt-events', 'codex', 'closed',
                 '2026-03-01T00:00:00.000Z', '2026-03-02T00:00:00.000Z')",
        "INSERT INTO mutation_trace_processed_events (scope_id, event_id, created_at)
         VALUES ('scope-1', 'event-1', '2026-03-03T00:00:00.000Z')",
        "INSERT INTO mutation_trace_event_active_scopes (worktree_id, revision, scope_id)
         VALUES ('wt-events', X'0000000000000002', 'scope-1')",
        "INSERT INTO mutation_trace_scope_provenance (scope_id, session_id, model_id, created_at)
         VALUES ('scope-1', 'session-1', 'model-1', '2026-03-04T00:00:00.000Z')",
    ];

    fn keyed_text_rows(db: &RepositoryAgentTraceDb, sql: &str) -> Vec<(String, String)> {
        db.query_map(sql, (), |row| {
            Ok((row.get::<String>(0)?, row.get::<String>(1)?))
        })
        .expect("snapshot query should succeed")
    }

    fn keyed_flag_rows(db: &RepositoryAgentTraceDb, sql: &str) -> Vec<(String, i64)> {
        db.query_map(sql, (), |row| {
            Ok((row.get::<String>(0)?, row.get::<i64>(1)?))
        })
        .expect("flag query should succeed")
    }

    fn applied_migration_ids(db: &RepositoryAgentTraceDb) -> Vec<String> {
        db.query_map(
            "SELECT id FROM __sce_migrations ORDER BY id ASC",
            (),
            |row| row.get::<String>(0).map_err(Into::into),
        )
        .expect("migration metadata query should succeed")
    }

    fn health_table_rows(db: &RepositoryAgentTraceDb) -> Vec<Vec<(String, String)>> {
        let mut snapshots = vec![
            keyed_text_rows(db, WORKTREE_ROWS_SNAPSHOT_SQL),
            keyed_text_rows(db, EVENT_ROWS_SNAPSHOT_SQL),
        ];
        snapshots.extend(
            UNTOUCHED_TABLE_SNAPSHOT_SQL
                .iter()
                .map(|sql| keyed_text_rows(db, sql)),
        );
        snapshots
    }

    fn insert_health_pair(
        db: &RepositoryAgentTraceDb,
        table: &str,
        tainted: i64,
        failure_kind: &str,
    ) -> Result<u64> {
        let sql = match table {
            "mutation_trace_worktrees" => {
                "INSERT INTO mutation_trace_worktrees
                    (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline)
                 VALUES ('wt-1', 'tree-0', X'0000000000000000', ?1, ?2, 0)"
            }
            "mutation_trace_events" => {
                "INSERT INTO mutation_trace_events
                    (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
                     attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id)
                 VALUES ('wt-1', X'0000000000000001', 'tree-0', 'tree-1', ?1, ?2,
                         'ineligible_unscoped', NULL, 'flush', NULL, NULL)"
            }
            other => panic!("unexpected health-invariant table '{other}'"),
        };
        db.execute(sql, (tainted, failure_kind))
    }

    fn assert_health_pair_accepted(table: &str, tainted: i64, failure_kind: &str) {
        let db_path = unique_test_db_path("mutation-trace-health-accepted");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        insert_health_pair(&db, table, tainted, failure_kind).unwrap_or_else(|error| {
            panic!("{table} should accept (tainted={tainted}, '{failure_kind}'): {error}")
        });
        assert_eq!(row_count(&db, table), 1);

        remove_test_db(&db_path);
    }

    fn assert_health_pair_rejected(table: &str, tainted: i64, failure_kind: &str) {
        let db_path = unique_test_db_path("mutation-trace-health-rejected");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        let error = insert_health_pair(&db, table, tainted, failure_kind).expect_err(&format!(
            "{table} must reject (tainted={tainted}, '{failure_kind}')"
        ));
        assert!(
            error.to_string().contains("CHECK"),
            "unexpected error: {error}"
        );
        assert_eq!(row_count(&db, table), 0);

        remove_test_db(&db_path);
    }

    #[test]
    fn mutation_trace_worktrees_accepts_untainted_healthy_pair() {
        assert_health_pair_accepted("mutation_trace_worktrees", 0, "healthy");
    }

    #[test]
    fn mutation_trace_worktrees_accepts_tainted_snapshot_failure_pair() {
        assert_health_pair_accepted("mutation_trace_worktrees", 1, "snapshot_failure");
    }

    #[test]
    fn mutation_trace_worktrees_rejects_untainted_snapshot_failure_pair() {
        assert_health_pair_rejected("mutation_trace_worktrees", 0, "snapshot_failure");
    }

    #[test]
    fn mutation_trace_worktrees_rejects_tainted_healthy_pair() {
        assert_health_pair_rejected("mutation_trace_worktrees", 1, "healthy");
    }

    #[test]
    fn mutation_trace_events_accepts_untainted_healthy_pair() {
        assert_health_pair_accepted("mutation_trace_events", 0, "healthy");
    }

    #[test]
    fn mutation_trace_events_accepts_tainted_snapshot_failure_pair() {
        assert_health_pair_accepted("mutation_trace_events", 1, "snapshot_failure");
    }

    #[test]
    fn mutation_trace_events_rejects_untainted_snapshot_failure_pair() {
        assert_health_pair_rejected("mutation_trace_events", 0, "snapshot_failure");
    }

    #[test]
    fn mutation_trace_events_rejects_tainted_healthy_pair() {
        assert_health_pair_rejected("mutation_trace_events", 1, "healthy");
    }

    #[test]
    fn migration_006_keeps_the_existing_mutation_trace_column_checks_and_primary_keys() {
        let db_path = unique_test_db_path("mutation-trace-health-existing-checks");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        for (statement, expected) in [
            (
                "INSERT INTO mutation_trace_worktrees
                    (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline)
                 VALUES ('wt-1', 'tree-0', X'0000000000000000', 2, 'snapshot_failure', 0)",
                "CHECK",
            ),
            (
                "INSERT INTO mutation_trace_worktrees
                    (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline)
                 VALUES ('wt-1', 'tree-0', X'0000000000000000', 1, 'unknown', 0)",
                "CHECK",
            ),
            (
                "INSERT INTO mutation_trace_worktrees
                    (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline)
                 VALUES ('wt-1', 'tree-0', X'0000000000000000', 0, 'healthy', 2)",
                "CHECK",
            ),
            (
                "INSERT INTO mutation_trace_events
                    (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
                     attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id)
                 VALUES ('wt-1', '12345678', 'tree-0', 'tree-1', 0, 'healthy',
                         'ineligible_unscoped', NULL, 'flush', NULL, NULL)",
                "CHECK",
            ),
            (
                "INSERT INTO mutation_trace_events
                    (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
                     attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id)
                 VALUES ('wt-1', X'0000000000000001', 'tree-0', 'tree-1', 0, 'healthy',
                         'ineligible_unscoped', 'scope-1', 'flush', NULL, NULL)",
                "CHECK",
            ),
            (
                "INSERT INTO mutation_trace_events
                    (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
                     attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id)
                 VALUES ('wt-1', X'0000000000000001', 'tree-0', 'tree-1', 0, 'healthy',
                         'ineligible_unscoped', NULL, 'flush', 'scope-1', 'event-1')",
                "CHECK",
            ),
        ] {
            let error = db
                .execute(statement, ())
                .expect_err("a row violating a 004 column check must be rejected after 006");
            assert!(
                error.to_string().contains(expected),
                "unexpected error: {error}"
            );
        }

        insert_health_pair(&db, "mutation_trace_worktrees", 0, "healthy")
            .expect("first worktree row should insert");
        insert_health_pair(&db, "mutation_trace_events", 0, "healthy")
            .expect("first event row should insert");
        for table in HEALTH_INVARIANT_TABLES {
            let error = insert_health_pair(&db, table, 0, "healthy")
                .expect_err("a duplicate primary key must be rejected after 006");
            assert!(
                error.to_string().contains("UNIQUE") || error.to_string().contains("PRIMARY KEY"),
                "unexpected error: {error}"
            );
        }

        remove_test_db(&db_path);
    }

    #[test]
    fn migration_006_normalizes_legacy_health_pairs_from_failure_kind_and_preserves_other_columns()
    {
        let db_path = unique_test_db_path("mutation-trace-health-legacy");
        seed_pre_health_invariant_fixture(&db_path, &LEGACY_HEALTH_ROWS_SQL);

        let legacy = RepositoryAgentTraceDb::open_without_migrations_at(&db_path)
            .expect("pre-006 fixture should reopen without migrations");
        let legacy_rows = health_table_rows(&legacy);
        assert_eq!(
            keyed_flag_rows(&legacy, WORKTREE_TAINT_SQL),
            vec![
                (String::from("wt-healthy-tainted"), 1),
                (String::from("wt-healthy-untainted"), 0),
                (String::from("wt-snapshot-failure-tainted"), 1),
                (String::from("wt-snapshot-failure-untainted"), 0),
            ]
        );
        drop(legacy);

        let migrated =
            RepositoryAgentTraceDb::new_at(&db_path).expect("pre-006 database should upgrade");

        assert_eq!(
            applied_migration_ids(&migrated).last().map(String::as_str),
            Some(HEALTH_INVARIANT_MIGRATION_ID)
        );
        assert_eq!(
            keyed_flag_rows(&migrated, WORKTREE_TAINT_SQL),
            vec![
                (String::from("wt-healthy-tainted"), 0),
                (String::from("wt-healthy-untainted"), 0),
                (String::from("wt-snapshot-failure-tainted"), 1),
                (String::from("wt-snapshot-failure-untainted"), 1),
            ]
        );
        assert_eq!(
            keyed_flag_rows(&migrated, EVENT_TAINT_SQL),
            vec![
                (String::from("wt-events:0000000000000001"), 0),
                (String::from("wt-events:0000000000000002"), 0),
                (String::from("wt-events:0000000000000003"), 1),
                (String::from("wt-events:0000000000000004"), 1),
            ]
        );
        assert_eq!(
            health_table_rows(&migrated),
            legacy_rows,
            "006 must preserve every non-tainted column and leave the other mutation-trace tables untouched"
        );
        migrated
            .ensure_schema_ready_for_hooks()
            .expect("upgraded repository DB schema should be ready for hooks");

        remove_test_db(&db_path);
    }

    #[test]
    fn fresh_and_upgraded_databases_share_the_migration_006_table_sql() {
        let fresh_path = unique_test_db_path("mutation-trace-health-fresh-schema");
        let fresh = RepositoryAgentTraceDb::new_at(&fresh_path).expect("fresh DB should open");

        let upgraded_path = unique_test_db_path("mutation-trace-health-upgraded-schema");
        seed_pre_health_invariant_fixture(&upgraded_path, &LEGACY_HEALTH_ROWS_SQL);
        let upgraded = RepositoryAgentTraceDb::new_at(&upgraded_path)
            .expect("pre-006 database should upgrade");

        for table in HEALTH_INVARIANT_TABLES {
            let fresh_sql = table_sql(&fresh, table);
            assert!(
                fresh_sql.contains("CASE WHEN failure_kind = 'healthy' THEN 0 ELSE 1 END"),
                "{table} should carry the cross-column health check: {fresh_sql}"
            );
            assert_eq!(fresh_sql, table_sql(&upgraded, table));
            assert!(!sqlite_object_exists(
                &fresh,
                "table",
                &format!("{table}_v006")
            ));
            assert!(!sqlite_object_exists(
                &upgraded,
                "table",
                &format!("{table}_v006")
            ));
        }

        remove_test_db(&fresh_path);
        remove_test_db(&upgraded_path);
    }

    #[test]
    fn migration_006_sql_body_reruns_without_changing_schema_or_rows() {
        let db_path = unique_test_db_path("mutation-trace-health-rerun");
        seed_pre_health_invariant_fixture(&db_path, &LEGACY_HEALTH_ROWS_SQL);

        let migrated =
            RepositoryAgentTraceDb::new_at(&db_path).expect("pre-006 database should upgrade");
        let schema_after_first_run =
            HEALTH_INVARIANT_TABLES.map(|table| table_sql(&migrated, table));
        let rows_after_first_run = health_table_rows(&migrated);
        let worktree_taint_after_first_run = keyed_flag_rows(&migrated, WORKTREE_TAINT_SQL);
        let event_taint_after_first_run = keyed_flag_rows(&migrated, EVENT_TAINT_SQL);
        let migrations_after_first_run = applied_migration_ids(&migrated);
        migrated
            .execute(
                "DELETE FROM __sce_migrations WHERE id = ?1",
                (HEALTH_INVARIANT_MIGRATION_ID,),
            )
            .expect("test should drop the 006 metadata row to force a re-run");
        drop(migrated);

        let rerun = RepositoryAgentTraceDb::new_at(&db_path)
            .expect("the 006 SQL body should re-run against an already-rebuilt schema");

        assert_eq!(
            HEALTH_INVARIANT_TABLES.map(|table| table_sql(&rerun, table)),
            schema_after_first_run
        );
        assert_eq!(health_table_rows(&rerun), rows_after_first_run);
        assert_eq!(
            keyed_flag_rows(&rerun, WORKTREE_TAINT_SQL),
            worktree_taint_after_first_run
        );
        assert_eq!(
            keyed_flag_rows(&rerun, EVENT_TAINT_SQL),
            event_taint_after_first_run
        );
        assert_eq!(applied_migration_ids(&rerun), migrations_after_first_run);

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

    fn conversation_text_event_fixture() -> (InsertMessageInsert, InsertPartInsert) {
        (
            InsertMessageInsert {
                session_id: "cx_session-1".to_string(),
                message_id: "cx:turn-1:user".to_string(),
                role: MessageRole::User,
                generated_at_unix_ms: 1_000,
            },
            InsertPartInsert {
                part_type: PartType::Text,
                text: "hello world".to_string(),
                session_id: "cx_session-1".to_string(),
                message_id: "cx:turn-1:user".to_string(),
                generated_at_unix_ms: 1_000,
            },
        )
    }

    #[test]
    fn insert_conversation_text_event_inserts_message_and_part_together() {
        let db_path = unique_test_db_path("conversation-event-insert");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let (message, part) = conversation_text_event_fixture();

        let inserted = db
            .insert_conversation_text_event(message, part)
            .expect("conversation text event insert should succeed");

        assert!(inserted, "first delivery should insert both rows");
        assert_eq!(row_count(&db, "messages"), 1);
        assert_eq!(row_count(&db, "parts"), 1);

        remove_test_db(&db_path);
    }

    #[test]
    fn insert_conversation_text_event_is_a_no_op_on_sequential_replay() {
        let db_path = unique_test_db_path("conversation-event-replay");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        let (message, part) = conversation_text_event_fixture();
        let first = db
            .insert_conversation_text_event(message, part)
            .expect("first delivery should succeed");

        let (message, part) = conversation_text_event_fixture();
        let second = db
            .insert_conversation_text_event(message, part)
            .expect("replayed delivery should succeed");

        assert!(first);
        assert!(!second, "a replayed delivery must be a no-op");
        assert_eq!(row_count(&db, "messages"), 1);
        assert_eq!(row_count(&db, "parts"), 1);

        remove_test_db(&db_path);
    }

    #[test]
    fn insert_conversation_text_event_ten_sequential_replays_still_leave_one_row_pair() {
        let db_path = unique_test_db_path("conversation-event-replay-ten");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        for _ in 0..10 {
            let (message, part) = conversation_text_event_fixture();
            db.insert_conversation_text_event(message, part)
                .expect("every replayed delivery should succeed");
        }

        assert_eq!(row_count(&db, "messages"), 1);
        assert_eq!(row_count(&db, "parts"), 1);

        remove_test_db(&db_path);
    }

    #[test]
    fn insert_conversation_text_event_injected_failure_rolls_back_both_rows() {
        let db_path = unique_test_db_path("conversation-event-rollback");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let (message, part) = conversation_text_event_fixture();

        let error = db
            .insert_conversation_text_event_with_injected_failure(message, part)
            .expect_err("an injected failure before the part insert should propagate as an error");
        assert!(error.to_string().contains("injected failure"));

        assert_eq!(
            row_count(&db, "messages"),
            0,
            "the message row must roll back along with the failed part insert"
        );
        assert_eq!(row_count(&db, "parts"), 0);

        remove_test_db(&db_path);
    }

    #[test]
    fn insert_conversation_text_event_concurrent_duplicate_delivery_leaves_one_row_pair() {
        use std::sync::Arc;

        let db_path = unique_test_db_path("conversation-event-concurrent");

        // Create the schema up front so every thread races only on the
        // conversation text event insert, not schema creation, mirroring
        // `concurrent_initialization_converges_on_one_source_instance_id`.
        RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

        let db_path = Arc::new(db_path);
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let db_path = Arc::clone(&db_path);
                std::thread::spawn(move || {
                    let db = RepositoryAgentTraceDb::open_without_migrations_at(&*db_path)
                        .expect("repository DB should reopen for concurrent delivery");
                    retry_while_database_locked(|| {
                        let (message, part) = conversation_text_event_fixture();
                        db.insert_conversation_text_event(message, part)
                    })
                })
            })
            .collect();

        let results: Vec<bool> = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .expect("worker thread should not panic")
                    .expect("every concurrent delivery worker should eventually complete")
            })
            .collect();

        assert_eq!(
            results.iter().filter(|inserted| **inserted).count(),
            1,
            "exactly one concurrent delivery should have performed the insert"
        );

        let db = RepositoryAgentTraceDb::open_without_migrations_at(&*db_path)
            .expect("repository DB should reopen for verification");
        assert_eq!(row_count(&db, "messages"), 1);
        assert_eq!(row_count(&db, "parts"), 1);

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

#[cfg(test)]
pub(crate) mod pre_health_invariant_fixture {
    use std::path::{Path, PathBuf};

    use anyhow::Result;

    use crate::{
        generated_migrations,
        services::db::{DbSpec, TursoDb},
    };

    const PRE_HEALTH_INVARIANT_MIGRATION_COUNT: usize = 5;

    struct PreHealthInvariantDbSpec;

    impl DbSpec for PreHealthInvariantDbSpec {
        fn db_name() -> &'static str {
            "pre-006 repository Agent Trace DB"
        }

        fn db_path() -> Result<PathBuf> {
            anyhow::bail!("pre-006 repository Agent Trace DB has no canonical path")
        }

        fn migrations() -> &'static [(&'static str, &'static str)] {
            &generated_migrations::AGENT_TRACE_REPOSITORY_MIGRATIONS
                [..PRE_HEALTH_INVARIANT_MIGRATION_COUNT]
        }

        fn db_config_key() -> &'static str {
            "agent_trace_db"
        }
    }

    pub(crate) fn seed_pre_health_invariant_fixture(db_path: &Path, statements: &[&str]) {
        let fixture = TursoDb::<PreHealthInvariantDbSpec>::new_at(db_path)
            .expect("pre-006 fixture DB should migrate through 005");
        let applied_ids = fixture
            .query_map(
                "SELECT id FROM __sce_migrations ORDER BY id ASC",
                (),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .expect("migration metadata query should succeed");
        assert_eq!(
            applied_ids.last().map(String::as_str),
            Some("005_mutation_scope_provenance"),
            "the pre-006 fixture must stop at 005"
        );
        for statement in statements {
            fixture
                .execute(statement, ())
                .unwrap_or_else(|error| panic!("fixture row should insert: {error}"));
        }
        drop(fixture);
    }
}
