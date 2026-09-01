use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Barrier};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
use super::ref_reconciliation::{reconcile_worktree_inner, ReconciliationOutcome};
use super::worktree_lock::{acquire_inner, WorktreeLock};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn unique_path(label: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sce-mutation-trace-runtime-{label}-{}-{nonce}-{id}",
        std::process::id()
    ))
}

fn run_git(dir: &Path, args: &[&str]) {
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
}

fn init_repo(root: &Path) {
    std::fs::create_dir_all(root).expect("repository directory should be created");
    run_git(root, &["init", "--quiet"]);
    run_git(root, &["config", "user.email", "test@example.com"]);
    run_git(root, &["config", "user.name", "Test"]);
    run_git(root, &["commit", "--allow-empty", "--quiet", "-m", "init"]);
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
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
fn linked_worktrees_have_independent_locks_and_worktree_ids() {
    let main_root = unique_path("linked-main");
    init_repo(&main_root);
    let linked_root = unique_path("linked-secondary");
    run_git(
        &main_root,
        &[
            "worktree",
            "add",
            "--quiet",
            linked_root.to_str().expect("worktree path should be UTF-8"),
        ],
    );

    let db_path = main_root.join("agent-trace.db");
    let db_main = RepositoryAgentTraceDb::new_at(&db_path).expect("main repository DB should open");

    let main_git_dir = resolve_git_dir(&main_root).expect("main git dir should resolve");
    let linked_git_dir = resolve_git_dir(&linked_root).expect("linked git dir should resolve");
    assert_ne!(
        main_git_dir, linked_git_dir,
        "a linked worktree must resolve its own worktree-specific git dir, giving it a distinct lock and identity path"
    );

    let main_outcome = coordinate(&main_root, &RuntimeBoundary::Flush, || {
        RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
    })
    .expect("first observation on the main worktree should succeed");

    let held = WorktreeLock::acquire(&main_git_dir, Duration::from_secs(5))
        .expect("the main worktree's runtime lock should be acquirable");

    let linked_outcome = coordinate(&linked_root, &RuntimeBoundary::Flush, || {
        // A second handle to the same caller-supplied repository-scoped DB path,
        // opened through the provider while the main worktree's lock is held.
        RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
    })
    .expect(
        "coordinate() on the linked worktree must acquire its own distinct runtime lock while the main worktree's lock is still held",
    );

    drop(held);

    assert_ne!(
        main_outcome.worktree_id, linked_outcome.worktree_id,
        "each linked worktree must derive a distinct WorktreeId from its own checkout identity"
    );

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

    let linked_snapshot = GitSnapshotService::new(&linked_root)
        .expect("a snapshot service should construct for the linked worktree");
    linked_snapshot
        .diff_trees(&main_outcome.observed_tree, &main_outcome.observed_tree)
        .expect("a tree pinned by the main worktree's coordinator must resolve through the linked worktree's git dir");

    cleanup(&linked_root);
    cleanup(&main_root);
}

#[test]
fn agent_trace_storage_and_coordinator_observe_the_same_checkout_id() {
    let repo_root = unique_path("cross-caller-repo");
    init_repo(&repo_root);
    run_git(
        &repo_root,
        &["remote", "add", "origin", "git@github.com:acme/widgets.git"],
    );
    let state_root = unique_path("cross-caller-state");
    std::fs::create_dir_all(&state_root).expect("state root should be created");

    let coordinator_db_path = repo_root.join("coordinator.db");
    RepositoryAgentTraceDb::new_at(&coordinator_db_path)
        .expect("the coordinator's repository DB should open with schema");

    let barrier = Arc::new(Barrier::new(2));

    let storage_thread = {
        let repo_root = repo_root.clone();
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
    let outcome = coordinate(&repo_root, &RuntimeBoundary::Flush, || {
        RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&coordinator_db_path)
    })
    .expect("the coordinator's first observation should succeed");

    let storage_checkout_id = storage_thread
        .join()
        .expect("the storage thread should not panic");

    assert_eq!(
        outcome.worktree_id.0, storage_checkout_id,
        "the coordinator and agent_trace_storage must converge on one checkout identity for the same physical checkout"
    );

    let on_disk = read_checkout_id(&resolve_git_dir(&repo_root).expect("git dir should resolve"))
        .expect("reading the checkout-id file should succeed")
        .expect("a checkout id must have been persisted");
    assert_eq!(
        on_disk, storage_checkout_id,
        "the on-disk checkout-id file must contain the converged identity"
    );

    cleanup(&repo_root);
    cleanup(&state_root);
}

#[test]
fn a_snapshot_failure_then_recovery_cycle_runs_through_the_public_api() {
    let repo_root = unique_path("failure-recovery-repo");
    init_repo(&repo_root);
    let db_path = repo_root.join("agent-trace.db");
    let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");

    let baseline = coordinate(&repo_root, &RuntimeBoundary::Flush, || {
        RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
    })
    .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();

    let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
    let tmp_index_dir = git_dir.join("sce").join("tmp");
    let _ = std::fs::remove_dir_all(&tmp_index_dir);
    std::fs::write(&tmp_index_dir, b"not a directory").expect(
        "planting a file where the snapshot service expects its temp-index directory should succeed",
    );

    let scope = ScopeId("scope-recovery".to_string());
    let failure = coordinate(
        &repo_root,
        &RuntimeBoundary::Start {
            scope: scope.clone(),
            event: EventId("evt-during-failure".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        || RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path),
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

    let recovered = coordinate(&repo_root, &RuntimeBoundary::Flush, || {
        RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
    })
    .expect("the coordinator should recover from the taint and process the boundary");
    assert_eq!(
        recovered.worktree_id, worktree_id,
        "recovery must operate on the same worktree identity"
    );

    let store = MutationTraceStore::new(&db);
    let projection = store
        .load_worktree(&worktree_id, None, None)
        .expect("loading the worktree row should succeed")
        .expect("the worktree row should still exist");
    assert!(
        !projection.worktree_state.tainted,
        "taint recovery must clear the tainted flag before the triggering boundary is processed"
    );

    cleanup(&repo_root);
}

#[test]
fn a_successful_coordinate_through_the_public_api_leaves_no_external_taint_marker() {
    let repo_root = unique_path("public-success-no-marker");
    init_repo(&repo_root);
    // The DB lives outside the worktree so it never perturbs the observed tree.
    let db_path = unique_path("public-success-no-marker-db").join("agent-trace.db");
    RepositoryAgentTraceDb::new_at(&db_path).expect("the repository DB should open with schema");
    let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
    let marker = ExternalTaintMarker::new(&git_dir);

    let outcome = coordinate(&repo_root, &RuntimeBoundary::Flush, || {
        RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
    })
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

    cleanup(
        db_path
            .parent()
            .expect("the DB path has a parent directory"),
    );
    cleanup(&repo_root);
}

#[test]
#[allow(clippy::too_many_lines)]
fn a_db_open_failure_after_arming_leaves_the_marker_and_the_next_invocation_rebaselines_without_evidence(
) {
    let repo_root = unique_path("public-db-open-failure-gap");
    init_repo(&repo_root);
    // The DB lives outside the worktree so it never perturbs the observed tree.
    let db_path = unique_path("public-db-open-failure-gap-db").join("agent-trace.db");
    RepositoryAgentTraceDb::new_at(&db_path).expect("the repository DB should open with schema");
    let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
    let marker = ExternalTaintMarker::new(&git_dir);
    let ok_db = || RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path);

    // A trusted baseline at cursor A, then one exclusive AI edit A -> B.
    let baseline = coordinate(&repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();
    let scope = ScopeId("scope-across-the-gap".to_string());
    coordinate(
        &repo_root,
        &RuntimeBoundary::Start {
            scope: scope.clone(),
            event: EventId("evt-start".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        ok_db,
    )
    .expect("starting the scope should succeed");
    std::fs::write(repo_root.join("work.txt"), b"v1").expect("the A -> B edit should write");
    let advanced = coordinate(
        &repo_root,
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

    // A boundary invocation whose DB provider fails after the marker is armed,
    // then the working tree keeps moving during the lost interval: B -> C.
    let db_failure = coordinate(
        &repo_root,
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
    std::fs::write(repo_root.join("work.txt"), b"v2-during-the-gap").expect("the B -> C edit");

    // The next successful invocation rebaselines to C with no evidence for the gap.
    let recovered = coordinate(&repo_root, &RuntimeBoundary::Flush, ok_db)
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
    let db = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
        .expect("reopening the DB for assertions should succeed");
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

    cleanup(
        db_path
            .parent()
            .expect("the DB path has a parent directory"),
    );
    cleanup(&repo_root);
}

#[test]
fn a_stale_marker_rebaselines_to_the_current_tree_abandons_scopes_then_processes_the_boundary() {
    let repo_root = unique_path("public-stale-marker-rebaseline");
    init_repo(&repo_root);
    // The DB lives outside the worktree so it never perturbs the observed tree.
    let db_path = unique_path("public-stale-marker-rebaseline-db").join("agent-trace.db");
    RepositoryAgentTraceDb::new_at(&db_path).expect("the repository DB should open with schema");
    let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
    let marker = ExternalTaintMarker::new(&git_dir);
    let ok_db = || RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path);

    // Trusted cursor A plus an active scope S.
    let baseline = coordinate(&repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();
    let tree_a = baseline.observed_tree.clone();
    let scope = ScopeId("scope-stranded".to_string());
    coordinate(
        &repo_root,
        &RuntimeBoundary::Start {
            scope: scope.clone(),
            event: EventId("evt-start".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        ok_db,
    )
    .expect("starting scope S should succeed");

    // A crashed invocation: the marker was armed and never cleared.
    marker
        .persist()
        .expect("simulating a crashed invocation that armed but never cleared the marker");

    // The working tree moves on to C while the process was gone.
    std::fs::write(
        repo_root.join("stranded.txt"),
        b"edited-while-the-process-was-gone",
    )
    .expect("the A -> C edit should write");

    // The next invocation inherits the stale marker.
    let recovered = coordinate(
        &repo_root,
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

    let db = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
        .expect("reopening the DB for assertions should succeed");
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

    // The triggering boundary was processed inside the inheriting invocation:
    // a plain follow-up flush finds nothing left to recover or advance.
    let stable = coordinate(&repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("a plain flush after recovery should succeed");
    assert_eq!(
        stable.revision, recovered.revision,
        "recovery and the triggering boundary already completed in the inheriting invocation"
    );
    assert!(stable.mutation_event.is_none());

    cleanup(
        db_path
            .parent()
            .expect("the DB path has a parent directory"),
    );
    cleanup(&repo_root);
}

#[test]
fn a_first_ever_failed_invocation_that_never_materialized_a_worktree_row_creates_no_evidence() {
    let repo_root = unique_path("public-first-ever-failure");
    init_repo(&repo_root);
    // The DB lives outside the worktree so it never perturbs the observed tree.
    let db_path = unique_path("public-first-ever-failure-db").join("agent-trace.db");
    RepositoryAgentTraceDb::new_at(&db_path).expect("the repository DB should open with schema");
    let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
    let marker = ExternalTaintMarker::new(&git_dir);

    std::fs::write(
        repo_root.join("pre-existing.txt"),
        b"content before any observation",
    )
    .expect("a pre-existing edit should write");

    // The very first invocation fails opening the DB, after arming the marker.
    let first = coordinate(
        &repo_root,
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

    // More edits during the still-unobserved interval.
    std::fs::write(
        repo_root.join("during-the-gap.txt"),
        b"more unknown-interval content",
    )
    .expect("another unobserved edit should write");

    // The first *successful* invocation establishes a baseline with no evidence
    // for the unknown interval.
    let established = coordinate(&repo_root, &RuntimeBoundary::Flush, || {
        RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
    })
    .expect("the first successful invocation establishes the baseline");
    assert!(
        established.mutation_event.is_none(),
        "a worktree with no prior durable row cannot produce evidence for the unknown interval"
    );
    assert!(
        !marker.exists().expect("marker existence should resolve"),
        "the successful baseline must clear the inherited marker"
    );

    let db = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
        .expect("reopening the DB for assertions should succeed");
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

    cleanup(
        db_path
            .parent()
            .expect("the DB path has a parent directory"),
    );
    cleanup(&repo_root);
}

#[test]
#[allow(clippy::too_many_lines)]
fn linked_worktrees_keep_independent_external_taint_markers_over_a_shared_db() {
    let main_root = unique_path("public-taint-linked-main");
    init_repo(&main_root);
    let linked_root = unique_path("public-taint-linked-secondary");
    run_git(
        &main_root,
        &[
            "worktree",
            "add",
            "--quiet",
            linked_root.to_str().expect("worktree path should be UTF-8"),
        ],
    );

    // One shared repository DB, outside either worktree so it never perturbs a
    // captured tree.
    let db_path = unique_path("public-taint-linked-db").join("agent-trace.db");
    RepositoryAgentTraceDb::new_at(&db_path).expect("the shared repository DB should open");

    let main_git_dir = resolve_git_dir(&main_root).expect("main git dir should resolve");
    let linked_git_dir = resolve_git_dir(&linked_root).expect("linked git dir should resolve");
    let main_marker = ExternalTaintMarker::new(&main_git_dir);
    let linked_marker = ExternalTaintMarker::new(&linked_git_dir);

    // Baseline both worktrees against the one shared DB.
    let main_baseline = coordinate(&main_root, &RuntimeBoundary::Flush, || {
        RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
    })
    .expect("the main worktree baseline should succeed");
    let linked_baseline = coordinate(&linked_root, &RuntimeBoundary::Flush, || {
        RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
    })
    .expect("the linked worktree baseline should succeed");
    assert_ne!(main_baseline.worktree_id, linked_baseline.worktree_id);

    // Strand a live scope in the linked worktree behind an armed marker.
    let linked_scope = ScopeId("scope-linked".to_string());
    coordinate(
        &linked_root,
        &RuntimeBoundary::Start {
            scope: linked_scope.clone(),
            event: EventId("evt-linked-start".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        || RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path),
    )
    .expect("starting the linked worktree's scope should succeed");
    linked_marker
        .persist()
        .expect("arming the linked worktree's marker should succeed");

    // A boundary in the MAIN worktree must not observe the linked worktree's marker.
    std::fs::write(main_root.join("main-work.txt"), b"v1")
        .expect("a main-worktree edit should write");
    coordinate(&main_root, &RuntimeBoundary::Flush, || {
        RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
    })
    .expect("the main worktree flush must succeed without inheriting the linked worktree's marker");
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

    let db = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
        .expect("reopening the shared DB for assertions should succeed");
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

    // The linked worktree's own next invocation inherits and recovers.
    std::fs::write(linked_root.join("linked-work.txt"), b"v1")
        .expect("a linked-worktree edit should write");
    let linked_recovered = coordinate(&linked_root, &RuntimeBoundary::Flush, || {
        RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
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

    cleanup(
        db_path
            .parent()
            .expect("the DB path has a parent directory"),
    );
    cleanup(&linked_root);
    cleanup(&main_root);
}

#[test]
fn a_snapshot_failure_arms_the_marker_and_the_next_invocation_recovers_once() {
    let repo_root = unique_path("public-snapshot-failure-marker-recovery");
    init_repo(&repo_root);
    // The DB lives outside the worktree so it never perturbs the observed tree.
    let db_path = unique_path("public-snapshot-failure-marker-recovery-db").join("agent-trace.db");
    RepositoryAgentTraceDb::new_at(&db_path).expect("the repository DB should open with schema");
    let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
    let marker = ExternalTaintMarker::new(&git_dir);
    let ok_db = || RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path);

    let baseline = coordinate(&repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();
    assert!(!marker.exists().expect("marker existence should resolve"));

    // Plant a file where the snapshot service needs its temp-index directory.
    let tmp_index_dir = git_dir.join("sce").join("tmp");
    let _ = std::fs::remove_dir_all(&tmp_index_dir);
    std::fs::write(&tmp_index_dir, b"not a directory")
        .expect("planting a file where the snapshot service expects its temp-index directory");

    let scope = ScopeId("scope-during-the-failure".to_string());
    let failure = coordinate(
        &repo_root,
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
        let db = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
            .expect("reopening the DB for assertions should succeed");
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
    std::fs::write(repo_root.join("work.txt"), b"v1")
        .expect("an edit before recovery should write");

    let recovered = coordinate(&repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the coordinator should recover from the taint and process the boundary");
    assert!(
        recovered.mutation_event.is_none(),
        "the conservative recovery emits no evidence for the fenced interval"
    );
    assert!(
        !marker.exists().expect("marker existence should resolve"),
        "a successful recovery clears the marker"
    );

    let db = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
        .expect("reopening the DB for assertions should succeed");
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

    // A plain follow-up flush proves the recovery already fully completed once:
    // there is nothing left to recover.
    let stable = coordinate(&repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("a plain flush after recovery should succeed");
    assert_eq!(
        stable.revision, recovered.revision,
        "the single conservative recovery already completed; a follow-up flush is a no-op"
    );
    assert!(stable.mutation_event.is_none());

    cleanup(
        db_path
            .parent()
            .expect("the DB path has a parent directory"),
    );
    cleanup(&repo_root);
}

#[test]
#[allow(clippy::too_many_lines)]
fn a_marker_clear_failure_after_a_durable_boundary_keeps_the_marker_for_a_later_recovery() {
    let repo_root = unique_path("public-marker-clear-failure");
    init_repo(&repo_root);
    // The DB lives outside the worktree so it never perturbs the observed tree.
    let db_path = unique_path("public-marker-clear-failure-db").join("agent-trace.db");
    RepositoryAgentTraceDb::new_at(&db_path).expect("the repository DB should open with schema");
    let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
    let marker = ExternalTaintMarker::new(&git_dir);
    let marker_path = git_dir.join("sce").join("mutation-cursor-tainted");
    let ok_db = || RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path);

    let baseline = coordinate(&repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();

    let scope = ScopeId("scope-attributable".to_string());
    coordinate(
        &repo_root,
        &RuntimeBoundary::Start {
            scope: scope.clone(),
            event: EventId("evt-start".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        ok_db,
    )
    .expect("starting the scope should succeed");
    std::fs::write(repo_root.join("work.txt"), b"v1")
        .expect("an exclusive edit before the boundary should write");

    let clear_marker_path = marker_path.clone();
    let clear_db_path = db_path.clone();
    let error = coordinate(
        &repo_root,
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
        let db = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
            .expect("reopening the DB for assertions should succeed");
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
    std::fs::write(repo_root.join("work.txt"), b"v2").expect("a later edit should write");

    let recovered = coordinate(&repo_root, &RuntimeBoundary::Flush, ok_db)
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

    cleanup(
        db_path
            .parent()
            .expect("the DB path has a parent directory"),
    );
    cleanup(&repo_root);
}

#[test]
fn reconciliation_blocks_on_the_worktree_lock_and_retains_a_pin_that_becomes_durable_under_it() {
    let repo_root = unique_path("reconcile-blocks-on-worktree-lock");
    init_repo(&repo_root);
    let db_path = unique_path("reconcile-blocks-on-worktree-lock-db").join("agent-trace.db");
    RepositoryAgentTraceDb::new_at(&db_path).expect("the repository DB should open with schema");
    let ok_db = || RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path);

    let baseline = coordinate(&repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();
    let baseline_tree = baseline.observed_tree.clone();

    let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
    let held = acquire_inner(&git_dir, Duration::from_secs(5), || {})
        .expect("the test should hold a real WorktreeLock before the worker runs");

    let (contention_tx, contention_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let repo_root_clone = repo_root.clone();
    let db_path_clone = db_path.clone();
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

    let snapshot = GitSnapshotService::new(&repo_root)
        .expect("a snapshot service should construct for the worktree");
    std::fs::write(repo_root.join("under-lock.txt"), b"under the lock")
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
        &repo_root,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/sce/mutation-cursor/{}/{}", worktree_id.0, x.0),
        ],
    );

    cleanup(
        db_path
            .parent()
            .expect("the DB path has a parent directory"),
    );
    cleanup(&repo_root);
}

#[test]
#[allow(clippy::too_many_lines)]
fn reconciliation_blocks_until_a_real_coordinate_cas_commits_the_pinned_tree() {
    let repo_root = unique_path("reconcile-blocks-until-real-cas");
    init_repo(&repo_root);
    let db_path = unique_path("reconcile-blocks-until-real-cas-db").join("agent-trace.db");
    RepositoryAgentTraceDb::new_at(&db_path).expect("the repository DB should open with schema");
    let ok_db = || RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path);

    let baseline = coordinate(&repo_root, &RuntimeBoundary::Flush, ok_db)
        .expect("the baseline observation should materialize the worktree");
    let worktree_id = baseline.worktree_id.clone();
    let baseline_tree = baseline.observed_tree.clone();

    std::fs::write(
        repo_root.join("under-real-cas.txt"),
        b"observed before the real CAS",
    )
    .expect("an edit before the coordinated boundary should write");

    let (pinned_tx, pinned_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let (coordinate_done_tx, coordinate_done_rx) = mpsc::channel();
    let coord_repo_root = repo_root.clone();
    let coord_db_path = db_path.clone();
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
    let rec_repo_root = repo_root.clone();
    let rec_db_path = db_path.clone();
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
        &repo_root,
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

    cleanup(
        db_path
            .parent()
            .expect("the DB path has a parent directory"),
    );
    cleanup(&repo_root);
}
