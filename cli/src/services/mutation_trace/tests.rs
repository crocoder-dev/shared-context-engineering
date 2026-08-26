use std::collections::{BTreeMap, BTreeSet};

use super::protocol::{
    abandon, attribution_for, commit, database_failure, live_scopes_on, prepare, recover, taint,
    CommitOutcome,
};
use super::types::*;

fn worktree(id: &str) -> WorktreeId {
    WorktreeId(id.to_string())
}

fn scope(id: &str) -> ScopeId {
    ScopeId(id.to_string())
}

fn event(id: &str) -> EventId {
    EventId(id.to_string())
}

fn tree(id: &str) -> TreeId {
    TreeId(id.to_string())
}

fn attempt_id(id: &str) -> AttemptId {
    AttemptId(id.to_string())
}

fn healthy_worktree(cursor_tree: TreeId, revision: u64) -> WorktreeState {
    WorktreeState {
        cursor_tree,
        revision,
        tainted: false,
        failure_kind: FailureKind::Healthy,
        needs_rebaseline: false,
    }
}

fn scope_with_status(status: ScopeStatus, worktree_id: WorktreeId) -> ScopeState {
    ScopeState {
        status,
        actor_kind: ActorKind::Codex,
        worktree_id,
    }
}

fn start_boundary() -> Boundary {
    Boundary::Start {
        scope: scope("scope0"),
        event: event("event0"),
    }
}

fn advance_boundary() -> Boundary {
    Boundary::Advance {
        scope: scope("scope0"),
        event: event("event1"),
    }
}

fn close_boundary() -> Boundary {
    Boundary::Close {
        scope: scope("scope0"),
        event: event("event2"),
    }
}

fn flush_boundary() -> Boundary {
    Boundary::Flush {
        worktree: worktree("wt0"),
    }
}

fn scopes() -> BTreeMap<ScopeId, ScopeState> {
    BTreeMap::from([
        (
            scope("scope0"),
            ScopeState {
                status: ScopeStatus::Active,
                actor_kind: ActorKind::Codex,
                worktree_id: worktree("wt0"),
            },
        ),
        (
            scope("scope1"),
            ScopeState {
                status: ScopeStatus::Active,
                actor_kind: ActorKind::ClaudeCode,
                worktree_id: worktree("wt1"),
            },
        ),
    ])
}

/// Prepares and immediately commits `attempt` against `boundary`, the shape
/// every state-sequence test below chains repeatedly.
fn prepare_and_commit(
    state: &ProtocolState,
    attempt: &AttemptId,
    boundary: Boundary,
    observed_tree: TreeId,
) -> CommitOutcome {
    let prepared = prepare(state, attempt.clone(), boundary, observed_tree);
    commit(&prepared, attempt)
}

#[test]
fn is_live_holds_only_for_active() {
    assert!(!is_live(ScopeStatus::NeverSeen));
    assert!(is_live(ScopeStatus::Active));
    assert!(!is_live(ScopeStatus::Closed));
    assert!(!is_live(ScopeStatus::Abandoned));
}

#[test]
fn is_terminal_holds_only_for_closed_or_abandoned() {
    assert!(!is_terminal(ScopeStatus::NeverSeen));
    assert!(!is_terminal(ScopeStatus::Active));
    assert!(is_terminal(ScopeStatus::Closed));
    assert!(is_terminal(ScopeStatus::Abandoned));
}

#[test]
fn scope_state_accessors_mirror_stored_fields() {
    let state = ScopeState {
        status: ScopeStatus::Active,
        actor_kind: ActorKind::Codex,
        worktree_id: worktree("wt1"),
    };
    assert_eq!(state.scope_worktree(), worktree("wt1"));
    assert_eq!(state.scope_actor(), ActorKind::Codex);
    assert!(state.is_live());
    assert!(!state.is_terminal());
}

#[test]
fn boundary_worktree_looks_up_the_scope_named_by_the_boundary() {
    let scopes = scopes();

    // Start/Advance/Close all name scope0, which the map assigns to wt0.
    // The presence of scope1 on a different worktree must not affect this.
    assert_eq!(
        boundary_worktree(&start_boundary(), &scopes),
        Some(worktree("wt0"))
    );
    assert_eq!(
        boundary_worktree(&advance_boundary(), &scopes),
        Some(worktree("wt0"))
    );
    assert_eq!(
        boundary_worktree(&close_boundary(), &scopes),
        Some(worktree("wt0"))
    );
}

#[test]
fn boundary_worktree_is_keyed_by_the_boundarys_own_scope_id() {
    // A boundary naming a different scope resolves to that scope's own
    // worktree, proving the lookup is keyed by the boundary's ScopeId rather
    // than an arbitrary caller-supplied worktree.
    let scopes = scopes();
    let boundary = Boundary::Start {
        scope: scope("scope1"),
        event: event("event0"),
    };
    assert_eq!(boundary_worktree(&boundary, &scopes), Some(worktree("wt1")));
}

#[test]
fn boundary_worktree_is_none_for_a_scope_missing_from_the_map() {
    let scopes = scopes();
    let boundary = Boundary::Start {
        scope: scope("missing_scope"),
        event: event("event0"),
    };
    assert_eq!(boundary_worktree(&boundary, &scopes), None);
}

#[test]
fn boundary_worktree_resolves_directly_for_flush_ignoring_the_scope_map() {
    let empty_scopes = BTreeMap::new();
    assert_eq!(
        boundary_worktree(&flush_boundary(), &empty_scopes),
        Some(worktree("wt0"))
    );
}

#[test]
fn boundary_scope_and_event_are_none_only_for_flush() {
    for boundary in [start_boundary(), advance_boundary(), close_boundary()] {
        assert!(boundary_scope(&boundary).is_some());
        assert!(boundary_event(&boundary).is_some());
        assert!(boundary_event_key(&boundary).is_some());
    }
    assert_eq!(boundary_scope(&flush_boundary()), None);
    assert_eq!(boundary_event(&flush_boundary()), None);
    assert_eq!(boundary_event_key(&flush_boundary()), None);
}

#[test]
fn boundary_event_key_pairs_the_boundarys_own_scope_and_event() {
    let key = boundary_event_key(&start_boundary()).expect("start boundary has an event key");
    assert_eq!(key.scope_id, scope("scope0"));
    assert_eq!(key.event_id, event("event0"));
}

#[test]
fn is_hook_holds_for_start_advance_close_but_not_flush() {
    assert!(is_hook(&start_boundary()));
    assert!(is_hook(&advance_boundary()));
    assert!(is_hook(&close_boundary()));
    assert!(!is_hook(&flush_boundary()));
}

#[test]
fn boundary_kind_predicates_are_mutually_exclusive() {
    let boundaries = [
        start_boundary(),
        advance_boundary(),
        close_boundary(),
        flush_boundary(),
    ];
    for boundary in &boundaries {
        let flags = [
            is_start(boundary),
            is_advance(boundary),
            is_close(boundary),
            is_flush(boundary),
        ];
        assert_eq!(
            flags.iter().filter(|flag| **flag).count(),
            1,
            "exactly one predicate should hold for {boundary:?}"
        );
    }
    assert!(is_start(&boundaries[0]));
    assert!(is_advance(&boundaries[1]));
    assert!(is_close(&boundaries[2]));
    assert!(is_flush(&boundaries[3]));
}

