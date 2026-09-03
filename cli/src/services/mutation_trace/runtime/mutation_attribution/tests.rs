use std::cell::RefCell;
use std::collections::HashMap;

use super::*;
use crate::services::mutation_trace::store::AttributionKind;
use crate::services::mutation_trace::types::{FailureKind, ScopeId};
use crate::services::patch::{FileChangeKind, PatchFileChange, PatchHunk, TouchedLine};

fn tree(id: &str) -> TreeId {
    TreeId(id.to_owned())
}

fn worktree() -> WorktreeId {
    WorktreeId("wt".to_owned())
}

fn empty() -> ParsedPatch {
    ParsedPatch { files: Vec::new() }
}

fn added(number: u64, content: &str) -> TouchedLine {
    TouchedLine {
        kind: TouchedLineKind::Added,
        line_number: number,
        content: content.to_owned(),
        session_id: None,
    }
}

fn committed(
    path: &str,
    old_start: u64,
    old_count: u64,
    new_start: u64,
    lines: Vec<TouchedLine>,
) -> ParsedPatch {
    ParsedPatch {
        files: vec![PatchFileChange {
            old_path: path.to_owned(),
            new_path: path.to_owned(),
            kind: FileChangeKind::Modified,
            hunks: vec![PatchHunk {
                old_start,
                old_count,
                new_start,
                new_count: lines.len() as u64,
                model_id: None,
                lines,
            }],
        }],
    }
}

fn page_row(
    revision: u64,
    before: &str,
    after: &str,
    kind: AttributionKind,
    scope: Option<&str>,
) -> MutationEventPageRow {
    MutationEventPageRow {
        revision,
        before_tree: tree(before),
        after_tree: tree(after),
        tainted: false,
        failure_kind: FailureKind::Healthy,
        attribution_kind: kind,
        attribution_scope_id: scope.map(|id| ScopeId(id.to_owned())),
    }
}

fn ai_row(revision: u64, before: &str, after: &str, scope: &str) -> MutationEventPageRow {
    page_row(
        revision,
        before,
        after,
        AttributionKind::AiExclusive,
        Some(scope),
    )
}

