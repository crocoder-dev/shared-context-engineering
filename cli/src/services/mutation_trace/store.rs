//! Domain<->SQL codecs and bounded read access for the mutation-cursor
//! persistence layer.
//!
//! The codecs are the only translation between `super::types` domain values
//! and the `TEXT`/`BLOB` representations `cli/migrations/agent-trace-
//! repository/004_mutation_trace_protocol.sql` constrains those columns to.
//! Every codec here is an explicit function over a fixed set of variants — no
//! codec derives from `Debug` or a serde representation, so a variant rename
//! cannot silently change the durable encoding.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Context, Result};

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::db::TransactionStatement;

use super::types::{
    ActorKind, Attribution, Boundary, EventId, EventKey, FailureKind, MutationEvent, ProtocolState,
    ScopeId, ScopeState, ScopeStatus, TreeId, WorktreeId, WorktreeState,
};

/// Encodes a worktree/event revision as the 8-byte big-endian `BLOB` stored
/// by every `revision` column in migration `004`.
pub fn encode_revision(revision: u64) -> [u8; 8] {
    revision.to_be_bytes()
}

/// Decodes a worktree/event revision from the 8-byte big-endian `BLOB`
/// migration `004`'s `CHECK (typeof(revision) = 'blob' AND length(revision)
/// = 8)` constraint guarantees on every stored value.
pub fn decode_revision(blob: &[u8]) -> Result<u64> {
    let bytes: [u8; 8] = blob.try_into().map_err(|_| {
        anyhow::anyhow!("revision blob must be exactly 8 bytes, got {}", blob.len())
    })?;
    Ok(u64::from_be_bytes(bytes))
}

/// Encodes an [`ActorKind`] as the `mutation_trace_scopes.actor_kind` `TEXT`
/// value migration `004`'s `CHECK (actor_kind IN (...))` allow-list expects.
pub fn encode_actor_kind(actor_kind: ActorKind) -> &'static str {
    match actor_kind {
        ActorKind::ClaudeCode => "claude_code",
        ActorKind::Codex => "codex",
        ActorKind::OpenCode => "opencode",
        ActorKind::Pi => "pi",
    }
}

/// Decodes an [`ActorKind`] from `mutation_trace_scopes.actor_kind`.
pub fn decode_actor_kind(value: &str) -> Result<ActorKind> {
    match value {
        "claude_code" => Ok(ActorKind::ClaudeCode),
        "codex" => Ok(ActorKind::Codex),
        "opencode" => Ok(ActorKind::OpenCode),
        "pi" => Ok(ActorKind::Pi),
        other => bail!("unrecognized actor_kind: {other:?}"),
    }
}

/// Encodes a [`FailureKind`] as the `failure_kind` `TEXT` value migration
/// `003` constrains `mutation_trace_worktrees.failure_kind` and
/// `mutation_trace_events.failure_kind` to.
pub fn encode_failure_kind(failure_kind: FailureKind) -> &'static str {
    match failure_kind {
        FailureKind::Healthy => "healthy",
        FailureKind::SnapshotFailure => "snapshot_failure",
    }
}

/// Decodes a [`FailureKind`] from a `failure_kind` column.
pub fn decode_failure_kind(value: &str) -> Result<FailureKind> {
    match value {
        "healthy" => Ok(FailureKind::Healthy),
        "snapshot_failure" => Ok(FailureKind::SnapshotFailure),
        other => bail!("unrecognized failure_kind: {other:?}"),
    }
}

/// Encodes a [`ScopeStatus`] as the `mutation_trace_scopes.status` `TEXT`
/// value migration `004`'s `CHECK (status IN (...))` allow-list expects.
pub fn encode_scope_status(status: ScopeStatus) -> &'static str {
    match status {
        ScopeStatus::NeverSeen => "never_seen",
        ScopeStatus::Active => "active",
        ScopeStatus::Closed => "closed",
        ScopeStatus::Abandoned => "abandoned",
    }
}

/// Decodes a [`ScopeStatus`] from `mutation_trace_scopes.status`.
pub fn decode_scope_status(value: &str) -> Result<ScopeStatus> {
    match value {
        "never_seen" => Ok(ScopeStatus::NeverSeen),
        "active" => Ok(ScopeStatus::Active),
        "closed" => Ok(ScopeStatus::Closed),
        "abandoned" => Ok(ScopeStatus::Abandoned),
        other => bail!("unrecognized scope status: {other:?}"),
    }
}

/// [`Attribution`]'s discriminant, decoupled from its `AiExclusive` payload
/// (`ScopeId`). Reconstructing a full [`Attribution`] from a persisted row
/// also needs `attribution_scope_id`, which is a `mutation_trace_events`
/// query concern owned by a later task, not by this codec.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttributionKind {
    IneligibleUnscoped,
    AiExclusive,
    AiContended,
}

/// The discriminant of an [`Attribution`] value.
pub fn attribution_kind(attribution: &Attribution) -> AttributionKind {
    match attribution {
        Attribution::IneligibleUnscoped => AttributionKind::IneligibleUnscoped,
        Attribution::AiExclusive(_) => AttributionKind::AiExclusive,
        Attribution::AiContended => AttributionKind::AiContended,
    }
}

/// Encodes an [`AttributionKind`] as the
/// `mutation_trace_events.attribution_kind` `TEXT` value migration `004`'s
/// `CHECK (attribution_kind IN (...))` allow-list expects.
pub fn encode_attribution_kind(kind: AttributionKind) -> &'static str {
    match kind {
        AttributionKind::IneligibleUnscoped => "ineligible_unscoped",
        AttributionKind::AiExclusive => "ai_exclusive",
        AttributionKind::AiContended => "ai_contended",
    }
}

/// Decodes an [`AttributionKind`] from `mutation_trace_events.attribution_kind`.
pub fn decode_attribution_kind(value: &str) -> Result<AttributionKind> {
    match value {
        "ineligible_unscoped" => Ok(AttributionKind::IneligibleUnscoped),
        "ai_exclusive" => Ok(AttributionKind::AiExclusive),
        "ai_contended" => Ok(AttributionKind::AiContended),
        other => bail!("unrecognized attribution_kind: {other:?}"),
    }
}

/// [`Boundary`]'s discriminant, decoupled from its `scope`/`event`/`worktree`
/// payload. Reconstructing a full [`Boundary`] from a persisted row also
/// needs `boundary_scope_id`/`boundary_event_id`, which is a
/// `mutation_trace_events` query concern owned by a later task, not by this
/// codec.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryKind {
    Start,
    Advance,
    Close,
    Flush,
}

/// The discriminant of a [`Boundary`] value.
pub fn boundary_kind(boundary: &Boundary) -> BoundaryKind {
    match boundary {
        Boundary::Start { .. } => BoundaryKind::Start,
        Boundary::Advance { .. } => BoundaryKind::Advance,
        Boundary::Close { .. } => BoundaryKind::Close,
        Boundary::Flush { .. } => BoundaryKind::Flush,
    }
}

/// Encodes a [`BoundaryKind`] as the `mutation_trace_events.boundary_kind`
/// `TEXT` value migration `004`'s `CHECK (boundary_kind IN (...))`
/// allow-list expects.
pub fn encode_boundary_kind(kind: BoundaryKind) -> &'static str {
    match kind {
        BoundaryKind::Start => "start",
        BoundaryKind::Advance => "advance",
        BoundaryKind::Close => "close",
        BoundaryKind::Flush => "flush",
    }
}

/// Decodes a [`BoundaryKind`] from `mutation_trace_events.boundary_kind`.
pub fn decode_boundary_kind(value: &str) -> Result<BoundaryKind> {
    match value {
        "start" => Ok(BoundaryKind::Start),
        "advance" => Ok(BoundaryKind::Advance),
        "close" => Ok(BoundaryKind::Close),
        "flush" => Ok(BoundaryKind::Flush),
        other => bail!("unrecognized boundary_kind: {other:?}"),
    }
}

const SELECT_WORKTREE_SQL: &str =
    "SELECT cursor_tree, revision, tainted, failure_kind, needs_rebaseline
     FROM mutation_trace_worktrees WHERE worktree_id = ?1";
const SELECT_SCOPES_BY_WORKTREE_AND_STATUS_SQL: &str =
    "SELECT scope_id, worktree_id, actor_kind, status
     FROM mutation_trace_scopes WHERE worktree_id = ?1 AND status = ?2";
const SELECT_SCOPE_BY_ID_SQL: &str = "SELECT scope_id, worktree_id, actor_kind, status
     FROM mutation_trace_scopes WHERE scope_id = ?1";
const SELECT_PROCESSED_EVENT_SQL: &str =
    "SELECT 1 FROM mutation_trace_processed_events WHERE scope_id = ?1 AND event_id = ?2";
const SELECT_MUTATION_EVENT_SQL: &str = "SELECT before_tree, after_tree, tainted, failure_kind,
            attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id
     FROM mutation_trace_events WHERE worktree_id = ?1 AND revision = ?2";
const SELECT_MUTATION_EVENT_ACTIVE_SCOPES_SQL: &str =
    "SELECT scope_id FROM mutation_trace_event_active_scopes WHERE worktree_id = ?1 AND revision = ?2";
/// One worktree's complete durable tree root set — its cursor tree plus the
/// `before_tree` / `after_tree` of every historical `mutation_trace_events`
/// row — as a single `UNION` statement so the whole set is read from one
/// database snapshot, never assembled from independent `SELECT`s.
const SELECT_TREE_ROOTS_BY_WORKTREE_SQL: &str =
    "SELECT cursor_tree AS tree FROM mutation_trace_worktrees WHERE worktree_id = ?1
     UNION
     SELECT before_tree AS tree FROM mutation_trace_events    WHERE worktree_id = ?1
     UNION
     SELECT after_tree  AS tree FROM mutation_trace_events    WHERE worktree_id = ?1";
/// The same three `TreeId` columns unioned across **every** worktree in the
/// repository, in one statement / one snapshot — the reconciler's
/// repository-wide retention set.
const SELECT_ALL_TREE_ROOTS_SQL: &str = "SELECT cursor_tree AS tree FROM mutation_trace_worktrees
     UNION
     SELECT before_tree AS tree FROM mutation_trace_events
     UNION
     SELECT after_tree  AS tree FROM mutation_trace_events";
/// Idle-insert: only takes effect when `worktree_id` has no row yet, so an
/// existing worktree's cursor/revision/failure state is never overwritten.
const INSERT_WORKTREE_IF_ABSENT_SQL: &str = "INSERT INTO mutation_trace_worktrees
        (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline)
     VALUES (?1, ?2, ?3, 0, 'healthy', 0)
     ON CONFLICT (worktree_id) DO NOTHING";
/// Idle-insert: only takes effect when `scope_id` has no row yet, so an
/// existing scope's worktree/actor/status is never overwritten. The caller
/// re-reads the row afterward to detect a worktree/actor mismatch.
const INSERT_SCOPE_IF_ABSENT_SQL: &str =
    "INSERT INTO mutation_trace_scopes (scope_id, worktree_id, actor_kind, status)
     VALUES (?1, ?2, ?3, 'never_seen')
     ON CONFLICT (scope_id) DO NOTHING";
const UPDATE_WORKTREE_CAS_SQL: &str = "UPDATE mutation_trace_worktrees
     SET cursor_tree = ?1, revision = ?2, tainted = ?3, failure_kind = ?4, needs_rebaseline = ?5,
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
     WHERE worktree_id = ?6 AND revision = ?7";
const UPDATE_SCOPE_STATUS_SQL: &str = "UPDATE mutation_trace_scopes
     SET status = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
     WHERE scope_id = ?2";
const INSERT_PROCESSED_EVENT_SQL: &str =
    "INSERT INTO mutation_trace_processed_events (scope_id, event_id) VALUES (?1, ?2)";
const INSERT_MUTATION_EVENT_SQL: &str = "INSERT INTO mutation_trace_events
        (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
         attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)";
const INSERT_MUTATION_EVENT_ACTIVE_SCOPE_SQL: &str =
    "INSERT INTO mutation_trace_event_active_scopes (worktree_id, revision, scope_id) VALUES (?1, ?2, ?3)";

/// Bounded runtime projection of one worktree's durable protocol state,
/// loaded by [`MutationTraceStore::load_worktree`]. Scoped to that worktree's
/// currently `Active` scopes plus, when present, the scope `load_worktree`
/// was explicitly asked about (regardless of its status) — never every
/// historical scope, and never a `mutation_trace_events` row.
///
/// `attempts`, `mutation_events`, and `external_taint` are always empty:
/// `AttemptState` is transient and never persisted, historical
/// `MutationEvent`s are a cold-path concern
/// ([`MutationTraceStore::load_mutation_event`]), and `external_taint` is
/// never DB-authoritative (see the plan's non-goals).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeProjection {
    pub worktree_id: WorktreeId,
    pub worktree_state: WorktreeState,
    pub scopes: BTreeMap<ScopeId, ScopeState>,
    pub processed_events: BTreeSet<EventKey>,
}

impl WorktreeProjection {
    /// Widens this bounded projection into a full [`ProtocolState`] so pure
    /// `protocol.rs` functions can operate on it unchanged. `worktrees`
    /// carries only the one loaded worktree; `attempts`, `mutation_events`,
    /// and `external_taint` are always empty.
    pub fn into_protocol_state(self) -> ProtocolState {
        let mut worktrees = BTreeMap::new();
        worktrees.insert(self.worktree_id, self.worktree_state);

        ProtocolState {
            worktrees,
            scopes: self.scopes,
            external_taint: BTreeSet::new(),
            processed_events: self.processed_events,
            attempts: BTreeMap::new(),
            mutation_events: BTreeSet::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableTransition {
    worktree: WorktreeId,
    expected_revision: u64,
    next_worktree_state: WorktreeState,
    scope_status_changes: BTreeMap<ScopeId, ScopeStatus>,
    new_processed_event: Option<EventKey>,
    new_mutation_event: Option<MutationEvent>,
}

impl DurableTransition {
    pub fn between(
        before: &ProtocolState,
        after: &ProtocolState,
        worktree: &WorktreeId,
    ) -> Result<Option<Self>> {
        let (before_worktree_state, after_worktree_state) =
            diff_target_worktree(before, after, worktree)?;
        let scope_status_changes = diff_scopes(before, after, worktree)?;
        let new_processed_event = diff_new_processed_event(before, after, worktree)?;
        let new_mutation_event = diff_new_mutation_event(before, after, worktree)?;

        let no_change = before_worktree_state == after_worktree_state
            && scope_status_changes.is_empty()
            && new_processed_event.is_none()
            && new_mutation_event.is_none();

        if no_change {
            return Ok(None);
        }

        let expected_revision = before_worktree_state.revision;
        let next_revision = expected_revision.checked_add(1);
        if Some(after_worktree_state.revision) != next_revision {
            bail!(
                "worktree {worktree:?} revision must advance by exactly one from {expected_revision}, got {}",
                after_worktree_state.revision
            );
        }

        if let Some(event) = &new_mutation_event {
            if event.revision != after_worktree_state.revision {
                bail!(
                    "new mutation event revision {} does not match worktree {worktree:?}'s resulting revision {}",
                    event.revision,
                    after_worktree_state.revision
                );
            }
        }

        Ok(Some(Self {
            worktree: worktree.clone(),
            expected_revision,
            next_worktree_state: after_worktree_state.clone(),
            scope_status_changes,
            new_processed_event,
            new_mutation_event,
        }))
    }
}

fn diff_target_worktree<'s>(
    before: &'s ProtocolState,
    after: &'s ProtocolState,
    worktree: &WorktreeId,
) -> Result<(&'s WorktreeState, &'s WorktreeState)> {
    let Some(before_worktree_state) = before.worktrees.get(worktree) else {
        bail!("worktree {worktree:?} missing from before state");
    };
    let Some(after_worktree_state) = after.worktrees.get(worktree) else {
        bail!("worktree {worktree:?} missing from after state");
    };

    if before.worktrees.len() != after.worktrees.len() {
        bail!("worktree set changed between before and after");
    }
    for (id, before_state) in &before.worktrees {
        if id == worktree {
            continue;
        }
        match after.worktrees.get(id) {
            Some(after_state) if after_state == before_state => {}
            _ => bail!("unrelated worktree {id:?} changed"),
        }
    }

    Ok((before_worktree_state, after_worktree_state))
}

fn diff_scopes(
    before: &ProtocolState,
    after: &ProtocolState,
    worktree: &WorktreeId,
) -> Result<BTreeMap<ScopeId, ScopeStatus>> {
    let before_scope_ids: BTreeSet<&ScopeId> = before.scopes.keys().collect();
    let after_scope_ids: BTreeSet<&ScopeId> = after.scopes.keys().collect();
    if before_scope_ids != after_scope_ids {
        bail!("scope set changed between before and after");
    }

    let mut scope_status_changes = BTreeMap::new();
    for (scope_id, before_scope) in &before.scopes {
        let after_scope = after
            .scopes
            .get(scope_id)
            .expect("scope key sets already verified equal");

        if before_scope.worktree_id != after_scope.worktree_id {
            bail!("scope {scope_id:?} worktree_id changed");
        }
        if before_scope.actor_kind != after_scope.actor_kind {
            bail!("scope {scope_id:?} actor_kind changed");
        }
        if before_scope.status != after_scope.status {
            if before_scope.worktree_id != *worktree {
                bail!(
                    "scope {scope_id:?} status changed but belongs to worktree {:?}, not {worktree:?}",
                    before_scope.worktree_id
                );
            }
            scope_status_changes.insert(scope_id.clone(), after_scope.status);
        }
    }

    Ok(scope_status_changes)
}

fn diff_new_processed_event(
    before: &ProtocolState,
    after: &ProtocolState,
    worktree: &WorktreeId,
) -> Result<Option<EventKey>> {
    if !before.processed_events.is_subset(&after.processed_events) {
        bail!("a processed_events entry disappeared");
    }
    let new_processed_events: Vec<&EventKey> = after
        .processed_events
        .difference(&before.processed_events)
        .collect();
    if new_processed_events.len() > 1 {
        bail!("more than one new processed_events entry");
    }

    let Some(key) = new_processed_events.first() else {
        return Ok(None);
    };
    let scope = after.scopes.get(&key.scope_id).ok_or_else(|| {
        anyhow::anyhow!("new processed event {key:?} has no scope in after state")
    })?;
    if scope.worktree_id != *worktree {
        bail!(
            "new processed event {key:?} belongs to worktree {:?}, not {worktree:?}",
            scope.worktree_id
        );
    }
    Ok(Some((*key).clone()))
}

fn diff_new_mutation_event(
    before: &ProtocolState,
    after: &ProtocolState,
    worktree: &WorktreeId,
) -> Result<Option<MutationEvent>> {
    if !before.mutation_events.is_subset(&after.mutation_events) {
        bail!("a mutation_events entry disappeared");
    }
    let new_mutation_events: Vec<&MutationEvent> = after
        .mutation_events
        .difference(&before.mutation_events)
        .collect();
    if new_mutation_events.len() > 1 {
        bail!("more than one new mutation_events entry");
    }

    let Some(event) = new_mutation_events.first() else {
        return Ok(None);
    };
    if event.worktree_id != *worktree {
        bail!(
            "new mutation event belongs to worktree {:?}, not {worktree:?}",
            event.worktree_id
        );
    }
    Ok(Some((*event).clone()))
}

pub struct MutationTraceStore<'a> {
    db: &'a RepositoryAgentTraceDb,
}

impl<'a> MutationTraceStore<'a> {
    pub fn new(db: &'a RepositoryAgentTraceDb) -> Self {
        Self { db }
    }

