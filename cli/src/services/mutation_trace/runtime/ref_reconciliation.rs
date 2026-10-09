use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;

use super::git_snapshot::resolve_worktree_id;
use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::mutation_trace::store::MutationTraceStore;
use crate::services::mutation_trace::types::TreeId;

use super::git_snapshot::{GitSnapshotService, PinInventoryError, PinnedRef};
use super::worktree_lock::{WorktreeLock, WorktreeLockError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconciliationReport {
    pub local_required: usize,
    pub retained: usize,
    pub deleted: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconciliationOutcome {
    Reconciled(ReconciliationReport),
}

#[derive(Debug)]
pub enum ReconcileError {
    GitDir(anyhow::Error),
    Lock(WorktreeLockError),
    CheckoutIdentity(anyhow::Error),
    AgentTraceDbUnavailable(anyhow::Error),
    SnapshotService(anyhow::Error),
    PinInventory(anyhow::Error),
    MalformedPin { ref_name: String, reason: String },
    DurableRoots(anyhow::Error),
    MissingRequiredPins { missing: Vec<TreeId> },
    DeleteTransaction(anyhow::Error),
}

impl std::fmt::Display for ReconcileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReconcileError::Lock(source) => write!(f, "{source}"),
            ReconcileError::MalformedPin { ref_name, reason } => write!(
                f,
                "Malformed ref '{ref_name}' in the mutation-cursor snapshot \
                 namespace; reconciliation deleted nothing: {reason}"
            ),
            ReconcileError::MissingRequiredPins { missing } => write!(
                f,
                "{} durable root(s) of the target worktree have no snapshot pin; \
                 reconciliation failed closed and deleted nothing: {missing:?}",
                missing.len()
            ),
            ReconcileError::AgentTraceDbUnavailable(source) => {
                write!(f, "Repository Agent Trace DB is unavailable: {source}")
            }
            ReconcileError::GitDir(source)
            | ReconcileError::CheckoutIdentity(source)
            | ReconcileError::SnapshotService(source)
            | ReconcileError::PinInventory(source)
            | ReconcileError::DurableRoots(source)
            | ReconcileError::DeleteTransaction(source) => write!(f, "{source}"),
        }
    }
}

impl std::error::Error for ReconcileError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReconcilePhase {
    DbOpened,
    PinsInventoried,
}

pub(super) async fn reconcile_with_held_lock<P, H>(
    repository_root: &Path,
    lock: &WorktreeLock,
    open_db: P,
    mut on_phase: H,
) -> std::result::Result<ReconciliationOutcome, ReconcileError>
where
    P: std::ops::AsyncFnOnce() -> Result<RepositoryAgentTraceDb>,
    H: FnMut(ReconcilePhase),
{
    let worktree_id = resolve_worktree_id(repository_root)
        .await
        .map_err(ReconcileError::CheckoutIdentity)?;

    let mut db = open_db()
        .await
        .map_err(ReconcileError::AgentTraceDbUnavailable)?;
    on_phase(ReconcilePhase::DbOpened);

    let snapshot = GitSnapshotService::new(repository_root)
        .await
        .map_err(ReconcileError::SnapshotService)?;

    let actual = snapshot
        .list_pins(&worktree_id)
        .await
        .map_err(|error| match error {
            PinInventoryError::Git(source) => ReconcileError::PinInventory(source),
            PinInventoryError::MalformedRef { ref_name, reason } => {
                ReconcileError::MalformedPin { ref_name, reason }
            }
        })?;
    on_phase(ReconcilePhase::PinsInventoried);
    let pinned_trees: BTreeSet<TreeId> = actual.iter().map(|pin| pin.tree.clone()).collect();

    let store = MutationTraceStore::new(&mut db);

    let required_local = store
        .load_tree_roots(&worktree_id)
        .await
        .map_err(ReconcileError::DurableRoots)?;
    let missing_local: Vec<TreeId> = required_local.difference(&pinned_trees).cloned().collect();
    if !missing_local.is_empty() {
        return Err(ReconcileError::MissingRequiredPins {
            missing: missing_local,
        });
    }

    let required_repository = store
        .load_all_tree_roots()
        .await
        .map_err(ReconcileError::DurableRoots)?;
    let stale: Vec<PinnedRef> = actual
        .iter()
        .filter(|pin| !required_repository.contains(&pin.tree))
        .cloned()
        .collect();

    if !stale.is_empty() {
        snapshot
            .delete_pins(lock.lease(), &stale)
            .await
            .map_err(ReconcileError::DeleteTransaction)?;
    }

    Ok(ReconciliationOutcome::Reconciled(ReconciliationReport {
        local_required: required_local.len(),
        retained: actual.len() - stale.len(),
        deleted: stale.len(),
    }))
}