#[test]
fn worktree_state_and_attempt_state_construct_and_compare() {
    let a = WorktreeState {
        cursor_tree: tree("tree0"),
        revision: 0,
        tainted: false,
        failure_kind: FailureKind::Healthy,
        needs_rebaseline: false,
    };
    let b = a.clone();
    assert_eq!(a, b);

    let attempt = AttemptState {
        status: AttemptStatus::Available,
        boundary: start_boundary(),
        expected_revision: 0,
        before_tree: tree("tree0"),
        after_tree: tree("tree1"),
    };
    assert_eq!(attempt.status, AttemptStatus::Available);
    assert_eq!(attempt.expected_revision, 0);
}

#[test]
fn mutation_event_carries_attribution_and_active_scopes() {
    let mut active_scopes = std::collections::BTreeSet::new();
    active_scopes.insert(scope("scope0"));

    let mutation_event = MutationEvent {
        worktree_id: worktree("wt0"),
        revision: 1,
        before_tree: tree("tree0"),
        after_tree: tree("tree1"),
        active_scopes: active_scopes.clone(),
        tainted: false,
        failure_kind: FailureKind::Healthy,
        attribution: Attribution::AiExclusive(scope("scope0")),
        boundary: close_boundary(),
    };

    assert_eq!(mutation_event.active_scopes, active_scopes);
    assert_eq!(
        mutation_event.attribution,
        Attribution::AiExclusive(scope("scope0"))
    );
}

/// Final semantic check: proves the lookup is keyed by each boundary's own
/// `scope`, with no parameter through which a caller can independently
/// inject a worktree for a hook boundary.
#[test]
fn boundary_worktree_final_semantic_check() {
    let scopes = scopes();

    let start_scope0 = Boundary::Start {
        scope: scope("scope0"),
        event: event("event0"),
    };
    assert_eq!(
        boundary_worktree(&start_scope0, &scopes),
        Some(worktree("wt0"))
    );

    let start_scope1 = Boundary::Start {
        scope: scope("scope1"),
        event: event("event0"),
    };
    assert_eq!(
        boundary_worktree(&start_scope1, &scopes),
        Some(worktree("wt1"))
    );
}

#[test]
fn prepare_then_commit_accepts_a_fresh_start_and_activates_the_scope() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );

    let prepared = prepare(
        &state,
        attempt_id("attempt0"),
        start_boundary(),
        tree("tree1"),
    );
    let prepared_attempt = prepared
        .attempts
        .get(&attempt_id("attempt0"))
        .expect("attempt was prepared");
    assert_eq!(prepared_attempt.status, AttemptStatus::Prepared);
    assert_eq!(prepared_attempt.expected_revision, 0);
    assert_eq!(prepared_attempt.before_tree, tree("tree0"));
    assert_eq!(prepared_attempt.after_tree, tree("tree1"));

    let outcome = commit(&prepared, &attempt_id("attempt0"));
    assert!(outcome.evaluation.accepted);
    assert!(outcome.evaluation.observes);
    assert!(outcome.evaluation.observed_change);
    assert!(outcome.evaluation.changed);
    assert!(outcome.evaluation.advances_revision);

    assert_eq!(
        outcome
            .state
            .attempts
            .get(&attempt_id("attempt0"))
            .unwrap()
            .status,
        AttemptStatus::Committed
    );
    assert_eq!(
        outcome.state.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Active
    );
    let committed_worktree = outcome.state.worktrees.get(&worktree("wt0")).unwrap();
    assert_eq!(committed_worktree.revision, 1);
    assert_eq!(committed_worktree.cursor_tree, tree("tree1"));
    assert!(outcome.state.processed_events.contains(&EventKey {
        scope_id: scope("scope0"),
        event_id: event("event0"),
    }));
}

#[test]
fn commit_transitions_scope_to_closed_on_an_accepted_observing_close() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    let prepared = prepare(
        &state,
        attempt_id("attempt0"),
        close_boundary(),
        tree("tree1"),
    );
    let outcome = commit(&prepared, &attempt_id("attempt0"));

    assert!(outcome.evaluation.accepted);
    assert!(outcome.evaluation.observes);
    assert_eq!(
        outcome.state.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Closed
    );
    assert_eq!(
        outcome
            .state
            .worktrees
            .get(&worktree("wt0"))
            .unwrap()
            .cursor_tree,
        tree("tree1")
    );
}

#[test]
fn commit_rejects_a_stale_revision_attempt_without_mutating_state() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 1));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );
    state.attempts.insert(
        attempt_id("attempt0"),
        AttemptState {
            status: AttemptStatus::Prepared,
            boundary: start_boundary(),
            expected_revision: 0,
            before_tree: tree("tree0"),
            after_tree: tree("tree1"),
        },
    );
    let before = state.clone();

    let outcome = commit(&state, &attempt_id("attempt0"));

    assert!(!outcome.evaluation.accepted);
    assert!(!outcome.evaluation.observed_change);
    assert!(!outcome.evaluation.advances_revision);
    assert_eq!(outcome.state.worktrees, before.worktrees);
    assert_eq!(outcome.state.scopes, before.scopes);
    assert_eq!(outcome.state.processed_events, before.processed_events);
    assert_eq!(
        outcome
            .state
            .attempts
            .get(&attempt_id("attempt0"))
            .unwrap()
            .status,
        AttemptStatus::Rejected
    );
}

#[test]
fn commit_rejects_a_stale_before_tree_attempt_without_mutating_state() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree_current"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );
    state.attempts.insert(
        attempt_id("attempt0"),
        AttemptState {
            status: AttemptStatus::Prepared,
            boundary: start_boundary(),
            expected_revision: 0,
            before_tree: tree("tree_stale"),
            after_tree: tree("tree1"),
        },
    );
    let before = state.clone();

    let outcome = commit(&state, &attempt_id("attempt0"));

    assert!(!outcome.evaluation.accepted);
    assert_eq!(outcome.state.worktrees, before.worktrees);
    assert_eq!(outcome.state.scopes, before.scopes);
    assert_eq!(
        outcome
            .state
            .attempts
            .get(&attempt_id("attempt0"))
            .unwrap()
            .status,
        AttemptStatus::Rejected
    );
}

#[test]
fn commit_rejects_a_replayed_event_key_without_mutating_state() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );
    state.processed_events.insert(EventKey {
        scope_id: scope("scope0"),
        event_id: event("event0"),
    });
    state.attempts.insert(
        attempt_id("attempt0"),
        AttemptState {
            status: AttemptStatus::Prepared,
            boundary: start_boundary(),
            expected_revision: 0,
            before_tree: tree("tree0"),
            after_tree: tree("tree1"),
        },
    );
    let before = state.clone();

    let outcome = commit(&state, &attempt_id("attempt0"));

    assert!(!outcome.evaluation.accepted);
    assert_eq!(outcome.state.worktrees, before.worktrees);
    assert_eq!(outcome.state.scopes, before.scopes);
    assert_eq!(
        outcome
            .state
            .attempts
            .get(&attempt_id("attempt0"))
            .unwrap()
            .status,
        AttemptStatus::Rejected
    );
}

