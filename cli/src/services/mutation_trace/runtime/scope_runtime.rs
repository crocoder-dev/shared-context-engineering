use std::path::Path;

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::mutation_trace::protocol;
use crate::services::mutation_trace::store::{CasResult, DurableTransition, MutationTraceStore};
use crate::services::mutation_trace::types::{ProtocolState, ScopeId, ScopeStatus, WorktreeId};

use super::coordinator::MAX_CAS_RETRY_ATTEMPTS;
use super::protected_worktree::{
    ExternalTaintOperation, ProtectedWorktree, ProtectedWorktreeError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AbandonRecoveryReason {
    InheritedExternalTaint,
    MissingScope,
    NeverSeenScope,
    MissingWorktreeState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AbandonScopeOutcome {
    Abandoned {
        worktree_id: WorktreeId,
        scope: ScopeId,
        revision: u64,
    },
    AlreadyTerminal {
        worktree_id: WorktreeId,
        scope: ScopeId,
        status: ScopeStatus,
        revision: u64,
    },
    RecoveryRequired {
        worktree_id: WorktreeId,
        scope: ScopeId,
        reason: AbandonRecoveryReason,
    },
}

#[derive(Debug)]
pub enum AbandonScopeError {
    LockAcquisition(anyhow::Error),
    ExternalTaintMarker {
        operation: ExternalTaintOperation,
        source: anyhow::Error,
    },
    AgentTraceDbUnavailable(anyhow::Error),
    WorktreeIdentityMismatch {
        scope: ScopeId,
        scope_worktree_id: WorktreeId,
        invoking_worktree_id: WorktreeId,
    },
    RevisionExhausted {
        worktree_id: WorktreeId,
        revision: u64,
    },
    CasConflictExhausted {
        attempts: u32,
    },
    MarkerClearAfterCompletion {
        source: anyhow::Error,
        completed: Box<AbandonScopeOutcome>,
    },
    Other(anyhow::Error),
}

impl std::fmt::Display for AbandonScopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AbandonScopeError::ExternalTaintMarker { operation, source } => write!(
                f,
                "External-taint marker {operation:?} operation failed before any \
                 mutation-scope state was read: {source}"
            ),
            AbandonScopeError::AgentTraceDbUnavailable(source) => {
                write!(f, "Repository Agent Trace DB is unavailable: {source}")
            }
            AbandonScopeError::WorktreeIdentityMismatch {
                scope,
                scope_worktree_id,
                invoking_worktree_id,
            } => write!(
                f,
                "Scope {scope:?} belongs to worktree {scope_worktree_id:?} and cannot \
                 be abandoned through worktree {invoking_worktree_id:?}"
            ),
            AbandonScopeError::RevisionExhausted {
                worktree_id,
                revision,
            } => write!(
                f,
                "Scope on worktree {worktree_id:?} cannot be abandoned because its \
                 revision ({revision}) cannot be advanced"
            ),
            AbandonScopeError::CasConflictExhausted { attempts } => {
                write!(f, "Exhausted {attempts} CAS-conflict retry attempts")
            }
            AbandonScopeError::MarkerClearAfterCompletion { source, .. } => write!(
                f,
                "Mutation scope settled durably, but clearing the external-taint \
                 marker failed: {source}"
            ),
            AbandonScopeError::LockAcquisition(source) | AbandonScopeError::Other(source) => {
                write!(f, "{source}")
            }
        }
    }
}

impl std::error::Error for AbandonScopeError {}

pub async fn abandon_scope<P>(
    repository_root: &Path,
    scope: &ScopeId,
    open_db: P,
) -> Result<AbandonScopeOutcome, AbandonScopeError>
where
    P: std::ops::AsyncFnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
{
    abandon_scope_inner(repository_root, scope, open_db, |_attempt| {}).await
}

pub(super) async fn abandon_scope_inner<P, L>(
    repository_root: &Path,
    scope: &ScopeId,
    open_db: P,
    after_load: L,
) -> Result<AbandonScopeOutcome, AbandonScopeError>
where
    P: std::ops::AsyncFnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
    L: FnMut(u32),
{
    let protected = ProtectedWorktree::acquire(repository_root)
        .await
        .map_err(protected_worktree_failure)?;

    if protected.inherited_external_taint() {
        return Ok(AbandonScopeOutcome::RecoveryRequired {
            worktree_id: protected.worktree_id().clone(),
            scope: scope.clone(),
            reason: AbandonRecoveryReason::InheritedExternalTaint,
        });
    }

    let outcome = abandon_protected(protected.worktree_id(), scope, open_db, after_load).await?;

    if matches!(outcome, AbandonScopeOutcome::RecoveryRequired { .. }) {
        return Ok(outcome);
    }

    match protected.complete() {
        Ok(()) => Ok(outcome),
        Err(source) => Err(AbandonScopeError::MarkerClearAfterCompletion {
            source,
            completed: Box::new(outcome),
        }),
    }
}

