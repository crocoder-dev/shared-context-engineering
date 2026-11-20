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

#[derive(Clone, Copy, Debug)]
pub struct AgentTraceStorageContext<'a> {
    pub repository_root: &'a Path,
    pub explicit_repository_id: Option<&'a str>,
    pub repository_remote: &'a str,
}

pub struct ResolvedAgentTraceStorage {
    pub repository_identity: ResolvedRepositoryIdentity,
    pub db_path: PathBuf,
    pub db: RepositoryAgentTraceDb,
    pub metadata: RepositoryMetadata,
}

pub async fn resolve_agent_trace_storage(
    context: &AgentTraceStorageContext<'_>,
) -> Result<ResolvedAgentTraceStorage> {
    let repository_identity = resolve_identity(context)?;
    let db_path = agent_trace_db_path_for_repository(&repository_identity.identity.repository_id)?;
    open_storage(repository_identity, db_path).await
}

pub async fn resolve_agent_trace_storage_for_hook_runtime(
    context: &AgentTraceStorageContext<'_>,
) -> Result<ResolvedAgentTraceStorage> {
    let repository_identity = resolve_identity(context)?;
    let db_path = agent_trace_db_path_for_repository(&repository_identity.identity.repository_id)?;
    open_storage_for_hook_runtime(repository_identity, db_path).await
}

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

    if db.ensure_schema_ready_for_hook_runtime().await.is_err() {
        db.repair_missing_repository_schema_migration_metadata_for_hook_runtime()
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
    use crate::services::agent_trace_db::{
        AgentTraceInsert, ClaudeModelStateObservation, ClaudeSessionAuthState,
        ClaudeSessionAuthStateUpsertOutcome, DiffTraceInsert, ObservationKind,
    };
    use crate::services::db::{DatabaseKind, DbSpec, TursoDb};
    use crate::services::hooks::claude_auth_detection::{
        AuthEnvKey, AuthProvider, BillingMode, DetectionConfidence,
    };

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

    const CLAUDE_SESSION_AUTH_STATE_MIGRATION: &str = "008_claude_session_auth_state";
    const PRE_CLAUDE_SESSION_AUTH_REPOSITORY_ID: &str = "claude-session-auth-repo";

    struct Pre008RepositoryAgentTraceDbSpec;

    impl DbSpec for Pre008RepositoryAgentTraceDbSpec {
        fn db_name() -> &'static str {
            "pre-008 repository Agent Trace DB"
        }

        fn db_path() -> anyhow::Result<PathBuf> {
            anyhow::bail!("pre-008 fixture uses explicit paths")
        }

        fn migrations() -> &'static [(&'static str, &'static str)] {
            let position = AGENT_TRACE_REPOSITORY_MIGRATIONS
                .iter()
                .position(|(id, _)| *id == CLAUDE_SESSION_AUTH_STATE_MIGRATION)
                .expect("Claude session auth state migration is embedded");
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

    async fn create_pre_claude_session_auth_db(path: &Path) {
        let db = TursoDb::<Pre008RepositoryAgentTraceDbSpec>::new_at(path)
            .await
            .expect("pre-008 database");
        db.execute(
            "INSERT INTO repository_metadata (id, repository_id, source_instance_id) VALUES (1, ?1, 'pre-008-source-instance')",
            (PRE_CLAUDE_SESSION_AUTH_REPOSITORY_ID,),
        )
        .await
        .expect("seed metadata");
    }

    async fn create_current_db(path: &Path) -> RepositoryAgentTraceDb {
        let (db, _metadata) =
            open_repository_db_concurrently_safe(path, PRE_CLAUDE_SESSION_AUTH_REPOSITORY_ID)
                .await
                .expect("setup open");
        db
    }

    async fn claude_session_auth_state_table_exists(db: &RepositoryAgentTraceDb) -> bool {
        !db.query_map(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'claude_session_auth_state'",
            (),
            |row| row.get::<String>(0).map_err(Into::into),
        )
        .await
        .expect("sqlite_master lookup")
        .is_empty()
    }

    async fn assert_hook_runtime_rejects(path: &Path, expected_problem: &str) {
        let Err(error) =
            open_repository_db_for_hook_runtime(path, PRE_CLAUDE_SESSION_AUTH_REPOSITORY_ID).await
        else {
            panic!("hook runtime must reject a ledger with {expected_problem}");
        };
        let message = format!("{error:#}");
        assert!(message.contains("Run 'sce setup'."), "{message}");
        assert!(message.contains(expected_problem), "{message}");
    }

    #[test]
    fn agent_trace_repository_migrations_end_with_claude_session_auth_state() {
        let ids: Vec<&str> = AGENT_TRACE_REPOSITORY_MIGRATIONS
            .iter()
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(
            ids,
            vec![
                "001_repository_schema",
                "002_repository_source_instance_id",
                "003_claude_model_state",
                "004_mutation_trace_protocol",
                "005_mutation_scope_provenance",
                "006_mutation_trace_health_invariant",
                COMMIT_PATCH_ID_MIGRATION,
                CLAUDE_SESSION_AUTH_STATE_MIGRATION,
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn agent_trace_repository_migrations_create_constrained_claude_session_auth_state() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = create_current_db(&dir.path().join("agent-trace.db")).await;

        let columns = db
            .query_map("PRAGMA table_info(claude_session_auth_state)", (), |row| {
                Ok((
                    row.get::<String>(1)?,
                    row.get::<i64>(3)?,
                    row.get::<i64>(5)?,
                ))
            })
            .await
            .expect("claude_session_auth_state table info");
        assert_eq!(
            columns,
            vec![
                (String::from("session_id"), 0, 1),
                (String::from("provider"), 1, 0),
                (String::from("billing_mode"), 1, 0),
                (String::from("auth_env_key"), 0, 0),
                (String::from("confidence"), 1, 0),
                (String::from("observed_at_ms"), 1, 0),
            ]
        );

        db.execute(
            "INSERT INTO claude_session_auth_state (session_id, provider, billing_mode, auth_env_key, confidence, observed_at_ms) VALUES ('claude:valid', 'aws_bedrock', 'cloud_provider', 'claude_code_use_bedrock', 'inferred', 0)",
            (),
        )
        .await
        .expect("valid row");
        db.execute(
            "INSERT INTO claude_session_auth_state (session_id, provider, billing_mode, auth_env_key, confidence, observed_at_ms) VALUES ('claude:unknown', 'unknown', 'unknown', NULL, 'unknown', 5)",
            (),
        )
        .await
        .expect("unknown row with no env key");

        for (column, invalid_row) in [
            (
                "provider",
                "('claude:bad', 'openai', 'unknown', NULL, 'unknown', 1)",
            ),
            (
                "billing_mode",
                "('claude:bad', 'unknown', 'free', NULL, 'unknown', 1)",
            ),
            (
                "auth_env_key",
                "('claude:bad', 'unknown', 'unknown', 'anthropic_base_url', 'unknown', 1)",
            ),
            (
                "confidence",
                "('claude:bad', 'unknown', 'unknown', NULL, 'verified', 1)",
            ),
            (
                "observed_at_ms",
                "('claude:bad', 'unknown', 'unknown', NULL, 'unknown', -1)",
            ),
        ] {
            let insert = format!(
                "INSERT INTO claude_session_auth_state (session_id, provider, billing_mode, auth_env_key, confidence, observed_at_ms) VALUES {invalid_row}"
            );
            assert!(
                db.execute(&insert, ()).await.is_err(),
                "invalid {column} must be rejected"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_session_auth_schema_compat_hook_runtime_opens_pre_008_database() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("agent-trace.db");
        create_pre_claude_session_auth_db(&path).await;

        let (db, metadata) =
            open_repository_db_for_hook_runtime(&path, PRE_CLAUDE_SESSION_AUTH_REPOSITORY_ID)
                .await
                .expect("hook runtime opens pre-008 database");

        assert_eq!(
            metadata.repository_id,
            PRE_CLAUDE_SESSION_AUTH_REPOSITORY_ID
        );
        assert!(!applied_migration_ids(&db)
            .await
            .iter()
            .any(|id| id == CLAUDE_SESSION_AUTH_STATE_MIGRATION));
        assert!(!claude_session_auth_state_table_exists(&db).await);

        let observation = ClaudeModelStateObservation {
            session_id: String::from("claude:pre-008-session"),
            agent_id: String::new(),
            model_id: String::from("claude-opus-5-5"),
            observation_kind: ObservationKind::SessionStart,
            source: String::from("startup"),
            observed_at_ms: 3,
        };
        db.upsert_claude_model_state(observation.clone())
            .await
            .expect("model state persists on pre-008 database");
        assert_eq!(
            db.claude_model_state_by_session_and_agent("claude:pre-008-session", "")
                .await
                .expect("model state read"),
            Some(observation)
        );
        db.insert_diff_trace(DiffTraceInsert {
            time_ms: 4,
            session_id: "claude:pre-008-session",
            patch: "diff --git a/f b/f\n",
            model_id: None,
            tool_name: "claude",
            tool_version: None,
            payload_type: "claude_post_tool_use",
        })
        .await
        .expect("diff trace persists on pre-008 database");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_session_auth_schema_compat_hook_runtime_opens_current_database() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("agent-trace.db");
        create_current_db(&path).await;

        let (db, _metadata) =
            open_repository_db_for_hook_runtime(&path, PRE_CLAUDE_SESSION_AUTH_REPOSITORY_ID)
                .await
                .expect("hook runtime opens current database");
        assert_eq!(applied_migration_ids(&db).await, expected_migration_ids());
        assert!(claude_session_auth_state_table_exists(&db).await);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_session_auth_schema_compat_setup_applies_008_to_pre_008_database() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("agent-trace.db");
        create_pre_claude_session_auth_db(&path).await;

        let db = create_current_db(&path).await;

        assert_eq!(applied_migration_ids(&db).await, expected_migration_ids());
        assert!(claude_session_auth_state_table_exists(&db).await);
        db.ensure_schema_ready_for_hooks()
            .await
            .expect("upgraded schema passes strict readiness");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_session_auth_schema_compat_doctor_rejects_pre_008_database_before_setup() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("agent-trace.db");
        create_pre_claude_session_auth_db(&path).await;

        let Err(doctor_error) =
            RepositoryAgentTraceDb::open_existing_schema_ready_at(&path, || {}).await
        else {
            panic!("doctor readiness must reject a pre-008 database");
        };
        assert!(matches!(
            doctor_error,
            ExistingRepositoryDbError::IncompatibleSchema(_)
        ));
        assert!(doctor_error
            .to_string()
            .contains(CLAUDE_SESSION_AUTH_STATE_MIGRATION));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_session_auth_schema_compat_hook_runtime_rejects_other_missing_migration() {
        for removed in [
            vec![COMMIT_PATCH_ID_MIGRATION],
            vec![
                COMMIT_PATCH_ID_MIGRATION,
                CLAUDE_SESSION_AUTH_STATE_MIGRATION,
            ],
        ] {
            let dir = tempfile::tempdir().expect("temp dir");
            let path = dir.path().join("agent-trace.db");
            let db = create_current_db(&path).await;
            for id in &removed {
                db.execute("DELETE FROM __sce_migrations WHERE id = ?1", (*id,))
                    .await
                    .expect("remove ledger row");
            }
            drop(db);

            assert_hook_runtime_rejects(&path, COMMIT_PATCH_ID_MIGRATION).await;
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_session_auth_schema_compat_hook_runtime_rejects_unexpected_migration() {
        for remove_optional in [false, true] {
            let dir = tempfile::tempdir().expect("temp dir");
            let path = dir.path().join("agent-trace.db");
            let db = create_current_db(&path).await;
            if remove_optional {
                db.execute(
                    "DELETE FROM __sce_migrations WHERE id = ?1",
                    (CLAUDE_SESSION_AUTH_STATE_MIGRATION,),
                )
                .await
                .expect("remove optional ledger row");
            }
            db.execute(
                "INSERT INTO __sce_migrations (id) VALUES ('999_unexpected_future')",
                (),
            )
            .await
            .expect("insert unexpected ledger row");
            drop(db);

            assert_hook_runtime_rejects(&path, "999_unexpected_future").await;
        }
    }

    fn bedrock_auth_state(session_id: &str, observed_at_ms: i64) -> ClaudeSessionAuthState {
        ClaudeSessionAuthState {
            session_id: String::from(session_id),
            provider: AuthProvider::AwsBedrock,
            billing_mode: BillingMode::CloudProvider,
            auth_env_key: Some(AuthEnvKey::ClaudeCodeUseBedrock),
            confidence: DetectionConfidence::Inferred,
            observed_at_ms,
        }
    }

    fn unknown_auth_state(session_id: &str, observed_at_ms: i64) -> ClaudeSessionAuthState {
        ClaudeSessionAuthState {
            session_id: String::from(session_id),
            provider: AuthProvider::Unknown,
            billing_mode: BillingMode::Unknown,
            auth_env_key: None,
            confidence: DetectionConfidence::Unknown,
            observed_at_ms,
        }
    }

    async fn claude_session_auth_state_row_count(db: &RepositoryAgentTraceDb) -> i64 {
        db.query_map(
            "SELECT COUNT(*) FROM claude_session_auth_state",
            (),
            |row| row.get::<i64>(0).map_err(Into::into),
        )
        .await
        .expect("row count")[0]
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_session_auth_state_identical_replay_leaves_row_unchanged() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = create_current_db(&dir.path().join("agent-trace.db")).await;

        for state in [
            bedrock_auth_state("claude:replay", 10),
            unknown_auth_state("claude:replay-unknown", 10),
        ] {
            let session_id = state.session_id.clone();
            assert_eq!(
                db.upsert_claude_session_auth_state(state.clone())
                    .await
                    .expect("first upsert"),
                ClaudeSessionAuthStateUpsertOutcome::Upserted { rows_changed: 1 }
            );

            let mut replay = state.clone();
            replay.observed_at_ms = 20;
            assert_eq!(
                db.upsert_claude_session_auth_state(replay)
                    .await
                    .expect("replay upsert"),
                ClaudeSessionAuthStateUpsertOutcome::Upserted { rows_changed: 0 }
            );

            assert_eq!(
                db.claude_session_auth_state_by_session(&session_id)
                    .await
                    .expect("read"),
                Some(state)
            );
        }
        assert_eq!(claude_session_auth_state_row_count(&db).await, 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_session_auth_state_changed_classification_updates_row_in_place() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = create_current_db(&dir.path().join("agent-trace.db")).await;

        db.upsert_claude_session_auth_state(bedrock_auth_state("claude:change", 10))
            .await
            .expect("initial upsert");

        let api_key = ClaudeSessionAuthState {
            session_id: String::from("claude:change"),
            provider: AuthProvider::Anthropic,
            billing_mode: BillingMode::ApiKey,
            auth_env_key: Some(AuthEnvKey::AnthropicApiKey),
            confidence: DetectionConfidence::Inferred,
            observed_at_ms: 20,
        };
        assert_eq!(
            db.upsert_claude_session_auth_state(api_key.clone())
                .await
                .expect("changed upsert"),
            ClaudeSessionAuthStateUpsertOutcome::Upserted { rows_changed: 1 }
        );
        assert_eq!(
            db.claude_session_auth_state_by_session("claude:change")
                .await
                .expect("read after change"),
            Some(api_key)
        );

        let unknown = unknown_auth_state("claude:change", 30);
        assert_eq!(
            db.upsert_claude_session_auth_state(unknown.clone())
                .await
                .expect("env key cleared upsert"),
            ClaudeSessionAuthStateUpsertOutcome::Upserted { rows_changed: 1 }
        );
        assert_eq!(
            db.claude_session_auth_state_by_session("claude:change")
                .await
                .expect("read after clearing env key"),
            Some(unknown)
        );
        assert_eq!(claude_session_auth_state_row_count(&db).await, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_session_auth_state_read_returns_only_exact_session() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = create_current_db(&dir.path().join("agent-trace.db")).await;

        let first = bedrock_auth_state("claude:session-a", 10);
        let second = unknown_auth_state("claude:session-b", 11);
        db.upsert_claude_session_auth_state(first.clone())
            .await
            .expect("first session upsert");
        db.upsert_claude_session_auth_state(second.clone())
            .await
            .expect("second session upsert");

        assert_eq!(
            db.claude_session_auth_state_by_session("claude:session-a")
                .await
                .expect("read a"),
            Some(first)
        );
        assert_eq!(
            db.claude_session_auth_state_by_session("claude:session-b")
                .await
                .expect("read b"),
            Some(second)
        );
        for absent in ["claude:session", "session-a", "claude:session-c"] {
            assert_eq!(
                db.claude_session_auth_state_by_session(absent)
                    .await
                    .expect("read absent"),
                None
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_session_auth_state_read_rejects_unknown_stored_value() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = create_current_db(&dir.path().join("agent-trace.db")).await;
        db.execute("DROP TABLE claude_session_auth_state", ())
            .await
            .expect("drop constrained table");
        db.execute(
            "CREATE TABLE claude_session_auth_state (session_id TEXT PRIMARY KEY, provider TEXT NOT NULL, billing_mode TEXT NOT NULL, auth_env_key TEXT, confidence TEXT NOT NULL, observed_at_ms INTEGER NOT NULL)",
            (),
        )
        .await
        .expect("unconstrained table");
        db.execute(
            "INSERT INTO claude_session_auth_state VALUES ('claude:bad', 'openai', 'unknown', NULL, 'unknown', 1)",
            (),
        )
        .await
        .expect("insert unknown provider");

        assert!(db
            .claude_session_auth_state_by_session("claude:bad")
            .await
            .is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_session_auth_state_upsert_skips_pre_008_database() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("agent-trace.db");
        create_pre_claude_session_auth_db(&path).await;
        let (db, _metadata) =
            open_repository_db_for_hook_runtime(&path, PRE_CLAUDE_SESSION_AUTH_REPOSITORY_ID)
                .await
                .expect("hook runtime opens pre-008 database");

        assert_eq!(
            db.upsert_claude_session_auth_state(bedrock_auth_state("claude:pre-008", 10))
                .await
                .expect("skipped upsert is not an error"),
            ClaudeSessionAuthStateUpsertOutcome::SkippedMissingMigration
        );
        assert!(!claude_session_auth_state_table_exists(&db).await);
    }
}
