use std::collections::BTreeSet;

use super::types::{
    boundary_event_key, boundary_scope, boundary_worktree, is_advance, is_close, is_flush, is_hook,
    is_start, ActorKind, AttemptId, AttemptState, AttemptStatus, Attribution, Boundary,
    FailureKind, MutationEvent, ProtocolState, ScopeId, ScopeState, ScopeStatus, TreeId,
    WorktreeId, WorktreeState,
};

fn next_revision(revision: u64) -> Option<u64> {
    revision.checked_add(1)
}

pub fn live_scopes_on(state: &ProtocolState, worktree: &WorktreeId) -> BTreeSet<ScopeId> {
    state
        .scopes
        .iter()
        .filter(|(_, scope_state)| scope_state.worktree_id == *worktree && scope_state.is_live())
        .map(|(scope_id, _)| scope_id.clone())
        .collect()
}

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

pub fn requires_boundary_confirmation(actor_kind: ActorKind) -> bool {
    match actor_kind {
        ActorKind::Codex | ActorKind::OpenCode | ActorKind::Pi => true,
        ActorKind::ClaudeCode => false,
    }
}

pub fn scope_requires_confirmation(state: &ProtocolState, scope: &ScopeId) -> bool {
    state
        .scopes
        .get(scope)
        .is_some_and(|scope_state| requires_boundary_confirmation(scope_state.actor_kind))
}

pub fn boundary_confirms_scope(boundary: &Boundary, scope: &ScopeId) -> bool {
    is_close(boundary) && boundary_scope(boundary).as_ref() == Some(scope)
}

pub fn has_unconfirmed_required_scope(
    state: &ProtocolState,
    live: &BTreeSet<ScopeId>,
    boundary: &Boundary,
) -> bool {
    live.iter().any(|scope| {
        scope_requires_confirmation(state, scope) && !boundary_confirms_scope(boundary, scope)
    })
}

pub fn boundary_closes_never_seen_scope(state: &ProtocolState, boundary: &Boundary) -> bool {
    is_close(boundary)
        && boundary_scope(boundary)
            .and_then(|scope| state.scopes.get(&scope))
            .is_some_and(|scope_state| scope_state.status == ScopeStatus::NeverSeen)
}

