use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::super::super::git_snapshot::{resolve_worktree_id, GitSnapshotService};
use super::super::super::maintenance_state::write_state_atomically;
use super::super::super::ref_reconciliation::{ReconcileError, ReconciliationReport};
use super::super::{open_authoritative_db_at_state_root, reconcile_explicit_with, ExplicitOutcome};
use super::support::{
    coordinate_flush, hook_ok, pinned_trees, sce_refs, start_payload, write_file, writer,
};
use super::{git, pin_orphan, Fixture, NOW};
use crate::services::mutation_trace::store::MutationTraceStore;
use crate::services::mutation_trace::types::WorktreeId;
use crate::services::repository_identity::resolve::resolve_repository_identity;

const REMOTE: &str = "https://example.invalid/org/repo-a.git";

struct Linked {
    fixture: Fixture,
    root: PathBuf,
    id: String,
}

impl Linked {
    async fn new() -> Self {
        let fixture = Fixture::new(REMOTE);
        let root = fixture.dir.path().join("linked");
        git(
            &fixture.root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "linked-branch",
                root.to_str().expect("utf-8 path"),
            ],
        );
        let main_identity = resolve_repository_identity(&fixture.root, None, "origin")
            .expect("main identity")
            .identity
            .repository_id;
        let linked_identity = resolve_repository_identity(&root, None, "origin")
            .expect("linked identity")
            .identity
            .repository_id;
        assert_eq!(main_identity, linked_identity);
        assert_eq!(
            resolve_worktree_id(&fixture.root).await.expect("main id"),
            WorktreeId("main".to_string())
        );
        let id = resolve_worktree_id(&root).await.expect("linked id").0;
        assert!(id.starts_with("worktrees/"));
        fixture.create_db().await;
        Self { fixture, root, id }
    }

    async fn run(&self, root: &Path) -> ExplicitOutcome {
        reconcile_explicit_with(
            root,
            async || open_authoritative_db_at_state_root(root, &self.fixture.state_root).await,
            || NOW,
            write_state_atomically,
            |_| {},
            Duration::from_secs(10),
        )
        .await
    }

    async fn linked_hook(
        &self,
        payload: crate::services::hooks::mutation_scope::MutationScopePayload,
    ) {
        super::support::run_hook(
            &self.root,
            &self.fixture.db_path(),
            payload,
            super::super::super::protected_worktree::WORKTREE_LOCK_TIMEOUT,
            || {},
            || {},
            super::support::no_advice,
        )
        .await
        .expect("linked hook completes");
    }

    fn linked_pins(&self) -> BTreeSet<String> {
        pinned_trees(&self.root, &self.id)
    }

    fn main_pins(&self) -> BTreeSet<String> {
        pinned_trees(&self.fixture.root, "main")
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_historical_roots_of_a_real_coordinator_sequence_stay_protected() {
    let linked = Linked::new().await;
    let fixture = &linked.fixture;
    hook_ok(fixture, start_payload("scope-1", "start-1")).await;
    let db_path = fixture.db_path();

    let mut trees = BTreeSet::new();
    let mut store_db = writer(&db_path).await.expect("db");
    let cursor = MutationTraceStore::new(&mut store_db)
        .load_worktree(&WorktreeId("main".to_string()), None, None)
        .await
        .expect("load")
        .expect("row")
        .worktree_state
        .cursor_tree
        .0;
    trees.insert(cursor);
    for (name, content) in [("b.txt", "b\n"), ("c.txt", "c\n"), ("d.txt", "d\n")] {
        write_file(&fixture.root, name, content);
        let outcome = coordinate_flush(&fixture.root, &db_path).await;
        trees.insert(outcome.observed_tree.0);
    }
    assert_eq!(trees.len(), 4);
    assert_eq!(linked.main_pins(), trees);

    write_file(&fixture.root, "orphan.txt", "orphan\n");
    let orphan = pin_orphan(&fixture.root).await;
    assert!(!trees.contains(&orphan));

    let outcome = linked.run(&fixture.root).await;
    let ExplicitOutcome::Completed(report) = outcome else {
        panic!("expected completed pass, got {outcome:?}");
    };
    assert_eq!(
        report,
        ReconciliationReport {
            local_required: 4,
            retained: 4,
            deleted: 1,
        }
    );
    assert_eq!(
        linked.main_pins(),
        trees,
        "historical A, B, C and D are all retained although only D is the cursor"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_pin_of_one_real_worktree_is_retained_while_another_needs_the_tree() {
    let linked = Linked::new().await;
    let fixture = &linked.fixture;

    write_file(&fixture.root, "shared.txt", "shared\n");
    let shared = pin_orphan(&fixture.root).await;
    write_file(&linked.root, "shared.txt", "shared\n");
    linked
        .linked_hook(start_payload("linked-scope", "linked-start"))
        .await;
    assert_eq!(linked.linked_pins(), [shared.clone()].into_iter().collect());

    write_file(&fixture.root, "orphan.txt", "orphan\n");
    let orphan = pin_orphan(&fixture.root).await;
    assert_eq!(
        linked.main_pins(),
        [shared.clone(), orphan.clone()].into_iter().collect()
    );

    let outcome = linked.run(&fixture.root).await;
    let ExplicitOutcome::Completed(report) = outcome else {
        panic!("expected completed pass, got {outcome:?}");
    };
    assert_eq!(
        report,
        ReconciliationReport {
            local_required: 0,
            retained: 1,
            deleted: 1,
        }
    );
    assert_eq!(
        linked.main_pins(),
        [shared.clone()].into_iter().collect(),
        "main's pin is the same tree the linked worktree durably requires"
    );
    assert_eq!(linked.linked_pins(), [shared].into_iter().collect());
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_reconciliation_missing_pin_in_another_real_worktree_neither_fails_nor_is_repaired() {
    let linked = Linked::new().await;
    let fixture = &linked.fixture;
    let db_path = fixture.db_path();

    write_file(&fixture.root, "shared.txt", "shared\n");
    let shared = pin_orphan(&fixture.root).await;
    write_file(&linked.root, "shared.txt", "shared\n");
    linked
        .linked_hook(start_payload("linked-scope", "linked-start"))
        .await;
    write_file(&linked.root, "more.txt", "more\n");
    let flushed = coordinate_flush(&linked.root, &db_path).await;
    let later = flushed.observed_tree.0;
    assert_eq!(
        linked.linked_pins(),
        [shared.clone(), later.clone()].into_iter().collect()
    );

    let linked_ref = format!("refs/sce/mutation-cursor/{}/{shared}", linked.id);
    git(&linked.root, &["update-ref", "-d", &linked_ref]);
    assert_eq!(linked.linked_pins(), [later.clone()].into_iter().collect());

    write_file(&fixture.root, "orphan.txt", "orphan\n");
    let orphan = pin_orphan(&fixture.root).await;
    let refs_of_linked_before: Vec<String> = sce_refs(&fixture.root)
        .into_iter()
        .filter(|line| line.contains(&linked.id))
        .collect();

    let outcome = linked.run(&fixture.root).await;
    let ExplicitOutcome::Completed(report) = outcome else {
        panic!("expected completed pass, got {outcome:?}");
    };
    assert_eq!(
        report,
        ReconciliationReport {
            local_required: 0,
            retained: 1,
            deleted: 1,
        }
    );
    assert!(!linked.main_pins().contains(&orphan));
    assert_eq!(
        linked.main_pins(),
        [shared.clone()].into_iter().collect(),
        "the remaining protection for the other worktree's tree is retained"
    );
    let refs_of_linked_after: Vec<String> = sce_refs(&fixture.root)
        .into_iter()
        .filter(|line| line.contains(&linked.id))
        .collect();
    assert_eq!(refs_of_linked_before, refs_of_linked_after);
    assert_eq!(
        linked.linked_pins(),
        [later].into_iter().collect(),
        "reconciling main never recreates the other worktree's missing pin"
    );

    let degraded = linked.run(&linked.root).await;
    let ExplicitOutcome::Failed { error, .. } = degraded else {
        panic!("the degraded worktree fails closed in its own pass, got {degraded:?}");
    };
    let ReconcileError::MissingRequiredPins { missing } = error else {
        panic!("expected missing required pins, got {error:?}");
    };
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].0, shared);
    assert!(GitSnapshotService::new(&linked.root)
        .await
        .expect("snapshot")
        .list_pins(&WorktreeId(linked.id.clone()))
        .await
        .expect("pins")
        .iter()
        .all(|pin| pin.tree.0 != shared));
}
