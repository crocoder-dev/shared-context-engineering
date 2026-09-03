mod coordinator;
mod external_taint;
mod git_snapshot;
mod mutation_attribution;
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
pub(crate) use mutation_attribution::{
    resolve_bounded_mutation_attribution, resolve_post_commit_mutation_ai_patch,
    BoundedMutationAttribution, MutationAttributionBarrier, MutationEventPageSource,
    TreeDiffSource, MAX_MUTATION_ATTRIBUTION_EVENTS,
};
#[allow(unused_imports)]
pub(crate) use scope_runtime::{
    abandon_scope, AbandonRecoveryReason, AbandonScopeError, AbandonScopeOutcome,
};
