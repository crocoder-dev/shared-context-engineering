use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Context, Result};

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_db::CommitTimeCutoff;
use crate::services::db::TransactionStatement;

use super::types::{
    ActorKind, Attribution, Boundary, EventId, EventKey, FailureKind, MutationEvent, ProtocolState,
    ScopeId, ScopeState, ScopeStatus, TreeId, WorktreeId, WorktreeState,
};

pub fn encode_revision(revision: u64) -> [u8; 8] {
    revision.to_be_bytes()
}

pub fn decode_revision(blob: &[u8]) -> Result<u64> {
    let bytes: [u8; 8] = blob.try_into().map_err(|_| {
        anyhow::anyhow!("revision blob must be exactly 8 bytes, got {}", blob.len())
    })?;
    Ok(u64::from_be_bytes(bytes))
}

pub fn encode_actor_kind(actor_kind: ActorKind) -> &'static str {
    match actor_kind {
        ActorKind::ClaudeCode => "claude_code",
        ActorKind::Codex => "codex",
        ActorKind::OpenCode => "opencode",
        ActorKind::Pi => "pi",
    }
}

pub fn decode_actor_kind(value: &str) -> Result<ActorKind> {
    match value {
        "claude_code" => Ok(ActorKind::ClaudeCode),
        "codex" => Ok(ActorKind::Codex),
        "opencode" => Ok(ActorKind::OpenCode),
        "pi" => Ok(ActorKind::Pi),
        other => bail!("unrecognized actor_kind: {other:?}"),
    }
}

pub fn encode_failure_kind(failure_kind: FailureKind) -> &'static str {
    match failure_kind {
        FailureKind::Healthy => "healthy",
        FailureKind::SnapshotFailure => "snapshot_failure",
    }
}

pub fn decode_failure_kind(value: &str) -> Result<FailureKind> {
    match value {
        "healthy" => Ok(FailureKind::Healthy),
        "snapshot_failure" => Ok(FailureKind::SnapshotFailure),
        other => bail!("unrecognized failure_kind: {other:?}"),
    }
}

pub fn encode_scope_status(status: ScopeStatus) -> &'static str {
    match status {
        ScopeStatus::NeverSeen => "never_seen",
        ScopeStatus::Active => "active",
        ScopeStatus::Closed => "closed",
        ScopeStatus::Abandoned => "abandoned",
    }
}

