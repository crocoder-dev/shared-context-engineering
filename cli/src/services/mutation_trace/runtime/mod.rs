mod coordinator;
mod external_taint;
mod git_snapshot;
mod protected_worktree;
mod ref_reconciliation;
mod scope_runtime;
mod worktree_lock;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub(crate) use coordinator::{
    coordinate, CoordinateError, CoordinateOutcome, ExternalTaintOperation, RuntimeBoundary,
};
#[allow(unused_imports)]
pub(crate) use scope_runtime::{
    abandon_scope, AbandonRecoveryReason, AbandonScopeError, AbandonScopeOutcome,
};
