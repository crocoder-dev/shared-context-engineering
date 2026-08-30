use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use uuid::Uuid;

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::checkout::{get_or_create_checkout_id, resolve_git_dir};
use crate::services::mutation_trace::protocol;
use crate::services::mutation_trace::store::{CasResult, DurableTransition, MutationTraceStore};
use crate::services::mutation_trace::types::{
    self, ActorKind, AttemptId, Boundary, EventId, MutationEvent, ScopeId, TreeId, WorktreeId,
};

use super::external_taint::ExternalTaintMarker;
use super::git_snapshot::GitSnapshotService;
use super::worktree_lock::{acquire_inner, WorktreeLockError};

const MAX_CAS_RETRY_ATTEMPTS: u32 = 5;

const WORKTREE_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub enum RuntimeBoundary {
    Start {
        scope: ScopeId,
        event: EventId,
        actor_kind: ActorKind,
    },
    Advance {
        scope: ScopeId,
        event: EventId,
        actor_kind: ActorKind,
    },
    Close {
        scope: ScopeId,
        event: EventId,
        actor_kind: ActorKind,
    },
    Flush,
}

#[derive(Debug)]
pub struct CoordinateOutcome {
    pub worktree_id: WorktreeId,
    pub observed_tree: TreeId,
    pub revision: u64,
    pub evaluation: protocol::CommitEvaluation,
    pub mutation_event: Option<MutationEvent>,
}

/// Which pre-commit [`ExternalTaintMarker`] operation failed while coordinating a
/// boundary. Both happen **before** any protected work, so no
/// [`CoordinateOutcome`] exists yet. A marker-clear failure happens *after* a
/// durable commit and is reported through
/// [`CoordinateError::MarkerClearAfterCommit`] instead, which carries the
/// committed outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalTaintOperation {
    Inspect,
    Persist,
}

#[derive(Debug)]
pub enum CoordinateError {
    SnapshotFailure {
        persisted_taint: bool,
        source: anyhow::Error,
    },
    ScopeIdentityConflict(anyhow::Error),
    CasConflictExhausted {
        attempts: u32,
    },
    RevisionExhausted {
        worktree_id: WorktreeId,
        revision: u64,
    },
    LockAcquisition(anyhow::Error),
    /// Inspecting or persisting the worktree-local external-taint marker failed.
    /// Both operations run **before** any checkout-identity, DB, snapshot, or
    /// protocol work, so no mutation boundary has committed and there is no
    /// [`CoordinateOutcome`] to surface — the boundary is aborted fail-closed
    /// with the fence left in whatever state it was in.
    ExternalTaintMarker {
        operation: ExternalTaintOperation,
        source: anyhow::Error,
    },
    /// The mutation boundary committed successfully to the Agent Trace DB and
    /// produced a [`CoordinateOutcome`], but clearing the write-ahead
    /// external-taint marker afterwards failed. The boundary did **not** fail:
    /// `committed` carries the durable outcome (including any [`MutationEvent`])
    /// so the caller never loses it. The marker remains logically armed, so the
    /// next invocation conservatively recovers.
    MarkerClearAfterCommit {
        source: anyhow::Error,
        committed: Box<CoordinateOutcome>,
    },
    /// The caller-supplied Agent Trace DB provider returned `Err` after the
    /// external-taint marker was already armed. The marker is intentionally
    /// left in place so a later invocation treats the lost interval
    /// conservatively.
    AgentTraceDbUnavailable(anyhow::Error),
    Other(anyhow::Error),
}

impl std::fmt::Display for CoordinateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoordinateError::SnapshotFailure {
                persisted_taint,
                source,
            } => write!(
                f,
                "Git snapshot capture/pin failed (taint persisted: {persisted_taint}): {source}"
            ),
            CoordinateError::CasConflictExhausted { attempts } => {
                write!(f, "Exhausted {attempts} CAS-conflict retry attempts")
            }
            CoordinateError::RevisionExhausted {
                worktree_id,
                revision,
            } => write!(
                f,
                "Worktree {worktree_id:?} requires recovery but its revision \
                 ({revision}) cannot be advanced"
            ),
            CoordinateError::ExternalTaintMarker { operation, source } => write!(
                f,
                "External-taint marker {operation:?} operation failed before any \
                 mutation boundary committed: {source}"
            ),
            CoordinateError::MarkerClearAfterCommit { source, .. } => write!(
                f,
                "Mutation boundary committed, but clearing the external-taint \
                 marker failed: {source}"
            ),
            CoordinateError::AgentTraceDbUnavailable(source) => {
                write!(f, "Repository Agent Trace DB is unavailable: {source}")
            }
            CoordinateError::ScopeIdentityConflict(source)
            | CoordinateError::LockAcquisition(source)
            | CoordinateError::Other(source) => write!(f, "{source}"),
        }
    }
}

impl std::error::Error for CoordinateError {}

pub trait SnapshotCapture {
    fn capture(&self) -> Result<TreeId>;
    fn pin(&self, worktree_id: &WorktreeId, tree: &TreeId) -> Result<()>;
}

impl SnapshotCapture for GitSnapshotService {
    fn capture(&self) -> Result<TreeId> {
        self.capture_tree()
    }

    fn pin(&self, worktree_id: &WorktreeId, tree: &TreeId) -> Result<()> {
        self.pin_tree(worktree_id, tree)
    }
}

/// Coordinates one mutation-cursor runtime boundary end to end.
///
/// The entrypoint owns the whole protected operation: it resolves `git_dir`,
/// acquires the [`WorktreeLock`](super::worktree_lock::WorktreeLock), arms the
/// worktree-local [`ExternalTaintMarker`] write-ahead — **before** acquiring the
/// Agent Trace DB — and only then invokes the caller-supplied `open_db`
/// provider, captures a snapshot, runs the snapshot / recovery / protocol / CAS
/// pipeline, and clears the marker on complete success. Any failure after the
/// marker is armed — including `open_db` returning `Err` — leaves the marker in
/// place for the next invocation.
pub fn coordinate<P>(
    repository_root: &Path,
    boundary: &RuntimeBoundary,
    open_db: P,
) -> Result<CoordinateOutcome, CoordinateError>
where
    P: FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
{
    coordinate_inner(repository_root, boundary, open_db, || {}, |_attempt| Ok(()))
}

fn coordinate_inner<P, F, R>(
    repository_root: &Path,
    boundary: &RuntimeBoundary,
    open_db: P,
    on_lock_contention: F,
    after_recovery: R,
) -> Result<CoordinateOutcome, CoordinateError>
where
    P: FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
    F: FnOnce(),
    R: FnMut(u32) -> Result<()>,
{
    let git_dir = resolve_git_dir(repository_root).map_err(CoordinateError::Other)?;

    let _lock = acquire_inner(&git_dir, WORKTREE_LOCK_TIMEOUT, on_lock_contention)
        .map_err(lock_acquisition)?;

    let marker = ExternalTaintMarker::new(&git_dir);
    let inherited_external_taint =
        marker
            .exists()
            .map_err(|source| CoordinateError::ExternalTaintMarker {
                operation: ExternalTaintOperation::Inspect,
                source,
            })?;
    marker
        .persist()
        .map_err(|source| CoordinateError::ExternalTaintMarker {
            operation: ExternalTaintOperation::Persist,
            source,
        })?;

    let outcome = coordinate_protected(
        repository_root,
        &git_dir,
        boundary,
        open_db,
        inherited_external_taint,
        after_recovery,
    )?;

    match marker.clear() {
        Ok(()) => Ok(outcome),
        Err(source) => Err(CoordinateError::MarkerClearAfterCommit {
            source,
            committed: Box::new(outcome),
        }),
    }
}

fn coordinate_protected<P, R>(
    repository_root: &Path,
    git_dir: &Path,
    boundary: &RuntimeBoundary,
    open_db: P,
    inherited_external_taint: bool,
    after_recovery: R,
) -> Result<CoordinateOutcome, CoordinateError>
where
    P: FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
    R: FnMut(u32) -> Result<()>,
{
    let checkout_id = get_or_create_checkout_id(git_dir).map_err(CoordinateError::Other)?;
    let worktree_id = WorktreeId(checkout_id);

    let db = open_db().map_err(CoordinateError::AgentTraceDbUnavailable)?;

    let snapshot = GitSnapshotService::new(repository_root).map_err(CoordinateError::Other)?;

    coordinate_boundary_inner(
        &db,
        &snapshot,
        &worktree_id,
        boundary,
        inherited_external_taint,
        |_attempt| {},
        after_recovery,
    )
}

