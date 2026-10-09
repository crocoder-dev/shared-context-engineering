use std::path::Path;
use std::time::Duration;

use super::super::super::external_taint::ExternalTaintMarker;
use super::super::super::git_snapshot::resolve_git_dir;
use super::super::super::maintenance_state::{
    evaluate_recommendation, read_state, state_path, write_state_atomically, MaintenanceState,
    PersistFailure, PersistPhase, StateRead, FUTURE_SKEW_TOLERANCE_MS,
    RECONCILIATION_ADVISORY_AFTER_MS,
};
use super::super::super::ref_advisory::{advise_if_due, AdvisoryOutcome};
use super::super::super::worktree_lock::acquire_inner_async;
use super::super::{reconcile_explicit_with, ExplicitOutcome, SkipReason, StatePersistWarning};
use super::support::pinned_trees;
use super::{
    failing_write, pin_orphan, remove_database, unavailable_cause, Fixture, NOW,
    OTHER_REPOSITORY_ID,
};
use crate::services::agent_trace_db::repository::{
    ExistingRepositoryDbError, RepositoryAgentTraceDb,
};

const REMOTE: &str = "https://example.invalid/org/repo-a.git";

async fn seed_state(fixture: &Fixture, state: &MaintenanceState) -> std::path::PathBuf {
    let git_dir = resolve_git_dir(&fixture.root).await.expect("git dir");
    let path = state_path(&git_dir);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("runtime dir");
    write_state_atomically(&path, state).expect("seed state");
    path
}

fn invalid_state() -> MaintenanceState {
    MaintenanceState {
        anchor: Some(NOW + 10 * RECONCILIATION_ADVISORY_AFTER_MS),
        last_success: Some(NOW + FUTURE_SKEW_TOLERANCE_MS + 1),
        last_advised: Some(NOW + 10 * RECONCILIATION_ADVISORY_AFTER_MS),
        ..MaintenanceState::default()
    }
}

fn assert_within_tolerance(state: &MaintenanceState) {
    for timestamp in [
        state.anchor,
        state.last_success,
        state.last_attempt,
        state.last_advised,
    ]
    .into_iter()
    .flatten()
    {
        assert!(timestamp <= NOW + FUTURE_SKEW_TOLERANCE_MS);
    }
}

