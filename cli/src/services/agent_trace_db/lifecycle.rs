use anyhow::{Context, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};

use crate::app::HasRepoRoot;
use crate::services::checkout::{self, registry};
use crate::services::db::{bootstrap_db_parent, collect_db_path_health, DbSpec};
use crate::services::default_paths::{agent_trace_db_path, agent_trace_db_path_for_checkout};
use crate::services::lifecycle::{
    FixOutcome, FixResultRecord, HealthCategory, HealthFixability, HealthProblem,
    HealthProblemKind, HealthSeverity, LifecycleProviderId, ServiceLifecycle, SetupOutcome,
};

use super::{AgentTraceDb, AgentTraceDbSpec};

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
        let checkout_setup = match ctx.repo_root() {
            Some(repo_root) => {
                let identity_setup = setup_checkout_identity(repo_root).context(
                    "Agent trace DB lifecycle setup failed while resolving checkout identity",
                )?;
                Some(
                    initialize_checkout_agent_trace_db(repo_root, &identity_setup.checkout_id).context(
                        "Agent trace DB lifecycle setup failed while initializing checkout database",
                    )?,
                )
            }
            None => None,
        };

        Ok(SetupOutcome {
            messages: checkout_setup
                .iter()
                .map(format_checkout_identity_setup_message)
                .collect(),
            ..SetupOutcome::default()
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CheckoutIdentitySetup {
    checkout_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CheckoutDatabaseSetup {
    checkout_id: String,
    database_path: PathBuf,
}

fn setup_checkout_identity(repo_root: &std::path::Path) -> Result<CheckoutIdentitySetup> {
    let git_dir = checkout::resolve_git_dir(repo_root).with_context(|| {
        format!(
            "failed to resolve git directory for checkout identity from '{}'",
            repo_root.display()
        )
    })?;
    let checkout_id = checkout::get_or_create_checkout_id(&git_dir).with_context(|| {
        format!(
            "failed to get or create checkout identity under '{}'",
            git_dir.display()
        )
    })?;
    registry::register_checkout(registry::CheckoutRecord {
        checkout_id: checkout_id.clone(),
        path: repo_root.display().to_string(),
        last_seen: Utc::now().to_rfc3339(),
        remote_url: None,
        database_path: None,
    })
    .context("failed to register checkout identity")?;

    Ok(CheckoutIdentitySetup { checkout_id })
}

fn initialize_checkout_agent_trace_db(
    repo_root: &Path,
    checkout_id: &str,
) -> Result<CheckoutDatabaseSetup> {
    let db_path = agent_trace_db_path_for_checkout(checkout_id).with_context(|| {
        format!("failed to resolve Agent Trace DB path for checkout ID {checkout_id}")
    })?;

    AgentTraceDb::open_at(&db_path).with_context(|| {
        format!(
            "failed to initialize Agent Trace DB for checkout {} at '{}'",
            checkout_id,
            db_path.display()
        )
    })?;

    registry::register_checkout(registry::CheckoutRecord {
        checkout_id: checkout_id.to_string(),
        path: repo_root.display().to_string(),
        last_seen: Utc::now().to_rfc3339(),
        remote_url: None,
        database_path: Some(db_path.display().to_string()),
    })
    .context("failed to register checkout Agent Trace database path")?;

    Ok(CheckoutDatabaseSetup {
        checkout_id: checkout_id.to_string(),
        database_path: db_path,
    })
}

fn format_checkout_identity_setup_message(setup: &CheckoutDatabaseSetup) -> String {
    format!(
        "Agent Trace checkout identity: {}\nAgent Trace database initialized at '{}'.",
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
                remediation: String::from("Verify that the current platform exposes a writable SCE state directory before rerunning 'sce doctor'."),
                next_action: "manual_steps",
            });
            return problems;
        }
    };

    collect_db_path_health(
        <AgentTraceDbSpec as DbSpec>::db_name(),
        &db_path,
        &mut problems,
    );

    if db_path.exists() && db_path.is_file() {
        match AgentTraceDb::open_for_hooks_without_migrations_at(&db_path) {
            Ok(db) => {
                if let Err(error) = db.ensure_schema_ready_for_hooks() {
                    problems.push(HealthProblem {
                        kind: HealthProblemKind::AgentTraceDbSchemaNotReady,
                        category: HealthCategory::GlobalState,
                        severity: HealthSeverity::Error,
                        fixability: HealthFixability::ManualOnly,
                        summary: format!(
                            "Agent Trace database schema at '{}' is not ready: {error}",
                            db_path.display()
                        ),
                        remediation: String::from(
                            "Re-run 'sce setup' to apply missing migrations, or inspect the database file for corruption.",
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
                        "Unable to open checkout Agent Trace database at '{}': {error}",
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
    bootstrap_db_parent(<AgentTraceDbSpec as DbSpec>::db_name(), &db_path)
}

fn resolve_lifecycle_agent_trace_db_path(repo_root: Option<&Path>) -> Result<PathBuf> {
    if let Some(repo_root) = repo_root {
        let git_dir = checkout::resolve_git_dir(repo_root).with_context(|| {
            format!(
                "failed to resolve git directory for agent trace DB health from '{}'",
                repo_root.display()
            )
        })?;
        if let Some(checkout_id) = checkout::read_checkout_id(&git_dir)? {
            return agent_trace_db_path_for_checkout(&checkout_id);
        }
    }

    agent_trace_db_path()
}
