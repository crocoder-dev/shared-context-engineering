use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;

use crate::services::agent_trace_db::repository::{RepositoryAgentTraceDb, RepositoryMetadata};
use crate::services::default_paths::{
    agent_trace_db_path_for_repository, agent_trace_db_path_for_repository_at,
};
use crate::services::repository_identity::resolve::{
    resolve_repository_identity, ResolvedRepositoryIdentity,
};

const REPOSITORY_DB_INITIALIZATION_ATTEMPTS: usize = 20;
const REPOSITORY_DB_INITIALIZATION_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Inputs needed to resolve the active repository-scoped Agent Trace storage.
///
/// The identity inputs mirror the `agent_trace.repository_id` and
/// `agent_trace.repository_remote` configuration keys; callers pass the
/// already-resolved configuration values.
#[derive(Clone, Copy, Debug)]
pub struct AgentTraceStorageContext<'a> {
    /// Root of the Git working tree the current command runs in.
    pub repository_root: &'a Path,
    /// Explicit `agent_trace.repository_id` configuration value, if set.
    pub explicit_repository_id: Option<&'a str>,
    /// Configured `agent_trace.repository_remote` name (default `origin`).
    pub repository_remote: &'a str,
}

/// The resolved active Agent Trace storage for one repository checkout.
pub struct ResolvedAgentTraceStorage {
    /// Repository identity (canonical identity plus repository ID) and the
    /// source it was resolved from.
    pub repository_identity: ResolvedRepositoryIdentity,
    /// Repository-scoped database path
    /// `<state-root>/sce/repos/<repository-id>/agent-trace.db`.
    pub db_path: PathBuf,
    /// Open repository-scoped Agent Trace database.
    pub db: RepositoryAgentTraceDb,
    /// Physical database identity (`repository_id`/`source_instance_id`)
    /// produced by the same verification/initialization call that opened
    /// `db`.
    pub metadata: RepositoryMetadata,
}

/// Resolves the repository-scoped Agent Trace storage for a checkout using
/// the canonical state root from the default-path catalog.
pub async fn resolve_agent_trace_storage(
    context: &AgentTraceStorageContext<'_>,
) -> Result<ResolvedAgentTraceStorage> {
    let repository_identity = resolve_identity(context)?;
    let db_path = agent_trace_db_path_for_repository(&repository_identity.identity.repository_id)?;
    open_storage(repository_identity, db_path).await
}

/// Resolves repository-scoped Agent Trace storage for high-frequency hook
/// runtime callers using the canonical state root.
///
/// Never runs migrations, including migration `002`: the database must
/// already have been brought up to date by `sce setup`. A missing or
/// baseline-only (pre-`002`) database fails with the same `sce setup`
/// guidance `ensure_schema_ready_for_hooks` already reports, rather than
/// silently migrating it from a hook path.
pub async fn resolve_agent_trace_storage_for_hook_runtime(
    context: &AgentTraceStorageContext<'_>,
) -> Result<ResolvedAgentTraceStorage> {
    let repository_identity = resolve_identity(context)?;
    let db_path = agent_trace_db_path_for_repository(&repository_identity.identity.repository_id)?;
    open_storage_for_hook_runtime(repository_identity, db_path).await
}

/// Hook-runtime resolution core against an explicit state root, so tests can
/// exercise the full path without touching the real user state directory.
pub async fn resolve_agent_trace_storage_for_hook_runtime_at_state_root(
    context: &AgentTraceStorageContext<'_>,
    state_root: &Path,
) -> Result<ResolvedAgentTraceStorage> {
    let repository_identity = resolve_identity(context)?;
    let db_path = agent_trace_db_path_for_repository_at(
        state_root,
        &repository_identity.identity.repository_id,
    )?;
    open_storage_for_hook_runtime(repository_identity, db_path).await
}

fn resolve_identity(context: &AgentTraceStorageContext<'_>) -> Result<ResolvedRepositoryIdentity> {
    resolve_repository_identity(
        context.repository_root,
        context.explicit_repository_id,
        context.repository_remote,
    )
    .map_err(|error| anyhow::anyhow!("{error}"))
}

async fn open_storage(
    repository_identity: ResolvedRepositoryIdentity,
    db_path: PathBuf,
) -> Result<ResolvedAgentTraceStorage> {
    open_storage_with(
        repository_identity,
        db_path,
        open_repository_db_concurrently_safe,
    )
    .await
}

async fn open_storage_for_hook_runtime(
    repository_identity: ResolvedRepositoryIdentity,
    db_path: PathBuf,
) -> Result<ResolvedAgentTraceStorage> {
    open_storage_with(
        repository_identity,
        db_path,
        open_repository_db_for_hook_runtime,
    )
    .await
}