fn lock_acquisition(error: WorktreeLockError) -> CoordinateError {
    CoordinateError::LockAcquisition(anyhow::Error::new(error))
}

#[cfg(test)]
fn coordinate_boundary<C: SnapshotCapture>(
    db: &RepositoryAgentTraceDb,
    capture: &C,
    worktree_id: &WorktreeId,
    boundary: &RuntimeBoundary,
    inherited_external_taint: bool,
) -> Result<CoordinateOutcome, CoordinateError> {
    coordinate_boundary_inner(
        db,
        capture,
        worktree_id,
        boundary,
        inherited_external_taint,
        |_attempt| {},
        |_attempt| Ok(()),
    )
}

fn coordinate_boundary_inner<C, AfterLoad, AfterRecovery>(
    db: &RepositoryAgentTraceDb,
    capture: &C,
    worktree_id: &WorktreeId,
    boundary: &RuntimeBoundary,
    inherited_external_taint: bool,
    mut after_load: AfterLoad,
    mut after_recovery: AfterRecovery,
) -> Result<CoordinateOutcome, CoordinateError>
where
    C: SnapshotCapture,
    AfterLoad: FnMut(u32),
    AfterRecovery: FnMut(u32) -> Result<()>,
{
    let store = MutationTraceStore::new(db);

    let observed_tree = match capture.capture().and_then(|tree| {
        capture.pin(worktree_id, &tree)?;
        Ok(tree)
    }) {
        Ok(tree) => tree,
        Err(source) => return Err(handle_snapshot_failure(&store, worktree_id, source)),
    };

    store
        .initialize_worktree(worktree_id, &observed_tree)
        .map_err(CoordinateError::Other)?;

    if let Some((scope, actor_kind)) = hook_identity(boundary) {
        store
            .register_scope(scope, worktree_id, actor_kind)
            .map_err(CoordinateError::ScopeIdentityConflict)?;
    }

    let type_boundary = into_protocol_boundary(boundary, worktree_id);
    let scope_ref = types::boundary_scope(&type_boundary);
    let event_key_ref = types::boundary_event_key(&type_boundary);

    let mut external_taint_pending = inherited_external_taint;

    for attempt_index in 0..MAX_CAS_RETRY_ATTEMPTS {
        let Some(projection) = store
            .load_worktree(worktree_id, scope_ref.as_ref(), event_key_ref.as_ref())
            .map_err(CoordinateError::Other)?
        else {
            return Err(CoordinateError::Other(anyhow::anyhow!(
                "worktree {worktree_id:?} missing durable state immediately after initialize_worktree"
            )));
        };

        after_load(attempt_index);

        let mut state = projection.into_protocol_state();

        if external_taint_pending {
            state = protocol::database_failure(&state, worktree_id);
        }

        if needs_recovery(&state, worktree_id) {
            let recovered = protocol::recover(&state, worktree_id, observed_tree.clone());
            let Some(transition) = DurableTransition::between(&state, &recovered, worktree_id)
                .map_err(CoordinateError::Other)?
            else {
                let revision = state
                    .worktrees
                    .get(worktree_id)
                    .map(|worktree_state| worktree_state.revision)
                    .unwrap_or_default();
                return Err(CoordinateError::RevisionExhausted {
                    worktree_id: worktree_id.clone(),
                    revision,
                });
            };

            match store.commit(&transition).map_err(CoordinateError::Other)? {
                CasResult::Applied => {
                    state = recovered;
                    external_taint_pending = false;
                    after_recovery(attempt_index).map_err(CoordinateError::Other)?;
                }
                CasResult::Conflict => continue,
            }
        }

        let attempt = AttemptId(Uuid::new_v4().to_string());
        let prepared = protocol::prepare(
            &state,
            attempt.clone(),
            type_boundary.clone(),
            observed_tree.clone(),
        );
        let outcome = protocol::commit(&prepared, &attempt);

        match DurableTransition::between(&state, &outcome.state, worktree_id)
            .map_err(CoordinateError::Other)?
        {
            Some(transition) => match store.commit(&transition).map_err(CoordinateError::Other)? {
                CasResult::Applied => {
                    return Ok(build_outcome(worktree_id, observed_tree, &outcome))
                }
                CasResult::Conflict => {}
            },
            None => return Ok(build_outcome(worktree_id, observed_tree, &outcome)),
        }
    }

    Err(CoordinateError::CasConflictExhausted {
        attempts: MAX_CAS_RETRY_ATTEMPTS,
    })
}

fn needs_recovery(state: &types::ProtocolState, worktree_id: &WorktreeId) -> bool {
    state.external_taint.contains(worktree_id)
        || state
            .worktrees
            .get(worktree_id)
            .is_some_and(|worktree_state| worktree_state.tainted || worktree_state.needs_rebaseline)
}

fn build_outcome(
    worktree_id: &WorktreeId,
    observed_tree: TreeId,
    outcome: &protocol::CommitOutcome,
) -> CoordinateOutcome {
    let revision = outcome
        .state
        .worktrees
        .get(worktree_id)
        .map(|worktree_state| worktree_state.revision)
        .expect("the coordinated worktree's durable state must still be present after commit");
    let mutation_event = outcome.state.mutation_events.iter().next().cloned();

    CoordinateOutcome {
        worktree_id: worktree_id.clone(),
        observed_tree,
        revision,
        evaluation: outcome.evaluation,
        mutation_event,
    }
}

fn into_protocol_boundary(boundary: &RuntimeBoundary, worktree_id: &WorktreeId) -> Boundary {
    match boundary {
        RuntimeBoundary::Start { scope, event, .. } => Boundary::Start {
            scope: scope.clone(),
            event: event.clone(),
        },
        RuntimeBoundary::Advance { scope, event, .. } => Boundary::Advance {
            scope: scope.clone(),
            event: event.clone(),
        },
        RuntimeBoundary::Close { scope, event, .. } => Boundary::Close {
            scope: scope.clone(),
            event: event.clone(),
        },
        RuntimeBoundary::Flush => Boundary::Flush {
            worktree: worktree_id.clone(),
        },
    }
}

fn hook_identity(boundary: &RuntimeBoundary) -> Option<(&ScopeId, ActorKind)> {
    match boundary {
        RuntimeBoundary::Start {
            scope, actor_kind, ..
        }
        | RuntimeBoundary::Advance {
            scope, actor_kind, ..
        }
        | RuntimeBoundary::Close {
            scope, actor_kind, ..
        } => Some((scope, *actor_kind)),
        RuntimeBoundary::Flush => None,
    }
}

fn handle_snapshot_failure(
    store: &MutationTraceStore<'_>,
    worktree_id: &WorktreeId,
    source: anyhow::Error,
) -> CoordinateError {
    match run_taint_retry_loop(store, worktree_id) {
        Ok(persisted_taint) => CoordinateError::SnapshotFailure {
            persisted_taint,
            source,
        },
        Err(db_err) => CoordinateError::Other(db_err),
    }
}

fn run_taint_retry_loop(store: &MutationTraceStore<'_>, worktree_id: &WorktreeId) -> Result<bool> {
    run_taint_retry_loop_inner(store, worktree_id, |_attempt| {})
}

