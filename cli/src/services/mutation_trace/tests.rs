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

fn scope_state_in(worktree_id: WorktreeId) -> ScopeState {
    ScopeState {
        status: ScopeStatus::Active,
        actor_kind: ActorKind::Codex,
        worktree_id,
    }
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
fn boundary_worktree_resolves_via_scope_for_hook_boundaries() {
    let scope_state = scope_state_in(worktree("wt0"));
    assert_eq!(
        boundary_worktree(&start_boundary(), Some(&scope_state)),
        Some(worktree("wt0"))
    );
    assert_eq!(
        boundary_worktree(&advance_boundary(), Some(&scope_state)),
        Some(worktree("wt0"))
    );
    assert_eq!(
        boundary_worktree(&close_boundary(), Some(&scope_state)),
        Some(worktree("wt0"))
    );
}

#[test]
fn boundary_worktree_resolves_directly_for_flush() {
    // Flush needs no scope context: it carries its own worktree.
    assert_eq!(
        boundary_worktree(&flush_boundary(), None),
        Some(worktree("wt0"))
    );
}

#[test]
fn boundary_worktree_reflects_the_scopes_own_worktree_not_a_guess() {
    // A hook boundary's worktree always comes from its scope's true, durable
    // assignment; it is never independently stored on the boundary itself,
    // so an unrelated worktree's scope state cannot be mistaken for it.
    let scope_state = scope_state_in(worktree("wt1"));
    assert_eq!(
        boundary_worktree(&start_boundary(), Some(&scope_state)),
        Some(worktree("wt1"))
    );
}

#[test]
fn boundary_worktree_is_none_for_hook_boundary_without_scope_context() {
    assert_eq!(boundary_worktree(&start_boundary(), None), None);
    assert_eq!(boundary_worktree(&advance_boundary(), None), None);
    assert_eq!(boundary_worktree(&close_boundary(), None), None);
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
