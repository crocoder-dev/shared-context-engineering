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

use super::AgentTraceDbSpec;

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
        let checkout_setup = ctx
            .repo_root()
            .map(setup_checkout_identity)
            .transpose()
            .context("Agent trace DB lifecycle setup failed while resolving checkout identity")?;

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

fn format_checkout_identity_setup_message(setup: &CheckoutIdentitySetup) -> String {
    format!(
        "Agent Trace checkout identity: {}\nAgent Trace database will be created on first write.",
        setup.checkout_id
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
