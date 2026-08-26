//! Pure transition logic for the mutation-cursor protocol.
//!
//! Refines `liveScopesOn`/`attributionFor` (`spec/mutation_cursor.qnt:265-301`),
//! `mkMutationEvent` (`spec/mutation_cursor.qnt:303-323`),
//! `prepareAvailable`/`prepare`/`commitAttempt`
//! (`spec/mutation_cursor.qnt:417-661`), and
//! `taintHealthy`/`taint`/`recordDatabaseFailure`/`databaseFailure`
//! (`spec/mutation_cursor.qnt:663-737`). Every function here takes and
//! returns plain [`super::types::ProtocolState`] values; none performs Git,
//! database, filesystem, environment, network, async, or lock I/O.

use std::collections::BTreeSet;

use super::types::{
    boundary_event_key, boundary_scope, boundary_worktree, is_advance, is_close, is_flush, is_hook,
    is_start, AttemptId, AttemptState, AttemptStatus, Attribution, Boundary, FailureKind,
    MutationEvent, ProtocolState, ScopeId, ScopeStatus, TreeId, WorktreeId, WorktreeState,
};

/// The live scopes belonging to `worktree`, read from `state`. Refines
/// `liveScopesOn` (`spec/mutation_cursor.qnt:265-269`).
///
/// The Quint function filters a fixed `SCOPES` universe by
/// `worktreeId == worktree and isLive(status)`. This refinement's `ScopeId`
/// space is unbounded (see `types.rs`'s module doc comment), so it filters
/// the known `state.scopes` map by the same predicate instead.
pub fn live_scopes_on(state: &ProtocolState, worktree: &WorktreeId) -> BTreeSet<ScopeId> {
    state
        .scopes
        .iter()
        .filter(|(_, scope_state)| scope_state.worktree_id == *worktree && scope_state.is_live())
        .map(|(scope_id, _)| scope_id.clone())
        .collect()
}

/// The mutation-evidence attribution for `worktree`, read from `state`.
/// Refines `attributionFor` (`spec/mutation_cursor.qnt:285-301`).
///
/// `IneligibleUnscoped` when the worktree is unhealthy, externally tainted,
/// needs rebaseline, or has no live scopes; `AiExclusive` for exactly one
/// live scope; `AiContended` for more than one. An unresolvable worktree (no
/// durable state) also yields `IneligibleUnscoped`, matching the "no live
/// scopes" case, since the Quint model's `worktree` always resolves within
/// its finite domain and has no equivalent missing case to refine.
pub fn attribution_for(state: &ProtocolState, worktree: &WorktreeId) -> Attribution {
    let live = live_scopes_on(state, worktree);
    let unhealthy = state
        .worktrees
        .get(worktree)
        .is_none_or(|w| w.failure_kind != FailureKind::Healthy || w.needs_rebaseline);

    if unhealthy || state.external_taint.contains(worktree) || live.is_empty() {
        Attribution::IneligibleUnscoped
    } else if live.len() == 1 {
        Attribution::AiExclusive(live.into_iter().next().expect("live has exactly one scope"))
    } else {
        Attribution::AiContended
    }
}

/// Prepares `attempt` against `boundary`, snapshotting the worktree's current
/// `revision`/`cursor_tree` as the attempt's CAS baseline and `observed_tree`
/// as its target `after_tree`. Refines `prepareAvailable`/`prepare`
/// (`spec/mutation_cursor.qnt:417-453`).
///
/// `observed_tree` corresponds to Quint's `worktreeTrees.get(worktree)`: the
/// currently observed tree at the boundary's resolved worktree, supplied by
/// the caller rather than read internally, since the pure kernel performs no
/// Git I/O.
///
/// A no-op (refining Quint's `stutter`) when `attempt` already exists with a
/// status other than `Available`, when the boundary's worktree cannot be
/// resolved (an unregistered scope for a hook boundary), or when that
/// worktree has no durable state.
pub fn prepare(
    state: &ProtocolState,
    attempt: AttemptId,
    boundary: Boundary,
    observed_tree: TreeId,
) -> ProtocolState {
    let already_underway = state
        .attempts
        .get(&attempt)
        .is_some_and(|existing| existing.status != AttemptStatus::Available);
    if already_underway {
        return state.clone();
    }

    let Some(worktree) = boundary_worktree(&boundary, &state.scopes) else {
        return state.clone();
    };
    let Some(worktree_state) = state.worktrees.get(&worktree) else {
        return state.clone();
    };

    let mut next = state.clone();
    next.attempts.insert(
        attempt,
        AttemptState {
            status: AttemptStatus::Prepared,
            boundary,
            expected_revision: worktree_state.revision,
            before_tree: worktree_state.cursor_tree.clone(),
            after_tree: observed_tree,
        },
    );
    next
}

