use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{mpsc, Arc, Barrier};
use std::thread;
use std::time::Duration;

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_storage::{
    resolve_agent_trace_storage_at_state_root, AgentTraceStorageContext,
};
use crate::services::checkout::{read_checkout_id, resolve_git_dir};
use crate::services::mutation_trace::store::{encode_revision, MutationTraceStore};
use crate::services::mutation_trace::types::{
    ActorKind, EventId, FailureKind, ScopeId, ScopeStatus,
};

use super::coordinator::{coordinate, coordinate_inner, CoordinateError, RuntimeBoundary};
use super::external_taint::ExternalTaintMarker;
use super::git_snapshot::GitSnapshotService;
use super::ref_reconciliation::{
    reconcile_worktree, reconcile_worktree_inner, ReconcileError, ReconciliationOutcome,
};
use super::worktree_lock::{acquire_inner, WorktreeLock};

fn run_git(dir: &Path, args: &[&str]) -> String {
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

fn init_repo(root: &Path) {
    std::fs::create_dir_all(root).expect("repository directory should be created");
    run_git(root, &["init", "--quiet"]);
    run_git(root, &["config", "user.email", "test@example.com"]);
    run_git(root, &["config", "user.name", "Test"]);
    run_git(root, &["commit", "--allow-empty", "--quiet", "-m", "init"]);
}

struct TestRepo {
    _temp_dir: tempfile::TempDir,
    repo_root: PathBuf,
    db_path: PathBuf,
}

impl TestRepo {
    fn new(label: &str) -> Self {
        let temp_dir = tempfile::Builder::new()
            .prefix(&format!("sce-mutation-trace-runtime-{label}-"))
            .tempdir()
            .expect("test temp directory should be created");
        let repo_root = temp_dir.path().join("repo");
        init_repo(&repo_root);
        let db_path = temp_dir.path().join("agent-trace.db");
        RepositoryAgentTraceDb::new_at(&db_path)
            .expect("the repository DB should open with schema");
        Self {
            _temp_dir: temp_dir,
            repo_root,
            db_path,
        }
    }

    fn open_db(&self) -> anyhow::Result<RepositoryAgentTraceDb> {
        RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&self.db_path)
    }

    fn db(&self) -> RepositoryAgentTraceDb {
        self.open_db()
            .expect("reopening the DB for assertions should succeed")
    }
}

struct LinkedTestRepo {
    _temp_dir: tempfile::TempDir,
    main_root: PathBuf,
    linked_root: PathBuf,
    db_path: PathBuf,
}

impl LinkedTestRepo {
    fn new(label: &str) -> Self {
        let temp_dir = tempfile::Builder::new()
            .prefix(&format!("sce-mutation-trace-runtime-{label}-"))
            .tempdir()
            .expect("test temp directory should be created");
        let main_root = temp_dir.path().join("main");
        init_repo(&main_root);
        let linked_root = temp_dir.path().join("linked");
        run_git(
            &main_root,
            &[
                "worktree",
                "add",
                "--quiet",
                linked_root.to_str().expect("worktree path should be UTF-8"),
            ],
        );
        let db_path = temp_dir.path().join("agent-trace.db");
        RepositoryAgentTraceDb::new_at(&db_path)
            .expect("the shared repository DB should open with schema");
        Self {
            _temp_dir: temp_dir,
            main_root,
            linked_root,
            db_path,
        }
    }

    fn open_db(&self) -> anyhow::Result<RepositoryAgentTraceDb> {
        RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&self.db_path)
    }

    fn db(&self) -> RepositoryAgentTraceDb {
        self.open_db()
            .expect("reopening the shared DB for assertions should succeed")
    }
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

fn ref_exists(dir: &Path, ref_name: &str) -> bool {
    Command::new("git")
        .args(["show-ref", "--verify", "--quiet", ref_name])
        .current_dir(dir)
        .status()
        .expect("git show-ref should spawn")
        .success()
}

fn row_count(db: &RepositoryAgentTraceDb, table: &str) -> i64 {
    db.query_map(&format!("SELECT COUNT(*) FROM {table}"), (), |row| {
        row.get::<i64>(0).map_err(Into::into)
    })
    .expect("count query should succeed")
    .into_iter()
    .next()
    .expect("count row should exist")
}

#[test]
fn linked_worktrees_have_independent_locks_and_worktree_ids() {
    let repo = LinkedTestRepo::new("linked-ids");

    let main_git_dir = resolve_git_dir(&repo.main_root).expect("main git dir should resolve");
    let linked_git_dir = resolve_git_dir(&repo.linked_root).expect("linked git dir should resolve");
    assert_ne!(
        main_git_dir, linked_git_dir,
        "a linked worktree must resolve its own worktree-specific git dir, giving it a distinct lock and identity path"
    );

    let main_outcome = coordinate(&repo.main_root, &RuntimeBoundary::Flush, || repo.open_db())
        .expect("first observation on the main worktree should succeed");

    let held = WorktreeLock::acquire(&main_git_dir, Duration::from_secs(5))
        .expect("the main worktree's runtime lock should be acquirable");

    let linked_outcome = coordinate(&repo.linked_root, &RuntimeBoundary::Flush, || {
        repo.open_db()
    })
    .expect(
        "coordinate() on the linked worktree must acquire its own distinct runtime lock while the main worktree's lock is still held",
    );

    drop(held);

    assert_ne!(
        main_outcome.worktree_id, linked_outcome.worktree_id,
        "each linked worktree must derive a distinct WorktreeId from its own checkout identity"
    );

    let db_main = repo.db();
    let store = MutationTraceStore::new(&db_main);
    assert!(
        store
            .load_worktree(&main_outcome.worktree_id, None, None)
            .expect("loading the main worktree row should succeed")
            .is_some(),
        "the main worktree's coordinator should have persisted a row into the caller-supplied repository-scoped DB"
    );
    assert!(
        store
            .load_worktree(&linked_outcome.worktree_id, None, None)
            .expect("loading the linked worktree row should succeed")
            .is_some(),
        "the linked worktree's coordinator should have persisted its distinct row into the same caller-supplied DB"
    );

    let linked_snapshot = GitSnapshotService::new(&repo.linked_root)
        .expect("a snapshot service should construct for the linked worktree");
    linked_snapshot
        .diff_trees(&main_outcome.observed_tree, &main_outcome.observed_tree)
        .expect("a tree pinned by the main worktree's coordinator must resolve through the linked worktree's git dir");
}

#[test]
fn agent_trace_storage_and_coordinator_observe_the_same_checkout_id() {
    let repo = TestRepo::new("cross-caller");
    run_git(
        &repo.repo_root,
        &["remote", "add", "origin", "git@github.com:acme/widgets.git"],
    );
    let state_dir = tempfile::Builder::new()
        .prefix("sce-mutation-trace-runtime-cross-caller-state-")
        .tempdir()
        .expect("state root temp directory should be created");
    let state_root = state_dir.path().to_path_buf();

    let barrier = Arc::new(Barrier::new(2));

    let storage_thread = {
        let repo_root = repo.repo_root.clone();
        let state_root = state_root.clone();
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let context = AgentTraceStorageContext {
                repository_root: &repo_root,
                explicit_repository_id: None,
                repository_remote: "origin",
            };
            barrier.wait();
            resolve_agent_trace_storage_at_state_root(&context, &state_root)
                .expect("agent_trace_storage resolution should succeed")
                .checkout_id
        })
    };

    barrier.wait();
    let outcome = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, || repo.open_db())
        .expect("the coordinator's first observation should succeed");

    let storage_checkout_id = storage_thread
        .join()
        .expect("the storage thread should not panic");

    assert_eq!(
        outcome.worktree_id.0, storage_checkout_id,
        "the coordinator and agent_trace_storage must converge on one checkout identity for the same physical checkout"
    );

    let on_disk =
        read_checkout_id(&resolve_git_dir(&repo.repo_root).expect("git dir should resolve"))
            .expect("reading the checkout-id file should succeed")
            .expect("a checkout id must have been persisted");
    assert_eq!(
        on_disk, storage_checkout_id,
        "the on-disk checkout-id file must contain the converged identity"
    );
}

