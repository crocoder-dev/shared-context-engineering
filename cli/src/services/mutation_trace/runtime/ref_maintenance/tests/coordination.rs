use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc::unbounded_channel;

use super::super::super::coordinator::coordinate_inner;
use super::super::super::external_taint::ExternalTaintMarker;
use super::super::super::git_snapshot::resolve_git_dir;
use super::super::super::maintenance_state::{
    state_path, write_state_atomically, MaintenanceState, PersistPhase,
    RECONCILIATION_ADVISORY_AFTER_MS,
};
use super::super::super::protected_worktree::WORKTREE_LOCK_TIMEOUT;
use super::super::super::ref_advisory::{advise_if_due, advise_if_due_with, AdvisoryOutcome};
use super::super::super::ref_reconciliation::{ReconcilePhase, ReconciliationReport};
use super::super::super::worktree_lock::acquire_inner;
use super::super::ExplicitOutcome;
use super::support::{
    await_signal, close_payload, durable_snapshot, hook_ok, main_worktree, no_advice, pinned_trees,
    run_hook, sce_refs, spawn_parked_pass, start_payload, writer, WATCHDOG,
};
use super::{failing_write, pin_orphan, Fixture, NOW};
use crate::services::hooks::mutation_scope::MutationScopePayload;
use crate::services::mutation_trace::runtime::{advise_after_completed_boundary, RuntimeBoundary};
use crate::services::mutation_trace::types::ScopeStatus;

const REMOTE: &str = "https://example.invalid/org/repo-a.git";
const SCOPE: &str = "scope-1";

async fn git_dir_of(fixture: &Fixture) -> PathBuf {
    resolve_git_dir(&fixture.root).await.expect("git dir")
}

async fn started_fixture() -> Fixture {
    let fixture = Fixture::new(REMOTE);
    fixture.create_db().await;
    hook_ok(&fixture, start_payload(SCOPE, "start-1")).await;
    fixture
}

fn state_bytes(git_dir: &Path) -> Option<Vec<u8>> {
    std::fs::read(state_path(git_dir)).ok()
}

async fn assert_coordinate_blocked_until_pass_releases(phase: ReconcilePhase) {
    let fixture = started_fixture().await;
    let git_dir = git_dir_of(&fixture).await;
    let baseline = durable_snapshot(&fixture.db_path(), &main_worktree(), SCOPE).await;
    assert_eq!(baseline.roots.len(), 1);
    let cursor_tree = baseline.worktree.cursor_tree.0.clone();

    super::support::write_file(&fixture.root, "orphan.txt", "orphan\n");
    let orphan = pin_orphan(&fixture.root).await;
    super::support::write_file(&fixture.root, "next.txt", "next\n");

    let mut pass = spawn_parked_pass(&fixture, phase);
    await_signal(&mut pass.parked).await;
    let refs_while_parked = sce_refs(&fixture.root);
    assert_eq!(
        pinned_trees(&fixture.root, "main"),
        [cursor_tree.clone(), orphan.clone()].into_iter().collect()
    );

    let (contended_tx, mut contended_rx) = unbounded_channel::<()>();
    let db_opened = Arc::new(AtomicBool::new(false));
    let refs_at_load = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    let db_path = fixture.db_path();
    let root = fixture.root.clone();

    let coordinate = coordinate_inner(
        &root,
        &RuntimeBoundary::Flush,
        async || {
            db_opened.store(true, Ordering::SeqCst);
            writer(&db_path).await
        },
        WATCHDOG,
        move || {
            let _ = contended_tx.send(());
        },
        |_attempt| {
            refs_at_load
                .lock()
                .expect("refs at load")
                .push(sce_refs(&root));
        },
        |_attempt| Ok(()),
    );

    let marker = ExternalTaintMarker::new(&git_dir);
    let driver = async {
        await_signal(&mut contended_rx).await;
        assert!(
            !db_opened.load(Ordering::SeqCst),
            "the coordinator must not reach its protected section while the pass holds the lock"
        );
        assert!(
            !marker.exists().expect("marker"),
            "the coordinator must not arm its marker before it owns the lock"
        );
        assert_eq!(sce_refs(&fixture.root), refs_while_parked);
        assert_eq!(
            durable_snapshot(&fixture.db_path(), &main_worktree(), SCOPE).await,
            baseline
        );
        pass.finish().await
    };

    let (coordinated, outcome) = tokio::join!(coordinate, driver);

    let ExplicitOutcome::Completed(report) = outcome else {
        panic!("expected completed pass, got {outcome:?}");
    };
    assert_eq!(
        report,
        ReconciliationReport {
            local_required: 1,
            retained: 1,
            deleted: 1,
        }
    );
    let coordinated = coordinated.expect("the coordinator completes once the pass releases");
    let new_tree = coordinated.observed_tree.0.clone();
    assert_ne!(new_tree, cursor_tree);
    assert_ne!(new_tree, orphan);

    assert_eq!(
        pinned_trees(&fixture.root, "main"),
        [cursor_tree, new_tree.clone()].into_iter().collect(),
        "the pin created after the pass released the lock must survive; only the orphan is gone"
    );
    assert!(!refs_while_parked
        .iter()
        .any(|line| line.contains(&new_tree)));
    let seen_at_load = refs_at_load.lock().expect("refs at load").clone();
    assert_new_pin_preceded_the_durable_root(&fixture, &new_tree, &seen_at_load).await;

    drop(
        acquire_inner(&git_dir, Duration::ZERO, || {})
            .expect("neither operation leaves the lock held"),
    );
}

