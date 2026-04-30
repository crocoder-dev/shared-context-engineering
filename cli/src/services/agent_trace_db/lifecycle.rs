use anyhow::{Context, Result};

use crate::app::AppContext;
use crate::services::db::{bootstrap_db_parent, collect_db_path_health, DbSpec};
use crate::services::default_paths::agent_trace_db_path;
use crate::services::lifecycle::{
    FixOutcome, FixResultRecord, HealthCategory, HealthFixability, HealthProblem,
    HealthProblemKind, HealthSeverity, ServiceLifecycle, SetupOutcome,
};

use super::{AgentTraceDb, AgentTraceDbSpec};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AgentTraceDbLifecycle;

impl ServiceLifecycle for AgentTraceDbLifecycle {
    fn diagnose(&self, _ctx: &AppContext) -> Vec<HealthProblem> {
        diagnose_agent_trace_db_health()
    }

    fn fix(&self, _ctx: &AppContext, problems: &[HealthProblem]) -> Vec<FixResultRecord> {
        let should_bootstrap_parent = problems.iter().any(|problem| {
            problem.category == HealthCategory::GlobalState
                && problem.fixability == HealthFixability::AutoFixable
        });
        if !should_bootstrap_parent {
            return Vec::new();
        }

        match bootstrap_agent_trace_db_parent() {
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

    fn setup(&self, _ctx: &AppContext) -> Result<SetupOutcome> {
        AgentTraceDb::new()
            .context("Agent trace DB lifecycle setup failed while initializing agent trace DB")?;
        Ok(SetupOutcome::default())
    }
}

pub fn diagnose_agent_trace_db_health() -> Vec<HealthProblem> {
    let mut problems = Vec::new();

    let db_path = match agent_trace_db_path() {
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

fn bootstrap_agent_trace_db_parent() -> Result<std::path::PathBuf> {
    let db_path = agent_trace_db_path().context("failed to resolve agent trace DB path")?;
    bootstrap_db_parent(<AgentTraceDbSpec as DbSpec>::db_name(), &db_path)
}