#[test]
fn a_snapshot_failure_then_recovery_cycle_runs_through_the_public_api() {
    let repo = TestRepo::new("failure-recovery");
    let ok_db = || repo.open_db();

    let baseline = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();

    let git_dir = resolve_git_dir(&repo.repo_root).expect("git dir should resolve");
    let tmp_index_dir = git_dir.join("sce").join("tmp");
    let _ = std::fs::remove_dir_all(&tmp_index_dir);
    std::fs::write(&tmp_index_dir, b"not a directory").expect(
        "planting a file where the snapshot service expects its temp-index directory should succeed",
    );

    let scope = ScopeId("scope-recovery".to_string());
    let failure = coordinate(
        &repo.repo_root,
        &RuntimeBoundary::Start {
            scope: scope.clone(),
            event: EventId("evt-during-failure".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        ok_db,
    )
    .expect_err("a Git snapshot failure against a materialized worktree should be reported");
    match failure {
        CoordinateError::SnapshotFailure {
            persisted_taint, ..
        } => assert!(
            persisted_taint,
            "the existing worktree row must be durably tainted by the snapshot failure"
        ),
        other => panic!("expected CoordinateError::SnapshotFailure, got {other:?}"),
    }

    {
        let db = repo.db();
        let store = MutationTraceStore::new(&db);
        let projection = store
            .load_worktree(&worktree_id, None, None)
            .expect("loading the worktree row should succeed")
            .expect("the worktree row should still exist");
        assert!(
            projection.worktree_state.tainted,
            "the worktree row should be tainted after the snapshot failure"
        );
    }

    std::fs::remove_file(&tmp_index_dir)
        .expect("removing the planted file should let the snapshot service recreate its temp dir");

    let recovered = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the coordinator should recover from the taint and process the boundary");
    assert_eq!(
        recovered.worktree_id, worktree_id,
        "recovery must operate on the same worktree identity"
    );

    let db = repo.db();
    let store = MutationTraceStore::new(&db);
    let projection = store
        .load_worktree(&worktree_id, None, None)
        .expect("loading the worktree row should succeed")
        .expect("the worktree row should still exist");
    assert!(
        !projection.worktree_state.tainted,
        "taint recovery must clear the tainted flag before the triggering boundary is processed"
    );
}

#[test]
fn a_successful_coordinate_through_the_public_api_leaves_no_external_taint_marker() {
    let repo = TestRepo::new("public-success-no-marker");
    let git_dir = resolve_git_dir(&repo.repo_root).expect("git dir should resolve");
    let marker = ExternalTaintMarker::new(&git_dir);

    let outcome = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, || repo.open_db())
        .expect("a first observation through the public entrypoint should succeed");
    assert_eq!(
        outcome.revision, 0,
        "a first-observation flush should not advance the revision"
    );
    assert!(
        !marker
            .exists()
            .expect("marker existence should resolve after a successful coordinate()"),
        "a successful coordinate() must clear the external-taint marker it armed"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn a_db_open_failure_after_arming_leaves_the_marker_and_the_next_invocation_rebaselines_without_evidence(
) {
    let repo = TestRepo::new("public-db-open-failure-gap");
    let git_dir = resolve_git_dir(&repo.repo_root).expect("git dir should resolve");
    let marker = ExternalTaintMarker::new(&git_dir);
    let ok_db = || repo.open_db();

    let baseline = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();
    let scope = ScopeId("scope-across-the-gap".to_string());
    coordinate(
        &repo.repo_root,
        &RuntimeBoundary::Start {
            scope: scope.clone(),
            event: EventId("evt-start".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        ok_db,
    )
    .expect("starting the scope should succeed");
    std::fs::write(repo.repo_root.join("work.txt"), b"v1").expect("the A -> B edit should write");
    let advanced = coordinate(
        &repo.repo_root,
        &RuntimeBoundary::Advance {
            scope: scope.clone(),
            event: EventId("evt-advance".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        ok_db,
    )
    .expect("the advance should commit exactly one event");
    let tree_b = advanced.observed_tree.clone();
    assert!(
        advanced.mutation_event.is_some(),
        "the exclusive A -> B edit must land one trustworthy event before the gap"
    );

    let db_failure = coordinate(
        &repo.repo_root,
        &RuntimeBoundary::Flush,
        || -> anyhow::Result<RepositoryAgentTraceDb> {
            Err(anyhow::anyhow!("simulated Agent Trace DB open failure"))
        },
    )
    .expect_err("a failing DB provider must fail coordinate()");
    assert!(
        matches!(db_failure, CoordinateError::AgentTraceDbUnavailable(_)),
        "expected AgentTraceDbUnavailable, got {db_failure:?}"
    );
    assert!(
        marker.exists().expect("marker existence should resolve"),
        "the armed marker must survive an invocation whose DB provider returned Err"
    );
    std::fs::write(repo.repo_root.join("work.txt"), b"v2-during-the-gap").expect("the B -> C edit");

    let recovered = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the follow-up invocation with a working provider should recover, then process its boundary");
    let tree_c = recovered.observed_tree.clone();
    assert_ne!(
        tree_c, tree_b,
        "the gap edit must have moved the observed tree"
    );
    assert!(
        recovered.mutation_event.is_none(),
        "no mutation evidence may span the interval the DB-open failure fenced off"
    );
    assert!(
        !marker.exists().expect("marker existence should resolve"),
        "the recovering invocation must clear the marker on success"
    );
    let db = repo.db();
    let store = MutationTraceStore::new(&db);
    let projection = store
        .load_worktree(&worktree_id, Some(&scope), None)
        .expect("loading the worktree row should succeed")
        .expect("the worktree row should exist");
    assert_eq!(
        projection.worktree_state.cursor_tree, tree_c,
        "recovery must rebaseline the cursor to the follow-up invocation's own observed tree"
    );
    assert!(!projection.worktree_state.tainted);
    assert_eq!(
        projection.scopes.get(&scope).map(|s| s.status),
        Some(ScopeStatus::Abandoned),
        "a scope live across the fenced interval must be abandoned afterward"
    );
    for revision in 1..=recovered.revision {
        if let Some(event) = store
            .load_mutation_event(&worktree_id, revision)
            .expect("loading a mutation event should succeed")
        {
            assert_ne!(
                event.after_tree, tree_c,
                "no MutationEvent may treat an interval ending at the post-gap tree as one trustworthy AI-attributable interval"
            );
        }
    }
}

#[test]
fn a_stale_marker_rebaselines_to_the_current_tree_abandons_scopes_then_processes_the_boundary() {
    let repo = TestRepo::new("public-stale-marker-rebaseline");
    let git_dir = resolve_git_dir(&repo.repo_root).expect("git dir should resolve");
    let marker = ExternalTaintMarker::new(&git_dir);
    let ok_db = || repo.open_db();

    let baseline = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();
    let tree_a = baseline.observed_tree.clone();
    let scope = ScopeId("scope-stranded".to_string());
    coordinate(
        &repo.repo_root,
        &RuntimeBoundary::Start {
            scope: scope.clone(),
            event: EventId("evt-start".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        ok_db,
    )
    .expect("starting scope S should succeed");

    marker
        .persist()
        .expect("simulating a crashed invocation that armed but never cleared the marker");

    std::fs::write(
        repo.repo_root.join("stranded.txt"),
        b"edited-while-the-process-was-gone",
    )
    .expect("the A -> C edit should write");

    let recovered = coordinate(
        &repo.repo_root,
        &RuntimeBoundary::Advance {
            scope: scope.clone(),
            event: EventId("evt-advance".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        ok_db,
    )
    .expect("an inherited marker must recover, then process the triggering boundary");
    let tree_c = recovered.observed_tree.clone();
    assert_ne!(
        tree_c, tree_a,
        "the working tree must have moved during the gap"
    );
    assert!(
        recovered.mutation_event.is_none(),
        "no A -> C evidence may be emitted across the fenced interval"
    );
    assert!(
        !marker.exists().expect("marker existence should resolve"),
        "a successful recovery must clear the inherited marker"
    );

    let db = repo.db();
    let store = MutationTraceStore::new(&db);
    let projection = store
        .load_worktree(&worktree_id, Some(&scope), None)
        .expect("loading the worktree row should succeed")
        .expect("the worktree row should exist");
    assert_eq!(
        projection.worktree_state.cursor_tree, tree_c,
        "recovery must rebaseline the cursor to the current tree C"
    );
    assert!(!projection.worktree_state.tainted);
    assert_eq!(
        projection.scopes.get(&scope).map(|s| s.status),
        Some(ScopeStatus::Abandoned),
        "the scope that was live across the gap must be abandoned during recovery"
    );

    let stable = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("a plain flush after recovery should succeed");
    assert_eq!(
        stable.revision, recovered.revision,
        "recovery and the triggering boundary already completed in the inheriting invocation"
    );
    assert!(stable.mutation_event.is_none());
}

#[test]
fn a_first_ever_failed_invocation_that_never_materialized_a_worktree_row_creates_no_evidence() {
    let repo = TestRepo::new("public-first-ever-failure");
    let git_dir = resolve_git_dir(&repo.repo_root).expect("git dir should resolve");
    let marker = ExternalTaintMarker::new(&git_dir);

    std::fs::write(
        repo.repo_root.join("pre-existing.txt"),
        b"content before any observation",
    )
    .expect("a pre-existing edit should write");

    let first = coordinate(
        &repo.repo_root,
        &RuntimeBoundary::Flush,
        || -> anyhow::Result<RepositoryAgentTraceDb> {
            Err(anyhow::anyhow!(
                "simulated Agent Trace DB open failure on the first-ever invocation"
            ))
        },
    )
    .expect_err("the first-ever invocation's DB provider fails");
    assert!(
        matches!(first, CoordinateError::AgentTraceDbUnavailable(_)),
        "expected AgentTraceDbUnavailable, got {first:?}"
    );
    assert!(
        marker.exists().expect("marker existence should resolve"),
        "the first-ever failed invocation still leaves an armed marker"
    );

    std::fs::write(
        repo.repo_root.join("during-the-gap.txt"),
        b"more unknown-interval content",
    )
    .expect("another unobserved edit should write");

    let established = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, || repo.open_db())
        .expect("the first successful invocation establishes the baseline");
    assert!(
        established.mutation_event.is_none(),
        "a worktree with no prior durable row cannot produce evidence for the unknown interval"
    );
    assert!(
        !marker.exists().expect("marker existence should resolve"),
        "the successful baseline must clear the inherited marker"
    );

    let db = repo.db();
    let store = MutationTraceStore::new(&db);
    let projection = store
        .load_worktree(&established.worktree_id, None, None)
        .expect("loading the worktree row should succeed")
        .expect("the worktree row should now exist");
    assert_eq!(
        projection.worktree_state.cursor_tree, established.observed_tree,
        "the baseline is established against the first observed tree"
    );
    assert!(!projection.worktree_state.tainted);
    assert_eq!(projection.worktree_state.failure_kind, FailureKind::Healthy);
    for revision in 0..=established.revision {
        assert!(
            store
                .load_mutation_event(&established.worktree_id, revision)
                .expect("loading a mutation event should succeed")
                .is_none(),
            "no MutationEvent may exist for a worktree whose history began with an unobserved interval"
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn linked_worktrees_keep_independent_external_taint_markers_over_a_shared_db() {
    let repo = LinkedTestRepo::new("public-taint-linked");

    let main_git_dir = resolve_git_dir(&repo.main_root).expect("main git dir should resolve");
    let linked_git_dir = resolve_git_dir(&repo.linked_root).expect("linked git dir should resolve");
    let main_marker = ExternalTaintMarker::new(&main_git_dir);
    let linked_marker = ExternalTaintMarker::new(&linked_git_dir);

    let main_baseline = coordinate(&repo.main_root, &RuntimeBoundary::Flush, || repo.open_db())
        .expect("the main worktree baseline should succeed");
    let linked_baseline = coordinate(&repo.linked_root, &RuntimeBoundary::Flush, || {
        repo.open_db()
    })
    .expect("the linked worktree baseline should succeed");
    assert_ne!(main_baseline.worktree_id, linked_baseline.worktree_id);

    let linked_scope = ScopeId("scope-linked".to_string());
    coordinate(
        &repo.linked_root,
        &RuntimeBoundary::Start {
            scope: linked_scope.clone(),
            event: EventId("evt-linked-start".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        || repo.open_db(),
    )
    .expect("starting the linked worktree's scope should succeed");
    linked_marker
        .persist()
        .expect("arming the linked worktree's marker should succeed");

    std::fs::write(repo.main_root.join("main-work.txt"), b"v1")
        .expect("a main-worktree edit should write");
    coordinate(&repo.main_root, &RuntimeBoundary::Flush, || repo.open_db()).expect(
        "the main worktree flush must succeed without inheriting the linked worktree's marker",
    );
    assert!(
        !main_marker
            .exists()
            .expect("marker existence should resolve"),
        "the main worktree clears its own marker on success"
    );
    assert!(
        linked_marker
            .exists()
            .expect("marker existence should resolve"),
        "the main worktree's invocation must not touch the linked worktree's independent marker"
    );

    let db = repo.db();
    let store = MutationTraceStore::new(&db);
    let linked_mid = store
        .load_worktree(&linked_baseline.worktree_id, Some(&linked_scope), None)
        .expect("loading the linked worktree row should succeed")
        .expect("the linked worktree row should exist");
    assert_eq!(
        linked_mid.scopes.get(&linked_scope).map(|s| s.status),
        Some(ScopeStatus::Active),
        "a marker in the linked worktree must not trigger recovery of the linked worktree from the main worktree's invocation"
    );

    std::fs::write(repo.linked_root.join("linked-work.txt"), b"v1")
        .expect("a linked-worktree edit should write");
    let linked_recovered = coordinate(&repo.linked_root, &RuntimeBoundary::Flush, || {
        repo.open_db()
    })
    .expect("the linked worktree's own invocation recovers from its inherited marker");
    assert!(
        linked_recovered.mutation_event.is_none(),
        "the linked worktree's conservative recovery emits no evidence for its fenced interval"
    );
    assert!(
        !linked_marker
            .exists()
            .expect("marker existence should resolve"),
        "the linked worktree clears its marker after its own successful recovery"
    );

    let linked_after = store
        .load_worktree(&linked_baseline.worktree_id, Some(&linked_scope), None)
        .expect("loading the linked worktree row should succeed")
        .expect("the linked worktree row should exist");
    assert_eq!(
        linked_after.scopes.get(&linked_scope).map(|s| s.status),
        Some(ScopeStatus::Abandoned),
        "the linked worktree's inherited-taint recovery abandons its own live scope"
    );
}

#[test]
fn a_snapshot_failure_arms_the_marker_and_the_next_invocation_recovers_once() {
    let repo = TestRepo::new("public-snapshot-failure-marker-recovery");
    let git_dir = resolve_git_dir(&repo.repo_root).expect("git dir should resolve");
    let marker = ExternalTaintMarker::new(&git_dir);
    let ok_db = || repo.open_db();

    let baseline = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();
    assert!(!marker.exists().expect("marker existence should resolve"));

    let tmp_index_dir = git_dir.join("sce").join("tmp");
    let _ = std::fs::remove_dir_all(&tmp_index_dir);
    std::fs::write(&tmp_index_dir, b"not a directory")
        .expect("planting a file where the snapshot service expects its temp-index directory");

    let scope = ScopeId("scope-during-the-failure".to_string());
    let failure = coordinate(
        &repo.repo_root,
        &RuntimeBoundary::Start {
            scope: scope.clone(),
            event: EventId("evt-during-failure".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        ok_db,
    )
    .expect_err("a Git snapshot failure after marker arming should be reported");
    assert!(
        matches!(
            failure,
            CoordinateError::SnapshotFailure {
                persisted_taint: true,
                ..
            }
        ),
        "expected a SnapshotFailure that durably tainted the worktree, got {failure:?}"
    );
    assert!(
        marker.exists().expect("marker existence should resolve"),
        "a snapshot failure after arming must leave BOTH a durable taint and the external-taint marker"
    );

    {
        let db = repo.db();
        let store = MutationTraceStore::new(&db);
        let projection = store
            .load_worktree(&worktree_id, None, None)
            .expect("loading the worktree row should succeed")
            .expect("the worktree row should still exist");
        assert!(projection.worktree_state.tainted);
        assert_eq!(
            projection.worktree_state.failure_kind,
            FailureKind::SnapshotFailure
        );
    }

    std::fs::remove_file(&tmp_index_dir)
        .expect("removing the planted file should let the snapshot service recreate its temp dir");
    std::fs::write(repo.repo_root.join("work.txt"), b"v1")
        .expect("an edit before recovery should write");

    let recovered = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the coordinator should recover from the taint and process the boundary");
    assert!(
        recovered.mutation_event.is_none(),
        "the conservative recovery emits no evidence for the fenced interval"
    );
    assert!(
        !marker.exists().expect("marker existence should resolve"),
        "a successful recovery clears the marker"
    );

    let db = repo.db();
    let store = MutationTraceStore::new(&db);
    let projection = store
        .load_worktree(&worktree_id, None, None)
        .expect("loading the worktree row should succeed")
        .expect("the worktree row should still exist");
    assert!(
        !projection.worktree_state.tainted,
        "recovery must clear the durable taint"
    );
    assert_eq!(projection.worktree_state.failure_kind, FailureKind::Healthy);
    assert_eq!(
        projection.worktree_state.cursor_tree, recovered.observed_tree,
        "recovery rebaselines the cursor to the recovering invocation's own observed tree"
    );

    let stable = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("a plain flush after recovery should succeed");
    assert_eq!(
        stable.revision, recovered.revision,
        "the single conservative recovery already completed; a follow-up flush is a no-op"
    );
    assert!(stable.mutation_event.is_none());
}

#[test]
#[allow(clippy::too_many_lines)]
fn a_marker_clear_failure_after_a_durable_boundary_keeps_the_marker_for_a_later_recovery() {
    let repo = TestRepo::new("public-marker-clear-failure");
    let git_dir = resolve_git_dir(&repo.repo_root).expect("git dir should resolve");
    let marker = ExternalTaintMarker::new(&git_dir);
    let marker_path = git_dir.join("sce").join("mutation-cursor-tainted");
    let ok_db = || repo.open_db();

    let baseline = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();

    let scope = ScopeId("scope-attributable".to_string());
    coordinate(
        &repo.repo_root,
        &RuntimeBoundary::Start {
            scope: scope.clone(),
            event: EventId("evt-start".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        ok_db,
    )
    .expect("starting the scope should succeed");
    std::fs::write(repo.repo_root.join("work.txt"), b"v1")
        .expect("an exclusive edit before the boundary should write");

    let clear_marker_path = marker_path.clone();
    let clear_db_path = repo.db_path.clone();
    let error = coordinate(
        &repo.repo_root,
        &RuntimeBoundary::Advance {
            scope: scope.clone(),
            event: EventId("evt-advance".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        move || {
            std::fs::remove_file(&clear_marker_path)
                .expect("the armed marker file should be present mid-invocation");
            std::fs::create_dir_all(clear_marker_path.join("nested"))
                .expect("planting a non-empty directory at the marker path should succeed");
            RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&clear_db_path)
        },
    )
    .expect_err("clearing a marker that is now a non-empty directory must fail");

    let committed = match error {
        CoordinateError::MarkerClearAfterCommit { committed, .. } => committed,
        other => panic!("expected MarkerClearAfterCommit, got {other:?}"),
    };
    assert!(
        marker.exists().expect("marker existence should resolve"),
        "the marker stays logically armed after a post-commit clear failure"
    );

    let (durable_revision, durable_cursor) = {
        let db = repo.db();
        let store = MutationTraceStore::new(&db);
        let projection = store
            .load_worktree(&worktree_id, Some(&scope), None)
            .expect("loading the worktree row should succeed")
            .expect("the worktree row should exist");
        let event = store
            .load_mutation_event(&worktree_id, projection.worktree_state.revision)
            .expect("loading the committed mutation event should succeed")
            .expect("the attributable Advance must have committed one durable event");
        assert_eq!(event.after_tree, projection.worktree_state.cursor_tree);
        (
            projection.worktree_state.revision,
            projection.worktree_state.cursor_tree.clone(),
        )
    };

    assert_eq!(committed.worktree_id, worktree_id);
    assert_eq!(committed.revision, durable_revision);
    assert_eq!(committed.observed_tree, durable_cursor);
    assert!(
        committed.mutation_event.is_some(),
        "the committed outcome carried by the error must still expose the MutationEvent"
    );

    std::fs::remove_dir_all(&marker_path).expect("removing the planted directory should succeed");
    marker
        .persist()
        .expect("re-arming a plain marker file should succeed");
    std::fs::write(repo.repo_root.join("work.txt"), b"v2").expect("a later edit should write");

    let recovered = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the later invocation recovers from the still-armed marker");
    assert!(
        recovered.mutation_event.is_none(),
        "the deferred conservative recovery emits no evidence for the interval it could not prove"
    );
    assert!(
        !marker.exists().expect("marker existence should resolve"),
        "the successful later recovery finally clears the marker"
    );
    assert_ne!(
        recovered.observed_tree, durable_cursor,
        "the later recovery rebaselines to the newer tree"
    );
}

#[test]
fn reconciliation_blocks_on_the_worktree_lock_and_retains_a_pin_that_becomes_durable_under_it() {
    let repo = TestRepo::new("reconcile-blocks-on-worktree-lock");
    let ok_db = || repo.open_db();

    let baseline = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();
    let baseline_tree = baseline.observed_tree.clone();

    let git_dir = resolve_git_dir(&repo.repo_root).expect("git dir should resolve");
    let held = acquire_inner(&git_dir, Duration::from_secs(5), || {})
        .expect("the test should hold a real WorktreeLock before the worker runs");

    let (contention_tx, contention_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let repo_root_clone = repo.repo_root.clone();
    let db_path_clone = repo.db_path.clone();
    let worker = thread::spawn(move || {
        let outcome = reconcile_worktree_inner(
            &repo_root_clone,
            || RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path_clone),
            move || {
                contention_tx
                    .send(())
                    .expect("contention signal channel should still be open");
            },
        );
        result_tx
            .send(())
            .expect("result signal channel should still be open");
        outcome
    });

    contention_rx.recv_timeout(Duration::from_secs(5)).expect(
        "reconcile_worktree_inner should reach the WorktreeLock try_lock loop and \
         observe contention while this test still holds the lock",
    );
    assert!(
        result_rx.recv_timeout(Duration::from_millis(300)).is_err(),
        "the reconciliation pass must not complete while the worktree lock is still held"
    );

    let snapshot = GitSnapshotService::new(&repo.repo_root)
        .expect("a snapshot service should construct for the worktree");
    std::fs::write(repo.repo_root.join("under-lock.txt"), b"under the lock")
        .expect("an edit under the lock should write");
    let x = snapshot.capture_tree().expect("capturing X should succeed");
    snapshot
        .pin_tree(&worktree_id, &x)
        .expect("pinning X should succeed");
    seed_event(
        &ok_db().expect("reopening the DB to seed the event should succeed"),
        &worktree_id.0,
        1,
        &baseline_tree.0,
        &x.0,
    );

    drop(held);

    result_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the same reconciliation pass should complete once the lock is released");
    let outcome = worker
        .join()
        .expect("the reconciliation worker thread should not panic")
        .expect(
            "reconciliation should succeed once it can acquire the worktree lock after release",
        );
    let report = match outcome {
        ReconciliationOutcome::Reconciled(report) => report,
        ReconciliationOutcome::SkippedNoCheckoutIdentity => {
            panic!("expected a Reconciled outcome, got SkippedNoCheckoutIdentity")
        }
    };

    assert_eq!(
        report.deleted, 0,
        "reconciliation must retain X: it became a durable root under the very lock it was waiting on"
    );
    assert_eq!(
        report.local_required, 2,
        "the worktree's durable roots are exactly the baseline cursor tree and X"
    );
    run_git(
        &repo.repo_root,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/sce/mutation-cursor/{}/{}", worktree_id.0, x.0),
        ],
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn reconciliation_blocks_until_a_real_coordinate_cas_commits_the_pinned_tree() {
    let repo = TestRepo::new("reconcile-blocks-until-real-cas");
    let ok_db = || repo.open_db();

    let baseline = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();
    let baseline_tree = baseline.observed_tree.clone();

    std::fs::write(
        repo.repo_root.join("under-real-cas.txt"),
        b"observed before the real CAS",
    )
    .expect("an edit before the coordinated boundary should write");

    let (pinned_tx, pinned_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let (coordinate_done_tx, coordinate_done_rx) = mpsc::channel();
    let coord_repo_root = repo.repo_root.clone();
    let coord_db_path = repo.db_path.clone();
    let coordinator = thread::spawn(move || {
        let mut paused = false;
        let outcome = coordinate_inner(
            &coord_repo_root,
            &RuntimeBoundary::Flush,
            || RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&coord_db_path),
            || {},
            move |_attempt| {
                if !paused {
                    paused = true;
                    pinned_tx
                        .send(())
                        .expect("the pin-done signal channel should still be open");
                    release_rx
                        .recv()
                        .expect("the release channel should deliver before the real CAS");
                }
            },
            |_attempt| Ok(()),
        );
        coordinate_done_tx
            .send(())
            .expect("the coordinate-done signal channel should still be open");
        outcome
    });

    pinned_rx.recv_timeout(Duration::from_secs(5)).expect(
        "coordinate_inner should pin X, load the worktree, and pause in after_load \
         while still holding the WorktreeLock",
    );

    let (contention_tx, contention_rx) = mpsc::channel();
    let (reconcile_done_tx, reconcile_done_rx) = mpsc::channel();
    let rec_repo_root = repo.repo_root.clone();
    let rec_db_path = repo.db_path.clone();
    let reconciler = thread::spawn(move || {
        let outcome = reconcile_worktree_inner(
            &rec_repo_root,
            || RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&rec_db_path),
            move || {
                contention_tx
                    .send(())
                    .expect("the contention signal channel should still be open");
            },
        );
        reconcile_done_tx
            .send(())
            .expect("the reconcile-done signal channel should still be open");
        outcome
    });

    contention_rx.recv_timeout(Duration::from_secs(5)).expect(
        "reconcile_worktree_inner should observe WorktreeLock contention while the \
         real coordinate() CAS is still pending",
    );
    assert!(
        reconcile_done_rx
            .recv_timeout(Duration::from_millis(300))
            .is_err(),
        "reconciliation must not complete while coordinate() still holds the lock across pin -> CAS"
    );

    release_tx
        .send(())
        .expect("releasing the coordinate worker should succeed");

    coordinate_done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the real coordinate() invocation should complete after the CAS");
    let coordinate_outcome = coordinator
        .join()
        .expect("the coordinate worker thread should not panic")
        .expect("the real coordinate() CAS should apply the observed drift");
    let x = coordinate_outcome.observed_tree.clone();
    assert_ne!(
        x, baseline_tree,
        "the coordinated Flush must observe a real drift from the baseline tree"
    );
    assert!(
        coordinate_outcome.mutation_event.is_some(),
        "the observed drift must commit exactly one durable MutationEvent through the real CAS"
    );

    reconcile_done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("reconciliation should complete once the coordinator releases the lock");
    let outcome = reconciler
        .join()
        .expect("the reconciliation worker thread should not panic")
        .expect(
            "reconciliation should succeed once it can acquire the worktree lock after release",
        );
    let report = match outcome {
        ReconciliationOutcome::Reconciled(report) => report,
        ReconciliationOutcome::SkippedNoCheckoutIdentity => {
            panic!("expected a Reconciled outcome, got SkippedNoCheckoutIdentity")
        }
    };
    assert_eq!(
        report.deleted, 0,
        "reconciliation must retain X: it became a durable root through the real coordinate() CAS \
         under the very lock reconciliation was waiting on"
    );
    assert_eq!(
        report.local_required, 2,
        "the worktree's durable roots are exactly the baseline cursor tree and X"
    );

    run_git(
        &repo.repo_root,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/sce/mutation-cursor/{}/{}", worktree_id.0, x.0),
        ],
    );

    let db = ok_db().expect("reopening the DB for assertions should succeed");
    let store = MutationTraceStore::new(&db);
    let projection = store
        .load_worktree(&worktree_id, None, None)
        .expect("loading the worktree row should succeed")
        .expect("the worktree row should exist");
    assert_eq!(
        projection.worktree_state.cursor_tree, x,
        "the real coordinator CAS advanced the durable cursor to X"
    );
    let event = store
        .load_mutation_event(&worktree_id, projection.worktree_state.revision)
        .expect("loading the committed mutation event should succeed")
        .expect("the observed drift must have committed one durable event");
    assert_eq!(event.before_tree, baseline_tree);
    assert_eq!(event.after_tree, x);
}

#[test]
fn a_pin_with_no_durable_root_is_reclaimed_by_a_later_reconciliation() {
    let repo = TestRepo::new("orphan-reclaimed-via-public-api");
    let ok_db = || repo.open_db();

    let baseline = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();

    let snapshot = GitSnapshotService::new(&repo.repo_root)
        .expect("a snapshot service should construct for the worktree");
    std::fs::write(
        repo.repo_root.join("orphan.txt"),
        b"never durably referenced",
    )
    .expect("the orphan-producing edit should write");
    let x = snapshot.capture_tree().expect("capturing X should succeed");
    snapshot
        .pin_tree(&worktree_id, &x)
        .expect("pinning X should succeed");
    let x_ref = format!("refs/sce/mutation-cursor/{}/{}", worktree_id.0, x.0);
    assert!(
        ref_exists(&repo.repo_root, &x_ref),
        "the orphan pin must exist before reconciliation runs"
    );

    let outcome = reconcile_worktree(&repo.repo_root, ok_db)
        .expect("reconciliation should succeed through the public entrypoint");
    let report = match outcome {
        ReconciliationOutcome::Reconciled(report) => report,
        ReconciliationOutcome::SkippedNoCheckoutIdentity => {
            panic!("expected a Reconciled outcome, got SkippedNoCheckoutIdentity")
        }
    };

    assert_eq!(
        report.deleted, 1,
        "the pass must delete exactly the one pin with no durable root anywhere in the repository"
    );
    assert!(
        !ref_exists(&repo.repo_root, &x_ref),
        "X's snapshot ref must be gone after reconciliation reclaims it"
    );
}

#[test]
fn current_cursor_pin_survives_reconciliation_without_a_referencing_event_through_the_public_api() {
    let repo = TestRepo::new("cursor-retained-no-event");
    let ok_db = || repo.open_db();

    let baseline = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();
    let cursor_ref = format!(
        "refs/sce/mutation-cursor/{}/{}",
        worktree_id.0, baseline.observed_tree.0
    );
    assert!(
        ref_exists(&repo.repo_root, &cursor_ref),
        "the baseline flush must pin the current cursor tree"
    );

    let db = repo.db();
    let store = MutationTraceStore::new(&db);
    let roots = store
        .load_tree_roots(&worktree_id)
        .expect("load_tree_roots should succeed");
    assert_eq!(
        roots.len(),
        1,
        "with no event yet, the cursor tree is the worktree's only durable root"
    );
    assert!(
        roots.contains(&baseline.observed_tree),
        "the current cursor tree must be that sole durable root"
    );

    let outcome = reconcile_worktree(&repo.repo_root, ok_db)
        .expect("reconciliation should succeed through the public entrypoint");
    let report = match outcome {
        ReconciliationOutcome::Reconciled(report) => report,
        ReconciliationOutcome::SkippedNoCheckoutIdentity => {
            panic!("expected a Reconciled outcome, got SkippedNoCheckoutIdentity")
        }
    };

    assert_eq!(
        report.deleted, 0,
        "the current cursor tree is a durable root via cursor_tree alone and must be retained"
    );
    assert!(
        ref_exists(&repo.repo_root, &cursor_ref),
        "the cursor pin must still resolve after reconciliation"
    );
}

#[test]
fn historical_before_and_after_pins_survive_reconciliation_after_real_coordinate_transitions() {
    let repo = TestRepo::new("historical-retention-real-transitions");
    let ok_db = || repo.open_db();

    let baseline = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();
    let tree_a = baseline.observed_tree.clone();

    let scope = ScopeId("scope-historical-abcd".to_string());
    coordinate(
        &repo.repo_root,
        &RuntimeBoundary::Start {
            scope: scope.clone(),
            event: EventId("evt-start".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        ok_db,
    )
    .expect("starting the scope should succeed");

    std::fs::write(repo.repo_root.join("work.txt"), b"v1").expect("the A -> B edit should write");
    let advance_b = coordinate(
        &repo.repo_root,
        &RuntimeBoundary::Advance {
            scope: scope.clone(),
            event: EventId("evt-advance-b".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        ok_db,
    )
    .expect("the A -> B advance should commit one event");
    let tree_b = advance_b.observed_tree.clone();

    std::fs::write(repo.repo_root.join("work.txt"), b"v2").expect("the B -> C edit should write");
    let advance_c = coordinate(
        &repo.repo_root,
        &RuntimeBoundary::Advance {
            scope: scope.clone(),
            event: EventId("evt-advance-c".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        ok_db,
    )
    .expect("the B -> C advance should commit one event");
    let tree_c = advance_c.observed_tree.clone();

    std::fs::write(repo.repo_root.join("work.txt"), b"v3").expect("the C -> D edit should write");
    let advance_d = coordinate(
        &repo.repo_root,
        &RuntimeBoundary::Advance {
            scope,
            event: EventId("evt-advance-d".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        ok_db,
    )
    .expect("the C -> D advance should commit one event");
    let tree_d = advance_d.observed_tree.clone();

    assert_ne!(
        tree_a, tree_b,
        "the A -> B advance must observe a real drift"
    );
    assert_ne!(
        tree_b, tree_c,
        "the B -> C advance must observe a real drift"
    );
    assert_ne!(
        tree_c, tree_d,
        "the C -> D advance must observe a real drift"
    );

    let outcome = reconcile_worktree(&repo.repo_root, ok_db)
        .expect("reconciliation should succeed through the public entrypoint");
    let report = match outcome {
        ReconciliationOutcome::Reconciled(report) => report,
        ReconciliationOutcome::SkippedNoCheckoutIdentity => {
            panic!("expected a Reconciled outcome, got SkippedNoCheckoutIdentity")
        }
    };
    assert_eq!(
        report.deleted, 0,
        "every historical before_tree/after_tree is a durable root: the history A -> B -> C -> D \
         must delete none of {{A, B, C, D}}"
    );
    for tree in [&tree_a, &tree_b, &tree_c, &tree_d] {
        let tree_ref = format!("refs/sce/mutation-cursor/{}/{}", worktree_id.0, tree.0);
        assert!(
            ref_exists(&repo.repo_root, &tree_ref),
            "the historical pin for {tree:?} must survive reconciliation after the cursor advanced past it"
        );
    }
}

#[test]
fn reconciliation_through_the_public_api_is_idempotent() {
    let repo = TestRepo::new("idempotent-via-public-api");
    let ok_db = || repo.open_db();

    let baseline = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();

    let snapshot = GitSnapshotService::new(&repo.repo_root)
        .expect("a snapshot service should construct for the worktree");
    std::fs::write(repo.repo_root.join("orphan-1.txt"), b"orphan one")
        .expect("the first orphan-producing edit should write");
    let x1 = snapshot
        .capture_tree()
        .expect("capturing X1 should succeed");
    snapshot
        .pin_tree(&worktree_id, &x1)
        .expect("pinning X1 should succeed");
    std::fs::write(repo.repo_root.join("orphan-2.txt"), b"orphan two")
        .expect("the second orphan-producing edit should write");
    let x2 = snapshot
        .capture_tree()
        .expect("capturing X2 should succeed");
    snapshot
        .pin_tree(&worktree_id, &x2)
        .expect("pinning X2 should succeed");

    let first =
        match reconcile_worktree(&repo.repo_root, ok_db).expect("the first pass should succeed") {
            ReconciliationOutcome::Reconciled(report) => report,
            ReconciliationOutcome::SkippedNoCheckoutIdentity => {
                panic!("expected a Reconciled outcome, got SkippedNoCheckoutIdentity")
            }
        };
    assert_eq!(
        first.deleted, 2,
        "the first pass must reclaim both orphan pins"
    );

    let second =
        match reconcile_worktree(&repo.repo_root, ok_db).expect("the second pass should succeed") {
            ReconciliationOutcome::Reconciled(report) => report,
            ReconciliationOutcome::SkippedNoCheckoutIdentity => {
                panic!("expected a Reconciled outcome, got SkippedNoCheckoutIdentity")
            }
        };
    assert_eq!(
        second.deleted, 0,
        "the second pass has nothing left to reclaim"
    );
    assert_eq!(
        first.local_required, second.local_required,
        "the local durable-root count must be stable across both passes"
    );
    assert_eq!(
        first.retained, second.retained,
        "the retained-pin count must be identical once the pass has nothing more to reclaim"
    );
}

#[test]
fn reconcile_one_linked_worktree_leaves_the_other_worktrees_pins_and_shared_objects_intact() {
    let repo = LinkedTestRepo::new("linked-isolation");
    let ok_db = || repo.open_db();

    let a_baseline = coordinate(&repo.main_root, &RuntimeBoundary::Flush, ok_db)
        .expect("A's baseline observation should materialize its worktree");
    let a_id = a_baseline.worktree_id.clone();
    let b_baseline = coordinate(&repo.linked_root, &RuntimeBoundary::Flush, ok_db)
        .expect("B's baseline observation should materialize its worktree");
    let b_id = b_baseline.worktree_id.clone();
    assert_ne!(
        a_id, b_id,
        "A and B must derive distinct checkout identities"
    );

    let b_cursor_ref = format!(
        "refs/sce/mutation-cursor/{}/{}",
        b_id.0, b_baseline.observed_tree.0
    );
    assert!(
        ref_exists(&repo.linked_root, &b_cursor_ref),
        "B's own cursor pin must exist before A reconciles"
    );

    let a_snapshot = GitSnapshotService::new(&repo.main_root)
        .expect("a snapshot service should construct for A");
    std::fs::write(repo.main_root.join("a-orphan.txt"), b"A's own orphan")
        .expect("A's orphan-producing edit should write");
    let a_orphan = a_snapshot
        .capture_tree()
        .expect("capturing A's orphan tree should succeed");
    a_snapshot
        .pin_tree(&a_id, &a_orphan)
        .expect("pinning A's orphan should succeed");
    let a_orphan_ref = format!("refs/sce/mutation-cursor/{}/{}", a_id.0, a_orphan.0);

    let outcome = reconcile_worktree(&repo.main_root, ok_db)
        .expect("reconciling A should succeed through the public entrypoint");
    let report = match outcome {
        ReconciliationOutcome::Reconciled(report) => report,
        ReconciliationOutcome::SkippedNoCheckoutIdentity => {
            panic!("expected a Reconciled outcome, got SkippedNoCheckoutIdentity")
        }
    };
    assert_eq!(
        report.deleted, 1,
        "reconcile_worktree(A) must delete only A's own orphan pin"
    );
    assert!(
        !ref_exists(&repo.main_root, &a_orphan_ref),
        "A's orphan pin must be gone"
    );

    assert!(
        ref_exists(&repo.linked_root, &b_cursor_ref),
        "reconcile_worktree(A) must never enumerate or delete a refs/sce/mutation-cursor/<B-id>/ ref"
    );
    let b_snapshot = GitSnapshotService::new(&repo.linked_root)
        .expect("a snapshot service should construct for B");
    b_snapshot
        .diff_trees(&b_baseline.observed_tree, &b_baseline.observed_tree)
        .expect("B's durable tree must still resolve in the shared object database after A's pass");

    let db = repo.db();
    let store = MutationTraceStore::new(&db);
    assert!(
        store
            .load_worktree(&b_id, None, None)
            .expect("loading B's row should succeed")
            .is_some(),
        "reconcile_worktree(A) must not disturb B's durable worktree row"
    );
}

#[test]
fn reconcile_a_retains_its_pin_when_another_worktree_durably_requires_the_same_tree() {
    let repo = LinkedTestRepo::new("cross-worktree-degraded-retention");
    let ok_db = || repo.open_db();

    let a_baseline = coordinate(&repo.main_root, &RuntimeBoundary::Flush, ok_db)
        .expect("A's baseline observation should materialize its worktree");
    let a_id = a_baseline.worktree_id.clone();
    let b_baseline = coordinate(&repo.linked_root, &RuntimeBoundary::Flush, ok_db)
        .expect("B's baseline observation should materialize its worktree");
    let b_id = b_baseline.worktree_id.clone();

    let b_snapshot = GitSnapshotService::new(&repo.linked_root)
        .expect("a snapshot service should construct for B");
    std::fs::write(
        repo.linked_root.join("shared-content.txt"),
        b"identical content",
    )
    .expect("B's edit toward T should write");
    let tree_t = b_snapshot
        .capture_tree()
        .expect("capturing T should succeed");
    let db = repo.db();
    seed_event(&db, &b_id.0, 1, &b_baseline.observed_tree.0, &tree_t.0);
    let b_t_ref = format!("refs/sce/mutation-cursor/{}/{}", b_id.0, tree_t.0);
    assert!(
        !ref_exists(&repo.linked_root, &b_t_ref),
        "B's own pin for T must be deliberately absent -- the degraded state under test"
    );

    let a_snapshot = GitSnapshotService::new(&repo.main_root)
        .expect("a snapshot service should construct for A");
    std::fs::write(
        repo.main_root.join("shared-content.txt"),
        b"identical content",
    )
    .expect("A's edit toward the byte-identical T should write");
    let a_tree_t = a_snapshot
        .capture_tree()
        .expect("capturing A's view of T should succeed");
    assert_eq!(
        a_tree_t, tree_t,
        "A and B must independently capture byte-identical tree content"
    );
    a_snapshot
        .pin_tree(&a_id, &a_tree_t)
        .expect("A pinning T should succeed");
    let a_t_ref = format!("refs/sce/mutation-cursor/{}/{}", a_id.0, a_tree_t.0);

    let outcome = reconcile_worktree(&repo.main_root, ok_db)
        .expect("reconciling A should succeed through the public entrypoint");
    let report = match outcome {
        ReconciliationOutcome::Reconciled(report) => report,
        ReconciliationOutcome::SkippedNoCheckoutIdentity => {
            panic!("expected a Reconciled outcome, got SkippedNoCheckoutIdentity")
        }
    };
    assert_eq!(
        report.deleted, 0,
        "A must retain its T pin: T is a repository-wide durable root via B's historical event, \
         even though A does not durably reference T itself"
    );
    assert!(
        ref_exists(&repo.main_root, &a_t_ref),
        "A's T pin is the last SCE ref protecting T and must survive"
    );
    run_git(&repo.main_root, &["cat-file", "-t", &a_tree_t.0]);
}

#[test]
fn missing_local_required_pin_fails_closed_and_deletes_nothing_even_when_another_worktree_pins_the_tree(
) {
    let repo = LinkedTestRepo::new("missing-required-pin-fail-closed");
    let ok_db = || repo.open_db();

    let a_baseline = coordinate(&repo.main_root, &RuntimeBoundary::Flush, ok_db)
        .expect("A's baseline observation should materialize its worktree");
    let a_id = a_baseline.worktree_id.clone();
    let tree_a = a_baseline.observed_tree.clone();

    let a_snapshot = GitSnapshotService::new(&repo.main_root)
        .expect("a snapshot service should construct for A");
    std::fs::write(repo.main_root.join("required-but-unpinned.txt"), b"B")
        .expect("the edit toward B should write");
    let tree_b = a_snapshot
        .capture_tree()
        .expect("capturing B should succeed");
    let db = repo.db();
    seed_event(&db, &a_id.0, 1, &tree_a.0, &tree_b.0);
    let pin_ref_for_missing_b = format!("refs/sce/mutation-cursor/{}/{}", a_id.0, tree_b.0);
    assert!(
        !ref_exists(&repo.main_root, &pin_ref_for_missing_b),
        "A's pin for B must be deliberately absent"
    );

    std::fs::write(repo.main_root.join("unrelated.txt"), b"X")
        .expect("the unrelated edit should write");
    let tree_x = a_snapshot
        .capture_tree()
        .expect("capturing X should succeed");
    a_snapshot
        .pin_tree(&a_id, &tree_x)
        .expect("pinning X should succeed");
    let pin_ref_for_x = format!("refs/sce/mutation-cursor/{}/{}", a_id.0, tree_x.0);

    let b_baseline = coordinate(&repo.linked_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the linked worktree's baseline observation should materialize");
    let b_id = b_baseline.worktree_id.clone();
    let b_snapshot = GitSnapshotService::new(&repo.linked_root)
        .expect("a snapshot service should construct for the linked worktree");
    b_snapshot
        .pin_tree(&b_id, &tree_b)
        .expect("the other worktree pinning B should succeed");

    let outcome = reconcile_worktree(&repo.main_root, ok_db);
    match outcome {
        Err(ReconcileError::MissingRequiredPins { missing }) => {
            assert_eq!(
                missing,
                vec![tree_b.clone()],
                "the missing pin must name exactly B"
            );
        }
        other => panic!("expected MissingRequiredPins naming B, got {other:?}"),
    }

    assert!(
        ref_exists(&repo.main_root, &pin_ref_for_x),
        "A's X pin must be left untouched by a fail-closed pass"
    );
    assert!(
        !ref_exists(&repo.main_root, &pin_ref_for_missing_b),
        "A's still-absent pin for B must remain absent -- reconciliation never creates or repairs a pin"
    );
}

#[test]
fn a_malformed_namespace_ref_fails_closed_through_the_public_entrypoint() {
    let repo = TestRepo::new("malformed-ref-fail-closed");
    let ok_db = || repo.open_db();

    let baseline = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();
    let cursor_ref = format!(
        "refs/sce/mutation-cursor/{}/{}",
        worktree_id.0, baseline.observed_tree.0
    );

    let snapshot = GitSnapshotService::new(&repo.repo_root)
        .expect("a snapshot service should construct for the worktree");
    std::fs::write(repo.repo_root.join("unrelated.txt"), b"unrelated content")
        .expect("the unrelated edit should write");
    let some_tree = snapshot
        .capture_tree()
        .expect("capturing an arbitrary tree should succeed");
    let symbolic_ref = format!("refs/sce/mutation-cursor/{}/{}", worktree_id.0, some_tree.0);
    run_git(
        &repo.repo_root,
        &["symbolic-ref", &symbolic_ref, &cursor_ref],
    );

    let outcome = reconcile_worktree(&repo.repo_root, ok_db);
    match outcome {
        Err(ReconcileError::MalformedPin { ref_name, .. }) => {
            assert_eq!(ref_name, symbolic_ref);
        }
        other => panic!("expected MalformedPin for the symbolic ref, got {other:?}"),
    }

    assert!(
        ref_exists(&repo.repo_root, &cursor_ref),
        "the pass must fail closed before deleting anything, including the well-formed cursor pin"
    );
    assert_eq!(
        run_git(&repo.repo_root, &["symbolic-ref", &symbolic_ref]).trim(),
        cursor_ref,
        "the malformed symbolic ref itself must be left untouched"
    );
}

#[test]
fn reconciliation_makes_no_protocol_or_marker_write() {
    let repo = TestRepo::new("no-protocol-or-marker-write");
    let ok_db = || repo.open_db();
    let git_dir = resolve_git_dir(&repo.repo_root).expect("git dir should resolve");
    let marker = ExternalTaintMarker::new(&git_dir);

    let baseline = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();

    let snapshot = GitSnapshotService::new(&repo.repo_root)
        .expect("a snapshot service should construct for the worktree");
    std::fs::write(repo.repo_root.join("orphan.txt"), b"orphan")
        .expect("the orphan-producing edit should write");
    let orphan = snapshot
        .capture_tree()
        .expect("capturing the orphan tree should succeed");
    snapshot
        .pin_tree(&worktree_id, &orphan)
        .expect("pinning the orphan should succeed");

    let tables = [
        "mutation_trace_worktrees",
        "mutation_trace_scopes",
        "mutation_trace_events",
        "mutation_trace_processed_events",
        "mutation_trace_event_active_scopes",
    ];
    let db_before = repo.db();
    let counts_before: Vec<i64> = tables
        .iter()
        .map(|table| row_count(&db_before, table))
        .collect();
    let store_before = MutationTraceStore::new(&db_before);
    let worktree_state_before = store_before
        .load_worktree(&worktree_id, None, None)
        .expect("loading the worktree row should succeed")
        .expect("the worktree row should exist")
        .worktree_state;
    let marker_existed_before = marker
        .exists()
        .expect("marker existence should resolve before the pass");
    assert!(
        !marker_existed_before,
        "a successful coordinate() must have already cleared the marker before reconciliation runs"
    );

    let outcome = reconcile_worktree(&repo.repo_root, ok_db)
        .expect("reconciliation should succeed through the public entrypoint");
    let report = match outcome {
        ReconciliationOutcome::Reconciled(report) => report,
        ReconciliationOutcome::SkippedNoCheckoutIdentity => {
            panic!("expected a Reconciled outcome, got SkippedNoCheckoutIdentity")
        }
    };
    assert_eq!(
        report.deleted, 1,
        "the pass must actually mutate Git refs, not be a trivial no-op"
    );

    let db_after = repo.db();
    let counts_after: Vec<i64> = tables
        .iter()
        .map(|table| row_count(&db_after, table))
        .collect();
    assert_eq!(
        counts_before, counts_after,
        "every mutation-trace table's row count must be byte-identical after a reconciliation pass"
    );
    let store_after = MutationTraceStore::new(&db_after);
    let worktree_state_after = store_after
        .load_worktree(&worktree_id, None, None)
        .expect("loading the worktree row should succeed")
        .expect("the worktree row should exist")
        .worktree_state;
    assert_eq!(
        worktree_state_before, worktree_state_after,
        "cursor_tree/revision/tainted/failure_kind/needs_rebaseline must be unchanged by reconciliation"
    );
    assert!(
        !marker
            .exists()
            .expect("marker existence should resolve after the pass"),
        "reconciliation must not create the external-taint marker"
    );
}

#[test]
fn reconciliation_deletes_a_stale_ref_without_reclaiming_the_object_through_the_public_api() {
    let repo = TestRepo::new("no-object-reclamation-via-public-api");
    let ok_db = || repo.open_db();

    let baseline = coordinate(&repo.repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();

    let snapshot = GitSnapshotService::new(&repo.repo_root)
        .expect("a snapshot service should construct for the worktree");
    std::fs::write(
        repo.repo_root
            .join("only-reachable-through-a-stale-pin.txt"),
        b"O",
    )
    .expect("the edit toward O should write");
    let tree_o = snapshot.capture_tree().expect("capturing O should succeed");
    snapshot
        .pin_tree(&worktree_id, &tree_o)
        .expect("pinning O should succeed");
    let o_ref = format!("refs/sce/mutation-cursor/{}/{}", worktree_id.0, tree_o.0);

    run_git(&repo.repo_root, &["cat-file", "-t", &tree_o.0]);

    let outcome = reconcile_worktree(&repo.repo_root, ok_db)
        .expect("reconciliation should succeed through the public entrypoint");
    let report = match outcome {
        ReconciliationOutcome::Reconciled(report) => report,
        ReconciliationOutcome::SkippedNoCheckoutIdentity => {
            panic!("expected a Reconciled outcome, got SkippedNoCheckoutIdentity")
        }
    };
    assert_eq!(
        report.deleted, 1,
        "O's only SCE ref must be reclaimed as an orphan pin"
    );
    assert!(
        !ref_exists(&repo.repo_root, &o_ref),
        "O's ref must be gone after reconciliation"
    );

    let cat_file_type = run_git(&repo.repo_root, &["cat-file", "-t", &tree_o.0])
        .trim()
        .to_string();
    assert_eq!(
        cat_file_type, "tree",
        "O must still resolve via git cat-file -t immediately after its stale ref is deleted, \
         because reconciliation runs no git gc / git prune / git reflog expire"
    );
}

#[test]
fn missing_checkout_identity_through_the_public_entrypoint_returns_skipped_outcome() {
    let repo = TestRepo::new("missing-checkout-identity-public-entrypoint");

    let git_dir = resolve_git_dir(&repo.repo_root).expect("git dir should resolve");
    assert!(
        read_checkout_id(&git_dir)
            .expect("reading the checkout-id file should succeed")
            .is_none(),
        "a freshly initialized repository must have no checkout identity yet"
    );

    let outcome = reconcile_worktree(&repo.repo_root, || {
        panic!("open_db must never be called on the missing-checkout-identity path")
    })
    .expect("the missing-checkout-identity path is a clean Ok(..), never an Err");

    assert_eq!(
        outcome,
        ReconciliationOutcome::SkippedNoCheckoutIdentity,
        "with no current checkout identity to derive an owned namespace from, reconciliation must \
         return the distinct skip outcome, never a zero-count Reconciled(..) report"
    );
    assert!(
        read_checkout_id(&git_dir)
            .expect("reading the checkout-id file should succeed")
            .is_none(),
        "the skip must not create a checkout identity"
    );
}
