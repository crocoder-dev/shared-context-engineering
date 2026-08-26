use std::collections::{BTreeMap, BTreeSet};

use super::protocol::{
    abandon, attribution_for, commit, database_failure, live_scopes_on, prepare, recover, taint,
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