async fn assert_new_pin_preceded_the_durable_root(
    fixture: &Fixture,
    new_tree: &str,
    refs_at_load: &[Vec<String>],
) {
    assert_eq!(refs_at_load.len(), 1);
    assert!(
        refs_at_load[0].iter().any(|line| line.contains(new_tree)),
        "the new pin must already exist before the durable root commit"
    );
    let after = durable_snapshot(&fixture.db_path(), &main_worktree(), SCOPE).await;
    assert_eq!(after.worktree.cursor_tree.0, new_tree);
    assert!(after.roots.iter().any(|root| root.0 == new_tree));
    for root in &after.roots {
        assert!(
            pinned_trees(&fixture.root, "main").contains(&root.0),
            "every durable root stays reachable"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ref_reconciliation_real_coordinate_waits_for_a_pass_parked_after_db_open() {
    assert_coordinate_blocked_until_pass_releases(ReconcilePhase::DbOpened).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ref_reconciliation_real_coordinate_waits_for_a_pass_parked_after_pin_inventory() {
    assert_coordinate_blocked_until_pass_releases(ReconcilePhase::PinsInventoried).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ref_reconciliation_coordinate_lock_timeout_under_maintenance_commits_nothing_and_retries()
{
    let fixture = started_fixture().await;
    let git_dir = git_dir_of(&fixture).await;
    let marker = ExternalTaintMarker::new(&git_dir);
    super::support::write_file(&fixture.root, "work.txt", "work\n");
    let before = durable_snapshot(&fixture.db_path(), &main_worktree(), SCOPE).await;
    let refs_before = sce_refs(&fixture.root);
    assert_eq!(before.scope, Some(ScopeStatus::Active));

    let mut pass = spawn_parked_pass(&fixture, ReconcilePhase::DbOpened);
    await_signal(&mut pass.parked).await;

    let advisories = Arc::new(AtomicU32::new(0));
    let contended = Arc::new(AtomicU32::new(0));
    let failed = run_hook(
        &fixture.root,
        &fixture.db_path(),
        close_payload(SCOPE, "close-1"),
        Duration::from_millis(300),
        {
            let contended = Arc::clone(&contended);
            move || {
                contended.fetch_add(1, Ordering::SeqCst);
            }
        },
        || {},
        {
            let advisories = Arc::clone(&advisories);
            move |root| {
                advisories.fetch_add(1, Ordering::SeqCst);
                no_advice(root)
            }
        },
    )
    .await
    .expect_err("the lock cannot be acquired while the pass holds it");

    let message = format!("{failed:#}");
    assert!(
        message.contains("before durable completion")
            && message.contains("Timed out after")
            && message.contains("waiting for worktree lock"),
        "unexpected error: {message}"
    );
    assert_eq!(contended.load(Ordering::SeqCst), 1);
    assert_eq!(advisories.load(Ordering::SeqCst), 0);
    assert_eq!(
        durable_snapshot(&fixture.db_path(), &main_worktree(), SCOPE).await,
        before
    );
    assert_eq!(sce_refs(&fixture.root), refs_before);
    assert!(!marker.exists().expect("marker"));
    assert_eq!(state_bytes(&git_dir), None);

    let outcome = pass.finish().await;
    assert!(matches!(outcome, ExplicitOutcome::Completed(_)));
    let after_pass_state = state_bytes(&git_dir);

    let output = run_hook(
        &fixture.root,
        &fixture.db_path(),
        close_payload(SCOPE, "close-1"),
        WORKTREE_LOCK_TIMEOUT,
        || {},
        || {},
        {
            let advisories = Arc::clone(&advisories);
            move |root| {
                advisories.fetch_add(1, Ordering::SeqCst);
                no_advice(root)
            }
        },
    )
    .await
    .expect("the same boundary succeeds once the maintenance pass released the lock");
    assert_eq!(output, "");
    assert_eq!(advisories.load(Ordering::SeqCst), 1);
    assert_eq!(state_bytes(&git_dir), after_pass_state);

    let after = durable_snapshot(&fixture.db_path(), &main_worktree(), SCOPE).await;
    assert_eq!(after.scope, Some(ScopeStatus::Closed));
    assert_eq!(after.worktree.revision, before.worktree.revision + 1);
    assert_ne!(after.worktree.cursor_tree, before.worktree.cursor_tree);
    assert!(pinned_trees(&fixture.root, "main").contains(&after.worktree.cursor_tree.0));
    assert!(after.roots.contains(&after.worktree.cursor_tree));
    assert!(!marker.exists().expect("marker"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ref_reconciliation_flush_lock_timeout_under_maintenance_commits_nothing_and_retries() {
    let fixture = started_fixture().await;
    let git_dir = git_dir_of(&fixture).await;
    let marker = ExternalTaintMarker::new(&git_dir);
    super::support::write_file(&fixture.root, "work.txt", "work\n");
    let before = durable_snapshot(&fixture.db_path(), &main_worktree(), SCOPE).await;
    let refs_before = sce_refs(&fixture.root);

    let mut pass = spawn_parked_pass(&fixture, ReconcilePhase::DbOpened);
    await_signal(&mut pass.parked).await;

    let advisories = Arc::new(AtomicU32::new(0));
    let counting = |advisories: &Arc<AtomicU32>| {
        let advisories = Arc::clone(advisories);
        move |root: &Path| {
            advisories.fetch_add(1, Ordering::SeqCst);
            no_advice(root)
        }
    };
    let failed = run_hook(
        &fixture.root,
        &fixture.db_path(),
        MutationScopePayload::Flush,
        Duration::from_millis(300),
        || {},
        || {},
        counting(&advisories),
    )
    .await
    .expect_err("the lock cannot be acquired while the pass holds it");
    assert!(format!("{failed:#}").contains("Timed out after"));
    assert_eq!(advisories.load(Ordering::SeqCst), 0);
    assert_eq!(
        durable_snapshot(&fixture.db_path(), &main_worktree(), SCOPE).await,
        before
    );
    assert_eq!(sce_refs(&fixture.root), refs_before);
    assert!(!marker.exists().expect("marker"));
    assert_eq!(state_bytes(&git_dir), None);

    assert!(matches!(pass.finish().await, ExplicitOutcome::Completed(_)));

    let output = run_hook(
        &fixture.root,
        &fixture.db_path(),
        MutationScopePayload::Flush,
        WORKTREE_LOCK_TIMEOUT,
        || {},
        || {},
        counting(&advisories),
    )
    .await
    .expect("the same flush succeeds once the maintenance pass released the lock");
    assert_eq!(output, "");
    assert_eq!(advisories.load(Ordering::SeqCst), 1);
    let after = durable_snapshot(&fixture.db_path(), &main_worktree(), SCOPE).await;
    assert_eq!(after.worktree.revision, before.worktree.revision + 1);
    assert_ne!(after.worktree.cursor_tree, before.worktree.cursor_tree);
    assert!(pinned_trees(&fixture.root, "main").contains(&after.worktree.cursor_tree.0));
    assert!(!marker.exists().expect("marker"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ref_reconciliation_advisory_runs_after_the_real_coordinator_released_its_lock() {
    let fixture = started_fixture().await;
    let git_dir = git_dir_of(&fixture).await;
    let marker = ExternalTaintMarker::new(&git_dir);
    super::support::write_file(&fixture.root, "work.txt", "work\n");
    let before = durable_snapshot(&fixture.db_path(), &main_worktree(), SCOPE).await;

    let at_advisory = Arc::new(Mutex::new(None));
    let refs_around = Arc::new(Mutex::new(None));
    let db_path = fixture.db_path();
    let advise = {
        let at_advisory = Arc::clone(&at_advisory);
        let refs_around = Arc::clone(&refs_around);
        let marker = marker.clone();
        let git_dir = git_dir.clone();
        move |root: &Path| {
            drop(
                acquire_inner(&git_dir, Duration::ZERO, || {})
                    .expect("the coordinator lock is released before the advisory runs"),
            );
            *at_advisory.lock().expect("snapshot slot") = Some(tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(durable_snapshot(
                    &db_path,
                    &main_worktree(),
                    SCOPE,
                ))
            }));
            let marker_before = marker.exists().expect("marker");
            let refs_before = sce_refs(root);
            let report = advise_after_completed_boundary(root);
            *refs_around.lock().expect("refs slot") = Some((refs_before, sce_refs(root)));
            assert_eq!(
                marker_before,
                marker.exists().expect("marker"),
                "the advisory never creates, clears or modifies the marker"
            );
            assert!(!marker_before);
            report
        }
    };

    let output = run_hook(
        &fixture.root,
        &fixture.db_path(),
        close_payload(SCOPE, "close-1"),
        WORKTREE_LOCK_TIMEOUT,
        || {},
        || {},
        advise,
    )
    .await
    .expect("hook succeeds");

    assert_eq!(output, "");
    let committed = at_advisory
        .lock()
        .expect("snapshot slot")
        .clone()
        .expect("advisory callback ran");
    assert_eq!(committed.scope, Some(ScopeStatus::Closed));
    assert_eq!(committed.worktree.revision, before.worktree.revision + 1);
    assert_ne!(committed.worktree.cursor_tree, before.worktree.cursor_tree);
    assert_eq!(
        durable_snapshot(&fixture.db_path(), &main_worktree(), SCOPE).await,
        committed,
        "the advisory must not change the committed result"
    );
    let (refs_before, refs_after) = refs_around
        .lock()
        .expect("refs slot")
        .clone()
        .expect("refs recorded");
    assert_eq!(refs_before, refs_after, "the advisory never touches refs");
    assert!(state_bytes(&git_dir).is_some());
    assert!(!marker.exists().expect("marker"));
    drop(
        acquire_inner(&git_dir, Duration::ZERO, || {})
            .expect("no lock is held after the advisory returns"),
    );
}

fn marker_fingerprint(path: &Path) -> (bool, Vec<String>) {
    let metadata = std::fs::symlink_metadata(path).expect("marker path exists");
    let mut children: Vec<String> = if metadata.is_dir() {
        std::fs::read_dir(path)
            .expect("read marker dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    } else {
        Vec::new()
    };
    children.sort();
    (metadata.is_dir(), children)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ref_reconciliation_marker_clear_failure_after_commit_keeps_taint_for_the_next_coordinate()
{
    let fixture = started_fixture().await;
    let git_dir = git_dir_of(&fixture).await;
    let marker = ExternalTaintMarker::new(&git_dir);
    let marker_path = git_dir.join("sce").join("mutation-cursor-tainted");
    let saved_path = git_dir.join("sce").join("mutation-cursor-tainted.saved");
    assert!(!marker.exists().expect("marker"));

    super::support::write_file(&fixture.root, "one.txt", "one\n");
    hook_ok(
        &fixture,
        super::support::start_payload("scope-2", "start-2"),
    )
    .await;
    let control = durable_snapshot(&fixture.db_path(), &main_worktree(), SCOPE).await;
    assert_eq!(control.scope, Some(ScopeStatus::Active));
    assert!(!marker.exists().expect("marker"));

    super::support::write_file(&fixture.root, "two.txt", "two\n");
    let before_fault = durable_snapshot(&fixture.db_path(), &main_worktree(), SCOPE).await;
    let armed_len = Arc::new(Mutex::new(None));
    let fingerprints = Arc::new(Mutex::new(None));
    let output = run_hook(
        &fixture.root,
        &fixture.db_path(),
        close_payload("scope-2", "close-2"),
        WORKTREE_LOCK_TIMEOUT,
        || {},
        {
            let marker_path = marker_path.clone();
            let saved_path = saved_path.clone();
            let armed_len = Arc::clone(&armed_len);
            move || {
                *armed_len.lock().expect("len slot") =
                    Some(std::fs::metadata(&marker_path).expect("armed marker").len());
                std::fs::rename(&marker_path, &saved_path).expect("set armed marker aside");
                std::fs::create_dir(&marker_path).expect("block unlink with a directory");
            }
        },
        {
            let marker_path = marker_path.clone();
            let fingerprints = Arc::clone(&fingerprints);
            move |root| {
                let before = marker_fingerprint(&marker_path);
                let refs_before = sce_refs(root);
                let report = advise_after_completed_boundary(root);
                let after = marker_fingerprint(&marker_path);
                assert_eq!(refs_before, sce_refs(root));
                *fingerprints.lock().expect("fingerprint slot") = Some((before, after));
                report
            }
        },
    )
    .await
    .expect("MarkerClearAfterCommit is a durable success for the hook");
    assert_eq!(output, "");

    let (fingerprint_before, fingerprint_after) = fingerprints
        .lock()
        .expect("fingerprint slot")
        .clone()
        .expect("advisory ran after the failed marker clear");
    assert_eq!(fingerprint_before, fingerprint_after);
    assert!(fingerprint_before.0, "the injected fault is still in place");

    let after_fault = durable_snapshot(&fixture.db_path(), &main_worktree(), SCOPE).await;
    assert_eq!(
        after_fault.worktree.revision,
        before_fault.worktree.revision + 1
    );
    assert_eq!(after_fault.scope, Some(ScopeStatus::Active));

    std::fs::remove_dir(&marker_path).expect("remove injected fault");
    std::fs::rename(&saved_path, &marker_path).expect("restore the armed marker");
    assert!(marker.exists().expect("marker"));
    assert_eq!(
        std::fs::metadata(&marker_path)
            .expect("restored marker")
            .len(),
        armed_len
            .lock()
            .expect("len slot")
            .expect("length recorded")
    );

    super::support::write_file(&fixture.root, "three.txt", "three\n");
    hook_ok(&fixture, close_payload("scope-2", "close-3")).await;

    let recovered = durable_snapshot(&fixture.db_path(), &main_worktree(), SCOPE).await;
    assert_eq!(
        recovered.scope,
        Some(ScopeStatus::Abandoned),
        "inherited taint conservatively abandons the live scope"
    );
    assert!(recovered.worktree.revision >= after_fault.worktree.revision + 2);
    assert!(!recovered.worktree.tainted);
    assert!(!recovered.worktree.needs_rebaseline);
    assert!(pinned_trees(&fixture.root, "main").contains(&recovered.worktree.cursor_tree.0));
    assert!(!marker.exists().expect("marker"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ref_reconciliation_advisory_after_a_real_commit_is_busy_when_a_pass_wins_the_handoff() {
    let fixture = started_fixture().await;
    let git_dir = git_dir_of(&fixture).await;
    super::support::write_file(&fixture.root, "orphan.txt", "orphan\n");
    let orphan = pin_orphan(&fixture.root).await;
    super::support::write_file(&fixture.root, "work.txt", "work\n");

    let (handoff_tx, mut handoff_rx) = unbounded_channel::<()>();
    let (holding_tx, holding_rx) = std::sync::mpsc::channel::<()>();
    let (advisory_done_tx, mut advisory_done_rx) = unbounded_channel::<()>();

    let orchestrator = {
        let root = fixture.root.clone();
        let state_root = fixture.state_root.clone();
        tokio::spawn(async move {
            await_signal(&mut handoff_rx).await;
            let mut pass = spawn_parked_pass_for(&root, &state_root);
            await_signal(&mut pass.parked).await;
            holding_tx.send(()).expect("resume the advisory");
            await_signal(&mut advisory_done_rx).await;
            pass.finish().await
        })
    };

    let observed = Arc::new(Mutex::new(None));
    let advise = {
        let observed = Arc::clone(&observed);
        let git_dir = git_dir.clone();
        move |root: &Path| {
            handoff_tx.send(()).expect("announce the handoff point");
            holding_rx
                .recv_timeout(WATCHDOG)
                .expect("the pass takes the lock");
            let state_before = state_bytes(&git_dir);
            let refs_before = sce_refs(root);
            let report = advise_after_completed_boundary(root);
            *observed.lock().expect("observed slot") = Some((
                state_before,
                state_bytes(&git_dir),
                refs_before,
                sce_refs(root),
                report.outcome,
            ));
            advisory_done_tx.send(()).expect("advisory finished");
            report
        }
    };

    let output = run_hook(
        &fixture.root,
        &fixture.db_path(),
        close_payload(SCOPE, "close-1"),
        WORKTREE_LOCK_TIMEOUT,
        || {},
        || {},
        advise,
    )
    .await
    .expect("the committed boundary still reports success");
    assert_eq!(output, "");

    let (state_before, state_after, refs_before, refs_after, outcome) = observed
        .lock()
        .expect("observed slot")
        .clone()
        .expect("advisory ran");
    assert_eq!(outcome, "busy");
    assert_eq!(state_before, state_after);
    assert_eq!(refs_before, refs_after);

    let pass_outcome = orchestrator.await.expect("orchestrator joins");
    let ExplicitOutcome::Completed(report) = pass_outcome else {
        panic!("expected completed pass, got {pass_outcome:?}");
    };
    assert_eq!(report.deleted, 1);
    let committed = durable_snapshot(&fixture.db_path(), &main_worktree(), SCOPE).await;
    assert_eq!(committed.scope, Some(ScopeStatus::Closed));
    let pins = pinned_trees(&fixture.root, "main");
    assert!(!pins.contains(&orphan));
    assert!(pins.contains(&committed.worktree.cursor_tree.0));
}

fn spawn_parked_pass_for(root: &Path, state_root: &Path) -> super::support::ParkedPass {
    super::support::spawn_parked_pass_at(root, state_root, ReconcilePhase::DbOpened)
}

async fn advisory_from_independent_task(root: &Path, now: i64) -> AdvisoryOutcome {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || advise_if_due(&root, || now))
        .await
        .expect("advisory task joins")
}

async fn assert_independent_advisory_is_busy_while_parked(phase: ReconcilePhase) {
    let fixture = Fixture::new(REMOTE);
    fixture.create_db().await;
    let orphan = pin_orphan(&fixture.root).await;
    let git_dir = git_dir_of(&fixture).await;

    let mut pass = spawn_parked_pass(&fixture, phase);
    await_signal(&mut pass.parked).await;
    let refs_while_parked = sce_refs(&fixture.root);
    let state_while_parked = state_bytes(&git_dir);

    let advisory =
        tokio::time::timeout(WATCHDOG, advisory_from_independent_task(&fixture.root, NOW))
            .await
            .expect("the advisory must not wait for the pass");
    assert!(matches!(advisory, AdvisoryOutcome::Busy));
    assert_eq!(state_bytes(&git_dir), state_while_parked);
    assert_eq!(sce_refs(&fixture.root), refs_while_parked);
    assert!(pinned_trees(&fixture.root, "main").contains(&orphan));

    let outcome = pass.finish().await;
    let ExplicitOutcome::Completed(report) = outcome else {
        panic!("expected completed pass, got {outcome:?}");
    };
    assert_eq!(report.deleted, 1);
    assert!(pinned_trees(&fixture.root, "main").is_empty());

    let recorded = state_bytes(&git_dir).expect("pass recorded state");
    assert!(matches!(
        advisory_from_independent_task(&fixture.root, NOW).await,
        AdvisoryOutcome::NoAction
    ));
    assert_eq!(state_bytes(&git_dir), Some(recorded));
    assert!(matches!(
        advisory_from_independent_task(&fixture.root, NOW + RECONCILIATION_ADVISORY_AFTER_MS + 1)
            .await,
        AdvisoryOutcome::Advised
    ));
    let stored: MaintenanceState =
        match super::super::super::maintenance_state::read_state(&state_path(&git_dir))
            .expect("state")
        {
            super::super::super::maintenance_state::StateRead::Valid(state) => state,
            other => panic!("expected valid state, got {other:?}"),
        };
    assert_eq!(stored.last_success, Some(NOW));
    assert_eq!(
        stored.last_advised,
        Some(NOW + RECONCILIATION_ADVISORY_AFTER_MS + 1)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ref_reconciliation_independent_advisory_task_is_busy_while_a_pass_is_parked_after_db_open()
{
    assert_independent_advisory_is_busy_while_parked(ReconcilePhase::DbOpened).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ref_reconciliation_independent_advisory_task_is_busy_while_a_pass_is_parked_after_pin_inventory(
) {
    assert_independent_advisory_is_busy_while_parked(ReconcilePhase::PinsInventoried).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ref_reconciliation_advisory_failures_after_a_real_commit_never_change_the_hook_result() {
    for (case, expected) in [
        ("write_failed", "state_write_failed"),
        ("corrupt_state", "anchored"),
        ("unavailable_state", "state_unavailable"),
    ] {
        let fixture = started_fixture().await;
        let git_dir = git_dir_of(&fixture).await;
        let marker = ExternalTaintMarker::new(&git_dir);
        let path = state_path(&git_dir);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("runtime dir");
        match case {
            "corrupt_state" => std::fs::write(&path, b"not json").expect("corrupt state"),
            "unavailable_state" => std::fs::create_dir(&path).expect("directory as state"),
            _ => {}
        }
        super::support::write_file(&fixture.root, "work.txt", "work\n");

        let observed = Arc::new(Mutex::new(None));
        let db_path = fixture.db_path();
        let advise = {
            let observed = Arc::clone(&observed);
            let marker = marker.clone();
            move |root: &Path| {
                let committed = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(durable_snapshot(
                        &db_path,
                        &main_worktree(),
                        SCOPE,
                    ))
                });
                let refs_before = sce_refs(root);
                let marker_before = marker.exists().expect("marker");
                let outcome = if case == "write_failed" {
                    advise_if_due_with(root, || NOW, failing_write(PersistPhase::Rename))
                } else {
                    advise_if_due_with(root, || NOW, write_state_atomically)
                };
                let report = outcome.report();
                *observed.lock().expect("observed slot") = Some((
                    committed,
                    refs_before == sce_refs(root),
                    marker_before == marker.exists().expect("marker"),
                    report.outcome,
                ));
                report
            }
        };

        let output = run_hook(
            &fixture.root,
            &fixture.db_path(),
            close_payload(SCOPE, "close-1"),
            WORKTREE_LOCK_TIMEOUT,
            || {},
            || {},
            advise,
        )
        .await
        .unwrap_or_else(|error| panic!("{case}: hook must succeed, got {error:#}"));

        assert_eq!(output, "", "{case}");
        let (committed, refs_unchanged, marker_unchanged, outcome) = observed
            .lock()
            .expect("observed slot")
            .clone()
            .expect("advisory ran");
        assert_eq!(outcome, expected, "{case}");
        assert!(refs_unchanged, "{case}");
        assert!(marker_unchanged, "{case}");
        assert_eq!(committed.scope, Some(ScopeStatus::Closed), "{case}");
        assert_eq!(
            durable_snapshot(&fixture.db_path(), &main_worktree(), SCOPE).await,
            committed,
            "{case}"
        );
        assert!(!marker.exists().expect("marker"), "{case}");
    }
}
