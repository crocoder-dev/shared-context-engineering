#![allow(
    dead_code,
    reason = "maintenance entrypoints are wired by later tasks of context/plans/mutation-cursor-ref-reconciliation-wiring.md"
)]

use std::path::Path;
use std::time::Duration;

use anyhow::Result;

use super::git_snapshot::resolve_git_dir;
use super::maintenance_state::{
    read_state, record_failure, record_success, state_path, system_now_ms, write_state_atomically,
    MaintenanceState, StateRead, StoredReport,
};
use super::ref_reconciliation::{
    reconcile_with_held_lock, ReconcileError, ReconcilePhase, ReconciliationOutcome,
    ReconciliationReport,
};
use super::worktree_lock::{acquire_inner_async, WorktreeLockError};
use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;

const EXPLICIT_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SkipReason {
    Busy,
}

#[derive(Debug)]
pub(super) enum ExplicitOutcome {
    Completed(ReconciliationReport),
    CompletedStatePersistFailed {
        report: ReconciliationReport,
        warning: String,
    },
    Failed(ReconcileError),
    Skipped(SkipReason),
}

pub(super) async fn reconcile_explicit<P>(repository_root: &Path, open_db: P) -> ExplicitOutcome
where
    P: std::ops::AsyncFnOnce() -> Result<RepositoryAgentTraceDb>,
{
    reconcile_explicit_with(
        repository_root,
        open_db,
        system_now_ms,
        |path, state| write_state_atomically(path, state, |from, to| std::fs::rename(from, to)),
        |_| {},
        EXPLICIT_LOCK_TIMEOUT,
    )
    .await
}

pub(super) async fn reconcile_explicit_with<P, C, W, H>(
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
    W: Fn(&Path, &MaintenanceState) -> std::io::Result<()>,
    H: FnMut(ReconcilePhase),
{
    let git_dir = match resolve_git_dir(repository_root).await {
        Ok(git_dir) => git_dir,
        Err(source) => return ExplicitOutcome::Failed(ReconcileError::GitDir(source)),
    };

    let lock = match acquire_inner_async(&git_dir, lock_timeout, || {}).await {
        Ok(lock) => lock,
        Err(WorktreeLockError::TimedOut { .. }) => {
            return ExplicitOutcome::Skipped(SkipReason::Busy);
        }
        Err(error) => return ExplicitOutcome::Failed(ReconcileError::Lock(error)),
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
        Ok(ReconciliationOutcome::SkippedNoCheckoutIdentity) => {
            let error = ReconcileError::CheckoutIdentity(anyhow::anyhow!(
                "no checkout identity could be derived"
            ));
            record_failed_pass(&previous, &write_state, &path, now_ms, &error);
            ExplicitOutcome::Failed(error)
        }
        Err(error) => {
            record_failed_pass(&previous, &write_state, &path, now_ms, &error);
            ExplicitOutcome::Failed(error)
        }
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
) -> std::result::Result<(), String>
where
    W: Fn(&Path, &MaintenanceState) -> std::io::Result<()>,
{
    if let Err(error) = previous {
        return Err(format!("maintenance state could not be read: {error}"));
    }
    write_state(path, next).map_err(|error| format!("maintenance state was not recorded: {error}"))
}

fn record_failed_pass<W>(
    previous: &std::io::Result<StateRead>,
    write_state: &W,
    path: &Path,
    now_ms: i64,
    error: &ReconcileError,
) where
    W: Fn(&Path, &MaintenanceState) -> std::io::Result<()>,
{
    let next = record_failure(
        &previous_state(previous),
        now_ms,
        failure_kind(error),
        &error.to_string(),
    );
    let _ = persist(previous, write_state, path, &next);
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