    /// Idempotently initializes `worktree`'s durable cursor row: `revision=0`,
    /// healthy, not tainted, not needing rebaseline, with `cursor_tree` set to
    /// `initial_tree`. A no-op when the worktree row already exists — an
    /// existing cursor, revision, or failure state is never overwritten.
    pub fn initialize_worktree(&self, worktree: &WorktreeId, initial_tree: &TreeId) -> Result<()> {
        self.db.execute(
            INSERT_WORKTREE_IF_ABSENT_SQL,
            (
                worktree.0.as_str(),
                initial_tree.0.as_str(),
                encode_revision(0).as_slice(),
            ),
        )?;

        Ok(())
    }

    /// Idempotently registers `scope` as belonging to `worktree` and
    /// `actor_kind`. Inserts a new `NeverSeen` row when `scope` has none yet.
    /// When a row already exists, returns its current state unchanged as long
    /// as its `worktree_id` and `actor_kind` agree with the arguments — this
    /// never resurrects a terminal scope or changes its status — and returns
    /// `Err` when either disagrees, since a scope's worktree and actor are
    /// permanent facts fixed at first registration.
    ///
    /// `worktree` must already have a durable `mutation_trace_worktrees` row
    /// (via [`MutationTraceStore::initialize_worktree`]), checked before any
    /// scope row is inserted or read back — this never auto-creates the
    /// worktree. This applies identically to a fresh `scope` and to an
    /// existing one: an existing scope whose stored `worktree_id` has no
    /// worktree row is never returned as valid merely because it matches the
    /// arguments.
    pub fn register_scope(
        &self,
        scope: &ScopeId,
        worktree: &WorktreeId,
        actor_kind: ActorKind,
    ) -> Result<ScopeState> {
        if self.load_worktree_state(worktree)?.is_none() {
            bail!(
                "cannot register scope {scope:?}: worktree {worktree:?} has no mutation_trace_worktrees row"
            );
        }

        self.db.execute(
            INSERT_SCOPE_IF_ABSENT_SQL,
            (
                scope.0.as_str(),
                worktree.0.as_str(),
                encode_actor_kind(actor_kind),
            ),
        )?;

        let scope_state = self.load_scope(scope)?.ok_or_else(|| {
            anyhow::anyhow!("scope {scope:?} has no row immediately after register_scope insert")
        })?;

        if scope_state.worktree_id != *worktree {
            bail!(
                "scope {scope:?} is already registered to worktree {:?}, not {worktree:?}",
                scope_state.worktree_id
            );
        }

        if scope_state.actor_kind != actor_kind {
            bail!(
                "scope {scope:?} is already registered to actor {:?}, not {actor_kind:?}",
                scope_state.actor_kind
            );
        }

        Ok(scope_state)
    }

    /// Loads a bounded projection of `worktree`'s durable protocol state, or
    /// `None` when the worktree does not exist.
    ///
    /// `scope` and `event_key.scope_id` are two ways of naming the same
    /// operation-local scope identity: when both are supplied they must
    /// agree, or this returns `Err` before loading or querying anything.
    /// Otherwise the supplied `scope`, or `event_key.scope_id` when only
    /// `event_key` is supplied, becomes the effective referenced scope: a
    /// durable `mutation_trace_scopes` row for it must exist, or this returns
    /// `Err` — a missing effective scope is never silently omitted from the
    /// projection. When it exists it is loaded and included in the
    /// projection regardless of its status, and this returns `Err` if it
    /// belongs to a worktree other than the one requested. Both checks run
    /// before the `processed_events` replay lookup, so an orphan
    /// `mutation_trace_processed_events` row can never enter the projection
    /// without its owning scope. The projection's `scopes` otherwise contains
    /// only this worktree's currently `Active` scopes. `processed_events`
    /// contains `event_key` only when a matching `(scope_id, event_id)` row
    /// already exists; the lookup never references a `worktree_id` column,
    /// since `mutation_trace_processed_events` has none. This method never
    /// queries `mutation_trace_events`.
    pub fn load_worktree(
        &self,
        worktree: &WorktreeId,
        scope: Option<&ScopeId>,
        event_key: Option<&EventKey>,
    ) -> Result<Option<WorktreeProjection>> {
        let effective_scope = effective_referenced_scope(scope, event_key)?;

        let Some(worktree_state) = self.load_worktree_state(worktree)? else {
            return Ok(None);
        };

        let mut scopes = self.load_active_scopes(worktree)?;

        if let Some(effective_scope_id) = effective_scope {
            if !scopes.contains_key(effective_scope_id) {
                let scope_state = self.load_scope(effective_scope_id)?.ok_or_else(|| {
                    anyhow::anyhow!(
                        "effective referenced scope {effective_scope_id:?} has no mutation_trace_scopes row"
                    )
                })?;
                if scope_state.worktree_id != *worktree {
                    bail!(
                        "scope {:?} belongs to worktree {:?}, not the requested worktree {:?}",
                        effective_scope_id,
                        scope_state.worktree_id,
                        worktree
                    );
                }
                scopes.insert(effective_scope_id.clone(), scope_state);
            }
        }

        let processed_events = match event_key {
            Some(event_key) if self.processed_event_exists(event_key)? => {
                let mut processed_events = BTreeSet::new();
                processed_events.insert(event_key.clone());
                processed_events
            }
            _ => BTreeSet::new(),
        };

        Ok(Some(WorktreeProjection {
            worktree_id: worktree.clone(),
            worktree_state,
            scopes,
            processed_events,
        }))
    }

    /// Reconstructs one historical [`MutationEvent`] for `(worktree,
    /// revision)`, decoding its full `Attribution` and `Boundary`, or `None`
    /// when no such row exists. Never called from `load_worktree` or from
    /// any hook-boundary path.
    pub fn load_mutation_event(
        &self,
        worktree: &WorktreeId,
        revision: u64,
    ) -> Result<Option<MutationEvent>> {
        let revision_blob = encode_revision(revision);

        let rows = self.db.query_map(
            SELECT_MUTATION_EVENT_SQL,
            (worktree.0.as_str(), revision_blob.as_slice()),
            mutation_event_row_from_turso,
        )?;

        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };

        let active_scopes = self.load_mutation_event_active_scopes(worktree, &revision_blob)?;

        Ok(Some(MutationEvent {
            worktree_id: worktree.clone(),
            revision,
            before_tree: TreeId(row.before_tree),
            after_tree: TreeId(row.after_tree),
            active_scopes,
            tainted: row.tainted,
            failure_kind: row.failure_kind,
            attribution: reconstruct_attribution(row.attribution_kind, row.attribution_scope_id)?,
            boundary: reconstruct_boundary(
                row.boundary_kind,
                worktree,
                row.boundary_scope_id,
                row.boundary_event_id,
            )?,
        }))
    }

    /// Reads `worktree`'s complete durable tree root set: its
    /// `mutation_trace_worktrees.cursor_tree`, plus the `before_tree` and
    /// `after_tree` of every `mutation_trace_events` row for `worktree`,
    /// deduplicated. Returns an empty set (not an error) when `worktree` has
    /// no durable row at all.
    ///
    /// Read-only, cold path — never called from `load_worktree` or any
    /// hook-boundary path, exactly like [`MutationTraceStore::load_mutation_event`].
    /// It reads only the three `TreeId` columns above: never
    /// `mutation_trace_scopes` / `mutation_trace_processed_events` /
    /// `mutation_trace_event_active_scopes`, never another worktree's trees,
    /// and never transient `AttemptState` / `external_taint`.
    ///
    /// The whole set is produced by **one** SQL statement (a `UNION` of the
    /// three columns) through **one** `query_map` call, so a concurrent
    /// mutation-cursor commit — which atomically moves `cursor_tree` from `T`
    /// to `X` and inserts `MutationEvent { before_tree = T, after_tree = X }`
    /// in the same transaction — cannot expose a torn root set that omits `T`:
    /// the single statement observes either the pre-commit snapshot
    /// (`cursor_tree` still contains `T`) or the post-commit snapshot
    /// (`before_tree` contains `T`).
    pub fn load_tree_roots(&self, worktree: &WorktreeId) -> Result<BTreeSet<TreeId>> {
        let rows = self.db.query_map(
            SELECT_TREE_ROOTS_BY_WORKTREE_SQL,
            (worktree.0.as_str(),),
            tree_root_row_from_turso,
        )?;

        Ok(rows.into_iter().collect())
    }

    /// Reads the repository-wide durable tree root set: the union of
    /// `mutation_trace_worktrees.cursor_tree`, `mutation_trace_events.before_tree`,
    /// and `mutation_trace_events.after_tree` across **every** worktree,
    /// deduplicated. Returns an empty set (not an error) for a repository with
    /// no mutation-cursor rows.
    ///
    /// This is the reconciler's retention set: linked worktrees share one Git
    /// object database, so a ref owned by worktree `A` may be the last SCE ref
    /// protecting a tree that only worktree `B` durably requires. Read-only,
    /// cold path, and — like [`MutationTraceStore::load_tree_roots`] — one SQL
    /// statement through one `query_map` call, so it cannot tear across a
    /// concurrent atomic `cursor T -> X` + `event T -> X` commit on another
    /// worktree.
    pub fn load_all_tree_roots(&self) -> Result<BTreeSet<TreeId>> {
        let rows = self
            .db
            .query_map(SELECT_ALL_TREE_ROOTS_SQL, (), tree_root_row_from_turso)?;

        Ok(rows.into_iter().collect())
    }

    /// Loads the durable [`ScopeState`] for `scope_id` — its status,
    /// `actor_kind`, and `worktree_id` — or `None` when no
    /// `mutation_trace_scopes` row exists for it.
    ///
    /// A cold-path single-row read, and deliberately the narrowest scope seam
    /// there is: it reads one `mutation_trace_scopes` row and nothing else. It
    /// never consults `mutation_trace_events`,
    /// `mutation_trace_processed_events`, or the scope's
    /// `mutation_trace_worktrees` row, and it must not widen into a
    /// projection — [`MutationTraceStore::load_worktree`] is the projection
    /// seam, and a caller needing worktree state alongside a scope belongs
    /// there instead.
    ///
    /// This never adjudicates worktree identity: a scope whose `worktree_id`
    /// differs from the caller's own worktree is returned as-is, not rejected.
    /// Comparing the two is the caller's decision, since the same row is a
    /// legitimate read from its owning worktree and a cross-worktree reference
    /// from any other.
    pub fn load_scope(&self, scope_id: &ScopeId) -> Result<Option<ScopeState>> {
        let rows = self.db.query_map(
            SELECT_SCOPE_BY_ID_SQL,
            (scope_id.0.as_str(),),
            scope_row_from_turso,
        )?;

        Ok(rows.into_iter().next().map(|(_, scope_state)| scope_state))
    }

    fn load_worktree_state(&self, worktree: &WorktreeId) -> Result<Option<WorktreeState>> {
        let rows = self.db.query_map(
            SELECT_WORKTREE_SQL,
            (worktree.0.as_str(),),
            worktree_state_row_from_turso,
        )?;

        Ok(rows.into_iter().next())
    }

    fn load_active_scopes(&self, worktree: &WorktreeId) -> Result<BTreeMap<ScopeId, ScopeState>> {
        let rows = self.db.query_map(
            SELECT_SCOPES_BY_WORKTREE_AND_STATUS_SQL,
            (
                worktree.0.as_str(),
                encode_scope_status(ScopeStatus::Active),
            ),
            scope_row_from_turso,
        )?;

        Ok(rows.into_iter().collect())
    }

    fn processed_event_exists(&self, event_key: &EventKey) -> Result<bool> {
        let rows = self.db.query_map(
            SELECT_PROCESSED_EVENT_SQL,
            (event_key.scope_id.0.as_str(), event_key.event_id.0.as_str()),
            |row| row.get::<i64>(0).map_err(Into::into),
        )?;

        Ok(!rows.is_empty())
    }

    fn load_mutation_event_active_scopes(
        &self,
        worktree: &WorktreeId,
        revision_blob: &[u8],
    ) -> Result<BTreeSet<ScopeId>> {
        let rows = self.db.query_map(
            SELECT_MUTATION_EVENT_ACTIVE_SCOPES_SQL,
            (worktree.0.as_str(), revision_blob),
            |row| row.get::<String>(0).map(ScopeId).map_err(Into::into),
        )?;

        Ok(rows.into_iter().collect())
    }

