use quint_connect::{quint_run, quint_test};

use super::driver::MutationCursorDriver;

#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testMbtDriverTransportsNonDefaultArguments",
    max_samples = 1
)]
fn mutation_cursor_transports_non_default_arguments() -> impl Driver {
    MutationCursorDriver::default()
}

#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testStartObservesBeforeActivation",
    max_samples = 1
)]
fn mutation_cursor_start_observes_before_activation() -> impl Driver {
    MutationCursorDriver::default()
}

#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testCloseObservesBeforeDeactivation",
    max_samples = 1
)]
fn mutation_cursor_close_observes_before_deactivation() -> impl Driver {
    MutationCursorDriver::default()
}

#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testContendedIntervalsRemainAiContended",
    max_samples = 1
)]
fn mutation_cursor_contended_intervals_remain_ai_contended() -> impl Driver {
    MutationCursorDriver::default()
}

#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testNoChangeHookReplayCannotStealFutureChange",
    max_samples = 1
)]
fn mutation_cursor_no_change_hook_replay_cannot_steal_future_change() -> impl Driver {
    MutationCursorDriver::default()
}

#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testConcurrentObservationsHaveOneWinner",
    max_samples = 1
)]
fn mutation_cursor_concurrent_observations_have_one_winner() -> impl Driver {
    MutationCursorDriver::default()
}

#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testTaintInvalidatesPreparedObservation",
    max_samples = 1
)]
fn mutation_cursor_taint_invalidates_prepared_observation() -> impl Driver {
    MutationCursorDriver::default()
}

#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testRecoveryEstablishesBaseline",
    max_samples = 1
)]
fn mutation_cursor_recovery_establishes_baseline() -> impl Driver {
    MutationCursorDriver::default()
}

#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testClosedScopeCannotReactivate",
    max_samples = 1
)]
fn mutation_cursor_closed_scope_cannot_reactivate() -> impl Driver {
    MutationCursorDriver::default()
}

#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testOpenCodeCloseConfirmsExclusiveAttribution",
    max_samples = 1
)]
fn mutation_cursor_opencode_close_confirms_exclusive_attribution() -> impl Driver {
    MutationCursorDriver::default()
}

#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testOpenCodeCloseConfirmsContendedAttribution",
    max_samples = 1
)]
fn mutation_cursor_opencode_close_confirms_contended_attribution() -> impl Driver {
    MutationCursorDriver::default()
}

#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testUnconfirmedOpenCodeScopeBlocksCrossHarnessAttribution",
    max_samples = 1
)]
fn mutation_cursor_unconfirmed_opencode_scope_blocks_cross_harness_attribution() -> impl Driver {
    MutationCursorDriver::default()
}

#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testMbtGuardedPrepareInvokesRealPrepare",
    max_samples = 1
)]
fn mutation_cursor_guarded_prepare_invokes_real_prepare() -> impl Driver {
    MutationCursorDriver::default()
}

#[quint_test(
    spec = "../spec/mutation_cursor.qnt",
    test = "testMbtGuardedRecoverInvokesRealRecover",
    max_samples = 1
)]
fn mutation_cursor_guarded_recover_invokes_real_recover() -> impl Driver {
    MutationCursorDriver::default()
}

#[quint_run(
    spec = "../spec/mutation_cursor.qnt",
    max_samples = 200,
    max_steps = 30,
    seed = "0xbd646ab9"
)]
fn mutation_cursor_generated_traces_refine_rust_protocol() -> impl Driver {
    MutationCursorDriver::default()
}

#[quint_test(spec = "../spec/mutation_cursor.qnt", test = "test.*", max_samples = 1)]
fn mutation_cursor_all_named_scenarios_refine_rust_protocol() -> impl Driver {
    MutationCursorDriver::default()
}
