use std::path::Path;

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::mutation_trace::protocol;
use crate::services::mutation_trace::store::{CasResult, DurableTransition, MutationTraceStore};
use crate::services::mutation_trace::types::{ScopeId, ScopeStatus, WorktreeId};

use super::coordinator::MAX_CAS_RETRY_ATTEMPTS;
use super::protected_worktree::{
    ExternalTaintOperation, ProtectedWorktree, ProtectedWorktreeError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AbandonRecoveryReason {
    InheritedExternalTaint,
    MissingScope,
    NeverSeenScope,
    MissingWorktreeState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AbandonScopeOutcome {
    Abandoned {
        worktree_id: WorktreeId,
        scope: ScopeId,
        revision: u64,
    },
    AlreadyTerminal {
        worktree_id: WorktreeId,
        scope: ScopeId,
        status: ScopeStatus,
        revision: u64,
    },
    RecoveryRequired {
        worktree_id: WorktreeId,
        scope: ScopeId,
        reason: AbandonRecoveryReason,
    },
}

#[derive(Debug)]
pub enum AbandonScopeError {
    LockAcquisition(anyhow::Error),
    ExternalTaintMarker {
        operation: ExternalTaintOperation,
        source: anyhow::Error,
    },
    AgentTraceDbUnavailable(anyhow::Error),
    WorktreeIdentityMismatch {
        scope: ScopeId,
        scope_worktree_id: WorktreeId,
        invoking_worktree_id: WorktreeId,
    },
    RevisionExhausted {
        worktree_id: WorktreeId,
        revision: u64,
    },
    CasConflictExhausted {
        attempts: u32,
    },
    MarkerClearAfterCompletion {
        source: anyhow::Error,
        completed: Box<AbandonScopeOutcome>,
    },
    Other(anyhow::Error),
}

impl std::fmt::Display for AbandonScopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AbandonScopeError::ExternalTaintMarker { operation, source } => write!(
                f,
                "External-taint marker {operation:?} operation failed before any \
                 mutation-scope state was read: {source}"
            ),
            AbandonScopeError::AgentTraceDbUnavailable(source) => {
                write!(f, "Repository Agent Trace DB is unavailable: {source}")
            }
            AbandonScopeError::WorktreeIdentityMismatch {
                scope,
                scope_worktree_id,
                invoking_worktree_id,
            } => write!(
                f,
                "Scope {scope:?} belongs to worktree {scope_worktree_id:?} and cannot \
                 be abandoned through worktree {invoking_worktree_id:?}"
            ),
            AbandonScopeError::RevisionExhausted {
                worktree_id,
                revision,
            } => write!(
                f,
                "Scope on worktree {worktree_id:?} cannot be abandoned because its \
                 revision ({revision}) cannot be advanced"
            ),
            AbandonScopeError::CasConflictExhausted { attempts } => {
                write!(f, "Exhausted {attempts} CAS-conflict retry attempts")
            }
            AbandonScopeError::MarkerClearAfterCompletion { source, .. } => write!(
                f,
                "Mutation scope settled durably, but clearing the external-taint \
                 marker failed: {source}"
            ),
            AbandonScopeError::LockAcquisition(source) | AbandonScopeError::Other(source) => {
                write!(f, "{source}")
            }
        }
    }
}

impl std::error::Error for AbandonScopeError {}

pub fn abandon_scope<P>(
    repository_root: &Path,
    scope: &ScopeId,
    open_db: P,
) -> Result<AbandonScopeOutcome, AbandonScopeError>
where
    P: FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
{
    abandon_scope_inner(repository_root, scope, open_db, |_attempt| {})
}

pub(super) fn abandon_scope_inner<P, L>(
    repository_root: &Path,
    scope: &ScopeId,
    open_db: P,
    after_load: L,
) -> Result<AbandonScopeOutcome, AbandonScopeError>
where
    P: FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
    L: FnMut(u32),
{
    let protected =
        ProtectedWorktree::acquire(repository_root).map_err(protected_worktree_failure)?;

    if protected.inherited_external_taint() {
        return Ok(AbandonScopeOutcome::RecoveryRequired {
            worktree_id: protected.worktree_id().clone(),
            scope: scope.clone(),
            reason: AbandonRecoveryReason::InheritedExternalTaint,
        });
    }

    let outcome = abandon_protected(protected.worktree_id(), scope, open_db, after_load)?;

    if matches!(outcome, AbandonScopeOutcome::RecoveryRequired { .. }) {
        return Ok(outcome);
    }

    match protected.complete() {
        Ok(()) => Ok(outcome),
        Err(source) => Err(AbandonScopeError::MarkerClearAfterCompletion {
            source,
            completed: Box::new(outcome),
        }),
    }
}

