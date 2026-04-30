#![allow(dead_code)]

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::app::AppContext;
use crate::services::default_paths::local_db_path;
use crate::services::lifecycle::{
    FixOutcome, FixResultRecord, HealthCategory, HealthFixability, HealthProblem,
    HealthProblemKind, HealthSeverity, ServiceLifecycle, SetupOutcome,
};

use super::LocalDb;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LocalDbLifecycle;

impl ServiceLifecycle for LocalDbLifecycle {
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

    collect_local_db_path_health(&db_path, &mut problems);
    problems
}

fn collect_local_db_path_health(db_path: &Path, problems: &mut Vec<HealthProblem>) {
    let Some(parent) = db_path.parent() else {
        problems.push(HealthProblem {
            kind: HealthProblemKind::UnableToResolveStateRoot,
            category: HealthCategory::GlobalState,
            severity: HealthSeverity::Error,
            fixability: HealthFixability::ManualOnly,
            summary: format!(
                "Unable to resolve parent directory for local DB path '{}'.",
                db_path.display()
            ),
            remediation: String::from("Verify that the current platform exposes a writable SCE state directory before rerunning 'sce doctor'."),
            next_action: "manual_steps",
        });
        return;
    };

    if !parent.exists() {
        problems.push(HealthProblem {
            kind: HealthProblemKind::UnableToResolveStateRoot,
            category: HealthCategory::GlobalState,
            severity: HealthSeverity::Error,
            fixability: HealthFixability::AutoFixable,
            summary: format!(
                "Local DB parent directory '{}' does not exist.",
                parent.display()
            ),
            remediation: format!(
                "Run 'sce doctor --fix' to create the canonical local DB parent directory at '{}'.",
                parent.display()
            ),
            next_action: "doctor_fix",
        });
    } else if !parent.is_dir() {
        problems.push(HealthProblem {
            kind: HealthProblemKind::UnableToResolveStateRoot,
            category: HealthCategory::GlobalState,
            severity: HealthSeverity::Error,
            fixability: HealthFixability::ManualOnly,
            summary: format!(
                "Local DB parent path '{}' is not a directory.",
                parent.display()
            ),
            remediation: format!(
                "Replace '{}' with a writable directory before rerunning 'sce doctor'.",
                parent.display()
            ),
            next_action: "manual_steps",
        });
    }

    if db_path.exists() && !db_path.is_file() {
        problems.push(HealthProblem {
            kind: HealthProblemKind::UnableToResolveStateRoot,
            category: HealthCategory::GlobalState,
            severity: HealthSeverity::Error,
            fixability: HealthFixability::ManualOnly,
            summary: format!("Local DB path '{}' is not a file.", db_path.display()),
            remediation: format!(
                "Replace '{}' with a writable local DB file path before rerunning 'sce doctor'.",
                db_path.display()
            ),
            next_action: "manual_steps",
        });
    }
}

fn bootstrap_local_db_parent() -> Result<std::path::PathBuf> {
    let db_path = local_db_path().context("failed to resolve local DB path")?;
    let parent = db_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("local DB path has no parent: {}", db_path.display()))?;

    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create local DB parent directory: {}",
            parent.display()
        )
    })?;

    Ok(parent.to_path_buf())
}
