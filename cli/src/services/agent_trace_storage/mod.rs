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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{
        open_repository_db_concurrently_safe, open_repository_db_for_hook_runtime,
        resolve_existing_agent_trace_storage_for_maintenance_at_state_root,
        AgentTraceStorageContext,
    };
    use crate::generated_migrations::AGENT_TRACE_REPOSITORY_MIGRATIONS;
    use crate::services::agent_trace_db::repository::{
        ExistingRepositoryDbError, RepositoryAgentTraceDb,
    };
    use crate::services::agent_trace_db::AgentTraceInsert;
    use crate::services::db::{DatabaseKind, DbSpec, TursoDb};

    const COMMIT_PATCH_ID_MIGRATION: &str = "007_commit_patch_id";
    const PRE_COMMIT_PATCH_ID_REPOSITORY_ID: &str = "commit-patch-id-repo";
    const PRE_COMMIT_PATCH_ID_SOURCE_INSTANCE_ID: &str = "pre-007-source-instance";
    const PRE_COMMIT_PATCH_ID_TRACE_JSON: &str = "{\n  \"version\": \"0.1.0\"\n}\n";

    struct Pre007RepositoryAgentTraceDbSpec;

    impl DbSpec for Pre007RepositoryAgentTraceDbSpec {
        fn db_name() -> &'static str {
            "pre-007 repository Agent Trace DB"
        }

        fn db_path() -> anyhow::Result<PathBuf> {
            anyhow::bail!("pre-007 fixture uses explicit paths")
        }

        fn migrations() -> &'static [(&'static str, &'static str)] {
            let position = AGENT_TRACE_REPOSITORY_MIGRATIONS
                .iter()
                .position(|(id, _)| *id == COMMIT_PATCH_ID_MIGRATION)
                .expect("commit patch ID migration is embedded");
            &AGENT_TRACE_REPOSITORY_MIGRATIONS[..position]
        }

        const KIND: DatabaseKind = DatabaseKind::AgentTrace;
    }

    type AgentTraceRow = (
        i64,
        String,
        i64,
        String,
        String,
        String,
        String,
        Option<String>,
    );

    async fn create_pre_commit_patch_id_db(path: &Path) {
        let db = TursoDb::<Pre007RepositoryAgentTraceDbSpec>::new_at(path)
            .await
            .expect("pre-007 database");
        db.execute(
            "INSERT INTO repository_metadata (id, repository_id, source_instance_id) VALUES (1, ?1, ?2)",
            (
                PRE_COMMIT_PATCH_ID_REPOSITORY_ID,
                PRE_COMMIT_PATCH_ID_SOURCE_INSTANCE_ID,
            ),
        )
        .await
        .expect("seed metadata");
        db.execute(
            "INSERT INTO agent_traces (id, commit_id, commit_time_ms, trace_json, agent_trace_id, url, remote_url) VALUES (41, 'abc123', 7, ?1, 'trace-41', 'https://example.test/trace-41', 'https://example.test/repo.git')",
            (PRE_COMMIT_PATCH_ID_TRACE_JSON,),
        )
        .await
        .expect("seed pre-007 trace");
    }

    async fn applied_migration_ids(db: &RepositoryAgentTraceDb) -> Vec<String> {
        db.query_map(
            "SELECT id FROM __sce_migrations ORDER BY id ASC",
            (),
            |row| row.get::<String>(0).map_err(Into::into),
        )
        .await
        .expect("migration ledger")
    }

    async fn agent_trace_columns(db: &RepositoryAgentTraceDb) -> Vec<(String, i64)> {
        db.query_map("PRAGMA table_info(agent_traces)", (), |row| {
            Ok((row.get::<String>(1)?, row.get::<i64>(3)?))
        })
        .await
        .expect("agent_traces table info")
    }

    async fn agent_trace_rows(db: &RepositoryAgentTraceDb) -> Vec<AgentTraceRow> {
        db.query_map(
            "SELECT id, commit_id, commit_time_ms, trace_json, agent_trace_id, url, remote_url, commit_patch_id FROM agent_traces ORDER BY id ASC",
            (),
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .await
        .expect("agent_traces rows")
    }

    fn expected_migration_ids() -> Vec<String> {
        AGENT_TRACE_REPOSITORY_MIGRATIONS
            .iter()
            .map(|(id, _)| (*id).to_string())
            .collect()
    }

    fn pre_commit_patch_id_row() -> AgentTraceRow {
        (
            41,
            String::from("abc123"),
            7,
            String::from(PRE_COMMIT_PATCH_ID_TRACE_JSON),
            String::from("trace-41"),
            String::from("https://example.test/trace-41"),
            String::from("https://example.test/repo.git"),
            None,
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn commit_patch_id_setup_upgrades_pre_007_database_and_preserves_rows() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("agent-trace.db");
        create_pre_commit_patch_id_db(&path).await;

        for _ in 0..2 {
            let (db, metadata) =
                open_repository_db_concurrently_safe(&path, PRE_COMMIT_PATCH_ID_REPOSITORY_ID)
                    .await
                    .expect("setup open");

            assert_eq!(
                metadata.source_instance_id,
                PRE_COMMIT_PATCH_ID_SOURCE_INSTANCE_ID
            );
            assert_eq!(applied_migration_ids(&db).await, expected_migration_ids());
            assert!(agent_trace_columns(&db)
                .await
                .contains(&(String::from("commit_patch_id"), 0)));
            assert_eq!(agent_trace_rows(&db).await, vec![pre_commit_patch_id_row()]);
            db.ensure_schema_ready_for_hooks()
                .await
                .expect("upgraded schema is hook-ready");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn commit_patch_id_hook_runtime_and_doctor_refuse_pre_007_database() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("agent-trace.db");
        create_pre_commit_patch_id_db(&path).await;

        let Err(hook_error) =
            open_repository_db_for_hook_runtime(&path, PRE_COMMIT_PATCH_ID_REPOSITORY_ID).await
        else {
            panic!("hook runtime must not open a pre-007 database");
        };
        let hook_message = format!("{hook_error:#}");
        assert!(hook_message.contains("Run 'sce setup'."), "{hook_message}");
        assert!(
            hook_message.contains(COMMIT_PATCH_ID_MIGRATION),
            "{hook_message}"
        );

        let Err(doctor_error) =
            RepositoryAgentTraceDb::open_existing_schema_ready_at(&path, || {}).await
        else {
            panic!("doctor readiness must reject a pre-007 database");
        };
        assert!(matches!(
            doctor_error,
            ExistingRepositoryDbError::IncompatibleSchema(_)
        ));

        let db = RepositoryAgentTraceDb::open_existing_without_migrations_at(&path)
            .await
            .expect("read-only open");
        assert!(!applied_migration_ids(&db)
            .await
            .iter()
            .any(|id| id == COMMIT_PATCH_ID_MIGRATION));
        assert!(!agent_trace_columns(&db)
            .await
            .iter()
            .any(|(name, _)| name == "commit_patch_id"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn commit_patch_id_insert_persists_present_and_absent_values() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("agent-trace.db");
        let (db, _metadata) =
            open_repository_db_concurrently_safe(&path, PRE_COMMIT_PATCH_ID_REPOSITORY_ID)
                .await
                .expect("setup open");

        for (agent_trace_id, commit_patch_id) in [
            (
                "trace-with-patch-id",
                Some("0123456789abcdef0123456789abcdef01234567"),
            ),
            ("trace-without-patch-id", None),
        ] {
            db.insert_agent_trace(AgentTraceInsert {
                commit_id: "fedcba9876543210fedcba9876543210fedcba98",
                commit_time_ms: 11,
                trace_json: PRE_COMMIT_PATCH_ID_TRACE_JSON,
                agent_trace_id,
                url: "https://example.test/trace",
                remote_url: "https://example.test/repo.git",
                commit_patch_id,
            })
            .await
            .expect("insert agent trace");
        }

        let stored = db
            .query_map(
                "SELECT agent_trace_id, commit_id, trace_json, commit_patch_id FROM agent_traces ORDER BY id ASC",
                (),
                |row| {
                    Ok((
                        row.get::<String>(0)?,
                        row.get::<String>(1)?,
                        row.get::<String>(2)?,
                        row.get::<Option<String>>(3)?,
                    ))
                },
            )
            .await
            .expect("stored traces");

        assert_eq!(
            stored,
            vec![
                (
                    String::from("trace-with-patch-id"),
                    String::from("fedcba9876543210fedcba9876543210fedcba98"),
                    String::from(PRE_COMMIT_PATCH_ID_TRACE_JSON),
                    Some(String::from("0123456789abcdef0123456789abcdef01234567")),
                ),
                (
                    String::from("trace-without-patch-id"),
                    String::from("fedcba9876543210fedcba9876543210fedcba98"),
                    String::from(PRE_COMMIT_PATCH_ID_TRACE_JSON),
                    None,
                ),
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn agent_trace_storage_maintenance_open_of_missing_db_creates_nothing() {
        let repository = tempfile::tempdir().expect("repository dir");
        let state_root = tempfile::tempdir().expect("state root");
        let context = AgentTraceStorageContext {
            repository_root: repository.path(),
            explicit_repository_id: Some("maintenance-open-missing"),
            repository_remote: "origin",
        };

        let Err(error) = resolve_existing_agent_trace_storage_for_maintenance_at_state_root(
            &context,
            state_root.path(),
        )
        .await
        else {
            panic!("a missing database must not open");
        };

        assert!(matches!(
            error.downcast_ref::<ExistingRepositoryDbError>(),
            Some(ExistingRepositoryDbError::Missing { .. })
        ));
        let entries: Vec<_> = std::fs::read_dir(state_root.path())
            .expect("state root listing")
            .collect();
        assert!(
            entries.is_empty(),
            "state root must stay empty: {entries:?}"
        );
    }
}