fn abandon_protected<P, L>(
    worktree_id: &WorktreeId,
    scope: &ScopeId,
    open_db: P,
    mut after_load: L,
) -> Result<AbandonScopeOutcome, AbandonScopeError>
where
    P: FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
    L: FnMut(u32),
{
    let db = open_db().map_err(AbandonScopeError::AgentTraceDbUnavailable)?;
    let store = MutationTraceStore::new(&db);

    for attempt_index in 0..MAX_CAS_RETRY_ATTEMPTS {
        let Some(scope_state) = store.load_scope(scope).map_err(AbandonScopeError::Other)? else {
            return Ok(recovery_required(
                worktree_id,
                scope,
                AbandonRecoveryReason::MissingScope,
            ));
        };
        if scope_state.worktree_id != *worktree_id {
            return Err(AbandonScopeError::WorktreeIdentityMismatch {
                scope: scope.clone(),
                scope_worktree_id: scope_state.worktree_id,
                invoking_worktree_id: worktree_id.clone(),
            });
        }

        let Some(projection) = store
            .load_worktree(worktree_id, Some(scope), None)
            .map_err(AbandonScopeError::Other)?
        else {
            return Ok(recovery_required(
                worktree_id,
                scope,
                AbandonRecoveryReason::MissingWorktreeState,
            ));
        };

        after_load(attempt_index);

        let state = projection.into_protocol_state();
        let revision = state
            .worktrees
            .get(worktree_id)
            .map(|worktree_state| worktree_state.revision)
            .ok_or_else(|| {
                AbandonScopeError::Other(anyhow::anyhow!(
                    "worktree {worktree_id:?} missing from its own loaded projection"
                ))
            })?;
        let status = state
            .scopes
            .get(scope)
            .map(|loaded| loaded.status)
            .ok_or_else(|| {
                AbandonScopeError::Other(anyhow::anyhow!(
                    "scope {scope:?} missing from the projection that loaded it as its \
                     effective referenced scope"
                ))
            })?;

        match status {
            ScopeStatus::NeverSeen => {
                return Ok(recovery_required(
                    worktree_id,
                    scope,
                    AbandonRecoveryReason::NeverSeenScope,
                ))
            }
            ScopeStatus::Closed | ScopeStatus::Abandoned => {
                return Ok(AbandonScopeOutcome::AlreadyTerminal {
                    worktree_id: worktree_id.clone(),
                    scope: scope.clone(),
                    status,
                    revision,
                })
            }
            ScopeStatus::Active => {}
        }

        let abandoned = protocol::abandon(&state, scope);

        let Some(transition) = DurableTransition::between(&state, &abandoned, worktree_id)
            .map_err(AbandonScopeError::Other)?
        else {
            return Err(AbandonScopeError::RevisionExhausted {
                worktree_id: worktree_id.clone(),
                revision,
            });
        };

        match store
            .commit(&transition)
            .map_err(AbandonScopeError::Other)?
        {
            CasResult::Applied => {
                let next_revision = abandoned
                    .worktrees
                    .get(worktree_id)
                    .map(|worktree_state| worktree_state.revision)
                    .expect("the abandoned worktree's state must still be present after abandon");
                return Ok(AbandonScopeOutcome::Abandoned {
                    worktree_id: worktree_id.clone(),
                    scope: scope.clone(),
                    revision: next_revision,
                });
            }
            CasResult::Conflict => {}
        }
    }

    Err(AbandonScopeError::CasConflictExhausted {
        attempts: MAX_CAS_RETRY_ATTEMPTS,
    })
}

fn recovery_required(
    worktree_id: &WorktreeId,
    scope: &ScopeId,
    reason: AbandonRecoveryReason,
) -> AbandonScopeOutcome {
    AbandonScopeOutcome::RecoveryRequired {
        worktree_id: worktree_id.clone(),
        scope: scope.clone(),
        reason,
    }
}