/// The computed evaluation flags `commitAttempt` derives before applying its
/// state transition, exposed for callers that need them without
/// reconstructing them from the returned state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct CommitEvaluation {
    /// `fresh`: the attempt was `Prepared` against a CAS baseline
    /// (`expected_revision`/`before_tree`) that still matches the worktree's
    /// current `revision`/`cursor_tree`, the worktree is not externally
    /// tainted, and, for hook boundaries, the event has not already been
    /// processed.
    pub accepted: bool,
    /// Whether this boundary observes live protocol state for its scope
    /// (`Start` requires `NeverSeen`, `Advance` requires live, `Close`
    /// accepts `NeverSeen` or live, `Flush` is always `true`).
    pub observes: bool,
    /// `accepted and observes and before_tree != after_tree`.
    pub observed_change: bool,
    /// `observed_change and not needs_rebaseline`. Gates whether [`commit`]
    /// materializes a `MutationEvent` for this attempt.
    pub changed: bool,
    /// `accepted and (not is_flush(boundary) or observed_change)`.
    pub advances_revision: bool,
}

/// The result of evaluating and committing one prepared attempt: the
/// evaluation flags plus the resulting protocol state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitOutcome {
    pub evaluation: CommitEvaluation,
    pub state: ProtocolState,
}

/// Evaluates and commits `attempt`, refining `commitAttempt`
/// (`spec/mutation_cursor.qnt:455-661`) for all four boundary kinds
/// (`Start`/`Advance`/`Close`/`Flush`) in one pass.
///
/// On rejection (`accepted == false`), only the attempt's own status moves to
/// `Rejected` (or stays as-is if it was never `Prepared`); no other durable
/// state changes, so a rejected or stale attempt never advances the
/// revision, moves the cursor, marks its event processed, or emits mutation
/// evidence.
///
/// On acceptance, applies scope lifecycle transitions
/// (`NeverSeen`→`Active` on an accepted, observing `Start`; →`Closed` on an
/// accepted, observing `Close`), cursor advancement (`after_tree` when
/// `observes and not needs_rebaseline`, otherwise unchanged), revision
/// advancement and the attempt's `Committed` status, processed-event-key
/// recording for hook boundaries, and — when `changed` — materializes exactly
/// one `MutationEvent` (refining `mkMutationEvent`,
/// `spec/mutation_cursor.qnt:303-323`) whose `active_scopes`/`attribution`
/// are computed by [`live_scopes_on`]/[`attribution_for`] against the
/// **pre-transition** state passed into this call, exactly as `commitAttempt`
/// computes `live`/`attribution` before applying `nextScope`
/// (`spec/mutation_cursor.qnt:484-485` precede the `nextScope` `val` at line
/// 530): a `Start` boundary's emitted event never attributes the mutation to
/// the scope it is about to activate, and a `Close` boundary's emitted event
/// still attributes to the scope it is about to close.
///
/// A no-op (evaluation flags all `false`, state unchanged) when `attempt` has
/// no prepared record or its boundary's worktree cannot be resolved — an
/// attempt only reaches this state via [`prepare`], which already refuses to
/// prepare against an unresolvable worktree.
pub fn commit(state: &ProtocolState, attempt: &AttemptId) -> CommitOutcome {
    let Some(resolved) = ResolvedAttempt::resolve(state, attempt) else {
        return CommitOutcome {
            evaluation: CommitEvaluation::default(),
            state: state.clone(),
        };
    };
    let evaluation = resolved.evaluate(state);
    let state = resolved.apply(state, attempt, evaluation);
    CommitOutcome { evaluation, state }
}

