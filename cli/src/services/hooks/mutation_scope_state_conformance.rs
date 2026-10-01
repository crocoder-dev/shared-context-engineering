use std::fmt::Debug;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Barrier;
use std::thread;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

const SCE_STATE_DIR: &str = "sce";
const SEEDED_GENERATION: u64 = 7;
const SEEDED_NEXT_GENERATION: u64 = SEEDED_GENERATION + 3;
const UNKNOWN_SCOPE_ID: &str = "state-conformance-unknown-scope";
const INJECTED_INTERRUPTION: &str = "injected interruption before rename";
const FLUSH_CLAIM_CONTENDER_COUNT: usize = 4;
const PARALLEL_REMOVAL_COUNT: usize = 8;

static NEXT_CONFORMANCE_GIT_DIR_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveryView {
    Clear,
    Pending(u64),
    Flushing(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompletionView {
    Cleared,
    Superseded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FlushClaimView {
    Claimed(u64),
    Blocked,
    Other(String),
}

pub(crate) trait StateConformance {
    type State: Clone + Debug + PartialEq;

    const ADAPTER: &'static str;
    const SUPPORTED_VERSION: u32;

    fn state_path(git_dir: &Path) -> PathBuf;

    fn default_state() -> Self::State;

    fn fixture_state(
        recovery: RecoveryView,
        next_recovery_generation: u64,
        attempt_count: usize,
    ) -> Self::State;

    fn read_state(git_dir: &Path) -> Result<Self::State>;

    fn persist(git_dir: &Path, state: &Self::State) -> Result<()>;

    fn persist_with_before_rename_hook<F>(
        git_dir: &Path,
        state: &Self::State,
        before_rename: F,
    ) -> Result<()>
    where
        F: FnOnce(&Path, &Path) -> Result<()>;

    fn recovery_view(state: &Self::State) -> RecoveryView;

    fn next_recovery_generation(state: &Self::State) -> u64;

    fn scope_ids(state: &Self::State) -> Vec<String>;

    fn remove_attempt(git_dir: &Path, scope_id: &str) -> Result<()>;

    fn normalize_recovery_after_boundary_lock_acquired(git_dir: &Path) -> Result<()>;

    fn complete_recovery_flush(git_dir: &Path, generation: u64) -> Result<CompletionView>;

    fn relinquish_recovery_flush(git_dir: &Path, generation: u64) -> Result<()>;

    fn claim_flush_by_admitting_a_new_attempt(
        git_dir: &Path,
        contender: usize,
    ) -> Result<FlushClaimView>;
}

struct ConformanceGitDir(PathBuf);

impl ConformanceGitDir {
    fn create<A: StateConformance>(contract: &str) -> Self {
        let id = NEXT_CONFORMANCE_GIT_DIR_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sce-{}-mutation-scope-state-conformance-{contract}-{}-{id}",
            A::ADAPTER,
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("git dir should be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ConformanceGitDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn seed<A: StateConformance>(git_dir: &Path, state: &A::State) {
    A::persist(git_dir, state).expect("seeding the canonical state should succeed");
}

fn reload<A: StateConformance>(git_dir: &Path) -> A::State {
    A::read_state(git_dir).expect("persisted state should be readable")
}

fn canonical_bytes<A: StateConformance>(git_dir: &Path) -> Vec<u8> {
    std::fs::read(A::state_path(git_dir)).expect("canonical state file should be readable")
}

fn overwrite_canonical_bytes<A: StateConformance>(git_dir: &Path, bytes: &[u8]) {
    let path = A::state_path(git_dir);
    std::fs::create_dir_all(path.parent().expect("state path should have a parent"))
        .expect("state dir should be created");
    std::fs::write(path, bytes).expect("canonical state file should be written");
}

fn assert_every_transaction_fails_closed<A: StateConformance>(git_dir: &Path, rejected: &[u8]) {
    assert!(
        A::remove_attempt(git_dir, UNKNOWN_SCOPE_ID).is_err(),
        "{}: remove_attempt must not proceed over a rejected state file",
        A::ADAPTER
    );
    assert!(
        A::normalize_recovery_after_boundary_lock_acquired(git_dir).is_err(),
        "{}: normalize must not proceed over a rejected state file",
        A::ADAPTER
    );
    assert!(
        A::complete_recovery_flush(git_dir, SEEDED_GENERATION).is_err(),
        "{}: complete_recovery_flush must not proceed over a rejected state file",
        A::ADAPTER
    );
    assert!(
        A::relinquish_recovery_flush(git_dir, SEEDED_GENERATION).is_err(),
        "{}: relinquish_recovery_flush must not proceed over a rejected state file",
        A::ADAPTER
    );
    assert!(
        A::claim_flush_by_admitting_a_new_attempt(git_dir, 0).is_err(),
        "{}: admission must not proceed over a rejected state file",
        A::ADAPTER
    );
    assert_eq!(
        canonical_bytes::<A>(git_dir),
        rejected,
        "{}: a rejected state file must never be replaced by fabricated bookkeeping",
        A::ADAPTER
    );
}

pub(crate) fn missing_state_file_reads_as_default_without_fabricating_a_file<
    A: StateConformance,
>() {
    let git_dir = ConformanceGitDir::create::<A>("missing-state");

    let state =
        A::read_state(git_dir.path()).expect("a missing state file must read as the default");

    assert_eq!(state, A::default_state());
    assert_eq!(A::recovery_view(&state), RecoveryView::Clear);
    assert_eq!(A::next_recovery_generation(&state), 1);
    assert!(A::scope_ids(&state).is_empty());
    assert!(!A::state_path(git_dir.path()).exists());
    assert!(!git_dir.path().join(SCE_STATE_DIR).exists());
}

pub(crate) fn state_file_is_checkout_local_below_git_dir_sce<A: StateConformance>() {
    let git_dir_a = ConformanceGitDir::create::<A>("checkout-local-a");
    let git_dir_b = ConformanceGitDir::create::<A>("checkout-local-b");
    let state_path_a = A::state_path(git_dir_a.path());
    let state_path_b = A::state_path(git_dir_b.path());

    assert!(state_path_a.starts_with(git_dir_a.path().join(SCE_STATE_DIR)));
    assert!(state_path_b.starts_with(git_dir_b.path().join(SCE_STATE_DIR)));
    assert_eq!(state_path_a.file_name(), state_path_b.file_name());

    let seeded = A::fixture_state(
        RecoveryView::Pending(SEEDED_GENERATION),
        SEEDED_NEXT_GENERATION,
        2,
    );
    seed::<A>(git_dir_a.path(), &seeded);

    assert!(state_path_a.is_file());
    assert_eq!(reload::<A>(git_dir_a.path()), seeded);
    assert!(!state_path_b.exists());
    assert_eq!(reload::<A>(git_dir_b.path()), A::default_state());
}

pub(crate) fn state_round_trips_durably_through_the_canonical_path<A: StateConformance>() {
    let git_dir = ConformanceGitDir::create::<A>("round-trip");
    let attempt_count = 3;

    for recovery in [
        RecoveryView::Flushing(SEEDED_GENERATION),
        RecoveryView::Pending(SEEDED_GENERATION),
        RecoveryView::Clear,
    ] {
        let state = A::fixture_state(recovery, SEEDED_NEXT_GENERATION, attempt_count);
        let mut distinct_scope_ids = A::scope_ids(&state);
        distinct_scope_ids.sort_unstable();
        distinct_scope_ids.dedup();
        assert_eq!(distinct_scope_ids.len(), attempt_count);

        A::persist(git_dir.path(), &state).expect("durable write should succeed");

        assert!(A::state_path(git_dir.path()).is_file());
        let on_disk: Value = serde_json::from_slice(&canonical_bytes::<A>(git_dir.path()))
            .expect("canonical state file should hold valid JSON");
        assert_eq!(on_disk["version"], json!(A::SUPPORTED_VERSION));

        let reloaded = reload::<A>(git_dir.path());
        assert_eq!(reloaded, state);
        assert_eq!(A::recovery_view(&reloaded), recovery);
        assert_eq!(
            A::next_recovery_generation(&reloaded),
            SEEDED_NEXT_GENERATION
        );
        assert_eq!(A::scope_ids(&reloaded), A::scope_ids(&state));
    }
}

pub(crate) fn malformed_state_file_is_rejected_without_fabricating_bookkeeping<
    A: StateConformance,
>() {
    for payload in ["not json", "", "{}", "{\"version\":"] {
        let git_dir = ConformanceGitDir::create::<A>("malformed-state");
        overwrite_canonical_bytes::<A>(git_dir.path(), payload.as_bytes());

        let error = A::read_state(git_dir.path())
            .expect_err("a malformed state file must be rejected, never read as default");
        assert!(
            error.to_string().contains("malformed"),
            "{}: payload {payload:?} produced {error}",
            A::ADAPTER
        );

        assert_every_transaction_fails_closed::<A>(git_dir.path(), payload.as_bytes());
    }
}

pub(crate) fn a_state_file_with_any_other_version_is_rejected<A: StateConformance>() {
    let wrong_versions = (0..A::SUPPORTED_VERSION).chain([A::SUPPORTED_VERSION + 1, 99]);

    for version in wrong_versions {
        let git_dir = ConformanceGitDir::create::<A>("unsupported-version");
        seed::<A>(
            git_dir.path(),
            &A::fixture_state(
                RecoveryView::Pending(SEEDED_GENERATION),
                SEEDED_NEXT_GENERATION,
                2,
            ),
        );

        let mut payload: Value = serde_json::from_slice(&canonical_bytes::<A>(git_dir.path()))
            .expect("canonical state file should hold valid JSON");
        assert_eq!(payload["version"], json!(A::SUPPORTED_VERSION));
        payload["version"] = json!(version);
        let rejected = serde_json::to_vec(&payload).expect("payload should serialize");
        overwrite_canonical_bytes::<A>(git_dir.path(), &rejected);

        let error = A::read_state(git_dir.path())
            .expect_err("a state file with another version must be rejected, never migrated");
        assert!(
            error.to_string().contains("unsupported version"),
            "{}: version {version} produced {error}",
            A::ADAPTER
        );

        assert_every_transaction_fails_closed::<A>(git_dir.path(), &rejected);
    }
}

pub(crate) fn interruption_before_rename_leaves_the_canonical_path_unaffected<
    A: StateConformance,
>() {
    let git_dir = ConformanceGitDir::create::<A>("interrupted-before-rename");
    let canonical_path = A::state_path(git_dir.path());
    let established = A::fixture_state(
        RecoveryView::Pending(SEEDED_GENERATION),
        SEEDED_NEXT_GENERATION,
        1,
    );
    let replacement = A::fixture_state(
        RecoveryView::Flushing(SEEDED_GENERATION + 1),
        SEEDED_NEXT_GENERATION + 1,
        3,
    );

    let error = A::persist_with_before_rename_hook(
        git_dir.path(),
        &established,
        |tmp_path, hook_canonical_path| {
            assert!(tmp_path.is_file());
            assert_ne!(tmp_path, hook_canonical_path);
            assert_eq!(hook_canonical_path, canonical_path);
            assert!(!hook_canonical_path.exists());
            Err(anyhow!(INJECTED_INTERRUPTION))
        },
    )
    .expect_err("an interrupted first write must fail");
    assert!(error.to_string().contains(INJECTED_INTERRUPTION));
    assert!(!canonical_path.exists());
    assert_eq!(reload::<A>(git_dir.path()), A::default_state());

    seed::<A>(git_dir.path(), &established);
    let established_bytes = canonical_bytes::<A>(git_dir.path());

    let error = A::persist_with_before_rename_hook(
        git_dir.path(),
        &replacement,
        |tmp_path, hook_canonical_path| {
            assert!(tmp_path.is_file());
            assert_ne!(tmp_path, hook_canonical_path);
            assert_eq!(
                std::fs::read(hook_canonical_path)?,
                established_bytes,
                "the canonical file must not be written before the rename"
            );
            Err(anyhow!(INJECTED_INTERRUPTION))
        },
    )
    .expect_err("an interrupted replacement must fail");
    assert!(error.to_string().contains(INJECTED_INTERRUPTION));
    assert_eq!(canonical_bytes::<A>(git_dir.path()), established_bytes);
    assert_eq!(reload::<A>(git_dir.path()), established);

    A::persist(git_dir.path(), &replacement)
        .expect("a later durable write must succeed after an interrupted one");
    assert_eq!(reload::<A>(git_dir.path()), replacement);
}

pub(crate) fn removing_an_unknown_attempt_is_a_safe_no_op<A: StateConformance>() {
    let git_dir = ConformanceGitDir::create::<A>("remove-unknown");

    A::remove_attempt(git_dir.path(), UNKNOWN_SCOPE_ID)
        .expect("removing an unknown scope from absent state must succeed");
    assert!(!A::state_path(git_dir.path()).exists());

    let seeded = A::fixture_state(
        RecoveryView::Pending(SEEDED_GENERATION),
        SEEDED_NEXT_GENERATION,
        2,
    );
    seed::<A>(git_dir.path(), &seeded);
    let seeded_bytes = canonical_bytes::<A>(git_dir.path());

    A::remove_attempt(git_dir.path(), UNKNOWN_SCOPE_ID)
        .expect("removing an unknown scope from populated state must succeed");
    assert_eq!(canonical_bytes::<A>(git_dir.path()), seeded_bytes);
    assert_eq!(reload::<A>(git_dir.path()), seeded);
}

pub(crate) fn removing_an_already_removed_attempt_is_a_safe_no_op<A: StateConformance>() {
    let git_dir = ConformanceGitDir::create::<A>("remove-idempotent");
    let seeded = A::fixture_state(
        RecoveryView::Flushing(SEEDED_GENERATION),
        SEEDED_NEXT_GENERATION,
        3,
    );
    seed::<A>(git_dir.path(), &seeded);
    let scope_ids = A::scope_ids(&seeded);

    A::remove_attempt(git_dir.path(), &scope_ids[1]).expect("first removal should succeed");

    let after_removal = reload::<A>(git_dir.path());
    assert_eq!(
        A::scope_ids(&after_removal),
        vec![scope_ids[0].clone(), scope_ids[2].clone()]
    );
    assert_eq!(
        A::recovery_view(&after_removal),
        RecoveryView::Flushing(SEEDED_GENERATION)
    );
    assert_eq!(
        A::next_recovery_generation(&after_removal),
        SEEDED_NEXT_GENERATION
    );
    let bytes_after_removal = canonical_bytes::<A>(git_dir.path());

    A::remove_attempt(git_dir.path(), &scope_ids[1])
        .expect("duplicate terminal delivery after cleanup must be a safe no-op");
    assert_eq!(canonical_bytes::<A>(git_dir.path()), bytes_after_removal);
    assert_eq!(reload::<A>(git_dir.path()), after_removal);
}

pub(crate) fn normalize_after_boundary_lock_reclaims_orphaned_flushing_to_pending_same_generation<
    A: StateConformance,
>() {
    let git_dir = ConformanceGitDir::create::<A>("normalize-orphaned-flushing");
    let seeded = A::fixture_state(
        RecoveryView::Flushing(SEEDED_GENERATION),
        SEEDED_NEXT_GENERATION,
        2,
    );
    seed::<A>(git_dir.path(), &seeded);

    A::normalize_recovery_after_boundary_lock_acquired(git_dir.path())
        .expect("normalize should succeed");

    let normalized = reload::<A>(git_dir.path());
    assert_eq!(
        A::recovery_view(&normalized),
        RecoveryView::Pending(SEEDED_GENERATION)
    );
    assert_eq!(
        A::next_recovery_generation(&normalized),
        SEEDED_NEXT_GENERATION
    );
    assert_eq!(A::scope_ids(&normalized), A::scope_ids(&seeded));
}

pub(crate) fn complete_recovery_flush_clears_only_with_the_matching_generation<
    A: StateConformance,
>() {
    let git_dir = ConformanceGitDir::create::<A>("complete-matching-generation");
    let seeded = A::fixture_state(
        RecoveryView::Flushing(SEEDED_GENERATION),
        SEEDED_NEXT_GENERATION,
        2,
    );
    seed::<A>(git_dir.path(), &seeded);
    let seeded_bytes = canonical_bytes::<A>(git_dir.path());

    for other_generation in [
        SEEDED_GENERATION - 1,
        SEEDED_GENERATION + 1,
        SEEDED_NEXT_GENERATION,
    ] {
        assert_eq!(
            A::complete_recovery_flush(git_dir.path(), other_generation)
                .expect("wrong-generation completion should return"),
            CompletionView::Superseded,
        );
        assert_eq!(canonical_bytes::<A>(git_dir.path()), seeded_bytes);
    }
    assert_eq!(reload::<A>(git_dir.path()), seeded);

    assert_eq!(
        A::complete_recovery_flush(git_dir.path(), SEEDED_GENERATION)
            .expect("matching completion should return"),
        CompletionView::Cleared,
    );

    let completed = reload::<A>(git_dir.path());
    assert_eq!(A::recovery_view(&completed), RecoveryView::Clear);
    assert_eq!(
        A::next_recovery_generation(&completed),
        SEEDED_NEXT_GENERATION
    );
    assert_eq!(A::scope_ids(&completed), A::scope_ids(&seeded));
}

pub(crate) fn relinquish_recovery_flush_returns_only_the_claimed_generation_to_pending<
    A: StateConformance,
>() {
    let git_dir = ConformanceGitDir::create::<A>("relinquish-to-pending");
    let seeded = A::fixture_state(
        RecoveryView::Flushing(SEEDED_GENERATION),
        SEEDED_NEXT_GENERATION,
        2,
    );
    seed::<A>(git_dir.path(), &seeded);
    let seeded_bytes = canonical_bytes::<A>(git_dir.path());

    for other_generation in [SEEDED_GENERATION - 1, SEEDED_GENERATION + 1] {
        A::relinquish_recovery_flush(git_dir.path(), other_generation)
            .expect("wrong-generation relinquish should return");
        assert_eq!(canonical_bytes::<A>(git_dir.path()), seeded_bytes);
    }
    assert_eq!(reload::<A>(git_dir.path()), seeded);

    A::relinquish_recovery_flush(git_dir.path(), SEEDED_GENERATION)
        .expect("relinquish should succeed");

    let relinquished = reload::<A>(git_dir.path());
    assert_eq!(
        A::recovery_view(&relinquished),
        RecoveryView::Pending(SEEDED_GENERATION)
    );
    assert_eq!(
        A::next_recovery_generation(&relinquished),
        SEEDED_NEXT_GENERATION
    );
    assert_eq!(A::scope_ids(&relinquished), A::scope_ids(&seeded));
    let relinquished_bytes = canonical_bytes::<A>(git_dir.path());

    A::relinquish_recovery_flush(git_dir.path(), SEEDED_GENERATION)
        .expect("a second relinquish should return");
    assert_eq!(canonical_bytes::<A>(git_dir.path()), relinquished_bytes);
}

pub(crate) fn only_one_concurrent_caller_claims_the_flush_for_a_generation<A: StateConformance>() {
    let git_dir = ConformanceGitDir::create::<A>("one-flush-owner");
    seed::<A>(
        git_dir.path(),
        &A::fixture_state(
            RecoveryView::Pending(SEEDED_GENERATION),
            SEEDED_NEXT_GENERATION,
            0,
        ),
    );
    let start = Barrier::new(FLUSH_CLAIM_CONTENDER_COUNT);

    let claims: Vec<FlushClaimView> = thread::scope(|scope| {
        let handles: Vec<_> = (0..FLUSH_CLAIM_CONTENDER_COUNT)
            .map(|contender| {
                let start = &start;
                let git_dir = git_dir.path();
                scope.spawn(move || {
                    start.wait();
                    A::claim_flush_by_admitting_a_new_attempt(git_dir, contender)
                        .expect("a contending admission should not error")
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("contender should not panic"))
            .collect()
    });

    let claimed = claims
        .iter()
        .filter(|claim| **claim == FlushClaimView::Claimed(SEEDED_GENERATION))
        .count();
    let blocked = claims
        .iter()
        .filter(|claim| **claim == FlushClaimView::Blocked)
        .count();
    assert_eq!(
        claimed,
        1,
        "{}: exactly one caller may claim Flushing(g), got {claims:?}",
        A::ADAPTER
    );
    assert_eq!(
        blocked,
        FLUSH_CLAIM_CONTENDER_COUNT - 1,
        "{}: every other concurrent caller must stay fail-closed, got {claims:?}",
        A::ADAPTER
    );

    let claimed_state = reload::<A>(git_dir.path());
    assert_eq!(
        A::recovery_view(&claimed_state),
        RecoveryView::Flushing(SEEDED_GENERATION)
    );
    assert_eq!(
        A::next_recovery_generation(&claimed_state),
        SEEDED_NEXT_GENERATION
    );
    assert!(A::scope_ids(&claimed_state).is_empty());
}

pub(crate) fn parallel_state_transactions_serialize_without_lost_updates<A: StateConformance>() {
    let git_dir = ConformanceGitDir::create::<A>("parallel-transactions");
    let seeded = A::fixture_state(
        RecoveryView::Flushing(SEEDED_GENERATION),
        SEEDED_NEXT_GENERATION,
        PARALLEL_REMOVAL_COUNT + 1,
    );
    seed::<A>(git_dir.path(), &seeded);
    let mut scope_ids = A::scope_ids(&seeded);
    let survivor = scope_ids.pop().expect("fixture should hold a survivor");
    let start = Barrier::new(PARALLEL_REMOVAL_COUNT + 1);

    thread::scope(|scope| {
        let start = &start;
        let git_dir = git_dir.path();
        for scope_id in &scope_ids {
            scope.spawn(move || {
                start.wait();
                A::remove_attempt(git_dir, scope_id).expect("concurrent removal should succeed");
            });
        }
        scope.spawn(move || {
            start.wait();
            A::normalize_recovery_after_boundary_lock_acquired(git_dir)
                .expect("concurrent normalize should succeed");
        });
    });

    let converged = reload::<A>(git_dir.path());
    assert_eq!(A::scope_ids(&converged), vec![survivor]);
    assert_eq!(
        A::recovery_view(&converged),
        RecoveryView::Pending(SEEDED_GENERATION)
    );
    assert_eq!(
        A::next_recovery_generation(&converged),
        SEEDED_NEXT_GENERATION
    );
}

macro_rules! mutation_scope_state_conformance_tests {
    ($adapter:ty) => {
        $crate::services::hooks::mutation_scope_state_conformance::mutation_scope_state_conformance_tests! {
            $adapter =>
            missing_state_file_reads_as_default_without_fabricating_a_file,
            state_file_is_checkout_local_below_git_dir_sce,
            state_round_trips_durably_through_the_canonical_path,
            malformed_state_file_is_rejected_without_fabricating_bookkeeping,
            a_state_file_with_any_other_version_is_rejected,
            interruption_before_rename_leaves_the_canonical_path_unaffected,
            removing_an_unknown_attempt_is_a_safe_no_op,
            removing_an_already_removed_attempt_is_a_safe_no_op,
            normalize_after_boundary_lock_reclaims_orphaned_flushing_to_pending_same_generation,
            complete_recovery_flush_clears_only_with_the_matching_generation,
            relinquish_recovery_flush_returns_only_the_claimed_generation_to_pending,
            only_one_concurrent_caller_claims_the_flush_for_a_generation,
            parallel_state_transactions_serialize_without_lost_updates,
        }
    };
    ($adapter:ty => $($contract:ident),+ $(,)?) => {
        $(
            #[test]
            fn $contract() {
                $crate::services::hooks::mutation_scope_state_conformance::$contract::<$adapter>();
            }
        )+
    };
}

pub(crate) use mutation_scope_state_conformance_tests;