#[test]
fn commit_rejects_an_externally_tainted_worktree_even_with_a_fresh_revision_and_before_tree() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );
    state.external_taint.insert(worktree("wt0"));
    state.attempts.insert(
        attempt_id("attempt0"),
        AttemptState {
            status: AttemptStatus::Prepared,
            boundary: start_boundary(),
            expected_revision: 0,
            before_tree: tree("tree0"),
            after_tree: tree("tree1"),
        },
    );

    let outcome = commit(&state, &attempt_id("attempt0"));

    assert!(!outcome.evaluation.accepted);
    assert_eq!(
        outcome
            .state
            .worktrees
            .get(&worktree("wt0"))
            .unwrap()
            .revision,
        0
    );
    assert_eq!(
        outcome.state.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::NeverSeen
    );
}

#[test]
fn accepted_but_non_observing_start_on_an_already_active_scope_advances_revision_without_moving_cursor_or_scope(
) {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    let prepared = prepare(
        &state,
        attempt_id("attempt0"),
        start_boundary(),
        tree("tree1"),
    );
    let outcome = commit(&prepared, &attempt_id("attempt0"));

    assert!(outcome.evaluation.accepted);
    assert!(!outcome.evaluation.observes);
    assert!(!outcome.evaluation.observed_change);
    assert!(!outcome.evaluation.changed);
    assert!(outcome.evaluation.advances_revision);

    let committed_worktree = outcome.state.worktrees.get(&worktree("wt0")).unwrap();
    assert_eq!(committed_worktree.revision, 1);
    assert_eq!(committed_worktree.cursor_tree, tree("tree0"));
    assert_eq!(
        outcome.state.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Active
    );
    assert_eq!(
        outcome
            .state
            .attempts
            .get(&attempt_id("attempt0"))
            .unwrap()
            .status,
        AttemptStatus::Committed
    );
    assert!(outcome.state.processed_events.contains(&EventKey {
        scope_id: scope("scope0"),
        event_id: event("event0"),
    }));
}

#[test]
fn accepted_but_non_observing_advance_on_a_never_seen_scope_advances_revision_without_moving_cursor(
) {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );

    let prepared = prepare(
        &state,
        attempt_id("attempt0"),
        advance_boundary(),
        tree("tree1"),
    );
    let outcome = commit(&prepared, &attempt_id("attempt0"));

    assert!(outcome.evaluation.accepted);
    assert!(!outcome.evaluation.observes);
    assert!(!outcome.evaluation.changed);
    assert!(outcome.evaluation.advances_revision);
    assert_eq!(
        outcome
            .state
            .worktrees
            .get(&worktree("wt0"))
            .unwrap()
            .cursor_tree,
        tree("tree0")
    );
    assert_eq!(
        outcome.state.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::NeverSeen
    );
}

#[test]
fn accepted_but_non_observing_close_on_a_terminal_scope_advances_revision_without_reactivating_it()
{
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Abandoned, worktree("wt0")),
    );

    let prepared = prepare(
        &state,
        attempt_id("attempt0"),
        close_boundary(),
        tree("tree1"),
    );
    let outcome = commit(&prepared, &attempt_id("attempt0"));

    assert!(outcome.evaluation.accepted);
    assert!(!outcome.evaluation.observes);
    assert!(!outcome.evaluation.changed);
    assert!(outcome.evaluation.advances_revision);
    assert_eq!(
        outcome.state.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Abandoned
    );
}

#[test]
fn flush_does_not_advance_revision_on_a_no_op_tree_unlike_hook_boundaries() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));

    let prepared = prepare(
        &state,
        attempt_id("attempt0"),
        flush_boundary(),
        tree("tree0"),
    );
    let outcome = commit(&prepared, &attempt_id("attempt0"));

    assert!(outcome.evaluation.accepted);
    assert!(outcome.evaluation.observes);
    assert!(!outcome.evaluation.observed_change);
    assert!(!outcome.evaluation.changed);
    assert!(!outcome.evaluation.advances_revision);

    let worktree_state = outcome.state.worktrees.get(&worktree("wt0")).unwrap();
    assert_eq!(worktree_state.revision, 0);
    assert_eq!(
        outcome
            .state
            .attempts
            .get(&attempt_id("attempt0"))
            .unwrap()
            .status,
        AttemptStatus::Committed
    );
}

#[test]
fn flush_advances_revision_when_it_observes_a_real_tree_change() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));

    let prepared = prepare(
        &state,
        attempt_id("attempt0"),
        flush_boundary(),
        tree("tree1"),
    );
    let outcome = commit(&prepared, &attempt_id("attempt0"));

    assert!(outcome.evaluation.accepted);
    assert!(outcome.evaluation.observed_change);
    assert!(outcome.evaluation.advances_revision);
    let worktree_state = outcome.state.worktrees.get(&worktree("wt0")).unwrap();
    assert_eq!(worktree_state.revision, 1);
    assert_eq!(worktree_state.cursor_tree, tree("tree1"));
}

#[test]
fn live_scopes_on_filters_by_worktree_and_liveness() {
    let mut state = ProtocolState::default();
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );
    state.scopes.insert(
        scope("scope1"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );
    state.scopes.insert(
        scope("scope2"),
        scope_with_status(ScopeStatus::Active, worktree("wt1")),
    );

    let live = live_scopes_on(&state, &worktree("wt0"));
    assert_eq!(live, BTreeSet::from([scope("scope0")]));
}

#[test]
fn attribution_for_is_ineligible_unscoped_when_no_scope_is_live() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));

    assert_eq!(
        attribution_for(&state, &worktree("wt0")),
        Attribution::IneligibleUnscoped
    );
}

#[test]
fn attribution_for_is_ai_exclusive_for_exactly_one_live_scope() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    assert_eq!(
        attribution_for(&state, &worktree("wt0")),
        Attribution::AiExclusive(scope("scope0"))
    );
}

#[test]
fn attribution_for_is_ai_contended_for_multiple_live_scopes() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );
    state.scopes.insert(
        scope("scope1"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    assert_eq!(
        attribution_for(&state, &worktree("wt0")),
        Attribution::AiContended
    );
}

#[test]
fn attribution_for_is_ineligible_unscoped_when_worktree_has_a_snapshot_failure_even_with_an_active_scope(
) {
    let mut state = ProtocolState::default();
    state.worktrees.insert(
        worktree("wt0"),
        WorktreeState {
            cursor_tree: tree("tree0"),
            revision: 0,
            tainted: true,
            failure_kind: FailureKind::SnapshotFailure,
            needs_rebaseline: false,
        },
    );
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    assert_eq!(
        attribution_for(&state, &worktree("wt0")),
        Attribution::IneligibleUnscoped
    );
}

#[test]
fn attribution_for_is_ineligible_unscoped_when_worktree_is_externally_tainted_even_with_an_active_scope(
) {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.external_taint.insert(worktree("wt0"));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    assert_eq!(
        attribution_for(&state, &worktree("wt0")),
        Attribution::IneligibleUnscoped
    );
}

#[test]
fn attribution_for_is_ineligible_unscoped_when_worktree_needs_rebaseline_even_with_an_active_scope()
{
    let mut state = ProtocolState::default();
    state.worktrees.insert(
        worktree("wt0"),
        WorktreeState {
            cursor_tree: tree("tree0"),
            revision: 0,
            tainted: false,
            failure_kind: FailureKind::Healthy,
            needs_rebaseline: true,
        },
    );
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    assert_eq!(
        attribution_for(&state, &worktree("wt0")),
        Attribution::IneligibleUnscoped
    );
}

#[test]
fn commit_emits_no_mutation_event_for_a_no_op_tree_change() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );

    let prepared = prepare(
        &state,
        attempt_id("attempt0"),
        start_boundary(),
        tree("tree0"),
    );
    let outcome = commit(&prepared, &attempt_id("attempt0"));

    assert!(!outcome.evaluation.changed);
    assert!(outcome.state.mutation_events.is_empty());
}