fn run_taint_retry_loop_inner<F>(
    store: &MutationTraceStore<'_>,
    worktree_id: &WorktreeId,
    mut after_load: F,
) -> Result<bool>
where
    F: FnMut(u32),
{
    for attempt in 0..MAX_CAS_RETRY_ATTEMPTS {
        let Some(projection) = store.load_worktree(worktree_id, None, None)? else {
            return Ok(false);
        };
        after_load(attempt);

        let state = projection.into_protocol_state();
        let tainted_state = protocol::taint(&state, worktree_id);
        match DurableTransition::between(&state, &tainted_state, worktree_id)? {
            None => {
                let currently_tainted = state
                    .worktrees
                    .get(worktree_id)
                    .is_some_and(|worktree_state| worktree_state.tainted);
                return Ok(currently_tainted);
            }
            Some(transition) => match store.commit(&transition)? {
                CasResult::Applied => return Ok(true),
                CasResult::Conflict => {}
            },
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::thread;

    use super::*;
    use crate::services::mutation_trace::store::encode_revision;
    use crate::services::mutation_trace::types::{Attribution, FailureKind, ScopeStatus};

    static NEXT_TEST_DB_ID: AtomicU64 = AtomicU64::new(0);

    fn unique_test_db_path(label: &str) -> std::path::PathBuf {
        let id = NEXT_TEST_DB_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir()
            .join(format!(
                "sce-mutation-trace-coordinator-{label}-{}-{id}",
                std::process::id()
            ))
            .join("agent-trace.db")
    }

    fn test_db(label: &str) -> (RepositoryAgentTraceDb, std::path::PathBuf) {
        let db_path = unique_test_db_path(label);
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test db should open");
        (db, db_path)
    }

    fn remove_test_db(db_path: &std::path::Path) {
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }

    fn unique_test_repo(label: &str) -> std::path::PathBuf {
        let id = NEXT_TEST_DB_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "sce-mutation-trace-coordinator-repo-{label}-{}-{id}",
            std::process::id()
        ))
    }

    fn init_repo(repo_root: &std::path::Path) {
        std::fs::create_dir_all(repo_root).expect("repo root should be created");
        run_git(repo_root, &["init", "--quiet"]);
        run_git(repo_root, &["config", "user.email", "test@example.com"]);
        run_git(repo_root, &["config", "user.name", "Test"]);
    }

    fn run_git(repo_root: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .output()
            .expect("git command should spawn");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn remove_test_repo(repo_root: &std::path::Path) {
        let _ = std::fs::remove_dir_all(repo_root);
    }

    enum FakeOutcome {
        Succeed(TreeId),
        Fail(String),
    }

    struct FakeSnapshotCapture {
        outcomes: RefCell<VecDeque<FakeOutcome>>,
        default_tree: TreeId,
        capture_calls: Cell<u32>,
        pin_calls: Cell<u32>,
    }

    impl FakeSnapshotCapture {
        fn new(default_tree: TreeId) -> Self {
            Self {
                outcomes: RefCell::new(VecDeque::new()),
                default_tree,
                capture_calls: Cell::new(0),
                pin_calls: Cell::new(0),
            }
        }

        fn push_success(&self, tree: TreeId) {
            self.outcomes
                .borrow_mut()
                .push_back(FakeOutcome::Succeed(tree));
        }

        fn push_failure(&self, message: &str) {
            self.outcomes
                .borrow_mut()
                .push_back(FakeOutcome::Fail(message.to_string()));
        }

        fn capture_call_count(&self) -> u32 {
            self.capture_calls.get()
        }

        fn pin_call_count(&self) -> u32 {
            self.pin_calls.get()
        }
    }

    impl SnapshotCapture for FakeSnapshotCapture {
        fn capture(&self) -> Result<TreeId> {
            self.capture_calls.set(self.capture_calls.get() + 1);
            match self.outcomes.borrow_mut().pop_front() {
                Some(FakeOutcome::Succeed(tree)) => Ok(tree),
                Some(FakeOutcome::Fail(message)) => Err(anyhow::anyhow!(message)),
                None => Ok(self.default_tree.clone()),
            }
        }

        fn pin(&self, _worktree_id: &WorktreeId, _tree: &TreeId) -> Result<()> {
            self.pin_calls.set(self.pin_calls.get() + 1);
            Ok(())
        }
    }

    struct HookedFailingCapture<'a> {
        message: String,
        hook: RefCell<Option<Box<dyn FnMut() + 'a>>>,
    }

    impl<'a> HookedFailingCapture<'a> {
        fn new(message: &str, hook: impl FnMut() + 'a) -> Self {
            Self {
                message: message.to_string(),
                hook: RefCell::new(Some(Box::new(hook))),
            }
        }
    }

    impl SnapshotCapture for HookedFailingCapture<'_> {
        fn capture(&self) -> Result<TreeId> {
            if let Some(hook) = self.hook.borrow_mut().as_mut() {
                hook();
            }
            Err(anyhow::anyhow!(self.message.clone()))
        }

        fn pin(&self, _worktree_id: &WorktreeId, _tree: &TreeId) -> Result<()> {
            Ok(())
        }
    }

    fn commit_competing_advance(
        store: &MutationTraceStore<'_>,
        worktree: &WorktreeId,
        scope: &ScopeId,
        event_id: &str,
        first: bool,
    ) {
        if first {
            store
                .register_scope(scope, worktree, ActorKind::ClaudeCode)
                .expect("competing register_scope should succeed");
        }

        let loaded = store
            .load_worktree(worktree, Some(scope), None)
            .expect("competing load should succeed")
            .expect("worktree should already be materialized")
            .into_protocol_state();
        let cursor_tree = loaded.worktrees[worktree].cursor_tree.clone();
        let boundary = if first {
            Boundary::Start {
                scope: scope.clone(),
                event: EventId(event_id.to_string()),
            }
        } else {
            Boundary::Advance {
                scope: scope.clone(),
                event: EventId(event_id.to_string()),
            }
        };
        let attempt_id = AttemptId(format!("competing-{event_id}"));
        let prepared = protocol::prepare(&loaded, attempt_id.clone(), boundary, cursor_tree);
        let outcome = protocol::commit(&prepared, &attempt_id);
        let transition = DurableTransition::between(&loaded, &outcome.state, worktree)
            .expect("diff should succeed")
            .expect("each competing advance should durably bump the revision");
        assert_eq!(
            store
                .commit(&transition)
                .expect("competing commit should not error"),
            CasResult::Applied,
            "the competing commit should land before this iteration's own commit"
        );
    }

    fn insert_worktree_at_revision(
        db: &RepositoryAgentTraceDb,
        worktree_id: &str,
        cursor_tree: &str,
        revision: u64,
        tainted: bool,
        failure_kind: &str,
        needs_rebaseline: bool,
    ) {
        db.execute(
            "INSERT INTO mutation_trace_worktrees
                (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                worktree_id,
                cursor_tree,
                encode_revision(revision).as_slice(),
                tainted,
                failure_kind,
                needs_rebaseline,
            ),
        )
        .expect("worktree insert should succeed");
    }

    #[test]
    fn first_observation_establishes_baseline_without_evidence() {
        let (db, db_path) = test_db("ac1-first-observation");
        let worktree = WorktreeId("wt-1".to_string());
        let capture = FakeSnapshotCapture::new(TreeId("tree-a".to_string()));

        let outcome = coordinate_boundary(&db, &capture, &worktree, &RuntimeBoundary::Flush, false)
            .expect("first observation should succeed");

        assert_eq!(outcome.observed_tree, TreeId("tree-a".to_string()));
        assert_eq!(
            outcome.revision, 0,
            "a Flush that observes no change from the freshly established baseline must not advance the revision"
        );
        assert!(
            outcome.mutation_event.is_none(),
            "no mutation evidence should be emitted for the worktree's first observation"
        );
        assert_eq!(capture.capture_call_count(), 1);
        assert_eq!(capture.pin_call_count(), 1);

        remove_test_db(&db_path);
    }

    #[test]
    fn exclusive_edit_between_start_and_advance_commits_one_event() {
        let (db, db_path) = test_db("ac2-exclusive-edit");
        let worktree = WorktreeId("wt-1".to_string());
        let scope = ScopeId("scope-1".to_string());
        let capture = FakeSnapshotCapture::new(TreeId("tree-a".to_string()));

        coordinate_boundary(
            &db,
            &capture,
            &worktree,
            &RuntimeBoundary::Start {
                scope: scope.clone(),
                event: EventId("evt-start".to_string()),
                actor_kind: ActorKind::ClaudeCode,
            },
            false,
        )
        .expect("start should succeed");

        capture.push_success(TreeId("tree-b".to_string()));
        let outcome = coordinate_boundary(
            &db,
            &capture,
            &worktree,
            &RuntimeBoundary::Advance {
                scope: scope.clone(),
                event: EventId("evt-advance".to_string()),
                actor_kind: ActorKind::ClaudeCode,
            },
            false,
        )
        .expect("advance should succeed");

        let event = outcome
            .mutation_event
            .expect("an edit observed between Start and Advance should commit exactly one event");
        assert_eq!(event.before_tree, TreeId("tree-a".to_string()));
        assert_eq!(event.after_tree, TreeId("tree-b".to_string()));
        assert_eq!(event.attribution, Attribution::AiExclusive(scope));

        remove_test_db(&db_path);
    }

    #[test]
    fn replaying_the_same_scope_event_key_does_not_duplicate_evidence() {
        let (db, db_path) = test_db("ac3-replay");
        let worktree = WorktreeId("wt-1".to_string());
        let scope = ScopeId("scope-1".to_string());
        let capture = FakeSnapshotCapture::new(TreeId("tree-a".to_string()));

        coordinate_boundary(
            &db,
            &capture,
            &worktree,
            &RuntimeBoundary::Start {
                scope: scope.clone(),
                event: EventId("evt-start".to_string()),
                actor_kind: ActorKind::ClaudeCode,
            },
            false,
        )
        .expect("start should succeed");

        let advance = RuntimeBoundary::Advance {
            scope: scope.clone(),
            event: EventId("evt-advance".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        };

        capture.push_success(TreeId("tree-b".to_string()));
        let first = coordinate_boundary(&db, &capture, &worktree, &advance, false)
            .expect("first advance should commit");
        assert!(first.mutation_event.is_some());

        capture.push_success(TreeId("tree-b".to_string()));
        let replay = coordinate_boundary(&db, &capture, &worktree, &advance, false).expect(
            "replaying the identical (scope, event) boundary must be a no-op, not an error",
        );
        assert!(
            replay.mutation_event.is_none(),
            "replay must not duplicate mutation evidence"
        );
        assert_eq!(
            replay.revision, first.revision,
            "replay must not advance the revision beyond the original commit"
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn close_boundary_attributes_using_pre_close_scope_set() {
        let (db, db_path) = test_db("ac4-close-attribution");
        let worktree = WorktreeId("wt-1".to_string());
        let scope = ScopeId("scope-1".to_string());
        let capture = FakeSnapshotCapture::new(TreeId("tree-a".to_string()));

        coordinate_boundary(
            &db,
            &capture,
            &worktree,
            &RuntimeBoundary::Start {
                scope: scope.clone(),
                event: EventId("evt-start".to_string()),
                actor_kind: ActorKind::ClaudeCode,
            },
            false,
        )
        .expect("start should succeed");

        capture.push_success(TreeId("tree-b".to_string()));
        let outcome = coordinate_boundary(
            &db,
            &capture,
            &worktree,
            &RuntimeBoundary::Close {
                scope: scope.clone(),
                event: EventId("evt-close".to_string()),
                actor_kind: ActorKind::ClaudeCode,
            },
            false,
        )
        .expect("close should succeed");

        let event = outcome
            .mutation_event
            .expect("a real tree change observed at Close should still commit an event");
        assert_eq!(
            event.attribution,
            Attribution::AiExclusive(scope.clone()),
            "the Close boundary's own emitted event must still attribute to the scope it is about to close"
        );

        let store = MutationTraceStore::new(&db);
        let projection = store
            .load_worktree(&worktree, Some(&scope), None)
            .expect("load should succeed")
            .expect("worktree should exist");
        assert_eq!(
            projection.scopes.get(&scope).map(|s| s.status),
            Some(ScopeStatus::Closed)
        );

        remove_test_db(&db_path);
    }

    fn assert_contended_attribution(label: &str, actor_a: ActorKind, actor_b: ActorKind) {
        let (db, db_path) = test_db(label);
        let worktree = WorktreeId("wt-1".to_string());
        let capture = FakeSnapshotCapture::new(TreeId("tree-a".to_string()));

        coordinate_boundary(
            &db,
            &capture,
            &worktree,
            &RuntimeBoundary::Start {
                scope: ScopeId("scope-a".to_string()),
                event: EventId("evt-start-a".to_string()),
                actor_kind: actor_a,
            },
            false,
        )
        .expect("starting scope a should succeed");
        coordinate_boundary(
            &db,
            &capture,
            &worktree,
            &RuntimeBoundary::Start {
                scope: ScopeId("scope-b".to_string()),
                event: EventId("evt-start-b".to_string()),
                actor_kind: actor_b,
            },
            false,
        )
        .expect("starting scope b should succeed");

        capture.push_success(TreeId("tree-b".to_string()));
        let outcome = coordinate_boundary(
            &db,
            &capture,
            &worktree,
            &RuntimeBoundary::Advance {
                scope: ScopeId("scope-a".to_string()),
                event: EventId("evt-advance-a".to_string()),
                actor_kind: actor_a,
            },
            false,
        )
        .expect("advance should succeed");

        let event = outcome
            .mutation_event
            .expect("a real tree change with two live scopes should commit an event");
        assert_eq!(event.attribution, Attribution::AiContended);

        remove_test_db(&db_path);
    }

    #[test]
    fn contended_scopes_yield_ai_contended_same_and_different_actor() {
        assert_contended_attribution(
            "ac5-same-actor",
            ActorKind::ClaudeCode,
            ActorKind::ClaudeCode,
        );
        assert_contended_attribution(
            "ac5-different-actor",
            ActorKind::ClaudeCode,
            ActorKind::Codex,
        );
    }

    #[test]
    fn cas_conflict_reloads_and_recomputes_without_a_second_snapshot() {
        const WRITERS: usize = 3;

        let db_path = unique_test_db_path("ac8-cas-conflict");
        let worktree = WorktreeId("wt-1".to_string());

        {
            let db = RepositoryAgentTraceDb::new_at(&db_path).expect("test db should open");
            let bootstrap_capture = FakeSnapshotCapture::new(TreeId("tree-a".to_string()));
            coordinate_boundary(
                &db,
                &bootstrap_capture,
                &worktree,
                &RuntimeBoundary::Flush,
                false,
            )
            .expect("baseline flush should succeed");
        }

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(WRITERS));
        let mut handles = Vec::new();
        for i in 0..WRITERS {
            let db_path = db_path.clone();
            let worktree = worktree.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let db = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
                    .expect("writer handle should open");
                let capture = FakeSnapshotCapture::new(TreeId(format!("tree-writer-{i}")));
                barrier.wait();
                let outcome =
                    coordinate_boundary(&db, &capture, &worktree, &RuntimeBoundary::Flush, false)
                        .expect(
                            "each racing writer should eventually succeed after reload+recompute",
                        );
                (
                    outcome.revision,
                    capture.capture_call_count(),
                    capture.pin_call_count(),
                )
            }));
        }

        let mut revisions = Vec::new();
        for handle in handles {
            let (revision, captures, pins) = handle.join().expect("writer thread should not panic");
            assert_eq!(
                captures, 1,
                "each invocation must capture its Git snapshot exactly once, even across CAS retries"
            );
            assert_eq!(
                pins, 1,
                "each invocation must pin exactly once, even across CAS retries"
            );
            revisions.push(revision);
        }

        revisions.sort_unstable();
        revisions.dedup();
        assert_eq!(
            revisions.len(),
            WRITERS,
            "each racing writer's commit must land at a distinct revision, proving reload+recompute rather than a lost update"
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn recovers_from_needs_rebaseline_preserving_live_scopes() {
        let (db, db_path) = test_db("ac10-needs-rebaseline");
        let worktree = WorktreeId("wt-1".to_string());
        let live_scope = ScopeId("scope-live".to_string());
        let abandoned_scope = ScopeId("scope-abandoned".to_string());
        let capture = FakeSnapshotCapture::new(TreeId("tree-a".to_string()));

        coordinate_boundary(
            &db,
            &capture,
            &worktree,
            &RuntimeBoundary::Start {
                scope: live_scope.clone(),
                event: EventId("evt-start-live".to_string()),
                actor_kind: ActorKind::ClaudeCode,
            },
            false,
        )
        .expect("starting the live scope should succeed");
        coordinate_boundary(
            &db,
            &capture,
            &worktree,
            &RuntimeBoundary::Start {
                scope: abandoned_scope.clone(),
                event: EventId("evt-start-abandoned".to_string()),
                actor_kind: ActorKind::ClaudeCode,
            },
            false,
        )
        .expect("starting the to-be-abandoned scope should succeed");

        let store = MutationTraceStore::new(&db);
        let state = store
            .load_worktree(&worktree, Some(&abandoned_scope), None)
            .expect("load should succeed")
            .expect("worktree should exist")
            .into_protocol_state();
        let abandoned_state = protocol::abandon(&state, &abandoned_scope);
        let transition = DurableTransition::between(&state, &abandoned_state, &worktree)
            .expect("diff should succeed")
            .expect("abandon should durably change state");
        assert_eq!(
            store
                .commit(&transition)
                .expect("abandon commit should not error"),
            CasResult::Applied
        );

        let before_recovery = store
            .load_worktree(&worktree, None, None)
            .expect("load should succeed")
            .expect("worktree should exist");
        assert!(before_recovery.worktree_state.needs_rebaseline);

        capture.push_success(TreeId("tree-b".to_string()));
        let outcome = coordinate_boundary(
            &db,
            &capture,
            &worktree,
            &RuntimeBoundary::Advance {
                scope: live_scope.clone(),
                event: EventId("evt-advance-live".to_string()),
                actor_kind: ActorKind::ClaudeCode,
            },
            false,
        )
        .expect("advance should trigger needs_rebaseline recovery first, then succeed");
        assert!(
            outcome.mutation_event.is_none(),
            "no evidence should be emitted for the just-discarded interval recovery rebaselined away"
        );

        let after = store
            .load_worktree(&worktree, Some(&live_scope), None)
            .expect("load should succeed")
            .expect("worktree should exist");
        assert!(
            !after.worktree_state.needs_rebaseline,
            "recovery should clear needs_rebaseline"
        );
        assert_eq!(
            after.worktree_state.cursor_tree,
            TreeId("tree-b".to_string()),
            "recovery should rebaseline the cursor to the recovering invocation's own observed tree"
        );
        assert_eq!(
            after.scopes.get(&live_scope).map(|s| s.status),
            Some(ScopeStatus::Active),
            "needs_rebaseline recovery must preserve live scopes"
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn recovers_from_snapshot_failure_taint_abandoning_live_scopes() {
        let (db, db_path) = test_db("ac10-taint-recovery");
        let worktree = WorktreeId("wt-1".to_string());
        let live_scope = ScopeId("scope-live".to_string());
        let capture = FakeSnapshotCapture::new(TreeId("tree-a".to_string()));

        coordinate_boundary(
            &db,
            &capture,
            &worktree,
            &RuntimeBoundary::Start {
                scope: live_scope.clone(),
                event: EventId("evt-start".to_string()),
                actor_kind: ActorKind::ClaudeCode,
            },
            false,
        )
        .expect("start should succeed");

        let store = MutationTraceStore::new(&db);
        let state = store
            .load_worktree(&worktree, None, None)
            .expect("load should succeed")
            .expect("worktree should exist")
            .into_protocol_state();
        let tainted_state = protocol::taint(&state, &worktree);
        let transition = DurableTransition::between(&state, &tainted_state, &worktree)
            .expect("diff should succeed")
            .expect("taint should durably change state");
        assert_eq!(
            store
                .commit(&transition)
                .expect("taint commit should not error"),
            CasResult::Applied
        );

        let before_recovery = store
            .load_worktree(&worktree, None, None)
            .expect("load should succeed")
            .expect("worktree should exist");
        assert!(before_recovery.worktree_state.tainted);

        capture.push_success(TreeId("tree-b".to_string()));
        coordinate_boundary(
            &db,
            &capture,
            &worktree,
            &RuntimeBoundary::Advance {
                scope: live_scope.clone(),
                event: EventId("evt-advance".to_string()),
                actor_kind: ActorKind::ClaudeCode,
            },
            false,
        )
        .expect("advance should trigger taint recovery first, then succeed");

        let after = store
            .load_worktree(&worktree, Some(&live_scope), None)
            .expect("load should succeed")
            .expect("worktree should exist");
        assert!(
            !after.worktree_state.tainted,
            "recovery should clear the taint"
        );
        assert_eq!(
            after.scopes.get(&live_scope).map(|s| s.status),
            Some(ScopeStatus::Abandoned),
            "taint recovery must abandon live scopes, unlike needs_rebaseline recovery"
        );

        remove_test_db(&db_path);
    }

    fn assert_recovery_at_revision_exhaustion_is_rejected(
        label: &str,
        tainted: bool,
        failure_kind: &str,
        needs_rebaseline: bool,
    ) {
        let (db, db_path) = test_db(label);
        let worktree = WorktreeId("wt-1".to_string());
        let scope = ScopeId("scope-1".to_string());
        let event = EventId("evt-start".to_string());

        insert_worktree_at_revision(
            &db,
            &worktree.0,
            "tree-a",
            u64::MAX,
            tainted,
            failure_kind,
            needs_rebaseline,
        );

        let capture = FakeSnapshotCapture::new(TreeId("tree-b".to_string()));
        let error = coordinate_boundary(
            &db,
            &capture,
            &worktree,
            &RuntimeBoundary::Start {
                scope: scope.clone(),
                event: event.clone(),
                actor_kind: ActorKind::ClaudeCode,
            },
            false,
        )
        .expect_err("recovery that cannot advance revision must reject the triggering boundary");

        match error {
            CoordinateError::RevisionExhausted {
                worktree_id,
                revision,
            } => {
                assert_eq!(worktree_id, worktree);
                assert_eq!(revision, u64::MAX);
            }
            other => panic!("expected RevisionExhausted, got {other:?}"),
        }

        let store = MutationTraceStore::new(&db);
        let projection = store
            .load_worktree(&worktree, Some(&scope), None)
            .expect("load should succeed")
            .expect("worktree should exist");
        assert_eq!(projection.worktree_state.revision, u64::MAX);
        assert_eq!(projection.worktree_state.tainted, tainted);
        assert_eq!(projection.worktree_state.needs_rebaseline, needs_rebaseline);
        assert_eq!(
            projection.worktree_state.cursor_tree,
            TreeId("tree-a".to_string()),
            "the cursor must never move when the triggering boundary was never evaluated"
        );
        assert_eq!(
            projection.scopes.get(&scope).map(|s| s.status),
            Some(ScopeStatus::NeverSeen),
            "the triggering boundary must never have transitioned the scope"
        );
        assert!(
            projection.processed_events.is_empty(),
            "the triggering boundary's event must never have been recorded as processed"
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn mandatory_recovery_that_cannot_advance_revision_rejects_the_triggering_boundary() {
        assert_recovery_at_revision_exhaustion_is_rejected(
            "revision-exhausted-tainted",
            true,
            "snapshot_failure",
            false,
        );
        assert_recovery_at_revision_exhaustion_is_rejected(
            "revision-exhausted-needs-rebaseline",
            false,
            "healthy",
            true,
        );
    }

    #[test]
    fn snapshot_failure_taints_an_existing_worktree() {
        let (db, db_path) = test_db("ac11-taints-existing");
        let worktree = WorktreeId("wt-1".to_string());
        let bootstrap = FakeSnapshotCapture::new(TreeId("tree-a".to_string()));
        coordinate_boundary(&db, &bootstrap, &worktree, &RuntimeBoundary::Flush, false)
            .expect("baseline flush should materialize the worktree");

        let failing = FakeSnapshotCapture::new(TreeId("tree-b".to_string()));
        failing.push_failure("simulated git snapshot failure");

        let error = coordinate_boundary(&db, &failing, &worktree, &RuntimeBoundary::Flush, false)
            .expect_err("a capture failure against an existing worktree should be reported");
        match error {
            CoordinateError::SnapshotFailure {
                persisted_taint, ..
            } => assert!(persisted_taint),
            other => panic!("expected SnapshotFailure, got {other:?}"),
        }

        let store = MutationTraceStore::new(&db);
        let projection = store
            .load_worktree(&worktree, None, None)
            .expect("load should succeed")
            .expect("worktree should exist");
        assert!(projection.worktree_state.tainted);
        assert_eq!(
            projection.worktree_state.failure_kind,
            FailureKind::SnapshotFailure
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn snapshot_failure_taint_survives_a_losing_cas_and_commits_on_retry() {
        let (db, db_path) = test_db("ac11-taint-retry-succeeds");
        let worktree = WorktreeId("wt-1".to_string());
        let bootstrap = FakeSnapshotCapture::new(TreeId("tree-a".to_string()));
        coordinate_boundary(&db, &bootstrap, &worktree, &RuntimeBoundary::Flush, false)
            .expect("baseline flush should materialize the worktree");

        let store = MutationTraceStore::new(&db);
        let competing_scope = ScopeId("competing-scope".to_string());
        let mut interfered = false;
        let persisted_taint = run_taint_retry_loop_inner(&store, &worktree, |attempt| {
            if attempt == 0 && !interfered {
                interfered = true;
                commit_competing_advance(
                    &store,
                    &worktree,
                    &competing_scope,
                    "competing-event-1",
                    true,
                );
            }
        })
        .expect("taint retry loop should not error");

        assert!(
            persisted_taint,
            "the taint should eventually be persisted after reloading past the losing CAS"
        );

        let projection = store
            .load_worktree(&worktree, None, None)
            .expect("load should succeed")
            .expect("worktree should exist");
        assert!(projection.worktree_state.tainted);
        assert_eq!(
            projection.worktree_state.failure_kind,
            FailureKind::SnapshotFailure
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn snapshot_failure_taint_reports_not_persisted_after_retries_are_exhausted() {
        let (db, db_path) = test_db("ac11-taint-exhaustion");
        let worktree = WorktreeId("wt-1".to_string());
        let bootstrap = FakeSnapshotCapture::new(TreeId("tree-a".to_string()));
        coordinate_boundary(&db, &bootstrap, &worktree, &RuntimeBoundary::Flush, false)
            .expect("baseline flush should materialize the worktree");

        let store = MutationTraceStore::new(&db);
        let competing_scope = ScopeId("competing-scope".to_string());
        let mut competing_calls = 0u32;
        let persisted_taint = run_taint_retry_loop_inner(&store, &worktree, |_attempt| {
            competing_calls += 1;
            commit_competing_advance(
                &store,
                &worktree,
                &competing_scope,
                &format!("competing-event-{competing_calls}"),
                competing_calls == 1,
            );
        })
        .expect("taint retry loop should not error even when every attempt conflicts");

        assert!(
            !persisted_taint,
            "taint must not be reported as persisted once every bounded attempt has conflicted"
        );

        let projection = store
            .load_worktree(&worktree, None, None)
            .expect("load should succeed")
            .expect("worktree should exist");
        assert!(
            !projection.worktree_state.tainted,
            "the worktree must remain untainted when the taint itself was never actually committed"
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn snapshot_failure_before_any_baseline_makes_no_durable_write() {
        let (db, db_path) = test_db("ac11-bootstrap-failure");
        let worktree = WorktreeId("wt-1".to_string());
        let failing = FakeSnapshotCapture::new(TreeId("unused".to_string()));
        failing.push_failure("simulated git snapshot failure before any baseline exists");

        let error = coordinate_boundary(&db, &failing, &worktree, &RuntimeBoundary::Flush, false)
            .expect_err("a capture failure with no prior worktree row should still be reported");
        match error {
            CoordinateError::SnapshotFailure {
                persisted_taint, ..
            } => assert!(!persisted_taint),
            other => panic!("expected SnapshotFailure, got {other:?}"),
        }

        let store = MutationTraceStore::new(&db);
        let projection = store
            .load_worktree(&worktree, None, None)
            .expect("load should succeed");
        assert!(
            projection.is_none(),
            "no durable worktree row should ever be created for a bootstrap-time snapshot failure"
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn snapshot_failure_taints_a_worktree_materialized_concurrently_during_capture() {
        let (db, db_path) = test_db("ac11-concurrent-materialization");
        let worktree = WorktreeId("wt-1".to_string());

        let capture = HookedFailingCapture::new(
            "simulated git snapshot failure racing a concurrent materialization",
            || {
                let store = MutationTraceStore::new(&db);
                store
                    .initialize_worktree(
                        &worktree,
                        &TreeId("tree-materialized-concurrently".to_string()),
                    )
                    .expect("concurrent materialization should succeed");
            },
        );

        let error = coordinate_boundary(&db, &capture, &worktree, &RuntimeBoundary::Flush, false)
            .expect_err(
                "a capture failure racing a concurrent materialization should still be reported",
            );
        match error {
            CoordinateError::SnapshotFailure { persisted_taint, .. } => assert!(
                persisted_taint,
                "the failure handler's fresh, post-failure read must still find and taint the concurrently materialized worktree"
            ),
            other => panic!("expected SnapshotFailure, got {other:?}"),
        }

        let store = MutationTraceStore::new(&db);
        let projection = store
            .load_worktree(&worktree, None, None)
            .expect("load should succeed")
            .expect("worktree should exist");
        assert!(projection.worktree_state.tainted);

        remove_test_db(&db_path);
    }

    #[test]
    fn two_threads_on_the_same_worktree_serialize() {
        let repo_root = unique_test_repo("t05-serialize");
        init_repo(&repo_root);
        let db_path = repo_root.join("agent-trace.db");

        let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
        let held = acquire_inner(&git_dir, Duration::from_secs(5), || {})
            .expect("the test should hold a real WorktreeLock before the worker runs");

        let (contention_tx, contention_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let repo_root_clone = repo_root.clone();
        let db_path_clone = db_path.clone();
        let worker = thread::spawn(move || {
            let outcome = coordinate_inner(
                &repo_root_clone,
                &RuntimeBoundary::Flush,
                || RepositoryAgentTraceDb::new_at(&db_path_clone),
                move || {
                    contention_tx
                        .send(())
                        .expect("contention signal channel should still be open");
                },
                |_attempt| Ok(()),
            );
            result_tx
                .send(())
                .expect("result signal channel should still be open");
            outcome
        });

        contention_rx.recv_timeout(Duration::from_secs(5)).expect(
            "coordinate() should reach the WorktreeLock try_lock loop and observe \
             TryLockError::WouldBlock while the first guard is still held",
        );

        assert!(
            result_rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "the worker's coordinate() call must not complete while the first worktree lock guard is still held"
        );

        drop(held);

        result_rx.recv_timeout(Duration::from_secs(5)).expect(
            "the same coordinate() invocation should complete once the first guard is released",
        );

        let outcome = worker
            .join()
            .expect("worker thread should not panic")
            .expect(
                "coordinate() should succeed once it can acquire the worktree lock after release",
            );
        assert_eq!(
            outcome.revision, 0,
            "the worker's first-observation flush should not advance the revision"
        );

        remove_test_repo(&repo_root);
    }

    #[test]
    fn public_coordinate_clears_marker_on_success() {
        let repo_root = unique_test_repo("t02-success-clears-marker");
        init_repo(&repo_root);
        let db_path = repo_root.join("agent-trace.db");
        RepositoryAgentTraceDb::new_at(&db_path).expect("seed db should open with schema");
        let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
        let marker = ExternalTaintMarker::new(&git_dir);

        let outcome = coordinate(&repo_root, &RuntimeBoundary::Flush, || {
            RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
        })
        .expect("a first observation should succeed");
        assert_eq!(
            outcome.revision, 0,
            "a first-observation flush should not advance the revision"
        );
        assert!(
            !marker.exists().expect("marker existence should resolve"),
            "a successful coordinate() must clear the marker it armed"
        );

        remove_test_repo(&repo_root);
    }

    #[test]
    fn public_coordinate_leaves_marker_after_a_snapshot_failure() {
        let repo_root = unique_test_repo("t02-snapshot-failure-marker");
        init_repo(&repo_root);
        let db_path = repo_root.join("agent-trace.db");
        RepositoryAgentTraceDb::new_at(&db_path).expect("seed db should open with schema");
        let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
        let marker = ExternalTaintMarker::new(&git_dir);

        coordinate(&repo_root, &RuntimeBoundary::Flush, || {
            RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
        })
        .expect("the baseline observation should succeed");
        assert!(
            !marker.exists().expect("marker existence should resolve"),
            "the successful baseline must have cleared its marker"
        );

        let tmp_index_dir = git_dir.join("sce").join("tmp");
        let _ = std::fs::remove_dir_all(&tmp_index_dir);
        std::fs::write(&tmp_index_dir, b"not a directory").expect(
            "planting a file where the snapshot service expects its temp-index directory should succeed",
        );

        let error = coordinate(&repo_root, &RuntimeBoundary::Flush, || {
            RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
        })
        .expect_err("a Git snapshot failure after marker arming should be reported");
        assert!(
            matches!(error, CoordinateError::SnapshotFailure { .. }),
            "expected SnapshotFailure, got {error:?}"
        );
        assert!(
            marker.exists().expect("marker existence should resolve"),
            "a snapshot failure after arming must leave the external-taint marker in place"
        );

        remove_test_repo(&repo_root);
    }

    #[test]
    fn public_coordinate_leaves_marker_after_a_non_snapshot_failure() {
        let repo_root = unique_test_repo("t02-non-snapshot-failure-marker");
        init_repo(&repo_root);
        let db_path = repo_root.join("agent-trace.db");
        let seed_db = RepositoryAgentTraceDb::new_at(&db_path).expect("seed db should open");
        let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
        let checkout_id = get_or_create_checkout_id(&git_dir).expect("checkout id should resolve");
        insert_worktree_at_revision(
            &seed_db,
            &checkout_id,
            "tree-a",
            u64::MAX,
            true,
            "snapshot_failure",
            false,
        );
        drop(seed_db);

        let marker = ExternalTaintMarker::new(&git_dir);
        let error = coordinate(&repo_root, &RuntimeBoundary::Flush, || {
            RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
        })
        .expect_err("mandatory recovery that cannot advance the revision must reject the boundary");
        assert!(
            matches!(error, CoordinateError::RevisionExhausted { .. }),
            "expected RevisionExhausted, got {error:?}"
        );
        assert!(
            marker.exists().expect("marker existence should resolve"),
            "a non-snapshot failure after arming must leave the external-taint marker in place"
        );

        remove_test_repo(&repo_root);
    }

    #[test]
    fn public_coordinate_fails_closed_when_the_marker_cannot_be_armed() {
        let repo_root = unique_test_repo("t02-marker-arm-failure");
        init_repo(&repo_root);
        let db_path = repo_root.join("agent-trace.db");
        RepositoryAgentTraceDb::new_at(&db_path).expect("seed db should open with schema");
        let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");

        // Plant a directory exactly where the marker file must be created, so
        // `ExternalTaintMarker::persist` fails deterministically regardless of uid.
        std::fs::create_dir_all(git_dir.join("sce").join("mutation-cursor-tainted"))
            .expect("planting a directory at the marker path should succeed");

        let provider_called = std::sync::atomic::AtomicBool::new(false);
        let error = coordinate(&repo_root, &RuntimeBoundary::Flush, || {
            provider_called.store(true, Ordering::SeqCst);
            RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
        })
        .expect_err("an unarmed marker must fail coordinate() closed");
        assert!(
            matches!(
                error,
                CoordinateError::ExternalTaintMarker {
                    operation: ExternalTaintOperation::Persist,
                    ..
                }
            ),
            "expected an ExternalTaintMarker persist failure, got {error:?}"
        );
        assert!(
            !provider_called.load(Ordering::SeqCst),
            "a marker-arming failure must return before DB-provider, snapshot, or protocol work"
        );

        remove_test_repo(&repo_root);
    }

    #[test]
    fn public_coordinate_fails_closed_when_marker_inspection_fails() {
        let repo_root = unique_test_repo("t02-marker-inspect-failure");
        init_repo(&repo_root);
        let db_path = repo_root.join("agent-trace.db");
        RepositoryAgentTraceDb::new_at(&db_path).expect("seed db should open with schema");
        let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
        let sce_dir = git_dir.join("sce");
        std::fs::create_dir_all(&sce_dir).expect("the sce directory should be creatable");

        let held = acquire_inner(&git_dir, Duration::from_secs(5), || {})
            .expect("the test should hold the runtime lock before the worker runs");

        let provider_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (contention_tx, contention_rx) = mpsc::channel();

        let worker = {
            let repo_root = repo_root.clone();
            let db_path = db_path.clone();
            let provider_called = std::sync::Arc::clone(&provider_called);
            thread::spawn(move || {
                coordinate_inner(
                    &repo_root,
                    &RuntimeBoundary::Flush,
                    || {
                        provider_called.store(true, Ordering::SeqCst);
                        RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
                    },
                    move || {
                        contention_tx
                            .send(())
                            .expect("contention signal channel should still be open");
                    },
                    |_attempt| Ok(()),
                )
            })
        };

        contention_rx.recv_timeout(Duration::from_secs(5)).expect(
            "the worker's coordinate_inner should reach the WorktreeLock contention branch",
        );

        std::fs::remove_dir_all(&sce_dir).expect("the sce directory should be removable");
        std::fs::write(&sce_dir, b"not a directory")
            .expect("planting a file where the sce directory was should succeed");

        drop(held);

        let error = worker
            .join()
            .expect("worker thread should not panic")
            .expect_err("marker inspection failure must fail coordinate() closed");
        assert!(
            matches!(
                error,
                CoordinateError::ExternalTaintMarker {
                    operation: ExternalTaintOperation::Inspect,
                    ..
                }
            ),
            "expected an ExternalTaintMarker inspect failure, got {error:?}"
        );
        assert!(
            !provider_called.load(Ordering::SeqCst),
            "marker inspection failing closed must precede DB-provider, checkout, snapshot, and protocol work"
        );

        remove_test_repo(&repo_root);
    }

    #[test]
    fn public_coordinate_leaves_marker_when_the_db_provider_fails() {
        let repo_root = unique_test_repo("t02-db-provider-failure-marker");
        init_repo(&repo_root);
        let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
        let marker = ExternalTaintMarker::new(&git_dir);

        let error = coordinate(
            &repo_root,
            &RuntimeBoundary::Flush,
            || -> anyhow::Result<RepositoryAgentTraceDb> {
                Err(anyhow::anyhow!("simulated Agent Trace DB open failure"))
            },
        )
        .expect_err("a DB provider that returns Err must fail coordinate()");
        assert!(
            matches!(error, CoordinateError::AgentTraceDbUnavailable(_)),
            "expected AgentTraceDbUnavailable, got {error:?}"
        );
        assert!(
            marker.exists().expect("marker existence should resolve"),
            "a DB-provider failure after arming must leave the external-taint marker present"
        );

        remove_test_repo(&repo_root);
    }

    #[test]
    fn inherited_external_taint_recovers_once_before_the_boundary() {
        let (db, db_path) = test_db("t03-inherited-recovers-once");
        let worktree = WorktreeId("wt-1".to_string());
        let scope = ScopeId("scope-live".to_string());
        let capture = FakeSnapshotCapture::new(TreeId("tree-a".to_string()));

        coordinate_boundary(
            &db,
            &capture,
            &worktree,
            &RuntimeBoundary::Start {
                scope: scope.clone(),
                event: EventId("evt-start".to_string()),
                actor_kind: ActorKind::ClaudeCode,
            },
            false,
        )
        .expect("start should establish the baseline and the live scope");

        let advance_capture = FakeSnapshotCapture::new(TreeId("tree-b".to_string()));
        let outcome = coordinate_boundary(
            &db,
            &advance_capture,
            &worktree,
            &RuntimeBoundary::Advance {
                scope: scope.clone(),
                event: EventId("evt-advance".to_string()),
                actor_kind: ActorKind::ClaudeCode,
            },
            true,
        )
        .expect("an inherited external-taint marker must recover, then process the boundary");

        assert!(
            outcome.mutation_event.is_none(),
            "no mutation evidence may span the interval the inherited marker fenced off"
        );
        assert_eq!(
            outcome.revision, 3,
            "one recovery transition (rev 1 -> 2), then the triggering boundary (rev 2 -> 3)"
        );
        assert_eq!(
            advance_capture.capture_call_count(),
            1,
            "recovery and the triggering boundary must share the single already-captured snapshot"
        );
        assert_eq!(advance_capture.pin_call_count(), 1);

        let store = MutationTraceStore::new(&db);
        let projection = store
            .load_worktree(&worktree, Some(&scope), None)
            .expect("load should succeed")
            .expect("worktree should exist");
        assert!(!projection.worktree_state.tainted);
        assert!(!projection.worktree_state.needs_rebaseline);
        assert_eq!(projection.worktree_state.failure_kind, FailureKind::Healthy);
        assert_eq!(
            projection.worktree_state.cursor_tree,
            TreeId("tree-b".to_string()),
            "recovery rebaselines the cursor to this invocation's observed tree"
        );
        assert_eq!(
            projection.scopes.get(&scope).map(|s| s.status),
            Some(ScopeStatus::Abandoned),
            "inherited-taint recovery abandons the live scopes the fenced interval made untrustworthy"
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn inherited_external_taint_with_no_worktree_row_baselines_without_evidence() {
        let (db, db_path) = test_db("t03-inherited-no-row");
        let worktree = WorktreeId("wt-1".to_string());
        let capture = FakeSnapshotCapture::new(TreeId("tree-a".to_string()));

        let outcome = coordinate_boundary(&db, &capture, &worktree, &RuntimeBoundary::Flush, true)
            .expect("a first-ever invocation carrying an inherited marker must still succeed");

        assert!(
            outcome.mutation_event.is_none(),
            "a worktree with no prior durable row cannot produce evidence for the unknown interval"
        );
        assert_eq!(
            outcome.revision, 1,
            "the freshly initialized worktree is baselined against the observed tree, then conservatively recovered once"
        );

        let store = MutationTraceStore::new(&db);
        let projection = store
            .load_worktree(&worktree, None, None)
            .expect("load should succeed")
            .expect("the worktree row should now exist");
        assert!(!projection.worktree_state.tainted);
        assert_eq!(projection.worktree_state.failure_kind, FailureKind::Healthy);
        assert_eq!(
            projection.worktree_state.cursor_tree,
            TreeId("tree-a".to_string())
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn a_losing_recovery_cas_reinjects_external_taint_until_it_applies() {
        let (db, db_path) = test_db("t03-recovery-cas-reinjection");
        let worktree = WorktreeId("wt-1".to_string());
        let scope = ScopeId("scope-live".to_string());
        let bootstrap = FakeSnapshotCapture::new(TreeId("tree-a".to_string()));

        coordinate_boundary(
            &db,
            &bootstrap,
            &worktree,
            &RuntimeBoundary::Start {
                scope: scope.clone(),
                event: EventId("evt-start".to_string()),
                actor_kind: ActorKind::ClaudeCode,
            },
            false,
        )
        .expect("bootstrap start should establish the baseline and live scope");

        let store = MutationTraceStore::new(&db);
        let competing_scope = ScopeId("competing-scope".to_string());
        let interfered = Cell::new(false);
        let capture = FakeSnapshotCapture::new(TreeId("tree-a".to_string()));

        let outcome = coordinate_boundary_inner(
            &db,
            &capture,
            &worktree,
            &RuntimeBoundary::Flush,
            true,
            |attempt| {
                if attempt == 0 && !interfered.get() {
                    interfered.set(true);
                    commit_competing_advance(
                        &store,
                        &worktree,
                        &competing_scope,
                        "competing-event-1",
                        true,
                    );
                }
            },
            |_attempt| Ok(()),
        )
        .expect("recovery must recompute past the losing CAS and still succeed");

        assert_eq!(
            capture.capture_call_count(),
            1,
            "a recovery CAS conflict must not trigger a second Git snapshot"
        );
        assert_eq!(
            outcome.revision, 3,
            "bootstrap start (1) + competing advance (2) + exactly one landed recovery (3)"
        );

        let projection = store
            .load_worktree(&worktree, Some(&scope), None)
            .expect("load should succeed")
            .expect("worktree should exist");
        assert!(!projection.worktree_state.tainted);
        assert_eq!(
            projection.scopes.get(&scope).map(|s| s.status),
            Some(ScopeStatus::Abandoned),
            "the re-injected recovery still abandons the fenced-off live scope"
        );

        let competing_projection = store
            .load_worktree(&worktree, Some(&competing_scope), None)
            .expect("load should succeed")
            .expect("worktree should exist");
        assert_eq!(
            competing_projection
                .scopes
                .get(&competing_scope)
                .map(|s| s.status),
            Some(ScopeStatus::Abandoned)
        );

        remove_test_db(&db_path);
    }

    #[test]
    fn a_landed_recovery_clears_the_flag_so_a_boundary_cas_retry_does_not_re_recover() {
        let (db, db_path) = test_db("t03-flag-clears-after-recovery");
        let worktree = WorktreeId("wt-1".to_string());
        let scope = ScopeId("scope-live".to_string());
        let bootstrap = FakeSnapshotCapture::new(TreeId("tree-a".to_string()));

        coordinate_boundary(
            &db,
            &bootstrap,
            &worktree,
            &RuntimeBoundary::Start {
                scope: scope.clone(),
                event: EventId("evt-start".to_string()),
                actor_kind: ActorKind::ClaudeCode,
            },
            false,
        )
        .expect("bootstrap start should establish the baseline and live scope");

        let store = MutationTraceStore::new(&db);
        let competing_scope = ScopeId("competing-scope".to_string());
        let interfered = Cell::new(false);
        let capture = FakeSnapshotCapture::new(TreeId("tree-a".to_string()));
        capture.push_success(TreeId("tree-b".to_string()));

        let outcome = coordinate_boundary_inner(
            &db,
            &capture,
            &worktree,
            &RuntimeBoundary::Advance {
                scope: scope.clone(),
                event: EventId("evt-advance".to_string()),
                actor_kind: ActorKind::ClaudeCode,
            },
            true,
            |_attempt| {},
            |attempt| {
                if attempt == 0 && !interfered.get() {
                    interfered.set(true);
                    commit_competing_advance(
                        &store,
                        &worktree,
                        &competing_scope,
                        "competing-event-1",
                        true,
                    );
                }
                Ok(())
            },
        )
        .expect("the boundary CAS retry after a landed recovery must still succeed");

        assert_eq!(
            capture.capture_call_count(),
            1,
            "neither the recovery nor the boundary retry may take a second snapshot"
        );
        assert_eq!(
            outcome.revision, 4,
            "start (1) + one landed recovery (2) + competing advance (3) + the retried boundary (4); a re-triggered recovery on the retry would land at 5"
        );

        remove_test_db(&db_path);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn a_failure_after_recovery_before_boundary_commit_leaves_marker_and_forces_later_recovery() {
        let repo_root = unique_test_repo("t-ac8-recovery-then-fail");
        init_repo(&repo_root);
        let db_path = unique_test_db_path("t-ac8-recovery-then-fail");
        RepositoryAgentTraceDb::new_at(&db_path).expect("seed db should open with schema");
        let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
        let marker = ExternalTaintMarker::new(&git_dir);
        let ok_db = || RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&db_path);

        std::fs::write(repo_root.join("work.txt"), b"a").expect("the baseline edit should write");
        let baseline = coordinate(&repo_root, &RuntimeBoundary::Flush, ok_db)
            .expect("the baseline observation should establish cursor A");
        let worktree_id = baseline.worktree_id.clone();
        let tree_a = baseline.observed_tree.clone();

        let scope = ScopeId("scope-live".to_string());
        coordinate(
            &repo_root,
            &RuntimeBoundary::Start {
                scope: scope.clone(),
                event: EventId("evt-start".to_string()),
                actor_kind: ActorKind::ClaudeCode,
            },
            ok_db,
        )
        .expect("starting the live scope should succeed");

        marker.persist().expect(
            "simulating a prior crashed invocation that armed but never cleared the marker",
        );
        std::fs::write(repo_root.join("work.txt"), b"b").expect("the A -> B edit should write");

        let error = coordinate_inner(
            &repo_root,
            &RuntimeBoundary::Advance {
                scope: scope.clone(),
                event: EventId("evt-advance".to_string()),
                actor_kind: ActorKind::ClaudeCode,
            },
            ok_db,
            || {},
            |_attempt| {
                anyhow::bail!("injected failure after recovery, before the boundary commits")
            },
        )
        .expect_err("the injected post-recovery failure must fail the invocation");
        assert!(
            matches!(error, CoordinateError::Other(_)),
            "expected CoordinateError::Other from the injected failure, got {error:?}"
        );
        assert!(
            marker.exists().expect("marker existence should resolve"),
            "a failure after recovery but before the boundary commits must leave the marker armed"
        );

        let store_db = ok_db().expect("reopening the DB for assertions should succeed");
        let store = MutationTraceStore::new(&store_db);
        let after_fail = store
            .load_worktree(&worktree_id, Some(&scope), None)
            .expect("loading the worktree row should succeed")
            .expect("the worktree row should exist");
        assert!(
            !after_fail.worktree_state.tainted,
            "the recovery CAS committed durably before the injected failure"
        );
        assert_eq!(
            after_fail.worktree_state.failure_kind,
            FailureKind::Healthy,
            "recovery cleared the failure state before the injected failure"
        );
        assert!(
            after_fail.worktree_state.revision >= 2,
            "the durable recovery advanced the revision past the start boundary"
        );
        assert_ne!(
            after_fail.worktree_state.cursor_tree, tree_a,
            "recovery rebaselined the cursor away from A to the invocation's own observed tree"
        );
        assert_eq!(
            after_fail.scopes.get(&scope).map(|s| s.status),
            Some(ScopeStatus::Abandoned),
            "the live scope was abandoned by the durable recovery"
        );
        assert!(
            after_fail.processed_events.is_empty(),
            "the triggering Advance must never have been processed"
        );
        assert!(
            store
                .load_mutation_event(&worktree_id, after_fail.worktree_state.revision)
                .expect("loading a mutation event should succeed")
                .is_none(),
            "no MutationEvent may be emitted for the boundary that never committed"
        );
        drop(store_db);

        std::fs::write(repo_root.join("work.txt"), b"c").expect("the B -> C edit should write");
        let recovered = coordinate(&repo_root, &RuntimeBoundary::Flush, ok_db)
            .expect("the later invocation inherits the still-armed marker and recovers again");
        assert!(
            recovered.mutation_event.is_none(),
            "no evidence may cross the interval the still-armed marker fenced off"
        );
        assert_ne!(
            recovered.observed_tree, tree_a,
            "the later recovery rebaselines to the newer tree C"
        );
        assert!(
            !marker.exists().expect("marker existence should resolve"),
            "the later successful recovery finally clears the marker"
        );

        let store_db = ok_db().expect("reopening the DB for assertions should succeed");
        let store = MutationTraceStore::new(&store_db);
        let after_recover = store
            .load_worktree(&worktree_id, Some(&scope), None)
            .expect("loading the worktree row should succeed")
            .expect("the worktree row should exist");
        assert_eq!(
            after_recover.worktree_state.cursor_tree, recovered.observed_tree,
            "the later recovery rebaselines the cursor to its own observed tree"
        );
        assert_eq!(
            after_recover.scopes.get(&scope).map(|s| s.status),
            Some(ScopeStatus::Abandoned),
            "the later invocation must not resurrect the abandoned scope"
        );
        drop(store_db);

        remove_test_db(&db_path);
        remove_test_repo(&repo_root);
    }
}