    pub fn commit(&self, transition: &DurableTransition) -> Result<CasResult> {
        let expected_revision_blob = encode_revision(transition.expected_revision);
        let next_revision_blob = encode_revision(transition.next_worktree_state.revision);

        let guard = TransactionStatement::new(
            UPDATE_WORKTREE_CAS_SQL,
            (
                transition.next_worktree_state.cursor_tree.0.as_str(),
                next_revision_blob.as_slice(),
                transition.next_worktree_state.tainted,
                encode_failure_kind(transition.next_worktree_state.failure_kind),
                transition.next_worktree_state.needs_rebaseline,
                transition.worktree.0.as_str(),
                expected_revision_blob.as_slice(),
            ),
        )?;

        let mut statements = Vec::new();

        for (scope_id, status) in &transition.scope_status_changes {
            statements.push(
                TransactionStatement::new(
                    UPDATE_SCOPE_STATUS_SQL,
                    (encode_scope_status(*status), scope_id.0.as_str()),
                )?
                .expect_rows_affected(1),
            );
        }

        if let Some(event_key) = &transition.new_processed_event {
            statements.push(
                TransactionStatement::new(
                    INSERT_PROCESSED_EVENT_SQL,
                    (event_key.scope_id.0.as_str(), event_key.event_id.0.as_str()),
                )?
                .expect_rows_affected(1),
            );
        }

        if let Some(event) = &transition.new_mutation_event {
            let event_revision_blob = encode_revision(event.revision);
            let attribution_scope_id = attribution_scope_id(&event.attribution);
            let (boundary_scope_id, boundary_event_id) = boundary_payload(&event.boundary);

            statements.push(
                TransactionStatement::new(
                    INSERT_MUTATION_EVENT_SQL,
                    (
                        event.worktree_id.0.as_str(),
                        event_revision_blob.as_slice(),
                        event.before_tree.0.as_str(),
                        event.after_tree.0.as_str(),
                        event.tainted,
                        encode_failure_kind(event.failure_kind),
                        encode_attribution_kind(attribution_kind(&event.attribution)),
                        attribution_scope_id,
                        encode_boundary_kind(boundary_kind(&event.boundary)),
                        boundary_scope_id,
                        boundary_event_id,
                    ),
                )?
                .expect_rows_affected(1),
            );

            for scope_id in &event.active_scopes {
                statements.push(
                    TransactionStatement::new(
                        INSERT_MUTATION_EVENT_ACTIVE_SCOPE_SQL,
                        (
                            event.worktree_id.0.as_str(),
                            event_revision_blob.as_slice(),
                            scope_id.0.as_str(),
                        ),
                    )?
                    .expect_rows_affected(1),
                );
            }
        }

        let applied = self.db.execute_transactional_cas_batch(
            "commit mutation-trace durable transition",
            "reload the worktree and retry the transition",
            &guard,
            &statements,
        )?;

        Ok(if applied {
            CasResult::Applied
        } else {
            CasResult::Conflict
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CasResult {
    Applied,
    Conflict,
}

fn attribution_scope_id(attribution: &Attribution) -> Option<&str> {
    match attribution {
        Attribution::AiExclusive(scope_id) => Some(scope_id.0.as_str()),
        Attribution::IneligibleUnscoped | Attribution::AiContended => None,
    }
}

fn boundary_payload(boundary: &Boundary) -> (Option<&str>, Option<&str>) {
    match boundary {
        Boundary::Start { scope, event }
        | Boundary::Advance { scope, event }
        | Boundary::Close { scope, event } => (Some(scope.0.as_str()), Some(event.0.as_str())),
        Boundary::Flush { .. } => (None, None),
    }
}

/// Derives the single effective referenced scope from `scope` and
/// `event_key`, per the four-case definition in the
/// `mutation-cursor-store-persistence` plan's T03: `None` when neither is
/// supplied; the supplied one when only one is; the agreeing identity when
/// both are supplied and equal; `Err` when both are supplied and disagree.
fn effective_referenced_scope<'k>(
    scope: Option<&'k ScopeId>,
    event_key: Option<&'k EventKey>,
) -> Result<Option<&'k ScopeId>> {
    match (scope, event_key) {
        (None, None) => Ok(None),
        (Some(scope_id), None) => Ok(Some(scope_id)),
        (None, Some(event_key)) => Ok(Some(&event_key.scope_id)),
        (Some(scope_id), Some(event_key)) if *scope_id == event_key.scope_id => Ok(Some(scope_id)),
        (Some(scope_id), Some(event_key)) => bail!(
            "scope {scope_id:?} and event_key.scope_id {:?} disagree",
            event_key.scope_id
        ),
    }
}

fn tree_root_row_from_turso(row: &turso::Row) -> Result<TreeId> {
    let tree: String = row
        .get(0)
        .context("failed to read a durable tree root column")?;
    Ok(TreeId(tree))
}

fn worktree_state_row_from_turso(row: &turso::Row) -> Result<WorktreeState> {
    let cursor_tree: String = row
        .get(0)
        .context("failed to read mutation_trace_worktrees.cursor_tree")?;
    let revision_blob: Vec<u8> = row
        .get(1)
        .context("failed to read mutation_trace_worktrees.revision")?;
    let tainted: bool = row
        .get(2)
        .context("failed to read mutation_trace_worktrees.tainted")?;
    let failure_kind: String = row
        .get(3)
        .context("failed to read mutation_trace_worktrees.failure_kind")?;
    let needs_rebaseline: bool = row
        .get(4)
        .context("failed to read mutation_trace_worktrees.needs_rebaseline")?;

    Ok(WorktreeState {
        cursor_tree: TreeId(cursor_tree),
        revision: decode_revision(&revision_blob)?,
        tainted,
        failure_kind: decode_failure_kind(&failure_kind)?,
        needs_rebaseline,
    })
}

fn scope_row_from_turso(row: &turso::Row) -> Result<(ScopeId, ScopeState)> {
    let scope_id: String = row
        .get(0)
        .context("failed to read mutation_trace_scopes.scope_id")?;
    let worktree_id: String = row
        .get(1)
        .context("failed to read mutation_trace_scopes.worktree_id")?;
    let actor_kind: String = row
        .get(2)
        .context("failed to read mutation_trace_scopes.actor_kind")?;
    let status: String = row
        .get(3)
        .context("failed to read mutation_trace_scopes.status")?;

    Ok((
        ScopeId(scope_id),
        ScopeState {
            status: decode_scope_status(&status)?,
            actor_kind: decode_actor_kind(&actor_kind)?,
            worktree_id: WorktreeId(worktree_id),
        },
    ))
}

/// Raw decoded `mutation_trace_events` row fields, prior to reconstructing
/// the full `Attribution`/`Boundary`/`active_scopes` a [`MutationEvent`]
/// carries.
struct MutationEventRow {
    before_tree: String,
    after_tree: String,
    tainted: bool,
    failure_kind: FailureKind,
    attribution_kind: AttributionKind,
    attribution_scope_id: Option<String>,
    boundary_kind: BoundaryKind,
    boundary_scope_id: Option<String>,
    boundary_event_id: Option<String>,
}

fn mutation_event_row_from_turso(row: &turso::Row) -> Result<MutationEventRow> {
    let before_tree: String = row
        .get(0)
        .context("failed to read mutation_trace_events.before_tree")?;
    let after_tree: String = row
        .get(1)
        .context("failed to read mutation_trace_events.after_tree")?;
    let tainted: bool = row
        .get(2)
        .context("failed to read mutation_trace_events.tainted")?;
    let failure_kind: String = row
        .get(3)
        .context("failed to read mutation_trace_events.failure_kind")?;
    let attribution_kind: String = row
        .get(4)
        .context("failed to read mutation_trace_events.attribution_kind")?;
    let attribution_scope_id: Option<String> = row
        .get(5)
        .context("failed to read mutation_trace_events.attribution_scope_id")?;
    let boundary_kind: String = row
        .get(6)
        .context("failed to read mutation_trace_events.boundary_kind")?;
    let boundary_scope_id: Option<String> = row
        .get(7)
        .context("failed to read mutation_trace_events.boundary_scope_id")?;
    let boundary_event_id: Option<String> = row
        .get(8)
        .context("failed to read mutation_trace_events.boundary_event_id")?;

    Ok(MutationEventRow {
        before_tree,
        after_tree,
        tainted,
        failure_kind: decode_failure_kind(&failure_kind)?,
        attribution_kind: decode_attribution_kind(&attribution_kind)?,
        attribution_scope_id,
        boundary_kind: decode_boundary_kind(&boundary_kind)?,
        boundary_scope_id,
        boundary_event_id,
    })
}

fn reconstruct_attribution(kind: AttributionKind, scope_id: Option<String>) -> Result<Attribution> {
    match (kind, scope_id) {
        (AttributionKind::IneligibleUnscoped, None) => Ok(Attribution::IneligibleUnscoped),
        (AttributionKind::AiContended, None) => Ok(Attribution::AiContended),
        (AttributionKind::AiExclusive, Some(scope_id)) => {
            Ok(Attribution::AiExclusive(ScopeId(scope_id)))
        }
        (kind, scope_id) => {
            bail!("inconsistent attribution row: kind={kind:?} scope_id={scope_id:?}")
        }
    }
}

fn reconstruct_boundary(
    kind: BoundaryKind,
    worktree: &WorktreeId,
    scope_id: Option<String>,
    event_id: Option<String>,
) -> Result<Boundary> {
    match kind {
        BoundaryKind::Flush => {
            if scope_id.is_some() || event_id.is_some() {
                bail!("flush boundary row must not carry boundary_scope_id/boundary_event_id");
            }
            Ok(Boundary::Flush {
                worktree: worktree.clone(),
            })
        }
        BoundaryKind::Start | BoundaryKind::Advance | BoundaryKind::Close => {
            let scope = scope_id
                .map(ScopeId)
                .ok_or_else(|| anyhow::anyhow!("hook boundary row missing boundary_scope_id"))?;
            let event = event_id
                .map(EventId)
                .ok_or_else(|| anyhow::anyhow!("hook boundary row missing boundary_event_id"))?;

            Ok(match kind {
                BoundaryKind::Start => Boundary::Start { scope, event },
                BoundaryKind::Advance => Boundary::Advance { scope, event },
                BoundaryKind::Close => Boundary::Close { scope, event },
                BoundaryKind::Flush => unreachable!("Flush handled above"),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;
    use crate::services::mutation_trace::protocol::{
        abandon, commit, database_failure, prepare, recover, taint,
    };
    use crate::services::mutation_trace::types::{AttemptId, EventId, ScopeId};

    #[test]
    fn revision_round_trips_at_boundary_values() {
        for revision in [0u64, 1, i64::MAX as u64, (i64::MAX as u64) + 1, u64::MAX] {
            let encoded = encode_revision(revision);
            assert_eq!(encoded.len(), 8);
            assert_eq!(decode_revision(&encoded).unwrap(), revision);
        }
    }

    #[test]
    fn decode_revision_rejects_wrong_length() {
        assert!(decode_revision(&[0u8; 7]).is_err());
        assert!(decode_revision(&[0u8; 9]).is_err());
    }

    #[test]
    fn actor_kind_round_trips_every_variant() {
        for actor_kind in [
            ActorKind::ClaudeCode,
            ActorKind::Codex,
            ActorKind::OpenCode,
            ActorKind::Pi,
        ] {
            let encoded = encode_actor_kind(actor_kind);
            assert_eq!(decode_actor_kind(encoded).unwrap(), actor_kind);
        }
    }

    #[test]
    fn decode_actor_kind_rejects_unknown_value() {
        assert!(decode_actor_kind("unknown").is_err());
    }

    #[test]
    fn failure_kind_round_trips_every_variant() {
        for failure_kind in [FailureKind::Healthy, FailureKind::SnapshotFailure] {
            let encoded = encode_failure_kind(failure_kind);
            assert_eq!(decode_failure_kind(encoded).unwrap(), failure_kind);
        }
    }

    #[test]
    fn decode_failure_kind_rejects_unknown_value() {
        assert!(decode_failure_kind("unknown").is_err());
    }

    #[test]
    fn scope_status_round_trips_every_variant() {
        for status in [
            ScopeStatus::NeverSeen,
            ScopeStatus::Active,
            ScopeStatus::Closed,
            ScopeStatus::Abandoned,
        ] {
            let encoded = encode_scope_status(status);
            assert_eq!(decode_scope_status(encoded).unwrap(), status);
        }
    }

    #[test]
    fn decode_scope_status_rejects_unknown_value() {
        assert!(decode_scope_status("unknown").is_err());
    }

    #[test]
    fn attribution_kind_round_trips_every_variant() {
        let ineligible = Attribution::IneligibleUnscoped;
        let exclusive = Attribution::AiExclusive(ScopeId("scope-1".to_string()));
        let contended = Attribution::AiContended;

        for attribution in [&ineligible, &exclusive, &contended] {
            let kind = attribution_kind(attribution);
            let encoded = encode_attribution_kind(kind);
            assert_eq!(decode_attribution_kind(encoded).unwrap(), kind);
        }

        assert_eq!(attribution_kind(&exclusive), AttributionKind::AiExclusive);
    }

    #[test]
    fn decode_attribution_kind_rejects_unknown_value() {
        assert!(decode_attribution_kind("unknown").is_err());
    }

    #[test]
    fn boundary_kind_round_trips_every_variant() {
        let start = Boundary::Start {
            scope: ScopeId("scope-1".to_string()),
            event: EventId("event-1".to_string()),
        };
        let advance = Boundary::Advance {
            scope: ScopeId("scope-1".to_string()),
            event: EventId("event-2".to_string()),
        };
        let close = Boundary::Close {
            scope: ScopeId("scope-1".to_string()),
            event: EventId("event-3".to_string()),
        };
        let flush = Boundary::Flush {
            worktree: crate::services::mutation_trace::types::WorktreeId("wt-1".to_string()),
        };

        for boundary in [&start, &advance, &close, &flush] {
            let kind = boundary_kind(boundary);
            let encoded = encode_boundary_kind(kind);
            assert_eq!(decode_boundary_kind(encoded).unwrap(), kind);
        }
    }

    #[test]
    fn decode_boundary_kind_rejects_unknown_value() {
        assert!(decode_boundary_kind("unknown").is_err());
    }

    struct TestDbPath {
        _temp_dir: tempfile::TempDir,
        path: std::path::PathBuf,
    }

    impl TestDbPath {
        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    fn test_db_path(label: &str) -> TestDbPath {
        let temp_dir = tempfile::Builder::new()
            .prefix(&format!("sce-mutation-trace-store-{label}-"))
            .tempdir()
            .expect("test temp directory should be created");

        let path = temp_dir.path().join("agent-trace.db");

        TestDbPath {
            _temp_dir: temp_dir,
            path,
        }
    }

    fn insert_worktree(db: &RepositoryAgentTraceDb, worktree_id: &str, revision: u64) {
        db.execute(
            "INSERT INTO mutation_trace_worktrees
                (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline)
             VALUES (?1, 'tree-0', ?2, 0, 'healthy', 0)",
            (worktree_id, encode_revision(revision).as_slice()),
        )
        .expect("worktree insert should succeed");
    }

    fn insert_scope(
        db: &RepositoryAgentTraceDb,
        scope_id: &str,
        worktree_id: &str,
        status: ScopeStatus,
    ) {
        db.execute(
            "INSERT INTO mutation_trace_scopes (scope_id, worktree_id, actor_kind, status)
             VALUES (?1, ?2, 'claude_code', ?3)",
            (scope_id, worktree_id, encode_scope_status(status)),
        )
        .expect("scope insert should succeed");
    }

    fn insert_processed_event(db: &RepositoryAgentTraceDb, scope_id: &str, event_id: &str) {
        db.execute(
            "INSERT INTO mutation_trace_processed_events (scope_id, event_id) VALUES (?1, ?2)",
            (scope_id, event_id),
        )
        .expect("processed-event insert should succeed");
    }

    fn insert_active_scope(
        db: &RepositoryAgentTraceDb,
        worktree_id: &str,
        revision: u64,
        scope_id: &str,
    ) {
        db.execute(
            "INSERT INTO mutation_trace_event_active_scopes (worktree_id, revision, scope_id)
             VALUES (?1, ?2, ?3)",
            (worktree_id, encode_revision(revision).as_slice(), scope_id),
        )
        .expect("active-scope insert should succeed");
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_mutation_event(
        db: &RepositoryAgentTraceDb,
        worktree_id: &str,
        revision: u64,
        before_tree: &str,
        after_tree: &str,
        attribution_kind: &str,
        attribution_scope_id: Option<&str>,
        boundary_kind: &str,
        boundary_scope_id: Option<&str>,
        boundary_event_id: Option<&str>,
        active_scopes: &[&str],
    ) {
        let revision_blob = encode_revision(revision);

        db.execute(
            "INSERT INTO mutation_trace_events
                (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
                 attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id)
             VALUES (?1, ?2, ?3, ?4, 0, 'healthy', ?5, ?6, ?7, ?8, ?9)",
            (
                worktree_id,
                revision_blob.as_slice(),
                before_tree,
                after_tree,
                attribution_kind,
                attribution_scope_id,
                boundary_kind,
                boundary_scope_id,
                boundary_event_id,
            ),
        )
        .expect("mutation event insert should succeed");

        for scope_id in active_scopes {
            db.execute(
                "INSERT INTO mutation_trace_event_active_scopes (worktree_id, revision, scope_id)
                 VALUES (?1, ?2, ?3)",
                (worktree_id, revision_blob.as_slice(), *scope_id),
            )
            .expect("active-scope insert should succeed");
        }
    }

    #[test]
    fn load_worktree_returns_none_for_a_missing_worktree() {
        let db_fixture = test_db_path("missing-worktree");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        let projection = store
            .load_worktree(&WorktreeId("wt-missing".to_string()), None, None)
            .expect("load_worktree should succeed");
        assert!(projection.is_none());
    }

    #[test]
    fn load_worktree_with_no_scope_or_event_key_loads_only_active_scopes() {
        let db_fixture = test_db_path("case-1-active-only");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 5);
        insert_scope(&db, "scope-active", "wt-1", ScopeStatus::Active);
        insert_scope(&db, "scope-closed", "wt-1", ScopeStatus::Closed);

        let projection = store
            .load_worktree(&WorktreeId("wt-1".to_string()), None, None)
            .expect("load_worktree should succeed")
            .expect("worktree should exist");

        assert_eq!(projection.worktree_id, WorktreeId("wt-1".to_string()));
        assert_eq!(projection.worktree_state.revision, 5);
        assert_eq!(
            projection.scopes.keys().collect::<Vec<_>>(),
            vec![&ScopeId("scope-active".to_string())]
        );
        assert!(projection.processed_events.is_empty());
    }

    #[test]
    fn load_worktree_with_explicit_scope_includes_it_regardless_of_status() {
        let db_fixture = test_db_path("case-2-explicit-scope");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 0);
        insert_scope(&db, "scope-closed", "wt-1", ScopeStatus::Closed);

        let projection = store
            .load_worktree(
                &WorktreeId("wt-1".to_string()),
                Some(&ScopeId("scope-closed".to_string())),
                None,
            )
            .expect("load_worktree should succeed")
            .expect("worktree should exist");

        assert_eq!(
            projection.scopes.get(&ScopeId("scope-closed".to_string())),
            Some(&ScopeState {
                status: ScopeStatus::Closed,
                actor_kind: ActorKind::ClaudeCode,
                worktree_id: WorktreeId("wt-1".to_string()),
            })
        );
    }

    #[test]
    fn load_worktree_with_explicit_scope_on_another_worktree_errors() {
        let db_fixture = test_db_path("case-2-wrong-worktree");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 0);
        insert_worktree(&db, "wt-2", 0);
        insert_scope(&db, "scope-1", "wt-2", ScopeStatus::Active);

        let error = store
            .load_worktree(
                &WorktreeId("wt-1".to_string()),
                Some(&ScopeId("scope-1".to_string())),
                None,
            )
            .expect_err("scope belonging to another worktree should error");
        assert!(error.to_string().contains("scope-1"));
    }

    #[test]
    fn load_worktree_with_explicit_missing_scope_errors() {
        let db_fixture = test_db_path("case-2-missing-scope");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 0);

        let error = store
            .load_worktree(
                &WorktreeId("wt-1".to_string()),
                Some(&ScopeId("scope-missing".to_string())),
                None,
            )
            .expect_err("missing effective scope should error");
        assert!(error.to_string().contains("scope-missing"));
    }

    #[test]
    fn load_worktree_with_only_event_key_loads_its_scope_and_replay_row() {
        let db_fixture = test_db_path("case-3-event-key-only");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 0);
        insert_scope(&db, "scope-1", "wt-1", ScopeStatus::NeverSeen);
        insert_processed_event(&db, "scope-1", "event-1");

        let event_key = EventKey {
            scope_id: ScopeId("scope-1".to_string()),
            event_id: EventId("event-1".to_string()),
        };

        let projection = store
            .load_worktree(&WorktreeId("wt-1".to_string()), None, Some(&event_key))
            .expect("load_worktree should succeed")
            .expect("worktree should exist");

        assert_eq!(
            projection
                .scopes
                .get(&ScopeId("scope-1".to_string()))
                .map(|s| s.status),
            Some(ScopeStatus::NeverSeen)
        );
        assert_eq!(
            projection.processed_events,
            [event_key].into_iter().collect()
        );
    }

    #[test]
    fn load_worktree_with_event_key_scope_on_another_worktree_errors() {
        let db_fixture = test_db_path("case-3-wrong-worktree");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 0);
        insert_worktree(&db, "wt-2", 0);
        insert_scope(&db, "scope-1", "wt-2", ScopeStatus::Active);

        let event_key = EventKey {
            scope_id: ScopeId("scope-1".to_string()),
            event_id: EventId("event-1".to_string()),
        };

        let error = store
            .load_worktree(&WorktreeId("wt-1".to_string()), None, Some(&event_key))
            .expect_err("event_key scope on another worktree should error");
        assert!(error.to_string().contains("scope-1"));
    }

    #[test]
    fn load_worktree_with_event_key_missing_scope_errors() {
        let db_fixture = test_db_path("case-3-missing-scope");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 0);

        let event_key = EventKey {
            scope_id: ScopeId("scope-missing".to_string()),
            event_id: EventId("event-1".to_string()),
        };

        let error = store
            .load_worktree(&WorktreeId("wt-1".to_string()), None, Some(&event_key))
            .expect_err("missing event_key.scope_id should error");
        assert!(error.to_string().contains("scope-missing"));
    }

    #[test]
    fn load_worktree_with_event_key_missing_scope_and_orphan_replay_row_errors() {
        let db_fixture = test_db_path("case-3-orphan-replay-row");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 0);
        insert_processed_event(&db, "scope-missing", "event-1");

        let event_key = EventKey {
            scope_id: ScopeId("scope-missing".to_string()),
            event_id: EventId("event-1".to_string()),
        };

        let error = store
            .load_worktree(&WorktreeId("wt-1".to_string()), None, Some(&event_key))
            .expect_err(
                "an orphan processed-event row must not let a missing scope produce a projection",
            );
        assert!(error.to_string().contains("scope-missing"));
    }