#[test]
fn commit_emits_exactly_one_mutation_event_with_correct_attribution_boundary_and_revision_for_a_real_change(
) {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    // Advance never changes the scope set, so the pre- and post-transition
    // live scopes are identical: attribution is unambiguously AiExclusive.
    let prepared = prepare(
        &state,
        attempt_id("attempt0"),
        advance_boundary(),
        tree("tree1"),
    );
    let outcome = commit(&prepared, &attempt_id("attempt0"));

    assert!(outcome.evaluation.changed);
    assert_eq!(outcome.state.mutation_events.len(), 1);
    let event = outcome.state.mutation_events.iter().next().unwrap();
    assert_eq!(event.worktree_id, worktree("wt0"));
    assert_eq!(event.revision, 1);
    assert_eq!(event.before_tree, tree("tree0"));
    assert_eq!(event.after_tree, tree("tree1"));
    assert_eq!(event.boundary, advance_boundary());
    assert_eq!(event.attribution, Attribution::AiExclusive(scope("scope0")));
    assert_eq!(event.active_scopes, BTreeSet::from([scope("scope0")]));
}

#[test]
fn commit_start_on_a_never_seen_scope_that_also_observes_a_change_excludes_the_newly_activated_scope_from_attribution(
) {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );

    let prepared = prepare(
        &state,
        attempt_id("attempt0"),
        start_boundary(),
        tree("tree1"),
    );
    let outcome = commit(&prepared, &attempt_id("attempt0"));

    assert!(outcome.evaluation.changed);
    assert_eq!(
        outcome.state.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Active
    );
    assert_eq!(outcome.state.mutation_events.len(), 1);
    let event = outcome.state.mutation_events.iter().next().unwrap();
    assert_eq!(event.attribution, Attribution::IneligibleUnscoped);
    assert!(event.active_scopes.is_empty());
}

#[test]
fn commit_close_on_the_sole_live_scope_that_also_observes_a_change_still_counts_it_as_live_in_attribution(
) {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    let prepared = prepare(
        &state,
        attempt_id("attempt0"),
        close_boundary(),
        tree("tree1"),
    );
    let outcome = commit(&prepared, &attempt_id("attempt0"));

    assert!(outcome.evaluation.changed);
    assert_eq!(
        outcome.state.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Closed
    );
    assert_eq!(outcome.state.mutation_events.len(), 1);
    let event = outcome.state.mutation_events.iter().next().unwrap();
    assert_eq!(event.attribution, Attribution::AiExclusive(scope("scope0")));
    assert_eq!(event.active_scopes, BTreeSet::from([scope("scope0")]));
}

#[test]
fn taint_changes_exactly_tainted_failure_kind_and_revision() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 3));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    let next = taint(&state, &worktree("wt0"));

    let tainted = next.worktrees.get(&worktree("wt0")).unwrap();
    assert!(tainted.tainted);
    assert_eq!(tainted.failure_kind, FailureKind::SnapshotFailure);
    assert_eq!(tainted.revision, 4);
    assert_eq!(tainted.cursor_tree, tree("tree0"));
    assert!(!tainted.needs_rebaseline);

    assert_eq!(next.scopes, state.scopes);
    assert_eq!(next.external_taint, state.external_taint);
    assert_eq!(next.processed_events, state.processed_events);
    assert_eq!(next.attempts, state.attempts);
    assert_eq!(next.mutation_events, state.mutation_events);
}

#[test]
fn taint_is_a_no_op_when_already_tainted() {
    let mut state = ProtocolState::default();
    state.worktrees.insert(
        worktree("wt0"),
        WorktreeState {
            cursor_tree: tree("tree0"),
            revision: 0,
            tainted: true,
            failure_kind: FailureKind::SnapshotFailure,
            needs_rebaseline: false,
        },
    );

    let next = taint(&state, &worktree("wt0"));

    assert_eq!(next, state);
}

#[test]
fn taint_is_a_no_op_when_externally_tainted() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.external_taint.insert(worktree("wt0"));

    let next = taint(&state, &worktree("wt0"));

    assert_eq!(next, state);
}

#[test]
fn taint_is_a_no_op_for_an_unknown_worktree() {
    let state = ProtocolState::default();

    let next = taint(&state, &worktree("unknown"));

    assert_eq!(next, state);
    assert!(!next.worktrees.contains_key(&worktree("unknown")));
}

#[test]
fn database_failure_changes_exactly_external_taint() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 3));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    let next = database_failure(&state, &worktree("wt0"));

    assert_eq!(next.external_taint, BTreeSet::from([worktree("wt0")]));
    assert_eq!(next.worktrees, state.worktrees);
    assert_eq!(next.scopes, state.scopes);
    assert_eq!(next.processed_events, state.processed_events);
    assert_eq!(next.attempts, state.attempts);
    assert_eq!(next.mutation_events, state.mutation_events);
}

#[test]
fn database_failure_is_a_no_op_when_already_externally_tainted() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.external_taint.insert(worktree("wt0"));

    let next = database_failure(&state, &worktree("wt0"));

    assert_eq!(next, state);
}

#[test]
fn database_failure_is_a_no_op_for_an_unknown_worktree() {
    let state = ProtocolState::default();

    let next = database_failure(&state, &worktree("unknown"));

    assert_eq!(next, state);
    assert!(!next.external_taint.contains(&worktree("unknown")));
}

#[test]
fn abandon_transitions_a_live_scope_without_moving_the_cursor_or_changing_identity() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 3));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    let next = abandon(&state, &scope("scope0"));

    let abandoned = next.scopes.get(&scope("scope0")).unwrap();
    assert_eq!(abandoned.status, ScopeStatus::Abandoned);
    assert_eq!(abandoned.actor_kind, ActorKind::Codex);
    assert_eq!(abandoned.worktree_id, worktree("wt0"));

    let owning_worktree = next.worktrees.get(&worktree("wt0")).unwrap();
    assert!(owning_worktree.needs_rebaseline);
    assert_eq!(owning_worktree.revision, 4);
    assert_eq!(owning_worktree.cursor_tree, tree("tree0"));
    assert!(!owning_worktree.tainted);
    assert_eq!(owning_worktree.failure_kind, FailureKind::Healthy);

    assert_eq!(next.external_taint, state.external_taint);
    assert_eq!(next.processed_events, state.processed_events);
    assert_eq!(next.attempts, state.attempts);
    assert_eq!(next.mutation_events, state.mutation_events);
}

#[test]
fn abandon_preserves_other_live_scopes_on_the_same_worktree() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );
    state.scopes.insert(
        scope("scope1"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    let next = abandon(&state, &scope("scope0"));

    assert_eq!(
        next.scopes.get(&scope("scope1")),
        state.scopes.get(&scope("scope1"))
    );
}

#[test]
fn abandon_is_a_no_op_for_a_never_seen_scope() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );

    let next = abandon(&state, &scope("scope0"));

    assert_eq!(next, state);
}

#[test]
fn abandon_is_a_no_op_for_an_already_closed_scope() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Closed, worktree("wt0")),
    );

    let next = abandon(&state, &scope("scope0"));

    assert_eq!(next, state);
}