fn protected_worktree_failure(error: ProtectedWorktreeError) -> AbandonScopeError {
    match error {
        ProtectedWorktreeError::GitDirResolution(source)
        | ProtectedWorktreeError::CheckoutIdentity(source) => AbandonScopeError::Other(source),
        ProtectedWorktreeError::LockAcquisition(source) => {
            AbandonScopeError::LockAcquisition(anyhow::Error::new(source))
        }
        ProtectedWorktreeError::ExternalTaintMarker { operation, source } => {
            AbandonScopeError::ExternalTaintMarker { operation, source }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::*;
    use crate::services::checkout::{get_or_create_checkout_id, resolve_git_dir};
    use crate::services::mutation_trace::runtime::external_taint::ExternalTaintMarker;
    use crate::services::mutation_trace::store::{decode_revision, encode_revision};
    use crate::services::mutation_trace::types::WorktreeState;

    struct TestScopeRepo {
        _temp_dir: tempfile::TempDir,
        repo_root: PathBuf,
        git_dir: PathBuf,
        db_path: PathBuf,
    }

    impl TestScopeRepo {
        fn new(label: &str) -> Self {
            let temp_dir = tempfile::Builder::new()
                .prefix(&format!("sce-mutation-trace-scope-runtime-{label}-"))
                .tempdir()
                .expect("test temp directory should be created");
            let repo_root = temp_dir.path().join("repo");
            std::fs::create_dir_all(&repo_root).expect("repository directory should be created");
            run_git(&repo_root, &["init", "--quiet"]);
            let git_dir = resolve_git_dir(&repo_root).expect("git dir should resolve");
            let db_path = temp_dir.path().join("agent-trace.db");
            RepositoryAgentTraceDb::new_at(&db_path)
                .expect("the repository DB should open with schema");
            Self {
                _temp_dir: temp_dir,
                repo_root,
                git_dir,
                db_path,
            }
        }

        fn open_db(&self) -> anyhow::Result<RepositoryAgentTraceDb> {
            RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(&self.db_path)
        }

        fn db(&self) -> RepositoryAgentTraceDb {
            self.open_db()
                .expect("reopening the DB for assertions should succeed")
        }

        fn worktree_id(&self) -> WorktreeId {
            WorktreeId(
                get_or_create_checkout_id(&self.git_dir)
                    .expect("the checkout identity should resolve"),
            )
        }

        fn marker(&self) -> ExternalTaintMarker {
            ExternalTaintMarker::new(&self.git_dir)
        }

        fn marker_path(&self) -> PathBuf {
            self.git_dir.join("sce").join("mutation-cursor-tainted")
        }

        fn marker_exists(&self) -> bool {
            self.marker()
                .exists()
                .expect("marker existence should resolve")
        }
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git command should spawn");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn seed_worktree(db: &RepositoryAgentTraceDb, worktree: &WorktreeId, revision: u64) {
        db.execute(
            "INSERT INTO mutation_trace_worktrees
                (worktree_id, cursor_tree, revision, tainted, failure_kind, needs_rebaseline)
             VALUES (?1, 'tree-0', ?2, 0, 'healthy', 0)",
            (worktree.0.as_str(), encode_revision(revision).as_slice()),
        )
        .expect("worktree insert should succeed");
    }

    fn seed_scope(
        db: &RepositoryAgentTraceDb,
        scope: &ScopeId,
        worktree: &WorktreeId,
        status: ScopeStatus,
    ) {
        db.execute(
            "INSERT INTO mutation_trace_scopes (scope_id, worktree_id, actor_kind, status)
             VALUES (?1, ?2, 'claude_code', ?3)",
            (
                scope.0.as_str(),
                worktree.0.as_str(),
                crate::services::mutation_trace::store::encode_scope_status(status),
            ),
        )
        .expect("scope insert should succeed");
    }

    fn bump_revision(db_path: &Path, worktree: &WorktreeId) {
        let db = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(db_path)
            .expect("the competing writer should open the DB");
        let current = read_worktree(&db, worktree).expect("the worktree row should exist");
        db.execute(
            "UPDATE mutation_trace_worktrees SET revision = ?1 WHERE worktree_id = ?2",
            (
                encode_revision(current.revision + 1).as_slice(),
                worktree.0.as_str(),
            ),
        )
        .expect("the competing revision bump should succeed");
    }

    fn set_scope_status(db_path: &Path, scope: &ScopeId, status: ScopeStatus) {
        let db = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(db_path)
            .expect("the competing writer should open the DB");
        db.execute(
            "UPDATE mutation_trace_scopes SET status = ?1 WHERE scope_id = ?2",
            (
                crate::services::mutation_trace::store::encode_scope_status(status),
                scope.0.as_str(),
            ),
        )
        .expect("the competing scope-status write should succeed");
    }

    fn arm_a_scope_status_write_collision(db_path: &Path) {
        let db = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(db_path)
            .expect("the competing writer should open the DB");
        db.execute(
            "CREATE UNIQUE INDEX idx_test_one_scope_per_status
             ON mutation_trace_scopes (status)",
            (),
        )
        .expect("the collision index should be created");
    }

    fn read_worktree(db: &RepositoryAgentTraceDb, worktree: &WorktreeId) -> Option<WorktreeState> {
        let rows = db
            .query_map(
                "SELECT cursor_tree, revision, tainted, failure_kind, needs_rebaseline
                 FROM mutation_trace_worktrees WHERE worktree_id = ?1",
                (worktree.0.as_str(),),
                |row| {
                    let cursor_tree = row.get::<String>(0)?;
                    let revision = row.get::<Vec<u8>>(1)?;
                    let tainted = row.get::<i64>(2)?;
                    let failure_kind = row.get::<String>(3)?;
                    let needs_rebaseline = row.get::<i64>(4)?;
                    Ok((
                        cursor_tree,
                        revision,
                        tainted,
                        failure_kind,
                        needs_rebaseline,
                    ))
                },
            )
            .expect("the worktree read should succeed");

        rows.into_iter().next().map(
            |(cursor_tree, revision, tainted, failure_kind, needs_rebaseline)| WorktreeState {
                cursor_tree: crate::services::mutation_trace::types::TreeId(cursor_tree),
                revision: decode_revision(&revision).expect("the revision should decode"),
                tainted: tainted != 0,
                failure_kind: crate::services::mutation_trace::store::decode_failure_kind(
                    &failure_kind,
                )
                .expect("the failure kind should decode"),
                needs_rebaseline: needs_rebaseline != 0,
            },
        )
    }

    fn read_scope_status(db: &RepositoryAgentTraceDb, scope: &ScopeId) -> Option<ScopeStatus> {
        let rows = db
            .query_map(
                "SELECT status FROM mutation_trace_scopes WHERE scope_id = ?1",
                (scope.0.as_str(),),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .expect("the scope read should succeed");

        rows.into_iter().next().map(|status| {
            crate::services::mutation_trace::store::decode_scope_status(&status)
                .expect("the scope status should decode")
        })
    }

    fn count_rows(db: &RepositoryAgentTraceDb, table: &str) -> i64 {
        let rows = db
            .query_map(&format!("SELECT COUNT(*) FROM {table}"), (), |row| {
                row.get::<i64>(0).map_err(Into::into)
            })
            .expect("the row count should succeed");

        rows.into_iter().next().unwrap_or_default()
    }

    #[test]
    fn an_inherited_marker_requires_recovery_without_consulting_the_db_provider() {
        let repo = TestScopeRepo::new("inherited-marker");
        let scope = ScopeId("scope-dead".to_string());
        repo.marker()
            .persist()
            .expect("the earlier invocation's marker should arm");

        let provider_called = Cell::new(false);
        let outcome = abandon_scope(&repo.repo_root, &scope, || {
            provider_called.set(true);
            Err(anyhow::anyhow!("the DB provider must never be invoked"))
        })
        .expect("an inherited marker settles as a successful recovery-required outcome");

        assert!(
            matches!(
                outcome,
                AbandonScopeOutcome::RecoveryRequired {
                    reason: AbandonRecoveryReason::InheritedExternalTaint,
                    ..
                }
            ),
            "expected InheritedExternalTaint, got {outcome:?}"
        );
        assert!(
            !provider_called.get(),
            "the inherited-marker short-circuit must return before the DB provider runs"
        );
        assert!(
            repo.marker_exists(),
            "the inherited marker must stay armed for the next invocation to recover"
        );
    }

    #[test]
    fn a_missing_scope_row_requires_recovery_and_leaves_the_fence_armed() {
        let repo = TestScopeRepo::new("missing-scope");
        let worktree = repo.worktree_id();
        let scope = ScopeId("scope-never-registered".to_string());
        seed_worktree(&repo.db(), &worktree, 3);

        let outcome = abandon_scope(&repo.repo_root, &scope, || repo.open_db())
            .expect("a missing scope row settles as a recovery-required outcome");

        assert!(
            matches!(
                outcome,
                AbandonScopeOutcome::RecoveryRequired {
                    reason: AbandonRecoveryReason::MissingScope,
                    ..
                }
            ),
            "expected MissingScope, got {outcome:?}"
        );
        assert!(
            repo.marker_exists(),
            "a scope whose Start never committed must leave the fence armed"
        );

        let db = repo.db();
        assert_eq!(
            read_worktree(&db, &worktree)
                .expect("the worktree row should exist")
                .revision,
            3,
            "a recovery-required outcome must not advance the revision"
        );
        assert_eq!(read_scope_status(&db, &scope), None);
    }

    #[test]
    fn a_never_seen_scope_requires_recovery_and_leaves_the_fence_armed() {
        let repo = TestScopeRepo::new("never-seen-scope");
        let worktree = repo.worktree_id();
        let scope = ScopeId("scope-registered-only".to_string());
        {
            let db = repo.db();
            seed_worktree(&db, &worktree, 3);
            seed_scope(&db, &scope, &worktree, ScopeStatus::NeverSeen);
        }

        let outcome = abandon_scope(&repo.repo_root, &scope, || repo.open_db())
            .expect("a NeverSeen scope settles as a recovery-required outcome");

        assert!(
            matches!(
                outcome,
                AbandonScopeOutcome::RecoveryRequired {
                    reason: AbandonRecoveryReason::NeverSeenScope,
                    ..
                }
            ),
            "expected NeverSeenScope, got {outcome:?}"
        );
        assert!(
            repo.marker_exists(),
            "a scope with no observed Start must leave the fence armed"
        );

        let db = repo.db();
        assert_eq!(
            read_worktree(&db, &worktree)
                .expect("the worktree row should exist")
                .revision,
            3,
            "a recovery-required outcome must not advance the revision"
        );
        assert_eq!(
            read_scope_status(&db, &scope),
            Some(ScopeStatus::NeverSeen),
            "a recovery-required outcome must not change any scope status"
        );
    }

    #[test]
    fn a_scope_whose_worktree_row_is_missing_requires_recovery() {
        let repo = TestScopeRepo::new("missing-worktree-row");
        let worktree = repo.worktree_id();
        let scope = ScopeId("scope-orphan".to_string());
        seed_scope(&repo.db(), &scope, &worktree, ScopeStatus::Active);

        let outcome = abandon_scope(&repo.repo_root, &scope, || repo.open_db())
            .expect("a missing worktree row settles as a recovery-required outcome");

        assert!(
            matches!(
                outcome,
                AbandonScopeOutcome::RecoveryRequired {
                    reason: AbandonRecoveryReason::MissingWorktreeState,
                    ..
                }
            ),
            "expected MissingWorktreeState, got {outcome:?}"
        );
        assert!(
            repo.marker_exists(),
            "a worktree with no durable row must leave the fence armed"
        );
        assert_eq!(
            read_scope_status(&repo.db(), &scope),
            Some(ScopeStatus::Active),
            "no scope status may change when there is no worktree to transition"
        );
    }

    #[test]
    fn abandoning_a_live_scope_writes_only_the_scope_and_worktree_rows() {
        let repo = TestScopeRepo::new("active-abandonment");
        let worktree = repo.worktree_id();
        let scope = ScopeId("scope-live".to_string());
        let bystander = ScopeId("scope-bystander".to_string());
        {
            let db = repo.db();
            seed_worktree(&db, &worktree, 3);
            seed_scope(&db, &scope, &worktree, ScopeStatus::Active);
            seed_scope(&db, &bystander, &worktree, ScopeStatus::Active);
        }

        let outcome = abandon_scope(&repo.repo_root, &scope, || repo.open_db())
            .expect("abandoning a live scope should succeed");

        assert_eq!(
            outcome,
            AbandonScopeOutcome::Abandoned {
                worktree_id: worktree.clone(),
                scope: scope.clone(),
                revision: 4,
            }
        );
        assert!(
            !repo.marker_exists(),
            "a settled abandonment must clear the marker it armed"
        );

        let db = repo.db();
        let worktree_state = read_worktree(&db, &worktree).expect("the worktree row should exist");
        assert_eq!(worktree_state.revision, 4, "the revision advances by one");
        assert!(worktree_state.needs_rebaseline);
        assert_eq!(
            worktree_state.cursor_tree,
            crate::services::mutation_trace::types::TreeId("tree-0".to_string()),
            "abandonment observes no tree, so the cursor is left where it was"
        );
        assert!(!worktree_state.tainted);
        assert_eq!(
            worktree_state.failure_kind,
            crate::services::mutation_trace::types::FailureKind::Healthy
        );

        assert_eq!(
            read_scope_status(&db, &scope),
            Some(ScopeStatus::Abandoned),
            "the named scope is retired"
        );
        assert_eq!(
            read_scope_status(&db, &bystander),
            Some(ScopeStatus::Active),
            "abandonment retires only the scope it was given"
        );

        assert_eq!(count_rows(&db, "mutation_trace_events"), 0);
        assert_eq!(count_rows(&db, "mutation_trace_event_active_scopes"), 0);
        assert_eq!(count_rows(&db, "mutation_trace_processed_events"), 0);
    }

    #[test]
    fn a_closed_scope_settles_as_a_terminal_no_op() {
        let repo = TestScopeRepo::new("closed-scope");
        let worktree = repo.worktree_id();
        let scope = ScopeId("scope-closed".to_string());
        {
            let db = repo.db();
            seed_worktree(&db, &worktree, 7);
            seed_scope(&db, &scope, &worktree, ScopeStatus::Closed);
        }

        let outcome = abandon_scope(&repo.repo_root, &scope, || repo.open_db())
            .expect("an already-closed scope settles successfully");

        assert_eq!(
            outcome,
            AbandonScopeOutcome::AlreadyTerminal {
                worktree_id: worktree.clone(),
                scope: scope.clone(),
                status: ScopeStatus::Closed,
                revision: 7,
            }
        );
        assert!(
            !repo.marker_exists(),
            "a proven-terminal no-op must clear the marker it armed"
        );

        let db = repo.db();
        assert_eq!(
            read_worktree(&db, &worktree)
                .expect("the worktree row should exist")
                .revision,
            7,
            "a terminal no-op writes nothing"
        );
        assert_eq!(read_scope_status(&db, &scope), Some(ScopeStatus::Closed));
    }

    #[test]
    fn an_already_abandoned_scope_settles_as_a_terminal_no_op() {
        let repo = TestScopeRepo::new("abandoned-scope");
        let worktree = repo.worktree_id();
        let scope = ScopeId("scope-abandoned".to_string());
        {
            let db = repo.db();
            seed_worktree(&db, &worktree, 7);
            seed_scope(&db, &scope, &worktree, ScopeStatus::Abandoned);
        }

        let outcome = abandon_scope(&repo.repo_root, &scope, || repo.open_db())
            .expect("an already-abandoned scope settles successfully");

        assert_eq!(
            outcome,
            AbandonScopeOutcome::AlreadyTerminal {
                worktree_id: worktree.clone(),
                scope: scope.clone(),
                status: ScopeStatus::Abandoned,
                revision: 7,
            }
        );
        assert!(
            !repo.marker_exists(),
            "a proven-terminal no-op must clear the marker it armed"
        );
        assert_eq!(
            read_worktree(&repo.db(), &worktree)
                .expect("the worktree row should exist")
                .revision,
            7,
            "a terminal no-op never abandons a scope a second time"
        );
    }

    #[test]
    fn a_scope_owned_by_another_worktree_is_rejected_without_writing() {
        let repo = TestScopeRepo::new("cross-worktree");
        let worktree = repo.worktree_id();
        let other = WorktreeId("wt-other".to_string());
        let scope = ScopeId("scope-elsewhere".to_string());
        {
            let db = repo.db();
            seed_worktree(&db, &worktree, 3);
            seed_worktree(&db, &other, 11);
            seed_scope(&db, &scope, &other, ScopeStatus::Active);
        }

        let error = abandon_scope(&repo.repo_root, &scope, || repo.open_db())
            .expect_err("a scope owned by another worktree must be rejected");

        match &error {
            AbandonScopeError::WorktreeIdentityMismatch {
                scope: rejected,
                scope_worktree_id,
                invoking_worktree_id,
            } => {
                assert_eq!(rejected, &scope);
                assert_eq!(scope_worktree_id, &other);
                assert_eq!(invoking_worktree_id, &worktree);
            }
            other => panic!("expected WorktreeIdentityMismatch, got {other:?}"),
        }

        let db = repo.db();
        assert_eq!(
            read_worktree(&db, &worktree)
                .expect("this worktree's row should exist")
                .revision,
            3
        );
        assert_eq!(
            read_worktree(&db, &other)
                .expect("the other worktree's row should exist")
                .revision,
            11
        );
        assert_eq!(read_scope_status(&db, &scope), Some(ScopeStatus::Active));
    }

    #[test]
    fn a_live_scope_on_an_exhausted_revision_is_a_distinct_error() {
        let repo = TestScopeRepo::new("revision-exhausted");
        let worktree = repo.worktree_id();
        let scope = ScopeId("scope-live".to_string());
        {
            let db = repo.db();
            seed_worktree(&db, &worktree, u64::MAX);
            seed_scope(&db, &scope, &worktree, ScopeStatus::Active);
        }

        let error = abandon_scope(&repo.repo_root, &scope, || repo.open_db())
            .expect_err("a worktree at the maximum revision cannot abandon");

        match &error {
            AbandonScopeError::RevisionExhausted {
                worktree_id,
                revision,
            } => {
                assert_eq!(worktree_id, &worktree);
                assert_eq!(*revision, u64::MAX);
            }
            other => panic!("expected RevisionExhausted, got {other:?}"),
        }
        assert_eq!(
            read_scope_status(&repo.db(), &scope),
            Some(ScopeStatus::Active),
            "the scope is still live and still needs retiring"
        );
    }

    #[test]
    fn a_cas_conflict_recomputes_the_abandonment_from_fresh_state() {
        let repo = TestScopeRepo::new("cas-conflict-retry");
        let worktree = repo.worktree_id();
        let scope = ScopeId("scope-live".to_string());
        {
            let db = repo.db();
            seed_worktree(&db, &worktree, 3);
            seed_scope(&db, &scope, &worktree, ScopeStatus::Active);
        }

        let attempts = Cell::new(0u32);
        let outcome = abandon_scope_inner(
            &repo.repo_root,
            &scope,
            || repo.open_db(),
            |attempt| {
                attempts.set(attempts.get() + 1);
                if attempt == 0 {
                    bump_revision(&repo.db_path, &worktree);
                }
            },
        )
        .expect("a losing CAS attempt should retry and settle");

        assert_eq!(
            attempts.get(),
            2,
            "the first attempt loses the CAS and the second recomputes from fresh state"
        );
        assert_eq!(
            outcome,
            AbandonScopeOutcome::Abandoned {
                worktree_id: worktree.clone(),
                scope: scope.clone(),
                revision: 5,
            },
            "the retry advances from the competitor's revision, not the stale one"
        );

        let db = repo.db();
        assert_eq!(
            read_worktree(&db, &worktree)
                .expect("the worktree row should exist")
                .revision,
            5
        );
        assert_eq!(read_scope_status(&db, &scope), Some(ScopeStatus::Abandoned));
    }

    #[test]
    fn a_cas_conflict_whose_competitor_ended_the_scope_settles_as_a_terminal_no_op() {
        let repo = TestScopeRepo::new("cas-conflict-terminal");
        let worktree = repo.worktree_id();
        let scope = ScopeId("scope-live".to_string());
        {
            let db = repo.db();
            seed_worktree(&db, &worktree, 3);
            seed_scope(&db, &scope, &worktree, ScopeStatus::Active);
        }

        let outcome = abandon_scope_inner(
            &repo.repo_root,
            &scope,
            || repo.open_db(),
            |attempt| {
                if attempt == 0 {
                    set_scope_status(&repo.db_path, &scope, ScopeStatus::Closed);
                    bump_revision(&repo.db_path, &worktree);
                }
            },
        )
        .expect("a competitor that ended the scope should settle the retry");

        assert_eq!(
            outcome,
            AbandonScopeOutcome::AlreadyTerminal {
                worktree_id: worktree.clone(),
                scope: scope.clone(),
                status: ScopeStatus::Closed,
                revision: 4,
            },
            "the retry must settle on the competitor's terminal status, not overwrite it"
        );

        let db = repo.db();
        assert_eq!(
            read_scope_status(&db, &scope),
            Some(ScopeStatus::Closed),
            "a competitor's Close must never be overwritten by a second abandonment"
        );
        assert_eq!(
            read_worktree(&db, &worktree)
                .expect("the worktree row should exist")
                .revision,
            4
        );
    }

    #[test]
    fn a_persistence_failure_rolls_back_the_whole_transition_and_leaves_the_fence_armed() {
        let repo = TestScopeRepo::new("persistence-failure");
        let worktree = repo.worktree_id();
        let scope = ScopeId("scope-live".to_string());
        let bystander = ScopeId("scope-already-abandoned".to_string());
        {
            let db = repo.db();
            seed_worktree(&db, &worktree, 3);
            seed_scope(&db, &scope, &worktree, ScopeStatus::Active);
            seed_scope(&db, &bystander, &worktree, ScopeStatus::Abandoned);
        }

        let error = abandon_scope_inner(
            &repo.repo_root,
            &scope,
            || repo.open_db(),
            |attempt| {
                if attempt == 0 {
                    arm_a_scope_status_write_collision(&repo.db_path);
                }
            },
        )
        .expect_err("a failing durable write must fail the abandonment");

        assert!(
            matches!(error, AbandonScopeError::Other(_)),
            "a persistence failure surfaces as the store-error variant, got {error:?}"
        );
        assert!(
            format!("{error}").contains("mutation_trace_scopes"),
            "the failure must be the transaction's scope-status write — the statement that \
             runs after the worktree CAS guard — not an earlier read or a settled conflict; \
             got {error}"
        );
        assert!(
            repo.marker_exists(),
            "a persistence failure after the fence is armed must leave it armed"
        );

        let db = repo.db();
        let worktree_state = read_worktree(&db, &worktree).expect("the worktree row should exist");
        assert_eq!(
            worktree_state.revision, 3,
            "the transaction's worktree CAS guard ran before the failing statement, so a \
             partially applied revision here would mean the batch is not atomic"
        );
        assert!(
            !worktree_state.needs_rebaseline,
            "the rolled-back transition must not leave the worktree needing rebaseline"
        );
        assert_eq!(
            worktree_state.cursor_tree,
            crate::services::mutation_trace::types::TreeId("tree-0".to_string())
        );
        assert_eq!(
            read_scope_status(&db, &scope),
            Some(ScopeStatus::Active),
            "the target scope must not be left Abandoned by a failed transition"
        );
        assert_eq!(
            read_scope_status(&db, &bystander),
            Some(ScopeStatus::Abandoned)
        );

        assert_eq!(count_rows(&db, "mutation_trace_events"), 0);
        assert_eq!(count_rows(&db, "mutation_trace_event_active_scopes"), 0);
        assert_eq!(count_rows(&db, "mutation_trace_processed_events"), 0);
    }

    #[test]
    fn a_db_provider_failure_leaves_the_fence_armed() {
        let repo = TestScopeRepo::new("db-provider-failure");
        let scope = ScopeId("scope-live".to_string());

        let error = abandon_scope(&repo.repo_root, &scope, || {
            Err(anyhow::anyhow!("simulated Agent Trace DB open failure"))
        })
        .expect_err("a DB provider that returns Err must fail abandon_scope()");

        assert!(
            matches!(error, AbandonScopeError::AgentTraceDbUnavailable(_)),
            "expected AgentTraceDbUnavailable, got {error:?}"
        );
        assert!(
            repo.marker_exists(),
            "a DB-provider failure after arming must leave the fence armed"
        );
    }

    #[test]
    fn a_marker_clear_failure_carries_the_already_settled_outcome() {
        let repo = TestScopeRepo::new("marker-clear-failure");
        let worktree = repo.worktree_id();
        let scope = ScopeId("scope-live".to_string());
        {
            let db = repo.db();
            seed_worktree(&db, &worktree, 3);
            seed_scope(&db, &scope, &worktree, ScopeStatus::Active);
        }

        let marker_path = repo.marker_path();
        let error = abandon_scope(&repo.repo_root, &scope, || {
            std::fs::remove_file(&marker_path)
                .expect("the armed marker file should be present mid-invocation");
            std::fs::create_dir_all(marker_path.join("nested"))
                .expect("planting a non-empty directory at the marker path should succeed");
            repo.open_db()
        })
        .expect_err("clearing a marker that is now a non-empty directory must fail");

        let completed = match error {
            AbandonScopeError::MarkerClearAfterCompletion { completed, .. } => completed,
            other => panic!("expected MarkerClearAfterCompletion, got {other:?}"),
        };
        assert_eq!(
            *completed,
            AbandonScopeOutcome::Abandoned {
                worktree_id: worktree.clone(),
                scope: scope.clone(),
                revision: 4,
            },
            "the error must carry the outcome that already settled durably"
        );
        assert!(
            repo.marker_exists(),
            "the marker stays logically armed after a post-completion clear failure"
        );

        let db = repo.db();
        let worktree_state = read_worktree(&db, &worktree).expect("the worktree row should exist");
        assert_eq!(worktree_state.revision, 4);
        assert!(worktree_state.needs_rebaseline);
        assert_eq!(read_scope_status(&db, &scope), Some(ScopeStatus::Abandoned));

        std::fs::remove_dir_all(&marker_path)
            .expect("removing the planted directory should succeed");
    }
}
