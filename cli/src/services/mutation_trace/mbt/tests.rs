//! Deterministic Quint Connect replays through the real `protocol.rs`.

use quint_connect::quint_test;

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
    test = "testMbtDriverTransportsNonDefaultArguments"
)]
fn mutation_cursor_transports_non_default_arguments() -> impl Driver {
    MutationCursorDriver::default()
}
