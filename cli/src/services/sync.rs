use anyhow::{Context, Result};
use serde_json::json;
use std::sync::OnceLock;

use crate::services::auth;
use crate::services::config;
use crate::services::local_db::{run_smoke_check, LocalDatabaseTarget};
use crate::services::output_format::OutputFormat;
use crate::services::resilience::{run_with_retry, RetryPolicy};
use crate::services::style::{self};
use crate::services::token_storage;

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
        .enable_io()
        .enable_time()
        .build()
        .context("failed to create shared tokio runtime for sync placeholder")?;

    Ok(SYNC_RUNTIME.get_or_init(|| runtime))
}

pub fn run_placeholder_sync(request: SyncRequest) -> Result<String> {
    ensure_sync_auth_if_expired()?;

    let service = PlaceholderSyncService::new(PlaceholderCloudSyncGateway);
    let cloud_request = CloudSyncRequest {
        workspace: "local",
        phase: CloudSyncPhase::PlanOnly,
    };
    let report = service.run(&cloud_request)?;

    match request.format {
        SyncFormat::Text => Ok(format!(
            "{}: '{}' cloud workflows are planned and not implemented yet. {} {} row inserted; cloud sync placeholder enumerates {} phase(s) and plan holds {} checkpoint(s). {}: rerun with '--format json' for machine-readable placeholder checkpoints.",
            style::label("TODO"),
            style::command_name(NAME),
            style::label("Local Turso smoke check succeeded"),
            style::value(&report.inserted_rows.to_string()),
            style::value(&SUPPORTED_PHASES.len().to_string()),
            style::value(&report.checkpoints.len().to_string()),
            style::label("Next step")
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

fn ensure_sync_auth_if_expired() -> Result<()> {
    let Some(stored_tokens) = token_storage::load_tokens()? else {
        return Ok(());
    };

    if !auth::is_stored_token_expired(&stored_tokens)? {
        return Ok(());
    }

    let cwd = std::env::current_dir()
        .context("failed to determine current directory for auth config resolution")?;
    let client_id = config::resolve_auth_runtime_config(&cwd)?
        .workos_client_id
        .value
        .unwrap_or_default();
    let client = reqwest::Client::new();
    let runtime = shared_runtime()?;

    runtime
        .block_on(auth::ensure_valid_token(
            &client,
            auth::WORKOS_DEFAULT_BASE_URL,
            &client_id,
        ))
        .map(|_| ())
        .map_err(|error| {
            anyhow::anyhow!(error.to_string())
                .context("failed to renew expired authentication before running 'sce sync'")
        })
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

    use super::{CloudSyncGateway, CloudSyncPhase, CloudSyncRequest, PlaceholderCloudSyncGateway};

    use super::shared_runtime;

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
