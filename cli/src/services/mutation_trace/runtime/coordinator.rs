use std::path::Path;

use anyhow::Result;
use uuid::Uuid;

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::mutation_trace::protocol;
use crate::services::mutation_trace::store::{
    CasResult, DurableTransition, MutationTraceStore, ScopeProvenance,
};
use crate::services::mutation_trace::types::{
    self, ActorKind, AttemptId, Boundary, EventId, FailureKind, MutationEvent, ScopeId,
    ScopeStatus, TreeId, WorktreeId,
};

use super::git_snapshot::GitSnapshotService;
use super::protected_worktree::{ProtectedWorktree, ProtectedWorktreeError};
use super::worktree_lock::WorktreeLockLease;

pub use super::protected_worktree::ExternalTaintOperation;

pub(super) const MAX_CAS_RETRY_ATTEMPTS: u32 = 5;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartProvenance {
    pub session_id: String,
    pub model_id: Option<String>,
}

#[derive(Clone, Debug)]
pub enum RuntimeBoundary {
    Start {
        scope: ScopeId,
        event: EventId,
        actor_kind: ActorKind,
        provenance: Option<StartProvenance>,
    },
    Advance {
        scope: ScopeId,
        event: EventId,
        actor_kind: ActorKind,
    },
    Close {
        scope: ScopeId,
        event: EventId,
        actor_kind: ActorKind,
    },
    Flush,
}

#[derive(Debug)]
pub struct CoordinateOutcome {
    pub worktree_id: WorktreeId,
    pub observed_tree: TreeId,
    pub revision: u64,
    pub evaluation: protocol::CommitEvaluation,
    pub mutation_event: Option<MutationEvent>,
}

#[derive(Debug)]
pub enum CoordinateError {
    SnapshotFailure {
        persisted_taint: bool,
        source: anyhow::Error,
    },
    ScopeIdentityConflict(anyhow::Error),
    ScopeProvenanceRegistration(anyhow::Error),
    CasConflictExhausted {
        attempts: u32,
    },
    RevisionExhausted {
        worktree_id: WorktreeId,
        revision: u64,
    },
    LockAcquisition(anyhow::Error),
    /// Inspecting or persisting the worktree-local external-taint marker failed.
    /// Both operations run **before** any checkout-identity, DB, snapshot, or
    /// protocol work, so no mutation boundary has committed and there is no
    /// [`CoordinateOutcome`] to surface — the boundary is aborted fail-closed
    /// with the fence left in whatever state it was in.
    ExternalTaintMarker {
        operation: ExternalTaintOperation,
        source: anyhow::Error,
    },
    /// The mutation boundary committed successfully to the Agent Trace DB and
    /// produced a [`CoordinateOutcome`], but clearing the write-ahead
    /// external-taint marker afterwards failed. The boundary did **not** fail:
    /// `committed` carries the durable outcome (including any [`MutationEvent`])
    /// so the caller never loses it. The marker remains logically armed, so the
    /// next invocation conservatively recovers.
    MarkerClearAfterCommit {
        source: anyhow::Error,
        committed: Box<CoordinateOutcome>,
    },
    /// The caller-supplied Agent Trace DB provider returned `Err` after the
    /// external-taint marker was already armed. The marker is intentionally
    /// left in place so a later invocation treats the lost interval
    /// conservatively.
    AgentTraceDbUnavailable(anyhow::Error),
    Other(anyhow::Error),
}

impl std::fmt::Display for CoordinateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoordinateError::SnapshotFailure {
                persisted_taint,
                source,
            } => write!(
                f,
                "Git snapshot capture/pin failed (taint persisted: {persisted_taint}): {source}"
            ),
            CoordinateError::CasConflictExhausted { attempts } => {
                write!(f, "Exhausted {attempts} CAS-conflict retry attempts")
            }
            CoordinateError::RevisionExhausted {
                worktree_id,
                revision,
            } => write!(
                f,
                "Worktree {worktree_id:?} requires recovery but its revision \
                 ({revision}) cannot be advanced"
            ),
            CoordinateError::ExternalTaintMarker { operation, source } => write!(
                f,
                "External-taint marker {operation:?} operation failed before any \
                 mutation boundary committed: {source}"
            ),
            CoordinateError::MarkerClearAfterCommit { source, .. } => write!(
                f,
                "Mutation boundary committed, but clearing the external-taint \
                 marker failed: {source}"
            ),
            CoordinateError::AgentTraceDbUnavailable(source) => {
                write!(f, "Repository Agent Trace DB is unavailable: {source}")
            }
            CoordinateError::ScopeIdentityConflict(source)
            | CoordinateError::ScopeProvenanceRegistration(source)
            | CoordinateError::LockAcquisition(source)
            | CoordinateError::Other(source) => write!(f, "{source}"),
        }
    }
}

