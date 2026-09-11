use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::checkout::{read_checkout_id, resolve_git_dir};
use crate::services::mutation_trace::store::MutationTraceStore;
use crate::services::mutation_trace::types::{TreeId, WorktreeId};

use super::git_snapshot::{GitSnapshotService, PinInventoryError, PinnedRef};
use super::worktree_lock::{acquire_inner, WorktreeLockError};

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
    /// `read_checkout_id` returned `Err` — a corrupt or unreadable checkout id,
    /// which is **not** the same as an absent one (`Ok(None)` is a clean no-op).
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
pub fn reconcile_worktree<P>(
    repository_root: &Path,
    open_db: P,
) -> std::result::Result<ReconciliationOutcome, ReconcileError>
where
    P: FnOnce() -> Result<RepositoryAgentTraceDb>,
{
    reconcile_worktree_inner(repository_root, open_db, || {})
}

/// Body of [`reconcile_worktree`] with a deterministic test seam:
/// `on_lock_contention` fires once the moment the pass first observes the
/// `WorktreeLock` is already held by another owner (see
/// [`super::worktree_lock::acquire_inner`]). `pub(super)` keeps it reachable
/// from `runtime` and `runtime::tests` but invisible outside `runtime`.
///
/// Every fallible step below runs entirely while holding the worktree's
/// `WorktreeLock`, which is the same lock file `coordinate()` holds across
/// `pin -> recovery -> prepare -> CAS -> marker clear -> return` — the mutual
/// exclusion that makes the pin -> DB-CAS race structurally impossible.
pub(super) fn reconcile_worktree_inner<P, F>(
    repository_root: &Path,
    open_db: P,
    on_lock_contention: F,
) -> std::result::Result<ReconciliationOutcome, ReconcileError>
where
    P: FnOnce() -> Result<RepositoryAgentTraceDb>,
    F: FnOnce(),
{
    let git_dir = resolve_git_dir(repository_root).map_err(ReconcileError::GitDir)?;

    let _lock = acquire_inner(&git_dir, RECONCILIATION_LOCK_TIMEOUT, on_lock_contention)
        .map_err(ReconcileError::Lock)?;

    // The lock is held from here until this function returns.
    let worktree_id = match read_checkout_id(&git_dir).map_err(ReconcileError::CheckoutIdentity)? {
        Some(id) => WorktreeId(id),
        // No current checkout identity to derive a `WorktreeId` and its owned
        // `refs/sce/mutation-cursor/<worktree-id>/` prefix from — nothing to
        // inventory, validate, or delete. Clean no-op; no identity is created.
        None => {
            return Ok(ReconciliationOutcome::SkippedNoCheckoutIdentity);
        }
    };

    let db = open_db().map_err(ReconcileError::AgentTraceDbUnavailable)?;

    let snapshot =
        GitSnapshotService::new(repository_root).map_err(ReconcileError::SnapshotService)?;

    // Inventory the worktree's pins first, so every durable-root read that
    // follows is compared against a fixed observation of the namespace.
    let actual = snapshot
        .list_pins(&worktree_id)
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
        .map_err(ReconcileError::DurableRoots)?;
    let stale: Vec<PinnedRef> = actual
        .iter()
        .filter(|pin| !required_repository.contains(&pin.tree))
        .cloned()
        .collect();

    if !stale.is_empty() {
        snapshot
            .delete_pins(&stale)
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
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
    use crate::services::checkout::get_or_create_checkout_id;
    use crate::services::mutation_trace::store::encode_revision;

    use super::*;

    const NAMESPACE: &str = "refs/sce/mutation-cursor";

    struct Fixture {
        _temp_dir: tempfile::TempDir,
        repo_root: PathBuf,
        db_path: PathBuf,
        worktree_id: WorktreeId,
    }

    fn expect_reconciled(
        outcome: std::result::Result<ReconciliationOutcome, ReconcileError>,
    ) -> ReconciliationReport {
        match outcome.expect("reconciliation should succeed") {
            ReconciliationOutcome::Reconciled(report) => report,
            ReconciliationOutcome::SkippedNoCheckoutIdentity => {
                panic!("expected a Reconciled outcome, got SkippedNoCheckoutIdentity")
            }
        }
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git command should spawn");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("git output should be valid UTF-8")
    }

    fn fixture(label: &str) -> Fixture {
        let temp_dir = tempfile::Builder::new()
            .prefix(&format!("sce-ref-reconciliation-{label}-"))
            .tempdir()
            .expect("test temp directory should be created");

        let repo_root = temp_dir.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo root should be created");
        git(&repo_root, &["init", "--quiet"]);
        git(&repo_root, &["config", "user.email", "test@example.com"]);
        git(&repo_root, &["config", "user.name", "Test"]);
        git(
            &repo_root,
            &["commit", "--allow-empty", "--quiet", "-m", "init"],
        );

        let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
        let checkout_id = get_or_create_checkout_id(&git_dir).expect("checkout id should resolve");

        // The DB lives beside the worktree, never inside it, so it can never
        // perturb a captured tree.
        let db_path = temp_dir.path().join("agent-trace.db");
        RepositoryAgentTraceDb::new_at(&db_path).expect("repository schema DB should open");

        Fixture {
            _temp_dir: temp_dir,
            repo_root,
            db_path,
            worktree_id: WorktreeId(checkout_id),
        }
    }

    impl Fixture {
        fn open_db(&self) -> Result<RepositoryAgentTraceDb> {
            RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&self.db_path)
        }

        fn db(&self) -> RepositoryAgentTraceDb {
            self.open_db().expect("repository DB should reopen")
        }

        fn snapshot(&self) -> GitSnapshotService {
            GitSnapshotService::new(&self.repo_root).expect("snapshot service should construct")
        }

        /// Write `contents` to `file` in the worktree and capture the resulting
        /// tree, so successive distinct contents yield distinct tree SHAs.
        fn capture_after_writing(&self, file: &str, contents: &str) -> TreeId {
            std::fs::write(self.repo_root.join(file), contents)
                .expect("worktree file should write");
            self.snapshot()
                .capture_tree()
                .expect("capture should succeed")
        }

        fn pin(&self, tree: &TreeId) {
            self.snapshot()
                .pin_tree(&self.worktree_id, tree)
                .expect("pin should succeed");
        }

        fn pin_for(&self, worktree_id: &WorktreeId, tree: &TreeId) {
            self.snapshot()
                .pin_tree(worktree_id, tree)
                .expect("pin should succeed");
        }

        fn reconcile(&self) -> std::result::Result<ReconciliationOutcome, ReconcileError> {
            reconcile_worktree(&self.repo_root, || self.open_db())
        }

        fn owned_ref(&self, tree: &TreeId) -> String {
            format!("{NAMESPACE}/{}/{}", self.worktree_id.0, tree.0)
        }

        fn ref_exists(&self, ref_name: &str) -> bool {
            Command::new("git")
                .args(["show-ref", "--verify", "--quiet", ref_name])
                .current_dir(&self.repo_root)
                .status()
                .expect("git show-ref should spawn")
                .success()
        }

        fn ref_representation(&self, ref_name: &str) -> String {
            git(
                &self.repo_root,
                &[
                    "for-each-ref",
                    "--format=%(refname)%00%(objectname)%00%(objecttype)%00%(symref)",
                    ref_name,
                ],
            )
        }

        fn object_type(&self, sha: &str) -> Option<String> {
            let output = Command::new("git")
                .args(["cat-file", "-t", sha])
                .current_dir(&self.repo_root)
                .output()
                .expect("git cat-file should spawn");
            if output.status.success() {
                Some(
                    String::from_utf8(output.stdout)
                        .expect("git cat-file output should be UTF-8")
                        .trim()
                        .to_string(),
                )
            } else {
                None
            }
        }
    }

    fn seed_worktree_cursor(
        db: &RepositoryAgentTraceDb,
        worktree_id: &str,
        revision: u64,
        cursor_tree: &str,
    ) {
        db.execute(
            "INSERT INTO mutation_trace_worktrees
                (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline)
             VALUES (?1, ?2, ?3, 0, 'healthy', 0)",
            (
                worktree_id,
                cursor_tree,
                encode_revision(revision).as_slice(),
            ),
        )
        .expect("worktree row insert should succeed");
    }

    fn seed_event(
        db: &RepositoryAgentTraceDb,
        worktree_id: &str,
        revision: u64,
        before_tree: &str,
        after_tree: &str,
    ) {
        db.execute(
            "INSERT INTO mutation_trace_events
                (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
                 attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id,
                 boundary_event_id)
             VALUES (?1, ?2, ?3, ?4, 0, 'healthy', 'ineligible_unscoped', NULL, 'flush', NULL, NULL)",
            (
                worktree_id,
                encode_revision(revision).as_slice(),
                before_tree,
                after_tree,
            ),
        )
        .expect("event row insert should succeed");
    }

    #[test]
    fn orphan_pin_with_a_worktree_row_is_deleted() {
        let fx = fixture("orphan-with-row");
        let cursor = fx.capture_after_writing("a.txt", "cursor\n");
        let orphan = fx.capture_after_writing("a.txt", "orphan\n");
        fx.pin(&cursor);
        fx.pin(&orphan);
        seed_worktree_cursor(&fx.db(), &fx.worktree_id.0, 1, &cursor.0);

        let report = expect_reconciled(fx.reconcile());

        assert_eq!(
            report,
            ReconciliationReport {
                local_required: 1,
                retained: 1,
                deleted: 1,
            }
        );
        assert!(fx.ref_exists(&fx.owned_ref(&cursor)), "cursor pin retained");
        assert!(!fx.ref_exists(&fx.owned_ref(&orphan)), "orphan pin deleted");
    }

    #[test]
    fn orphan_pin_with_no_worktree_row_is_deleted() {
        let fx = fixture("orphan-no-row");
        let orphan = fx.capture_after_writing("a.txt", "orphan\n");
        fx.pin(&orphan);

        let report = expect_reconciled(fx.reconcile());

        assert_eq!(
            report,
            ReconciliationReport {
                local_required: 0,
                retained: 0,
                deleted: 1,
            }
        );
        assert!(!fx.ref_exists(&fx.owned_ref(&orphan)), "orphan pin deleted");
    }

    #[test]
    fn current_cursor_pin_is_retained_without_a_referencing_event() {
        let fx = fixture("cursor-no-event");
        let cursor = fx.capture_after_writing("a.txt", "cursor\n");
        fx.pin(&cursor);
        seed_worktree_cursor(&fx.db(), &fx.worktree_id.0, 3, &cursor.0);

        let report = expect_reconciled(fx.reconcile());

        assert_eq!(
            report,
            ReconciliationReport {
                local_required: 1,
                retained: 1,
                deleted: 0,
            }
        );
        assert!(
            fx.ref_exists(&fx.owned_ref(&cursor)),
            "current cursor pin retained"
        );
    }

    #[test]
    fn historical_event_before_and_after_pins_are_retained_after_the_cursor_advances() {
        let fx = fixture("historical-retention");
        let tree_a = fx.capture_after_writing("a.txt", "A\n");
        let tree_b = fx.capture_after_writing("a.txt", "B\n");
        let tree_c = fx.capture_after_writing("a.txt", "C\n");
        let tree_d = fx.capture_after_writing("a.txt", "D\n");
        for tree in [&tree_a, &tree_b, &tree_c, &tree_d] {
            fx.pin(tree);
        }

        let db = fx.db();
        seed_worktree_cursor(&db, &fx.worktree_id.0, 3, &tree_d.0);
        seed_event(&db, &fx.worktree_id.0, 1, &tree_a.0, &tree_b.0);
        seed_event(&db, &fx.worktree_id.0, 2, &tree_b.0, &tree_c.0);
        seed_event(&db, &fx.worktree_id.0, 3, &tree_c.0, &tree_d.0);

        let report = expect_reconciled(fx.reconcile());

        assert_eq!(
            report,
            ReconciliationReport {
                local_required: 4,
                retained: 4,
                deleted: 0,
            }
        );
        for tree in [&tree_a, &tree_b, &tree_c, &tree_d] {
            assert!(
                fx.ref_exists(&fx.owned_ref(tree)),
                "historical tree {} pin retained",
                tree.0
            );
        }
    }

    #[test]
    fn a_pin_another_worktree_durably_requires_is_retained() {
        let fx = fixture("cross-worktree-retention");
        let shared = fx.capture_after_writing("a.txt", "shared\n");
        let orphan = fx.capture_after_writing("a.txt", "orphan\n");
        fx.pin(&shared);
        fx.pin(&orphan);

        // Another worktree in the same repository durably references `shared`;
        // this worktree does not.
        seed_worktree_cursor(&fx.db(), "other-worktree", 1, &shared.0);

        let report = expect_reconciled(fx.reconcile());

        assert_eq!(
            report,
            ReconciliationReport {
                local_required: 0,
                retained: 1,
                deleted: 1,
            },
            "a pin another worktree durably needs is retained even though it is \
             not a local root; retained exceeds local_required"
        );
        assert!(
            fx.ref_exists(&fx.owned_ref(&shared)),
            "the repository-wide durable tree pin is retained"
        );
        assert!(
            !fx.ref_exists(&fx.owned_ref(&orphan)),
            "the orphan pin is deleted"
        );
        assert_eq!(
            fx.object_type(&shared.0).as_deref(),
            Some("tree"),
            "the retained ref keeps the shared tree resolvable"
        );
    }

    #[test]
    fn a_missing_required_pin_fails_closed_and_deletes_nothing() {
        let fx = fixture("missing-required-pin");
        let tree_a = fx.capture_after_writing("a.txt", "A\n");
        let tree_b = fx.capture_after_writing("a.txt", "B\n");
        let tree_x = fx.capture_after_writing("a.txt", "X\n");
        // Local durable roots are {A, B}; only A and X are pinned (B has no pin).
        fx.pin(&tree_a);
        fx.pin(&tree_x);

        let db = fx.db();
        seed_worktree_cursor(&db, &fx.worktree_id.0, 1, &tree_b.0);
        seed_event(&db, &fx.worktree_id.0, 1, &tree_a.0, &tree_b.0);

        let error = fx
            .reconcile()
            .expect_err("reconciliation should fail closed");

        match error {
            ReconcileError::MissingRequiredPins { missing } => {
                assert_eq!(
                    missing,
                    vec![tree_b.clone()],
                    "the missing local root is named"
                );
            }
            other => panic!("expected MissingRequiredPins, got {other:?}"),
        }
        assert!(
            fx.ref_exists(&fx.owned_ref(&tree_a)),
            "A's pin is left in place"
        );
        assert!(
            fx.ref_exists(&fx.owned_ref(&tree_x)),
            "X's pin is left in place"
        );
    }

    #[test]
    fn a_malformed_namespace_ref_fails_closed_and_deletes_nothing() {
        let fx = fixture("malformed-ref");
        let cursor = fx.capture_after_writing("a.txt", "cursor\n");
        let orphan = fx.capture_after_writing("a.txt", "orphan\n");
        fx.pin(&cursor);
        fx.pin(&orphan);
        seed_worktree_cursor(&fx.db(), &fx.worktree_id.0, 1, &cursor.0);

        // A symbolic ref inside the SCE mutation-cursor namespace is malformed.
        let symref = format!("{NAMESPACE}/{}/symbolic", fx.worktree_id.0);
        git(
            &fx.repo_root,
            &["symbolic-ref", &symref, &fx.owned_ref(&cursor)],
        );

        let error = fx
            .reconcile()
            .expect_err("reconciliation should fail closed");

        match error {
            ReconcileError::MalformedPin { ref_name, .. } => {
                assert_eq!(ref_name, symref, "the malformed ref is named");
            }
            other => panic!("expected MalformedPin, got {other:?}"),
        }
        assert!(
            fx.ref_exists(&fx.owned_ref(&cursor)),
            "the cursor pin is untouched"
        );
        assert!(
            fx.ref_exists(&fx.owned_ref(&orphan)),
            "the orphan pin is untouched"
        );
    }

    #[test]
    fn reconciliation_is_idempotent() {
        let fx = fixture("idempotent");
        let cursor = fx.capture_after_writing("a.txt", "cursor\n");
        let orphan = fx.capture_after_writing("a.txt", "orphan\n");
        fx.pin(&cursor);
        fx.pin(&orphan);
        seed_worktree_cursor(&fx.db(), &fx.worktree_id.0, 1, &cursor.0);

        let first = expect_reconciled(fx.reconcile());
        assert_eq!(
            first,
            ReconciliationReport {
                local_required: 1,
                retained: 1,
                deleted: 1,
            }
        );

        let second = expect_reconciled(fx.reconcile());
        assert_eq!(
            second,
            ReconciliationReport {
                local_required: 1,
                retained: 1,
                deleted: 0,
            },
            "a second pass with no intervening change deletes nothing and \
             reports identical local_required/retained counts"
        );
    }

    #[test]
    fn reconciliation_deletes_refs_without_reclaiming_objects() {
        let fx = fixture("no-object-reclamation");
        let orphan = fx.capture_after_writing("a.txt", "orphan\n");
        fx.pin(&orphan);

        assert_eq!(fx.object_type(&orphan.0).as_deref(), Some("tree"));

        let report = expect_reconciled(fx.reconcile());
        assert_eq!(report.deleted, 1);
        assert!(
            !fx.ref_exists(&fx.owned_ref(&orphan)),
            "the stale ref is deleted"
        );

        assert_eq!(
            fx.object_type(&orphan.0).as_deref(),
            Some("tree"),
            "the now-unreachable object is still resolvable — reconciliation ran \
             no git gc / git prune"
        );
    }

    #[test]
    fn no_checkout_identity_returns_a_distinct_skipped_outcome() {
        let fx = fixture("no-identity");
        // Remove the checkout id so `read_checkout_id` returns `Ok(None)`.
        let git_dir = resolve_git_dir(&fx.repo_root).expect("git dir should resolve");
        std::fs::remove_file(git_dir.join("sce").join("checkout-id"))
            .expect("checkout id file should be removable");

        let orphan = fx.capture_after_writing("a.txt", "orphan\n");
        fx.pin_for(&fx.worktree_id, &orphan);

        let outcome = fx.reconcile().expect("the skip is an Ok, not an Err");

        assert_eq!(outcome, ReconciliationOutcome::SkippedNoCheckoutIdentity);
        assert!(
            fx.ref_exists(&fx.owned_ref(&orphan)),
            "no pin is inventoried, validated, or deleted without a derivable identity"
        );
    }

    #[test]
    fn a_missing_checkout_identity_skip_touches_no_db_and_no_ref() {
        let fx = fixture("no-identity-no-db");
        let git_dir = resolve_git_dir(&fx.repo_root).expect("git dir should resolve");
        std::fs::remove_file(git_dir.join("sce").join("checkout-id"))
            .expect("checkout id file should be removable");

        let orphan = fx.capture_after_writing("a.txt", "orphan\n");
        fx.pin_for(&fx.worktree_id, &orphan);
        let owned = fx.owned_ref(&orphan);
        let before = fx.ref_representation(&owned);
        assert!(
            !before.is_empty(),
            "the pre-seeded pin ref must exist before the skip"
        );

        let outcome = reconcile_worktree(&fx.repo_root, || {
            panic!("open_db must not be invoked on the missing-checkout-identity skip path")
        })
        .expect("the skip is an Ok, not an Err");

        assert_eq!(outcome, ReconciliationOutcome::SkippedNoCheckoutIdentity);
        assert_eq!(
            fx.ref_representation(&owned),
            before,
            "the skip touches no ref: name, target SHA, object type, and direct/symbolic \
             shape are all structurally unchanged"
        );
    }
}
