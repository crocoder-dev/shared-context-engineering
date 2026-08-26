//! Pure domain types for the mutation-cursor protocol.
//!
//! Refines `spec/mutation_cursor.qnt:2-117` (state types) and
//! `spec/mutation_cursor.qnt:151-245` (pure accessors). The Quint model uses
//! finite enumerated identities (`WT0`/`Scope0`/...) as bounded verification
//! domains only; `spec/mutation_cursor.md` states production code "must
//! support larger and unbounded identifier spaces". This module therefore
//! refines every identity type as an opaque wrapper over an owned string
//! rather than a fixed enum, and carries no Git/DB/filesystem/environment/
//! network/lock/async dependency.

/// Durable identity of a worktree. Refines `WorktreeId` (`spec/mutation_cursor.qnt:2`).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorktreeId(pub String);

/// Durable identity of an AI scope/session. Refines `ScopeId` (`spec/mutation_cursor.qnt:4`).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ScopeId(pub String);

/// Identity of a captured worktree tree snapshot. Refines `TreeId` (`spec/mutation_cursor.qnt:5`).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TreeId(pub String);

/// Identity of a hook delivery event. Refines `EventId` (`spec/mutation_cursor.qnt:6-17`).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EventId(pub String);

/// Identity of a speculative attempt. Refines `AttemptId` (`spec/mutation_cursor.qnt:17`).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AttemptId(pub String);

/// Replay/idempotency identity for a hook delivery, scoped by `ScopeId` and
/// `EventId`. Refines `EventKey` (`spec/mutation_cursor.qnt:18-21`).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EventKey {
    pub scope_id: ScopeId,
    pub event_id: EventId,
}

/// The harness that owns a scope. Unlike the identity types above, this is a
/// real closed set (every supported harness), not a bounded verification
/// domain, so it stays a fixed enum. Refines `ActorKind` (`spec/mutation_cursor.qnt:3`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActorKind {
    ClaudeCode,
    Codex,
    OpenCode,
    Pi,
}

/// Snapshot-failure state of a worktree. Refines `FailureKind` (`spec/mutation_cursor.qnt:22`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureKind {
    Healthy,
    SnapshotFailure,
}

/// Lifecycle status of a scope. Refines `ScopeStatus` (`spec/mutation_cursor.qnt:24`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeStatus {
    NeverSeen,
    Active,
    Closed,
    Abandoned,
}

/// Lifecycle status of a speculative attempt. Refines `AttemptStatus` (`spec/mutation_cursor.qnt:25`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptStatus {
    Available,
    Prepared,
    Committed,
    Rejected,
}

/// Mutation-evidence attribution for a worktree. Refines `Attribution`
/// (`spec/mutation_cursor.qnt:26-29`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Attribution {
    IneligibleUnscoped,
    AiExclusive(ScopeId),
    AiContended,
}

/// A hook or flush boundary at which the protocol may transition state.
/// Refines `Boundary` (`spec/mutation_cursor.qnt:31-35`), field-for-field:
/// `Start`/`Advance`/`Close` carry only `scope`/`event`, exactly like the
/// Quint constructors, and `Flush` carries only `worktree`.
///
/// A hook variant deliberately carries no independent `worktree` field. The
/// Quint model's `boundaryWorktree` never stores a worktree either — it
/// derives one from `scopeWorktree(data.scope)`. Storing a worktree directly
/// on the boundary would let a value claim a worktree inconsistent with its
/// own scope's true (durable, assigned-for-life) worktree, a state the
/// Quint type cannot represent. See [`boundary_worktree`] for how this
/// refinement resolves a hook boundary's worktree without that field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Boundary {
    Start { scope: ScopeId, event: EventId },
    Advance { scope: ScopeId, event: EventId },
    Close { scope: ScopeId, event: EventId },
    Flush { worktree: WorktreeId },
}

/// Durable per-worktree cursor and failure state. Refines `WorktreeState`
/// (`spec/mutation_cursor.qnt:37-43`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeState {
    pub cursor_tree: TreeId,
    pub revision: u64,
    pub tainted: bool,
    pub failure_kind: FailureKind,
    pub needs_rebaseline: bool,
}

/// Durable per-scope lifecycle state. Refines `ScopeState` (`spec/mutation_cursor.qnt:45-49`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeState {
    pub status: ScopeStatus,
    pub actor_kind: ActorKind,
    pub worktree_id: WorktreeId,
}

/// Transient speculative-attempt state. Refines `AttemptState` (`spec/mutation_cursor.qnt:51-57`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptState {
    pub status: AttemptStatus,
    pub boundary: Boundary,
    pub expected_revision: u64,
    pub before_tree: TreeId,
    pub after_tree: TreeId,
}

/// Durable mutation evidence emitted by a committed attempt. Refines
/// `MutationEvent` (`spec/mutation_cursor.qnt:99-109`).
#[derive(Clone, Debug, Eq, PartialEq)]
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

/// A scope is eligible to contribute to attribution. Refines `isLive`
/// (`spec/mutation_cursor.qnt:167`).
pub fn is_live(status: ScopeStatus) -> bool {
    status == ScopeStatus::Active
}

