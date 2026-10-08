use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::app::HasRepoRoot;
use crate::services::agent_trace_storage::{resolve_agent_trace_storage, AgentTraceStorageContext};
use crate::services::config;
use crate::services::db::{bootstrap_db_parent, collect_db_path_health, DbSpec};
use crate::services::default_paths::agent_trace_db_path_for_repository;
use crate::services::lifecycle::{
    FixOutcome, FixResultRecord, HealthCategory, HealthFixability, HealthProblem,
    HealthProblemKind, HealthSeverity, ServiceLifecycle, SetupOutcome,
};
use crate::services::repository_identity::resolve::{
    resolve_repository_identity, RepositoryIdentitySource,
};

use super::repository::{
    ExistingRepositoryDbError, RepositoryAgentTraceDb, RepositoryAgentTraceDbSpec,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AgentTraceDbLifecycle;

impl ServiceLifecycle for AgentTraceDbLifecycle {
    async fn diagnose<C: HasRepoRoot>(&self, ctx: &C) -> Vec<HealthProblem> {
        diagnose_agent_trace_db_health(ctx.repo_root()).await
    }

    async fn fix<C: HasRepoRoot>(
        &self,
        ctx: &C,
        problems: &[HealthProblem],
    ) -> Vec<FixResultRecord> {
        let should_bootstrap_parent = problems.iter().any(|problem| {
            problem.category == HealthCategory::GlobalState
                && problem.fixability == HealthFixability::AutoFixable
        });
        if !should_bootstrap_parent {
            return Vec::new();
        }

        match bootstrap_agent_trace_db_parent(ctx.repo_root()) {
            Ok(parent) => vec![FixResultRecord {
                category: HealthCategory::GlobalState,
                outcome: FixOutcome::Fixed,
                detail: format!(
                    "Agent trace DB parent directory bootstrapped at '{}'.",
                    parent.display()
                ),
            }],
            Err(error) => vec![FixResultRecord {
                category: HealthCategory::GlobalState,
                outcome: FixOutcome::Failed,
                detail: format!(
                    "Automatic agent trace DB parent directory bootstrap failed: {error}"
                ),
            }],
        }
    }

    async fn setup<C: HasRepoRoot>(&self, ctx: &C) -> Result<SetupOutcome> {
        let repository_setup = match ctx.repo_root() {
            Some(repo_root) => Some(
                initialize_repository_agent_trace_db(repo_root)
                    .await
                    .context(
                    "Agent trace DB lifecycle setup failed while initializing repository database",
                )?,
            ),
            None => None,
        };

        Ok(SetupOutcome {
            messages: repository_setup
                .iter()
                .map(format_repository_storage_setup_message)
                .collect(),
            ..SetupOutcome::default()
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RepositoryDatabaseSetup {
    repository_id: String,
    canonical_identity: String,
    identity_source: String,
    configured_remote: Option<String>,
    source_instance_id: String,
    database_path: PathBuf,
}

async fn initialize_repository_agent_trace_db(repo_root: &Path) -> Result<RepositoryDatabaseSetup> {
    let storage_config = config::resolve_agent_trace_storage_runtime_config(repo_root)
        .context("failed to resolve Agent Trace repository storage config")?;
    let storage_context = AgentTraceStorageContext {
        repository_root: repo_root,
        explicit_repository_id: storage_config.repository_id.as_deref(),
        repository_remote: &storage_config.repository_remote,
    };
    let storage = resolve_agent_trace_storage(&storage_context).await?;

    let (identity_source, configured_remote) = match storage.repository_identity.source {
        RepositoryIdentitySource::ExplicitConfig => (String::from("explicit_config"), None),
        RepositoryIdentitySource::RemoteUrl { remote_name } => {
            (String::from("remote_url"), Some(remote_name))
        }
    };

    Ok(RepositoryDatabaseSetup {
        repository_id: storage.repository_identity.identity.repository_id,
        canonical_identity: storage.repository_identity.identity.canonical_identity,
        identity_source,
        configured_remote,
        source_instance_id: storage.metadata.source_instance_id,
        database_path: storage.db_path,
    })
}

fn format_repository_storage_setup_message(setup: &RepositoryDatabaseSetup) -> String {
    let remote_line = setup
        .configured_remote
        .as_ref()
        .map(|remote| format!("\nAgent Trace configured remote: {remote}"))
        .unwrap_or_default();
    format!(
        "Agent Trace repository ID: {}\nAgent Trace identity source: {}\nAgent Trace canonical identity: {}{}\nAgent Trace source-instance ID: {}\nAgent Trace repository-scoped database initialized at '{}'.",
        setup.repository_id,
        setup.identity_source,
        setup.canonical_identity,
        remote_line,
        setup.source_instance_id,
        setup.database_path.display()
    )
}

pub async fn diagnose_agent_trace_db_health(repo_root: Option<&Path>) -> Vec<HealthProblem> {
    let mut problems = Vec::new();

    let db_path = match resolve_lifecycle_agent_trace_db_path(repo_root) {
        Ok(path) => path,
        Err(error) => {
            problems.push(HealthProblem {
                kind: HealthProblemKind::UnableToResolveStateRoot,
                category: HealthCategory::GlobalState,
                severity: HealthSeverity::Error,
                fixability: HealthFixability::ManualOnly,
                summary: format!("Unable to resolve expected agent trace DB path: {error}"),
                remediation: String::from("Configure agent_trace.repository_id in .sce/config.json or ensure the configured Git remote exists, then rerun 'sce doctor'."),
                next_action: "manual_steps",
            });
            return problems;
        }
    };

    collect_db_path_health(
        <RepositoryAgentTraceDbSpec as DbSpec>::db_name(),
        &db_path,
        &mut problems,
    );

    problems.extend(inspect_existing_db_schema(&db_path, || {}).await);
    problems
}

async fn inspect_existing_db_schema(
    db_path: &Path,
    before_open: impl FnOnce(),
) -> Vec<HealthProblem> {
    let mut problems = Vec::new();
    if db_path.exists() && !db_path.is_file() {
        return problems;
    }

    match RepositoryAgentTraceDb::open_existing_schema_ready_at(db_path, before_open).await {
        Ok(_db) => {}
        Err(ExistingRepositoryDbError::Missing { .. }) => {}
        Err(ExistingRepositoryDbError::IncompatibleSchema(error)) => {
            problems.push(HealthProblem {
                        kind: HealthProblemKind::AgentTraceDbSchemaNotReady,
                        category: HealthCategory::GlobalState,
                        severity: HealthSeverity::Error,
                        fixability: HealthFixability::ManualOnly,
                        summary: format!(
                            "Repository Agent Trace database schema at '{}' is not ready: {error}",
                            db_path.display()
                        ),
                        remediation: String::from(
                            "Re-run 'sce setup' to initialize the repository-scoped database, or inspect the database file for corruption.",
                        ),
                        next_action: "manual_steps",
                    });
        }
        Err(error) => {
            let error = match error {
                ExistingRepositoryDbError::Unreadable(source) => source.to_string(),
                other => other.to_string(),
            };
            problems.push(HealthProblem {
                    kind: HealthProblemKind::AgentTraceDbConnectionFailed,
                    category: HealthCategory::GlobalState,
                    severity: HealthSeverity::Error,
                    fixability: HealthFixability::ManualOnly,
                    summary: format!(
                        "Unable to open repository Agent Trace database at '{}': {error}",
                        db_path.display()
                    ),
                    remediation: String::from(
                        "Verify file permissions and ensure the file is a valid SQLite database. Re-run 'sce setup' to recreate it if needed.",
                    ),
                    next_action: "manual_steps",
                });
        }
    }

    problems
}

fn bootstrap_agent_trace_db_parent(repo_root: Option<&Path>) -> Result<PathBuf> {
    let db_path = resolve_lifecycle_agent_trace_db_path(repo_root)
        .context("failed to resolve agent trace DB path")?;
    bootstrap_db_parent(<RepositoryAgentTraceDbSpec as DbSpec>::db_name(), &db_path)
}

fn resolve_lifecycle_agent_trace_db_path(repo_root: Option<&Path>) -> Result<PathBuf> {
    if let Some(repo_root) = repo_root {
        let storage_config = config::resolve_agent_trace_storage_runtime_config(repo_root)
            .context("failed to resolve Agent Trace repository storage config")?;
        let identity = resolve_repository_identity(
            repo_root,
            storage_config.repository_id.as_deref(),
            &storage_config.repository_remote,
        )
        .map_err(|error| anyhow::anyhow!("{error}"))?;

        return agent_trace_db_path_for_repository(&identity.identity.repository_id);
    }

    // Outside a Git repository there is no repository identity to select an
    // active Agent Trace DB, and there is no global/checkout fallback path.
    // Report an actionable diagnostic instead of probing a sentinel path.
    anyhow::bail!(
        "Agent Trace diagnostics require a Git repository; run 'sce doctor' inside a repository \
         or configure agent_trace.repository_id in .sce/config.json"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_entries(dir: &Path) -> Vec<String> {
        let mut names = std::fs::read_dir(dir)
            .expect("read dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    async fn schema_objects(path: &Path) -> Vec<String> {
        let db = RepositoryAgentTraceDb::open_existing_without_migrations_at(path)
            .await
            .expect("read-only open");
        let mut names = db
            .query_map("SELECT name FROM sqlite_master ORDER BY name", (), |row| {
                row.get::<String>(0).map_err(Into::into)
            })
            .await
            .expect("schema query");
        names.sort();
        names
    }

    async fn count_rows(path: &Path, table: &str) -> i64 {
        let db = RepositoryAgentTraceDb::open_existing_without_migrations_at(path)
            .await
            .expect("read-only open");
        db.query_map(&format!("SELECT COUNT(*) FROM {table}"), (), |row| {
            row.get::<i64>(0).map_err(Into::into)
        })
        .await
        .expect("count query")[0]
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn missing_database_is_not_created_and_leaves_no_parent_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir
            .path()
            .join("state")
            .join("repos")
            .join("id")
            .join("agent-trace.db");

        let problems = inspect_existing_db_schema(&path, || {}).await;
        let again = inspect_existing_db_schema(&path, || {}).await;

        assert!(problems.is_empty());
        assert!(again.is_empty());
        assert!(!path.exists());
        assert!(dir_entries(dir.path()).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn database_removed_immediately_before_open_is_not_recreated() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("agent-trace.db");
        RepositoryAgentTraceDb::new_at(&path).await.expect("create");
        assert!(path.is_file());

        let problems = inspect_existing_db_schema(&path, || {
            for entry in std::fs::read_dir(dir.path()).expect("read dir") {
                std::fs::remove_file(entry.expect("entry").path()).expect("remove");
            }
        })
        .await;

        assert!(problems.is_empty());
        assert!(dir_entries(dir.path()).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn valid_database_is_inspected_without_problems_or_logical_change() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("agent-trace.db");
        let db = RepositoryAgentTraceDb::new_at(&path).await.expect("create");
        db.verify_or_initialize_repository_metadata("repo")
            .await
            .expect("metadata");
        drop(db);
        let objects_before = schema_objects(&path).await;
        let migrations_before = count_rows(&path, "__sce_migrations").await;
        let metadata_before = count_rows(&path, "repository_metadata").await;

        let problems = inspect_existing_db_schema(&path, || {}).await;
        let first_files = dir_entries(dir.path());
        let repeat = inspect_existing_db_schema(&path, || {}).await;

        assert!(problems.is_empty());
        assert!(repeat.is_empty());
        assert_eq!(dir_entries(dir.path()), first_files);
        assert_eq!(schema_objects(&path).await, objects_before);
        assert_eq!(
            count_rows(&path, "__sce_migrations").await,
            migrations_before
        );
        assert_eq!(
            count_rows(&path, "repository_metadata").await,
            metadata_before
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn incompatible_schema_is_diagnosed_without_initializing_or_migrating() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("agent-trace.db");
        let raw = RepositoryAgentTraceDb::open_without_migrations_at(&path)
            .await
            .expect("create empty");
        raw.execute("CREATE TABLE unrelated (id INTEGER)", ())
            .await
            .expect("unrelated table");
        drop(raw);
        let objects_before = schema_objects(&path).await;

        let problems = inspect_existing_db_schema(&path, || {}).await;

        assert_eq!(problems.len(), 1);
        assert_eq!(
            problems[0].kind,
            HealthProblemKind::AgentTraceDbSchemaNotReady
        );
        assert!(problems[0].summary.contains("is not ready"));
        assert_eq!(schema_objects(&path).await, objects_before);
        assert!(!objects_before.iter().any(|name| name == "__sce_migrations"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn non_database_file_is_unreadable_and_left_byte_identical() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("agent-trace.db");
        std::fs::write(
            &path,
            b"this is not a sqlite database at all, just some text bytes....",
        )
        .expect("write");
        let before = std::fs::read(&path).expect("read");

        let problems = inspect_existing_db_schema(&path, || {}).await;

        assert_eq!(problems.len(), 1);
        assert_ne!(
            problems[0].kind,
            HealthProblemKind::AgentTraceDbSchemaNotReady
        );
        assert_eq!(std::fs::read(&path).expect("read"), before);
        assert_eq!(dir_entries(dir.path()), vec!["agent-trace.db".to_string()]);
    }
}