fn projected_revision_and_status(
    state: &ProtocolState,
    worktree_id: &WorktreeId,
    scope: &ScopeId,
) -> Result<(u64, ScopeStatus), AbandonScopeError> {
    let revision = state
        .worktrees
        .get(worktree_id)
        .map(|worktree_state| worktree_state.revision)
        .ok_or_else(|| {
            AbandonScopeError::Other(anyhow::anyhow!(
                "worktree {worktree_id:?} missing from its own loaded projection"
            ))
        })?;
    let status = state
        .scopes
        .get(scope)
        .map(|loaded| loaded.status)
        .ok_or_else(|| {
            AbandonScopeError::Other(anyhow::anyhow!(
                "scope {scope:?} missing from the projection that loaded it as its \
                 effective referenced scope"
            ))
        })?;
    Ok((revision, status))
}

async fn abandon_protected<P, L>(
    worktree_id: &WorktreeId,
    scope: &ScopeId,
    open_db: P,
    mut after_load: L,
) -> Result<AbandonScopeOutcome, AbandonScopeError>
where
    P: std::ops::AsyncFnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
    L: FnMut(u32),
{
    let db = open_db()
        .await
        .map_err(AbandonScopeError::AgentTraceDbUnavailable)?;
    let store = MutationTraceStore::new(&db);

    for attempt_index in 0..MAX_CAS_RETRY_ATTEMPTS {
        let Some(scope_state) = store
            .load_scope(scope)
            .await
            .map_err(AbandonScopeError::Other)?
        else {
            return Ok(recovery_required(
                worktree_id,
                scope,
                AbandonRecoveryReason::MissingScope,
            ));
        };
        if scope_state.worktree_id != *worktree_id {
            return Err(AbandonScopeError::WorktreeIdentityMismatch {
                scope: scope.clone(),
                scope_worktree_id: scope_state.worktree_id,
                invoking_worktree_id: worktree_id.clone(),
            });
        }

        let Some(projection) = store
            .load_worktree(worktree_id, Some(scope), None)
            .await
            .map_err(AbandonScopeError::Other)?
        else {
            return Ok(recovery_required(
                worktree_id,
                scope,
                AbandonRecoveryReason::MissingWorktreeState,
            ));
        };

        after_load(attempt_index);

        let state = projection.into_protocol_state();
        let (revision, status) = projected_revision_and_status(&state, worktree_id, scope)?;

        match status {
            ScopeStatus::NeverSeen => {
                return Ok(recovery_required(
                    worktree_id,
                    scope,
                    AbandonRecoveryReason::NeverSeenScope,
                ))
            }
            ScopeStatus::Closed | ScopeStatus::Abandoned => {
                return Ok(AbandonScopeOutcome::AlreadyTerminal {
                    worktree_id: worktree_id.clone(),
                    scope: scope.clone(),
                    status,
                    revision,
                })
            }
            ScopeStatus::Active => {}
        }

        let abandoned = protocol::abandon(&state, scope);

        let Some(transition) = DurableTransition::between(&state, &abandoned, worktree_id)
            .map_err(AbandonScopeError::Other)?
        else {
            return Err(AbandonScopeError::RevisionExhausted {
                worktree_id: worktree_id.clone(),
                revision,
            });
        };

        match store
            .commit(&transition)
            .await
            .map_err(AbandonScopeError::Other)?
        {
            CasResult::Applied => {
                let next_revision = abandoned
                    .worktrees
                    .get(worktree_id)
                    .map(|worktree_state| worktree_state.revision)
                    .expect("the abandoned worktree's state must still be present after abandon");
                return Ok(AbandonScopeOutcome::Abandoned {
                    worktree_id: worktree_id.clone(),
                    scope: scope.clone(),
                    revision: next_revision,
                });
            }
            CasResult::Conflict => {}
        }
    }

    Err(AbandonScopeError::CasConflictExhausted {
        attempts: MAX_CAS_RETRY_ATTEMPTS,
    })
}

fn recovery_required(
    worktree_id: &WorktreeId,
    scope: &ScopeId,
    reason: AbandonRecoveryReason,
) -> AbandonScopeOutcome {
    AbandonScopeOutcome::RecoveryRequired {
        worktree_id: worktree_id.clone(),
        scope: scope.clone(),
        reason,
    }
}

fn protected_worktree_failure(error: ProtectedWorktreeError) -> AbandonScopeError {
    match error {
        ProtectedWorktreeError::GitDirResolution(source)
        | ProtectedWorktreeError::CheckoutIdentity(source) => AbandonScopeError::Other(source),
        ProtectedWorktreeError::LockAcquisition(source) => {
            AbandonScopeError::LockAcquisition(anyhow::Error::new(source))
        }
        ProtectedWorktreeError::ExternalTaintMarker { operation, source } => {
            AbandonScopeError::ExternalTaintMarker { operation, source }
        }
    }
}
