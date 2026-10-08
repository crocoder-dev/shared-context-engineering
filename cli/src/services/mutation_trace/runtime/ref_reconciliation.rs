use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;

use super::git_snapshot::resolve_worktree_id;
use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::mutation_trace::store::MutationTraceStore;
use crate::services::mutation_trace::types::TreeId;

use super::git_snapshot::{resolve_git_dir, GitSnapshotService, PinInventoryError, PinnedRef};
use super::worktree_lock::{acquire_inner_async, WorktreeLockError};

const RECONCILIATION_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

/// Outcome counts of one successful reconciliation pass.
///
/// - `local_required` — the target worktree's own durable-root count
///   (`load_tree_roots(W).len()`), the left side of the local consistency
///   invariant.
/// - `retained` — `actual_W.len() - deleted`: pins left in place, whether
///   because the target worktree still needs their tree or because another
///   worktree in the repository durably does.
/// - `deleted` — pins actually removed (inventoried under `W`'s prefix, tree
///   absent from the repository-wide durable root set).
///
/// `retained == local_required` is **not** an invariant — a pin retained only
/// because another worktree durably needs its tree counts toward `retained`
/// but not `local_required`. For `ReconciliationOutcome::Reconciled(report)`
/// the only relation that holds is `report.local_required <= report.retained`.
/// `ReconciliationOutcome::SkippedNoCheckoutIdentity` carries no report, so no
/// report invariant applies to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconciliationReport {
    pub local_required: usize,
    pub retained: usize,
    pub deleted: usize,
}

/// Outcome of one reconciliation pass: a real pass that ran, carrying its
/// [`ReconciliationReport`], versus a skip because no current checkout identity
/// could be derived (an `Ok`, never an `Err`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconciliationOutcome {
    Reconciled(ReconciliationReport),
    SkippedNoCheckoutIdentity,
}

/// Why a reconciliation pass could not complete. One variant per fallible step,
/// no `Other` catch-all — mirroring `CoordinateError`'s convention. Every
/// non-`Ok` outcome leaves the SCE ref namespace in a consistent state: either
/// untouched, or (only on `Ok`) with exactly the stale refs gone.
#[derive(Debug)]
pub enum ReconcileError {
    /// `resolve_git_dir` failed.
    GitDir(anyhow::Error),
    /// The worktree's `WorktreeLock` could not be acquired (timeout or I/O).
    Lock(WorktreeLockError),
    /// Git topology could not be resolved into a worktree identity.
    CheckoutIdentity(anyhow::Error),
    /// The caller-supplied `open_db` provider returned `Err`. This is a
    /// reconciliation maintenance error only: it never arms
    /// `ExternalTaintMarker` and never becomes
    /// `CoordinateError::AgentTraceDbUnavailable`, because no mutation boundary
    /// is being coordinated.
    AgentTraceDbUnavailable(anyhow::Error),
    /// `GitSnapshotService::new` failed.
    SnapshotService(anyhow::Error),
    /// `git for-each-ref` itself failed to execute or exited non-zero
    /// (`PinInventoryError::Git`).
    PinInventory(anyhow::Error),
    /// A ref inside the SCE mutation-cursor namespace is not shaped like a
    /// `pin_tree` output (`PinInventoryError::MalformedRef`). Reconciliation
    /// deletes nothing.
    MalformedPin { ref_name: String, reason: String },
    /// `load_tree_roots` / `load_all_tree_roots` failed (DB query error,
    /// migration `003` absent, ...).
    DurableRoots(anyhow::Error),
    /// A durable root of the **target** worktree has no live pin — the local
    /// consistency invariant is violated. Fail closed: nothing is deleted.
    MissingRequiredPins { missing: Vec<TreeId> },
    /// The atomic `delete_pins` transaction failed (including a ref that
    /// changed since inventory). Nothing is deleted.
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

/// Reconcile one worktree's SCE snapshot pins: remove orphan / unreferenced
/// pins while retaining every tree any current or historical durable
/// mutation-cursor state in the repository still references.
///
/// Module-private to `runtime`, exactly like `coordinate` — never re-exported
/// outside mutation-trace `runtime`. It is a one-line delegation to
/// [`reconcile_worktree_inner`] with a no-op lock-contention closure.
pub async fn reconcile_worktree<P>(
    repository_root: &Path,
    open_db: P,
) -> std::result::Result<ReconciliationOutcome, ReconcileError>
where
    P: std::ops::AsyncFnOnce() -> Result<RepositoryAgentTraceDb>,
{
    reconcile_worktree_inner(repository_root, open_db, || {}).await
}

pub(super) async fn reconcile_worktree_inner<P, F>(
    repository_root: &Path,
    open_db: P,
    on_lock_contention: F,
) -> std::result::Result<ReconciliationOutcome, ReconcileError>
where
    P: std::ops::AsyncFnOnce() -> Result<RepositoryAgentTraceDb>,
    F: FnOnce() + Send + 'static,
{
    let git_dir = resolve_git_dir(repository_root)
        .await
        .map_err(ReconcileError::GitDir)?;

    let lock = acquire_inner_async(&git_dir, RECONCILIATION_LOCK_TIMEOUT, on_lock_contention)
        .await
        .map_err(ReconcileError::Lock)?;

    let worktree_id = resolve_worktree_id(repository_root)
        .await
        .map_err(ReconcileError::CheckoutIdentity)?;

    let db = open_db()
        .await
        .map_err(ReconcileError::AgentTraceDbUnavailable)?;

    let snapshot = GitSnapshotService::new(repository_root)
        .await
        .map_err(ReconcileError::SnapshotService)?;

    // Inventory the worktree's pins first, so every durable-root read that
    // follows is compared against a fixed observation of the namespace.
    let actual = snapshot
        .list_pins(&worktree_id)
        .await
        .map_err(|error| match error {
            PinInventoryError::Git(source) => ReconcileError::PinInventory(source),
            PinInventoryError::MalformedRef { ref_name, reason } => {
                ReconcileError::MalformedPin { ref_name, reason }
            }
        })?;
    let pinned_trees: BTreeSet<TreeId> = actual.iter().map(|pin| pin.tree.clone()).collect();

    let store = MutationTraceStore::new(&db);

    // Local consistency invariant (a strictly per-worktree check): every tree
    // the target worktree's own durable evidence references must still have a
    // live pin, or the pass fails closed and deletes nothing.
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

    // Deletion safety invariant: an owned ref is removed only when its tree is
    // outside the durable root set of **every** worktree in the repository —
    // linked worktrees share one object database, so an A-owned ref may be the
    // last SCE ref protecting a tree that only worktree B durably requires.
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
