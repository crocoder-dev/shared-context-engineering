use std::time::Duration;

use super::super::super::external_taint::ExternalTaintMarker;
use super::super::super::git_snapshot::resolve_git_dir;
use super::super::super::maintenance_state::{read_state, state_path, StateRead};
use super::super::super::ref_reconciliation::{ReconcileError, ReconcilePhase};
use super::super::reconcile_explicit_with;
use super::super::{ExplicitOutcome, SkipReason};
use super::support::{await_signal, pinned_trees, spawn_parked_pass, write_file};
use super::{pin_orphan, seed_durable_root, Fixture, NOW};

const REMOTE: &str = "https://example.invalid/org/repo-a.git";

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_missing_local_pin_fails_closed_while_other_orphans_exist() {
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
    let refs_before = pinned_trees(&fixture.root, "main");
    assert_eq!(refs_before, [orphan].into_iter().collect());

    let (outcome, _) = fixture.run().await;

    let ExplicitOutcome::Failed { error, .. } = outcome else {
        panic!("expected failure, got {outcome:?}");
    };
    let ReconcileError::MissingRequiredPins { missing } = error else {
        panic!("expected missing required pins, got {error:?}");
    };
    assert_eq!(missing, vec![unpinned]);
    assert_eq!(
        pinned_trees(&fixture.root, "main"),
        refs_before,
        "the eligible orphan is not deleted when a required pin is missing"
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
