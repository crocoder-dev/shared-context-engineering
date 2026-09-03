//! Deterministic Quint Connect replays through the real `protocol.rs`.

use quint_connect::{quint_run, quint_test};

use super::driver::MutationCursorDriver;

/// Replays `testMbtDriverTransportsNonDefaultArguments`
/// (`spec/mutation_cursor.qnt`): `mutate(WT1, Tree3)` →
/// `prepare(Attempt5, Flush(WT1))` → `commitAttempt(Attempt5)`. Every value
/// here — worktree, tree, attempt, and boundary kind — differs from the
/// `WT0`/`Tree0`/`Tree1`/`Attempt0`/`Attempt1`/`Start`/`Advance`/`Close`
/// defaults the spec's other named scenarios use, so a passing replay proves
/// the driver transports the trace's actual concrete arguments into
/// `protocol::prepare`/`protocol::commit` rather than guessing or defaulting
/// them.
#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testMbtDriverTransportsNonDefaultArguments",
    max_samples = 1
)]
fn mutation_cursor_transports_non_default_arguments() -> impl Driver {
    MutationCursorDriver::default()
}

/// Replays `testStartObservesBeforeActivation`: the freshness-boundary
/// semantics for a `Start` observation.
#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testStartObservesBeforeActivation",
    max_samples = 1
)]
fn mutation_cursor_start_observes_before_activation() -> impl Driver {
    MutationCursorDriver::default()
}

/// Replays `testCloseObservesBeforeDeactivation`.
#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testCloseObservesBeforeDeactivation",
    max_samples = 1
)]
fn mutation_cursor_close_observes_before_deactivation() -> impl Driver {
    MutationCursorDriver::default()
}

/// Replays `testContendedIntervalsRemainAiContended`.
#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testContendedIntervalsRemainAiContended",
    max_samples = 1
)]
fn mutation_cursor_contended_intervals_remain_ai_contended() -> impl Driver {
    MutationCursorDriver::default()
}

/// Replays `testNoChangeHookReplayCannotStealFutureChange`.
#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testNoChangeHookReplayCannotStealFutureChange",
    max_samples = 1
)]
fn mutation_cursor_no_change_hook_replay_cannot_steal_future_change() -> impl Driver {
    MutationCursorDriver::default()
}

/// Replays `testConcurrentObservationsHaveOneWinner`.
#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testConcurrentObservationsHaveOneWinner",
    max_samples = 1
)]
fn mutation_cursor_concurrent_observations_have_one_winner() -> impl Driver {
    MutationCursorDriver::default()
}

/// Replays `testTaintInvalidatesPreparedObservation`.
#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testTaintInvalidatesPreparedObservation",
    max_samples = 1
)]
fn mutation_cursor_taint_invalidates_prepared_observation() -> impl Driver {
    MutationCursorDriver::default()
}

/// Replays `testRecoveryEstablishesBaseline`.
#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testRecoveryEstablishesBaseline",
    max_samples = 1
)]
fn mutation_cursor_recovery_establishes_baseline() -> impl Driver {
    MutationCursorDriver::default()
}

/// Replays `testClosedScopeCannotReactivate`.
#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testClosedScopeCannotReactivate",
    max_samples = 1
)]
fn mutation_cursor_closed_scope_cannot_reactivate() -> impl Driver {
    MutationCursorDriver::default()
}

/// Guarded-no-op regression: replays
/// `testMbtGuardedPrepareInvokesRealPrepare`
/// (`init.then(prepare(Attempt0, Start(...))).then(prepare(Attempt0,
/// Advance(...)))`), where the second `prepare` guards because `Attempt0` is
/// no longer `Available`. A passing replay proves the driver still calls
/// `protocol::prepare` on the guarded step — dispatch is on the `MbtAction`
/// variant Quint recorded, never skipped because Quint's own state happened
/// not to change — and independently reaches the same no-op outcome.
#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testMbtGuardedPrepareInvokesRealPrepare",
    max_samples = 1
)]
fn mutation_cursor_guarded_prepare_invokes_real_prepare() -> impl Driver {
    MutationCursorDriver::default()
}

/// Guarded-no-op regression: replays
/// `testMbtGuardedRecoverInvokesRealRecover` (`init.then(recover(WT0))`),
/// where `recover` guards because `WT0` is neither tainted, externally
/// tainted, nor needing rebaseline. A passing replay proves the driver still
/// calls `protocol::recover` on the guarded step and independently reaches
/// the same no-op outcome.
#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testMbtGuardedRecoverInvokesRealRecover",
    max_samples = 1
)]
fn mutation_cursor_guarded_recover_invokes_real_recover() -> impl Driver {
    MutationCursorDriver::default()
}

/// Generated-trace refinement: replays Quint-generated randomized
/// traces through the real `protocol.rs`, comparing `ModelState` against
/// Quint's semantic state after every step. Reproducible with a fixed
/// `QUINT_SEED`.
#[quint_run(
    spec = "../spec/mutation_cursor.qnt",
    max_samples = 500,
    max_steps = 30
)]
fn mutation_cursor_generated_traces_refine_rust_protocol() -> impl Driver {
    MutationCursorDriver::default()
}

/// Coverage backstop: `test = "test.*"` is passed through unescaped into
/// `quint test`'s `--match` as `^test.*$`, so this replays every top-level
/// `test...`-named `run` in `spec/mutation_cursor.qnt` — not just the
/// individually named scenarios above — through the real `protocol.rs`. Each
/// matched `run` is deterministic, so `max_samples = 1`. This exists so a new
/// named Quint scenario is automatically exercised here without also
/// requiring a new hand-written Rust wrapper; the individually named tests
/// above stay for readable `cargo test` failure output on the scenarios
/// worth naming.
#[quint_test(spec = "../spec/mutation_cursor.qnt", test = "test.*", max_samples = 1)]
fn mutation_cursor_all_named_scenarios_refine_rust_protocol() -> impl Driver {
    MutationCursorDriver::default()
}
