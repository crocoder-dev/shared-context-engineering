mod coordinator;
mod external_mutation_guard;
mod external_taint;
mod git_snapshot;
mod maintenance_state;
mod mutation_attribution;
mod protected_worktree;
mod ref_advisory;
mod ref_doctor;
mod ref_maintenance;
mod ref_reconciliation;
mod scope_runtime;
mod worktree_lock;

#[allow(unused_imports)]
pub(crate) use coordinator::{
    coordinate, CoordinateError, CoordinateOutcome, ExternalTaintOperation, RuntimeBoundary,
    StartProvenance,
};
#[allow(unused_imports)]
pub(crate) use external_mutation_guard::{
    arm_external_mutation_guard, ArmedExternalMutationGuard, GuardError, GuardEvent, GuardOutcome,
    GuardRequest,
};
#[allow(unused_imports)]
pub(crate) use git_snapshot::{resolve_git_dir, resolve_worktree_id};
#[allow(unused_imports)]
pub(crate) use mutation_attribution::{
    resolve_bounded_mutation_attribution, resolve_post_commit_mutation_ai_patch,
    BoundedMutationAttribution, MutationAttributionBarrier, MutationEventPageSource,
    TreeReadSource, MAX_MUTATION_ATTRIBUTION_EVENTS,
};
pub(crate) use ref_advisory::{advise_after_completed_boundary, AdvisoryReport, AdvisorySeverity};
pub(crate) use ref_doctor::{
    inspect_reconciliation_recommendation, run_explicit_reconciliation, ReconciliationCounts,
    ReconciliationFix, ReconciliationRecommendation, StateWarning,
};
#[allow(unused_imports)]
pub(crate) use scope_runtime::{
    abandon_scope, AbandonRecoveryReason, AbandonScopeError, AbandonScopeOutcome,
};
