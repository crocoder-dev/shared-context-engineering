use std::path::Path;

use crate::services::hooks::mutation_scope_health::{
    MutationScopeAdapterHealth, MutationScopeHealthStatus,
};
use crate::services::hooks::mutation_scope_owner::is_definitely_dead;
use crate::services::mutation_trace::types::ActorKind;

use super::state::{self, AttemptPhase, RecoveryState};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Repairability {
    AutoFixable,
    ManualOnly,
}

pub(crate) fn assess_repairability(git_dir: &Path) -> Repairability {
    let Ok(state) = state::read_state(git_dir) else {
        return Repairability::ManualOnly;
    };

    let pending_start: Vec<&state::AdapterAttempt> = state
        .attempts
        .iter()
        .filter(|attempt| attempt.phase == AttemptPhase::PendingStart)
        .collect();

    if pending_start.is_empty() {
        return Repairability::ManualOnly;
    }

    let every_owner_is_positively_dead = pending_start
        .iter()
        .all(|attempt| attempt.owner.as_ref().is_some_and(is_definitely_dead));

    if every_owner_is_positively_dead {
        Repairability::AutoFixable
    } else {
        Repairability::ManualOnly
    }
}

pub(crate) fn classify_health(git_dir: &Path) -> MutationScopeAdapterHealth {
    let state = match state::read_state(git_dir) {
        Ok(state) => state,
        Err(error) => {
            return MutationScopeAdapterHealth::new(
                ActorKind::OpenCode,
                MutationScopeHealthStatus::Invalid,
                "OpenCode mutation-scope state file could not be read or parsed.",
            )
            .with_detail(error.to_string());
        }
    };

    let has_pending_abandon = state
        .attempts
        .iter()
        .any(|attempt| attempt.phase == AttemptPhase::PendingAbandon);
    let has_pending_start = state
        .attempts
        .iter()
        .any(|attempt| attempt.phase == AttemptPhase::PendingStart);

    match state.recovery {
        RecoveryState::Clear if has_pending_abandon => MutationScopeAdapterHealth::new(
            ActorKind::OpenCode,
            MutationScopeHealthStatus::Invalid,
            "Persisted state has a PendingAbandon attempt with recovery already Clear, a combination the adapter's recovery-flush state machine cannot legitimately produce (a PendingAbandon attempt is only ever removed as part of the same recovery flush that clears recovery to Clear).",
        ),
        _ if has_pending_start => MutationScopeAdapterHealth::new(
            ActorKind::OpenCode,
            MutationScopeHealthStatus::Blocked,
            "A tracked attempt is stuck in PendingStart; only that same call's own ToolExecuteAfter/ToolError boundary retires a PendingStart attempt, and resolve_recovery only retries attempts already in PendingAbandon, so a concurrently Pending/Flushing recovery generation for an unrelated attempt can clear without ever touching this one, leaving future tracked admissions from other calls denied without self-clearing.",
        ),
        RecoveryState::Clear => MutationScopeAdapterHealth::new(
            ActorKind::OpenCode,
            MutationScopeHealthStatus::Healthy,
            "No persisted recovery condition blocks mutation-capable admission.",
        ),
        RecoveryState::Pending { .. } => MutationScopeAdapterHealth::new(
            ActorKind::OpenCode,
            MutationScopeHealthStatus::Recovering,
            "Recovery is pending; the next tracked admission from any call claims the flush and retries any outstanding abandonment automatically, regardless of which call performs it.",
        ),
        RecoveryState::Flushing { .. } => MutationScopeAdapterHealth::new(
            ActorKind::OpenCode,
            MutationScopeHealthStatus::Recovering,
            "An orphaned recovery flush is reclaimed from Flushing to Pending by the next tracked adapter boundary. A subsequent recovery-capable tracked admission claims the pending generation and retries recovery automatically.",
        ),
    }
}
