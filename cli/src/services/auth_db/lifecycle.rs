use anyhow::{Context, Result};

use crate::app::HasRepoRoot;
use crate::services::db::{bootstrap_db_parent, collect_db_path_health, DbSpec};
use crate::services::default_paths::auth_db_path;
use crate::services::lifecycle::{
    FixOutcome, FixResultRecord, HealthCategory, HealthFixability, HealthProblem,
    HealthProblemKind, HealthSeverity, LifecycleProviderId, ServiceLifecycle, SetupOutcome,
};

use super::{AuthDb, AuthDbSpec};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AuthDbLifecycle;

impl ServiceLifecycle for AuthDbLifecycle {
    fn id(&self) -> LifecycleProviderId {
        LifecycleProviderId::AuthDb
    }

    fn diagnose<C: HasRepoRoot>(&self, _ctx: &C) -> Vec<HealthProblem> {
        diagnose_auth_db_health()
    }

    fn fix<C: HasRepoRoot>(&self, _ctx: &C, problems: &[HealthProblem]) -> Vec<FixResultRecord> {
        let should_bootstrap_parent = problems.iter().any(|problem| {
            problem.category == HealthCategory::GlobalState
                && problem.fixability == HealthFixability::AutoFixable
        });
        if !should_bootstrap_parent {
            return Vec::new();
        }

        match bootstrap_auth_db_parent() {
            Ok(parent) => vec![FixResultRecord {
                category: HealthCategory::GlobalState,
                outcome: FixOutcome::Fixed,
                detail: format!(
                    "Auth DB parent directory bootstrapped at '{}'.",
                    parent.display()
                ),
            }],
            Err(error) => vec![FixResultRecord {
                category: HealthCategory::GlobalState,
                outcome: FixOutcome::Failed,
                detail: format!("Automatic auth DB parent directory bootstrap failed: {error}"),
            }],
        }
    }

    fn setup<C: HasRepoRoot>(&self, _ctx: &C) -> Result<SetupOutcome> {
        AuthDb::new().context("Auth DB lifecycle setup failed while initializing auth DB")?;
        Ok(SetupOutcome::default())
    }
}

fn diagnose_auth_db_health() -> Vec<HealthProblem> {
    let mut problems = Vec::new();

    let db_path = match auth_db_path() {
        Ok(path) => path,
        Err(error) => {
            problems.push(HealthProblem {
                kind: HealthProblemKind::UnableToResolveStateRoot,
                category: HealthCategory::GlobalState,
                severity: HealthSeverity::Error,
                fixability: HealthFixability::ManualOnly,
                summary: format!("Unable to resolve expected auth DB path: {error}"),
                remediation: String::from("Verify that the current platform exposes a writable SCE state directory before rerunning 'sce doctor'."),
                next_action: "manual_steps",
            });
            return problems;
        }
    };

    collect_db_path_health(<AuthDbSpec as DbSpec>::db_name(), &db_path, &mut problems);
    problems
}

fn bootstrap_auth_db_parent() -> Result<std::path::PathBuf> {
    let db_path = auth_db_path().context("failed to resolve auth DB path")?;
    bootstrap_db_parent(<AuthDbSpec as DbSpec>::db_name(), &db_path)
}
