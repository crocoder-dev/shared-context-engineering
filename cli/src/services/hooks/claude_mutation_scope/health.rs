use std::path::Path;

use crate::services::hooks::mutation_scope_health::{
    MutationScopeAdapterHealth, MutationScopeHealthStatus,
};
use crate::services::mutation_trace::types::ActorKind;

use super::state;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Repairability {
    AutoFixable,
    ManualOnly,
}

pub(crate) fn assess_repairability(git_dir: &Path) -> Repairability {
    let Ok(state) = state::read_state(git_dir) else {
        return Repairability::ManualOnly;
    };

    if !state.recovery_pending || state.attempts.is_empty() {
        return Repairability::ManualOnly;
    }

    let every_attempt_is_pending_abandon = state
        .attempts
        .iter()
        .all(|attempt| attempt.phase == state::AttemptPhase::PendingAbandon);

    if every_attempt_is_pending_abandon {
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
                ActorKind::ClaudeCode,
                MutationScopeHealthStatus::Invalid,
                "Claude mutation-scope state file could not be read or parsed.",
            )
            .with_detail(error.to_string());
        }
    };

    let has_pending_abandon = state
        .attempts
        .iter()
        .any(|attempt| attempt.phase == state::AttemptPhase::PendingAbandon);

    if !state.recovery_pending && has_pending_abandon {
        return MutationScopeAdapterHealth::new(
            ActorKind::ClaudeCode,
            MutationScopeHealthStatus::Invalid,
            "PendingAbandon means terminal cleanup has been durably established, therefore the recovery barrier cannot legitimately already be clear.",
        );
    }

    if !state.recovery_pending {
        return MutationScopeAdapterHealth::new(
            ActorKind::ClaudeCode,
            MutationScopeHealthStatus::Healthy,
            "No persisted recovery condition blocks mutation-capable admission.",
        );
    }

    if state.attempts.is_empty() {
        return MutationScopeAdapterHealth::new(
            ActorKind::ClaudeCode,
            MutationScopeHealthStatus::Recovering,
            "Recovery is pending with no unresolved attempts; the next tracked PreToolUse call flushes and clears it automatically.",
        );
    }

    MutationScopeAdapterHealth::new(
        ActorKind::ClaudeCode,
        MutationScopeHealthStatus::Blocked,
        "Recovery is pending with unresolved attempts; the recovery barrier's flush path only runs once attempts are empty, so future tracked PreToolUse calls deny without self-clearing.",
    )
}