impl std::error::Error for CoordinateError {}

pub trait SnapshotCapture {
    async fn capture(&self) -> Result<TreeId>;
    async fn pin(
        &self,
        lease: WorktreeLockLease,
        worktree_id: &WorktreeId,
        tree: &TreeId,
    ) -> Result<()>;
}

impl SnapshotCapture for GitSnapshotService {
    async fn capture(&self) -> Result<TreeId> {
        self.capture_tree().await
    }

    async fn pin(
        &self,
        lease: WorktreeLockLease,
        worktree_id: &WorktreeId,
        tree: &TreeId,
    ) -> Result<()> {
        self.pin_tree(lease, worktree_id, tree).await
    }
}

pub async fn coordinate<P>(
    repository_root: &Path,
    boundary: &RuntimeBoundary,
    open_db: P,
) -> Result<CoordinateOutcome, CoordinateError>
where
    P: std::ops::AsyncFnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
{
    coordinate_inner(
        repository_root,
        boundary,
        open_db,
        || {},
        |_attempt| {},
        |_attempt| Ok(()),
    )
    .await
}

pub(super) async fn coordinate_inner<P, F, L, R>(
    repository_root: &Path,
    boundary: &RuntimeBoundary,
    open_db: P,
    on_lock_contention: F,
    after_load: L,
    after_recovery: R,
) -> Result<CoordinateOutcome, CoordinateError>
where
    P: std::ops::AsyncFnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
    F: FnOnce() + Send + 'static,
    L: FnMut(u32),
    R: FnMut(u32) -> Result<()>,
{
    let protected = ProtectedWorktree::acquire_inner(repository_root, on_lock_contention)
        .await
        .map_err(protected_worktree_failure)?;

    let outcome = coordinate_protected(
        repository_root,
        protected.lock_lease(),
        protected.worktree_id(),
        boundary,
        open_db,
        protected.inherited_external_taint(),
        after_load,
        after_recovery,
    )
    .await?;

    match protected.complete() {
        Ok(()) => Ok(outcome),
        Err(source) => Err(CoordinateError::MarkerClearAfterCommit {
            source,
            committed: Box::new(outcome),
        }),
    }
}

#[allow(clippy::too_many_arguments)]
async fn coordinate_protected<P, L, R>(
    repository_root: &Path,
    pin_lease: WorktreeLockLease,
    worktree_id: &WorktreeId,
    boundary: &RuntimeBoundary,
    open_db: P,
    inherited_external_taint: bool,
    after_load: L,
    after_recovery: R,
) -> Result<CoordinateOutcome, CoordinateError>
where
    P: std::ops::AsyncFnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
    L: FnMut(u32),
    R: FnMut(u32) -> Result<()>,
{
    let db = open_db()
        .await
        .map_err(CoordinateError::AgentTraceDbUnavailable)?;

    let snapshot = GitSnapshotService::new(repository_root)
        .await
        .map_err(CoordinateError::Other)?;

    coordinate_boundary_inner(
        &db,
        &snapshot,
        pin_lease,
        worktree_id,
        boundary,
        inherited_external_taint,
        after_load,
        after_recovery,
    )
    .await
}

pub(super) async fn coordinate_on_held_worktree<P>(
    repository_root: &Path,
    pin_lease: WorktreeLockLease,
    worktree_id: &WorktreeId,
    boundary: &RuntimeBoundary,
    open_db: P,
    force_recovery: bool,
) -> Result<CoordinateOutcome, CoordinateError>
where
    P: std::ops::AsyncFnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
{
    coordinate_protected(
        repository_root,
        pin_lease,
        worktree_id,
        boundary,
        open_db,
        force_recovery,
        |_attempt| {},
        |_attempt| Ok(()),
    )
    .await
}

fn protected_worktree_failure(error: ProtectedWorktreeError) -> CoordinateError {
    match error {
        ProtectedWorktreeError::GitDirResolution(source)
        | ProtectedWorktreeError::CheckoutIdentity(source) => CoordinateError::Other(source),
        ProtectedWorktreeError::LockAcquisition(source) => {
            CoordinateError::LockAcquisition(anyhow::Error::new(source))
        }
        ProtectedWorktreeError::ExternalTaintMarker { operation, source } => {
            CoordinateError::ExternalTaintMarker { operation, source }
        }
    }
}

async fn capture_and_pin<C>(
    capture: &C,
    pin_lease: WorktreeLockLease,
    worktree_id: &WorktreeId,
) -> Result<TreeId>
where
    C: SnapshotCapture,
{
    let tree = capture.capture().await?;
    capture.pin(pin_lease, worktree_id, &tree).await?;
    Ok(tree)
}

