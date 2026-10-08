use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;

use super::git_snapshot::resolve_worktree_id;
use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::mutation_trace::store::MutationTraceStore;
use crate::services::mutation_trace::types::TreeId;

use super::git_snapshot::{resolve_git_dir, GitSnapshotService, PinInventoryError, PinnedRef};
use super::worktree_lock::{acquire_inner_async, WorktreeLock, WorktreeLockError};

#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
const RECONCILIATION_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
pub struct ReconciliationReport {
    pub local_required: usize,
    pub retained: usize,
    pub deleted: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
pub enum ReconciliationOutcome {
    Reconciled(ReconciliationReport),
    SkippedNoCheckoutIdentity,
}

#[derive(Debug)]
#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
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

#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
pub async fn reconcile_worktree<P>(
    repository_root: &Path,
    open_db: P,
) -> std::result::Result<ReconciliationOutcome, ReconcileError>
where
    P: std::ops::AsyncFnOnce() -> Result<RepositoryAgentTraceDb>,
{
    reconcile_worktree_inner(repository_root, open_db, || {}).await
}

#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
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

    reconcile_with_held_lock(repository_root, &lock, open_db, |_| {}).await
}

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

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;
    use std::time::Duration;

    use super::super::git_snapshot::{resolve_git_dir, GitSnapshotService};
    use super::super::maintenance_state::{
        read_state, state_path, write_state_atomically, StateRead,
    };
    use super::super::ref_maintenance::{reconcile_explicit_with, ExplicitOutcome, SkipReason};
    use super::super::worktree_lock::WorktreeLock;
    use super::{ReconcileError, ReconciliationReport};
    use crate::services::agent_trace_db::repository::{
        ExistingRepositoryDbError, RepositoryAgentTraceDb,
    };
    use crate::services::mutation_trace::types::WorktreeId;

    const NOW: i64 = 1_000_000_000_000;
    const REPOSITORY_ID: &str = "repo-under-test";

    fn git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git command");
        assert!(output.status.success(), "git {args:?} failed");
    }

    fn init_repo(root: &Path) {
        std::fs::create_dir_all(root).expect("repo dir");
        git(root, &["init", "-q"]);
        git(root, &["config", "user.email", "t@example.invalid"]);
        git(root, &["config", "user.name", "T"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("tracked.txt"), "tracked\n").expect("write file");
        git(root, &["add", "tracked.txt"]);
        git(root, &["commit", "-q", "-m", "initial"]);
    }

    async fn create_db(path: &Path, repository_id: &str) {
        let db = RepositoryAgentTraceDb::new_at(path)
            .await
            .expect("create db");
        db.verify_or_initialize_repository_metadata(repository_id)
            .await
            .expect("initialize metadata");
    }

    async fn pin_orphan(root: &Path) -> String {
        let snapshot = GitSnapshotService::new(root).await.expect("snapshot");
        let tree = snapshot.capture_tree().await.expect("tree");
        let git_dir = resolve_git_dir(root).await.expect("git dir");
        let lock = WorktreeLock::acquire_async(&git_dir, Duration::from_secs(10))
            .await
            .expect("lock");
        snapshot
            .pin_tree(lock.lease(), &WorktreeId("main".to_string()), &tree)
            .await
            .expect("pin");
        tree.0
    }

    fn write(
        path: &Path,
        state: &super::super::maintenance_state::MaintenanceState,
    ) -> std::io::Result<()> {
        write_state_atomically(path, state, |from, to| std::fs::rename(from, to))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ref_reconciliation_explicit_pass_deletes_orphan_and_records_success() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("repo");
        init_repo(&root);
        let db_path = dir.path().join("state").join("agent-trace.db");
        std::fs::create_dir_all(db_path.parent().expect("parent")).expect("db dir");
        create_db(&db_path, REPOSITORY_ID).await;
        pin_orphan(&root).await;

        let mut phases = Vec::new();
        let outcome = reconcile_explicit_with(
            &root,
            async || {
                RepositoryAgentTraceDb::open_verified_existing_at(&db_path, REPOSITORY_ID)
                    .await
                    .map(|(db, _)| db)
                    .map_err(anyhow::Error::new)
            },
            || NOW,
            write,
            |phase| phases.push(phase),
            Duration::from_secs(10),
        )
        .await;

        let ExplicitOutcome::Completed(report) = outcome else {
            panic!("expected completed pass, got {outcome:?}");
        };
        assert_eq!(
            report,
            ReconciliationReport {
                local_required: 0,
                retained: 0,
                deleted: 1,
            }
        );
        assert_eq!(phases.len(), 2);

        let snapshot = GitSnapshotService::new(&root).await.expect("snapshot");
        assert!(snapshot
            .list_pins(&WorktreeId("main".to_string()))
            .await
            .expect("pins")
            .is_empty());

        let git_dir = resolve_git_dir(&root).await.expect("git dir");
        let StateRead::Valid(state) = read_state(&state_path(&git_dir)).expect("state") else {
            panic!("expected recorded state");
        };
        assert_eq!(state.last_success, Some(NOW));
        assert_eq!(state.consecutive_failures, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ref_reconciliation_explicit_pass_failure_keeps_refs_and_counts_streak() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("repo");
        init_repo(&root);
        let missing_db = dir.path().join("absent").join("agent-trace.db");
        let tree = pin_orphan(&root).await;

        for expected_streak in 1..=2 {
            let outcome = reconcile_explicit_with(
                &root,
                async || {
                    RepositoryAgentTraceDb::open_verified_existing_at(&missing_db, REPOSITORY_ID)
                        .await
                        .map(|(db, _)| db)
                        .map_err(anyhow::Error::new)
                },
                || NOW,
                write,
                |_| {},
                Duration::from_secs(10),
            )
            .await;
            assert!(matches!(
                outcome,
                ExplicitOutcome::Failed(ReconcileError::AgentTraceDbUnavailable(_))
            ));

            let git_dir = resolve_git_dir(&root).await.expect("git dir");
            let StateRead::Valid(state) = read_state(&state_path(&git_dir)).expect("state") else {
                panic!("expected recorded state");
            };
            assert_eq!(state.consecutive_failures, expected_streak);
            assert_eq!(state.last_success, None);
        }

        let snapshot = GitSnapshotService::new(&root).await.expect("snapshot");
        let pins = snapshot
            .list_pins(&WorktreeId("main".to_string()))
            .await
            .expect("pins");
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].tree.0, tree);
        assert!(!missing_db.exists());
        assert!(!missing_db.parent().expect("parent").exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ref_reconciliation_explicit_pass_skips_when_lock_is_held_without_touching_state() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("repo");
        init_repo(&root);
        let git_dir = resolve_git_dir(&root).await.expect("git dir");
        let held = WorktreeLock::acquire_async(&git_dir, Duration::from_secs(10))
            .await
            .expect("hold lock");

        let outcome = reconcile_explicit_with(
            &root,
            async || -> anyhow::Result<RepositoryAgentTraceDb> { unreachable!("db must not open") },
            || NOW,
            write,
            |_| {},
            Duration::ZERO,
        )
        .await;

        assert!(matches!(
            outcome,
            ExplicitOutcome::Skipped(SkipReason::Busy)
        ));
        assert!(!state_path(&git_dir).exists());
        drop(held);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ref_reconciliation_explicit_pass_reports_state_persist_failure_distinctly() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("repo");
        init_repo(&root);
        let db_path = dir.path().join("agent-trace.db");
        create_db(&db_path, REPOSITORY_ID).await;
        pin_orphan(&root).await;

        let outcome = reconcile_explicit_with(
            &root,
            async || {
                RepositoryAgentTraceDb::open_verified_existing_at(&db_path, REPOSITORY_ID)
                    .await
                    .map(|(db, _)| db)
                    .map_err(anyhow::Error::new)
            },
            || NOW,
            |_, _| Err(std::io::Error::other("disk full")),
            |_| {},
            Duration::from_secs(10),
        )
        .await;

        let ExplicitOutcome::CompletedStatePersistFailed { report, warning } = outcome else {
            panic!("expected persist-failed completion, got {outcome:?}");
        };
        assert_eq!(report.deleted, 1);
        assert!(warning.contains("disk full"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ref_reconciliation_verified_opener_reports_typed_failures_without_writes() {
        let dir = tempfile::tempdir().expect("temp dir");

        let missing = dir.path().join("nested").join("agent-trace.db");
        let error = RepositoryAgentTraceDb::open_verified_existing_at(&missing, REPOSITORY_ID)
            .await
            .err()
            .expect("missing db fails");
        assert!(matches!(error, ExistingRepositoryDbError::Missing { .. }));
        assert!(!dir.path().join("nested").exists());

        let valid = dir.path().join("valid.db");
        create_db(&valid, REPOSITORY_ID).await;
        let (_, metadata) =
            RepositoryAgentTraceDb::open_verified_existing_at(&valid, REPOSITORY_ID)
                .await
                .expect("valid db opens");
        assert_eq!(metadata.repository_id, REPOSITORY_ID);

        let error = RepositoryAgentTraceDb::open_verified_existing_at(&valid, "another-repo")
            .await
            .err()
            .expect("mismatch fails");
        assert!(matches!(
            error,
            ExistingRepositoryDbError::RepositoryMismatch { .. }
        ));

        let no_metadata = dir.path().join("no-metadata.db");
        RepositoryAgentTraceDb::new_at(&no_metadata)
            .await
            .expect("migrated db");
        let error = RepositoryAgentTraceDb::open_verified_existing_at(&no_metadata, REPOSITORY_ID)
            .await
            .err()
            .expect("missing metadata fails");
        assert!(matches!(error, ExistingRepositoryDbError::MissingMetadata));
        let db = RepositoryAgentTraceDb::open_existing_without_migrations_at(&no_metadata)
            .await
            .expect("reopen");
        assert!(db
            .verify_existing_repository_metadata(REPOSITORY_ID)
            .await
            .is_err());

        let unmigrated = dir.path().join("unmigrated.db");
        drop(
            RepositoryAgentTraceDb::open_without_migrations_at(&unmigrated)
                .await
                .expect("create empty db"),
        );
        let error = RepositoryAgentTraceDb::open_verified_existing_at(&unmigrated, REPOSITORY_ID)
            .await
            .err()
            .expect("incompatible schema fails");
        assert!(matches!(
            error,
            ExistingRepositoryDbError::IncompatibleSchema(_)
        ));
    }
}
