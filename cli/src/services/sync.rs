use anyhow::{Context, Result};
use std::sync::OnceLock;

use crate::services::local_db::{run_smoke_check, LocalDatabaseTarget};

pub const NAME: &str = "sync";
const SUPPORTED_PHASES: [CloudSyncPhase; 3] = [
    CloudSyncPhase::PlanOnly,
    CloudSyncPhase::DryRun,
    CloudSyncPhase::Apply,
];

static SYNC_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloudSyncPhase {
    PlanOnly,
    DryRun,
    Apply,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloudSyncRequest {
    pub workspace: &'static str,
    pub phase: CloudSyncPhase,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloudSyncPlan {
    pub checkpoints: Vec<&'static str>,
    pub can_execute: bool,
}

pub trait CloudSyncGateway {
    fn plan(&self, request: &CloudSyncRequest) -> CloudSyncPlan;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PlaceholderCloudSyncGateway;

impl CloudSyncGateway for PlaceholderCloudSyncGateway {
    fn plan(&self, request: &CloudSyncRequest) -> CloudSyncPlan {
        let mut checkpoints = vec![
            "Resolve local context delta",
            "Build upload manifest",
            "Persist remote reconciliation state",
        ];

        if request.phase != CloudSyncPhase::PlanOnly {
            checkpoints.push("Phase-specific execution remains intentionally disabled");
        }

        if request.phase == CloudSyncPhase::Apply {
            checkpoints
                .push("Apply execution is intentionally blocked by placeholder safety checks");
        }

        CloudSyncPlan {
            checkpoints,
            can_execute: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PlaceholderSyncService<G>
where
    G: CloudSyncGateway,
{
    gateway: G,
}

impl<G> PlaceholderSyncService<G>
where
    G: CloudSyncGateway,
{
    pub fn new(gateway: G) -> Self {
        Self { gateway }
    }

    pub fn run(&self, request: &CloudSyncRequest) -> Result<String> {
        let runtime = shared_runtime()?;

        let outcome = runtime
            .block_on(run_smoke_check(LocalDatabaseTarget::InMemory))
            .context("local Turso smoke check failed")?;

        let plan = self.gateway.plan(request);

        Ok(format!(
            "TODO: '{NAME}' cloud workflows are planned and not implemented yet. Local Turso smoke check succeeded ({}) row inserted; cloud sync placeholder enumerates {} phase(s) and plan holds {} checkpoint(s).",
            outcome.inserted_rows,
            SUPPORTED_PHASES.len(),
            plan.checkpoints.len()
        ))
    }
}

fn shared_runtime() -> Result<&'static tokio::runtime::Runtime> {
    if let Some(runtime) = SYNC_RUNTIME.get() {
        return Ok(runtime);
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .context("failed to create shared tokio runtime for sync placeholder")?;

    let _ = SYNC_RUNTIME.set(runtime);

    SYNC_RUNTIME
        .get()
        .context("shared tokio runtime for sync placeholder is unavailable")
}

pub fn run_placeholder_sync() -> Result<String> {
    let service = PlaceholderSyncService::new(PlaceholderCloudSyncGateway);
    let request = CloudSyncRequest {
        workspace: "local",
        phase: CloudSyncPhase::PlanOnly,
    };
    service.run(&request)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::{
        run_placeholder_sync, CloudSyncGateway, CloudSyncPhase, CloudSyncRequest,
        PlaceholderCloudSyncGateway,
    };

    use super::shared_runtime;

    #[test]
    fn sync_placeholder_runs_local_smoke_check() -> Result<()> {
        let message = run_placeholder_sync()?;
        assert!(message.contains("Local Turso smoke check succeeded"));
        assert!(message.contains("cloud sync placeholder enumerates"));
        Ok(())
    }

    #[test]
    fn cloud_sync_gateway_stays_non_executable() {
        let gateway = PlaceholderCloudSyncGateway;
        let request = CloudSyncRequest {
            workspace: "local",
            phase: CloudSyncPhase::DryRun,
        };
        let plan = gateway.plan(&request);
        assert!(!plan.can_execute);
        assert!(plan.checkpoints.len() >= 3);
    }

    #[test]
    fn sync_runtime_is_reused_across_calls() -> Result<()> {
        let first = shared_runtime()?;
        let second = shared_runtime()?;
        assert!(std::ptr::eq(first, second));
        Ok(())
    }
}
