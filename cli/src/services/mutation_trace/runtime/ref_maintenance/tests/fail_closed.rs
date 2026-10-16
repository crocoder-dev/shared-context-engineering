use std::time::Duration;

use super::super::super::external_taint::ExternalTaintMarker;
use super::super::super::git_snapshot::resolve_git_dir;
use super::super::super::maintenance_state::{read_state, state_path, StateRead};
use super::super::super::ref_reconciliation::{ReconcileError, ReconcilePhase};
use super::super::reconcile_explicit_with;
use super::super::{ExplicitOutcome, SkipReason};
use super::support::{await_signal, pinned_trees, spawn_parked_pass, write_file};
use super::{
    event_revisions, pin_orphan, seed_durable_root, seed_event, Fixture, ABSENT_TREE,
    ABSENT_TREE_2, NOW,
};

const REMOTE: &str = "https://example.invalid/org/repo-a.git";

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_repins_missing_local_pin_whose_tree_exists_and_deletes_orphans() {
    let fixture = Fixture::new(REMOTE);
    fixture.create_db().await;
    write_file(&fixture.root, "orphan.txt", "orphan\n");
    let orphan = pin_orphan(&fixture.root).await;
    write_file(&fixture.root, "required.txt", "required\n");
    let snapshot = super::super::super::git_snapshot::GitSnapshotService::new(&fixture.root)
        .await
        .expect("snapshot");
    let unpinned = snapshot.capture_tree().await.expect("tree");
    seed_durable_root(&fixture.db_path(), &unpinned.0).await;
    assert_eq!(
        pinned_trees(&fixture.root, "main"),
        [orphan].into_iter().collect()
    );

    let (outcome, _) = fixture.run().await;

    let ExplicitOutcome::Completed(report) = outcome else {
        panic!("expected completed pass, got {outcome:?}");
    };
    assert_eq!(report.repinned, 1);
    assert_eq!(report.pruned_events, 0);
    assert_eq!(report.deleted, 1);
    assert_eq!(
        pinned_trees(&fixture.root, "main"),
        [unpinned.0].into_iter().collect()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_prunes_events_whose_trees_are_absent_and_repins_the_rest() {
    let fixture = Fixture::new(REMOTE);
    fixture.create_db().await;
    let snapshot = super::super::super::git_snapshot::GitSnapshotService::new(&fixture.root)
        .await
        .expect("snapshot");
    let first = snapshot.capture_tree().await.expect("tree");
    write_file(&fixture.root, "later.txt", "later\n");
    let second = snapshot.capture_tree().await.expect("tree");
    seed_durable_root(&fixture.db_path(), &second.0).await;
    seed_event(&fixture.db_path(), 1, ABSENT_TREE, ABSENT_TREE_2).await;
    seed_event(&fixture.db_path(), 2, ABSENT_TREE_2, &first.0).await;
    seed_event(&fixture.db_path(), 3, &first.0, &second.0).await;

    let (outcome, _) = fixture.run().await;

    let ExplicitOutcome::Completed(report) = outcome else {
        panic!("expected completed pass, got {outcome:?}");
    };
    assert_eq!(report.pruned_events, 2);
    assert_eq!(report.repinned, 2);
    assert_eq!(report.deleted, 0);
    assert_eq!(event_revisions(&fixture.db_path()).await, vec![3]);
    assert_eq!(
        pinned_trees(&fixture.root, "main"),
        [first.0, second.0].into_iter().collect()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_absent_cursor_tree_fails_closed_without_pruning() {
    let fixture = Fixture::new(REMOTE);
    fixture.create_db().await;
    write_file(&fixture.root, "orphan.txt", "orphan\n");
    let orphan = pin_orphan(&fixture.root).await;
    seed_durable_root(&fixture.db_path(), ABSENT_TREE).await;
    seed_event(&fixture.db_path(), 1, ABSENT_TREE_2, ABSENT_TREE).await;

    let (outcome, _) = fixture.run().await;

    let ExplicitOutcome::Failed { error, .. } = outcome else {
        panic!("expected failure, got {outcome:?}");
    };
    assert!(error
        .to_string()
        .contains("Let one agent tool run in this checkout"));
    assert!(error.to_string().contains("rerun `sce doctor --fix`"));
    let ReconcileError::UnrecoverableCursor { tree } = error else {
        panic!("expected unrecoverable cursor, got {error:?}");
    };
    assert_eq!(tree.0, ABSENT_TREE);
    assert_eq!(event_revisions(&fixture.db_path()).await, vec![1]);
    assert_eq!(
        pinned_trees(&fixture.root, "main"),
        [orphan].into_iter().collect()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_database_failure_never_arms_the_external_taint_marker() {
    let fixture = Fixture::new(REMOTE);
    pin_orphan(&fixture.root).await;
    let git_dir = resolve_git_dir(&fixture.root).await.expect("git dir");
    let marker = ExternalTaintMarker::new(&git_dir);

    let (outcome, _) = fixture.run().await;

    assert!(matches!(outcome, ExplicitOutcome::Failed { .. }));
    assert!(!marker.exists().expect("marker"));
    assert_eq!(pinned_trees(&fixture.root, "main").len(), 1);
    let StateRead::Valid(state) = read_state(&state_path(&git_dir)).expect("state") else {
        panic!("expected recorded failure");
    };
    assert_eq!(state.consecutive_failures, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ref_reconciliation_second_explicit_pass_is_busy_and_records_exactly_one_pass() {
    let fixture = Fixture::new(REMOTE);
    fixture.create_db().await;
    pin_orphan(&fixture.root).await;
    let git_dir = resolve_git_dir(&fixture.root).await.expect("git dir");

    let mut first = spawn_parked_pass(&fixture, ReconcilePhase::PinsInventoried);
    await_signal(&mut first.parked).await;

    let second = reconcile_explicit_with(
        &fixture.root,
        async || -> anyhow::Result<_> { unreachable!("a busy pass never opens the database") },
        || NOW,
        |_, _| unreachable!("a busy pass never writes state"),
        |_| {},
        Duration::ZERO,
    )
    .await;
    assert!(matches!(second, ExplicitOutcome::Skipped(SkipReason::Busy)));
    assert!(matches!(
        read_state(&state_path(&git_dir)).expect("state"),
        StateRead::Absent
    ));

    let outcome = first.finish().await;
    let ExplicitOutcome::Completed(report) = outcome else {
        panic!("expected completed pass, got {outcome:?}");
    };
    assert_eq!(report.deleted, 1);
    let StateRead::Valid(state) = read_state(&state_path(&git_dir)).expect("state") else {
        panic!("expected recorded success");
    };
    assert_eq!(state.last_success, Some(NOW));
    assert_eq!(state.consecutive_failures, 0);
    assert!(pinned_trees(&fixture.root, "main").is_empty());
}
