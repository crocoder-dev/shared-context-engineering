//! Comparable model state and Quint ITF wire types for the MBT harness.
//!
//! Quint Connect deserializes trace state via the [`itf`] wire format, where
//! sum types serialize as `{ tag, value }` records (see the `quint-connect`
//! crate README's "Enums" section). The `Wire*` types here mirror that exact
//! shape for every Quint type reachable from `spec/mutation_cursor.qnt`'s
//! comparable state, then convert into this crate's own domain types
//! (`super::super::types`) via `From` impls, so [`ModelState`] and the values
//! [`super::driver::MutationCursorDriver`] extracts stay expressed in the
//! same production types the rest of `mutation_trace` uses. `spec/
//! mutation_cursor.qnt`'s verification-only `mbtAction` variable is never
//! given a field here, so it is silently ignored by `serde`'s default
//! unknown-field handling when the whole top-level state record is
//! deserialized — that omission is what keeps `mbtAction` out of the
//! compared state.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use super::super::types::{
    ActorKind, AttemptId, AttemptState, AttemptStatus, Attribution, Boundary, EventId, EventKey,
    FailureKind, MutationEvent, ScopeId, ScopeState, ScopeStatus, TreeId, WorktreeId,
    WorktreeState,
};

// ---------------------------------------------------------------------
// Finite identity wire types (`spec/mutation_cursor.qnt`'s unit sum types)
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize)]
#[serde(tag = "tag")]
pub(super) enum WireWorktreeId {
    WT0,
    WT1,
}

