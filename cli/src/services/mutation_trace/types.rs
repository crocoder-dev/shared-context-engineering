#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorktreeId(pub String);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ScopeId(pub String);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TreeId(pub String);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EventId(pub String);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AttemptId(pub String);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EventKey {
    pub scope_id: ScopeId,
    pub event_id: EventId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActorKind {
    ClaudeCode,
    Codex,
    OpenCode,
    Pi,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FailureKind {
    Healthy,
    SnapshotFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeStatus {
    NeverSeen,
    Active,
    Closed,
    Abandoned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptStatus {
    Available,
    Prepared,
    Committed,
    Rejected,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Attribution {
    IneligibleUnscoped,
    AiExclusive(ScopeId),
    AiContended,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Boundary {
    Start { scope: ScopeId, event: EventId },
    Advance { scope: ScopeId, event: EventId },
    Close { scope: ScopeId, event: EventId },
    Flush { worktree: WorktreeId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeState {
    pub cursor_tree: TreeId,
    pub revision: u64,
    pub tainted: bool,
    pub failure_kind: FailureKind,
    pub needs_rebaseline: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeState {
    pub status: ScopeStatus,
    pub actor_kind: ActorKind,
    pub worktree_id: WorktreeId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptState {
    pub status: AttemptStatus,
    pub boundary: Boundary,
    pub expected_revision: u64,
    pub before_tree: TreeId,
    pub after_tree: TreeId,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct MutationEvent {
    pub worktree_id: WorktreeId,
    pub revision: u64,
    pub before_tree: TreeId,
    pub after_tree: TreeId,
    pub active_scopes: std::collections::BTreeSet<ScopeId>,
    pub tainted: bool,
    pub failure_kind: FailureKind,
    pub attribution: Attribution,
    pub boundary: Boundary,
}

pub fn is_live(status: ScopeStatus) -> bool {
    status == ScopeStatus::Active
}

impl ScopeState {
    pub fn scope_worktree(&self) -> WorktreeId {
        self.worktree_id.clone()
    }

    pub fn is_live(&self) -> bool {
        is_live(self.status)
    }
}

pub fn boundary_worktree(
    boundary: &Boundary,
    scopes: &std::collections::BTreeMap<ScopeId, ScopeState>,
) -> Option<WorktreeId> {
    match boundary {
        Boundary::Flush { worktree } => Some(worktree.clone()),
        Boundary::Start { scope, .. }
        | Boundary::Advance { scope, .. }
        | Boundary::Close { scope, .. } => scopes.get(scope).map(ScopeState::scope_worktree),
    }
}

pub fn boundary_scope(boundary: &Boundary) -> Option<ScopeId> {
    match boundary {
        Boundary::Start { scope, .. }
        | Boundary::Advance { scope, .. }
        | Boundary::Close { scope, .. } => Some(scope.clone()),
        Boundary::Flush { .. } => None,
    }
}

pub fn boundary_event(boundary: &Boundary) -> Option<EventId> {
    match boundary {
        Boundary::Start { event, .. }
        | Boundary::Advance { event, .. }
        | Boundary::Close { event, .. } => Some(event.clone()),
        Boundary::Flush { .. } => None,
    }
}

pub fn boundary_event_key(boundary: &Boundary) -> Option<EventKey> {
    match (boundary_scope(boundary), boundary_event(boundary)) {
        (Some(scope_id), Some(event_id)) => Some(EventKey { scope_id, event_id }),
        _ => None,
    }
}

pub fn is_hook(boundary: &Boundary) -> bool {
    !matches!(boundary, Boundary::Flush { .. })
}

pub fn is_start(boundary: &Boundary) -> bool {
    matches!(boundary, Boundary::Start { .. })
}

pub fn is_advance(boundary: &Boundary) -> bool {
    matches!(boundary, Boundary::Advance { .. })
}

pub fn is_close(boundary: &Boundary) -> bool {
    matches!(boundary, Boundary::Close { .. })
}

pub fn is_flush(boundary: &Boundary) -> bool {
    matches!(boundary, Boundary::Flush { .. })
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProtocolState {
    pub worktrees: std::collections::BTreeMap<WorktreeId, WorktreeState>,
    pub scopes: std::collections::BTreeMap<ScopeId, ScopeState>,
    pub external_taint: std::collections::BTreeSet<WorktreeId>,
    pub processed_events: std::collections::BTreeSet<EventKey>,
    pub attempts: std::collections::BTreeMap<AttemptId, AttemptState>,
    pub mutation_events: std::collections::BTreeSet<MutationEvent>,
}