#[test]
fn abandon_is_a_no_op_for_an_already_abandoned_scope() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Abandoned, worktree("wt0")),
    );

    let next = abandon(&state, &scope("scope0"));

    assert_eq!(next, state);
}

#[test]
fn abandon_is_a_no_op_for_a_live_scope_on_an_externally_tainted_worktree() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );
    state.external_taint.insert(worktree("wt0"));

    let next = abandon(&state, &scope("scope0"));

    assert_eq!(next, state);
}

#[test]
fn abandon_succeeds_for_a_live_scope_on_a_snapshot_tainted_worktree() {
    let mut state = ProtocolState::default();
    state.worktrees.insert(
        worktree("wt0"),
        WorktreeState {
            cursor_tree: tree("tree0"),
            revision: 3,
            tainted: true,
            failure_kind: FailureKind::SnapshotFailure,
            needs_rebaseline: false,
        },
    );
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    let next = abandon(&state, &scope("scope0"));

    let abandoned = next.scopes.get(&scope("scope0")).unwrap();
    assert_eq!(abandoned.status, ScopeStatus::Abandoned);
    assert_eq!(abandoned.actor_kind, ActorKind::Codex);
    assert_eq!(abandoned.worktree_id, worktree("wt0"));

    let owning_worktree = next.worktrees.get(&worktree("wt0")).unwrap();
    assert_eq!(owning_worktree.revision, 4);
    assert_eq!(owning_worktree.cursor_tree, tree("tree0"));
    assert!(owning_worktree.tainted);
    assert_eq!(owning_worktree.failure_kind, FailureKind::SnapshotFailure);
    assert!(owning_worktree.needs_rebaseline);

    assert_eq!(next.external_taint, state.external_taint);
    assert_eq!(next.processed_events, state.processed_events);
    assert_eq!(next.attempts, state.attempts);
    assert_eq!(next.mutation_events, state.mutation_events);
}

#[test]
fn abandon_is_a_no_op_for_an_unknown_scope() {
    let state = ProtocolState::default();

    let next = abandon(&state, &scope("unknown"));

    assert_eq!(next, state);
    assert!(!next.scopes.contains_key(&scope("unknown")));
}

#[test]
fn abandon_is_a_no_op_when_the_scopes_worktree_has_no_durable_state() {
    let mut state = ProtocolState::default();
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    let next = abandon(&state, &scope("scope0"));

    assert_eq!(next, state);
}

#[test]
fn recover_from_snapshot_taint_abandons_live_scopes_and_rebaselines_cursor() {
    let mut state = ProtocolState::default();
    state.worktrees.insert(
        worktree("wt0"),
        WorktreeState {
            cursor_tree: tree("tree0"),
            revision: 3,
            tainted: true,
            failure_kind: FailureKind::SnapshotFailure,
            needs_rebaseline: false,
        },
    );
    state
        .worktrees
        .insert(worktree("wt1"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );
    state.scopes.insert(
        scope("scope1"),
        scope_with_status(ScopeStatus::Closed, worktree("wt0")),
    );
    state.scopes.insert(
        scope("scope2"),
        scope_with_status(ScopeStatus::Active, worktree("wt1")),
    );

    let next = recover(&state, &worktree("wt0"), tree("tree1"));

    let recovered = next.worktrees.get(&worktree("wt0")).unwrap();
    assert_eq!(recovered.cursor_tree, tree("tree1"));
    assert_eq!(recovered.revision, 4);
    assert!(!recovered.tainted);
    assert_eq!(recovered.failure_kind, FailureKind::Healthy);
    assert!(!recovered.needs_rebaseline);
    assert_eq!(
        next.worktrees.get(&worktree("wt1")),
        state.worktrees.get(&worktree("wt1"))
    );

    assert_eq!(
        next.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Abandoned
    );
    assert_eq!(
        next.scopes.get(&scope("scope0")).unwrap().worktree_id,
        worktree("wt0")
    );
    assert_eq!(
        next.scopes.get(&scope("scope1")),
        state.scopes.get(&scope("scope1"))
    );
    assert_eq!(
        next.scopes.get(&scope("scope2")),
        state.scopes.get(&scope("scope2"))
    );

    assert_eq!(next.external_taint, state.external_taint);
    assert_eq!(next.processed_events, state.processed_events);
    assert_eq!(next.attempts, state.attempts);
    assert_eq!(next.mutation_events, state.mutation_events);
}

#[test]
fn recover_from_external_taint_abandons_live_scopes_and_clears_external_taint() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 2));
    state.external_taint.insert(worktree("wt0"));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    let next = recover(&state, &worktree("wt0"), tree("tree1"));

    let recovered = next.worktrees.get(&worktree("wt0")).unwrap();
    assert_eq!(recovered.cursor_tree, tree("tree1"));
    assert_eq!(recovered.revision, 3);
    assert!(!recovered.tainted);
    assert_eq!(recovered.failure_kind, FailureKind::Healthy);
    assert!(!recovered.needs_rebaseline);

    assert!(!next.external_taint.contains(&worktree("wt0")));
    assert_eq!(
        next.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Abandoned
    );
}

#[test]
fn recover_with_only_needs_rebaseline_preserves_live_scopes() {
    let mut state = ProtocolState::default();
    state.worktrees.insert(
        worktree("wt0"),
        WorktreeState {
            cursor_tree: tree("tree0"),
            revision: 1,
            tainted: false,
            failure_kind: FailureKind::Healthy,
            needs_rebaseline: true,
        },
    );
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    let next = recover(&state, &worktree("wt0"), tree("tree1"));

    let recovered = next.worktrees.get(&worktree("wt0")).unwrap();
    assert_eq!(recovered.cursor_tree, tree("tree1"));
    assert_eq!(recovered.revision, 2);
    assert!(!recovered.tainted);
    assert_eq!(recovered.failure_kind, FailureKind::Healthy);
    assert!(!recovered.needs_rebaseline);

    assert_eq!(
        next.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Active
    );
    assert_eq!(next.scopes, state.scopes);
}

#[test]
fn recover_is_a_no_op_on_an_already_healthy_worktree_with_no_rebaseline_need() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));

    let next = recover(&state, &worktree("wt0"), tree("tree1"));

    assert_eq!(next, state);
}

#[test]
fn recover_is_a_no_op_for_an_unknown_worktree() {
    let state = ProtocolState::default();

    let next = recover(&state, &worktree("unknown"), tree("tree1"));

    assert_eq!(next, state);
    assert!(!next.worktrees.contains_key(&worktree("unknown")));
}

// T07: cross-action state-sequence tests. Each of the eight scenarios below
// is required by the plan; none is a single-action test already covered by
// T01-T06.