fn non_ai_row(revision: u64, before: &str, after: &str) -> MutationEventPageRow {
    page_row(revision, before, after, AttributionKind::AiContended, None)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PageRequest {
    cursor: Option<u64>,
    limit: usize,
}

struct FakePageSource {
    events: Vec<MutationEventPageRow>,
    fail_on_page: Option<usize>,
    requests: RefCell<Vec<PageRequest>>,
}

impl FakePageSource {
    fn new(events: Vec<MutationEventPageRow>) -> Self {
        Self {
            events,
            fail_on_page: None,
            requests: RefCell::new(Vec::new()),
        }
    }

    fn failing_on_page(mut self, page: usize) -> Self {
        self.fail_on_page = Some(page);
        self
    }

    fn requests(&self) -> Vec<PageRequest> {
        self.requests.borrow().clone()
    }
}

impl MutationEventPageSource for FakePageSource {
    fn load_mutation_event_page(
        &self,
        _worktree: &WorktreeId,
        revision_cursor: Option<u64>,
        requested_limit: usize,
    ) -> Result<Vec<MutationEventPageRow>> {
        let page_number = self.requests.borrow().len() + 1;
        self.requests.borrow_mut().push(PageRequest {
            cursor: revision_cursor,
            limit: requested_limit,
        });
        if self.fail_on_page == Some(page_number) {
            anyhow::bail!("injected page failure");
        }
        let start = match revision_cursor {
            None => 0,
            Some(cursor) => self
                .events
                .iter()
                .position(|event| event.revision < cursor)
                .unwrap_or(self.events.len()),
        };
        Ok(self
            .events
            .iter()
            .skip(start)
            .take(requested_limit)
            .cloned()
            .collect())
    }
}

#[derive(Default)]
struct FakeTreeSource {
    diffs: HashMap<(String, String), String>,
    files: HashMap<(String, String), String>,
    fail_diff_on: Option<usize>,
    diff_calls: RefCell<usize>,
}

impl FakeTreeSource {
    fn new() -> Self {
        Self::default()
    }

    fn with_diff(mut self, before: &str, after: &str, text: &str) -> Self {
        self.diffs
            .insert((before.to_owned(), after.to_owned()), text.to_owned());
        self
    }

    fn with_file(mut self, tree: &str, path: &str, content: &str) -> Self {
        self.files
            .insert((tree.to_owned(), path.to_owned()), content.to_owned());
        self
    }

    fn failing_diff_on(mut self, call: usize) -> Self {
        self.fail_diff_on = Some(call);
        self
    }

    fn diff_calls(&self) -> usize {
        *self.diff_calls.borrow()
    }
}

impl TreeReadSource for FakeTreeSource {
    fn diff_trees(&self, before: &TreeId, after: &TreeId) -> Result<String> {
        let call = {
            let mut calls = self.diff_calls.borrow_mut();
            *calls += 1;
            *calls
        };
        if self.fail_diff_on == Some(call) {
            anyhow::bail!("injected diff failure");
        }
        if before == after {
            return Ok(String::new());
        }
        self.diffs
            .get(&(before.0.clone(), after.0.clone()))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no canned diff {before:?} -> {after:?}"))
    }

    fn file_at_tree(&self, tree: &TreeId, path: &str) -> Result<Option<String>> {
        Ok(self.files.get(&(tree.0.clone(), path.to_owned())).cloned())
    }
}

fn ai_contents(attr: &BoundedMutationAttribution) -> Vec<String> {
    attr.result
        .mutation_ai_patch
        .files
        .iter()
        .flat_map(|file| file.hunks.iter())
        .flat_map(|hunk| hunk.lines.iter())
        .map(|line| line.content.clone())
        .collect()
}

fn unresolved_contents(attr: &BoundedMutationAttribution) -> Vec<String> {
    attr.result
        .unresolved_patch
        .files
        .iter()
        .flat_map(|file| file.hunks.iter())
        .flat_map(|hunk| hunk.lines.iter())
        .map(|line| line.content.clone())
        .collect()
}

#[test]
fn no_events_leaves_every_target_line_unresolved() {
    let page_source = FakePageSource::new(Vec::new());
    let tree_source = FakeTreeSource::new();

    let attr = resolve_bounded_mutation_attribution(
        &page_source,
        &tree_source,
        &worktree(),
        &empty(),
        &committed("f.rs", 1, 1, 1, vec![added(2, "foo")]),
        &tree("commit"),
        Some(5),
    );

    assert!(ai_contents(&attr).is_empty());
    assert_eq!(unresolved_contents(&attr), vec!["foo".to_owned()]);
    assert_eq!(
        attr.loaded_pages, 0,
        "an empty page is not counted as loaded"
    );
    assert_eq!(page_source.requests().len(), 1);
}

#[test]
fn fully_direct_covered_target_does_zero_mutation_history_work() {
    let page_source = FakePageSource::new(vec![ai_row(1, "b", "a", "s")]);
    let tree_source = FakeTreeSource::new();

    let direct = committed("f.rs", 1, 1, 1, vec![added(2, "foo")]);
    let attr = resolve_bounded_mutation_attribution(
        &page_source,
        &tree_source,
        &worktree(),
        &direct,
        &direct,
        &tree("commit"),
        Some(1),
    );

    assert!(page_source.requests().is_empty());
    assert_eq!(tree_source.diff_calls(), 0);
    assert_eq!(attr.inspected_events, 0);
    assert!(attr.result.mutation_ai_patch.files.is_empty());
}

#[test]
fn a_surviving_ai_mutation_line_is_attributed() {
    let page_source = FakePageSource::new(vec![ai_row(1, "t0", "t1", "scope-1")]);
    let tree_source = FakeTreeSource::new()
        .with_file("t0", "f.rs", "a\n")
        .with_diff(
            "t0",
            "t1",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,2 @@\n a\n+foo\n",
        );

    let attr = resolve_bounded_mutation_attribution(
        &page_source,
        &tree_source,
        &worktree(),
        &empty(),
        &committed("f.rs", 1, 1, 1, vec![added(2, "foo")]),
        &tree("t1"),
        Some(1),
    );

    assert_eq!(ai_contents(&attr), vec!["foo".to_owned()]);
    assert!(unresolved_contents(&attr).is_empty());
    assert_eq!(attr.reconstructed_events, 1);
    assert_eq!(attr.barrier, None);
}

#[test]
fn ai_mutation_survives_an_unrelated_later_mutation() {
    let page_source = FakePageSource::new(vec![
        non_ai_row(2, "t1", "t2"),
        ai_row(1, "t0", "t1", "scope-1"),
    ]);
    let tree_source = FakeTreeSource::new()
        .with_file("t0", "f.rs", "a\n")
        .with_diff(
            "t0",
            "t1",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,2 @@\n a\n+foo\n",
        )
        .with_diff(
            "t1",
            "t2",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,0 +3,1 @@\n+bar\n",
        );

    let attr = resolve_bounded_mutation_attribution(
        &page_source,
        &tree_source,
        &worktree(),
        &empty(),
        &committed("f.rs", 1, 1, 1, vec![added(2, "foo"), added(3, "bar")]),
        &tree("t2"),
        Some(2),
    );

    assert_eq!(ai_contents(&attr), vec!["foo".to_owned()]);
    let non_ai: Vec<String> = attr
        .result
        .resolved_non_ai_patch
        .files
        .iter()
        .flat_map(|file| file.hunks.iter())
        .flat_map(|hunk| hunk.lines.iter())
        .map(|line| line.content.clone())
        .collect();
    assert_eq!(non_ai, vec!["bar".to_owned()]);
}

#[test]
fn a_stale_ai_mutation_cannot_resurrect_through_an_unobserved_tail() {
    let page_source = FakePageSource::new(vec![
        non_ai_row(2, "t1", "t2"),
        ai_row(1, "t0", "t1", "scope-1"),
    ]);
    let tree_source = FakeTreeSource::new()
        .with_file("t0", "f.rs", "a\n")
        .with_diff(
            "t0",
            "t1",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,2 @@\n a\n+foo\n",
        )
        .with_diff(
            "t1",
            "t2",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +1,0 @@\n-foo\n",
        )
        .with_diff(
            "t2",
            "commit",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,2 @@\n a\n+foo\n",
        );

    let attr = resolve_bounded_mutation_attribution(
        &page_source,
        &tree_source,
        &worktree(),
        &empty(),
        &committed("f.rs", 1, 1, 1, vec![added(2, "foo")]),
        &tree("commit"),
        Some(2),
    );

    assert!(
        ai_contents(&attr).is_empty(),
        "the re-added foo must not inherit E1's dead provenance"
    );
    assert_eq!(unresolved_contents(&attr), vec!["foo".to_owned()]);
}

#[test]
fn a_non_ai_replacement_of_an_ai_line_owns_the_new_line() {
    let page_source = FakePageSource::new(vec![
        non_ai_row(2, "t1", "t2"),
        ai_row(1, "t0", "t1", "scope-1"),
    ]);
    let tree_source = FakeTreeSource::new()
        .with_file("t0", "f.rs", "head\n")
        .with_diff(
            "t0",
            "t1",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,2 @@\n head\n+foo = 1\n",
        )
        .with_diff(
            "t1",
            "t2",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-foo = 1\n+foo = 2\n",
        );

    let attr = resolve_bounded_mutation_attribution(
        &page_source,
        &tree_source,
        &worktree(),
        &empty(),
        &committed("f.rs", 1, 1, 1, vec![added(2, "foo = 2")]),
        &tree("t2"),
        Some(2),
    );

    assert!(ai_contents(&attr).is_empty());
    let non_ai: Vec<String> = attr
        .result
        .resolved_non_ai_patch
        .files
        .iter()
        .flat_map(|file| file.hunks.iter())
        .flat_map(|hunk| hunk.lines.iter())
        .map(|line| line.content.clone())
        .collect();
    assert_eq!(non_ai, vec!["foo = 2".to_owned()]);
}

#[test]
fn a_history_gap_is_not_crossed_by_older_provenance() {
    let page_source = FakePageSource::new(vec![
        ai_row(2, "t9", "commit", "scope-2"),
        ai_row(1, "t0", "t1", "scope-1"),
    ]);
    let tree_source = FakeTreeSource::new()
        .with_file("t0", "f.rs", "a\n")
        .with_file("t9", "f.rs", "a\nfoo\n")
        .with_diff(
            "t0",
            "t1",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,2 @@\n a\n+foo\n",
        )
        .with_diff(
            "t9",
            "commit",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,0 +3,1 @@\n+bar\n",
        );

    let attr = resolve_bounded_mutation_attribution(
        &page_source,
        &tree_source,
        &worktree(),
        &empty(),
        &committed("f.rs", 1, 2, 1, vec![added(2, "foo"), added(3, "bar")]),
        &tree("commit"),
        Some(2),
    );

    assert_eq!(ai_contents(&attr), vec!["bar".to_owned()]);
    assert_eq!(unresolved_contents(&attr), vec!["foo".to_owned()]);
    assert_eq!(attr.gap_resets, 1);
}

#[test]
fn bounded_history_baseline_starts_unknown() {
    let page_source = FakePageSource::new(vec![non_ai_row(2, "t1", "t2")]);
    let tree_source = FakeTreeSource::new()
        .with_file("t1", "f.rs", "a\nfoo\n")
        .with_diff(
            "t1",
            "t2",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,0 +3,1 @@\n+bar\n",
        );

    let attr = resolve_bounded_mutation_attribution(
        &page_source,
        &tree_source,
        &worktree(),
        &empty(),
        &committed("f.rs", 1, 2, 1, vec![added(2, "foo")]),
        &tree("t2"),
        Some(2),
    );

    assert!(ai_contents(&attr).is_empty());
    assert_eq!(unresolved_contents(&attr), vec!["foo".to_owned()]);
}

#[test]
fn an_unobserved_tail_adds_unknown_lines_but_keeps_surviving_ai_lines() {
    let page_source = FakePageSource::new(vec![ai_row(1, "t0", "t1", "scope-1")]);
    let tree_source = FakeTreeSource::new()
        .with_file("t0", "f.rs", "a\n")
        .with_diff(
            "t0",
            "t1",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,2 @@\n a\n+ai_line\n",
        )
        .with_diff(
            "t1",
            "commit",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,0 +3,1 @@\n+human_line\n",
        );

    let attr = resolve_bounded_mutation_attribution(
        &page_source,
        &tree_source,
        &worktree(),
        &empty(),
        &committed(
            "f.rs",
            1,
            1,
            1,
            vec![added(2, "ai_line"), added(3, "human_line")],
        ),
        &tree("commit"),
        Some(1),
    );

    assert_eq!(ai_contents(&attr), vec!["ai_line".to_owned()]);
    assert_eq!(unresolved_contents(&attr), vec!["human_line".to_owned()]);
}

#[test]
fn an_event_after_the_commit_cut_has_no_influence() {
    let page_source = FakePageSource::new(vec![
        ai_row(2, "t0", "t1", "scope-after"),
        non_ai_row(1, "t0", "t0"),
    ]);
    let tree_source = FakeTreeSource::new()
        .with_file("t0", "f.rs", "a\nfoo\n")
        .with_diff(
            "t0",
            "t1",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,0 +2,1 @@\n+foo\n",
        );

    let attr = resolve_bounded_mutation_attribution(
        &page_source,
        &tree_source,
        &worktree(),
        &empty(),
        &committed("f.rs", 1, 2, 1, vec![added(2, "foo")]),
        &tree("t0"),
        Some(1),
    );

    assert!(
        ai_contents(&attr).is_empty(),
        "an event past the cut must not attribute"
    );
    assert_eq!(page_source.requests()[0].cursor, Some(2));
}

#[test]
fn event_128_within_the_horizon_contributes_and_event_129_is_never_loaded() {
    let total = 200u64;
    let relevant = total - 127;
    let mut events = Vec::new();
    for revision in (1..=total).rev() {
        if revision == relevant {
            events.push(ai_row(revision, "t0", "t1", "scope-128"));
        } else {
            events.push(non_ai_row(revision, "t1", "t1"));
        }
    }
    let tree_source = FakeTreeSource::new()
        .with_file("t0", "f.rs", "a\n")
        .with_diff(
            "t0",
            "t1",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,2 @@\n a\n+target\n",
        );
    let page_source = FakePageSource::new(events);

    let attr = resolve_bounded_mutation_attribution(
        &page_source,
        &tree_source,
        &worktree(),
        &empty(),
        &committed("f.rs", 1, 1, 1, vec![added(2, "target")]),
        &tree("t1"),
        None,
    );

    assert_eq!(ai_contents(&attr), vec!["target".to_owned()]);
    assert_eq!(attr.inspected_events, 128);
    assert_eq!(attr.loaded_pages, 4);
    assert_eq!(attr.loaded_rows, 128);
    assert_eq!(attr.barrier, None);
}

#[test]
fn a_page_query_failure_is_a_conservative_barrier() {
    let mut events = Vec::new();
    for revision in (1..=40).rev() {
        events.push(non_ai_row(revision, "t1", "t1"));
    }
    let tree_source = FakeTreeSource::new().with_file("t1", "f.rs", "a\n");
    let page_source = FakePageSource::new(events).failing_on_page(2);

    let attr = resolve_bounded_mutation_attribution(
        &page_source,
        &tree_source,
        &worktree(),
        &empty(),
        &committed("f.rs", 1, 1, 1, vec![added(2, "foo")]),
        &tree("t1"),
        None,
    );

    assert_eq!(attr.barrier, Some(MutationAttributionBarrier::PageQuery));
    assert_eq!(attr.loaded_rows, 32);
    assert_eq!(unresolved_contents(&attr), vec!["foo".to_owned()]);
}

#[test]
fn a_reconstruction_failure_reloads_a_conservative_baseline_and_keeps_going() {
    let page_source = FakePageSource::new(vec![
        ai_row(2, "t1", "t2", "scope-2"),
        ai_row(1, "t0", "t1", "scope-1"),
    ]);
    let tree_source = FakeTreeSource::new()
        .with_file("t0", "f.rs", "a\n")
        .with_file("t1", "f.rs", "a\n")
        .with_diff(
            "t1",
            "t2",
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,2 @@\n a\n+foo\n",
        )
        .failing_diff_on(1);

    let attr = resolve_bounded_mutation_attribution(
        &page_source,
        &tree_source,
        &worktree(),
        &empty(),
        &committed("f.rs", 1, 1, 1, vec![added(2, "foo")]),
        &tree("t2"),
        Some(2),
    );

    assert_eq!(
        attr.barrier,
        Some(MutationAttributionBarrier::EventReconstruction)
    );
    assert_eq!(ai_contents(&attr), vec!["foo".to_owned()]);
}

#[test]
fn real_git_snapshot_and_store_satisfy_the_consumer_seams() {
    use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
    use crate::services::mutation_trace::store::encode_revision;
    use std::process::Command;

    let temp = tempfile::Builder::new()
        .prefix("sce-lineage-consumer-")
        .tempdir()
        .expect("temp dir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("repo dir");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&repo_root)
            .output()
            .expect("git spawns");
        assert!(output.status.success(), "git {args:?} failed");
    };
    git(&["init", "--quiet"]);
    git(&["config", "user.email", "t@example.com"]);
    git(&["config", "user.name", "Test"]);
    git(&["commit", "--allow-empty", "--quiet", "-m", "init"]);

    let snapshot = GitSnapshotService::new(&repo_root).expect("snapshot service");
    std::fs::write(repo_root.join("file.rs"), b"one\n").expect("write");
    let before = snapshot.capture_tree().expect("capture before");
    std::fs::write(repo_root.join("file.rs"), b"one\ntwo\n").expect("write");
    let after = snapshot.capture_tree().expect("capture after");

    let db = RepositoryAgentTraceDb::new_at(temp.path().join("agent-trace.db")).expect("db opens");
    db.execute(
        "INSERT INTO mutation_trace_events
            (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
             attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id)
         VALUES (?1, ?2, ?3, ?4, 0, 'healthy', 'ai_exclusive', 'scope-real', 'flush', NULL, NULL)",
        (
            "wt-real",
            encode_revision(1).as_slice(),
            before.0.as_str(),
            after.0.as_str(),
        ),
    )
    .expect("event insert");
    let store = MutationTraceStore::new(&db);

    let attr = resolve_bounded_mutation_attribution(
        &store,
        &snapshot,
        &WorktreeId("wt-real".to_owned()),
        &empty(),
        &committed("file.rs", 1, 1, 1, vec![added(2, "two")]),
        &after,
        Some(1),
    );

    assert_eq!(ai_contents(&attr), vec!["two".to_owned()]);
    assert_eq!(attr.reconstructed_events, 1);
    assert_eq!(attr.barrier, None);
}

fn init_repo_with_commit(repo_root: &std::path::Path) {
    use std::process::Command;
    std::fs::create_dir_all(repo_root).expect("repo dir");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .output()
            .expect("git spawns");
        assert!(output.status.success(), "git {args:?} failed");
    };
    git(&["init", "--quiet"]);
    git(&["config", "user.email", "t@example.com"]);
    git(&["config", "user.name", "Test"]);
    std::fs::write(repo_root.join("seed"), b"seed\n").expect("seed");
    git(&["add", "-A"]);
    git(&["commit", "--quiet", "-m", "init"]);
}

