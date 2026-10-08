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

/// Maximum number of mutation-event rows returned by one attribution-history
/// page request.
pub const MUTATION_ATTRIBUTION_PAGE_SIZE: usize = 32;

/// The cold-path subset of a historical mutation event needed by attribution.
///
/// This deliberately omits boundary data and active scopes: attribution only
/// needs the tree transition and the event's health/attribution state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationEventPageRow {
    pub revision: u64,
    pub before_tree: TreeId,
    pub after_tree: TreeId,
    pub failure_kind: FailureKind,
    pub attribution_kind: AttributionKind,
    pub attribution_scope_id: Option<ScopeId>,
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
#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
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
const SELECT_SCOPE_PROVENANCE_SQL: &str =
    "SELECT scope_id, session_id, model_id FROM mutation_trace_scope_provenance WHERE scope_id = ?1";
const SELECT_PROCESSED_EVENT_SQL: &str =
    "SELECT 1 FROM mutation_trace_processed_events WHERE scope_id = ?1 AND event_id = ?2";
#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
const SELECT_MUTATION_EVENT_SQL: &str = "SELECT before_tree, after_tree, tainted, failure_kind,
            attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id
     FROM mutation_trace_events WHERE worktree_id = ?1 AND revision = ?2";
const SELECT_MUTATION_EVENT_PAGE_SQL: &str = "SELECT revision, before_tree, after_tree, tainted,
            failure_kind, attribution_kind, attribution_scope_id
     FROM mutation_trace_events
     WHERE worktree_id = ?1
     ORDER BY revision DESC
     LIMIT ?2";
const SELECT_MUTATION_EVENT_PAGE_AFTER_SQL: &str =
    "SELECT revision, before_tree, after_tree, tainted,
            failure_kind, attribution_kind, attribution_scope_id
     FROM mutation_trace_events
     WHERE worktree_id = ?1 AND revision < ?2
     ORDER BY revision DESC
     LIMIT ?3";
const SELECT_LATEST_MUTATION_EVENT_REVISION_SQL: &str = "SELECT revision FROM mutation_trace_events
     WHERE worktree_id = ?1
     ORDER BY revision DESC
     LIMIT 1";
#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
const SELECT_MUTATION_EVENT_ACTIVE_SCOPES_SQL: &str =
    "SELECT scope_id FROM mutation_trace_event_active_scopes WHERE worktree_id = ?1 AND revision = ?2";
/// One worktree's complete durable tree root set — its cursor tree plus the
/// `before_tree` / `after_tree` of every historical `mutation_trace_events`
/// row — as a single `UNION` statement so the whole set is read from one
/// database snapshot, never assembled from independent `SELECT`s.
#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
const SELECT_TREE_ROOTS_BY_WORKTREE_SQL: &str =
    "SELECT cursor_tree AS tree FROM mutation_trace_worktrees WHERE worktree_id = ?1
     UNION
     SELECT before_tree AS tree FROM mutation_trace_events    WHERE worktree_id = ?1
     UNION
     SELECT after_tree  AS tree FROM mutation_trace_events    WHERE worktree_id = ?1";
/// The same three `TreeId` columns unioned across **every** worktree in the
/// repository, in one statement / one snapshot — the reconciler's
/// repository-wide retention set.
#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
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
const INSERT_SCOPE_PROVENANCE_IF_ABSENT_SQL: &str =
    "INSERT INTO mutation_trace_scope_provenance (scope_id, session_id, model_id)
     VALUES (?1, ?2, ?3)
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

#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeProvenance {
    pub scope_id: ScopeId,
    pub session_id: String,
    pub model_id: Option<String>,
}

pub struct MutationTraceStore<'a> {
    db: &'a mut RepositoryAgentTraceDb,
}

impl<'a> MutationTraceStore<'a> {
    pub fn new(db: &'a mut RepositoryAgentTraceDb) -> Self {
        Self { db }
    }

