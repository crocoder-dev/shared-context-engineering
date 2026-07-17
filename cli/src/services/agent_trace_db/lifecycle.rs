use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::app::HasRepoRoot;
use crate::services::agent_trace_storage::{resolve_agent_trace_storage, AgentTraceStorageContext};
use crate::services::config;
use crate::services::db::{bootstrap_db_parent, collect_db_path_health, DbSpec};
use crate::services::default_paths::{agent_trace_db_path, agent_trace_db_path_for_repository};
use crate::services::lifecycle::{
    FixOutcome, FixResultRecord, HealthCategory, HealthFixability, HealthProblem,
    HealthProblemKind, HealthSeverity, LifecycleProviderId, ServiceLifecycle, SetupOutcome,
};
use crate::services::repository_identity::resolve::{
    resolve_repository_identity, RepositoryIdentitySource,
};

use super::repository::{RepositoryAgentTraceDb, RepositoryAgentTraceDbSpec};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AgentTraceDbLifecycle;

impl ServiceLifecycle for AgentTraceDbLifecycle {
    fn id(&self) -> LifecycleProviderId {
        LifecycleProviderId::AgentTraceDb
    }

    fn diagnose<C: HasRepoRoot>(&self, ctx: &C) -> Vec<HealthProblem> {
        diagnose_agent_trace_db_health(ctx.repo_root())
    }

    fn fix<C: HasRepoRoot>(&self, ctx: &C, problems: &[HealthProblem]) -> Vec<FixResultRecord> {
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

    fn setup<C: HasRepoRoot>(&self, ctx: &C) -> Result<SetupOutcome> {
        let repository_setup = match ctx.repo_root() {
            Some(repo_root) => Some(initialize_repository_agent_trace_db(repo_root).context(
                "Agent trace DB lifecycle setup failed while initializing repository database",
            )?),
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
    checkout_id: String,
    database_path: PathBuf,
}

fn initialize_repository_agent_trace_db(repo_root: &Path) -> Result<RepositoryDatabaseSetup> {
    let storage_config = config::resolve_agent_trace_storage_runtime_config(repo_root)
        .context("failed to resolve Agent Trace repository storage config")?;
    let storage_context = AgentTraceStorageContext {
        repository_root: repo_root,
        explicit_repository_id: storage_config.repository_id.as_deref(),
        repository_remote: &storage_config.repository_remote,
    };
    let storage = resolve_agent_trace_storage(&storage_context)?;

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
        checkout_id: storage.checkout_id,
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
        "Agent Trace repository ID: {}\nAgent Trace identity source: {}\nAgent Trace canonical identity: {}{}\nAgent Trace checkout identity: {}\nAgent Trace repository-scoped database initialized at '{}'.",
        setup.repository_id,
        setup.identity_source,
        setup.canonical_identity,
        remote_line,
        setup.checkout_id,
        setup.database_path.display()
    )
}

pub fn diagnose_agent_trace_db_health(repo_root: Option<&Path>) -> Vec<HealthProblem> {
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

    if db_path.exists() && db_path.is_file() {
        match RepositoryAgentTraceDb::open_without_migrations_at(&db_path) {
            Ok(db) => {
                if let Err(error) = db.ensure_schema_ready_for_hooks() {
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
            }
            Err(error) => {
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

    // Outside repository-targeted lifecycle contexts there is no repository
    // identity to select an active DB. Keep the historical global path as the
    // operator-state parent sentinel only; repository setup/hooks never use it
    // as an active write target.
    agent_trace_db_path()
}
