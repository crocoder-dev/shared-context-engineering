use std::path::Path;

use crate::services::hooks::mutation_scope_health::{
    MutationScopeAdapterHealth, MutationScopeHealthStatus,
};
use crate::services::mutation_trace::types::ActorKind;

use super::state::{self, AttemptPhase, RecoveryState};
use crate::services::hooks::mutation_scope_owner::is_definitely_dead;

pub(crate) fn classify_health(git_dir: &Path) -> MutationScopeAdapterHealth {
    let state = match state::read_state(git_dir) {
        Ok(state) => state,
        Err(error) => {
            return MutationScopeAdapterHealth::new(
                ActorKind::Pi,
                MutationScopeHealthStatus::Invalid,
                "Pi mutation-scope state file could not be read or parsed.",
            )
            .with_detail(error.to_string());
        }
    };

    let has_pending_abandon = state
        .attempts
        .iter()
        .any(|attempt| attempt.phase == AttemptPhase::PendingAbandon);
    let has_dead_owner_live_attempt = state.attempts.iter().any(|attempt| {
        matches!(
            attempt.phase,
            AttemptPhase::PendingStart | AttemptPhase::Executed
        ) && is_definitely_dead(&attempt.owner)
    });

    match state.recovery {
        RecoveryState::Clear if has_pending_abandon => MutationScopeAdapterHealth::new(
            ActorKind::Pi,
            MutationScopeHealthStatus::Invalid,
            "Persisted state has a PendingAbandon attempt with recovery already Clear, a combination the adapter's recovery-flush state machine cannot legitimately produce (a PendingAbandon attempt is only ever created and removed as part of the same recovery-flush generation that arms and then clears RecoveryState).",
        ),
        RecoveryState::Clear if has_dead_owner_live_attempt => MutationScopeAdapterHealth::new(
            ActorKind::Pi,
            MutationScopeHealthStatus::Recovering,
            "A tracked PendingStart/Executed attempt's recorded owner process is positively dead. The D10 stale-owner sweep (reconcile_stale_owners) runs unconditionally on every future tracked Start from any session, before that session's own admission is even considered, and automatically retires the dead-owner attempt through the ordinary flush/abandon/flush recovery sequence.",
        ),
        RecoveryState::Clear => MutationScopeAdapterHealth::new(
            ActorKind::Pi,
            MutationScopeHealthStatus::Healthy,
            "No persisted recovery condition blocks mutation-capable admission. A live or unprovably-dead PendingStart/Executed attempt never blocks an unrelated tracked admission on this adapter.",
        ),
        RecoveryState::Pending { .. } => MutationScopeAdapterHealth::new(
            ActorKind::Pi,
            MutationScopeHealthStatus::Recovering,
            "Recovery is pending. A subsequent recovery-capable tracked Start whose key is not already represented by a nonterminal attempt can claim the pending generation and retry every outstanding PendingAbandon attempt through the ordinary recovery path. A duplicate Start for an already-tracked PendingStart/Executed key may be idempotently reused before the recovery-state gate, so not every individual Start necessarily advances recovery. Pending is Recovering because an ordinary future tracked admission can advance it without manual intervention; an unrelated stuck PendingStart/Executed attempt, if any, does not prevent this resolution, since this adapter never gates admission on another attempt's PendingStart/Executed phase.",
        ),
        RecoveryState::Flushing { .. } => MutationScopeAdapterHealth::new(
            ActorKind::Pi,
            MutationScopeHealthStatus::Recovering,
            "An orphaned recovery flush is reclaimed from Flushing to Pending by the very next tracked adapter boundary (Start, ToolExecutionEnd, or ToolExecutionAbandon all normalize it before doing anything else). A subsequent recovery-capable fresh tracked Start can then claim the reclaimed generation and retry recovery automatically, but a duplicate Start for an already-tracked nonterminal key is not guaranteed to be the one that does so.",
        ),
    }
}