    /// Idempotently initializes `worktree`'s durable cursor row: `revision=0`,
    /// healthy, not tainted, not needing rebaseline, with `cursor_tree` set to
    /// `initial_tree`. A no-op when the worktree row already exists — an
    /// existing cursor, revision, or failure state is never overwritten.
    pub async fn initialize_worktree(
        &self,
        worktree: &WorktreeId,
        initial_tree: &TreeId,
    ) -> Result<()> {
        self.db
            .execute_idempotent_write(
                INSERT_WORKTREE_IF_ABSENT_SQL,
                (
                    worktree.0.as_str(),
                    initial_tree.0.as_str(),
                    encode_revision(0).as_slice(),
                ),
            )
            .await?;

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
    pub async fn register_scope(
        &self,
        scope: &ScopeId,
        worktree: &WorktreeId,
        actor_kind: ActorKind,
    ) -> Result<ScopeState> {
        if self.load_worktree_state(worktree).await?.is_none() {
            bail!(
                "cannot register scope {scope:?}: worktree {worktree:?} has no mutation_trace_worktrees row"
            );
        }

        self.db
            .execute_idempotent_write(
                INSERT_SCOPE_IF_ABSENT_SQL,
                (
                    scope.0.as_str(),
                    worktree.0.as_str(),
                    encode_actor_kind(actor_kind),
                ),
            )
            .await?;

        let scope_state = self.load_scope(scope).await?.ok_or_else(|| {
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
    pub async fn load_worktree(
        &self,
        worktree: &WorktreeId,
        scope: Option<&ScopeId>,
        event_key: Option<&EventKey>,
    ) -> Result<Option<WorktreeProjection>> {
        let effective_scope = effective_referenced_scope(scope, event_key)?;

        let Some(worktree_state) = self.load_worktree_state(worktree).await? else {
            return Ok(None);
        };

        let mut scopes = self.load_active_scopes(worktree).await?;

        if let Some(effective_scope_id) = effective_scope {
            if !scopes.contains_key(effective_scope_id) {
                let scope_state = self.load_scope(effective_scope_id).await?.ok_or_else(|| {
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
            Some(event_key) if self.processed_event_exists(event_key).await? => {
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
    #[allow(
        dead_code,
        reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
    )]
    pub async fn load_mutation_event(
        &self,
        worktree: &WorktreeId,
        revision: u64,
    ) -> Result<Option<MutationEvent>> {
        let revision_blob = encode_revision(revision);

        let rows = self
            .db
            .query_map(
                SELECT_MUTATION_EVENT_SQL,
                (worktree.0.as_str(), revision_blob.as_slice()),
                mutation_event_row_from_turso,
            )
            .await?;

        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };

        let active_scopes = self
            .load_mutation_event_active_scopes(worktree, &revision_blob)
            .await?;

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

    /// Reads one descending page of the historical mutation events for exactly
    /// `worktree`. When `revision_cursor` is present, only revisions strictly
    /// below it are returned, so the caller can continue from the last row of
    /// a prior page without duplicates. The requested limit is always capped
    /// at [`MUTATION_ATTRIBUTION_PAGE_SIZE`].
    ///
    /// This is a read-only cold path. It does not load active scopes, processed
    /// events, boundary data, or any timestamp column.
    pub async fn load_mutation_event_page(
        &self,
        worktree: &WorktreeId,
        revision_cursor: Option<u64>,
        requested_limit: usize,
    ) -> Result<Vec<MutationEventPageRow>> {
        let limit = requested_limit.min(MUTATION_ATTRIBUTION_PAGE_SIZE);
        let rows = match revision_cursor {
            Some(cursor) => {
                let cursor_blob = encode_revision(cursor);
                self.db
                    .query_map(
                        SELECT_MUTATION_EVENT_PAGE_AFTER_SQL,
                        (
                            worktree.0.as_str(),
                            cursor_blob.as_slice(),
                            limit_as_i64(limit),
                        ),
                        mutation_event_page_row_from_turso,
                    )
                    .await?
            }
            None => {
                self.db
                    .query_map(
                        SELECT_MUTATION_EVENT_PAGE_SQL,
                        (worktree.0.as_str(), limit_as_i64(limit)),
                        mutation_event_page_row_from_turso,
                    )
                    .await?
            }
        };

        Ok(rows)
    }

    pub async fn latest_mutation_event_revision(
        &self,
        worktree: &WorktreeId,
    ) -> Result<Option<u64>> {
        let rows = self
            .db
            .query_map(
                SELECT_LATEST_MUTATION_EVENT_REVISION_SQL,
                (worktree.0.as_str(),),
                |row| {
                    let blob: Vec<u8> = row
                        .get(0)
                        .context("failed to read mutation_trace_events.revision")?;
                    decode_revision(&blob)
                },
            )
            .await?;
        Ok(rows.into_iter().next())
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
    #[allow(
        dead_code,
        reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
    )]
    pub async fn load_tree_roots(&self, worktree: &WorktreeId) -> Result<BTreeSet<TreeId>> {
        let rows = self
            .db
            .query_map(
                SELECT_TREE_ROOTS_BY_WORKTREE_SQL,
                (worktree.0.as_str(),),
                tree_root_row_from_turso,
            )
            .await?;

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
    #[allow(
        dead_code,
        reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
    )]
    pub async fn load_all_tree_roots(&self) -> Result<BTreeSet<TreeId>> {
        let rows = self
            .db
            .query_map(SELECT_ALL_TREE_ROOTS_SQL, (), tree_root_row_from_turso)
            .await?;

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
    pub async fn load_scope(&self, scope_id: &ScopeId) -> Result<Option<ScopeState>> {
        let rows = self
            .db
            .query_map(
                SELECT_SCOPE_BY_ID_SQL,
                (scope_id.0.as_str(),),
                scope_row_from_turso,
            )
            .await?;

        Ok(rows.into_iter().next().map(|(_, scope_state)| scope_state))
    }

    pub async fn register_scope_provenance(
        &self,
        provenance: &ScopeProvenance,
    ) -> Result<ScopeProvenance> {
        if self.load_scope(&provenance.scope_id).await?.is_none() {
            bail!(
                "cannot register provenance for scope {:?}: it has no mutation_trace_scopes row",
                provenance.scope_id
            );
        }

        self.db
            .execute_idempotent_write(
                INSERT_SCOPE_PROVENANCE_IF_ABSENT_SQL,
                (
                    provenance.scope_id.0.as_str(),
                    provenance.session_id.as_str(),
                    provenance.model_id.as_deref(),
                ),
            )
            .await?;

        let stored = self
            .load_scope_provenance(&provenance.scope_id).await?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "scope {:?} has no provenance row immediately after register_scope_provenance insert",
                    provenance.scope_id
                )
            })?;

        if stored.session_id != provenance.session_id {
            bail!(
                "scope {:?} already has provenance for session {}, not {}",
                provenance.scope_id,
                stored.session_id,
                provenance.session_id
            );
        }

        Ok(stored)
    }

    pub async fn load_scope_provenance(
        &self,
        scope_id: &ScopeId,
    ) -> Result<Option<ScopeProvenance>> {
        let rows = self
            .db
            .query_map(
                SELECT_SCOPE_PROVENANCE_SQL,
                (scope_id.0.as_str(),),
                scope_provenance_row_from_turso,
            )
            .await?;

        Ok(rows.into_iter().next())
    }

    async fn load_worktree_state(&self, worktree: &WorktreeId) -> Result<Option<WorktreeState>> {
        let rows = self
            .db
            .query_map(
                SELECT_WORKTREE_SQL,
                (worktree.0.as_str(),),
                worktree_state_row_from_turso,
            )
            .await?;

        Ok(rows.into_iter().next())
    }

    async fn load_active_scopes(
        &self,
        worktree: &WorktreeId,
    ) -> Result<BTreeMap<ScopeId, ScopeState>> {
        let rows = self
            .db
            .query_map(
                SELECT_SCOPES_BY_WORKTREE_AND_STATUS_SQL,
                (
                    worktree.0.as_str(),
                    encode_scope_status(ScopeStatus::Active),
                ),
                scope_row_from_turso,
            )
            .await?;

        Ok(rows.into_iter().collect())
    }

    async fn processed_event_exists(&self, event_key: &EventKey) -> Result<bool> {
        let rows = self
            .db
            .query_map(
                SELECT_PROCESSED_EVENT_SQL,
                (event_key.scope_id.0.as_str(), event_key.event_id.0.as_str()),
                |row| row.get::<i64>(0).map_err(Into::into),
            )
            .await?;

        Ok(!rows.is_empty())
    }

    #[allow(
        dead_code,
        reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
    )]
    async fn load_mutation_event_active_scopes(
        &self,
        worktree: &WorktreeId,
        revision_blob: &[u8],
    ) -> Result<BTreeSet<ScopeId>> {
        let rows = self
            .db
            .query_map(
                SELECT_MUTATION_EVENT_ACTIVE_SCOPES_SQL,
                (worktree.0.as_str(), revision_blob),
                |row| row.get::<String>(0).map(ScopeId).map_err(Into::into),
            )
            .await?;

        Ok(rows.into_iter().collect())
    }

    pub async fn commit(&mut self, transition: &DurableTransition) -> Result<CasResult> {
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

        let applied = self
            .db
            .execute_transactional_cas_batch(
                "commit mutation-trace durable transition",
                "reload the worktree and retry the transition",
                &guard,
                &statements,
            )
            .await?;

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

fn limit_as_i64(limit: usize) -> i64 {
    i64::try_from(limit).expect("mutation attribution page limit should fit in i64")
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

fn validate_health_encoding(tainted: bool, failure_kind: FailureKind) -> Result<()> {
    let expected_tainted = failure_kind != FailureKind::Healthy;
    if tainted != expected_tainted {
        bail!("inconsistent mutation-trace health encoding: tainted={tainted} with failure_kind={failure_kind:?}");
    }
    Ok(())
}

#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
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

    let failure_kind = decode_failure_kind(&failure_kind)?;
    validate_health_encoding(tainted, failure_kind)
        .context("invalid mutation_trace_worktrees.tainted/failure_kind pair")?;

    Ok(WorktreeState {
        cursor_tree: TreeId(cursor_tree),
        revision: decode_revision(&revision_blob)?,
        tainted,
        failure_kind,
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

fn scope_provenance_row_from_turso(row: &turso::Row) -> Result<ScopeProvenance> {
    let scope_id: String = row
        .get(0)
        .context("failed to read mutation_trace_scope_provenance.scope_id")?;
    let session_id: String = row
        .get(1)
        .context("failed to read mutation_trace_scope_provenance.session_id")?;
    let model_id: Option<String> = row
        .get(2)
        .context("failed to read mutation_trace_scope_provenance.model_id")?;

    Ok(ScopeProvenance {
        scope_id: ScopeId(scope_id),
        session_id,
        model_id,
    })
}

fn mutation_event_page_row_from_turso(row: &turso::Row) -> Result<MutationEventPageRow> {
    let revision_blob: Vec<u8> = row
        .get(0)
        .context("failed to read mutation_trace_events.revision")?;
    let before_tree: String = row
        .get(1)
        .context("failed to read mutation_trace_events.before_tree")?;
    let after_tree: String = row
        .get(2)
        .context("failed to read mutation_trace_events.after_tree")?;
    let tainted: bool = row
        .get(3)
        .context("failed to read mutation_trace_events.tainted")?;
    let failure_kind: String = row
        .get(4)
        .context("failed to read mutation_trace_events.failure_kind")?;
    let attribution_kind: String = row
        .get(5)
        .context("failed to read mutation_trace_events.attribution_kind")?;
    let attribution_scope_id: Option<String> = row
        .get(6)
        .context("failed to read mutation_trace_events.attribution_scope_id")?;
    let failure_kind = decode_failure_kind(&failure_kind)?;
    validate_health_encoding(tainted, failure_kind)
        .context("invalid mutation_trace_events.tainted/failure_kind pair")?;
    let attribution_kind = decode_attribution_kind(&attribution_kind)?;
    reconstruct_attribution(attribution_kind, attribution_scope_id.clone())?;
    let attribution_scope_id = attribution_scope_id.map(ScopeId);

    Ok(MutationEventPageRow {
        revision: decode_revision(&revision_blob)?,
        before_tree: TreeId(before_tree),
        after_tree: TreeId(after_tree),
        failure_kind,
        attribution_kind,
        attribution_scope_id,
    })
}

/// Raw decoded `mutation_trace_events` row fields, prior to reconstructing
/// the full `Attribution`/`Boundary`/`active_scopes` a [`MutationEvent`]
/// carries.
#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
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

#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
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

    let failure_kind = decode_failure_kind(&failure_kind)?;
    validate_health_encoding(tainted, failure_kind)
        .context("invalid mutation_trace_events.tainted/failure_kind pair")?;

    Ok(MutationEventRow {
        before_tree,
        after_tree,
        tainted,
        failure_kind,
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

#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
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