/// The prepared attempt and its boundary's resolved worktree/scope context,
/// read once and shared by evaluation and application.
struct ResolvedAttempt {
    planned: AttemptState,
    boundary: Boundary,
    worktree: WorktreeId,
    worktree_state: WorktreeState,
    scope_id: Option<ScopeId>,
}

impl ResolvedAttempt {
    fn resolve(state: &ProtocolState, attempt: &AttemptId) -> Option<Self> {
        let planned = state.attempts.get(attempt)?.clone();
        let boundary = planned.boundary.clone();
        let worktree = boundary_worktree(&boundary, &state.scopes)?;
        let worktree_state = state.worktrees.get(&worktree)?.clone();
        let scope_id = boundary_scope(&boundary);
        Some(Self {
            planned,
            boundary,
            worktree,
            worktree_state,
            scope_id,
        })
    }

    /// Refines the `fresh`/`observes`/`accepted`/`observedChange`/`changed`/
    /// `advancesRevision` computation at `spec/mutation_cursor.qnt:462-483`.
    fn evaluate(&self, state: &ProtocolState) -> CommitEvaluation {
        let current_scope = self
            .scope_id
            .as_ref()
            .and_then(|scope_id| state.scopes.get(scope_id));

        let fresh = self.planned.status == AttemptStatus::Prepared
            && !state.external_taint.contains(&self.worktree)
            && self.planned.expected_revision == self.worktree_state.revision
            && self.planned.before_tree == self.worktree_state.cursor_tree
            && (!is_hook(&self.boundary)
                || boundary_event_key(&self.boundary)
                    .is_none_or(|key| !state.processed_events.contains(&key)));
        let observes = if is_start(&self.boundary) {
            current_scope.is_some_and(|s| s.status == ScopeStatus::NeverSeen)
        } else if is_advance(&self.boundary) {
            current_scope.is_some_and(super::types::ScopeState::is_live)
        } else if is_close(&self.boundary) {
            current_scope.is_some_and(|s| s.status == ScopeStatus::NeverSeen || s.is_live())
        } else {
            true
        };
        let accepted = fresh;
        let observed_change =
            accepted && observes && self.planned.before_tree != self.planned.after_tree;
        let changed = observed_change && !self.worktree_state.needs_rebaseline;
        let advances_revision = accepted && (!is_flush(&self.boundary) || observed_change);

        CommitEvaluation {
            accepted,
            observes,
            observed_change,
            changed,
            advances_revision,
        }
    }

    /// Refines the accepted/rejected state transition at
    /// `spec/mutation_cursor.qnt:503-660`.
    fn apply(
        &self,
        state: &ProtocolState,
        attempt: &AttemptId,
        evaluation: CommitEvaluation,
    ) -> ProtocolState {
        let mut next = state.clone();

        if !evaluation.accepted {
            if let Some(entry) = next.attempts.get_mut(attempt) {
                if entry.status == AttemptStatus::Prepared {
                    entry.status = AttemptStatus::Rejected;
                }
            }
            return next;
        }

        // `observes` already encodes the exact scope-status guard
        // `commitAttempt` repeats for its own scope transition (`NeverSeen`
        // for `Start`, `NeverSeen` or live for `Close`), so reusing it here
        // cannot diverge from the spec's separately-stated guard.
        if let Some(scope_id) = &self.scope_id {
            if is_start(&self.boundary) && evaluation.observes {
                if let Some(entry) = next.scopes.get_mut(scope_id) {
                    entry.status = ScopeStatus::Active;
                }
            } else if is_close(&self.boundary) && evaluation.observes {
                if let Some(entry) = next.scopes.get_mut(scope_id) {
                    entry.status = ScopeStatus::Closed;
                }
            }
        }

        let next_cursor = if evaluation.observes && !self.worktree_state.needs_rebaseline {
            self.planned.after_tree.clone()
        } else {
            self.worktree_state.cursor_tree.clone()
        };

        if evaluation.advances_revision {
            next.worktrees.insert(
                self.worktree.clone(),
                WorktreeState {
                    cursor_tree: next_cursor,
                    revision: self.worktree_state.revision + 1,
                    tainted: self.worktree_state.tainted,
                    failure_kind: self.worktree_state.failure_kind,
                    needs_rebaseline: self.worktree_state.needs_rebaseline,
                },
            );
        }

        if is_hook(&self.boundary) {
            if let Some(key) = boundary_event_key(&self.boundary) {
                next.processed_events.insert(key);
            }
        }

        if evaluation.changed {
            next.mutation_events.insert(MutationEvent {
                worktree_id: self.worktree.clone(),
                revision: self.worktree_state.revision + 1,
                before_tree: self.planned.before_tree.clone(),
                after_tree: self.planned.after_tree.clone(),
                active_scopes: live_scopes_on(state, &self.worktree),
                tainted: self.worktree_state.tainted,
                failure_kind: self.worktree_state.failure_kind,
                attribution: attribution_for(state, &self.worktree),
                boundary: self.boundary.clone(),
            });
        }

        if let Some(entry) = next.attempts.get_mut(attempt) {
            entry.status = AttemptStatus::Committed;
        }

        next
    }
}