pub fn attribution_for_boundary(
    state: &ProtocolState,
    worktree: &WorktreeId,
    boundary: &Boundary,
) -> Attribution {
    if boundary_closes_never_seen_scope(state, boundary) {
        return Attribution::IneligibleUnscoped;
    }
    let live = live_scopes_on(state, worktree);
    if has_unconfirmed_required_scope(state, &live, boundary) {
        return Attribution::IneligibleUnscoped;
    }
    attribution_for(state, worktree)
}

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct CommitEvaluation {
    pub accepted: bool,
    pub observes: bool,
    pub observed_change: bool,
    pub changed: bool,
    pub advances_revision: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitOutcome {
    pub evaluation: CommitEvaluation,
    pub state: ProtocolState,
}

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

        let tree_changed = observes && self.planned.before_tree != self.planned.after_tree;
        let would_advance_revision = !is_flush(&self.boundary) || tree_changed;
        let has_revision_headroom =
            !would_advance_revision || next_revision(self.worktree_state.revision).is_some();

        let accepted = fresh && has_revision_headroom;
        let observed_change = accepted && tree_changed;
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

        let advanced_revision = if evaluation.advances_revision {
            Some(
                next_revision(self.worktree_state.revision)
                    .expect("advancing revision requires headroom, see `evaluate`"),
            )
        } else {
            None
        };

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

        if let Some(advanced_revision) = advanced_revision {
            next.worktrees.insert(
                self.worktree.clone(),
                WorktreeState {
                    cursor_tree: next_cursor,
                    revision: advanced_revision,
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
            let revision = advanced_revision.expect("changed implies advances_revision");
            next.mutation_events.insert(MutationEvent {
                worktree_id: self.worktree.clone(),
                revision,
                before_tree: self.planned.before_tree.clone(),
                after_tree: self.planned.after_tree.clone(),
                active_scopes: live_scopes_on(state, &self.worktree),
                tainted: self.worktree_state.tainted,
                failure_kind: self.worktree_state.failure_kind,
                attribution: attribution_for_boundary(state, &self.worktree, &self.boundary),
                boundary: self.boundary.clone(),
            });
        }

        if let Some(entry) = next.attempts.get_mut(attempt) {
            entry.status = AttemptStatus::Committed;
        }

        next
    }
}

pub fn taint(state: &ProtocolState, worktree: &WorktreeId) -> ProtocolState {
    let Some(worktree_state) = state.worktrees.get(worktree) else {
        return state.clone();
    };
    if worktree_state.failure_kind != FailureKind::Healthy
        || state.external_taint.contains(worktree)
    {
        return state.clone();
    }
    let Some(next_rev) = next_revision(worktree_state.revision) else {
        return state.clone();
    };

    let mut next = state.clone();
    next.worktrees.insert(
        worktree.clone(),
        WorktreeState {
            cursor_tree: worktree_state.cursor_tree.clone(),
            revision: next_rev,
            tainted: true,
            failure_kind: FailureKind::SnapshotFailure,
            needs_rebaseline: worktree_state.needs_rebaseline,
        },
    );
    next
}

pub fn database_failure(state: &ProtocolState, worktree: &WorktreeId) -> ProtocolState {
    if !state.worktrees.contains_key(worktree) || state.external_taint.contains(worktree) {
        return state.clone();
    }

    let mut next = state.clone();
    next.external_taint.insert(worktree.clone());
    next
}

pub fn abandon(state: &ProtocolState, scope: &ScopeId) -> ProtocolState {
    let Some(scope_state) = state.scopes.get(scope) else {
        return state.clone();
    };
    if !scope_state.is_live() || state.external_taint.contains(&scope_state.worktree_id) {
        return state.clone();
    }
    let Some(worktree_state) = state.worktrees.get(&scope_state.worktree_id) else {
        return state.clone();
    };
    let Some(next_rev) = next_revision(worktree_state.revision) else {
        return state.clone();
    };

    let mut next = state.clone();
    next.worktrees.insert(
        scope_state.worktree_id.clone(),
        WorktreeState {
            cursor_tree: worktree_state.cursor_tree.clone(),
            revision: next_rev,
            tainted: worktree_state.tainted,
            failure_kind: worktree_state.failure_kind,
            needs_rebaseline: true,
        },
    );
    next.scopes.insert(
        scope.clone(),
        ScopeState {
            status: ScopeStatus::Abandoned,
            actor_kind: scope_state.actor_kind,
            worktree_id: scope_state.worktree_id.clone(),
        },
    );
    next
}

pub fn recover(
    state: &ProtocolState,
    worktree: &WorktreeId,
    observed_tree: TreeId,
) -> ProtocolState {
    let Some(worktree_state) = state.worktrees.get(worktree) else {
        return state.clone();
    };
    let externally_tainted = state.external_taint.contains(worktree);
    let unhealthy = worktree_state.failure_kind != FailureKind::Healthy;
    if !unhealthy && !externally_tainted && !worktree_state.needs_rebaseline {
        return state.clone();
    }
    let Some(next_rev) = next_revision(worktree_state.revision) else {
        return state.clone();
    };

    let abandon_live_scopes = unhealthy || externally_tainted;

    let mut next = state.clone();
    next.worktrees.insert(
        worktree.clone(),
        WorktreeState {
            cursor_tree: observed_tree,
            revision: next_rev,
            tainted: false,
            failure_kind: FailureKind::Healthy,
            needs_rebaseline: false,
        },
    );
    next.external_taint.remove(worktree);

    if abandon_live_scopes {
        for (scope_id, scope_state) in &state.scopes {
            if scope_state.worktree_id == *worktree && scope_state.is_live() {
                next.scopes.insert(
                    scope_id.clone(),
                    ScopeState {
                        status: ScopeStatus::Abandoned,
                        actor_kind: scope_state.actor_kind,
                        worktree_id: scope_state.worktree_id.clone(),
                    },
                );
            }
        }
    }

    next
}