#[test]
fn attribution_transitions_from_contended_to_exclusive_across_a_close_boundary() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );
    state.scopes.insert(
        scope("scope1"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    // Both scopes are live: an Advance observing a real change is AiContended.
    let step_a = prepare_and_commit(
        &state,
        &attempt_id("attempt_a"),
        Boundary::Advance {
            scope: scope("scope0"),
            event: event("event_a"),
        },
        tree("tree1"),
    );
    assert!(step_a.evaluation.changed);
    let event_a = step_a
        .state
        .mutation_events
        .iter()
        .find(|e| e.revision == 1)
        .expect("advance emitted a mutation event at revision 1");
    assert_eq!(event_a.attribution, Attribution::AiContended);
    assert_eq!(
        event_a.active_scopes,
        BTreeSet::from([scope("scope0"), scope("scope1")])
    );

    // Close also observes a change; because commitAttempt computes live
    // scopes *before* nextScope closes scope0, this still emits AiContended,
    // not AiExclusive.
    let step_b = prepare_and_commit(
        &step_a.state,
        &attempt_id("attempt_b"),
        Boundary::Close {
            scope: scope("scope0"),
            event: event("event_b"),
        },
        tree("tree2"),
    );
    assert!(step_b.evaluation.changed);
    assert_eq!(
        step_b.state.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Closed
    );
    let event_b = step_b
        .state
        .mutation_events
        .iter()
        .find(|e| e.revision == 2)
        .expect("close emitted a mutation event at revision 2");
    assert_eq!(event_b.attribution, Attribution::AiContended);
    assert_eq!(
        event_b.active_scopes,
        BTreeSet::from([scope("scope0"), scope("scope1")])
    );

    // Now only scope1 is live: the next observed change is where AiExclusive
    // first appears.
    let step_c = prepare_and_commit(
        &step_b.state,
        &attempt_id("attempt_c"),
        Boundary::Advance {
            scope: scope("scope1"),
            event: event("event_c"),
        },
        tree("tree3"),
    );
    assert!(step_c.evaluation.changed);
    let event_c = step_c
        .state
        .mutation_events
        .iter()
        .find(|e| e.revision == 3)
        .expect("advance emitted a mutation event at revision 3");
    assert_eq!(
        event_c.attribution,
        Attribution::AiExclusive(scope("scope1"))
    );
    assert_eq!(event_c.active_scopes, BTreeSet::from([scope("scope1")]));
}

#[test]
fn taint_then_recover_abandons_live_scopes_and_rebaselines_cursor() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    let tainted = taint(&state, &worktree("wt0"));
    let tainted_worktree = tainted.worktrees.get(&worktree("wt0")).unwrap();
    assert!(tainted_worktree.tainted);
    assert_eq!(tainted_worktree.failure_kind, FailureKind::SnapshotFailure);
    assert_eq!(tainted_worktree.revision, 1);

    let recovered = recover(&tainted, &worktree("wt0"), tree("tree1"));
    let recovered_worktree = recovered.worktrees.get(&worktree("wt0")).unwrap();
    assert_eq!(recovered_worktree.cursor_tree, tree("tree1"));
    assert_eq!(recovered_worktree.revision, 2);
    assert!(!recovered_worktree.tainted);
    assert_eq!(recovered_worktree.failure_kind, FailureKind::Healthy);
    assert!(!recovered_worktree.needs_rebaseline);
    assert_eq!(
        recovered.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Abandoned
    );
}

#[test]
fn database_failure_then_recover_clears_external_taint_and_rebaselines_cursor() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));

    let failed = database_failure(&state, &worktree("wt0"));
    assert!(failed.external_taint.contains(&worktree("wt0")));
    assert_eq!(
        failed.worktrees.get(&worktree("wt0")),
        state.worktrees.get(&worktree("wt0"))
    );

    let recovered = recover(&failed, &worktree("wt0"), tree("tree1"));
    assert!(!recovered.external_taint.contains(&worktree("wt0")));
    let recovered_worktree = recovered.worktrees.get(&worktree("wt0")).unwrap();
    assert_eq!(recovered_worktree.cursor_tree, tree("tree1"));
    assert_eq!(recovered_worktree.revision, 1);
    assert!(!recovered_worktree.tainted);
    assert_eq!(recovered_worktree.failure_kind, FailureKind::Healthy);
    assert!(!recovered_worktree.needs_rebaseline);
}

#[test]
fn abandon_then_needs_rebaseline_only_recovery_preserves_a_second_live_scope() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );
    state.scopes.insert(
        scope("scope1"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    let abandoned = abandon(&state, &scope("scope0"));
    let abandoned_worktree = abandoned.worktrees.get(&worktree("wt0")).unwrap();
    assert!(abandoned_worktree.needs_rebaseline);
    assert!(!abandoned_worktree.tainted);
    assert_eq!(
        abandoned.scopes.get(&scope("scope1")).unwrap().status,
        ScopeStatus::Active
    );

    let recovered = recover(&abandoned, &worktree("wt0"), tree("tree1"));
    let recovered_worktree = recovered.worktrees.get(&worktree("wt0")).unwrap();
    assert_eq!(recovered_worktree.cursor_tree, tree("tree1"));
    assert!(!recovered_worktree.needs_rebaseline);
    assert_eq!(
        recovered.scopes.get(&scope("scope1")).unwrap().status,
        ScopeStatus::Active,
        "a needsRebaseline-only recovery must preserve a still-live scope"
    );
    assert_eq!(
        recovered.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Abandoned
    );
}

#[test]
fn replay_of_a_committed_event_key_is_rejected_without_mutating_state() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );

    let first = prepare_and_commit(
        &state,
        &attempt_id("attempt_first"),
        start_boundary(),
        tree("tree1"),
    );
    assert!(first.evaluation.accepted);

    // The CAS baseline is fresh (matches the post-commit worktree state);
    // only the replayed EventKey (scope0/event0, same as `start_boundary`)
    // should cause rejection.
    let before = first.state.clone();
    let replay = prepare_and_commit(
        &first.state,
        &attempt_id("attempt_replay"),
        start_boundary(),
        tree("tree2"),
    );

    assert!(!replay.evaluation.accepted);
    assert_eq!(replay.state.worktrees, before.worktrees);
    assert_eq!(replay.state.scopes, before.scopes);
    assert_eq!(replay.state.mutation_events, before.mutation_events);
    assert_eq!(
        replay
            .state
            .attempts
            .get(&attempt_id("attempt_replay"))
            .unwrap()
            .status,
        AttemptStatus::Rejected
    );
}

#[test]
fn stale_attempt_prepared_before_an_intervening_flush_commit_is_rejected_without_advancing_revision_or_moving_cursor(
) {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );

    // Prepare the eventual stale attempt first, baselined at revision 0.
    let prepared_stale = prepare(
        &state,
        attempt_id("attempt_stale"),
        start_boundary(),
        tree("tree_target"),
    );

    // An unrelated Flush commits and advances the worktree before the
    // prepared attempt above is ever committed.
    let intervened = prepare_and_commit(
        &prepared_stale,
        &attempt_id("attempt_flush"),
        flush_boundary(),
        tree("tree_mid"),
    );
    assert!(intervened.evaluation.accepted);
    assert_eq!(
        intervened
            .state
            .worktrees
            .get(&worktree("wt0"))
            .unwrap()
            .revision,
        1
    );

    let before = intervened.state.clone();
    let outcome = commit(&intervened.state, &attempt_id("attempt_stale"));

    assert!(!outcome.evaluation.accepted);
    assert!(!outcome.evaluation.advances_revision);
    assert_eq!(outcome.state.worktrees, before.worktrees);
    assert_eq!(outcome.state.scopes, before.scopes);
    assert_eq!(outcome.state.mutation_events, before.mutation_events);
    assert_eq!(outcome.state.processed_events, before.processed_events);
    assert_eq!(
        outcome
            .state
            .attempts
            .get(&attempt_id("attempt_stale"))
            .unwrap()
            .status,
        AttemptStatus::Rejected
    );
}

