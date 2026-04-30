use anyhow::{Context, Result};

use crate::app::AppContext;
use crate::services::db::{bootstrap_db_parent, collect_db_path_health, DbSpec};
use crate::services::default_paths::local_db_path;
use crate::services::lifecycle::{
    FixOutcome, FixResultRecord, HealthCategory, HealthFixability, HealthProblem,
    HealthProblemKind, HealthSeverity, LifecycleProviderId, ServiceLifecycle, SetupOutcome,
};

use super::{LocalDb, LocalDbSpec};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LocalDbLifecycle;

impl ServiceLifecycle for LocalDbLifecycle {
    fn id(&self) -> LifecycleProviderId {
        LifecycleProviderId::LocalDb
    }

    fn diagnose(&self, _ctx: &AppContext) -> Vec<HealthProblem> {
        diagnose_local_db_health()
    }

    fn fix(&self, _ctx: &AppContext, problems: &[HealthProblem]) -> Vec<FixResultRecord> {
        let should_bootstrap_parent = problems.iter().any(|problem| {
            problem.category == HealthCategory::GlobalState
                && problem.fixability == HealthFixability::AutoFixable
        });
        if !should_bootstrap_parent {
            return Vec::new();
        }

        match bootstrap_local_db_parent() {
            Ok(parent) => vec![FixResultRecord {
                category: HealthCategory::GlobalState,
                outcome: FixOutcome::Fixed,
                detail: format!(
                    "Local DB parent directory bootstrapped at '{}'.",
                    parent.display()
                ),
            }],
            Err(error) => vec![FixResultRecord {
                category: HealthCategory::GlobalState,
                outcome: FixOutcome::Failed,
                detail: format!("Automatic local DB parent directory bootstrap failed: {error}"),
            }],
        }
    }

    fn setup(&self, _ctx: &AppContext) -> Result<SetupOutcome> {
        LocalDb::new().context("Local DB lifecycle setup failed while initializing local DB")?;
        Ok(SetupOutcome::default())
    }
}

pub fn diagnose_local_db_health() -> Vec<HealthProblem> {
    let mut problems = Vec::new();

    let db_path = match local_db_path() {
        Ok(path) => path,
        Err(error) => {
            problems.push(HealthProblem {
                kind: HealthProblemKind::UnableToResolveStateRoot,
                category: HealthCategory::GlobalState,
                severity: HealthSeverity::Error,
                fixability: HealthFixability::ManualOnly,
                summary: format!("Unable to resolve expected local DB path: {error}"),
                remediation: String::from("Verify that the current platform exposes a writable SCE state directory before rerunning 'sce doctor'."),
                next_action: "manual_steps",
            });
            return problems;
        }
    };

    collect_db_path_health(<LocalDbSpec as DbSpec>::db_name(), &db_path, &mut problems);
    problems
}

fn bootstrap_local_db_parent() -> Result<std::path::PathBuf> {
    let db_path = local_db_path().context("failed to resolve local DB path")?;
    bootstrap_db_parent(<LocalDbSpec as DbSpec>::db_name(), &db_path)
}