    #[test]
    fn load_worktree_with_agreeing_scope_and_event_key_loads_it_once() {
        let db_fixture = test_db_path("case-4-agreeing");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 0);
        insert_scope(&db, "scope-1", "wt-1", ScopeStatus::Active);

        let event_key = EventKey {
            scope_id: ScopeId("scope-1".to_string()),
            event_id: EventId("event-1".to_string()),
        };

        let projection = store
            .load_worktree(
                &WorktreeId("wt-1".to_string()),
                Some(&ScopeId("scope-1".to_string())),
                Some(&event_key),
            )
            .expect("load_worktree should succeed")
            .expect("worktree should exist");

        assert_eq!(projection.scopes.len(), 1);
        assert!(projection
            .scopes
            .contains_key(&ScopeId("scope-1".to_string())));
    }

    #[test]
    fn load_worktree_with_disagreeing_scope_and_event_key_errors_without_loading() {
        let db_fixture = test_db_path("case-5-disagreeing");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 0);
        insert_scope(&db, "scope-a", "wt-1", ScopeStatus::Active);
        insert_scope(&db, "scope-b", "wt-1", ScopeStatus::Active);

        let event_key = EventKey {
            scope_id: ScopeId("scope-b".to_string()),
            event_id: EventId("event-1".to_string()),
        };

        let error = store
            .load_worktree(
                &WorktreeId("wt-1".to_string()),
                Some(&ScopeId("scope-a".to_string())),
                Some(&event_key),
            )
            .expect_err("disagreeing scope/event_key.scope_id should error");
        assert!(error.to_string().contains("scope-a"));
        assert!(error.to_string().contains("scope-b"));
    }

    #[test]
    fn load_scope_returns_the_durable_state_for_a_known_scope() {
        let db_fixture = test_db_path("load-scope-known");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 7);
        insert_scope(&db, "scope-1", "wt-1", ScopeStatus::Closed);
        insert_mutation_event(
            &db,
            "wt-1",
            7,
            "tree-0",
            "tree-1",
            "ai_exclusive",
            Some("scope-1"),
            "close",
            Some("scope-1"),
            Some("event-1"),
            &["scope-1"],
        );
        insert_processed_event(&db, "scope-1", "event-1");

        let scope_state = store
            .load_scope(&ScopeId("scope-1".to_string()))
            .expect("load_scope should succeed")
            .expect("known scope should be present");

        assert_eq!(
            scope_state,
            ScopeState {
                status: ScopeStatus::Closed,
                actor_kind: ActorKind::ClaudeCode,
                worktree_id: WorktreeId("wt-1".to_string()),
            }
        );
    }

    #[test]
    fn load_scope_returns_none_for_an_unknown_scope() {
        let db_fixture = test_db_path("load-scope-unknown");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 0);
        insert_scope(&db, "scope-1", "wt-1", ScopeStatus::Active);

        let scope_state = store
            .load_scope(&ScopeId("scope-missing".to_string()))
            .expect("load_scope should succeed for an unknown scope");
        assert!(scope_state.is_none());
    }

    #[test]
    fn load_scope_returns_a_scope_belonging_to_another_worktree() {
        let db_fixture = test_db_path("load-scope-other-worktree");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 0);
        insert_scope(&db, "scope-other", "wt-2", ScopeStatus::Active);

        let scope_state = store
            .load_scope(&ScopeId("scope-other".to_string()))
            .expect("load_scope should not reject a scope on another worktree")
            .expect("the scope row should be returned as-is");

        assert_eq!(
            scope_state,
            ScopeState {
                status: ScopeStatus::Active,
                actor_kind: ActorKind::ClaudeCode,
                worktree_id: WorktreeId("wt-2".to_string()),
            }
        );

        let error = store
            .load_worktree(
                &WorktreeId("wt-1".to_string()),
                Some(&ScopeId("scope-other".to_string())),
                None,
            )
            .expect_err("load_worktree should still reject the cross-worktree scope");
        assert!(error.to_string().contains("scope-other"));
    }

    #[test]
    fn load_mutation_event_returns_none_when_missing() {
        let db_fixture = test_db_path("cold-path-missing");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        let event = store
            .load_mutation_event(&WorktreeId("wt-1".to_string()), 1)
            .expect("load_mutation_event should succeed");
        assert!(event.is_none());
    }

    #[test]
    fn load_mutation_event_reconstructs_ai_exclusive_start_event() {
        let db_fixture = test_db_path("cold-path-ai-exclusive-start");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_mutation_event(
            &db,
            "wt-1",
            1,
            "tree-0",
            "tree-1",
            "ai_exclusive",
            Some("scope-1"),
            "start",
            Some("scope-1"),
            Some("event-1"),
            &["scope-1"],
        );

        let event = store
            .load_mutation_event(&WorktreeId("wt-1".to_string()), 1)
            .expect("load_mutation_event should succeed")
            .expect("mutation event row should exist");

        assert_eq!(
            event,
            MutationEvent {
                worktree_id: WorktreeId("wt-1".to_string()),
                revision: 1,
                before_tree: TreeId("tree-0".to_string()),
                after_tree: TreeId("tree-1".to_string()),
                active_scopes: [ScopeId("scope-1".to_string())].into_iter().collect(),
                tainted: false,
                failure_kind: FailureKind::Healthy,
                attribution: Attribution::AiExclusive(ScopeId("scope-1".to_string())),
                boundary: Boundary::Start {
                    scope: ScopeId("scope-1".to_string()),
                    event: EventId("event-1".to_string()),
                },
            }
        );
    }

    #[test]
    fn load_mutation_event_reconstructs_a_flush_event_with_multiple_active_scopes() {
        let db_fixture = test_db_path("cold-path-flush");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_mutation_event(
            &db,
            "wt-1",
            3,
            "tree-2",
            "tree-3",
            "ai_contended",
            None,
            "flush",
            None,
            None,
            &["scope-1", "scope-2"],
        );

        let event = store
            .load_mutation_event(&WorktreeId("wt-1".to_string()), 3)
            .expect("load_mutation_event should succeed")
            .expect("mutation event row should exist");

        assert_eq!(
            event,
            MutationEvent {
                worktree_id: WorktreeId("wt-1".to_string()),
                revision: 3,
                before_tree: TreeId("tree-2".to_string()),
                after_tree: TreeId("tree-3".to_string()),
                active_scopes: [
                    ScopeId("scope-1".to_string()),
                    ScopeId("scope-2".to_string())
                ]
                .into_iter()
                .collect(),
                tainted: false,
                failure_kind: FailureKind::Healthy,
                attribution: Attribution::AiContended,
                boundary: Boundary::Flush {
                    worktree: WorktreeId("wt-1".to_string()),
                },
            }
        );
    }

    #[test]
    fn into_protocol_state_carries_only_the_loaded_worktree_and_leaves_transient_fields_empty() {
        let db_fixture = test_db_path("into-protocol-state");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 7);
        insert_scope(&db, "scope-1", "wt-1", ScopeStatus::Active);

        let projection = store
            .load_worktree(&WorktreeId("wt-1".to_string()), None, None)
            .expect("load_worktree should succeed")
            .expect("worktree should exist");

        let protocol_state = projection.into_protocol_state();

        assert_eq!(protocol_state.worktrees.len(), 1);
        assert_eq!(
            protocol_state
                .worktrees
                .get(&WorktreeId("wt-1".to_string()))
                .map(|w| w.revision),
            Some(7)
        );
        assert!(protocol_state.attempts.is_empty());
        assert!(protocol_state.mutation_events.is_empty());
        assert!(protocol_state.external_taint.is_empty());
    }

    #[test]
    fn initialize_worktree_inserts_a_fresh_healthy_cursor() {
        let db_fixture = test_db_path("init-worktree-fresh");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        store
            .initialize_worktree(
                &WorktreeId("wt-1".to_string()),
                &TreeId("tree-0".to_string()),
            )
            .expect("initialize_worktree should succeed");

        let projection = store
            .load_worktree(&WorktreeId("wt-1".to_string()), None, None)
            .expect("load_worktree should succeed")
            .expect("worktree should exist");

        assert_eq!(
            projection.worktree_state,
            WorktreeState {
                cursor_tree: TreeId("tree-0".to_string()),
                revision: 0,
                tainted: false,
                failure_kind: FailureKind::Healthy,
                needs_rebaseline: false,
            }
        );
    }

    #[test]
    fn initialize_worktree_never_overwrites_an_existing_cursor() {
        let db_fixture = test_db_path("init-worktree-idempotent");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 5);

        store
            .initialize_worktree(
                &WorktreeId("wt-1".to_string()),
                &TreeId("tree-new".to_string()),
            )
            .expect("initialize_worktree should succeed as a no-op");

        let projection = store
            .load_worktree(&WorktreeId("wt-1".to_string()), None, None)
            .expect("load_worktree should succeed")
            .expect("worktree should exist");

        assert_eq!(
            projection.worktree_state.cursor_tree,
            TreeId("tree-0".to_string())
        );
        assert_eq!(projection.worktree_state.revision, 5);
    }

    #[test]
    fn register_scope_inserts_never_seen_when_missing() {
        let db_fixture = test_db_path("register-scope-fresh");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 0);

        let scope_state = store
            .register_scope(
                &ScopeId("scope-1".to_string()),
                &WorktreeId("wt-1".to_string()),
                ActorKind::ClaudeCode,
            )
            .expect("register_scope should succeed");

        assert_eq!(
            scope_state,
            ScopeState {
                status: ScopeStatus::NeverSeen,
                actor_kind: ActorKind::ClaudeCode,
                worktree_id: WorktreeId("wt-1".to_string()),
            }
        );
    }

    #[test]
    fn register_scope_returns_existing_state_when_worktree_and_actor_match() {
        let db_fixture = test_db_path("register-scope-existing-match");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 0);
        insert_scope(&db, "scope-1", "wt-1", ScopeStatus::Active);

        let scope_state = store
            .register_scope(
                &ScopeId("scope-1".to_string()),
                &WorktreeId("wt-1".to_string()),
                ActorKind::ClaudeCode,
            )
            .expect("register_scope should succeed for a matching existing scope");

        assert_eq!(
            scope_state,
            ScopeState {
                status: ScopeStatus::Active,
                actor_kind: ActorKind::ClaudeCode,
                worktree_id: WorktreeId("wt-1".to_string()),
            }
        );
    }

    #[test]
    fn register_scope_errors_on_worktree_mismatch() {
        let db_fixture = test_db_path("register-scope-worktree-mismatch");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 0);
        insert_worktree(&db, "wt-2", 0);
        insert_scope(&db, "scope-1", "wt-1", ScopeStatus::Active);

        let error = store
            .register_scope(
                &ScopeId("scope-1".to_string()),
                &WorktreeId("wt-2".to_string()),
                ActorKind::ClaudeCode,
            )
            .expect_err("a worktree mismatch on an existing scope should error");
        assert!(error.to_string().contains("scope-1"));
    }

    #[test]
    fn register_scope_errors_on_actor_mismatch() {
        let db_fixture = test_db_path("register-scope-actor-mismatch");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree(&db, "wt-1", 0);
        insert_scope(&db, "scope-1", "wt-1", ScopeStatus::Active);

        let error = store
            .register_scope(
                &ScopeId("scope-1".to_string()),
                &WorktreeId("wt-1".to_string()),
                ActorKind::Codex,
            )
            .expect_err("an actor mismatch on an existing scope should error");
        assert!(error.to_string().contains("scope-1"));
    }

    #[test]
    fn register_scope_errors_when_worktree_does_not_exist_and_leaves_no_scope_row() {
        let db_fixture = test_db_path("register-scope-missing-worktree-fresh");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        let error = store
            .register_scope(
                &ScopeId("scope-1".to_string()),
                &WorktreeId("wt-missing".to_string()),
                ActorKind::ClaudeCode,
            )
            .expect_err("registering a scope against a missing worktree should error");
        assert!(error.to_string().contains("scope-1"));
        assert!(error.to_string().contains("wt-missing"));

        let scope_state = store
            .load_scope(&ScopeId("scope-1".to_string()))
            .expect("load_scope should succeed");
        assert!(
            scope_state.is_none(),
            "a failed register_scope must not leave an orphan scope row"
        );
    }

    #[test]
    fn register_scope_errors_when_existing_scopes_worktree_row_is_missing() {
        let db_fixture = test_db_path("register-scope-missing-worktree-existing");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_scope(&db, "scope-1", "wt-missing", ScopeStatus::Active);

        let error = store
            .register_scope(
                &ScopeId("scope-1".to_string()),
                &WorktreeId("wt-missing".to_string()),
                ActorKind::ClaudeCode,
            )
            .expect_err(
                "an existing scope whose worktree row is missing must not be accepted as valid",
            );
        assert!(error.to_string().contains("scope-1"));
        assert!(error.to_string().contains("wt-missing"));
    }

    fn healthy_worktree_state(revision: u64) -> WorktreeState {
        WorktreeState {
            cursor_tree: TreeId("tree0".to_string()),
            revision,
            tainted: false,
            failure_kind: FailureKind::Healthy,
            needs_rebaseline: false,
        }
    }

    fn state_with_scope(
        worktree_id: &WorktreeId,
        scope_id: &ScopeId,
        actor_kind: ActorKind,
        status: ScopeStatus,
        revision: u64,
    ) -> ProtocolState {
        let mut state = ProtocolState::default();
        state
            .worktrees
            .insert(worktree_id.clone(), healthy_worktree_state(revision));
        state.scopes.insert(
            scope_id.clone(),
            ScopeState {
                status,
                actor_kind,
                worktree_id: worktree_id.clone(),
            },
        );
        state
    }

    fn sample_mutation_event(worktree_id: &WorktreeId) -> MutationEvent {
        MutationEvent {
            worktree_id: worktree_id.clone(),
            revision: 1,
            before_tree: TreeId("tree0".to_string()),
            after_tree: TreeId("tree1".to_string()),
            active_scopes: BTreeSet::new(),
            tainted: false,
            failure_kind: FailureKind::Healthy,
            attribution: Attribution::IneligibleUnscoped,
            boundary: Boundary::Flush {
                worktree: worktree_id.clone(),
            },
        }
    }

    #[test]
    fn between_returns_none_for_a_database_failure_only_transition() {
        let wt = WorktreeId("wt0".to_string());
        let mut before = ProtocolState::default();
        before
            .worktrees
            .insert(wt.clone(), healthy_worktree_state(0));

        let after = database_failure(&before, &wt);

        assert_eq!(
            DurableTransition::between(&before, &after, &wt).unwrap(),
            None
        );
    }

    #[test]
    fn between_returns_none_for_a_no_change_flush() {
        let wt = WorktreeId("wt0".to_string());
        let mut before = ProtocolState::default();
        before
            .worktrees
            .insert(wt.clone(), healthy_worktree_state(0));

        let attempt = AttemptId("attempt0".to_string());
        let prepared = prepare(
            &before,
            attempt.clone(),
            Boundary::Flush {
                worktree: wt.clone(),
            },
            TreeId("tree0".to_string()),
        );
        let after = commit(&prepared, &attempt).state;

        assert_eq!(
            DurableTransition::between(&before, &after, &wt).unwrap(),
            None
        );
    }

    #[test]
    fn between_returns_some_with_correct_shape_for_a_start_transition() {
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        let before = state_with_scope(&wt, &scope_id, ActorKind::Codex, ScopeStatus::NeverSeen, 0);

        let attempt = AttemptId("attempt0".to_string());
        let event_id = EventId("event0".to_string());
        let prepared = prepare(
            &before,
            attempt.clone(),
            Boundary::Start {
                scope: scope_id.clone(),
                event: event_id.clone(),
            },
            TreeId("tree1".to_string()),
        );
        let after = commit(&prepared, &attempt).state;

        let transition = DurableTransition::between(&before, &after, &wt)
            .unwrap()
            .expect("a start transition should produce a durable transition");

        assert_eq!(transition.worktree, wt);
        assert_eq!(transition.expected_revision, 0);
        assert_eq!(transition.next_worktree_state.revision, 1);
        assert_eq!(
            transition.next_worktree_state.cursor_tree,
            TreeId("tree1".to_string())
        );
        assert_eq!(
            transition.scope_status_changes.get(&scope_id),
            Some(&ScopeStatus::Active)
        );
        assert_eq!(
            transition.new_processed_event,
            Some(EventKey {
                scope_id: scope_id.clone(),
                event_id,
            })
        );
        assert_eq!(after.mutation_events.len(), 1);
        assert_eq!(
            transition.new_mutation_event.as_ref(),
            after.mutation_events.iter().next()
        );
    }

    #[test]
    fn between_returns_some_with_correct_shape_for_an_advance_transition() {
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        let before = state_with_scope(&wt, &scope_id, ActorKind::Codex, ScopeStatus::Active, 0);

        let attempt = AttemptId("attempt0".to_string());
        let event_id = EventId("event0".to_string());
        let prepared = prepare(
            &before,
            attempt.clone(),
            Boundary::Advance {
                scope: scope_id.clone(),
                event: event_id.clone(),
            },
            TreeId("tree1".to_string()),
        );
        let after = commit(&prepared, &attempt).state;

        let transition = DurableTransition::between(&before, &after, &wt)
            .unwrap()
            .expect("an advance transition should produce a durable transition");

        assert!(transition.scope_status_changes.is_empty());
        assert_eq!(
            transition.new_processed_event,
            Some(EventKey { scope_id, event_id })
        );
        assert_eq!(after.mutation_events.len(), 1);
        assert_eq!(transition.next_worktree_state.revision, 1);
    }

    #[test]
    fn between_returns_some_with_correct_shape_for_a_close_transition() {
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        let before = state_with_scope(&wt, &scope_id, ActorKind::Codex, ScopeStatus::Active, 0);

        let attempt = AttemptId("attempt0".to_string());
        let event_id = EventId("event0".to_string());
        let prepared = prepare(
            &before,
            attempt.clone(),
            Boundary::Close {
                scope: scope_id.clone(),
                event: event_id.clone(),
            },
            TreeId("tree0".to_string()),
        );
        let after = commit(&prepared, &attempt).state;

        let transition = DurableTransition::between(&before, &after, &wt)
            .unwrap()
            .expect("a close transition should produce a durable transition");

        assert_eq!(
            transition.scope_status_changes.get(&scope_id),
            Some(&ScopeStatus::Closed)
        );
        assert_eq!(
            transition.new_processed_event,
            Some(EventKey { scope_id, event_id })
        );
        assert_eq!(transition.next_worktree_state.revision, 1);
    }

    #[test]
    fn between_returns_some_with_correct_shape_for_a_taint_transition() {
        let wt = WorktreeId("wt0".to_string());
        let mut before = ProtocolState::default();
        before
            .worktrees
            .insert(wt.clone(), healthy_worktree_state(0));

        let after = taint(&before, &wt);

        let transition = DurableTransition::between(&before, &after, &wt)
            .unwrap()
            .expect("a taint transition should produce a durable transition");

        assert!(transition.scope_status_changes.is_empty());
        assert!(transition.new_processed_event.is_none());
        assert!(transition.new_mutation_event.is_none());
        assert!(transition.next_worktree_state.tainted);
        assert_eq!(
            transition.next_worktree_state.failure_kind,
            FailureKind::SnapshotFailure
        );
        assert_eq!(transition.next_worktree_state.revision, 1);
    }

    #[test]
    fn between_returns_some_with_correct_shape_for_an_abandon_transition() {
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        let before = state_with_scope(&wt, &scope_id, ActorKind::Codex, ScopeStatus::Active, 0);

        let after = abandon(&before, &scope_id);

        let transition = DurableTransition::between(&before, &after, &wt)
            .unwrap()
            .expect("an abandon transition should produce a durable transition");

        assert_eq!(
            transition.scope_status_changes.get(&scope_id),
            Some(&ScopeStatus::Abandoned)
        );
        assert!(transition.next_worktree_state.needs_rebaseline);
        assert_eq!(transition.next_worktree_state.revision, 1);
        assert!(transition.new_processed_event.is_none());
        assert!(transition.new_mutation_event.is_none());
    }

    #[test]
    fn between_returns_some_with_correct_shape_for_a_recover_transition() {
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        let mut before = ProtocolState::default();
        before.worktrees.insert(
            wt.clone(),
            WorktreeState {
                cursor_tree: TreeId("tree0".to_string()),
                revision: 0,
                tainted: true,
                failure_kind: FailureKind::SnapshotFailure,
                needs_rebaseline: false,
            },
        );
        before.scopes.insert(
            scope_id.clone(),
            ScopeState {
                status: ScopeStatus::Active,
                actor_kind: ActorKind::Codex,
                worktree_id: wt.clone(),
            },
        );

        let after = recover(&before, &wt, TreeId("tree1".to_string()));

        let transition = DurableTransition::between(&before, &after, &wt)
            .unwrap()
            .expect("a recover transition should produce a durable transition");

        assert_eq!(
            transition.scope_status_changes.get(&scope_id),
            Some(&ScopeStatus::Abandoned)
        );
        assert!(!transition.next_worktree_state.tainted);
        assert_eq!(
            transition.next_worktree_state.failure_kind,
            FailureKind::Healthy
        );
        assert_eq!(
            transition.next_worktree_state.cursor_tree,
            TreeId("tree1".to_string())
        );
        assert_eq!(transition.next_worktree_state.revision, 1);
    }

    #[test]
    fn between_errors_when_a_scopes_actor_kind_changes() {
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        let before = state_with_scope(&wt, &scope_id, ActorKind::Codex, ScopeStatus::Active, 0);
        let mut after = before.clone();
        after.scopes.get_mut(&scope_id).unwrap().actor_kind = ActorKind::ClaudeCode;

        let error = DurableTransition::between(&before, &after, &wt)
            .expect_err("an actor_kind change must be rejected");
        assert!(error.to_string().contains("actor_kind"));
    }

    #[test]
    fn between_errors_when_a_scopes_worktree_id_changes() {
        let wt = WorktreeId("wt0".to_string());
        let other_wt = WorktreeId("wt1".to_string());
        let scope_id = ScopeId("scope0".to_string());
        let before = state_with_scope(&wt, &scope_id, ActorKind::Codex, ScopeStatus::Active, 0);
        let mut after = before.clone();
        after.scopes.get_mut(&scope_id).unwrap().worktree_id = other_wt;

        let error = DurableTransition::between(&before, &after, &wt)
            .expect_err("a scope worktree_id change must be rejected");
        assert!(error.to_string().contains("worktree_id"));
    }

    #[test]
    fn between_errors_when_a_processed_event_disappears() {
        let wt = WorktreeId("wt0".to_string());
        let mut before = ProtocolState::default();
        before
            .worktrees
            .insert(wt.clone(), healthy_worktree_state(0));
        before.processed_events.insert(EventKey {
            scope_id: ScopeId("scope0".to_string()),
            event_id: EventId("event0".to_string()),
        });
        let mut after = before.clone();
        after.processed_events.clear();

        let error = DurableTransition::between(&before, &after, &wt)
            .expect_err("a disappearing processed event must be rejected");
        assert!(error.to_string().contains("processed_events"));
    }

    #[test]
    fn between_errors_when_a_mutation_event_disappears() {
        let wt = WorktreeId("wt0".to_string());
        let mut before = ProtocolState::default();
        before
            .worktrees
            .insert(wt.clone(), healthy_worktree_state(0));
        before.mutation_events.insert(sample_mutation_event(&wt));
        let mut after = before.clone();
        after.mutation_events.clear();

        let error = DurableTransition::between(&before, &after, &wt)
            .expect_err("a disappearing mutation event must be rejected");
        assert!(error.to_string().contains("mutation_events"));
    }

    #[test]
    fn between_errors_when_a_new_mutation_events_revision_does_not_match_the_next_worktree_revision(
    ) {
        let wt = WorktreeId("wt0".to_string());
        let mut before = ProtocolState::default();
        before
            .worktrees
            .insert(wt.clone(), healthy_worktree_state(0));
        let mut after = before.clone();
        after.worktrees.get_mut(&wt).unwrap().revision = 1;
        let mut mismatched_event = sample_mutation_event(&wt);
        mismatched_event.revision = 2;
        after.mutation_events.insert(mismatched_event);

        let error = DurableTransition::between(&before, &after, &wt).expect_err(
            "a mutation event revision mismatched with the next worktree revision must be rejected",
        );
        assert!(error.to_string().contains("revision"));
    }

    #[test]
    fn between_errors_when_an_unrelated_worktree_changes() {
        let wt = WorktreeId("wt0".to_string());
        let other_wt = WorktreeId("wt1".to_string());
        let mut before = ProtocolState::default();
        before
            .worktrees
            .insert(wt.clone(), healthy_worktree_state(0));
        before
            .worktrees
            .insert(other_wt.clone(), healthy_worktree_state(0));

        let mut after = before.clone();
        after.worktrees.get_mut(&other_wt).unwrap().revision = 1;

        let error = DurableTransition::between(&before, &after, &wt)
            .expect_err("an unrelated worktree change must be rejected");
        assert!(error.to_string().contains("wt1"));
    }

    #[test]
    fn between_errors_when_the_revision_jumps_by_more_than_one() {
        let wt = WorktreeId("wt0".to_string());
        let mut before = ProtocolState::default();
        before
            .worktrees
            .insert(wt.clone(), healthy_worktree_state(0));
        let mut after = before.clone();
        after.worktrees.get_mut(&wt).unwrap().revision = 2;

        let error = DurableTransition::between(&before, &after, &wt)
            .expect_err("a revision jump of more than one must be rejected");
        assert!(error.to_string().contains("revision"));
    }

    #[test]
    fn between_errors_when_the_revision_decreases() {
        let wt = WorktreeId("wt0".to_string());
        let mut before = ProtocolState::default();
        before
            .worktrees
            .insert(wt.clone(), healthy_worktree_state(5));
        let mut after = before.clone();
        after.worktrees.get_mut(&wt).unwrap().revision = 4;

        let error = DurableTransition::between(&before, &after, &wt)
            .expect_err("a revision decrease must be rejected");
        assert!(error.to_string().contains("revision"));
    }

    #[test]
    fn between_errors_when_a_scope_unexpectedly_appears() {
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        let mut before = ProtocolState::default();
        before
            .worktrees
            .insert(wt.clone(), healthy_worktree_state(0));
        let mut after = before.clone();
        after.scopes.insert(
            scope_id,
            ScopeState {
                status: ScopeStatus::NeverSeen,
                actor_kind: ActorKind::Codex,
                worktree_id: wt.clone(),
            },
        );

        let error = DurableTransition::between(&before, &after, &wt)
            .expect_err("an unexpectedly appearing scope must be rejected");
        assert!(error.to_string().contains("scope set"));
    }

    #[test]
    fn between_errors_when_a_scope_unexpectedly_disappears() {
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        let before = state_with_scope(&wt, &scope_id, ActorKind::Codex, ScopeStatus::Active, 0);
        let mut after = before.clone();
        after.scopes.remove(&scope_id);

        let error = DurableTransition::between(&before, &after, &wt)
            .expect_err("an unexpectedly disappearing scope must be rejected");
        assert!(error.to_string().contains("scope set"));
    }

    #[test]
    fn commit_applies_a_full_transition_and_makes_every_write_visible() {
        let db_fixture = test_db_path("commit-applies-full-transition");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        insert_worktree(&db, &wt.0, 0);
        insert_scope(&db, &scope_id.0, &wt.0, ScopeStatus::Active);

        let event_key = EventKey {
            scope_id: scope_id.clone(),
            event_id: EventId("event-1".to_string()),
        };
        let mutation_event = MutationEvent {
            worktree_id: wt.clone(),
            revision: 1,
            before_tree: TreeId("tree0".to_string()),
            after_tree: TreeId("tree1".to_string()),
            active_scopes: BTreeSet::from([scope_id.clone()]),
            tainted: false,
            failure_kind: FailureKind::Healthy,
            attribution: Attribution::AiExclusive(scope_id.clone()),
            boundary: Boundary::Close {
                scope: scope_id.clone(),
                event: EventId("event-1".to_string()),
            },
        };

        let before = state_with_scope(
            &wt,
            &scope_id,
            ActorKind::ClaudeCode,
            ScopeStatus::Active,
            0,
        );
        let mut after = before.clone();
        after.worktrees.insert(
            wt.clone(),
            WorktreeState {
                cursor_tree: TreeId("tree1".to_string()),
                revision: 1,
                tainted: false,
                failure_kind: FailureKind::Healthy,
                needs_rebaseline: false,
            },
        );
        after.scopes.insert(
            scope_id.clone(),
            ScopeState {
                status: ScopeStatus::Closed,
                actor_kind: ActorKind::ClaudeCode,
                worktree_id: wt.clone(),
            },
        );
        after.processed_events.insert(event_key.clone());
        after.mutation_events.insert(mutation_event.clone());

        let transition = DurableTransition::between(&before, &after, &wt)
            .expect("between should succeed")
            .expect("a transition should exist for this change");

        let result = store.commit(&transition).expect("commit should succeed");
        assert_eq!(result, CasResult::Applied);

        let worktree_state = store
            .load_worktree_state(&wt)
            .expect("worktree read should succeed")
            .expect("worktree row should exist");
        assert_eq!(worktree_state, transition.next_worktree_state);

        let scope_state = store
            .load_scope(&scope_id)
            .expect("scope read should succeed")
            .expect("scope row should exist");
        assert_eq!(scope_state.status, ScopeStatus::Closed);

        assert!(store
            .processed_event_exists(&event_key)
            .expect("processed-event read should succeed"));

        let reloaded_event = store
            .load_mutation_event(&wt, 1)
            .expect("mutation-event read should succeed")
            .expect("mutation-event row should exist");
        assert_eq!(reloaded_event, mutation_event);
    }

    #[test]
    fn commit_returns_conflict_and_writes_nothing_when_the_worktree_revision_has_moved_on() {
        let db_fixture = test_db_path("commit-conflict");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);
        let wt = WorktreeId("wt0".to_string());
        insert_worktree(&db, &wt.0, 5);

        let mut before = ProtocolState::default();
        before
            .worktrees
            .insert(wt.clone(), healthy_worktree_state(0));
        let mut after = before.clone();
        after.worktrees.get_mut(&wt).unwrap().revision = 1;

        let transition = DurableTransition::between(&before, &after, &wt)
            .expect("between should succeed")
            .expect("a transition should exist for this change");

        let result = store.commit(&transition).expect("commit should succeed");
        assert_eq!(result, CasResult::Conflict);

        let worktree_state = store
            .load_worktree_state(&wt)
            .expect("worktree read should succeed")
            .expect("worktree row should exist");
        assert_eq!(worktree_state.revision, 5);
    }

    #[test]
    fn commit_propagates_a_deterministic_failure_without_reporting_conflict() {
        let db_fixture = test_db_path("commit-deterministic-failure");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        insert_worktree(&db, &wt.0, 0);
        insert_processed_event(&db, "scope0", "event-1");

        let before = state_with_scope(
            &wt,
            &scope_id,
            ActorKind::ClaudeCode,
            ScopeStatus::Active,
            0,
        );
        let mut after = before.clone();
        after.worktrees.get_mut(&wt).unwrap().revision = 1;
        after.processed_events.insert(EventKey {
            scope_id: scope_id.clone(),
            event_id: EventId("event-1".to_string()),
        });

        let transition = DurableTransition::between(&before, &after, &wt)
            .expect("between should succeed")
            .expect("a transition should exist for this change");

        let error = store
            .commit(&transition)
            .expect_err("a duplicate processed-event insert should fail deterministically");
        assert!(error.to_string().contains("execute failed"));

        let worktree_state = store
            .load_worktree_state(&wt)
            .expect("worktree read should succeed")
            .expect("worktree row should exist");
        assert_eq!(
            worktree_state.revision, 0,
            "the guard's own revision advance must roll back together with the failed insert"
        );
    }

    struct RaceEvidence {
        scope: ScopeId,
        event_key: EventKey,
        mutation_event: MutationEvent,
    }

    fn race_evidence(worktree: &WorktreeId, scope: ScopeId, label: &str) -> RaceEvidence {
        let event_key = EventKey {
            scope_id: scope.clone(),
            event_id: EventId(format!("event-{label}")),
        };
        let mutation_event = MutationEvent {
            worktree_id: worktree.clone(),
            revision: 1,
            before_tree: TreeId("tree0".to_string()),
            after_tree: TreeId(format!("tree-after-{label}")),
            active_scopes: BTreeSet::from([scope.clone()]),
            tainted: false,
            failure_kind: FailureKind::Healthy,
            attribution: Attribution::AiExclusive(scope.clone()),
            boundary: Boundary::Close {
                scope: scope.clone(),
                event: event_key.event_id.clone(),
            },
        };
        RaceEvidence {
            scope,
            event_key,
            mutation_event,
        }
    }

    fn closing_transition(
        before: &ProtocolState,
        worktree: &WorktreeId,
        evidence: &RaceEvidence,
    ) -> DurableTransition {
        let mut after = before.clone();
        after.worktrees.get_mut(worktree).unwrap().revision = evidence.mutation_event.revision;
        after.scopes.get_mut(&evidence.scope).unwrap().status = ScopeStatus::Closed;
        after.processed_events.insert(evidence.event_key.clone());
        after
            .mutation_events
            .insert(evidence.mutation_event.clone());
        DurableTransition::between(before, &after, worktree)
            .expect("between should succeed")
            .expect("a transition should exist for this change")
    }

    fn assert_race_winner_state(
        store: &MutationTraceStore,
        worktree: &WorktreeId,
        persisted_event: &MutationEvent,
        writer_a: &RaceEvidence,
        writer_b: &RaceEvidence,
    ) {
        let (winner, loser) = if persisted_event == &writer_a.mutation_event {
            (writer_a, writer_b)
        } else if persisted_event == &writer_b.mutation_event {
            (writer_b, writer_a)
        } else {
            panic!(
                "persisted mutation event matches neither writer's expected event: \
                 {persisted_event:?}"
            );
        };

        let winning_scope_state = store
            .load_scope(&winner.scope)
            .expect("winning scope read should succeed")
            .expect("winning scope row should exist");
        assert_eq!(
            winning_scope_state.status,
            ScopeStatus::Closed,
            "the winning transition's scope-status change must be durable"
        );

        let losing_scope_state = store
            .load_scope(&loser.scope)
            .expect("losing scope read should succeed")
            .expect("losing scope row should exist");
        assert_eq!(
            losing_scope_state.status,
            ScopeStatus::Active,
            "the losing transition's scope-status change must not have applied"
        );

        assert!(
            store
                .processed_event_exists(&winner.event_key)
                .expect("winning processed-event read should succeed"),
            "the winning transition's processed EventKey must exist"
        );
        assert!(
            !store
                .processed_event_exists(&loser.event_key)
                .expect("losing processed-event read should succeed"),
            "the losing transition's processed EventKey must not exist"
        );

        assert_eq!(
            persisted_event, &winner.mutation_event,
            "the persisted mutation event must equal the winning transition's expected event \
             field-for-field, with no evidence from the losing transition mixed in"
        );

        let persisted_active_scopes = store
            .load_mutation_event_active_scopes(
                worktree,
                encode_revision(winner.mutation_event.revision).as_slice(),
            )
            .expect("active-scope read should succeed");
        assert_eq!(
            &persisted_active_scopes, &winner.mutation_event.active_scopes,
            "persisted active scopes must equal exactly the winning transition's active_scopes, \
             with no loser-only active-scope rows present"
        );
    }

    #[test]
    fn commit_from_two_independent_connections_races_and_only_one_applies() {
        let db_fixture = test_db_path("commit-two-writer-race");
        let db_path = db_fixture.path();
        let wt = WorktreeId("wt0".to_string());
        let scope_a = ScopeId("scope-a".to_string());
        let scope_b = ScopeId("scope-b".to_string());

        {
            let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
            insert_worktree(&db, &wt.0, 0);
            insert_scope(&db, &scope_a.0, &wt.0, ScopeStatus::Active);
            insert_scope(&db, &scope_b.0, &wt.0, ScopeStatus::Active);
        }

        let mut before = ProtocolState::default();
        before
            .worktrees
            .insert(wt.clone(), healthy_worktree_state(0));
        before.scopes.insert(
            scope_a.clone(),
            ScopeState {
                status: ScopeStatus::Active,
                actor_kind: ActorKind::ClaudeCode,
                worktree_id: wt.clone(),
            },
        );
        before.scopes.insert(
            scope_b.clone(),
            ScopeState {
                status: ScopeStatus::Active,
                actor_kind: ActorKind::ClaudeCode,
                worktree_id: wt.clone(),
            },
        );

        let writer_a = race_evidence(&wt, scope_a, "a");
        let writer_b = race_evidence(&wt, scope_b, "b");
        let transition_a = closing_transition(&before, &wt, &writer_a);
        let transition_b = closing_transition(&before, &wt, &writer_b);

        let db_a = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(db_path)
            .expect("writer A handle should open");
        let db_b = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(db_path)
            .expect("writer B handle should open");

        let handle_a = thread::spawn(move || MutationTraceStore::new(&db_a).commit(&transition_a));
        let handle_b = thread::spawn(move || MutationTraceStore::new(&db_b).commit(&transition_b));

        let result_a = handle_a
            .join()
            .expect("writer A thread should not panic")
            .expect("writer A commit should not error");
        let result_b = handle_b
            .join()
            .expect("writer B thread should not panic")
            .expect("writer B commit should not error");
        let results = [result_a, result_b];

        assert_eq!(
            results.iter().filter(|r| **r == CasResult::Applied).count(),
            1,
            "exactly one writer should apply from the same starting revision: {results:?}"
        );
        assert_eq!(
            results
                .iter()
                .filter(|r| **r == CasResult::Conflict)
                .count(),
            1,
            "exactly one writer should conflict from the same starting revision: {results:?}"
        );

        let db_reopened = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(db_path)
            .expect("reopened handle should open");
        let store = MutationTraceStore::new(&db_reopened);

        let worktree_state = store
            .load_worktree_state(&wt)
            .expect("worktree read should succeed")
            .expect("worktree row should exist");
        assert_eq!(
            worktree_state.revision, 1,
            "revision should have advanced exactly once, not once per writer"
        );

        let persisted_event = store
            .load_mutation_event(&wt, 1)
            .expect("mutation-event read should succeed")
            .expect("exactly one writer's mutation event should be visible at the new revision");

        assert_race_winner_state(&store, &wt, &persisted_event, &writer_a, &writer_b);
    }

    fn assert_atomic_rollback_state(
        store: &MutationTraceStore,
        worktree: &WorktreeId,
        scope_id: &ScopeId,
        rolled_back_active_scope: &ScopeId,
        surviving_active_scope: &ScopeId,
    ) {
        let worktree_state = store
            .load_worktree_state(worktree)
            .expect("worktree read should succeed")
            .expect("worktree row should exist");
        assert_eq!(worktree_state.revision, 0, "revision must roll back");
        assert_eq!(
            worktree_state.cursor_tree,
            TreeId("tree-0".to_string()),
            "cursor_tree must roll back"
        );
        assert_eq!(
            worktree_state.failure_kind,
            FailureKind::Healthy,
            "failure_kind must roll back"
        );
        assert!(
            !worktree_state.needs_rebaseline,
            "needs_rebaseline must roll back"
        );

        let scope_state = store
            .load_scope(scope_id)
            .expect("scope read should succeed")
            .expect("scope row should exist");
        assert_eq!(
            scope_state.status,
            ScopeStatus::Active,
            "scope status must roll back even though its UPDATE ran before the failure"
        );

        assert!(
            !store
                .processed_event_exists(&EventKey {
                    scope_id: scope_id.clone(),
                    event_id: EventId("event-1".to_string()),
                })
                .expect("processed-event read should succeed"),
            "the processed event must not exist even though its INSERT ran before the failure"
        );

        assert!(
            store
                .load_mutation_event(worktree, 1)
                .expect("mutation-event read should succeed")
                .is_none(),
            "the mutation event must not exist even though its INSERT ran before the failure"
        );

        let active_scopes = store
            .load_mutation_event_active_scopes(worktree, encode_revision(1).as_slice())
            .expect("active-scope read should succeed");
        assert!(
            !active_scopes.contains(rolled_back_active_scope),
            "the active-scope INSERT that was ordered before the colliding one ran but must \
             have rolled back with everything else in the transaction"
        );
        assert_eq!(
            active_scopes,
            BTreeSet::from([surviving_active_scope.clone()]),
            "only the pre-seeded row, which predates this transaction and was never part of \
             it, should remain"
        );
    }

    #[test]
    fn commit_rolls_back_every_write_kind_together_on_a_deterministic_failure() {
        let db_fixture = test_db_path("commit-atomic-rollback");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        let scope_a = ScopeId("scope-a".to_string());
        let scope_z = ScopeId("scope-z".to_string());
        insert_worktree(&db, &wt.0, 0);
        insert_scope(&db, &scope_id.0, &wt.0, ScopeStatus::Active);
        insert_active_scope(&db, &wt.0, 1, &scope_z.0);

        let before = state_with_scope(
            &wt,
            &scope_id,
            ActorKind::ClaudeCode,
            ScopeStatus::Active,
            0,
        );
        let mut after = before.clone();
        after.worktrees.get_mut(&wt).unwrap().revision = 1;
        after.scopes.get_mut(&scope_id).unwrap().status = ScopeStatus::Closed;
        after.processed_events.insert(EventKey {
            scope_id: scope_id.clone(),
            event_id: EventId("event-1".to_string()),
        });
        after.mutation_events.insert(MutationEvent {
            worktree_id: wt.clone(),
            revision: 1,
            before_tree: TreeId("tree0".to_string()),
            after_tree: TreeId("tree1".to_string()),
            active_scopes: BTreeSet::from([scope_a.clone(), scope_z.clone()]),
            tainted: false,
            failure_kind: FailureKind::Healthy,
            attribution: Attribution::AiExclusive(scope_id.clone()),
            boundary: Boundary::Close {
                scope: scope_id.clone(),
                event: EventId("event-1".to_string()),
            },
        });

        let transition = DurableTransition::between(&before, &after, &wt)
            .expect("between should succeed")
            .expect("a transition should exist for this change");

        let error = store.commit(&transition).expect_err(
            "the pre-seeded (wt0, revision=1, scope-z) active-scope row should collide with \
             the second active-scope insert, after every earlier write kind already succeeded",
        );
        assert!(error.to_string().contains("execute failed"));

        assert_atomic_rollback_state(&store, &wt, &scope_id, &scope_a, &scope_z);
    }

    #[test]
    fn commit_round_trips_u64_max_through_the_real_database() {
        let db_fixture = test_db_path("commit-u64-max");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);
        let wt = WorktreeId("wt0".to_string());
        insert_worktree(&db, &wt.0, u64::MAX - 1);

        let mut before = ProtocolState::default();
        before
            .worktrees
            .insert(wt.clone(), healthy_worktree_state(u64::MAX - 1));
        let mut after = before.clone();
        after.worktrees.get_mut(&wt).unwrap().revision = u64::MAX;

        let transition = DurableTransition::between(&before, &after, &wt)
            .expect("between should succeed")
            .expect("a transition should exist for this change");

        let result = store.commit(&transition).expect("commit should succeed");
        assert_eq!(result, CasResult::Applied);

        let worktree_state = store
            .load_worktree_state(&wt)
            .expect("worktree read should succeed")
            .expect("worktree row should exist");
        assert_eq!(worktree_state.revision, u64::MAX);
    }

    #[test]
    fn commit_rejects_a_replayed_event_key_via_the_processed_event_uniqueness_constraint() {
        let db_fixture = test_db_path("commit-replay-uniqueness");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        insert_worktree(&db, &wt.0, 0);
        insert_scope(&db, &scope_id.0, &wt.0, ScopeStatus::Active);
        insert_processed_event(&db, "scope0", "event-1");

        let before = state_with_scope(
            &wt,
            &scope_id,
            ActorKind::ClaudeCode,
            ScopeStatus::Active,
            0,
        );
        let mut after = before.clone();
        after.worktrees.get_mut(&wt).unwrap().revision = 1;
        after.processed_events.insert(EventKey {
            scope_id: scope_id.clone(),
            event_id: EventId("event-1".to_string()),
        });

        let transition = DurableTransition::between(&before, &after, &wt)
            .expect("between should succeed")
            .expect("a transition should exist for this change");

        let error = store
            .commit(&transition)
            .expect_err("a replayed (scope_id, event_id) must be rejected, not silently applied");
        assert!(error.to_string().contains("execute failed"));

        let worktree_state = store
            .load_worktree_state(&wt)
            .expect("worktree read should succeed")
            .expect("worktree row should exist");
        assert_eq!(
            worktree_state.revision, 0,
            "the whole transaction must roll back on a replay rejection"
        );
    }

    #[test]
    fn commit_of_strong_recovery_abandons_every_live_scope_on_the_worktree() {
        let db_fixture = test_db_path("commit-strong-recovery");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);
        let wt = WorktreeId("wt0".to_string());
        let scope_a = ScopeId("scope-a".to_string());
        let scope_b = ScopeId("scope-b".to_string());
        insert_worktree(&db, &wt.0, 0);
        insert_scope(&db, &scope_a.0, &wt.0, ScopeStatus::Active);
        insert_scope(&db, &scope_b.0, &wt.0, ScopeStatus::Active);

        let mut before = ProtocolState::default();
        before.worktrees.insert(
            wt.clone(),
            WorktreeState {
                cursor_tree: TreeId("tree0".to_string()),
                revision: 0,
                tainted: true,
                failure_kind: FailureKind::SnapshotFailure,
                needs_rebaseline: false,
            },
        );
        before.scopes.insert(
            scope_a.clone(),
            ScopeState {
                status: ScopeStatus::Active,
                actor_kind: ActorKind::ClaudeCode,
                worktree_id: wt.clone(),
            },
        );
        before.scopes.insert(
            scope_b.clone(),
            ScopeState {
                status: ScopeStatus::Active,
                actor_kind: ActorKind::ClaudeCode,
                worktree_id: wt.clone(),
            },
        );

        let after = recover(&before, &wt, TreeId("tree1".to_string()));
        let transition = DurableTransition::between(&before, &after, &wt)
            .expect("between should succeed")
            .expect("strong recovery should produce a durable transition");

        let result = store.commit(&transition).expect("commit should succeed");
        assert_eq!(result, CasResult::Applied);

        for scope_id in [&scope_a, &scope_b] {
            let scope_state = store
                .load_scope(scope_id)
                .expect("scope read should succeed")
                .expect("scope row should exist");
            assert_eq!(scope_state.status, ScopeStatus::Abandoned);
        }
    }

    #[test]
    fn commit_of_needs_only_recovery_leaves_live_scopes_active() {
        let db_fixture = test_db_path("commit-needs-only-recovery");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        insert_worktree(&db, &wt.0, 0);
        insert_scope(&db, &scope_id.0, &wt.0, ScopeStatus::Active);

        let mut before = ProtocolState::default();
        before.worktrees.insert(
            wt.clone(),
            WorktreeState {
                cursor_tree: TreeId("tree0".to_string()),
                revision: 0,
                tainted: false,
                failure_kind: FailureKind::Healthy,
                needs_rebaseline: true,
            },
        );
        before.scopes.insert(
            scope_id.clone(),
            ScopeState {
                status: ScopeStatus::Active,
                actor_kind: ActorKind::ClaudeCode,
                worktree_id: wt.clone(),
            },
        );

        let after = recover(&before, &wt, TreeId("tree1".to_string()));
        let transition = DurableTransition::between(&before, &after, &wt)
            .expect("between should succeed")
            .expect("needs-only recovery should produce a durable transition");

        let result = store.commit(&transition).expect("commit should succeed");
        assert_eq!(result, CasResult::Applied);

        let worktree_state = store
            .load_worktree_state(&wt)
            .expect("worktree read should succeed")
            .expect("worktree row should exist");
        assert!(!worktree_state.needs_rebaseline);

        let scope_state = store
            .load_scope(&scope_id)
            .expect("scope read should succeed")
            .expect("scope row should exist");
        assert_eq!(
            scope_state.status,
            ScopeStatus::Active,
            "a live scope must survive needs-only recovery untouched"
        );
    }

    fn insert_worktree_with_state(
        db: &RepositoryAgentTraceDb,
        worktree_id: &str,
        revision: u64,
        tainted: bool,
        failure_kind: FailureKind,
        needs_rebaseline: bool,
    ) {
        db.execute(
            "INSERT INTO mutation_trace_worktrees
                (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline)
             VALUES (?1, 'tree0', ?2, ?3, ?4, ?5)",
            (
                worktree_id,
                encode_revision(revision).as_slice(),
                tainted,
                encode_failure_kind(failure_kind),
                needs_rebaseline,
            ),
        )
        .expect("worktree insert should succeed");
    }

    fn reopen_store(db_path: &std::path::Path) -> RepositoryAgentTraceDb {
        RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(db_path)
            .expect("reopened handle should open")
    }

    fn load_before_state(
        db_path: &std::path::Path,
        worktree: &WorktreeId,
        scope: Option<&ScopeId>,
        event_key: Option<&EventKey>,
    ) -> ProtocolState {
        let db = reopen_store(db_path);
        MutationTraceStore::new(&db)
            .load_worktree(worktree, scope, event_key)
            .expect("load_worktree should succeed")
            .expect("worktree projection should exist")
            .into_protocol_state()
    }

    fn commit_transition(db_path: &std::path::Path, transition: &DurableTransition) -> CasResult {
        let db = reopen_store(db_path);
        MutationTraceStore::new(&db)
            .commit(transition)
            .expect("commit should succeed")
    }

    fn expected_projection(
        after: &ProtocolState,
        worktree: &WorktreeId,
        scope: Option<&ScopeId>,
        event_key: Option<&EventKey>,
    ) -> WorktreeProjection {
        let worktree_state = after
            .worktrees
            .get(worktree)
            .expect("after should contain the worktree")
            .clone();

        let mut scopes: BTreeMap<ScopeId, ScopeState> = after
            .scopes
            .iter()
            .filter(|(_, scope_state)| {
                scope_state.worktree_id == *worktree && scope_state.status == ScopeStatus::Active
            })
            .map(|(scope_id, scope_state)| (scope_id.clone(), scope_state.clone()))
            .collect();

        let effective_scope = scope.or(event_key.map(|key| &key.scope_id));
        if let Some(effective_scope) = effective_scope {
            if let Some(scope_state) = after.scopes.get(effective_scope) {
                scopes.insert(effective_scope.clone(), scope_state.clone());
            }
        }

        let mut processed_events = BTreeSet::new();
        if let Some(key) = event_key {
            if after.processed_events.contains(key) {
                processed_events.insert(key.clone());
            }
        }

        WorktreeProjection {
            worktree_id: worktree.clone(),
            worktree_state,
            scopes,
            processed_events,
        }
    }

    fn assert_round_trip(
        db_path: &std::path::Path,
        worktree: &WorktreeId,
        scope: Option<&ScopeId>,
        event_key: Option<&EventKey>,
        after: &ProtocolState,
    ) -> WorktreeProjection {
        let db = reopen_store(db_path);
        let store = MutationTraceStore::new(&db);

        let reloaded = store
            .load_worktree(worktree, scope, event_key)
            .expect("load_worktree should succeed")
            .expect("worktree projection should exist");

        assert_eq!(
            reloaded,
            expected_projection(after, worktree, scope, event_key)
        );

        for expected_event in &after.mutation_events {
            let reloaded_event = store
                .load_mutation_event(worktree, expected_event.revision)
                .expect("load_mutation_event should succeed")
                .expect("mutation event row should exist");
            assert_eq!(&reloaded_event, expected_event);
        }

        reloaded
    }

    #[test]
    fn round_trip_start_persists_and_reloads_exactly_after_reopening_the_database() {
        let db_fixture = test_db_path("roundtrip-start");
        let db_path = db_fixture.path();
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        {
            let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
            insert_worktree(&db, &wt.0, 0);
            insert_scope(&db, &scope_id.0, &wt.0, ScopeStatus::NeverSeen);
        }

        let before = load_before_state(db_path, &wt, Some(&scope_id), None);
        let attempt = AttemptId("attempt0".to_string());
        let event_id = EventId("event0".to_string());
        let prepared = prepare(
            &before,
            attempt.clone(),
            Boundary::Start {
                scope: scope_id.clone(),
                event: event_id.clone(),
            },
            TreeId("tree1".to_string()),
        );
        let outcome = commit(&prepared, &attempt);
        assert!(
            outcome.evaluation.accepted,
            "a fresh Start should be accepted"
        );
        let after = outcome.state;

        let transition = DurableTransition::between(&before, &after, &wt)
            .expect("between should succeed")
            .expect("a start transition should produce a durable transition");
        assert_eq!(commit_transition(db_path, &transition), CasResult::Applied);

        let event_key = EventKey {
            scope_id: scope_id.clone(),
            event_id,
        };
        assert_round_trip(db_path, &wt, Some(&scope_id), Some(&event_key), &after);
    }

    #[test]
    fn round_trip_advance_persists_and_reloads_exactly_after_reopening_the_database() {
        let db_fixture = test_db_path("roundtrip-advance");
        let db_path = db_fixture.path();
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        {
            let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
            insert_worktree(&db, &wt.0, 0);
            insert_scope(&db, &scope_id.0, &wt.0, ScopeStatus::Active);
        }

        let before = load_before_state(db_path, &wt, Some(&scope_id), None);
        let attempt = AttemptId("attempt0".to_string());
        let event_id = EventId("event0".to_string());
        let prepared = prepare(
            &before,
            attempt.clone(),
            Boundary::Advance {
                scope: scope_id.clone(),
                event: event_id.clone(),
            },
            TreeId("tree1".to_string()),
        );
        let outcome = commit(&prepared, &attempt);
        assert!(
            outcome.evaluation.accepted,
            "a fresh Advance should be accepted"
        );
        let after = outcome.state;

        let transition = DurableTransition::between(&before, &after, &wt)
            .expect("between should succeed")
            .expect("an advance transition should produce a durable transition");
        assert_eq!(commit_transition(db_path, &transition), CasResult::Applied);

        let event_key = EventKey {
            scope_id: scope_id.clone(),
            event_id,
        };
        assert_round_trip(db_path, &wt, Some(&scope_id), Some(&event_key), &after);
    }

    #[test]
    fn round_trip_close_persists_and_reloads_exactly_after_reopening_the_database() {
        let db_fixture = test_db_path("roundtrip-close");
        let db_path = db_fixture.path();
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        {
            let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
            insert_worktree(&db, &wt.0, 0);
            insert_scope(&db, &scope_id.0, &wt.0, ScopeStatus::Active);
        }

        let before = load_before_state(db_path, &wt, Some(&scope_id), None);
        let attempt = AttemptId("attempt0".to_string());
        let event_id = EventId("event0".to_string());
        let prepared = prepare(
            &before,
            attempt.clone(),
            Boundary::Close {
                scope: scope_id.clone(),
                event: event_id.clone(),
            },
            TreeId("tree1".to_string()),
        );
        let outcome = commit(&prepared, &attempt);
        assert!(
            outcome.evaluation.accepted,
            "a fresh Close should be accepted"
        );
        let after = outcome.state;
        assert_eq!(
            after.scopes.get(&scope_id).map(|s| s.status),
            Some(ScopeStatus::Closed)
        );

        let transition = DurableTransition::between(&before, &after, &wt)
            .expect("between should succeed")
            .expect("a close transition should produce a durable transition");
        assert_eq!(commit_transition(db_path, &transition), CasResult::Applied);

        let event_key = EventKey {
            scope_id: scope_id.clone(),
            event_id,
        };
        assert_round_trip(db_path, &wt, Some(&scope_id), Some(&event_key), &after);
    }

    #[test]
    fn round_trip_flush_with_change_persists_and_reloads_exactly_after_reopening_the_database() {
        let db_fixture = test_db_path("roundtrip-flush-change");
        let db_path = db_fixture.path();
        let wt = WorktreeId("wt0".to_string());
        {
            let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
            insert_worktree(&db, &wt.0, 0);
        }

        let before = load_before_state(db_path, &wt, None, None);
        let attempt = AttemptId("attempt0".to_string());
        let prepared = prepare(
            &before,
            attempt.clone(),
            Boundary::Flush {
                worktree: wt.clone(),
            },
            TreeId("tree1".to_string()),
        );
        let outcome = commit(&prepared, &attempt);
        assert!(
            outcome.evaluation.changed,
            "an observed tree change should be recorded"
        );
        let after = outcome.state;

        let transition = DurableTransition::between(&before, &after, &wt)
            .expect("between should succeed")
            .expect("a changed flush transition should produce a durable transition");
        assert_eq!(commit_transition(db_path, &transition), CasResult::Applied);

        assert_round_trip(db_path, &wt, None, None, &after);
    }

    #[test]
    fn round_trip_flush_without_change_persists_nothing_new() {
        let db_fixture = test_db_path("roundtrip-flush-no-change");
        let db_path = db_fixture.path();
        let wt = WorktreeId("wt0".to_string());
        {
            let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
            insert_worktree(&db, &wt.0, 0);
        }

        let before = load_before_state(db_path, &wt, None, None);
        let attempt = AttemptId("attempt0".to_string());
        let prepared = prepare(
            &before,
            attempt.clone(),
            Boundary::Flush {
                worktree: wt.clone(),
            },
            before.worktrees[&wt].cursor_tree.clone(),
        );
        let outcome = commit(&prepared, &attempt);
        assert!(
            !outcome.evaluation.observed_change,
            "flushing the same tree should observe no change"
        );
        let after = outcome.state;

        assert_eq!(
            DurableTransition::between(&before, &after, &wt).expect("between should succeed"),
            None,
            "a no-change flush must produce no durable transition to persist"
        );

        assert_round_trip(db_path, &wt, None, None, &after);
    }

    #[test]
    fn round_trip_taint_persists_and_reloads_exactly_after_reopening_the_database() {
        let db_fixture = test_db_path("roundtrip-taint");
        let db_path = db_fixture.path();
        let wt = WorktreeId("wt0".to_string());
        {
            let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
            insert_worktree(&db, &wt.0, 0);
        }

        let before = load_before_state(db_path, &wt, None, None);
        let after = taint(&before, &wt);

        let transition = DurableTransition::between(&before, &after, &wt)
            .expect("between should succeed")
            .expect("taint should produce a durable transition");
        assert_eq!(commit_transition(db_path, &transition), CasResult::Applied);

        assert_round_trip(db_path, &wt, None, None, &after);
    }

    #[test]
    fn round_trip_abandon_persists_and_reloads_exactly_after_reopening_the_database() {
        let db_fixture = test_db_path("roundtrip-abandon");
        let db_path = db_fixture.path();
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        {
            let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
            insert_worktree(&db, &wt.0, 0);
            insert_scope(&db, &scope_id.0, &wt.0, ScopeStatus::Active);
        }

        let before = load_before_state(db_path, &wt, Some(&scope_id), None);
        let after = abandon(&before, &scope_id);

        let transition = DurableTransition::between(&before, &after, &wt)
            .expect("between should succeed")
            .expect("abandon should produce a durable transition");
        assert_eq!(commit_transition(db_path, &transition), CasResult::Applied);

        assert_round_trip(db_path, &wt, Some(&scope_id), None, &after);
    }

    #[test]
    fn round_trip_strong_recovery_abandons_every_live_scope_after_reopening_the_database() {
        let db_fixture = test_db_path("roundtrip-recover-strong");
        let db_path = db_fixture.path();
        let wt = WorktreeId("wt0".to_string());
        let scope_a = ScopeId("scope-a".to_string());
        let scope_b = ScopeId("scope-b".to_string());
        {
            let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
            insert_worktree_with_state(&db, &wt.0, 0, true, FailureKind::SnapshotFailure, false);
            insert_scope(&db, &scope_a.0, &wt.0, ScopeStatus::Active);
            insert_scope(&db, &scope_b.0, &wt.0, ScopeStatus::Active);
        }

        let before = load_before_state(db_path, &wt, None, None);
        let after = recover(&before, &wt, TreeId("tree1".to_string()));

        let transition = DurableTransition::between(&before, &after, &wt)
            .expect("between should succeed")
            .expect("strong recovery should produce a durable transition");
        assert_eq!(commit_transition(db_path, &transition), CasResult::Applied);

        assert_round_trip(db_path, &wt, None, None, &after);

        let db = reopen_store(db_path);
        let store = MutationTraceStore::new(&db);
        for scope_id in [&scope_a, &scope_b] {
            let scope_state = store
                .load_scope(scope_id)
                .expect("scope read should succeed")
                .expect("scope row should exist");
            assert_eq!(scope_state.status, ScopeStatus::Abandoned);
        }
    }

    #[test]
    fn round_trip_contended_mutation_persists_and_reloads_exactly_after_reopening_the_database() {
        let db_fixture = test_db_path("roundtrip-contended");
        let db_path = db_fixture.path();
        let wt = WorktreeId("wt0".to_string());
        let scope_a = ScopeId("scope-a".to_string());
        let scope_b = ScopeId("scope-b".to_string());
        {
            let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
            insert_worktree(&db, &wt.0, 0);
            insert_scope(&db, &scope_a.0, &wt.0, ScopeStatus::Active);
            insert_scope(&db, &scope_b.0, &wt.0, ScopeStatus::Active);
        }

        let before = load_before_state(db_path, &wt, None, None);
        let attempt = AttemptId("attempt0".to_string());
        let prepared = prepare(
            &before,
            attempt.clone(),
            Boundary::Flush {
                worktree: wt.clone(),
            },
            TreeId("tree1".to_string()),
        );
        let outcome = commit(&prepared, &attempt);
        assert!(
            outcome.evaluation.changed,
            "an observed tree change should be recorded"
        );
        let after = outcome.state;
        let mutation_event = after
            .mutation_events
            .iter()
            .next()
            .expect("a mutation event should have been produced");
        assert_eq!(mutation_event.attribution, Attribution::AiContended);
        assert_eq!(
            mutation_event.active_scopes,
            BTreeSet::from([scope_a.clone(), scope_b.clone()])
        );

        let transition = DurableTransition::between(&before, &after, &wt)
            .expect("between should succeed")
            .expect("a contended flush transition should produce a durable transition");
        assert_eq!(commit_transition(db_path, &transition), CasResult::Applied);

        let reloaded = assert_round_trip(db_path, &wt, None, None, &after);
        assert_eq!(
            reloaded.scopes.keys().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from([scope_a.clone(), scope_b.clone()]),
            "the reloaded live bounded projection must contain exactly the two contended scopes"
        );
        for scope_id in [&scope_a, &scope_b] {
            assert_eq!(
                reloaded.scopes.get(scope_id).map(|s| s.status),
                Some(ScopeStatus::Active)
            );
        }
    }

    #[test]
    fn round_trip_database_failure_changes_only_non_persistent_external_taint() {
        let db_fixture = test_db_path("roundtrip-database-failure");
        let db_path = db_fixture.path();
        let wt = WorktreeId("wt0".to_string());
        {
            let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
            insert_worktree(&db, &wt.0, 0);
        }

        let before = load_before_state(db_path, &wt, None, None);
        let after = database_failure(&before, &wt);

        assert!(!before.external_taint.contains(&wt));
        assert!(after.external_taint.contains(&wt));
        assert_eq!(before.worktrees, after.worktrees);
        assert_eq!(before.scopes, after.scopes);
        assert_eq!(before.processed_events, after.processed_events);
        assert_eq!(
            after.worktrees[&wt].revision,
            before.worktrees[&wt].revision
        );

        assert_eq!(
            DurableTransition::between(&before, &after, &wt).expect("between should succeed"),
            None,
            "database_failure only changes external_taint, which is never durable, so no \
             DurableTransition should exist to commit"
        );

        let reloaded = assert_round_trip(db_path, &wt, None, None, &after);
        assert!(reloaded.into_protocol_state().external_taint.is_empty());
    }

    #[test]
    fn round_trip_a_replayed_event_key_is_rejected_and_does_not_advance_the_worktree_again() {
        let db_fixture = test_db_path("roundtrip-replay");
        let db_path = db_fixture.path();
        let wt = WorktreeId("wt0".to_string());
        let scope_id = ScopeId("scope0".to_string());
        let event_key = EventKey {
            scope_id: scope_id.clone(),
            event_id: EventId("event0".to_string()),
        };
        {
            let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
            insert_worktree(&db, &wt.0, 0);
            insert_scope(&db, &scope_id.0, &wt.0, ScopeStatus::Active);
        }

        let before = load_before_state(db_path, &wt, Some(&scope_id), None);
        let attempt = AttemptId("attempt0".to_string());
        let prepared = prepare(
            &before,
            attempt.clone(),
            Boundary::Advance {
                scope: scope_id.clone(),
                event: event_key.event_id.clone(),
            },
            TreeId("tree1".to_string()),
        );
        let outcome = commit(&prepared, &attempt);
        assert!(
            outcome.evaluation.accepted,
            "the first delivery should be accepted"
        );
        let after = outcome.state;

        let transition = DurableTransition::between(&before, &after, &wt)
            .expect("between should succeed")
            .expect("the first delivery should produce a durable transition");
        assert_eq!(commit_transition(db_path, &transition), CasResult::Applied);

        let before_replay = load_before_state(db_path, &wt, Some(&scope_id), Some(&event_key));
        assert!(before_replay.processed_events.contains(&event_key));

        let replay_attempt = AttemptId("attempt1".to_string());
        let replay_prepared = prepare(
            &before_replay,
            replay_attempt.clone(),
            Boundary::Advance {
                scope: scope_id.clone(),
                event: event_key.event_id.clone(),
            },
            TreeId("tree2".to_string()),
        );
        let replay_outcome = commit(&replay_prepared, &replay_attempt);
        assert!(
            !replay_outcome.evaluation.accepted,
            "a replayed EventKey must be rejected"
        );
        let after_replay = replay_outcome.state;

        assert_eq!(
            DurableTransition::between(&before_replay, &after_replay, &wt)
                .expect("between should succeed"),
            None,
            "a rejected replay must produce no durable transition to persist"
        );

        assert_round_trip(db_path, &wt, Some(&scope_id), Some(&event_key), &after);
    }

    fn insert_worktree_with_cursor(
        db: &RepositoryAgentTraceDb,
        worktree_id: &str,
        revision: u64,
        cursor_tree: &str,
    ) {
        db.execute(
            "INSERT INTO mutation_trace_worktrees
                (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline)
             VALUES (?1, ?2, ?3, 0, 'healthy', 0)",
            (
                worktree_id,
                cursor_tree,
                encode_revision(revision).as_slice(),
            ),
        )
        .expect("worktree insert should succeed");
    }

    fn insert_event_trees(
        db: &RepositoryAgentTraceDb,
        worktree_id: &str,
        revision: u64,
        before_tree: &str,
        after_tree: &str,
    ) {
        db.execute(
            "INSERT INTO mutation_trace_events
                (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
                 attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id)
             VALUES (?1, ?2, ?3, ?4, 0, 'healthy', 'ineligible_unscoped', NULL, 'flush', NULL, NULL)",
            (
                worktree_id,
                encode_revision(revision).as_slice(),
                before_tree,
                after_tree,
            ),
        )
        .expect("mutation event insert should succeed");
    }

    fn tree_set<const N: usize>(trees: [&str; N]) -> BTreeSet<TreeId> {
        trees.into_iter().map(|t| TreeId(t.to_string())).collect()
    }

    #[test]
    fn load_tree_roots_returns_cursor_and_every_event_tree_deduplicated() {
        let db_fixture = test_db_path("tree-roots-cursor-and-events");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree_with_cursor(&db, "wt-1", 2, "tree-2");
        insert_event_trees(&db, "wt-1", 1, "tree-0", "tree-1");
        insert_event_trees(&db, "wt-1", 2, "tree-1", "tree-2");

        let roots = store
            .load_tree_roots(&WorktreeId("wt-1".to_string()))
            .expect("load_tree_roots should succeed");

        assert_eq!(roots, tree_set(["tree-0", "tree-1", "tree-2"]));
    }

    #[test]
    fn load_tree_roots_excludes_other_worktrees_trees() {
        let db_fixture = test_db_path("tree-roots-excludes-other-worktree");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree_with_cursor(&db, "wt-1", 1, "tree-1a");
        insert_event_trees(&db, "wt-1", 1, "tree-0a", "tree-1a");

        insert_worktree_with_cursor(&db, "wt-2", 1, "tree-1b");
        insert_event_trees(&db, "wt-2", 1, "tree-0b", "tree-1b");

        let roots = store
            .load_tree_roots(&WorktreeId("wt-1".to_string()))
            .expect("load_tree_roots should succeed");

        assert_eq!(roots, tree_set(["tree-0a", "tree-1a"]));
    }

    #[test]
    fn load_tree_roots_is_empty_for_an_unmaterialized_worktree() {
        let db_fixture = test_db_path("tree-roots-unmaterialized-worktree");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree_with_cursor(&db, "wt-other", 0, "tree-other");

        let roots = store
            .load_tree_roots(&WorktreeId("wt-missing".to_string()))
            .expect("load_tree_roots should return Ok for a worktree with no durable row");

        assert!(roots.is_empty());
    }

    #[test]
    fn load_tree_roots_remains_worktree_scoped() {
        let db_fixture = test_db_path("tree-roots-worktree-scoped");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree_with_cursor(&db, "wt-1", 1, "tree-1a");
        insert_event_trees(&db, "wt-1", 1, "tree-0a", "tree-1a");
        insert_worktree_with_cursor(&db, "wt-2", 2, "tree-2b");
        insert_event_trees(&db, "wt-2", 1, "tree-0b", "tree-1b");
        insert_event_trees(&db, "wt-2", 2, "tree-1b", "tree-2b");

        assert_eq!(
            store
                .load_tree_roots(&WorktreeId("wt-1".to_string()))
                .expect("load_tree_roots should succeed"),
            tree_set(["tree-0a", "tree-1a"]),
        );
        assert_eq!(
            store
                .load_tree_roots(&WorktreeId("wt-2".to_string()))
                .expect("load_tree_roots should succeed"),
            tree_set(["tree-0b", "tree-1b", "tree-2b"]),
        );
    }

    #[test]
    fn load_all_tree_roots_returns_every_worktree_cursor_and_event_tree_deduplicated() {
        let db_fixture = test_db_path("all-tree-roots-every-worktree");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree_with_cursor(&db, "wt-1", 1, "tree-1a");
        insert_event_trees(&db, "wt-1", 1, "tree-0a", "tree-1a");
        insert_worktree_with_cursor(&db, "wt-2", 2, "tree-2b");
        insert_event_trees(&db, "wt-2", 1, "tree-0b", "tree-1b");
        insert_event_trees(&db, "wt-2", 2, "tree-1b", "tree-2b");

        let roots = store
            .load_all_tree_roots()
            .expect("load_all_tree_roots should succeed");

        assert_eq!(
            roots,
            tree_set(["tree-0a", "tree-1a", "tree-0b", "tree-1b", "tree-2b",]),
        );
    }

    #[test]
    fn load_all_tree_roots_deduplicates_a_tree_shared_by_multiple_worktrees() {
        let db_fixture = test_db_path("all-tree-roots-shared-tree");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree_with_cursor(&db, "wt-1", 1, "tree-shared");
        insert_event_trees(&db, "wt-1", 1, "tree-0a", "tree-shared");
        insert_worktree_with_cursor(&db, "wt-2", 1, "tree-1b");
        insert_event_trees(&db, "wt-2", 1, "tree-shared", "tree-1b");

        let roots = store
            .load_all_tree_roots()
            .expect("load_all_tree_roots should succeed");

        assert_eq!(roots, tree_set(["tree-0a", "tree-shared", "tree-1b"]));
    }

    #[test]
    fn load_all_tree_roots_is_empty_for_an_empty_repository() {
        let db_fixture = test_db_path("all-tree-roots-empty-repository");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        let roots = store
            .load_all_tree_roots()
            .expect("load_all_tree_roots should return Ok for an empty repository");

        assert!(roots.is_empty());
    }

    fn apply_atomic_cursor_advance(db: &RepositoryAgentTraceDb) {
        let guard = TransactionStatement::new(
            "UPDATE mutation_trace_worktrees SET cursor_tree = ?1, revision = ?2
             WHERE worktree_id = ?3 AND revision = ?4",
            (
                "tree-x",
                encode_revision(1).as_slice(),
                "wt-b",
                encode_revision(0).as_slice(),
            ),
        )
        .expect("guard statement should build");
        let statements = [TransactionStatement::new(
            "INSERT INTO mutation_trace_events
                (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
                 attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id)
             VALUES (?1, ?2, ?3, ?4, 0, 'healthy', 'ineligible_unscoped', NULL, 'flush', NULL, NULL)",
            (
                "wt-b",
                encode_revision(1).as_slice(),
                "tree-t",
                "tree-x",
            ),
        )
        .expect("event statement should build")];

        let applied = db
            .execute_transactional_cas_batch(
                "atomic cursor advance test",
                "retry the operation",
                &guard,
                &statements,
            )
            .expect("the atomic cursor advance should commit");
        assert!(applied, "the CAS guard should have matched revision 0");
    }

    fn select_trees(db: &RepositoryAgentTraceDb, sql: &str) -> BTreeSet<TreeId> {
        db.query_map(sql, (), |row| {
            let tree: String = row.get(0).context("failed to read a tree column")?;
            Ok(TreeId(tree))
        })
        .expect("tree column select should succeed")
        .into_iter()
        .collect()
    }

    /// State-transition coverage only: before the advance `T` is a root
    /// through `cursor_tree`; after it, `T` is a root through `before_tree`.
    /// This does NOT prove single-statement snapshot isolation — a torn
    /// multi-read implementation would still pass this pre/post check.
    /// `load_all_tree_roots_reads_every_durable_root_in_one_sql_statement` is
    /// the deterministic regression for that property.
    #[test]
    fn load_all_tree_roots_retains_previous_cursor_after_atomic_cursor_advance() {
        let db_fixture = test_db_path("all-tree-roots-retains-previous-cursor");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree_with_cursor(&db, "wt-b", 0, "tree-t");

        let pre = store
            .load_all_tree_roots()
            .expect("load_all_tree_roots should succeed");
        assert!(
            pre.contains(&TreeId("tree-t".to_string())),
            "T is a durable root before the transition (via cursor_tree)"
        );

        apply_atomic_cursor_advance(&db);

        let post = store
            .load_all_tree_roots()
            .expect("load_all_tree_roots should succeed");
        assert!(
            post.contains(&TreeId("tree-t".to_string())),
            "T is still a durable root after the advance (via before_tree)"
        );
        assert!(
            post.contains(&TreeId("tree-x".to_string())),
            "X becomes a durable root after the advance"
        );
    }

    /// Deterministic regression for the actual concurrency boundary: one
    /// `load_all_tree_roots()` invocation must read `cursor_tree`,
    /// `before_tree`, and `after_tree` through a SINGLE SQL statement / one
    /// database snapshot. If it is reimplemented as two or more independent
    /// `SELECT`s unioned in Rust, an atomic `cursor T -> X` +
    /// `MutationEvent { before = T, after = X }` commit interleaved between
    /// those reads produces a torn set that omits `T`. The test constructs
    /// that torn set explicitly (an events read, the atomic advance, then a
    /// worktrees read, unioned in Rust — losing `T`) and then asserts the
    /// production path issues exactly one read statement, so it can never
    /// enter the interleaving and always retains `T`.
    #[test]
    fn load_all_tree_roots_reads_every_durable_root_in_one_sql_statement() {
        let db_fixture = test_db_path("all-tree-roots-single-statement");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree_with_cursor(&db, "wt-b", 0, "tree-t");

        let events_first = select_trees(
            &db,
            "SELECT before_tree AS tree FROM mutation_trace_events
             UNION
             SELECT after_tree AS tree FROM mutation_trace_events",
        );
        assert!(
            events_first.is_empty(),
            "no event references T before the advance"
        );

        apply_atomic_cursor_advance(&db);

        let cursors_second = select_trees(
            &db,
            "SELECT cursor_tree AS tree FROM mutation_trace_worktrees",
        );
        let torn: BTreeSet<TreeId> = events_first.union(&cursors_second).cloned().collect();
        assert!(
            !torn.contains(&TreeId("tree-t".to_string())),
            "a two-read implementation loses T across the atomic advance"
        );

        let (roots, statements_issued) = crate::services::db::count_read_statements(|| {
            store
                .load_all_tree_roots()
                .expect("load_all_tree_roots should succeed")
        });
        assert_eq!(
            statements_issued, 1,
            "load_all_tree_roots must read every durable-root column in one SQL statement"
        );
        assert!(
            roots.contains(&TreeId("tree-t".to_string())),
            "the single-statement snapshot always retains T (via before_tree)"
        );
        assert!(roots.contains(&TreeId("tree-x".to_string())));
    }

    /// The same single-statement / single-snapshot property, worktree-scoped:
    /// one `load_tree_roots(W)` call reads W's `cursor_tree` / `before_tree` /
    /// `after_tree` through one statement. A two-read reimplementation
    /// (events-for-W, then cursor-for-W) would tear across an atomic cursor
    /// advance in exactly the same way.
    #[test]
    fn load_tree_roots_reads_every_durable_root_in_one_sql_statement() {
        let db_fixture = test_db_path("tree-roots-single-statement");
        let db_path = db_fixture.path();
        let db = RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        insert_worktree_with_cursor(&db, "wt-b", 0, "tree-t");

        let events_first = select_trees(
            &db,
            "SELECT before_tree AS tree FROM mutation_trace_events WHERE worktree_id = 'wt-b'
             UNION
             SELECT after_tree AS tree FROM mutation_trace_events WHERE worktree_id = 'wt-b'",
        );
        assert!(
            events_first.is_empty(),
            "no event references T before the advance"
        );

        apply_atomic_cursor_advance(&db);

        let cursors_second = select_trees(
            &db,
            "SELECT cursor_tree AS tree FROM mutation_trace_worktrees WHERE worktree_id = 'wt-b'",
        );
        let torn: BTreeSet<TreeId> = events_first.union(&cursors_second).cloned().collect();
        assert!(
            !torn.contains(&TreeId("tree-t".to_string())),
            "a two-read implementation loses T across the atomic advance"
        );

        let (roots, statements_issued) = crate::services::db::count_read_statements(|| {
            store
                .load_tree_roots(&WorktreeId("wt-b".to_string()))
                .expect("load_tree_roots should succeed")
        });
        assert_eq!(
            statements_issued, 1,
            "load_tree_roots must read every durable-root column in one SQL statement"
        );
        assert!(
            roots.contains(&TreeId("tree-t".to_string())),
            "the single-statement snapshot always retains T (via before_tree)"
        );
        assert!(roots.contains(&TreeId("tree-x".to_string())));
    }
}