#[test]
fn competing_prepared_attempts_the_second_to_commit_is_rejected_by_cas() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));

    // Both A and B are prepared against the same revision-0 baseline.
    let prepared_a = prepare(
        &state,
        attempt_id("attempt_a"),
        flush_boundary(),
        tree("tree_a"),
    );
    let prepared_both = prepare(
        &prepared_a,
        attempt_id("attempt_b"),
        flush_boundary(),
        tree("tree_b"),
    );

    let outcome_a = commit(&prepared_both, &attempt_id("attempt_a"));
    assert!(outcome_a.evaluation.accepted);
    let worktree_after_a = outcome_a.state.worktrees.get(&worktree("wt0")).unwrap();
    assert_eq!(worktree_after_a.revision, 1);
    assert_eq!(worktree_after_a.cursor_tree, tree("tree_a"));

    let before = outcome_a.state.clone();
    let outcome_b = commit(&outcome_a.state, &attempt_id("attempt_b"));

    assert!(!outcome_b.evaluation.accepted);
    assert_eq!(outcome_b.state.worktrees, before.worktrees);
    assert_eq!(outcome_b.state.mutation_events, before.mutation_events);
    assert_eq!(
        outcome_b
            .state
            .attempts
            .get(&attempt_id("attempt_b"))
            .unwrap()
            .status,
        AttemptStatus::Rejected
    );
}

#[test]
fn taint_invalidates_a_prepared_attempt_via_stale_revision() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );

    let prepared = prepare(
        &state,
        attempt_id("attempt0"),
        start_boundary(),
        tree("tree1"),
    );
    let tainted = taint(&prepared, &worktree("wt0"));
    assert_eq!(
        tainted
            .attempts
            .get(&attempt_id("attempt0"))
            .unwrap()
            .status,
        AttemptStatus::Prepared,
        "taint does not touch attempt state directly"
    );

    let before = tainted.clone();
    let outcome = commit(&tainted, &attempt_id("attempt0"));

    assert!(!outcome.evaluation.accepted);
    assert_eq!(outcome.state.worktrees, before.worktrees);
    assert_eq!(outcome.state.scopes, before.scopes);
    assert_eq!(outcome.state.mutation_events, before.mutation_events);
    assert_eq!(
        outcome
            .state
            .attempts
            .get(&attempt_id("attempt0"))
            .unwrap()
            .status,
        AttemptStatus::Rejected
    );
}

// T07: invariant-style tests, named to mirror the Quint invariants this
// module refines. Each reaches its precondition through real transitions
// rather than a manually constructed state, per the task's own requirement.

#[test]
fn scope_started_at_most_once_and_stays_terminal_after_a_real_close() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );

    let started = prepare_and_commit(
        &state,
        &attempt_id("attempt_start"),
        start_boundary(),
        tree("tree1"),
    );
    assert_eq!(
        started.state.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Active
    );

    // ScopeStartedAtMostOnce: a second Start (fresh EventKey) on an
    // already-Active scope is accepted-but-non-observing and does not
    // re-run activation.
    let restarted = prepare_and_commit(
        &started.state,
        &attempt_id("attempt_restart"),
        Boundary::Start {
            scope: scope("scope0"),
            event: event("event_restart"),
        },
        tree("tree2"),
    );
    assert!(restarted.evaluation.accepted);
    assert!(!restarted.evaluation.observes);
    assert_eq!(
        restarted.state.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Active
    );
    assert_eq!(
        restarted
            .state
            .worktrees
            .get(&worktree("wt0"))
            .unwrap()
            .cursor_tree,
        tree("tree1"),
        "a non-observing restart must not move the cursor"
    );

    let closed = prepare_and_commit(
        &restarted.state,
        &attempt_id("attempt_close"),
        Boundary::Close {
            scope: scope("scope0"),
            event: event("event_close"),
        },
        tree("tree3"),
    );
    assert_eq!(
        closed.state.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Closed
    );

    // TerminalScopesStayTerminal: a Start against the now-Closed scope
    // cannot reactivate it.
    let reopen_attempt = prepare_and_commit(
        &closed.state,
        &attempt_id("attempt_reopen"),
        Boundary::Start {
            scope: scope("scope0"),
            event: event("event_reopen"),
        },
        tree("tree4"),
    );
    assert!(!reopen_attempt.evaluation.observes);
    assert_eq!(
        reopen_attempt
            .state
            .scopes
            .get(&scope("scope0"))
            .unwrap()
            .status,
        ScopeStatus::Closed
    );
}

#[test]
fn start_on_a_scope_abandoned_via_a_real_transition_never_reactivates_it() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );

    let started = prepare_and_commit(
        &state,
        &attempt_id("attempt_start"),
        start_boundary(),
        tree("tree1"),
    );
    let abandoned = abandon(&started.state, &scope("scope0"));
    assert_eq!(
        abandoned.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Abandoned
    );

    let reopen_attempt = prepare_and_commit(
        &abandoned,
        &attempt_id("attempt_reopen"),
        Boundary::Start {
            scope: scope("scope0"),
            event: event("event_reopen"),
        },
        tree("tree2"),
    );
    assert!(!reopen_attempt.evaluation.observes);
    assert_eq!(
        reopen_attempt
            .state
            .scopes
            .get(&scope("scope0"))
            .unwrap()
            .status,
        ScopeStatus::Abandoned
    );
}

#[test]
fn start_does_not_abandon_existing_scopes_multi_scope_sequence() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );
    state.scopes.insert(
        scope("scope1"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );
    let scope0_before = state.scopes.get(&scope("scope0")).unwrap().clone();

    let outcome = prepare_and_commit(
        &state,
        &attempt_id("attempt_start"),
        Boundary::Start {
            scope: scope("scope1"),
            event: event("event0"),
        },
        tree("tree1"),
    );

    assert_eq!(
        outcome.state.scopes.get(&scope("scope1")).unwrap().status,
        ScopeStatus::Active
    );
    assert_eq!(
        outcome.state.scopes.get(&scope("scope0")).unwrap(),
        &scope0_before,
        "starting scope1 must not alter the already-active scope0"
    );
}

#[test]
fn rejected_attempts_do_not_commit_evidence_across_a_mixed_accept_reject_sequence() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), 0));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );

    let prepared_start = prepare(
        &state,
        attempt_id("attempt_start"),
        start_boundary(),
        tree("tree1"),
    );
    let prepared_both = prepare(
        &prepared_start,
        attempt_id("attempt_advance"),
        Boundary::Advance {
            scope: scope("scope0"),
            event: event("event_advance"),
        },
        tree("tree2"),
    );

    let accepted = commit(&prepared_both, &attempt_id("attempt_start"));
    assert!(accepted.evaluation.accepted);
    assert!(accepted.evaluation.changed);

    let rejected = commit(&accepted.state, &attempt_id("attempt_advance"));
    assert!(!rejected.evaluation.accepted);
    assert_eq!(
        rejected
            .state
            .attempts
            .get(&attempt_id("attempt_advance"))
            .unwrap()
            .status,
        AttemptStatus::Rejected
    );

    // RejectedAttemptsDoNotCommitEvidence: the rejected Advance must not have
    // added any mutation evidence beyond what the accepted Start produced.
    assert_eq!(
        rejected.state.mutation_events,
        accepted.state.mutation_events
    );
    assert_eq!(rejected.state.mutation_events.len(), 1);
}