async fn open_storage_with(
    repository_identity: ResolvedRepositoryIdentity,
    db_path: PathBuf,
    open_db: impl std::ops::AsyncFnOnce(
        &Path,
        &str,
    ) -> Result<(RepositoryAgentTraceDb, RepositoryMetadata)>,
) -> Result<ResolvedAgentTraceStorage> {
    let repository_id = &repository_identity.identity.repository_id;
    let (db, metadata) = open_db(&db_path, repository_id).await?;

    Ok(ResolvedAgentTraceStorage {
        repository_identity,
        db_path,
        db,
        metadata,
    })
}

async fn open_repository_db_concurrently_safe(
    db_path: &Path,
    repository_id: &str,
) -> Result<(RepositoryAgentTraceDb, RepositoryMetadata)> {
    let mut last_error = None;

    for attempt in 1..=REPOSITORY_DB_INITIALIZATION_ATTEMPTS {
        let fast_open = async {
            let db = RepositoryAgentTraceDb::open_without_migrations_at(db_path).await?;
            if db.ensure_schema_ready_for_hooks().await.is_err() {
                db.repair_missing_repository_schema_migration_metadata()
                    .await?;
            }
            let metadata = db
                .verify_or_initialize_repository_metadata(repository_id)
                .await?;
            Ok::<_, anyhow::Error>((db, metadata))
        }
        .await;

        match fast_open {
            Ok(result) => return Ok(result),
            Err(fast_error) => match RepositoryAgentTraceDb::new_at(db_path).await {
                Ok(db) => {
                    let metadata = db
                        .verify_or_initialize_repository_metadata(repository_id)
                        .await?;
                    return Ok((db, metadata));
                }
                Err(init_error) => {
                    last_error = Some(anyhow::anyhow!(
                        "failed to initialize repository-scoped Agent Trace DB for repository {} at '{}' (fast-path attempt: {fast_error})",
                        repository_id,
                        db_path.display()
                    )
                    .context(init_error));
                    if attempt < REPOSITORY_DB_INITIALIZATION_ATTEMPTS {
                        tokio::time::sleep(REPOSITORY_DB_INITIALIZATION_RETRY_DELAY).await;
                    }
                }
            },
        }
    }

    Err(last_error.expect("repository DB initialization should record an error"))
}

/// No-migration open for high-frequency hook-runtime callers.
///
/// Verifies schema readiness and applies only the same narrow
/// concurrent-first-open migration-metadata repair the setup/lifecycle fast
/// path applies, then initializes `source_instance_id` once readiness is
/// confirmed. Never falls back to running migrations: a missing or
/// baseline-only (pre-`002`) database fails with the existing `sce setup`
/// guidance instead.
async fn open_repository_db_for_hook_runtime(
    db_path: &Path,
    repository_id: &str,
) -> Result<(RepositoryAgentTraceDb, RepositoryMetadata)> {
    let db = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(db_path).await?;

    if db.ensure_schema_ready_for_hooks().await.is_err() {
        db.repair_missing_repository_schema_migration_metadata()
            .await?;
    }

    let metadata = db
        .verify_or_initialize_repository_metadata(repository_id)
        .await?;
    Ok((db, metadata))
}

#[allow(
    dead_code,
    reason = "maintenance wiring lands in later tasks of context/plans/mutation-cursor-ref-reconciliation-wiring.md"
)]
pub async fn resolve_existing_agent_trace_storage_for_maintenance(
    context: &AgentTraceStorageContext<'_>,
) -> Result<ResolvedAgentTraceStorage> {
    let repository_identity = resolve_identity(context)?;
    let db_path = agent_trace_db_path_for_repository(&repository_identity.identity.repository_id)?;
    open_existing_storage_for_maintenance(repository_identity, db_path).await
}

#[allow(
    dead_code,
    reason = "maintenance wiring lands in later tasks of context/plans/mutation-cursor-ref-reconciliation-wiring.md"
)]
async fn open_existing_storage_for_maintenance(
    repository_identity: ResolvedRepositoryIdentity,
    db_path: PathBuf,
) -> Result<ResolvedAgentTraceStorage> {
    let repository_id = &repository_identity.identity.repository_id;
    let (db, metadata) =
        RepositoryAgentTraceDb::open_verified_existing_at(&db_path, repository_id).await?;

    Ok(ResolvedAgentTraceStorage {
        repository_identity,
        db_path,
        db,
        metadata,
    })
}

#[cfg(test)]
pub(crate) async fn resolve_existing_agent_trace_storage_for_maintenance_at_state_root(
    context: &AgentTraceStorageContext<'_>,
    state_root: &Path,
) -> Result<ResolvedAgentTraceStorage> {
    let repository_identity = resolve_identity(context)?;
    let db_path = agent_trace_db_path_for_repository_at(
        state_root,
        &repository_identity.identity.repository_id,
    )?;
    open_existing_storage_for_maintenance(repository_identity, db_path).await
}