impl From<WireWorktreeId> for WorktreeId {
    fn from(value: WireWorktreeId) -> Self {
        WorktreeId(
            match value {
                WireWorktreeId::WT0 => "wt0",
                WireWorktreeId::WT1 => "wt1",
            }
            .to_string(),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize)]
#[serde(tag = "tag")]
pub(super) enum WireScopeId {
    Scope0,
    Scope1,
    Scope2,
    Scope3,
}

impl From<WireScopeId> for ScopeId {
    fn from(value: WireScopeId) -> Self {
        ScopeId(
            match value {
                WireScopeId::Scope0 => "scope0",
                WireScopeId::Scope1 => "scope1",
                WireScopeId::Scope2 => "scope2",
                WireScopeId::Scope3 => "scope3",
            }
            .to_string(),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize)]
#[serde(tag = "tag")]
pub(super) enum WireTreeId {
    Tree0,
    Tree1,
    Tree2,
    Tree3,
}

impl From<WireTreeId> for TreeId {
    fn from(value: WireTreeId) -> Self {
        TreeId(
            match value {
                WireTreeId::Tree0 => "tree0",
                WireTreeId::Tree1 => "tree1",
                WireTreeId::Tree2 => "tree2",
                WireTreeId::Tree3 => "tree3",
            }
            .to_string(),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize)]
#[serde(tag = "tag")]
pub(super) enum WireEventId {
    Event0,
    Event1,
    Event2,
    Event3,
    Event4,
    Event5,
    Event6,
    Event7,
    Event8,
    Event9,
}

impl From<WireEventId> for EventId {
    fn from(value: WireEventId) -> Self {
        EventId(
            match value {
                WireEventId::Event0 => "event0",
                WireEventId::Event1 => "event1",
                WireEventId::Event2 => "event2",
                WireEventId::Event3 => "event3",
                WireEventId::Event4 => "event4",
                WireEventId::Event5 => "event5",
                WireEventId::Event6 => "event6",
                WireEventId::Event7 => "event7",
                WireEventId::Event8 => "event8",
                WireEventId::Event9 => "event9",
            }
            .to_string(),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize)]
#[serde(tag = "tag")]
pub(super) enum WireAttemptId {
    Attempt0,
    Attempt1,
    Attempt2,
    Attempt3,
    Attempt4,
    Attempt5,
}

impl From<WireAttemptId> for AttemptId {
    fn from(value: WireAttemptId) -> Self {
        AttemptId(
            match value {
                WireAttemptId::Attempt0 => "attempt0",
                WireAttemptId::Attempt1 => "attempt1",
                WireAttemptId::Attempt2 => "attempt2",
                WireAttemptId::Attempt3 => "attempt3",
                WireAttemptId::Attempt4 => "attempt4",
                WireAttemptId::Attempt5 => "attempt5",
            }
            .to_string(),
        )
    }
}

// ---------------------------------------------------------------------
// Enum wire types
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize)]
#[serde(tag = "tag")]
pub(super) enum WireActorKind {
    ClaudeCode,
    Codex,
    OpenCode,
    Pi,
}

impl From<WireActorKind> for ActorKind {
    fn from(value: WireActorKind) -> Self {
        match value {
            WireActorKind::ClaudeCode => ActorKind::ClaudeCode,
            WireActorKind::Codex => ActorKind::Codex,
            WireActorKind::OpenCode => ActorKind::OpenCode,
            WireActorKind::Pi => ActorKind::Pi,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize)]
#[serde(tag = "tag")]
pub(super) enum WireFailureKind {
    Healthy,
    SnapshotFailure,
}

impl From<WireFailureKind> for FailureKind {
    fn from(value: WireFailureKind) -> Self {
        match value {
            WireFailureKind::Healthy => FailureKind::Healthy,
            WireFailureKind::SnapshotFailure => FailureKind::SnapshotFailure,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize)]
#[serde(tag = "tag")]
pub(super) enum WireScopeStatus {
    NeverSeen,
    Active,
    Closed,
    Abandoned,
}

impl From<WireScopeStatus> for ScopeStatus {
    fn from(value: WireScopeStatus) -> Self {
        match value {
            WireScopeStatus::NeverSeen => ScopeStatus::NeverSeen,
            WireScopeStatus::Active => ScopeStatus::Active,
            WireScopeStatus::Closed => ScopeStatus::Closed,
            WireScopeStatus::Abandoned => ScopeStatus::Abandoned,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize)]
#[serde(tag = "tag")]
pub(super) enum WireAttemptStatus {
    Available,
    Prepared,
    Committed,
    Rejected,
}

impl From<WireAttemptStatus> for AttemptStatus {
    fn from(value: WireAttemptStatus) -> Self {
        match value {
            WireAttemptStatus::Available => AttemptStatus::Available,
            WireAttemptStatus::Prepared => AttemptStatus::Prepared,
            WireAttemptStatus::Committed => AttemptStatus::Committed,
            WireAttemptStatus::Rejected => AttemptStatus::Rejected,
        }
    }
}

// ---------------------------------------------------------------------
// Structured wire types
// ---------------------------------------------------------------------

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize)]
pub(super) struct WireScopeEvent {
    pub scope: WireScopeId,
    pub event: WireEventId,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize)]
#[serde(tag = "tag", content = "value")]
pub(super) enum WireBoundary {
    Start(WireScopeEvent),
    Advance(WireScopeEvent),
    Close(WireScopeEvent),
    Flush(WireWorktreeId),
}

impl From<WireBoundary> for Boundary {
    fn from(value: WireBoundary) -> Self {
        match value {
            WireBoundary::Start(data) => Boundary::Start {
                scope: data.scope.into(),
                event: data.event.into(),
            },
            WireBoundary::Advance(data) => Boundary::Advance {
                scope: data.scope.into(),
                event: data.event.into(),
            },
            WireBoundary::Close(data) => Boundary::Close {
                scope: data.scope.into(),
                event: data.event.into(),
            },
            WireBoundary::Flush(worktree) => Boundary::Flush {
                worktree: worktree.into(),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize)]
#[serde(tag = "tag", content = "value")]
pub(super) enum WireAttribution {
    IneligibleUnscoped,
    AiExclusive(WireScopeId),
    AiContended,
}

impl From<WireAttribution> for Attribution {
    fn from(value: WireAttribution) -> Self {
        match value {
            WireAttribution::IneligibleUnscoped => Attribution::IneligibleUnscoped,
            WireAttribution::AiExclusive(scope) => Attribution::AiExclusive(scope.into()),
            WireAttribution::AiContended => Attribution::AiContended,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WireWorktreeState {
    pub cursor_tree: WireTreeId,
    pub revision: u64,
    pub tainted: bool,
    pub failure_kind: WireFailureKind,
    pub needs_rebaseline: bool,
}

impl From<WireWorktreeState> for WorktreeState {
    fn from(value: WireWorktreeState) -> Self {
        WorktreeState {
            cursor_tree: value.cursor_tree.into(),
            revision: value.revision,
            tainted: value.tainted,
            failure_kind: value.failure_kind.into(),
            needs_rebaseline: value.needs_rebaseline,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WireScopeState {
    pub status: WireScopeStatus,
    pub actor_kind: WireActorKind,
    pub worktree_id: WireWorktreeId,
}

impl From<WireScopeState> for ScopeState {
    fn from(value: WireScopeState) -> Self {
        ScopeState {
            status: value.status.into(),
            actor_kind: value.actor_kind.into(),
            worktree_id: value.worktree_id.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WireAttemptState {
    pub status: WireAttemptStatus,
    pub boundary: WireBoundary,
    pub expected_revision: u64,
    pub before_tree: WireTreeId,
    pub after_tree: WireTreeId,
}

impl From<WireAttemptState> for AttemptState {
    fn from(value: WireAttemptState) -> Self {
        AttemptState {
            status: value.status.into(),
            boundary: value.boundary.into(),
            expected_revision: value.expected_revision,
            before_tree: value.before_tree.into(),
            after_tree: value.after_tree.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WireEventKey {
    pub scope_id: WireScopeId,
    pub event_id: WireEventId,
}

impl From<WireEventKey> for EventKey {
    fn from(value: WireEventKey) -> Self {
        EventKey {
            scope_id: value.scope_id.into(),
            event_id: value.event_id.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WireMutationEvent {
    pub worktree_id: WireWorktreeId,
    pub revision: u64,
    pub before_tree: WireTreeId,
    pub after_tree: WireTreeId,
    pub active_scopes: BTreeSet<WireScopeId>,
    pub tainted: bool,
    pub failure_kind: WireFailureKind,
    pub attribution: WireAttribution,
    pub boundary: WireBoundary,
}

impl From<WireMutationEvent> for MutationEvent {
    fn from(value: WireMutationEvent) -> Self {
        MutationEvent {
            worktree_id: value.worktree_id.into(),
            revision: value.revision,
            before_tree: value.before_tree.into(),
            after_tree: value.after_tree.into(),
            active_scopes: value.active_scopes.into_iter().map(Into::into).collect(),
            tainted: value.tainted,
            failure_kind: value.failure_kind.into(),
            attribution: value.attribution.into(),
            boundary: value.boundary.into(),
        }
    }
}

// ---------------------------------------------------------------------
// Comparable model state
// ---------------------------------------------------------------------

/// The comparable subset of `spec/mutation_cursor.qnt`'s state: every
/// variable named by AC5 (`worktrees`, `scopes`, `worktreeTrees`,
/// `externalTaint`, `processedEvents`, `attempts`, `mutationEvents`),
/// expressed in this crate's own domain types. `mbtAction` has no field here
/// and is dropped by `serde`'s default unknown-field handling when
/// [`WireModelState`] deserializes the full top-level state record.
#[derive(Debug, Eq, PartialEq, Deserialize)]
#[serde(from = "WireModelState")]
pub struct ModelState {
    pub worktrees: BTreeMap<WorktreeId, WorktreeState>,
    pub scopes: BTreeMap<ScopeId, ScopeState>,
    pub worktree_trees: BTreeMap<WorktreeId, TreeId>,
    pub external_taint: BTreeSet<WorktreeId>,
    pub processed_events: BTreeSet<EventKey>,
    pub attempts: BTreeMap<AttemptId, AttemptState>,
    pub mutation_events: BTreeSet<MutationEvent>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireModelState {
    worktrees: BTreeMap<WireWorktreeId, WireWorktreeState>,
    scopes: BTreeMap<WireScopeId, WireScopeState>,
    worktree_trees: BTreeMap<WireWorktreeId, WireTreeId>,
    external_taint: BTreeSet<WireWorktreeId>,
    processed_events: BTreeSet<WireEventKey>,
    attempts: BTreeMap<WireAttemptId, WireAttemptState>,
    mutation_events: BTreeSet<WireMutationEvent>,
}

impl From<WireModelState> for ModelState {
    fn from(value: WireModelState) -> Self {
        ModelState {
            worktrees: value
                .worktrees
                .into_iter()
                .map(|(id, state)| (id.into(), state.into()))
                .collect(),
            scopes: value
                .scopes
                .into_iter()
                .map(|(id, state)| (id.into(), state.into()))
                .collect(),
            worktree_trees: value
                .worktree_trees
                .into_iter()
                .map(|(id, tree)| (id.into(), tree.into()))
                .collect(),
            external_taint: value.external_taint.into_iter().map(Into::into).collect(),
            processed_events: value.processed_events.into_iter().map(Into::into).collect(),
            attempts: value
                .attempts
                .into_iter()
                .map(|(id, state)| (id.into(), state.into()))
                .collect(),
            mutation_events: value.mutation_events.into_iter().map(Into::into).collect(),
        }
    }
}