async fn logical_dump(path: &Path) -> Vec<String> {
    let db = RepositoryAgentTraceDb::open_without_migrations_at(path)
        .await
        .expect("inspection open");
    let tables = db
        .query_map(
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
            (),
            |row| row.get::<String>(0).map_err(Into::into),
        )
        .await
        .expect("tables");
    let mut dump = Vec::new();
    for table in &tables {
        let columns = db
            .query_map(
                &format!("SELECT name FROM pragma_table_info('{table}') ORDER BY cid"),
                (),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .await
            .expect("columns");
        let projection = columns
            .iter()
            .map(|column| format!("quote(\"{column}\")"))
            .collect::<Vec<_>>()
            .join(" || '|' || ");
        let rows = db
            .query_map(
                &format!("SELECT {projection} FROM \"{table}\" ORDER BY rowid"),
                (),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .await
            .expect("rows");
        dump.push(format!("{table}:{}", rows.join(";")));
    }
    if tables.iter().any(|table| table == "__sce_migrations") {
        let ids = db
            .query_map(
                "SELECT CAST(id AS TEXT) FROM __sce_migrations ORDER BY id",
                (),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .await
            .expect("migrations");
        dump.push(format!("migrations:{}", ids.join(",")));
    }
    if tables.iter().any(|table| table == "repository_metadata") {
        let ids = db
            .query_map(
                "SELECT repository_id FROM repository_metadata ORDER BY id",
                (),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .await
            .expect("metadata");
        dump.push(format!("metadata:{}", ids.join(",")));
    }
    dump
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_verification_failures_leave_the_logical_database_unchanged() {
    for case in ["incompatible", "missing_metadata", "mismatch"] {
        let fixture = Fixture::new(REMOTE);
        match case {
            "incompatible" => drop(
                RepositoryAgentTraceDb::open_without_migrations_at(fixture.db_path())
                    .await
                    .expect("unmigrated db"),
            ),
            "missing_metadata" => drop(
                RepositoryAgentTraceDb::new_at(fixture.db_path())
                    .await
                    .expect("migrated db"),
            ),
            _ => super::create_db(&fixture.db_path(), OTHER_REPOSITORY_ID).await,
        }
        pin_orphan(&fixture.root).await;
        let git_dir = resolve_git_dir(&fixture.root).await.expect("git dir");
        let before = logical_dump(&fixture.db_path()).await;
        if case == "mismatch" {
            assert!(before.iter().any(|line| line.contains(OTHER_REPOSITORY_ID)));
        }

        let (outcome, phases) = fixture.run().await;

        let ExplicitOutcome::Failed { error, .. } = outcome else {
            panic!("{case}: expected failure, got {outcome:?}");
        };
        match (case, unavailable_cause(&error)) {
            ("incompatible", ExistingRepositoryDbError::IncompatibleSchema(_))
            | ("missing_metadata", ExistingRepositoryDbError::MissingMetadata)
            | ("mismatch", ExistingRepositoryDbError::RepositoryMismatch { .. }) => {}
            (_, other) => panic!("{case}: unexpected cause {other:?}"),
        }
        assert!(phases.is_empty(), "{case}");
        assert_eq!(logical_dump(&fixture.db_path()).await, before, "{case}");
        assert_eq!(pinned_trees(&fixture.root, "main").len(), 1, "{case}");
        assert!(!ExternalTaintMarker::new(&git_dir).exists().expect("marker"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_busy_explicit_pass_leaves_invalid_state_byte_identical() {
    let fixture = Fixture::new(REMOTE);
    let path = seed_state(&fixture, &invalid_state()).await;
    let before = std::fs::read(&path).expect("state bytes");
    let git_dir = resolve_git_dir(&fixture.root).await.expect("git dir");
    let held = acquire_inner_async(&git_dir, Duration::from_secs(10), || {})
        .await
        .expect("hold lock");

    let outcome = reconcile_explicit_with(
        &fixture.root,
        async || -> anyhow::Result<RepositoryAgentTraceDb> { unreachable!("db must not open") },
        || NOW,
        |_, _| unreachable!("state must not be written"),
        |_| {},
        Duration::ZERO,
    )
    .await;
    drop(held);

    assert!(matches!(
        outcome,
        ExplicitOutcome::Skipped(SkipReason::Busy)
    ));
    assert_eq!(std::fs::read(&path).expect("state bytes"), before);
    let StateRead::Valid(state) = read_state(&path).expect("state") else {
        panic!("expected valid state");
    };
    assert!(evaluate_recommendation(&state, NOW).invalid_timestamps);
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_successful_pass_normalizes_invalid_state_and_a_later_read_does_not_rewrite(
) {
    let fixture = Fixture::new(REMOTE);
    fixture.create_db().await;
    pin_orphan(&fixture.root).await;
    let near_future = NOW + FUTURE_SKEW_TOLERANCE_MS;
    let seeded = MaintenanceState {
        last_advised: Some(near_future),
        ..invalid_state()
    };
    let path = seed_state(&fixture, &seeded).await;

    let (outcome, _) = fixture.run().await;
    assert!(matches!(outcome, ExplicitOutcome::Completed(_)));

    let StateRead::Valid(normalized) = read_state(&path).expect("state") else {
        panic!("expected valid state");
    };
    assert_within_tolerance(&normalized);
    assert_eq!(normalized.last_success, Some(NOW));
    assert_eq!(normalized.last_advised, Some(near_future));
    let bytes = std::fs::read(&path).expect("state bytes");

    assert!(matches!(
        advise_if_due(&fixture.root, || NOW),
        AdvisoryOutcome::NoAction
    ));
    assert_eq!(std::fs::read(&path).expect("state bytes"), bytes);
    assert!(!evaluate_recommendation(&normalized, NOW).invalid_timestamps);
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_failed_state_write_during_normalization_has_two_distinct_legs() {
    let fixture = Fixture::new(REMOTE);
    fixture.create_db().await;
    pin_orphan(&fixture.root).await;
    let path = seed_state(&fixture, &invalid_state()).await;
    let before = std::fs::read(&path).expect("state bytes");

    let (pre_rename, _) = fixture.run_with(failing_write(PersistPhase::Rename)).await;
    let ExplicitOutcome::CompletedStatePersistFailed { report, warning } = pre_rename else {
        panic!("expected persist-failed completion, got {pre_rename:?}");
    };
    assert_eq!(report.deleted, 1);
    assert!(matches!(
        warning,
        StatePersistWarning::WriteFailed(PersistFailure::NotApplied {
            phase: PersistPhase::Rename,
            ..
        })
    ));
    assert_eq!(std::fs::read(&path).expect("state bytes"), before);

    pin_orphan(&fixture.root).await;
    let (post_rename, _) = fixture
        .run_with(failing_write(PersistPhase::SyncParentDirectory))
        .await;
    let ExplicitOutcome::CompletedStatePersistFailed { warning, .. } = post_rename else {
        panic!("expected persist-failed completion, got {post_rename:?}");
    };
    assert!(matches!(
        warning,
        StatePersistWarning::WriteFailed(PersistFailure::DurabilityUncertain {
            phase: PersistPhase::SyncParentDirectory,
            ..
        })
    ));
    let StateRead::Valid(visible) = read_state(&path).expect("state") else {
        panic!("expected valid state");
    };
    assert_within_tolerance(&visible);
    assert_eq!(visible.last_success, Some(NOW));

    let bytes = std::fs::read(&path).expect("state bytes");
    assert!(matches!(
        advise_if_due(&fixture.root, || NOW),
        AdvisoryOutcome::NoAction
    ));
    assert_eq!(std::fs::read(&path).expect("state bytes"), bytes);
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_explicit_success_resets_the_streak_and_failure_keeps_last_success() {
    let fixture = Fixture::new(REMOTE);
    pin_orphan(&fixture.root).await;
    let git_dir = resolve_git_dir(&fixture.root).await.expect("git dir");
    let path = state_path(&git_dir);
    let stored = || match read_state(&path).expect("state") {
        StateRead::Valid(state) => state,
        other => panic!("expected valid state, got {other:?}"),
    };

    for expected in 1..=2 {
        let (outcome, _) = fixture.run().await;
        assert!(matches!(outcome, ExplicitOutcome::Failed { .. }));
        assert_eq!(stored().consecutive_failures, expected);
    }

    fixture.create_db().await;
    let (outcome, _) = fixture.run().await;
    assert!(matches!(outcome, ExplicitOutcome::Completed(_)));
    let healed = stored();
    assert_eq!(healed.consecutive_failures, 0);
    assert_eq!(healed.last_failure, None);
    assert_eq!(healed.last_success, Some(NOW));

    remove_database(&fixture.db_path());
    let (outcome, _) = fixture.run().await;
    assert!(matches!(outcome, ExplicitOutcome::Failed { .. }));
    let failed = stored();
    assert_eq!(failed.consecutive_failures, 1);
    assert_eq!(failed.last_success, Some(NOW));
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_explicit_pass_runs_and_rewrites_valid_state_from_every_stored_condition(
) {
    let aged = MaintenanceState {
        anchor: Some(NOW - 3 * RECONCILIATION_ADVISORY_AFTER_MS),
        last_advised: Some(NOW - 2 * RECONCILIATION_ADVISORY_AFTER_MS),
        ..MaintenanceState::default()
    };
    for case in ["aged", "invalid", "corrupt", "oversized"] {
        let fixture = Fixture::new(REMOTE);
        fixture.create_db().await;
        pin_orphan(&fixture.root).await;
        let path = match case {
            "aged" => seed_state(&fixture, &aged).await,
            "invalid" => seed_state(&fixture, &invalid_state()).await,
            _ => {
                let path = seed_state(&fixture, &MaintenanceState::default()).await;
                let junk = if case == "corrupt" {
                    b"not json".to_vec()
                } else {
                    vec![b' '; 5000]
                };
                std::fs::write(&path, junk).expect("unusable state");
                path
            }
        };

        let (outcome, _) = fixture.run().await;

        let ExplicitOutcome::Completed(report) = outcome else {
            panic!("{case}: expected completed pass, got {outcome:?}");
        };
        assert_eq!(report.deleted, 1, "{case}");
        let StateRead::Valid(rewritten) = read_state(&path).expect("state") else {
            panic!("{case}: state must be rewritten valid");
        };
        assert_eq!(rewritten.last_success, Some(NOW), "{case}");
        assert_within_tolerance(&rewritten);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_unnormalized_invalid_state_stays_eligible_until_a_successful_advisory()
{
    let fixture = Fixture::new(REMOTE);
    let invalid = MaintenanceState {
        anchor: Some(NOW + 10 * RECONCILIATION_ADVISORY_AFTER_MS),
        ..MaintenanceState::default()
    };
    let path = seed_state(&fixture, &invalid).await;
    let before = std::fs::read(&path).expect("state bytes");
    let git_dir = resolve_git_dir(&fixture.root).await.expect("git dir");

    for step in 0..3 {
        let now = NOW + step * RECONCILIATION_ADVISORY_AFTER_MS / 2;
        let recommendation = evaluate_recommendation(&invalid, now);
        assert!(recommendation.recommended);
        assert!(recommendation.invalid_timestamps);
    }
    let held = acquire_inner_async(&git_dir, Duration::from_secs(10), || {})
        .await
        .expect("hold lock");
    for step in 0..3 {
        assert!(matches!(
            advise_if_due(&fixture.root, || NOW
                + step * RECONCILIATION_ADVISORY_AFTER_MS),
            AdvisoryOutcome::Busy
        ));
    }
    drop(held);
    assert_eq!(std::fs::read(&path).expect("state bytes"), before);

    let later = NOW + RECONCILIATION_ADVISORY_AFTER_MS;
    assert!(matches!(
        advise_if_due(&fixture.root, || later),
        AdvisoryOutcome::Advised
    ));
    let StateRead::Valid(state) = read_state(&path).expect("state") else {
        panic!("expected valid state");
    };
    assert_eq!(state.last_advised, Some(later));
    assert_eq!(state.anchor, Some(later));
    assert!(matches!(
        advise_if_due(&fixture.root, || later),
        AdvisoryOutcome::NoAction
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_invalid_last_advised_does_not_suppress_a_due_advisory() {
    let fixture = Fixture::new(REMOTE);
    let seeded = MaintenanceState {
        last_success: Some(NOW - RECONCILIATION_ADVISORY_AFTER_MS - 1),
        last_advised: Some(NOW + 10 * RECONCILIATION_ADVISORY_AFTER_MS),
        ..MaintenanceState::default()
    };
    let path = seed_state(&fixture, &seeded).await;

    assert!(matches!(
        advise_if_due(&fixture.root, || NOW),
        AdvisoryOutcome::Advised
    ));
    let StateRead::Valid(state) = read_state(&path).expect("state") else {
        panic!("expected valid state");
    };
    assert_eq!(state.last_advised, Some(NOW));
}