#[allow(clippy::too_many_arguments)]
async fn coordinate_boundary_inner<C, AfterLoad, AfterRecovery>(
    db: &RepositoryAgentTraceDb,
    capture: &C,
    pin_lease: WorktreeLockLease,
    worktree_id: &WorktreeId,
    boundary: &RuntimeBoundary,
    inherited_external_taint: bool,
    mut after_load: AfterLoad,
    mut after_recovery: AfterRecovery,
) -> Result<CoordinateOutcome, CoordinateError>
where
    C: SnapshotCapture,
    AfterLoad: FnMut(u32),
    AfterRecovery: FnMut(u32) -> Result<()>,
{
    let store = MutationTraceStore::new(db);

    let observed_tree = match capture_and_pin(capture, pin_lease, worktree_id).await {
        Ok(tree) => tree,
        Err(source) => return Err(handle_snapshot_failure(&store, worktree_id, source).await),
    };

    store
        .initialize_worktree(worktree_id, &observed_tree)
        .await
        .map_err(CoordinateError::Other)?;

    let registered_scope = match hook_identity(boundary) {
        Some((scope, actor_kind)) => Some(
            store
                .register_scope(scope, worktree_id, actor_kind)
                .await
                .map_err(CoordinateError::ScopeIdentityConflict)?,
        ),
        None => None,
    };

    register_start_provenance(&store, boundary, registered_scope.as_ref()).await?;

    let type_boundary = into_protocol_boundary(boundary, worktree_id);
    let scope_ref = types::boundary_scope(&type_boundary);
    let event_key_ref = types::boundary_event_key(&type_boundary);

    let mut external_taint_pending = inherited_external_taint;

    for attempt_index in 0..MAX_CAS_RETRY_ATTEMPTS {
        let Some(projection) = store
            .load_worktree(worktree_id, scope_ref.as_ref(), event_key_ref.as_ref())
            .await
            .map_err(CoordinateError::Other)?
        else {
            return Err(CoordinateError::Other(anyhow::anyhow!(
                "worktree {worktree_id:?} missing durable state immediately after initialize_worktree"
            )));
        };

        after_load(attempt_index);

        let mut state = projection.into_protocol_state();

        if external_taint_pending {
            state = protocol::database_failure(&state, worktree_id);
        }

        if needs_recovery(&state, worktree_id) {
            let recovered = protocol::recover(&state, worktree_id, observed_tree.clone());
            let Some(transition) = DurableTransition::between(&state, &recovered, worktree_id)
                .map_err(CoordinateError::Other)?
            else {
                let revision = state
                    .worktrees
                    .get(worktree_id)
                    .map(|worktree_state| worktree_state.revision)
                    .unwrap_or_default();
                return Err(CoordinateError::RevisionExhausted {
                    worktree_id: worktree_id.clone(),
                    revision,
                });
            };

            match store
                .commit(&transition)
                .await
                .map_err(CoordinateError::Other)?
            {
                CasResult::Applied => {
                    state = recovered;
                    external_taint_pending = false;
                    after_recovery(attempt_index).map_err(CoordinateError::Other)?;
                }
                CasResult::Conflict => continue,
            }
        }

        let attempt = AttemptId(Uuid::new_v4().to_string());
        let prepared = protocol::prepare(
            &state,
            attempt.clone(),
            type_boundary.clone(),
            observed_tree.clone(),
        );
        let outcome = protocol::commit(&prepared, &attempt);

        match DurableTransition::between(&state, &outcome.state, worktree_id)
            .map_err(CoordinateError::Other)?
        {
            Some(transition) => match store
                .commit(&transition)
                .await
                .map_err(CoordinateError::Other)?
            {
                CasResult::Applied => {
                    return Ok(build_outcome(worktree_id, observed_tree, &outcome))
                }
                CasResult::Conflict => {}
            },
            None => return Ok(build_outcome(worktree_id, observed_tree, &outcome)),
        }
    }

    Err(CoordinateError::CasConflictExhausted {
        attempts: MAX_CAS_RETRY_ATTEMPTS,
    })
}

/// Applies the admission-bounded provenance rule for a `Start` carrying
/// provenance, using the durable [`types::ScopeState`] that `register_scope`
/// just returned.
///
/// An existing provenance row is always re-registered, so the store's immutable
/// session identity and first-observed model semantics stay authoritative on
/// every replay. A row may only be *created* while the durable scope is still
/// `NeverSeen` — a retry whose earlier attempt never committed the protocol
/// `Start`. Once the scope has crossed protocol admission, absent provenance
/// stays absent permanently: provenance describes its scope as observed at
/// admission, so it is never attached retroactively.
async fn register_start_provenance(
    store: &MutationTraceStore<'_>,
    boundary: &RuntimeBoundary,
    registered_scope: Option<&types::ScopeState>,
) -> Result<(), CoordinateError> {
    let Some((scope, provenance)) = start_provenance(boundary) else {
        return Ok(());
    };

    let stored = store
        .load_scope_provenance(scope)
        .await
        .map_err(CoordinateError::ScopeProvenanceRegistration)?;
    let before_admission =
        registered_scope.is_some_and(|scope_state| scope_state.status == ScopeStatus::NeverSeen);

    if stored.is_none() && !before_admission {
        return Ok(());
    }

    store
        .register_scope_provenance(&ScopeProvenance {
            scope_id: scope.clone(),
            session_id: provenance.session_id.clone(),
            model_id: provenance.model_id.clone(),
        })
        .await
        .map_err(CoordinateError::ScopeProvenanceRegistration)?;

    Ok(())
}