/// A scope has ended and can never be reactivated. Refines `isTerminal`
/// (`spec/mutation_cursor.qnt:169-170`).
pub fn is_terminal(status: ScopeStatus) -> bool {
    matches!(status, ScopeStatus::Closed | ScopeStatus::Abandoned)
}

impl ScopeState {
    /// The worktree this scope belongs to. Refines `scopeWorktree`
    /// (`spec/mutation_cursor.qnt:151-157`).
    ///
    /// The Quint function is a pure `match` over `ScopeId` only because the
    /// model's identity enum has a fixed, static worktree partition
    /// (`Scope0`/`Scope1`/`Scope2` always belong to `WT0`, `Scope3` always to
    /// `WT1`). This refinement's `ScopeId` is an opaque, dynamically created
    /// identifier with no such static partition (see the module doc
    /// comment), so the only faithful source for a scope's worktree is its
    /// own durable state — the same fact the Quint model also carries on
    /// `ScopeState.worktreeId`, kept consistent with the static function by
    /// construction at scope-creation time.
    pub fn scope_worktree(&self) -> WorktreeId {
        self.worktree_id.clone()
    }

    /// The harness that owns this scope. Refines `scopeActor`
    /// (`spec/mutation_cursor.qnt:159-165`).
    pub fn scope_actor(&self) -> ActorKind {
        self.actor_kind
    }

    /// Refines `isLive` applied to this scope's status.
    pub fn is_live(&self) -> bool {
        is_live(self.status)
    }

    /// Refines `isTerminal` applied to this scope's status.
    pub fn is_terminal(&self) -> bool {
        is_terminal(self.status)
    }
}

/// The worktree a boundary applies to. Refines `boundaryWorktree`
/// (`spec/mutation_cursor.qnt:172-178`).
///
/// A `Flush` boundary carries its worktree directly. A hook boundary
/// (`Start`/`Advance`/`Close`) does not (see the [`Boundary`] doc comment),
/// so resolving its worktree requires the associated scope's own durable
/// state — the caller looks it up by [`boundary_scope`] exactly as
/// `commitAttempt`-equivalent logic already must, to evaluate the boundary's
/// `observes` rule, and passes it here. `scope` is ignored for `Flush` and
/// the result is `None` for a hook boundary whose scope was not supplied
/// (for example, an unknown scope).
pub fn boundary_worktree(boundary: &Boundary, scope: Option<&ScopeState>) -> Option<WorktreeId> {
    match boundary {
        Boundary::Flush { worktree } => Some(worktree.clone()),
        Boundary::Start { .. } | Boundary::Advance { .. } | Boundary::Close { .. } => {
            scope.map(ScopeState::scope_worktree)
        }
    }
}

/// The scope a boundary applies to, or `None` for a `Flush` boundary, which
/// carries no scope. Refines `boundaryScope` (`spec/mutation_cursor.qnt:180-186`);
/// `None` replaces the Quint model's arbitrary `Scope0` placeholder default.
pub fn boundary_scope(boundary: &Boundary) -> Option<ScopeId> {
    match boundary {
        Boundary::Start { scope, .. }
        | Boundary::Advance { scope, .. }
        | Boundary::Close { scope, .. } => Some(scope.clone()),
        Boundary::Flush { .. } => None,
    }
}

/// The event a boundary applies to, or `None` for a `Flush` boundary, which
/// carries no event. Refines `boundaryEvent` (`spec/mutation_cursor.qnt:188-194`);
/// `None` replaces the Quint model's arbitrary `Event0` placeholder default.
pub fn boundary_event(boundary: &Boundary) -> Option<EventId> {
    match boundary {
        Boundary::Start { event, .. }
        | Boundary::Advance { event, .. }
        | Boundary::Close { event, .. } => Some(event.clone()),
        Boundary::Flush { .. } => None,
    }
}

/// The replay identity of a boundary, or `None` for a `Flush` boundary.
/// Refines `boundaryEventKey` (`spec/mutation_cursor.qnt:196-202`).
pub fn boundary_event_key(boundary: &Boundary) -> Option<EventKey> {
    match (boundary_scope(boundary), boundary_event(boundary)) {
        (Some(scope_id), Some(event_id)) => Some(EventKey { scope_id, event_id }),
        _ => None,
    }
}

/// A boundary is a hook delivery (`Start`/`Advance`/`Close`), not a `Flush`.
/// Refines `isHook` (`spec/mutation_cursor.qnt:204-210`).
pub fn is_hook(boundary: &Boundary) -> bool {
    !matches!(boundary, Boundary::Flush { .. })
}

/// Refines `isStart` (`spec/mutation_cursor.qnt:212-216`).
pub fn is_start(boundary: &Boundary) -> bool {
    matches!(boundary, Boundary::Start { .. })
}

/// Refines `isAdvance` (`spec/mutation_cursor.qnt:218-222`).
pub fn is_advance(boundary: &Boundary) -> bool {
    matches!(boundary, Boundary::Advance { .. })
}

/// Refines `isClose` (`spec/mutation_cursor.qnt:224-228`).
pub fn is_close(boundary: &Boundary) -> bool {
    matches!(boundary, Boundary::Close { .. })
}

/// Refines `isFlush` (`spec/mutation_cursor.qnt:230-234`).
pub fn is_flush(boundary: &Boundary) -> bool {
    matches!(boundary, Boundary::Flush { .. })
}
