use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Result;

use super::git_snapshot::resolve_worktree_id;
use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::mutation_trace::store::{CasResult, MutationTraceStore};
use crate::services::mutation_trace::types::{TreeId, WorktreeId};

use super::git_snapshot::{GitSnapshotService, PinInventoryError, PinnedRef};
use super::worktree_lock::{WorktreeLock, WorktreeLockError};

pub(super) struct MaintenanceDb {
    pub(super) db: RepositoryAgentTraceDb,
    pub(super) path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconciliationReport {
    pub local_required: usize,
    pub retained: usize,
    pub deleted: usize,
    pub repinned: usize,
    pub pruned_events: usize,
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
    ObjectInventory(anyhow::Error),
    UnrecoverableCursor { tree: TreeId },
    PruneTransaction(anyhow::Error),
    PruneConflict,
    RepinTransaction(anyhow::Error),
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
            ReconcileError::UnrecoverableCursor { tree } => write!(
                f,
                "The worktree's mutation cursor tree {} is absent from this repository's object \
                 store, so its durable history does not belong to this checkout; reconciliation \
                 changed nothing",
                tree.0
            ),
            ReconcileError::PruneConflict => write!(
                f,
                "The worktree's mutation cursor advanced while unrecoverable events were being \
                 pruned; reconciliation changed nothing. Rerun `sce doctor --fix`."
            ),
            ReconcileError::GitDir(source)
            | ReconcileError::CheckoutIdentity(source)
            | ReconcileError::SnapshotService(source)
            | ReconcileError::PinInventory(source)
            | ReconcileError::DurableRoots(source)
            | ReconcileError::ObjectInventory(source)
            | ReconcileError::PruneTransaction(source)
            | ReconcileError::RepinTransaction(source)
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
    P: std::ops::AsyncFnOnce() -> Result<MaintenanceDb>,
    H: FnMut(ReconcilePhase),
{
    let worktree_id = resolve_worktree_id(repository_root)
        .await
        .map_err(ReconcileError::CheckoutIdentity)?;

    let MaintenanceDb { db, path } = open_db()
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
    let mut pinned_trees: BTreeSet<TreeId> = actual.iter().map(|pin| pin.tree.clone()).collect();

    let (mut db, repair) =
        repair_missing_pins(&snapshot, lock, db, &path, &worktree_id, &pinned_trees).await?;
    pinned_trees.extend(repair.repinned.iter().cloned());

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
        repinned: repair.repinned.len(),
        pruned_events: repair.pruned_events,
    }))
}

struct PinRepair {
    repinned: Vec<TreeId>,
    pruned_events: usize,
}

async fn repair_missing_pins(
    snapshot: &GitSnapshotService,
    lock: &WorktreeLock,
    mut db: RepositoryAgentTraceDb,
    db_path: &Path,
    worktree_id: &WorktreeId,
    pinned_trees: &BTreeSet<TreeId>,
) -> std::result::Result<(RepositoryAgentTraceDb, PinRepair), ReconcileError> {
    let required = MutationTraceStore::new(&mut db)
        .load_tree_roots(worktree_id)
        .await
        .map_err(ReconcileError::DurableRoots)?;
    let missing: Vec<TreeId> = required.difference(pinned_trees).cloned().collect();
    if missing.is_empty() {
        return Ok((
            db,
            PinRepair {
                repinned: Vec::new(),
                pruned_events: 0,
            },
        ));
    }

    let absent = snapshot
        .absent_trees(&missing)
        .await
        .map_err(ReconcileError::ObjectInventory)?;

    let mut pruned_events = 0;
    if !absent.is_empty() {
        let store = MutationTraceStore::new(&mut db);
        let Some(worktree_state) = store
            .load_worktree_state(worktree_id)
            .await
            .map_err(ReconcileError::DurableRoots)?
        else {
            return Err(ReconcileError::MissingRequiredPins { missing });
        };
        if absent.contains(&worktree_state.cursor_tree) {
            return Err(ReconcileError::UnrecoverableCursor {
                tree: worktree_state.cursor_tree,
            });
        }

        let revisions: Vec<u64> = store
            .load_mutation_event_trees(worktree_id)
            .await
            .map_err(ReconcileError::DurableRoots)?
            .into_iter()
            .filter(|event| {
                absent.contains(&event.before_tree) || absent.contains(&event.after_tree)
            })
            .map(|event| event.revision)
            .collect();

        drop(db);
        db = RepositoryAgentTraceDb::open_without_migrations_at(db_path)
            .await
            .map_err(ReconcileError::PruneTransaction)?;
        match MutationTraceStore::new(&mut db)
            .prune_mutation_events(worktree_id, worktree_state.revision, &revisions)
            .await
            .map_err(ReconcileError::PruneTransaction)?
        {
            CasResult::Applied => pruned_events = revisions.len(),
            CasResult::Conflict => return Err(ReconcileError::PruneConflict),
        }
    }

    let required = MutationTraceStore::new(&mut db)
        .load_tree_roots(worktree_id)
        .await
        .map_err(ReconcileError::DurableRoots)?;
    let repinned: Vec<TreeId> = required.difference(pinned_trees).cloned().collect();
    let still_absent: Vec<TreeId> = repinned
        .iter()
        .filter(|tree| absent.contains(tree))
        .cloned()
        .collect();
    if !still_absent.is_empty() {
        return Err(ReconcileError::MissingRequiredPins {
            missing: still_absent,
        });
    }

    snapshot
        .pin_trees(lock.lease(), worktree_id, &repinned)
        .await
        .map_err(ReconcileError::RepinTransaction)?;

    Ok((
        db,
        PinRepair {
            repinned,
            pruned_events,
        },
    ))
}
