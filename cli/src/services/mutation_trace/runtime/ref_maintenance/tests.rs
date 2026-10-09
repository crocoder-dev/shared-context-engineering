use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::super::git_snapshot::{resolve_git_dir, GitSnapshotService, PinnedRef};
use super::super::maintenance_state::{
    evaluate_recommendation, read_state, state_path, write_state_atomically,
    write_state_atomically_with, FaultInjectingFilesystem, MaintenanceState, PersistFailure,
    PersistPhase, StateRead,
};
use super::super::ref_reconciliation::{
    reconcile_with_held_lock, ReconcileError, ReconcilePhase, ReconciliationOutcome,
    ReconciliationReport,
};
use super::super::worktree_lock::acquire_inner_async;
use super::{
    open_authoritative_db_at_state_root, reconcile_explicit_with, ExplicitOutcome, SkipReason,
    StatePersistWarning,
};
use crate::services::agent_trace_db::repository::{
    ExistingRepositoryDbError, RepositoryAgentTraceDb,
};
use crate::services::default_paths::agent_trace_db_path_for_repository_at;
use crate::services::mutation_trace::types::WorktreeId;
use crate::services::repository_identity::resolve::resolve_repository_identity;

const NOW: i64 = 1_000_000_000_000;
const OTHER_REPOSITORY_ID: &str = "some-other-repository";

mod coordination;
mod cross_process;
mod fail_closed;
mod state_recovery;
mod support;
mod worktrees;

struct Fixture {
    dir: tempfile::TempDir,
    root: PathBuf,
    state_root: PathBuf,
    repository_id: String,
}

impl Fixture {
    fn new(remote_url: &str) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("repo");
        init_repo(&root, remote_url);
        let repository_id = resolve_repository_identity(&root, None, "origin")
            .expect("identity")
            .identity
            .repository_id;
        let state_root = dir.path().join("state");
        Self {
            dir,
            root,
            state_root,
            repository_id,
        }
    }

    fn db_path(&self) -> PathBuf {
        agent_trace_db_path_for_repository_at(&self.state_root, &self.repository_id)
            .expect("db path")
    }

    async fn create_db(&self) {
        create_db(&self.db_path(), &self.repository_id).await;
    }

    async fn run(&self) -> (ExplicitOutcome, Vec<ReconcilePhase>) {
        self.run_with(write_state_atomically).await
    }

    async fn run_with<W>(&self, write_state: W) -> (ExplicitOutcome, Vec<ReconcilePhase>)
    where
        W: Fn(&Path, &MaintenanceState) -> Result<(), PersistFailure>,
    {
        let mut phases = Vec::new();
        let outcome = reconcile_explicit_with(
            &self.root,
            async || open_authoritative_db_at_state_root(&self.root, &self.state_root).await,
            || NOW,
            write_state,
            |phase| phases.push(phase),
            Duration::from_secs(10),
        )
        .await;
        (outcome, phases)
    }

    async fn pins(&self) -> Vec<PinnedRef> {
        GitSnapshotService::new(&self.root)
            .await
            .expect("snapshot")
            .list_pins(&WorktreeId("main".to_string()))
            .await
            .expect("pins")
    }

    async fn stored_state(&self) -> StateRead {
        let git_dir = resolve_git_dir(&self.root).await.expect("git dir");
        read_state(&state_path(&git_dir)).expect("state")
    }
}

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git command");
    assert!(output.status.success(), "git {args:?} failed");
}

