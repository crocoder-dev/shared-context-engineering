use std::path::Path;

use crate::services::hooks::mutation_scope_health::{
    MutationScopeAdapterHealth, MutationScopeHealthStatus,
};
use crate::services::mutation_trace::types::ActorKind;

use super::state::{self, RecoveryState};

pub(crate) fn classify_health(git_dir: &Path) -> MutationScopeAdapterHealth {
    let state = match state::read_state(git_dir) {
        Ok(state) => state,
        Err(error) => {
            return MutationScopeAdapterHealth::new(
                ActorKind::Codex,
                MutationScopeHealthStatus::Invalid,
                "Codex mutation-scope state file could not be read or parsed.",
            )
            .with_detail(error.to_string());
        }
    };

    match state.recovery {
        RecoveryState::Clear => MutationScopeAdapterHealth::new(
            ActorKind::Codex,
            MutationScopeHealthStatus::Healthy,
            "No persisted recovery condition blocks mutation-capable admission.",
        ),
        RecoveryState::Pending { .. } if state.attempts.is_empty() => {
            MutationScopeAdapterHealth::new(
                ActorKind::Codex,
                MutationScopeHealthStatus::Recovering,
                "Recovery is pending with no unresolved attempts; the next tracked PreToolUse call claims and completes the flush automatically.",
            )
        }
        RecoveryState::Pending { .. } => MutationScopeAdapterHealth::new(
            ActorKind::Codex,
            MutationScopeHealthStatus::Recovering,
            "Recovery is pending with unresolved attempts; an unrelated tracked PreToolUse remains denied by the global recovery barrier, but a later tracked PreToolUse in the same (session_id, turn_id) lane retries the stale predecessor's abandonment through the ordinary same-lane sweep, which can clear the attempt and advance recovery to a flush without manual intervention.",
        ),
        RecoveryState::Flushing { .. } if state.attempts.is_empty() => {
            MutationScopeAdapterHealth::new(
                ActorKind::Codex,
                MutationScopeHealthStatus::Recovering,
                "A recovery flush is in progress; an orphaned flush is reclaimed and its flush retried automatically on the next tracked PreToolUse boundary.",
            )
        }
        RecoveryState::Flushing { .. } => MutationScopeAdapterHealth::new(
            ActorKind::Codex,
            MutationScopeHealthStatus::Invalid,
            "Persisted state has a recovery flush in progress with unresolved attempts outstanding, a combination the adapter's state machine cannot legitimately produce.",
        ),
    }
}
