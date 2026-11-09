use std::path::PathBuf;

use anyhow::Result;
use uuid::Uuid;

use crate::{
    generated_migrations,
    services::db::{DatabaseKind, DbSpec, TursoDb},
};

use super::{
    diff_trace_patches_before_commit_with, insert_agent_trace_with,
    insert_conversation_text_event_with, insert_diff_trace_with, insert_messages_with,
    insert_parts_with, insert_post_commit_patch_intersection_with, recent_diff_trace_patches_with,
    upsert_claude_model_state_with, AgentTraceInsert, ClaudeModelStateObservation,
    CommitTimeCutoff, DiffTraceInsert, InsertMessageInsert, InsertPartInsert,
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

const INSERT_REPOSITORY_METADATA_SQL: &str =
    "INSERT INTO repository_metadata (id, repository_id) VALUES (1, ?1)
ON CONFLICT (id) DO NOTHING";

const CLAIM_SOURCE_INSTANCE_ID_SQL: &str =
    "UPDATE repository_metadata SET source_instance_id = ?1 WHERE id = 1 AND source_instance_id = ''";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryMetadata {
    pub repository_id: String,
    pub source_instance_id: String,
}

pub fn generate_source_instance_id() -> String {
    Uuid::new_v4().to_string()
}

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

#[derive(Debug)]
#[allow(
    dead_code,
    reason = "maintenance wiring lands in later tasks of context/plans/mutation-cursor-ref-reconciliation-wiring.md"
)]
pub enum ExistingRepositoryDbError {
    Missing { path: PathBuf },
    Unreadable(anyhow::Error),
    IncompatibleSchema(anyhow::Error),
    MissingMetadata,
    RepositoryMismatch { stored: String, resolved: String },
}

impl std::fmt::Display for ExistingRepositoryDbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExistingRepositoryDbError::Missing { path } => write!(
                f,
                "repository Agent Trace DB does not exist at '{}'. \
                 {REPOSITORY_AGENT_TRACE_SCHEMA_SETUP_GUIDANCE}",
                path.display()
            ),
            ExistingRepositoryDbError::Unreadable(source) => {
                write!(f, "repository Agent Trace DB is unreadable: {source}")
            }
            ExistingRepositoryDbError::IncompatibleSchema(source) => {
                write!(
                    f,
                    "repository Agent Trace DB schema is incompatible: {source}"
                )
            }
            ExistingRepositoryDbError::MissingMetadata => write!(
                f,
                "repository Agent Trace DB metadata is missing or has no valid source-instance ID. \
                 {REPOSITORY_AGENT_TRACE_SCHEMA_SETUP_GUIDANCE}"
            ),
            ExistingRepositoryDbError::RepositoryMismatch { stored, resolved } => write!(
                f,
                "repository Agent Trace DB metadata mismatch: stored repository ID {stored} \
                 does not match resolved repository ID {resolved}"
            ),
        }
    }
}

impl std::error::Error for ExistingRepositoryDbError {}

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

    const KIND: DatabaseKind = DatabaseKind::AgentTrace;
}

pub type RepositoryAgentTraceDb = TursoDb<RepositoryAgentTraceDbSpec>;