pub fn decode_scope_status(value: &str) -> Result<ScopeStatus> {
    match value {
        "never_seen" => Ok(ScopeStatus::NeverSeen),
        "active" => Ok(ScopeStatus::Active),
        "closed" => Ok(ScopeStatus::Closed),
        "abandoned" => Ok(ScopeStatus::Abandoned),
        other => bail!("unrecognized scope status: {other:?}"),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttributionKind {
    IneligibleUnscoped,
    AiExclusive,
    AiContended,
}

pub const MUTATION_ATTRIBUTION_PAGE_SIZE: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationEventPageRow {
    pub revision: u64,
    pub before_tree: TreeId,
    pub after_tree: TreeId,
    pub failure_kind: FailureKind,
    pub attribution_kind: AttributionKind,
    pub attribution_scope_id: Option<ScopeId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationEventTrees {
    pub revision: u64,
    pub before_tree: TreeId,
    pub after_tree: TreeId,
}

pub fn attribution_kind(attribution: &Attribution) -> AttributionKind {
    match attribution {
        Attribution::IneligibleUnscoped => AttributionKind::IneligibleUnscoped,
        Attribution::AiExclusive(_) => AttributionKind::AiExclusive,
        Attribution::AiContended => AttributionKind::AiContended,
    }
}

pub fn encode_attribution_kind(kind: AttributionKind) -> &'static str {
    match kind {
        AttributionKind::IneligibleUnscoped => "ineligible_unscoped",
        AttributionKind::AiExclusive => "ai_exclusive",
        AttributionKind::AiContended => "ai_contended",
    }
}

pub fn decode_attribution_kind(value: &str) -> Result<AttributionKind> {
    match value {
        "ineligible_unscoped" => Ok(AttributionKind::IneligibleUnscoped),
        "ai_exclusive" => Ok(AttributionKind::AiExclusive),
        "ai_contended" => Ok(AttributionKind::AiContended),
        other => bail!("unrecognized attribution_kind: {other:?}"),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryKind {
    Start,
    Advance,
    Close,
    Flush,
}

pub fn boundary_kind(boundary: &Boundary) -> BoundaryKind {
    match boundary {
        Boundary::Start { .. } => BoundaryKind::Start,
        Boundary::Advance { .. } => BoundaryKind::Advance,
        Boundary::Close { .. } => BoundaryKind::Close,
        Boundary::Flush { .. } => BoundaryKind::Flush,
    }
}

pub fn encode_boundary_kind(kind: BoundaryKind) -> &'static str {
    match kind {
        BoundaryKind::Start => "start",
        BoundaryKind::Advance => "advance",
        BoundaryKind::Close => "close",
        BoundaryKind::Flush => "flush",
    }
}

#[allow(
    dead_code,
    reason = "single-event loader has no production reader; attribution history uses load_mutation_event_page"
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
    reason = "single-event loader has no production reader; attribution history uses load_mutation_event_page"
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
const SELECT_MUTATION_EVENT_PAGE_BEFORE_COMMIT_SQL: &str =
    "SELECT revision, before_tree, after_tree, tainted,
            failure_kind, attribution_kind, attribution_scope_id
     FROM mutation_trace_events
     WHERE worktree_id = ?1
       AND created_at < strftime('%Y-%m-%dT%H:%M:%fZ', ?2, 'unixepoch')
     ORDER BY revision DESC
     LIMIT ?3";
const SELECT_MUTATION_EVENT_PAGE_AFTER_BEFORE_COMMIT_SQL: &str =
    "SELECT revision, before_tree, after_tree, tainted,
            failure_kind, attribution_kind, attribution_scope_id
     FROM mutation_trace_events
     WHERE worktree_id = ?1
       AND revision < ?2
       AND created_at < strftime('%Y-%m-%dT%H:%M:%fZ', ?3, 'unixepoch')
     ORDER BY revision DESC
     LIMIT ?4";
const SELECT_LATEST_MUTATION_EVENT_REVISION_SQL: &str = "SELECT revision FROM mutation_trace_events
     WHERE worktree_id = ?1
     ORDER BY revision DESC
     LIMIT 1";
#[allow(
    dead_code,
    reason = "single-event loader has no production reader; attribution history uses load_mutation_event_page"
)]
const SELECT_MUTATION_EVENT_ACTIVE_SCOPES_SQL: &str =
    "SELECT scope_id FROM mutation_trace_event_active_scopes WHERE worktree_id = ?1 AND revision = ?2";
const SELECT_TREE_ROOTS_BY_WORKTREE_SQL: &str =
    "SELECT cursor_tree AS tree FROM mutation_trace_worktrees WHERE worktree_id = ?1
     UNION
     SELECT before_tree AS tree FROM mutation_trace_events    WHERE worktree_id = ?1
     UNION
     SELECT after_tree  AS tree FROM mutation_trace_events    WHERE worktree_id = ?1";
const SELECT_MUTATION_EVENT_TREES_SQL: &str =
    "SELECT revision, before_tree, after_tree FROM mutation_trace_events
     WHERE worktree_id = ?1
     ORDER BY revision";
const TOUCH_WORKTREE_CAS_SQL: &str = "UPDATE mutation_trace_worktrees
     SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
     WHERE worktree_id = ?1 AND revision = ?2";
const DELETE_MUTATION_EVENT_ACTIVE_SCOPES_SQL: &str =
    "DELETE FROM mutation_trace_event_active_scopes WHERE worktree_id = ?1 AND revision = ?2";
const DELETE_MUTATION_EVENT_SQL: &str =
    "DELETE FROM mutation_trace_events WHERE worktree_id = ?1 AND revision = ?2";
const SELECT_ALL_TREE_ROOTS_SQL: &str = "SELECT cursor_tree AS tree FROM mutation_trace_worktrees
     UNION
     SELECT before_tree AS tree FROM mutation_trace_events
     UNION
     SELECT after_tree  AS tree FROM mutation_trace_events";
const INSERT_WORKTREE_IF_ABSENT_SQL: &str = "INSERT INTO mutation_trace_worktrees
        (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline)
     VALUES (?1, ?2, ?3, 0, 'healthy', 0)
     ON CONFLICT (worktree_id) DO NOTHING";
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeProjection {
    pub worktree_id: WorktreeId,
    pub worktree_state: WorktreeState,
    pub scopes: BTreeMap<ScopeId, ScopeState>,
    pub processed_events: BTreeSet<EventKey>,
}

impl WorktreeProjection {
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

    #[allow(
        dead_code,
        reason = "single-event loader has no production reader; attribution history uses load_mutation_event_page"
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

    pub async fn load_mutation_event_page_before_commit(
        &self,
        worktree: &WorktreeId,
        revision_cursor: Option<u64>,
        requested_limit: usize,
        cutoff: CommitTimeCutoff,
    ) -> Result<Vec<MutationEventPageRow>> {
        let limit = limit_as_i64(requested_limit.min(MUTATION_ATTRIBUTION_PAGE_SIZE));
        let rows = match revision_cursor {
            Some(cursor) => {
                let cursor_blob = encode_revision(cursor);
                self.db
                    .query_map(
                        SELECT_MUTATION_EVENT_PAGE_AFTER_BEFORE_COMMIT_SQL,
                        (
                            worktree.0.as_str(),
                            cursor_blob.as_slice(),
                            cutoff.unix_seconds(),
                            limit,
                        ),
                        mutation_event_page_row_from_turso,
                    )
                    .await?
            }
            None => {
                self.db
                    .query_map(
                        SELECT_MUTATION_EVENT_PAGE_BEFORE_COMMIT_SQL,
                        (worktree.0.as_str(), cutoff.unix_seconds(), limit),
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

    pub async fn load_mutation_event_trees(
        &self,
        worktree: &WorktreeId,
    ) -> Result<Vec<MutationEventTrees>> {
        self.db
            .query_map(
                SELECT_MUTATION_EVENT_TREES_SQL,
                (worktree.0.as_str(),),
                mutation_event_trees_row_from_turso,
            )
            .await
    }

    pub async fn prune_mutation_events(
        &mut self,
        worktree: &WorktreeId,
        expected_revision: u64,
        revisions: &[u64],
    ) -> Result<CasResult> {
        let expected_revision_blob = encode_revision(expected_revision);
        let guard = TransactionStatement::new(
            TOUCH_WORKTREE_CAS_SQL,
            (worktree.0.as_str(), expected_revision_blob.as_slice()),
        )?;

        let revision_blobs: Vec<[u8; 8]> = revisions
            .iter()
            .map(|revision| encode_revision(*revision))
            .collect();
        let mut statements = Vec::with_capacity(revision_blobs.len() * 2);
        for revision_blob in &revision_blobs {
            statements.push(TransactionStatement::new(
                DELETE_MUTATION_EVENT_ACTIVE_SCOPES_SQL,
                (worktree.0.as_str(), revision_blob.as_slice()),
            )?);
            statements.push(
                TransactionStatement::new(
                    DELETE_MUTATION_EVENT_SQL,
                    (worktree.0.as_str(), revision_blob.as_slice()),
                )?
                .expect_rows_affected(1),
            );
        }

        let applied = self
            .db
            .execute_transactional_cas_batch(
                "prune unrecoverable mutation-trace events",
                "rerun `sce doctor --fix`",
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

    pub async fn load_all_tree_roots(&self) -> Result<BTreeSet<TreeId>> {
        let rows = self
            .db
            .query_map(SELECT_ALL_TREE_ROOTS_SQL, (), tree_root_row_from_turso)
            .await?;

        Ok(rows.into_iter().collect())
    }

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

    pub async fn load_worktree_state(
        &self,
        worktree: &WorktreeId,
    ) -> Result<Option<WorktreeState>> {
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
        reason = "single-event loader has no production reader; attribution history uses load_mutation_event_page"
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

fn mutation_event_trees_row_from_turso(row: &turso::Row) -> Result<MutationEventTrees> {
    let revision_blob: Vec<u8> = row
        .get(0)
        .context("failed to read mutation_trace_events.revision")?;
    let before_tree: String = row
        .get(1)
        .context("failed to read mutation_trace_events.before_tree")?;
    let after_tree: String = row
        .get(2)
        .context("failed to read mutation_trace_events.after_tree")?;
    Ok(MutationEventTrees {
        revision: decode_revision(&revision_blob)?,
        before_tree: TreeId(before_tree),
        after_tree: TreeId(after_tree),
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

#[allow(
    dead_code,
    reason = "single-event loader has no production reader; attribution history uses load_mutation_event_page"
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
    reason = "single-event loader has no production reader; attribution history uses load_mutation_event_page"
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
    reason = "single-event loader has no production reader; attribution history uses load_mutation_event_page"
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