// T07 post-review correction: `needsRebaseline` must suppress mutation
// evidence even when `commit` genuinely observes a real tree change
// (`MutationEventsCrossOnlyTrustworthyProtocolStates`).

#[test]
fn needs_rebaseline_suppresses_mutation_event_even_when_commit_observes_a_real_tree_change() {
    let mut state = ProtocolState::default();
    state.worktrees.insert(
        worktree("wt0"),
        WorktreeState {
            cursor_tree: tree("tree0"),
            revision: 0,
            tainted: false,
            failure_kind: FailureKind::Healthy,
            needs_rebaseline: true,
        },
    );
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    let outcome = prepare_and_commit(
        &state,
        &attempt_id("attempt0"),
        Boundary::Advance {
            scope: scope("scope0"),
            event: event("event0"),
        },
        tree("tree1"),
    );

    assert!(outcome.evaluation.accepted);
    assert!(outcome.evaluation.observes);
    assert!(outcome.evaluation.observed_change);
    assert!(
        !outcome.evaluation.changed,
        "needs_rebaseline must suppress mutation evidence even though a real change was observed"
    );

    let committed_worktree = outcome.state.worktrees.get(&worktree("wt0")).unwrap();
    assert_eq!(
        committed_worktree.cursor_tree,
        tree("tree0"),
        "cursor must not move while needs_rebaseline is set"
    );
    assert!(committed_worktree.needs_rebaseline);
    assert!(
        outcome.state.mutation_events.is_empty(),
        "no MutationEvent may be emitted while needs_rebaseline is set"
    );
}

// T07 post-review correction: Quint's `revision: int` is unbounded; this
// refinement's `revision: u64` is not, so every revision-advancing action
// must refuse to wrap past `u64::MAX` rather than commit partial or wrapped
// state.

// A commit that would advance revision is rejected at `u64::MAX`: headroom
// is required for advancement, not for acceptance in general. Paired with
// `no_change_flush_commits_at_u64_max_without_advancing_revision` below,
// which proves the other half — a commit that would NOT advance revision
// (a no-change `Flush`) is accepted at `u64::MAX`.
#[test]
fn commit_that_would_advance_is_rejected_at_u64_max() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), u64::MAX));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::NeverSeen, worktree("wt0")),
    );
    let before = state.clone();

    let outcome = prepare_and_commit(
        &state,
        &attempt_id("attempt0"),
        start_boundary(),
        tree("tree1"),
    );

    assert!(
        !outcome.evaluation.accepted,
        "a commit that would advance revision past u64::MAX must be rejected, not wrapped"
    );
    assert_eq!(outcome.state.worktrees, before.worktrees);
    assert_eq!(outcome.state.scopes, before.scopes);
    assert_eq!(outcome.state.processed_events, before.processed_events);
    assert_eq!(outcome.state.mutation_events, before.mutation_events);
    assert_eq!(
        outcome
            .state
            .worktrees
            .get(&worktree("wt0"))
            .unwrap()
            .revision,
        u64::MAX,
        "revision must never wrap to 0"
    );
    assert_eq!(
        outcome
            .state
            .attempts
            .get(&attempt_id("attempt0"))
            .unwrap()
            .status,
        AttemptStatus::Rejected
    );
}

// T07 post-review correction: the first checked-u64 guard required revision
// headroom for every accepted commit, even a no-change `Flush` that would
// not advance revision. Quint's `commitAttempt` accepts and commits that
// case without advancing revision, so a fresh no-change `Flush` at
// `revision: u64::MAX` must commit successfully rather than being rejected
// for headroom it does not need.
#[test]
fn no_change_flush_commits_at_u64_max_without_advancing_revision() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), u64::MAX));
    let before = state.clone();

    let outcome = prepare_and_commit(
        &state,
        &attempt_id("attempt0"),
        flush_boundary(),
        tree("tree0"),
    );

    assert!(outcome.evaluation.accepted);
    assert!(outcome.evaluation.observes);
    assert!(!outcome.evaluation.observed_change);
    assert!(!outcome.evaluation.changed);
    assert!(
        !outcome.evaluation.advances_revision,
        "a no-change Flush must not advance revision even when accepted"
    );

    assert_eq!(
        outcome
            .state
            .attempts
            .get(&attempt_id("attempt0"))
            .unwrap()
            .status,
        AttemptStatus::Committed
    );

    let committed_worktree = outcome.state.worktrees.get(&worktree("wt0")).unwrap();
    assert_eq!(
        committed_worktree.revision,
        u64::MAX,
        "revision must stay at u64::MAX; a no-change Flush requires no headroom"
    );
    assert_eq!(committed_worktree.cursor_tree, tree("tree0"));
    assert_eq!(
        outcome.state.worktrees.get(&worktree("wt0")).unwrap(),
        before.worktrees.get(&worktree("wt0")).unwrap(),
        "the worktree must be otherwise unchanged"
    );

    assert!(
        outcome.state.mutation_events.is_empty(),
        "no MutationEvent may be emitted for a no-change Flush"
    );
    assert_eq!(outcome.state.processed_events, before.processed_events);
    assert_eq!(outcome.state.scopes, before.scopes);
    assert_eq!(outcome.state.external_taint, before.external_taint);
}

#[test]
fn taint_does_not_wrap_revision_at_u64_max() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), u64::MAX));

    let next = taint(&state, &worktree("wt0"));

    assert_eq!(
        next, state,
        "taint must be a no-op rather than wrap revision"
    );
    assert_eq!(
        next.worktrees.get(&worktree("wt0")).unwrap().revision,
        u64::MAX
    );
    assert!(!next.worktrees.get(&worktree("wt0")).unwrap().tainted);
}

#[test]
fn abandon_does_not_wrap_revision_at_u64_max() {
    let mut state = ProtocolState::default();
    state
        .worktrees
        .insert(worktree("wt0"), healthy_worktree(tree("tree0"), u64::MAX));
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    let next = abandon(&state, &scope("scope0"));

    assert_eq!(
        next, state,
        "abandon must be a no-op rather than wrap revision"
    );
    assert_eq!(
        next.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Active,
        "the scope must not be abandoned if doing so cannot be recorded safely"
    );
    assert_eq!(
        next.worktrees.get(&worktree("wt0")).unwrap().revision,
        u64::MAX
    );
}

#[test]
fn recover_does_not_wrap_revision_at_u64_max() {
    let mut state = ProtocolState::default();
    state.worktrees.insert(
        worktree("wt0"),
        WorktreeState {
            cursor_tree: tree("tree0"),
            revision: u64::MAX,
            tainted: false,
            failure_kind: FailureKind::Healthy,
            needs_rebaseline: true,
        },
    );
    state.scopes.insert(
        scope("scope0"),
        scope_with_status(ScopeStatus::Active, worktree("wt0")),
    );

    let next = recover(&state, &worktree("wt0"), tree("tree1"));

    assert_eq!(
        next, state,
        "recover must be a no-op rather than wrap revision, even though needs_rebaseline would otherwise trigger it"
    );
    assert_eq!(
        next.worktrees.get(&worktree("wt0")).unwrap().revision,
        u64::MAX
    );
    assert!(
        next.worktrees
            .get(&worktree("wt0"))
            .unwrap()
            .needs_rebaseline
    );
    assert_eq!(
        next.scopes.get(&scope("scope0")).unwrap().status,
        ScopeStatus::Active
    );
}