fn needs_recovery(state: &types::ProtocolState, worktree_id: &WorktreeId) -> bool {
    state.external_taint.contains(worktree_id)
        || state
            .worktrees
            .get(worktree_id)
            .is_some_and(|worktree_state| {
                worktree_state.failure_kind != FailureKind::Healthy
                    || worktree_state.needs_rebaseline
            })
}

fn build_outcome(
    worktree_id: &WorktreeId,
    observed_tree: TreeId,
    outcome: &protocol::CommitOutcome,
) -> CoordinateOutcome {
    let revision = outcome
        .state
        .worktrees
        .get(worktree_id)
        .map(|worktree_state| worktree_state.revision)
        .expect("the coordinated worktree's durable state must still be present after commit");
    let mutation_event = outcome.state.mutation_events.iter().next().cloned();

    CoordinateOutcome {
        worktree_id: worktree_id.clone(),
        observed_tree,
        revision,
        evaluation: outcome.evaluation,
        mutation_event,
    }
}

fn into_protocol_boundary(boundary: &RuntimeBoundary, worktree_id: &WorktreeId) -> Boundary {
    match boundary {
        RuntimeBoundary::Start { scope, event, .. } => Boundary::Start {
            scope: scope.clone(),
            event: event.clone(),
        },
        RuntimeBoundary::Advance { scope, event, .. } => Boundary::Advance {
            scope: scope.clone(),
            event: event.clone(),
        },
        RuntimeBoundary::Close { scope, event, .. } => Boundary::Close {
            scope: scope.clone(),
            event: event.clone(),
        },
        RuntimeBoundary::Flush => Boundary::Flush {
            worktree: worktree_id.clone(),
        },
    }
}

fn start_provenance(boundary: &RuntimeBoundary) -> Option<(&ScopeId, &StartProvenance)> {
    match boundary {
        RuntimeBoundary::Start {
            scope,
            provenance: Some(provenance),
            ..
        } => Some((scope, provenance)),
        _ => None,
    }
}

fn hook_identity(boundary: &RuntimeBoundary) -> Option<(&ScopeId, ActorKind)> {
    match boundary {
        RuntimeBoundary::Start {
            scope, actor_kind, ..
        }
        | RuntimeBoundary::Advance {
            scope, actor_kind, ..
        }
        | RuntimeBoundary::Close {
            scope, actor_kind, ..
        } => Some((scope, *actor_kind)),
        RuntimeBoundary::Flush => None,
    }
}

async fn handle_snapshot_failure(
    store: &MutationTraceStore<'_>,
    worktree_id: &WorktreeId,
    source: anyhow::Error,
) -> CoordinateError {
    match run_taint_retry_loop(store, worktree_id).await {
        Ok(persisted_taint) => CoordinateError::SnapshotFailure {
            persisted_taint,
            source,
        },
        Err(db_err) => CoordinateError::Other(db_err),
    }
}

async fn run_taint_retry_loop(
    store: &MutationTraceStore<'_>,
    worktree_id: &WorktreeId,
) -> Result<bool> {
    run_taint_retry_loop_inner(store, worktree_id, |_attempt| {}).await
}

async fn run_taint_retry_loop_inner<F>(
    store: &MutationTraceStore<'_>,
    worktree_id: &WorktreeId,
    mut after_load: F,
) -> Result<bool>
where
    F: FnMut(u32),
{
    for attempt in 0..MAX_CAS_RETRY_ATTEMPTS {
        let Some(projection) = store.load_worktree(worktree_id, None, None).await? else {
            return Ok(false);
        };
        after_load(attempt);

        let state = projection.into_protocol_state();
        let tainted_state = protocol::taint(&state, worktree_id);
        match DurableTransition::between(&state, &tainted_state, worktree_id)? {
            None => {
                let currently_unhealthy =
                    state
                        .worktrees
                        .get(worktree_id)
                        .is_some_and(|worktree_state| {
                            worktree_state.failure_kind != FailureKind::Healthy
                        });
                return Ok(currently_unhealthy);
            }
            Some(transition) => match store.commit(&transition).await? {
                CasResult::Applied => return Ok(true),
                CasResult::Conflict => {}
            },
        }
    }
    Ok(false)
}
