use std::path::PathBuf;

use anyhow::Result;
use uuid::Uuid;

use crate::{
    generated_migrations,
    services::db::{DbSpec, TursoDb},
};

use super::{
    insert_agent_trace_with, insert_conversation_text_event_with, insert_diff_trace_with,
    insert_messages_with, insert_parts_with, insert_post_commit_patch_intersection_with,
    recent_diff_trace_patches_with, upsert_claude_model_state_with, AgentTraceInsert,
    ClaudeModelStateObservation, DiffTraceInsert, InsertMessageInsert, InsertPartInsert,
    PostCommitPatchIntersectionInsert, RecentDiffTracePatches,
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

fn ensure_repository_id_matches(stored_repository_id: &str, repository_id: &str) -> Result<()> {
    if stored_repository_id != repository_id {
        anyhow::bail!(
            "repository Agent Trace DB metadata mismatch: stored repository ID \
             {stored_repository_id} does not match resolved repository ID {repository_id}"
        );
    }
    Ok(())
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
    pub async fn open_for_hooks_without_migrations_at(
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self> {
        TursoDb::<RepositoryAgentTraceDbSpec>::open_without_migrations_at(path).await
    }

    /// Verify that the repository-scoped schema baseline already exists.
    pub async fn ensure_schema_ready_for_hooks(&self) -> Result<()> {
        self.ensure_schema_ready(REPOSITORY_AGENT_TRACE_SCHEMA_SETUP_GUIDANCE)
            .await
    }

    /// Repair the narrow concurrent-initialization case where the one-file
    /// schema batch completed but recording `__sce_migrations` raced with
    /// another first opener. This never creates trace tables; it only records
    /// the baseline migration after all required repository tables already
    /// exist.
    pub async fn repair_missing_repository_schema_migration_metadata(&self) -> Result<()> {
        for table in REQUIRED_REPOSITORY_SCHEMA_TABLES {
            if !self.sqlite_object_exists("table", table).await? {
                anyhow::bail!(
                    "repository Agent Trace DB schema is incomplete; missing table {table}. \
                     {REPOSITORY_AGENT_TRACE_SCHEMA_SETUP_GUIDANCE}"
                );
            }
        }

        self.execute(RECORD_REPOSITORY_SCHEMA_MIGRATION_SQL, ())
            .await?;
        self.ensure_schema_ready_for_hooks().await
    }

    async fn sqlite_object_exists(&self, object_type: &str, name: &str) -> Result<bool> {
        let rows = self
            .query_map(SELECT_SQLITE_OBJECT_SQL, (object_type, name), |row| {
                row.get::<String>(0).map_err(Into::into)
            })
            .await?;
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
    pub async fn verify_or_initialize_repository_metadata(
        &self,
        repository_id: &str,
    ) -> Result<RepositoryMetadata> {
        if let Some((stored_repository_id, source_instance_id)) =
            self.select_repository_metadata_row().await?
        {
            ensure_repository_id_matches(&stored_repository_id, repository_id)?;
            if is_valid_source_instance_id(&source_instance_id) {
                return Ok(RepositoryMetadata {
                    repository_id: stored_repository_id,
                    source_instance_id,
                });
            }
        }

        self.execute_idempotent_write(INSERT_REPOSITORY_METADATA_SQL, (repository_id,))
            .await?;

        let Some((stored_repository_id, source_instance_id)) =
            self.select_repository_metadata_row().await?
        else {
            anyhow::bail!(
                "repository Agent Trace DB metadata is missing its repository ID row. \
                 {REPOSITORY_AGENT_TRACE_SCHEMA_SETUP_GUIDANCE}"
            );
        };

        ensure_repository_id_matches(&stored_repository_id, repository_id)?;

        if is_valid_source_instance_id(&source_instance_id) {
            return Ok(RepositoryMetadata {
                repository_id: stored_repository_id,
                source_instance_id,
            });
        }

        let candidate = generate_source_instance_id();
        self.execute_idempotent_write(CLAIM_SOURCE_INSTANCE_ID_SQL, (candidate.as_str(),))
            .await?;

        let (final_repository_id, final_source_instance_id) = self
            .select_repository_metadata_row()
            .await?
            .ok_or_else(|| {
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

    async fn select_repository_metadata_row(&self) -> Result<Option<(String, String)>> {
        let rows = self
            .query_map(SELECT_REPOSITORY_METADATA_SQL, (), |row| {
                Ok((row.get::<String>(0)?, row.get::<String>(1)?))
            })
            .await?;
        Ok(rows.into_iter().next())
    }

    /// Insert a diff trace payload into the repository-scoped `diff_traces`
    /// table. Rows remain repository-level; no checkout provenance is stored.
    pub async fn insert_diff_trace(&self, input: DiffTraceInsert<'_>) -> Result<u64> {
        insert_diff_trace_with(self, input).await
    }

    /// Insert a post-commit patch intersection into the repository-scoped
    /// `post_commit_patch_intersections` table.
    pub async fn insert_post_commit_patch_intersection(
        &self,
        input: PostCommitPatchIntersectionInsert<'_>,
    ) -> Result<u64> {
        insert_post_commit_patch_intersection_with(self, input).await
    }

    /// Insert a built Agent Trace payload into the repository-scoped
    /// `agent_traces` table.
    pub async fn insert_agent_trace(&self, input: AgentTraceInsert<'_>) -> Result<u64> {
        insert_agent_trace_with(self, input).await
    }

    pub async fn upsert_claude_model_state(
        &self,
        input: ClaudeModelStateObservation,
    ) -> Result<u64> {
        upsert_claude_model_state_with(self, input).await
    }

    pub async fn claude_model_state_by_session_and_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<ClaudeModelStateObservation>> {
        super::claude_model_state_by_session_and_agent_with(self, session_id, agent_id).await
    }

    /// Query and parse recent diff trace patches within the inclusive time
    /// window for this repository-scoped database. Rows remain repository-level;
    /// no checkout filter or checkout provenance is applied.
    pub async fn recent_diff_trace_patches(
        &self,
        cutoff_time_ms: i64,
        end_time_ms: i64,
    ) -> Result<RecentDiffTracePatches> {
        recent_diff_trace_patches_with(self, cutoff_time_ms, end_time_ms).await
    }

    /// Insert message rows with one multi-row statement, ignoring duplicate
    /// `(session_id, message_id)` rows.
    pub async fn insert_messages(&self, inputs: Vec<InsertMessageInsert>) -> Result<u64> {
        insert_messages_with(self, inputs).await
    }

    /// Append part rows with one multi-row statement.
    pub async fn insert_parts(&self, inputs: Vec<InsertPartInsert>) -> Result<u64> {
        insert_parts_with(self, inputs).await
    }

    /// Atomically insert one conversation `messages` row and its one
    /// `parts` row: if `(message.session_id, message.message_id)` already
    /// exists, this is a no-op (`Ok(false)`); otherwise both rows insert
    /// together in one transaction (`Ok(true)`). Used by conversation
    /// text-event handlers (e.g. Codex `UserPromptSubmit`/`Stop`) in place
    /// of separate `insert_messages`/`insert_parts` calls, so a replayed or
    /// concurrent duplicate delivery never produces an orphaned `parts` row.
    pub async fn insert_conversation_text_event(
        &mut self,
        message: InsertMessageInsert,
        part: InsertPartInsert,
    ) -> Result<bool> {
        insert_conversation_text_event_with(self, message, part, false).await
    }
}