fn init_repo(root: &Path, remote_url: &str) {
    std::fs::create_dir_all(root).expect("repo dir");
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "t@example.invalid"]);
    git(root, &["config", "user.name", "T"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    git(root, &["remote", "add", "origin", remote_url]);
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

fn ref_inventory(root: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args([
            "for-each-ref",
            "--format=%(refname) %(objectname) %(symref)",
            "refs/sce/",
        ])
        .current_dir(root)
        .output()
        .expect("for-each-ref");
    assert!(output.status.success(), "for-each-ref failed");
    let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    lines.sort();
    lines
}

async fn pin_orphan(root: &Path) -> String {
    let snapshot = GitSnapshotService::new(root).await.expect("snapshot");
    let tree = snapshot.capture_tree().await.expect("tree");
    let git_dir = resolve_git_dir(root).await.expect("git dir");
    let lock = acquire_inner_async(&git_dir, Duration::from_secs(10), || {})
        .await
        .expect("lock");
    snapshot
        .pin_tree(lock.lease(), &WorktreeId("main".to_string()), &tree)
        .await
        .expect("pin");
    tree.0
}

async fn seed_durable_root(db_path: &Path, tree: &str) {
    seed_durable_root_for(db_path, "main", tree).await;
}

async fn seed_durable_root_for(db_path: &Path, worktree: &str, tree: &str) {
    let db = RepositoryAgentTraceDb::open_without_migrations_at(db_path)
        .await
        .expect("writer");
    db.execute(
        "INSERT INTO mutation_trace_worktrees
            (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline)
         VALUES (?1, ?2, x'0000000000000000', 0, 'healthy', 0)",
        (worktree, tree),
    )
    .await
    .expect("seed durable root");
}

fn unavailable_cause(error: &ReconcileError) -> &ExistingRepositoryDbError {
    let ReconcileError::AgentTraceDbUnavailable(source) = error else {
        panic!("expected AgentTraceDbUnavailable, got {error:?}");
    };
    source
        .downcast_ref::<ExistingRepositoryDbError>()
        .expect("verification failure keeps its typed source")
}

fn failing_write(
    phase: PersistPhase,
) -> impl Fn(&Path, &MaintenanceState) -> Result<(), PersistFailure> {
    move |path, state| {
        write_state_atomically_with(&FaultInjectingFilesystem::failing_at(phase), path, state)
    }
}

fn staging_leftovers(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .expect("read dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains("staging"))
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_verified_database_reconciles_orphan_pin_and_records_success() {
    let fixture = Fixture::new("https://example.invalid/org/repo-a.git");
    fixture.create_db().await;
    pin_orphan(&fixture.root).await;

    let (outcome, phases) = fixture.run().await;

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
    assert_eq!(
        phases,
        vec![ReconcilePhase::DbOpened, ReconcilePhase::PinsInventoried]
    );
    assert!(fixture.pins().await.is_empty());
    let StateRead::Valid(state) = fixture.stored_state().await else {
        panic!("expected recorded state");
    };
    assert_eq!(state.last_success, Some(NOW));
    assert_eq!(state.consecutive_failures, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_database_of_another_repository_cannot_authorize_deletion() {
    let repo_a = Fixture::new("https://example.invalid/org/repo-a.git");
    let repo_b = Fixture::new("https://example.invalid/org/repo-b.git");
    assert_ne!(repo_a.repository_id, repo_b.repository_id);
    repo_b.create_db().await;
    pin_orphan(&repo_a.root).await;

    let (outcome, phases) = repo_a.run().await;
    let ExplicitOutcome::Failed { error, .. } = outcome else {
        panic!("expected failure, got {outcome:?}");
    };
    assert!(matches!(
        unavailable_cause(&error),
        ExistingRepositoryDbError::Missing { .. }
    ));
    assert!(phases.is_empty());
    assert_eq!(repo_a.pins().await.len(), 1);

    create_db(&repo_a.db_path(), &repo_b.repository_id).await;
    let (outcome, phases) = repo_a.run().await;
    let ExplicitOutcome::Failed { error, .. } = outcome else {
        panic!("expected failure, got {outcome:?}");
    };
    let ExistingRepositoryDbError::RepositoryMismatch { stored, resolved } =
        unavailable_cause(&error)
    else {
        panic!("expected repository mismatch");
    };
    assert_eq!(stored, &repo_b.repository_id);
    assert_eq!(resolved, &repo_a.repository_id);
    assert!(phases.is_empty());
    assert_eq!(repo_a.pins().await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_missing_metadata_or_schema_fails_before_pin_inventory() {
    let fixture = Fixture::new("https://example.invalid/org/repo-a.git");
    pin_orphan(&fixture.root).await;

    RepositoryAgentTraceDb::new_at(fixture.db_path())
        .await
        .expect("migrated db without metadata");
    let (outcome, phases) = fixture.run().await;
    let ExplicitOutcome::Failed { error, .. } = outcome else {
        panic!("expected failure, got {outcome:?}");
    };
    assert!(matches!(
        unavailable_cause(&error),
        ExistingRepositoryDbError::MissingMetadata
    ));
    assert!(phases.is_empty());
    assert_eq!(fixture.pins().await.len(), 1);

    std::fs::remove_file(fixture.db_path()).expect("remove db");
    drop(
        RepositoryAgentTraceDb::open_without_migrations_at(fixture.db_path())
            .await
            .expect("unmigrated db"),
    );
    let (outcome, phases) = fixture.run().await;
    let ExplicitOutcome::Failed { error, .. } = outcome else {
        panic!("expected failure, got {outcome:?}");
    };
    assert!(matches!(
        unavailable_cause(&error),
        ExistingRepositoryDbError::IncompatibleSchema(_)
    ));
    assert!(phases.is_empty());
    assert_eq!(fixture.pins().await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_missing_database_creates_nothing_and_deletes_nothing() {
    let fixture = Fixture::new("https://example.invalid/org/repo-a.git");
    pin_orphan(&fixture.root).await;

    let (outcome, phases) = fixture.run().await;

    let ExplicitOutcome::Failed { error, .. } = outcome else {
        panic!("expected failure, got {outcome:?}");
    };
    assert!(matches!(
        unavailable_cause(&error),
        ExistingRepositoryDbError::Missing { .. }
    ));
    assert!(phases.is_empty());
    assert!(!fixture.state_root.exists());
    assert_eq!(fixture.pins().await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_first_failed_pass_without_anchor_is_recommended_and_advised_once() {
    use super::super::ref_advisory::{advise_if_due_with, AdvisoryOutcome};

    let fixture = Fixture::new("https://example.invalid/org/repo-a.git");
    pin_orphan(&fixture.root).await;

    let (outcome, _) = fixture.run().await;
    assert!(matches!(outcome, ExplicitOutcome::Failed { .. }));
    let StateRead::Valid(state) = fixture.stored_state().await else {
        panic!("expected recorded state");
    };
    assert_eq!(state.anchor, None);
    assert_eq!(state.last_success, None);
    let now = state.last_attempt.expect("attempt time");
    assert!(evaluate_recommendation(&state, now).recommended);

    let first = advise_if_due_with(&fixture.root, || now, write_state_atomically);
    assert!(matches!(first, AdvisoryOutcome::Advised));
    let second = advise_if_due_with(&fixture.root, || now + 1000, write_state_atomically);
    assert!(matches!(second, AdvisoryOutcome::NoAction));

    let StateRead::Valid(after) = fixture.stored_state().await else {
        panic!("expected recorded state");
    };
    assert_eq!(after.consecutive_failures, 1);
    assert_eq!(after.last_failure, state.last_failure);
    assert!(evaluate_recommendation(&after, now + 1000).recommended);
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_failure_streak_counts_but_lock_contention_never_does() {
    let fixture = Fixture::new("https://example.invalid/org/repo-a.git");
    pin_orphan(&fixture.root).await;

    for expected in 1..=2 {
        let (outcome, _) = fixture.run().await;
        assert!(matches!(
            outcome,
            ExplicitOutcome::Failed {
                state_warning: None,
                ..
            }
        ));
        let StateRead::Valid(state) = fixture.stored_state().await else {
            panic!("expected recorded state");
        };
        assert_eq!(state.consecutive_failures, expected);
        assert_eq!(state.last_success, None);
    }

    let git_dir = resolve_git_dir(&fixture.root).await.expect("git dir");
    let before = std::fs::read(state_path(&git_dir)).expect("state bytes");
    let held = acquire_inner_async(&git_dir, Duration::from_secs(10), || {})
        .await
        .expect("hold lock");
    let mut phases = Vec::new();
    let outcome = reconcile_explicit_with(
        &fixture.root,
        async || -> anyhow::Result<RepositoryAgentTraceDb> { unreachable!("db must not open") },
        || NOW,
        |_, _| unreachable!("state must not be written on contention"),
        |phase| phases.push(phase),
        Duration::ZERO,
    )
    .await;
    drop(held);

    assert!(matches!(
        outcome,
        ExplicitOutcome::Skipped(SkipReason::Busy)
    ));
    assert!(phases.is_empty());
    assert_eq!(
        std::fs::read(state_path(&git_dir)).expect("state bytes"),
        before
    );
    let StateRead::Valid(state) = fixture.stored_state().await else {
        panic!("expected recorded state");
    };
    assert_eq!(state.consecutive_failures, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_missing_database_and_failed_persistence_keep_both_errors() {
    let fixture = Fixture::new("https://example.invalid/org/repo-a.git");
    let tree = pin_orphan(&fixture.root).await;

    let (outcome, _) = fixture.run_with(failing_write(PersistPhase::Rename)).await;

    let ExplicitOutcome::Failed {
        error,
        state_warning,
    } = outcome
    else {
        panic!("expected failure, got {outcome:?}");
    };
    assert!(matches!(
        unavailable_cause(&error),
        ExistingRepositoryDbError::Missing { .. }
    ));
    let Some(StatePersistWarning::WriteFailed(PersistFailure::NotApplied { phase, .. })) =
        state_warning
    else {
        panic!("expected retained persistence warning, got {state_warning:?}");
    };
    assert_eq!(phase, PersistPhase::Rename);
    let pins = fixture.pins().await;
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0].tree.0, tree);
    let git_dir = resolve_git_dir(&fixture.root).await.expect("git dir");
    assert_eq!(
        staging_leftovers(state_path(&git_dir).parent().expect("parent")),
        0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_missing_required_pin_and_failed_persistence_keep_both_errors() {
    let fixture = Fixture::new("https://example.invalid/org/repo-a.git");
    fixture.create_db().await;
    let snapshot = GitSnapshotService::new(&fixture.root)
        .await
        .expect("snapshot");
    let unpinned = snapshot.capture_tree().await.expect("tree");
    seed_durable_root(&fixture.db_path(), &unpinned.0).await;

    let (outcome, phases) = fixture
        .run_with(failing_write(PersistPhase::WriteStaging))
        .await;

    let ExplicitOutcome::Failed {
        error,
        state_warning,
    } = outcome
    else {
        panic!("expected failure, got {outcome:?}");
    };
    let ReconcileError::MissingRequiredPins { missing } = &error else {
        panic!("expected missing required pins, got {error:?}");
    };
    assert_eq!(missing, &vec![unpinned]);
    assert!(matches!(
        state_warning,
        Some(StatePersistWarning::WriteFailed(
            PersistFailure::NotApplied {
                phase: PersistPhase::WriteStaging,
                ..
            }
        ))
    ));
    assert_eq!(
        phases,
        vec![ReconcilePhase::DbOpened, ReconcilePhase::PinsInventoried]
    );
    assert!(fixture.pins().await.is_empty());
    assert!(matches!(fixture.stored_state().await, StateRead::Absent));
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_completed_pass_with_unconfirmed_durability_keeps_deletion() {
    let fixture = Fixture::new("https://example.invalid/org/repo-a.git");
    fixture.create_db().await;
    pin_orphan(&fixture.root).await;

    let (outcome, _) = fixture
        .run_with(failing_write(PersistPhase::SyncParentDirectory))
        .await;

    let ExplicitOutcome::CompletedStatePersistFailed { report, warning } = outcome else {
        panic!("expected persist-failed completion, got {outcome:?}");
    };
    assert_eq!(report.deleted, 1);
    assert!(matches!(
        warning,
        StatePersistWarning::WriteFailed(PersistFailure::DurabilityUncertain {
            phase: PersistPhase::SyncParentDirectory,
            ..
        })
    ));
    assert!(fixture.pins().await.is_empty());
    let StateRead::Valid(state) = fixture.stored_state().await else {
        panic!("new state is visible even though durability is unconfirmed");
    };
    assert_eq!(state.last_success, Some(NOW));
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_completed_pass_with_unwritten_state_reports_distinct_outcome() {
    let fixture = Fixture::new("https://example.invalid/org/repo-a.git");
    fixture.create_db().await;
    pin_orphan(&fixture.root).await;

    let (outcome, _) = fixture
        .run_with(failing_write(PersistPhase::CreateStaging))
        .await;

    let ExplicitOutcome::CompletedStatePersistFailed { report, warning } = outcome else {
        panic!("expected persist-failed completion, got {outcome:?}");
    };
    assert_eq!(report.deleted, 1);
    assert!(warning.to_string().contains("previous state preserved"));
    assert!(fixture.pins().await.is_empty());
    assert!(matches!(fixture.stored_state().await, StateRead::Absent));
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_unreadable_previous_state_is_reported_without_overwriting() {
    let fixture = Fixture::new("https://example.invalid/org/repo-a.git");
    pin_orphan(&fixture.root).await;
    let git_dir = resolve_git_dir(&fixture.root).await.expect("git dir");
    let path = state_path(&git_dir);
    std::fs::create_dir_all(&path).expect("directory in place of state file");

    let pins_before = fixture.pins().await;
    assert_eq!(pins_before.len(), 1);

    let (outcome, _) = fixture.run().await;

    let ExplicitOutcome::Failed {
        error,
        state_warning,
    } = outcome
    else {
        panic!("expected failure, got {outcome:?}");
    };
    assert!(matches!(
        unavailable_cause(&error),
        ExistingRepositoryDbError::Missing { .. }
    ));
    assert!(matches!(
        state_warning,
        Some(StatePersistWarning::PreviousStateUnreadable(_))
    ));
    assert!(path.is_dir());
    assert_eq!(std::fs::read_dir(&path).expect("read state dir").count(), 0);
    assert_eq!(fixture.pins().await, pins_before);
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_lock_held_implementation_never_reacquires_the_lock() {
    let fixture = Fixture::new("https://example.invalid/org/repo-a.git");
    fixture.create_db().await;
    pin_orphan(&fixture.root).await;
    let git_dir = resolve_git_dir(&fixture.root).await.expect("git dir");
    let lock = acquire_inner_async(&git_dir, Duration::from_secs(10), || {})
        .await
        .expect("lock");

    let outcome = tokio::time::timeout(
        Duration::from_secs(30),
        reconcile_with_held_lock(
            &fixture.root,
            &lock,
            async || open_authoritative_db_at_state_root(&fixture.root, &fixture.state_root).await,
            |_| {},
        ),
    )
    .await
    .expect("must not block on a second acquisition")
    .expect("reconcile");
    drop(lock);

    let ReconciliationOutcome::Reconciled(report) = outcome;
    assert_eq!(report.deleted, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_verified_open_never_creates_missing_database_or_directories() {
    let dir = tempfile::tempdir().expect("temp dir");
    let missing = dir.path().join("nested").join("agent-trace.db");

    let error = RepositoryAgentTraceDb::open_verified_existing_at(&missing, "repo")
        .await
        .err()
        .expect("missing db fails");

    assert!(matches!(error, ExistingRepositoryDbError::Missing { .. }));
    assert!(!dir.path().join("nested").exists());

    let missing_in_existing_dir = dir.path().join("agent-trace.db");
    let error = RepositoryAgentTraceDb::open_verified_existing_at(&missing_in_existing_dir, "repo")
        .await
        .err()
        .expect("missing db fails");
    assert!(matches!(error, ExistingRepositoryDbError::Missing { .. }));
    assert_eq!(std::fs::read_dir(dir.path()).expect("read dir").count(), 0);
}

const DATABASE_SIDECAR_SUFFIXES: [&str; 3] = ["-wal", "-tshm", "-shm"];

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn remove_database(path: &Path) {
    std::fs::remove_file(path).ok();
    for suffix in DATABASE_SIDECAR_SUFFIXES {
        std::fs::remove_file(with_suffix(path, suffix)).ok();
    }
}

fn swap_in_database(source: &Path, target: &Path) {
    remove_database(target);
    for suffix in DATABASE_SIDECAR_SUFFIXES {
        let sidecar = with_suffix(source, suffix);
        if sidecar.exists() {
            std::fs::rename(sidecar, with_suffix(target, suffix)).expect("move sidecar");
        }
    }
    std::fs::rename(source, target).expect("move database");
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_verified_open_rejects_database_removed_before_open() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("agent-trace.db");
    create_db(&path, "repo").await;
    let existed_when_hook_ran = std::cell::Cell::new(false);

    let error = RepositoryAgentTraceDb::open_verified_existing_at_with_hooks(
        &path,
        "repo",
        || {
            existed_when_hook_ran.set(path.is_file());
            remove_database(&path);
        },
        || {},
    )
    .await
    .err()
    .expect("removed db fails");

    assert!(existed_when_hook_ran.get());
    assert!(matches!(error, ExistingRepositoryDbError::Missing { .. }));
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(dir.path()).expect("read dir").count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_verified_open_rejects_database_replaced_before_open() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("agent-trace.db");
    create_db(&path, "repo").await;
    let foreign = dir.path().join("foreign.db");
    create_db(&foreign, "someone-else").await;

    let error = RepositoryAgentTraceDb::open_verified_existing_at_with_hooks(
        &path,
        "repo",
        || swap_in_database(&foreign, &path),
        || {},
    )
    .await
    .err()
    .expect("replacement with another repository fails");
    assert!(matches!(
        error,
        ExistingRepositoryDbError::RepositoryMismatch { .. }
    ));

    let empty = dir.path().join("empty.db");
    drop(
        RepositoryAgentTraceDb::open_without_migrations_at(&empty)
            .await
            .expect("empty db"),
    );
    let error = RepositoryAgentTraceDb::open_verified_existing_at_with_hooks(
        &path,
        "repo",
        || swap_in_database(&empty, &path),
        || {},
    )
    .await
    .err()
    .expect("replacement without schema fails");
    assert!(matches!(
        error,
        ExistingRepositoryDbError::IncompatibleSchema(_)
    ));

    let unusable = dir.path().join("unusable.db");
    std::fs::write(&unusable, b"this is not a database").expect("garbage");
    let error = RepositoryAgentTraceDb::open_verified_existing_at_with_hooks(
        &path,
        "repo",
        || swap_in_database(&unusable, &path),
        || {},
    )
    .await
    .err()
    .expect("replacement with garbage fails");
    assert!(matches!(
        error,
        ExistingRepositoryDbError::IncompatibleSchema(_) | ExistingRepositoryDbError::Unreadable(_)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_verified_open_accepts_a_replacement_only_if_it_verifies_itself() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("agent-trace.db");
    create_db(&path, "repo").await;
    let replacement = dir.path().join("replacement.db");
    create_db(&replacement, "repo").await;

    let (_, metadata) = RepositoryAgentTraceDb::open_verified_existing_at_with_hooks(
        &path,
        "repo",
        || swap_in_database(&replacement, &path),
        || {},
    )
    .await
    .expect("a replacement that passes every check is itself authoritative");
    assert_eq!(metadata.repository_id, "repo");
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_verified_open_never_recreates_a_database_removed_after_open() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("agent-trace.db");
    create_db(&path, "repo").await;

    let (db, metadata) = RepositoryAgentTraceDb::open_verified_existing_at_with_hooks(
        &path,
        "repo",
        || {},
        || remove_database(&path),
    )
    .await
    .expect("the opened handle is what gets verified");
    assert_eq!(metadata.repository_id, "repo");
    drop(db);

    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(dir.path()).expect("read dir").count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_verified_open_is_read_only() {
    const INSERT_WORKTREE: &str = "INSERT INTO mutation_trace_worktrees
            (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline)
         VALUES ('main', 'tree', x'0000000000000000', 0, 'healthy', 0)";
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("agent-trace.db");
    create_db(&path, "repo").await;

    let writer = RepositoryAgentTraceDb::new_at(&path).await.expect("writer");
    writer.execute("BEGIN", ()).await.expect("begin");
    writer
        .execute(INSERT_WORKTREE, ())
        .await
        .expect("the statement is valid on a writable connection");
    writer.execute("ROLLBACK", ()).await.expect("rollback");
    drop(writer);

    let (db, _) = RepositoryAgentTraceDb::open_verified_existing_at(&path, "repo")
        .await
        .expect("verified open");
    let error = db
        .execute(INSERT_WORKTREE, ())
        .await
        .expect_err("verified connection must reject writes");
    let rendered = format!("{error:#}");
    assert!(rendered.contains("database is readonly"), "{rendered}");
    drop(db);

    let inspector = RepositoryAgentTraceDb::new_at(&path)
        .await
        .expect("inspector");
    let rows = inspector
        .query_map(
            "SELECT worktree_id FROM mutation_trace_worktrees",
            (),
            |row| row.get::<String>(0).map_err(Into::into),
        )
        .await
        .expect("rows");
    assert!(rows.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_verified_open_coexists_with_a_live_multiprocess_wal_writer() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("agent-trace.db");
    let writer = RepositoryAgentTraceDb::new_at(&path).await.expect("writer");
    writer
        .verify_or_initialize_repository_metadata("repo")
        .await
        .expect("metadata");

    let (reader, metadata) = RepositoryAgentTraceDb::open_verified_existing_at(&path, "repo")
        .await
        .expect("verified open alongside live writer");
    assert_eq!(metadata.repository_id, "repo");

    writer
        .execute(
            "INSERT INTO mutation_trace_worktrees
                (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline)
             VALUES ('main', 'tree-after-open', x'0000000000000000', 0, 'healthy', 0)",
            (),
        )
        .await
        .expect("writer still writes");
    let rows = reader
        .query_map(
            "SELECT cursor_tree FROM mutation_trace_worktrees",
            (),
            |row| row.get::<String>(0).map_err(Into::into),
        )
        .await
        .expect("reader sees committed writer data");
    assert_eq!(rows, vec!["tree-after-open".to_string()]);
    assert!(with_suffix(&path, "-tshm").exists());
}

async fn pin_distinct(root: &Path, content: &str) -> String {
    std::fs::write(root.join("distinct.txt"), content).expect("write distinct file");
    pin_orphan(root).await
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_retains_every_repository_wide_durable_root_and_never_repairs_other_worktrees(
) {
    let fixture = Fixture::new("https://example.invalid/org/repo-a.git");
    fixture.create_db().await;
    let local = pin_distinct(&fixture.root, "local\n").await;
    let other = pin_distinct(&fixture.root, "other\n").await;
    let orphan_one = pin_distinct(&fixture.root, "orphan-one\n").await;
    let orphan_two = pin_distinct(&fixture.root, "orphan-two\n").await;
    let unpinned_elsewhere = "e".repeat(40);
    seed_durable_root(&fixture.db_path(), &local).await;
    seed_durable_root_for(&fixture.db_path(), "worktrees/other", &other).await;
    seed_durable_root_for(&fixture.db_path(), "worktrees/ghost", &unpinned_elsewhere).await;

    let (outcome, _) = fixture.run().await;

    let ExplicitOutcome::Completed(report) = outcome else {
        panic!("expected completed pass, got {outcome:?}");
    };
    assert_eq!(
        report,
        ReconciliationReport {
            local_required: 1,
            retained: 2,
            deleted: 2,
        }
    );
    let mut remaining: Vec<String> = fixture
        .pins()
        .await
        .into_iter()
        .map(|pin| pin.tree.0)
        .collect();
    remaining.sort();
    let mut expected = vec![local, other];
    expected.sort();
    assert_eq!(remaining, expected);
    assert!(!remaining.contains(&orphan_one) && !remaining.contains(&orphan_two));
    assert!(!remaining.contains(&unpinned_elsewhere));

    let (second, _) = fixture.run().await;
    let ExplicitOutcome::Completed(second) = second else {
        panic!("expected idempotent completed pass");
    };
    assert_eq!((second.retained, second.deleted), (2, 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_malformed_namespace_ref_deletes_nothing_and_records_failure() {
    let fixture = Fixture::new("https://example.invalid/org/repo-a.git");
    fixture.create_db().await;
    pin_orphan(&fixture.root).await;
    git(
        &fixture.root,
        &[
            "symbolic-ref",
            "refs/sce/mutation-cursor/main/not-a-tree",
            "HEAD",
        ],
    );
    let inventory_before = ref_inventory(&fixture.root);
    assert_eq!(inventory_before.len(), 2);

    let (outcome, _) = fixture.run().await;

    let ExplicitOutcome::Failed { error, .. } = outcome else {
        panic!("expected failure, got {outcome:?}");
    };
    let ReconcileError::MalformedPin { ref_name, .. } = &error else {
        panic!("expected MalformedPin, got {error:?}");
    };
    assert_eq!(ref_name, "refs/sce/mutation-cursor/main/not-a-tree");
    assert_eq!(
        ref_inventory(&fixture.root),
        inventory_before,
        "no ref may be deleted, replaced or retargeted"
    );
    let StateRead::Valid(state) = fixture.stored_state().await else {
        panic!("expected recorded state");
    };
    assert_eq!(state.consecutive_failures, 1);
}