/// Marks `worktree`'s Git snapshot capture as tainted by a snapshot failure.
/// Refines `taintHealthy`/`taint` (`spec/mutation_cursor.qnt:663-710`).
///
/// Sets `tainted=true` and `failure_kind=SnapshotFailure`, advances
/// `revision` by one, and leaves `cursor_tree`/`needs_rebaseline` untouched.
/// A guarded no-op (refining Quint's `stutter`) when `worktree` is already
/// `tainted`, already in `external_taint`, or has no durable state.
///
/// The last case has no Quint counterpart: `WorktreeId` ranges over the
/// finite `WORKTREES` universe there, and `init` materializes a
/// `WorktreeState` for every member, so every `WorktreeId` already resolves.
/// This refinement's `WorktreeId` is an unbounded runtime value, so an
/// unknown worktree is unresolved kernel input rather than a state `taint`
/// may create — the same existence contract [`database_failure`] enforces.
pub fn taint(state: &ProtocolState, worktree: &WorktreeId) -> ProtocolState {
    let Some(worktree_state) = state.worktrees.get(worktree) else {
        return state.clone();
    };
    if worktree_state.tainted || state.external_taint.contains(worktree) {
        return state.clone();
    }

    let mut next = state.clone();
    next.worktrees.insert(
        worktree.clone(),
        WorktreeState {
            cursor_tree: worktree_state.cursor_tree.clone(),
            revision: worktree_state.revision + 1,
            tainted: true,
            failure_kind: FailureKind::SnapshotFailure,
            needs_rebaseline: worktree_state.needs_rebaseline,
        },
    );
    next
}

/// Records a database failure for `worktree` by adding it to
/// `external_taint`. Refines `recordDatabaseFailure`/`databaseFailure`
/// (`spec/mutation_cursor.qnt:712-737`).
///
/// Changes `external_taint` only; every other durable worktree/scope field
/// stays as it was. A guarded no-op (refining Quint's `stutter`) when
/// `worktree` is already in `external_taint` or has no durable state.
///
/// Quint's `WorktreeId` ranges over the finite `WORKTREES` universe, and
/// `init` materializes a `WorktreeState` for every member, so
/// `recordDatabaseFailure` has no explicit existence guard because there is
/// no state for it to guard against — every `WorktreeId` already resolves.
/// This refinement's `WorktreeId` is an unbounded runtime value, so a
/// referenced worktree must already exist in `ProtocolState.worktrees`
/// before this action may operate on it; an unknown `WorktreeId` is
/// unresolved kernel input, not a worktree this action may bring into
/// existence, and keeping `external_taint` a subset of known worktrees
/// matches `taint`'s own existence guard.
pub fn database_failure(state: &ProtocolState, worktree: &WorktreeId) -> ProtocolState {
    if !state.worktrees.contains_key(worktree) || state.external_taint.contains(worktree) {
        return state.clone();
    }

    let mut next = state.clone();
    next.external_taint.insert(worktree.clone());
    next
}
