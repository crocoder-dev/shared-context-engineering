//! MBT driver: connects Quint Connect's generated/replayed traces to the
//! real `protocol.rs` transition functions.
//!
//! [`MutationCursorDriver::step`] dispatches on the `MbtAction` variant Quint
//! recorded — never on a before/after state diff — and every arm
//! unconditionally calls the corresponding `protocol::*` function with the
//! transported arguments, including on an `MbtAction` variant Quint produced
//! from a guarded/no-op path (T02's `mbtStutterAs` instrumentation): the
//! driver has no way to distinguish that case from a real transition, and
//! must not try to, since replaying the guarded call and comparing the
//! resulting no-op state against Quint is the point of the regressions in
//! T05.

use std::collections::{BTreeMap, BTreeSet};

use quint_connect::{switch, Config, Driver, Result, State, Step};

use super::super::protocol;
use super::super::types::{
    boundary_worktree, ActorKind, AttemptId, AttemptState, AttemptStatus, Boundary, FailureKind,
    ProtocolState, ScopeId, ScopeState, ScopeStatus, TreeId, WorktreeId, WorktreeState,
};
use super::model::{
    ModelState, WireAttemptId, WireBoundary, WireScopeId, WireTreeId, WireWorktreeId,
};

fn worktree(id: &str) -> WorktreeId {
    WorktreeId(id.to_string())
}

fn scope(id: &str) -> ScopeId {
    ScopeId(id.to_string())
}

fn tree(id: &str) -> TreeId {
    TreeId(id.to_string())
}

fn attempt_id(id: &str) -> AttemptId {
    AttemptId(id.to_string())
}

/// Replays a trace generated from/for `spec/mutation_cursor.qnt` through the
/// real `protocol.rs` functions.
///
/// Holds exactly the state the plan authorizes: the pure protocol state
/// (refines `worktrees`/`scopes`/`externalTaint`/`processedEvents`/
/// `attempts`/`mutationEvents`) plus `worktree_trees`, the driver-only
/// analogue of Quint's `worktreeTrees` var — the observed-tree input
/// `prepare`/`recover` take explicitly, since the pure kernel performs no Git
/// I/O. `MbtMutate` is the only action that touches `worktree_trees`; every
/// other action calls a `protocol::*` function, never reimplementing its
/// logic.
pub(super) struct MutationCursorDriver {
    protocol: ProtocolState,
    worktree_trees: BTreeMap<WorktreeId, TreeId>,
}

impl MutationCursorDriver {
    /// Exactly `spec/mutation_cursor.qnt`'s `init`: both worktrees at
    /// `Tree0`/revision `0`/healthy/no-rebaseline, all four scopes
    /// `NeverSeen` with `scopeActor`'s fixed partition (`Scope0`/`Scope1`
    /// Claude Code and `Scope2` Codex on `WT0`, `Scope3` `OpenCode` on `WT1`),
    /// and all six attempts `Available` with the same placeholder
    /// `Flush(WT0)`/revision `0`/`Tree0`/`Tree0` baseline Quint's `init`
    /// assigns every `AttemptId`.
    fn init() -> Self {
        let wt0 = worktree("wt0");
        let wt1 = worktree("wt1");

        let mut worktrees = BTreeMap::new();
        let mut worktree_trees = BTreeMap::new();
        for id in [&wt0, &wt1] {
            worktrees.insert(
                id.clone(),
                WorktreeState {
                    cursor_tree: tree("tree0"),
                    revision: 0,
                    tainted: false,
                    failure_kind: FailureKind::Healthy,
                    needs_rebaseline: false,
                },
            );
            worktree_trees.insert(id.clone(), tree("tree0"));
        }

        let scope_partition: [(&str, &WorktreeId, ActorKind); 4] = [
            ("scope0", &wt0, ActorKind::ClaudeCode),
            ("scope1", &wt0, ActorKind::ClaudeCode),
            ("scope2", &wt0, ActorKind::Codex),
            ("scope3", &wt1, ActorKind::OpenCode),
        ];
        let mut scopes = BTreeMap::new();
        for (id, owning_worktree, actor_kind) in scope_partition {
            scopes.insert(
                scope(id),
                ScopeState {
                    status: ScopeStatus::NeverSeen,
                    actor_kind,
                    worktree_id: owning_worktree.clone(),
                },
            );
        }

        let mut attempts = BTreeMap::new();
        for id in [
            "attempt0", "attempt1", "attempt2", "attempt3", "attempt4", "attempt5",
        ] {
            attempts.insert(
                attempt_id(id),
                AttemptState {
                    status: AttemptStatus::Available,
                    boundary: Boundary::Flush {
                        worktree: wt0.clone(),
                    },
                    expected_revision: 0,
                    before_tree: tree("tree0"),
                    after_tree: tree("tree0"),
                },
            );
        }

        Self {
            protocol: ProtocolState {
                worktrees,
                scopes,
                external_taint: BTreeSet::new(),
                processed_events: BTreeSet::new(),
                attempts,
                mutation_events: BTreeSet::new(),
            },
            worktree_trees,
        }
    }

