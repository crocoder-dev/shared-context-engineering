use anyhow::{Context, Result};
use serde_json::json;
use std::sync::OnceLock;

use crate::services::local_db::{run_smoke_check, LocalDatabaseTarget};
use crate::services::output_format::OutputFormat;
use crate::services::resilience::{run_with_retry, RetryPolicy};

pub const NAME: &str = "sync";

pub type SyncFormat = OutputFormat;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyncRequest {
    pub format: SyncFormat,
}

const SUPPORTED_PHASES: [CloudSyncPhase; 3] = [
    CloudSyncPhase::PlanOnly,
    CloudSyncPhase::DryRun,
    CloudSyncPhase::Apply,
];

static SYNC_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
const SYNC_SMOKE_RETRY_POLICY: RetryPolicy = RetryPolicy {
    max_attempts: 3,
    timeout_ms: 2_000,
    initial_backoff_ms: 100,
    max_backoff_ms: 400,
};

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

#[derive(Clone, Debug, Eq, PartialEq)]
struct SyncPlaceholderReport {
    workspace: &'static str,
    phase: CloudSyncPhase,
    inserted_rows: u64,
    checkpoints: Vec<&'static str>,
    can_execute: bool,
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

    fn run(&self, request: &CloudSyncRequest) -> Result<SyncPlaceholderReport> {
        let runtime = shared_runtime()?;

        let outcome = runtime
            .block_on(run_with_retry(
                SYNC_SMOKE_RETRY_POLICY,
                "sync.local_db_smoke_check",
                "rerun 'sce sync'; if the failure persists, verify local runtime health with 'sce doctor'.",
                |_| run_smoke_check(LocalDatabaseTarget::InMemory),
            ))
            .context("local Turso smoke check failed after bounded retries")?;

        let plan = self.gateway.plan(request);

        Ok(SyncPlaceholderReport {
            workspace: request.workspace,
            phase: request.phase,
            inserted_rows: outcome.inserted_rows,
            checkpoints: plan.checkpoints,
            can_execute: plan.can_execute,
        })
    }
}

fn shared_runtime() -> Result<&'static tokio::runtime::Runtime> {
    if let Some(runtime) = SYNC_RUNTIME.get() {
        return Ok(runtime);
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .context("failed to create shared tokio runtime for sync placeholder")?;

    Ok(SYNC_RUNTIME.get_or_init(|| runtime))
}

pub fn run_placeholder_sync(request: SyncRequest) -> Result<String> {
    let service = PlaceholderSyncService::new(PlaceholderCloudSyncGateway);
    let cloud_request = CloudSyncRequest {
        workspace: "local",
        phase: CloudSyncPhase::PlanOnly,
    };
    let report = service.run(&cloud_request)?;

    match request.format {
        SyncFormat::Text => Ok(format!(
            "TODO: '{NAME}' cloud workflows are planned and not implemented yet. Local Turso smoke check succeeded ({}) row inserted; cloud sync placeholder enumerates {} phase(s) and plan holds {} checkpoint(s). Next step: rerun with '--format json' for machine-readable placeholder checkpoints.",
            report.inserted_rows,
            SUPPORTED_PHASES.len(),
            report.checkpoints.len()
        )),
        SyncFormat::Json => {
            let payload = json!({
                "status": "ok",
                "command": NAME,
                "placeholder_state": "planned",
                "workspace": report.workspace,
                "phase": phase_name(report.phase),
                "supported_phases": SUPPORTED_PHASES
                    .iter()
                    .map(|phase| phase_name(*phase))
                    .collect::<Vec<_>>(),
                "local_smoke_check": {
                    "status": "ok",
                    "target": "in_memory",
                    "inserted_rows": report.inserted_rows,
                    "retry_policy": {
                        "max_attempts": SYNC_SMOKE_RETRY_POLICY.max_attempts,
                        "timeout_ms": SYNC_SMOKE_RETRY_POLICY.timeout_ms,
                        "initial_backoff_ms": SYNC_SMOKE_RETRY_POLICY.initial_backoff_ms,
                        "max_backoff_ms": SYNC_SMOKE_RETRY_POLICY.max_backoff_ms,
                    },
                },
                "cloud_plan": {
                    "can_execute": report.can_execute,
                    "checkpoints": report.checkpoints,
                },
                "next_step": "Rerun with '--format json' for machine-readable placeholder checkpoints.",
            });

            serde_json::to_string_pretty(&payload)
                .context("failed to serialize sync placeholder report to JSON")
        }
    }
}

fn phase_name(phase: CloudSyncPhase) -> &'static str {
    match phase {
        CloudSyncPhase::PlanOnly => "plan_only",
        CloudSyncPhase::DryRun => "dry_run",
        CloudSyncPhase::Apply => "apply",
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use serde_json::Value;

    use super::{
        run_placeholder_sync, CloudSyncGateway, CloudSyncPhase, CloudSyncRequest,
        PlaceholderCloudSyncGateway, SyncFormat, SyncRequest, NAME,
    };

    use super::shared_runtime;

    #[test]
    fn sync_placeholder_runs_local_smoke_check() -> Result<()> {
        let message = run_placeholder_sync(SyncRequest {
            format: SyncFormat::Text,
        })?;
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

    #[test]
    fn sync_json_output_includes_stable_fields() -> Result<()> {
        let output = run_placeholder_sync(SyncRequest {
            format: SyncFormat::Json,
        })?;
        let parsed: Value = serde_json::from_str(&output)?;
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["command"], NAME);
        assert_eq!(parsed["placeholder_state"], "planned");
        assert_eq!(parsed["workspace"], "local");
        assert_eq!(parsed["phase"], "plan_only");
        assert!(parsed["supported_phases"].is_array());
        assert!(parsed["local_smoke_check"].is_object());
        assert!(parsed["cloud_plan"].is_object());
        assert!(parsed["next_step"].as_str().is_some());
        Ok(())
    }

    #[test]
    fn sync_json_output_is_deterministic_for_same_request() -> Result<()> {
        let first = run_placeholder_sync(SyncRequest {
            format: SyncFormat::Json,
        })?;
        let second = run_placeholder_sync(SyncRequest {
            format: SyncFormat::Json,
        })?;

        assert_eq!(first, second);
        Ok(())
    }
}
