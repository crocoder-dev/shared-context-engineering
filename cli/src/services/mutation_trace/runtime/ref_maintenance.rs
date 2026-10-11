use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};

use super::git_snapshot::resolve_git_dir;
use super::maintenance_state::{
    read_state, record_failure, record_success, state_path, system_now_ms, write_state_atomically,
    MaintenanceState, PersistFailure, StateRead, StoredReport,
};
use super::ref_reconciliation::{
    reconcile_with_held_lock, ReconcileError, ReconcilePhase, ReconciliationOutcome,
    ReconciliationReport,
};
use super::worktree_lock::{acquire_inner_async, WorktreeLockError};
use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_storage::{
    resolve_existing_agent_trace_storage_for_maintenance, AgentTraceStorageContext,
};
use crate::services::config;

#[cfg(test)]
mod tests;

const EXPLICIT_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SkipReason {
    Busy,
}

#[derive(Debug)]
pub(super) enum StatePersistWarning {
    PreviousStateUnreadable(std::io::Error),
    WriteFailed(PersistFailure),
}

impl std::fmt::Display for StatePersistWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StatePersistWarning::PreviousStateUnreadable(error) => {
                write!(f, "maintenance state could not be read: {error}")
            }
            StatePersistWarning::WriteFailed(failure) => write!(f, "{failure}"),
        }
    }
}

#[derive(Debug)]
pub(super) enum ExplicitOutcome {
    Completed(ReconciliationReport),
    CompletedStatePersistFailed {
        report: ReconciliationReport,
        warning: StatePersistWarning,
    },
    Failed {
        error: ReconcileError,
        state_warning: Option<StatePersistWarning>,
    },
    Skipped(SkipReason),
}

pub(super) async fn reconcile_explicit(repository_root: &Path) -> ExplicitOutcome {
    reconcile_explicit_with(
        repository_root,
        async || open_authoritative_db(repository_root).await,
        system_now_ms,
        write_state_atomically,
        |_| {},
        EXPLICIT_LOCK_TIMEOUT,
    )
    .await
}

async fn open_authoritative_db(repository_root: &Path) -> Result<RepositoryAgentTraceDb> {
    let storage_config = config::resolve_agent_trace_storage_runtime_config(repository_root)
        .context("Failed to resolve Agent Trace repository storage config.")?;
    let context = AgentTraceStorageContext {
        repository_root,
        explicit_repository_id: storage_config.repository_id.as_deref(),
        repository_remote: &storage_config.repository_remote,
    };
    resolve_existing_agent_trace_storage_for_maintenance(&context)
        .await
        .map(|storage| storage.db)
}

async fn reconcile_explicit_with<P, C, W, H>(
    repository_root: &Path,
    open_db: P,
    now: C,
    write_state: W,
    on_phase: H,
    lock_timeout: Duration,
) -> ExplicitOutcome
where
    P: std::ops::AsyncFnOnce() -> Result<RepositoryAgentTraceDb>,
    C: Fn() -> i64,
    W: Fn(&Path, &MaintenanceState) -> std::result::Result<(), PersistFailure>,
    H: FnMut(ReconcilePhase),
{
    let git_dir = match resolve_git_dir(repository_root).await {
        Ok(git_dir) => git_dir,
        Err(source) => {
            return ExplicitOutcome::Failed {
                error: ReconcileError::GitDir(source),
                state_warning: None,
            };
        }
    };

    let lock = match acquire_inner_async(&git_dir, lock_timeout, || {}).await {
        Ok(lock) => lock,
        Err(WorktreeLockError::TimedOut { .. }) => {
            return ExplicitOutcome::Skipped(SkipReason::Busy);
        }
        Err(error) => {
            return ExplicitOutcome::Failed {
                error: ReconcileError::Lock(error),
                state_warning: None,
            };
        }
    };

    let result = reconcile_with_held_lock(repository_root, &lock, open_db, on_phase).await;
    let path = state_path(&git_dir);
    let now_ms = now();
    let previous = read_state(&path);

    let outcome = match result {
        Ok(ReconciliationOutcome::Reconciled(report)) => {
            let next = record_success(
                &previous_state(&previous),
                now_ms,
                StoredReport {
                    retained: report.retained,
                    deleted: report.deleted,
                    local_required: report.local_required,
                },
            );
            match persist(&previous, &write_state, &path, &next) {
                Ok(()) => ExplicitOutcome::Completed(report),
                Err(warning) => ExplicitOutcome::CompletedStatePersistFailed { report, warning },
            }
        }
        Err(error) => record_failed_pass(&previous, &write_state, &path, now_ms, error),
    };

    drop(lock);
    outcome
}

fn previous_state(previous: &std::io::Result<StateRead>) -> MaintenanceState {
    match previous {
        Ok(read) => read.clone().into_state(),
        Err(_) => MaintenanceState::default(),
    }
}

fn persist<W>(
    previous: &std::io::Result<StateRead>,
    write_state: &W,
    path: &Path,
    next: &MaintenanceState,
) -> std::result::Result<(), StatePersistWarning>
where
    W: Fn(&Path, &MaintenanceState) -> std::result::Result<(), PersistFailure>,
{
    if let Err(error) = previous {
        return Err(StatePersistWarning::PreviousStateUnreadable(
            std::io::Error::new(error.kind(), error.to_string()),
        ));
    }
    write_state(path, next).map_err(StatePersistWarning::WriteFailed)
}

fn record_failed_pass<W>(
    previous: &std::io::Result<StateRead>,
    write_state: &W,
    path: &Path,
    now_ms: i64,
    error: ReconcileError,
) -> ExplicitOutcome
where
    W: Fn(&Path, &MaintenanceState) -> std::result::Result<(), PersistFailure>,
{
    let next = record_failure(
        &previous_state(previous),
        now_ms,
        failure_kind(&error),
        &error.to_string(),
    );
    let state_warning = persist(previous, write_state, path, &next).err();
    ExplicitOutcome::Failed {
        error,
        state_warning,
    }
}

fn failure_kind(error: &ReconcileError) -> &'static str {
    match error {
        ReconcileError::GitDir(_) => "git_dir",
        ReconcileError::Lock(_) => "lock",
        ReconcileError::CheckoutIdentity(_) => "checkout_identity",
        ReconcileError::AgentTraceDbUnavailable(_) => "agent_trace_db_unavailable",
        ReconcileError::SnapshotService(_) => "snapshot_service",
        ReconcileError::PinInventory(_) => "pin_inventory",
        ReconcileError::MalformedPin { .. } => "malformed_pin",
        ReconcileError::DurableRoots(_) => "durable_roots",
        ReconcileError::MissingRequiredPins { .. } => "missing_required_pins",
        ReconcileError::DeleteTransaction(_) => "delete_transaction",
    }
}

#[cfg(test)]
async fn open_authoritative_db_at_state_root(
    repository_root: &Path,
    state_root: &Path,
) -> Result<RepositoryAgentTraceDb> {
    let context = AgentTraceStorageContext {
        repository_root,
        explicit_repository_id: None,
        repository_remote: "origin",
    };
    crate::services::agent_trace_storage::resolve_existing_agent_trace_storage_for_maintenance_at_state_root(
        &context,
        state_root,
    )
    .await
    .map(|storage| storage.db)
}