    fn mbt_init(&mut self) {
        *self = Self::init();
    }

    fn mbt_mutate(&mut self, worktree: WorktreeId, tree: TreeId) {
        self.worktree_trees.insert(worktree, tree);
    }

    fn observed_tree(&self, worktree: &WorktreeId) -> TreeId {
        self.worktree_trees
            .get(worktree)
            .cloned()
            .expect("every worktree tracked since init has an observed tree")
    }

    fn mbt_prepare(&mut self, attempt: AttemptId, boundary: Boundary) {
        let worktree = boundary_worktree(&boundary, &self.protocol.scopes)
            .expect("every boundary's scope is registered by init, matching Quint's static scopeWorktree partition");
        let observed_tree = self.observed_tree(&worktree);
        self.protocol = protocol::prepare(&self.protocol, attempt, boundary, observed_tree);
    }

    fn mbt_commit(&mut self, attempt: &AttemptId) {
        self.protocol = protocol::commit(&self.protocol, attempt).state;
    }

    fn mbt_taint(&mut self, worktree: &WorktreeId) {
        self.protocol = protocol::taint(&self.protocol, worktree);
    }

    fn mbt_database_failure(&mut self, worktree: &WorktreeId) {
        self.protocol = protocol::database_failure(&self.protocol, worktree);
    }

    fn mbt_abandon(&mut self, scope: &ScopeId) {
        self.protocol = protocol::abandon(&self.protocol, scope);
    }

    fn mbt_recover(&mut self, worktree: &WorktreeId) {
        let observed_tree = self.observed_tree(worktree);
        self.protocol = protocol::recover(&self.protocol, worktree, observed_tree);
    }

    /// Refines the explicit top-level `stutter` action: no state change.
    #[allow(clippy::unused_self)]
    fn mbt_stutter(&self) {}
}

impl Default for MutationCursorDriver {
    fn default() -> Self {
        Self::init()
    }
}

impl Driver for MutationCursorDriver {
    type State = ModelState;

    fn config() -> Config {
        Config {
            state: &[],
            nondet: &["mbtAction"],
        }
    }

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            MbtInit => self.mbt_init(),
            MbtMutate(worktree: WireWorktreeId, tree: WireTreeId) =>
                self.mbt_mutate(worktree.into(), tree.into()),
            MbtPrepare(attempt: WireAttemptId, boundary: WireBoundary) =>
                self.mbt_prepare(attempt.into(), boundary.into()),
            MbtCommit(attempt: WireAttemptId) => self.mbt_commit(&attempt.into()),
            MbtTaint(worktree: WireWorktreeId) => self.mbt_taint(&worktree.into()),
            MbtDatabaseFailure(worktree: WireWorktreeId) =>
                self.mbt_database_failure(&worktree.into()),
            MbtAbandon(scope: WireScopeId) => self.mbt_abandon(&scope.into()),
            MbtRecover(worktree: WireWorktreeId) => self.mbt_recover(&worktree.into()),
            MbtStutter => self.mbt_stutter(),
        })
    }
}

impl State<MutationCursorDriver> for ModelState {
    fn from_driver(driver: &MutationCursorDriver) -> Result<Self> {
        Ok(ModelState {
            worktrees: driver.protocol.worktrees.clone(),
            scopes: driver.protocol.scopes.clone(),
            worktree_trees: driver.worktree_trees.clone(),
            external_taint: driver.protocol.external_taint.clone(),
            processed_events: driver.protocol.processed_events.clone(),
            attempts: driver.protocol.attempts.clone(),
            mutation_events: driver.protocol.mutation_events.clone(),
        })
    }
}
