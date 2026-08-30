use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_storage::{
    resolve_agent_trace_storage_at_state_root, AgentTraceStorageContext,
};
use crate::services::checkout::{read_checkout_id, resolve_git_dir};
use crate::services::mutation_trace::store::MutationTraceStore;
use crate::services::mutation_trace::types::{ActorKind, EventId, ScopeId};

use super::coordinator::{coordinate, CoordinateError, RuntimeBoundary};
use super::git_snapshot::GitSnapshotService;
use super::worktree_lock::WorktreeLock;

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