impl RepositoryAgentTraceDb {
    pub async fn open_for_hooks_without_migrations_at(
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self> {
        TursoDb::<RepositoryAgentTraceDbSpec>::open_without_migrations_at(path).await
    }

    #[allow(
        dead_code,
        reason = "maintenance wiring lands in later tasks of context/plans/mutation-cursor-ref-reconciliation-wiring.md"
    )]
    pub async fn open_verified_existing_at(
        path: impl AsRef<std::path::Path>,
        repository_id: &str,
    ) -> std::result::Result<(Self, RepositoryMetadata), ExistingRepositoryDbError> {
        Self::open_verified_existing_with(path.as_ref(), repository_id, || {}, || {}).await
    }

    #[cfg(test)]
    pub(crate) async fn open_verified_existing_at_with_hooks(
        path: &std::path::Path,
        repository_id: &str,
        before_open: impl FnOnce(),
        after_open: impl FnOnce(),
    ) -> std::result::Result<(Self, RepositoryMetadata), ExistingRepositoryDbError> {
        Self::open_verified_existing_with(path, repository_id, before_open, after_open).await
    }

    pub async fn open_existing_schema_ready_at(
        path: &std::path::Path,
        before_open: impl FnOnce(),
    ) -> std::result::Result<Self, ExistingRepositoryDbError> {
        Self::open_existing_schema_ready_with(path, before_open, || {}).await
    }

    async fn open_existing_schema_ready_with(
        path: &std::path::Path,
        before_open: impl FnOnce(),
        after_open: impl FnOnce(),
    ) -> std::result::Result<Self, ExistingRepositoryDbError> {
        before_open();
        let opened =
            TursoDb::<RepositoryAgentTraceDbSpec>::open_existing_without_migrations_at(path).await;
        after_open();
        let db = match opened {
            Ok(db) => db,
            Err(source) => {
                return Err(match std::fs::symlink_metadata(path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        ExistingRepositoryDbError::Missing {
                            path: path.to_path_buf(),
                        }
                    }
                    Ok(_) | Err(_) => ExistingRepositoryDbError::Unreadable(source),
                });
            }
        };
        db.ensure_schema_ready_for_hooks()
            .await
            .map_err(ExistingRepositoryDbError::IncompatibleSchema)?;
        Ok(db)
    }

    async fn open_verified_existing_with(
        path: &std::path::Path,
        repository_id: &str,
        before_open: impl FnOnce(),
        after_open: impl FnOnce(),
    ) -> std::result::Result<(Self, RepositoryMetadata), ExistingRepositoryDbError> {
        let db = Self::open_existing_schema_ready_with(path, before_open, after_open).await?;
        let metadata = db
            .verify_existing_repository_metadata(repository_id)
            .await?;
        Ok((db, metadata))
    }

    #[allow(
        dead_code,
        reason = "maintenance wiring lands in later tasks of context/plans/mutation-cursor-ref-reconciliation-wiring.md"
    )]
    pub async fn verify_existing_repository_metadata(
        &self,
        repository_id: &str,
    ) -> std::result::Result<RepositoryMetadata, ExistingRepositoryDbError> {
        let Some((stored_repository_id, source_instance_id)) = self
            .select_repository_metadata_row()
            .await
            .map_err(ExistingRepositoryDbError::Unreadable)?
        else {
            return Err(ExistingRepositoryDbError::MissingMetadata);
        };
        if stored_repository_id != repository_id {
            return Err(ExistingRepositoryDbError::RepositoryMismatch {
                stored: stored_repository_id,
                resolved: repository_id.to_string(),
            });
        }
        if !is_valid_source_instance_id(&source_instance_id) {
            return Err(ExistingRepositoryDbError::MissingMetadata);
        }
        Ok(RepositoryMetadata {
            repository_id: stored_repository_id,
            source_instance_id,
        })
    }

    pub async fn ensure_schema_ready_for_hooks(&self) -> Result<()> {
        self.ensure_schema_ready(REPOSITORY_AGENT_TRACE_SCHEMA_SETUP_GUIDANCE)
            .await
    }

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

    pub async fn insert_diff_trace(&self, input: DiffTraceInsert<'_>) -> Result<u64> {
        insert_diff_trace_with(self, input).await
    }

    pub async fn insert_post_commit_patch_intersection(
        &self,
        input: PostCommitPatchIntersectionInsert<'_>,
    ) -> Result<u64> {
        insert_post_commit_patch_intersection_with(self, input).await
    }

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

    pub async fn recent_diff_trace_patches(
        &self,
        cutoff_time_ms: i64,
        end_time_ms: i64,
    ) -> Result<RecentDiffTracePatches> {
        recent_diff_trace_patches_with(self, cutoff_time_ms, end_time_ms).await
    }

    pub async fn recent_diff_trace_patches_before_commit(
        &self,
        lower_time_ms: i64,
        cutoff: CommitTimeCutoff,
    ) -> Result<RecentDiffTracePatches> {
        diff_trace_patches_before_commit_with(self, lower_time_ms, cutoff).await
    }

    pub async fn insert_messages(&self, inputs: Vec<InsertMessageInsert>) -> Result<u64> {
        insert_messages_with(self, inputs).await
    }

    pub async fn insert_parts(&self, inputs: Vec<InsertPartInsert>) -> Result<u64> {
        insert_parts_with(self, inputs).await
    }

    pub async fn insert_conversation_text_event(
        &mut self,
        message: InsertMessageInsert,
        part: InsertPartInsert,
    ) -> Result<bool> {
        insert_conversation_text_event_with(self, message, part, false).await
    }
}
