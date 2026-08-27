//! Domain<->SQL codecs and bounded read access for the mutation-cursor
//! persistence layer.
//!
//! The codecs are the only translation between `super::types` domain values
//! and the `TEXT`/`BLOB` representations `cli/migrations/agent-trace-
//! repository/003_mutation_trace_protocol.sql` constrains those columns to.
//! Every codec here is an explicit function over a fixed set of variants — no
//! codec derives from `Debug` or a serde representation, so a variant rename
//! cannot silently change the durable encoding.
//!
//! `MutationTraceStore` adds the hot-path bounded worktree read
//! (`load_worktree`) and the cold-path historical read (`load_mutation_event`)
//! against a `&RepositoryAgentTraceDb`. Initialization and CAS-commit logic
//! are later tasks (`mutation-cursor-store-persistence` T04/T06/T07); this
//! module carries no such logic yet.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Context, Result};

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;

use super::types::{
    ActorKind, Attribution, Boundary, EventId, EventKey, FailureKind, MutationEvent, ProtocolState,
    ScopeId, ScopeState, ScopeStatus, TreeId, WorktreeId, WorktreeState,
};

/// Encodes a worktree/event revision as the 8-byte big-endian `BLOB` stored
/// by every `revision` column in migration `003`.
pub fn encode_revision(revision: u64) -> [u8; 8] {
    revision.to_be_bytes()
}

/// Decodes a worktree/event revision from the 8-byte big-endian `BLOB`
/// migration `003`'s `CHECK (typeof(revision) = 'blob' AND length(revision)
/// = 8)` constraint guarantees on every stored value.
pub fn decode_revision(blob: &[u8]) -> Result<u64> {
    let bytes: [u8; 8] = blob.try_into().map_err(|_| {
        anyhow::anyhow!("revision blob must be exactly 8 bytes, got {}", blob.len())
    })?;
    Ok(u64::from_be_bytes(bytes))
}

/// Encodes an [`ActorKind`] as the `mutation_trace_scopes.actor_kind` `TEXT`
/// value migration `003`'s `CHECK (actor_kind IN (...))` allow-list expects.
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
/// value migration `003`'s `CHECK (status IN (...))` allow-list expects.
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
/// `mutation_trace_events.attribution_kind` `TEXT` value migration `003`'s
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
/// `TEXT` value migration `003`'s `CHECK (boundary_kind IN (...))`
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

/// Bounded read access to the durable mutation-cursor protocol state for one
/// repository, via [`RepositoryAgentTraceDb`]. Write/CAS-commit access is
/// added by later tasks (T04/T06/T07).
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

    fn load_scope(&self, scope_id: &ScopeId) -> Result<Option<ScopeState>> {
        let rows = self.db.query_map(
            SELECT_SCOPE_BY_ID_SQL,
            (scope_id.0.as_str(),),
            scope_row_from_turso,
        )?;

        Ok(rows.into_iter().next().map(|(_, scope_state)| scope_state))
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
    use super::*;
    use crate::services::mutation_trace::types::{EventId, ScopeId};

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

    fn unique_test_db_path(label: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "sce-mutation-trace-store-{label}-{}-{nonce}",
                std::process::id()
            ))
            .join("agent-trace.db")
    }

    fn remove_test_db(db_path: &std::path::Path) {
        if let Some(parent) = db_path.parent() {
            std::fs::remove_dir_all(parent).expect("test DB directory should be removed");
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
        let db_path = unique_test_db_path("missing-worktree");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        let projection = store
            .load_worktree(&WorktreeId("wt-missing".to_string()), None, None)
            .expect("load_worktree should succeed");
        assert!(projection.is_none());

        remove_test_db(&db_path);
    }

    #[test]
    fn load_worktree_with_no_scope_or_event_key_loads_only_active_scopes() {
        let db_path = unique_test_db_path("case-1-active-only");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn load_worktree_with_explicit_scope_includes_it_regardless_of_status() {
        let db_path = unique_test_db_path("case-2-explicit-scope");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn load_worktree_with_explicit_scope_on_another_worktree_errors() {
        let db_path = unique_test_db_path("case-2-wrong-worktree");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn load_worktree_with_explicit_missing_scope_errors() {
        let db_path = unique_test_db_path("case-2-missing-scope");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn load_worktree_with_only_event_key_loads_its_scope_and_replay_row() {
        let db_path = unique_test_db_path("case-3-event-key-only");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn load_worktree_with_event_key_scope_on_another_worktree_errors() {
        let db_path = unique_test_db_path("case-3-wrong-worktree");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn load_worktree_with_event_key_missing_scope_errors() {
        let db_path = unique_test_db_path("case-3-missing-scope");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn load_worktree_with_event_key_missing_scope_and_orphan_replay_row_errors() {
        let db_path = unique_test_db_path("case-3-orphan-replay-row");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn load_worktree_with_agreeing_scope_and_event_key_loads_it_once() {
        let db_path = unique_test_db_path("case-4-agreeing");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn load_worktree_with_disagreeing_scope_and_event_key_errors_without_loading() {
        let db_path = unique_test_db_path("case-5-disagreeing");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn load_mutation_event_returns_none_when_missing() {
        let db_path = unique_test_db_path("cold-path-missing");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let store = MutationTraceStore::new(&db);

        let event = store
            .load_mutation_event(&WorktreeId("wt-1".to_string()), 1)
            .expect("load_mutation_event should succeed");
        assert!(event.is_none());

        remove_test_db(&db_path);
    }

    #[test]
    fn load_mutation_event_reconstructs_ai_exclusive_start_event() {
        let db_path = unique_test_db_path("cold-path-ai-exclusive-start");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn load_mutation_event_reconstructs_a_flush_event_with_multiple_active_scopes() {
        let db_path = unique_test_db_path("cold-path-flush");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn into_protocol_state_carries_only_the_loaded_worktree_and_leaves_transient_fields_empty() {
        let db_path = unique_test_db_path("into-protocol-state");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn initialize_worktree_inserts_a_fresh_healthy_cursor() {
        let db_path = unique_test_db_path("init-worktree-fresh");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn initialize_worktree_never_overwrites_an_existing_cursor() {
        let db_path = unique_test_db_path("init-worktree-idempotent");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn register_scope_inserts_never_seen_when_missing() {
        let db_path = unique_test_db_path("register-scope-fresh");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn register_scope_returns_existing_state_when_worktree_and_actor_match() {
        let db_path = unique_test_db_path("register-scope-existing-match");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn register_scope_errors_on_worktree_mismatch() {
        let db_path = unique_test_db_path("register-scope-worktree-mismatch");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn register_scope_errors_on_actor_mismatch() {
        let db_path = unique_test_db_path("register-scope-actor-mismatch");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn register_scope_errors_when_worktree_does_not_exist_and_leaves_no_scope_row() {
        let db_path = unique_test_db_path("register-scope-missing-worktree-fresh");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }

    #[test]
    fn register_scope_errors_when_existing_scopes_worktree_row_is_missing() {
        let db_path = unique_test_db_path("register-scope-missing-worktree-existing");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
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

        remove_test_db(&db_path);
    }
}
