use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

use super::super::super::coordinator::{coordinate_inner, CoordinateOutcome};
use super::super::super::maintenance_state::write_state_atomically;
use super::super::super::protected_worktree::WORKTREE_LOCK_TIMEOUT;
use super::super::super::ref_reconciliation::ReconcilePhase;
use super::super::{open_authoritative_db_at_state_root, reconcile_explicit_with, ExplicitOutcome};
use super::{Fixture, NOW};
use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::hooks::mutation_scope::{drive_mutation_scope, MutationScopePayload};
use crate::services::mutation_trace::runtime::{
    AbandonScopeError, AbandonScopeOutcome, AdvisoryReport, RuntimeBoundary,
};
use crate::services::mutation_trace::store::MutationTraceStore;
use crate::services::mutation_trace::types::{
    ActorKind, ScopeId, ScopeStatus, TreeId, WorktreeId, WorktreeState,
};
use crate::services::observability::traits::NoopLogger;

pub(super) const WATCHDOG: Duration = Duration::from_mins(1);

pub(super) fn main_worktree() -> WorktreeId {
    WorktreeId("main".to_string())
}

pub(super) fn start_payload(scope: &str, event: &str) -> MutationScopePayload {
    MutationScopePayload::Start {
        scope_id: scope.to_string(),
        event_id: event.to_string(),
        actor_kind: ActorKind::ClaudeCode,
        provenance: None,
    }
}

pub(super) fn close_payload(scope: &str, event: &str) -> MutationScopePayload {
    MutationScopePayload::Close {
        scope_id: scope.to_string(),
        event_id: event.to_string(),
        actor_kind: ActorKind::ClaudeCode,
    }
}

pub(super) fn write_file(root: &Path, name: &str, content: &str) {
    std::fs::write(root.join(name), content).expect("write working tree file");
}

pub(super) fn sce_refs(root: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args([
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/sce/",
        ])
        .current_dir(root)
        .output()
        .expect("for-each-ref");
    assert!(output.status.success(), "for-each-ref failed");
    let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    lines.sort();
    lines
}

pub(super) fn pinned_trees(root: &Path, worktree: &str) -> BTreeSet<String> {
    let prefix = format!("refs/sce/mutation-cursor/{worktree}/");
    sce_refs(root)
        .into_iter()
        .filter_map(|line| {
            let (name, tree) = line.split_once(' ')?;
            name.strip_prefix(&prefix)
                .filter(|rest| !rest.contains('/'))
                .map(|_| tree.to_string())
        })
        .collect()
}

pub(super) async fn writer(db_path: &Path) -> anyhow::Result<RepositoryAgentTraceDb> {
    RepositoryAgentTraceDb::open_without_migrations_at(db_path).await
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DurableSnapshot {
    pub worktree: WorktreeState,
    pub scope: Option<ScopeStatus>,
    pub roots: BTreeSet<TreeId>,
}

pub(super) async fn durable_snapshot(
    db_path: &Path,
    worktree: &WorktreeId,
    scope: &str,
) -> DurableSnapshot {
    let mut db = writer(db_path).await.expect("inspection db");
    let store = MutationTraceStore::new(&mut db);
    let projection = store
        .load_worktree(worktree, None, None)
        .await
        .expect("load worktree")
        .expect("worktree row");
    let scope = store
        .load_scope(&ScopeId(scope.to_string()))
        .await
        .expect("load scope")
        .map(|state| state.status);
    let roots = store.load_all_tree_roots().await.expect("roots");
    DurableSnapshot {
        worktree: projection.worktree_state,
        scope,
        roots,
    }
}

pub(super) async fn run_hook<C, O, V>(
    root: &Path,
    db_path: &Path,
    payload: MutationScopePayload,
    lock_timeout: Duration,
    on_contention: C,
    in_open_db: O,
    advise: V,
) -> anyhow::Result<String>
where
    C: FnOnce() + Send + 'static,
    O: FnOnce(),
    V: FnOnce(&Path) -> AdvisoryReport,
{
    drive_mutation_scope(
        root,
        payload,
        None::<&NoopLogger>,
        &mut Vec::new(),
        async |root: &Path, boundary: &RuntimeBoundary| {
            coordinate_inner(
                root,
                boundary,
                async || {
                    in_open_db();
                    writer(db_path).await
                },
                lock_timeout,
                on_contention,
                |_attempt| {},
                |_attempt| Ok(()),
            )
            .await
        },
        async |_root: &Path, _scope: &ScopeId| -> Result<AbandonScopeOutcome, AbandonScopeError> {
            unreachable!("abandon is not driven by these tests")
        },
        advise,
    )
    .await
}

pub(super) fn no_advice(_root: &Path) -> AdvisoryReport {
    AdvisoryReport {
        outcome: "no_action",
        severity: crate::services::mutation_trace::runtime::AdvisorySeverity::Debug,
        recommendation: None,
        warning: None,
        diagnostic: None,
    }
}

pub(super) async fn hook_ok(fixture: &Fixture, payload: MutationScopePayload) -> String {
    run_hook(
        &fixture.root,
        &fixture.db_path(),
        payload,
        WORKTREE_LOCK_TIMEOUT,
        || {},
        || {},
        no_advice,
    )
    .await
    .expect("hook boundary completes")
}

pub(super) async fn coordinate_flush(root: &Path, db_path: &Path) -> CoordinateOutcome {
    coordinate_inner(
        root,
        &RuntimeBoundary::Flush,
        async || writer(db_path).await,
        WORKTREE_LOCK_TIMEOUT,
        || {},
        |_attempt| {},
        |_attempt| Ok(()),
    )
    .await
    .expect("flush completes")
}

pub(super) struct ParkedPass {
    outcome: tokio::sync::oneshot::Receiver<ExplicitOutcome>,
    pub parked: UnboundedReceiver<()>,
    release: mpsc::Sender<()>,
}

impl ParkedPass {
    pub(super) async fn finish(self) -> ExplicitOutcome {
        self.release.send(()).expect("release the pass");
        tokio::time::timeout(WATCHDOG, self.outcome)
            .await
            .expect("pass watchdog")
            .expect("pass thread reports its outcome")
    }
}

pub(super) fn spawn_parked_pass(fixture: &Fixture, target: ReconcilePhase) -> ParkedPass {
    spawn_parked_pass_at(&fixture.root, &fixture.state_root, target)
}

pub(super) fn spawn_parked_pass_at(
    root: &Path,
    state_root: &Path,
    target: ReconcilePhase,
) -> ParkedPass {
    let root: PathBuf = root.to_path_buf();
    let state_root: PathBuf = state_root.to_path_buf();
    let (parked_tx, parked) = unbounded_channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let (outcome_tx, outcome) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("pass runtime");
        let result = runtime.block_on(reconcile_explicit_with(
            &root,
            async || open_authoritative_db_at_state_root(&root, &state_root).await,
            || NOW,
            write_state_atomically,
            move |phase| {
                if phase == target {
                    let _ = parked_tx.send(());
                    let _ = release_rx.recv_timeout(WATCHDOG);
                }
            },
            Duration::from_secs(10),
        ));
        let _ = outcome_tx.send(result);
    });
    ParkedPass {
        outcome,
        parked,
        release,
    }
}

pub(super) async fn await_signal(receiver: &mut UnboundedReceiver<()>) {
    tokio::time::timeout(WATCHDOG, receiver.recv())
        .await
        .expect("watchdog expired while waiting for a synchronization signal")
        .expect("signal sender dropped before signalling");
}
