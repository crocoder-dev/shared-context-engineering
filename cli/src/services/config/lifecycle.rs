use std::path::Path;

use anyhow::{Context, Result};

use crate::app::AppContext;
use crate::services::default_paths::{resolve_sce_default_locations, RepoPaths};
use crate::services::lifecycle::{
    HealthCategory, HealthFixability, HealthProblem, HealthProblemKind, HealthSeverity,
    LifecycleProviderId, ServiceLifecycle, SetupOutcome,
};
use crate::services::setup::bootstrap_repo_local_config;

use super::validate_config_file;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConfigLifecycle;

impl ServiceLifecycle for ConfigLifecycle {
    fn id(&self) -> LifecycleProviderId {
        LifecycleProviderId::Config
    }

    fn diagnose(&self, ctx: &AppContext) -> Vec<HealthProblem> {
        let repository_root = match ctx.repo_root() {
            Some(path) => path.to_path_buf(),
            None => {
                return vec![HealthProblem {
                    kind: HealthProblemKind::NotInsideGitRepository,
                    category: HealthCategory::RepositoryTargeting,
                    severity: HealthSeverity::Error,
                    fixability: HealthFixability::ManualOnly,
                    summary: String::from("The current directory is not inside a git repository."),
                    remediation: String::from(
                        "Run 'sce doctor' from inside the target repository working tree to inspect repo-scoped SCE config health.",
                    ),
                    next_action: "manual_steps",
                }];
            }
        };

        diagnose_config_health(&repository_root)
    }

    fn setup(&self, ctx: &AppContext) -> Result<SetupOutcome> {
        let repository_root = ctx
            .repo_root()
            .context("Config lifecycle setup requires a resolved repository root")?;

        bootstrap_repo_local_config(repository_root)
            .context("Config lifecycle setup failed while bootstrapping repo-local config")?;

        Ok(SetupOutcome::default())
    }
}

pub fn diagnose_config_health(repository_root: &Path) -> Vec<HealthProblem> {
    let mut problems = Vec::new();
    collect_global_config_health(&mut problems);
    collect_local_config_health(repository_root, &mut problems);
    problems
}

fn collect_global_config_health(problems: &mut Vec<HealthProblem>) {
    let global_path = match resolve_sce_default_locations()
        .map(|locations| locations.global_config_file())
    {
        Ok(path) => path,
        Err(error) => {
            problems.push(HealthProblem {
                kind: HealthProblemKind::UnableToResolveGlobalConfigPath,
                category: HealthCategory::GlobalState,
                severity: HealthSeverity::Error,
                fixability: HealthFixability::ManualOnly,
                summary: format!("Unable to resolve expected global config path: {error}"),
                remediation: String::from("Verify that the current platform exposes a writable SCE config directory before rerunning 'sce doctor'."),
                next_action: "manual_steps",
            });
            return;
        }
    };

    if global_path.exists() {
        if let Err(error) = validate_config_file(&global_path) {
            problems.push(HealthProblem {
                kind: HealthProblemKind::GlobalConfigValidationFailed,
                category: HealthCategory::GlobalState,
                severity: HealthSeverity::Error,
                fixability: HealthFixability::ManualOnly,
                summary: format!(
                    "Global config file '{}' failed validation: {error}",
                    global_path.display()
                ),
                remediation: format!(
                    "Repair or remove the invalid global config file at '{}' and rerun 'sce doctor'.",
                    global_path.display()
                ),
                next_action: "manual_steps",
            });
        }
    }
}

fn collect_local_config_health(repository_root: &Path, problems: &mut Vec<HealthProblem>) {
    let local_path = RepoPaths::new(repository_root).sce_config_file();
    if local_path.exists() {
        if let Err(error) = validate_config_file(&local_path) {
            problems.push(HealthProblem {
                kind: HealthProblemKind::LocalConfigValidationFailed,
                category: HealthCategory::RepositoryTargeting,
                severity: HealthSeverity::Error,
                fixability: HealthFixability::ManualOnly,
                summary: format!(
                    "Local config file '{}' failed validation: {error}",
                    local_path.display()
                ),
                remediation: format!(
                    "Repair or remove the invalid local config file at '{}' and rerun 'sce doctor'.",
                    local_path.display()
                ),
                next_action: "manual_steps",
            });
        }
    }
}