#[test]
fn post_commit_entry_point_without_checkout_identity_yields_empty_and_creates_none() {
    use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;

    let temp = tempfile::Builder::new()
        .prefix("sce-post-commit-lineage-no-identity-")
        .tempdir()
        .expect("temp dir");
    let repo_root = temp.path().join("repo");
    init_repo_with_commit(&repo_root);

    let git_dir = resolve_git_dir(&repo_root).expect("git dir");
    let checkout_id_path = git_dir.join("sce").join("checkout-id");
    assert!(!checkout_id_path.exists());

    let db = RepositoryAgentTraceDb::new_at(temp.path().join("agent-trace.db")).expect("db opens");

    let result = resolve_post_commit_mutation_ai_patch(
        &repo_root,
        &db,
        &empty(),
        &committed("file.rs", 1, 1, 1, vec![added(2, "two")]),
    );

    assert!(result.files.is_empty());
    assert!(!checkout_id_path.exists());
}

#[test]
fn post_commit_entry_point_resolves_current_worktree_and_ignores_foreign_rows() {
    use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
    use crate::services::checkout::get_or_create_checkout_id;
    use crate::services::mutation_trace::store::encode_revision;

    let temp = tempfile::Builder::new()
        .prefix("sce-post-commit-lineage-current-")
        .tempdir()
        .expect("temp dir");
    let repo_root = temp.path().join("repo");
    init_repo_with_commit(&repo_root);

    let git_dir = resolve_git_dir(&repo_root).expect("git dir");
    let checkout_id = get_or_create_checkout_id(&git_dir).expect("checkout id");

    let snapshot = GitSnapshotService::new(&repo_root).expect("snapshot service");
    std::fs::write(repo_root.join("file.rs"), b"one\n").expect("write");
    let before = snapshot.capture_tree().expect("capture before");
    std::fs::write(repo_root.join("file.rs"), b"one\ntwo\n").expect("write");
    let after = snapshot.capture_tree().expect("capture after");

    std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(&repo_root)
        .output()
        .expect("git add");
    std::process::Command::new("git")
        .args(["commit", "--quiet", "-m", "two"])
        .current_dir(&repo_root)
        .output()
        .expect("git commit");

    let db = RepositoryAgentTraceDb::new_at(temp.path().join("agent-trace.db")).expect("db opens");
    for (worktree_id, revision, scope) in [
        (checkout_id.as_str(), 1u64, "scope-current"),
        ("wt-foreign", 2u64, "scope-foreign"),
    ] {
        db.execute(
            "INSERT INTO mutation_trace_events
                (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
                 attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id)
             VALUES (?1, ?2, ?3, ?4, 0, 'healthy', 'ai_exclusive', ?5, 'flush', NULL, NULL)",
            (
                worktree_id,
                encode_revision(revision).as_slice(),
                before.0.as_str(),
                after.0.as_str(),
                scope,
            ),
        )
        .expect("event insert");
    }

    let result = resolve_post_commit_mutation_ai_patch(
        &repo_root,
        &db,
        &empty(),
        &committed("file.rs", 1, 1, 1, vec![added(2, "two")]),
    );

    let contents: Vec<String> = result
        .files
        .iter()
        .flat_map(|file| file.hunks.iter())
        .flat_map(|hunk| hunk.lines.iter())
        .map(|line| line.content.clone())
        .collect();
    assert_eq!(contents, vec!["two".to_owned()]);
}
