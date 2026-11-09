use std::process::Command;

use super::*;
use crate::services::agent_trace::{
    build_agent_trace_from_evidence, AgentTrace, AgentTraceEvidence, AgentTraceMetadataInput,
    ContributorType, DirectEvidenceSource, RepositoryRoot, UnverifiedCommittedFiles,
};

const PLAN_PATH: &str = "plan.md";
const BOILERPLATE_LINE: &str = "  - Verify:";
const AUTHOR_SCOPE: &str = "scope-author";
const AUTHOR_SESSION: &str = "session-author";
const AUTHOR_MODEL: &str = "model-author";
const OLD_SESSION: &str = "session-old";
const OLD_MODEL: &str = "model-old";
const WORKTREE: &str = "main";
const COMMIT_TIMESTAMP: &str = "2026-11-06T12:23:44Z";

struct VecPages {
    rows: Vec<MutationEventPageRow>,
}

impl MutationEventPageSource for VecPages {
    async fn load_mutation_event_page(
        &self,
        _worktree: &WorktreeId,
        revision_cursor: Option<u64>,
        requested_limit: usize,
    ) -> Result<Vec<MutationEventPageRow>> {
        let mut rows: Vec<_> = self
            .rows
            .iter()
            .filter(|row| revision_cursor.is_none_or(|cursor| row.revision < cursor))
            .cloned()
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.revision));
        rows.truncate(requested_limit.min(MUTATION_ATTRIBUTION_PAGE_SIZE));
        Ok(rows)
    }

    async fn load_scope_provenance(&self, scope_id: &ScopeId) -> Result<Option<ScopeProvenance>> {
        Ok((scope_id.0 == AUTHOR_SCOPE).then(|| ScopeProvenance {
            scope_id: scope_id.clone(),
            session_id: AUTHOR_SESSION.to_owned(),
            model_id: Some(AUTHOR_MODEL.to_owned()),
        }))
    }
}

struct TreeRepo {
    dir: tempfile::TempDir,
}

impl TreeRepo {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = TreeRepo { dir };
        repo.git(&["init", "-q", "-b", "main"]);
        repo
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(self.root())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("git output is UTF-8")
            .trim()
            .to_owned()
    }

    fn tree(&self, plan: Option<&str>) -> TreeId {
        let path = self.root().join(PLAN_PATH);
        match plan {
            Some(content) => std::fs::write(&path, content).expect("write plan"),
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
        self.git(&["add", "-A"]);
        TreeId(self.git(&["write-tree"]))
    }

    fn trees_with(&self, files: &[(&str, Option<&str>)]) -> TreeId {
        for (path, content) in files {
            let path = self.root().join(path);
            match content {
                Some(content) => std::fs::write(&path, content).expect("write file"),
                None => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        self.git(&["add", "-A"]);
        TreeId(self.git(&["write-tree"]))
    }

    fn commit(&self, message: &str) -> String {
        self.git(&[
            "-c",
            "user.name=Attribution Test",
            "-c",
            "user.email=attribution@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            message,
        ]);
        self.git(&["rev-parse", "HEAD"])
    }

    async fn snapshot(&self) -> GitSnapshotService {
        GitSnapshotService::new(self.root())
            .await
            .expect("snapshot service")
    }
}

fn event(revision: u64, before: &TreeId, after: &TreeId, ai: bool) -> MutationEventPageRow {
    MutationEventPageRow {
        revision,
        before_tree: before.clone(),
        after_tree: after.clone(),
        failure_kind: FailureKind::Healthy,
        attribution_kind: if ai {
            AttributionKind::AiExclusive
        } else {
            AttributionKind::IneligibleUnscoped
        },
        attribution_scope_id: ai.then(|| ScopeId(AUTHOR_SCOPE.to_owned())),
    }
}

async fn committed_patch(
    snapshot: &GitSnapshotService,
    parent: &TreeId,
    commit: &TreeId,
) -> ParsedPatch {
    let text = snapshot.diff_trees(parent, commit).await.expect("diff");
    parse_patch(&text, None).expect("committed patch parses")
}

async fn resolve(
    repo: &TreeRepo,
    rows: Vec<MutationEventPageRow>,
    committed: &ParsedPatch,
    commit_tree: &TreeId,
    ceiling: Option<u64>,
) -> BoundedMutationAttribution {
    resolve_from(repo, rows, committed, commit_tree, None, ceiling).await
}

async fn resolve_from(
    repo: &TreeRepo,
    rows: Vec<MutationEventPageRow>,
    committed: &ParsedPatch,
    commit_tree: &TreeId,
    parent_tree: Option<&TreeId>,
    ceiling: Option<u64>,
) -> BoundedMutationAttribution {
    resolve_bounded_mutation_attribution(
        &VecPages { rows },
        &repo.snapshot().await,
        &WorktreeId(WORKTREE.to_owned()),
        committed,
        commit_tree,
        parent_tree,
        ceiling,
    )
    .await
}

fn old_session_historical_match() -> ParsedPatch {
    let mut patch = parse_patch(
        &format!(
            "diff --git a/{PLAN_PATH} b/{PLAN_PATH}\n--- a/{PLAN_PATH}\n+++ b/{PLAN_PATH}\n@@ -1,0 +1,1 @@\n+{BOILERPLATE_LINE}\n"
        ),
        Some(OLD_SESSION),
    )
    .expect("historical candidate parses");
    patch.files[0].hunks[0].model_id = Some(OLD_MODEL.to_owned());
    patch
}

fn build_trace(
    committed: &ParsedPatch,
    attribution: &MutationAttributionResult,
    direct_patches: &[ParsedPatch],
) -> AgentTrace {
    let direct_sources: Vec<_> = direct_patches
        .iter()
        .map(DirectEvidenceSource::repository_relative)
        .collect();
    build_agent_trace_from_evidence(
        AgentTraceEvidence {
            repository_root: &RepositoryRoot::new(std::path::Path::new("/workspace/project")),
            direct_sources: &direct_sources,
            mutation_ai_patch: &attribution.mutation_ai_patch,
            mutation_non_ai_patch: &attribution.resolved_non_ai_patch,
        },
        committed,
        AgentTraceMetadataInput {
            commit_timestamp: COMMIT_TIMESTAMP,
            commit_revision: "0000000000000000000000000000000000000001",
            vcs_type: None,
        },
        &UnverifiedCommittedFiles,
    )
    .expect("agent trace builds")
}

fn added_line_numbers(patch: &ParsedPatch) -> Vec<u64> {
    patch
        .files
        .iter()
        .flat_map(|file| file.hunks.iter())
        .flat_map(|hunk| hunk.lines.iter())
        .filter(|line| line.kind == TouchedLineKind::Added)
        .map(|line| line.line_number)
        .collect()
}

fn assert_barrier_keeps_line_unknown(
    attribution: &BoundedMutationAttribution,
    committed: &ParsedPatch,
) {
    assert!(added_line_numbers(&attribution.result.mutation_ai_patch).is_empty());
    assert_eq!(
        added_line_numbers(&attribution.result.unresolved_patch),
        vec![3]
    );

    let trace = build_trace(
        committed,
        &attribution.result,
        &[old_session_historical_match()],
    );
    let conversation = &trace.files[0].conversations[0];
    assert_eq!(
        conversation
            .contributor
            .as_ref()
            .expect("conversation contributor")
            .kind,
        ContributorType::Unknown
    );
    assert_eq!(
        conversation
            .contributor
            .as_ref()
            .expect("conversation contributor")
            .model_id,
        None
    );
    assert_eq!(conversation.related, None);
    let json = serde_json::to_string(&trace).expect("trace serializes");
    assert!(!json.contains(OLD_SESSION));
    assert!(!json.contains(OLD_MODEL));
}

const BASE: &str = "a\nb\n";
const WITH_LINE: &str = "a\nb\n  - Verify:\n";

#[tokio::test]
async fn an_observed_ai_event_attributes_the_line_over_an_old_historical_match() {
    let repo = TreeRepo::new();
    let base = repo.tree(Some(BASE));
    let commit = repo.tree(Some(WITH_LINE));
    let snapshot = repo.snapshot().await;
    let committed = committed_patch(&snapshot, &base, &commit).await;

    let attribution = resolve(
        &repo,
        vec![event(1, &base, &commit, true)],
        &committed,
        &commit,
        Some(1),
    )
    .await;
    assert_eq!(attribution.barrier, None);
    assert_eq!(
        added_line_numbers(&attribution.result.mutation_ai_patch),
        vec![3]
    );

    let trace = build_trace(
        &committed,
        &attribution.result,
        &[old_session_historical_match()],
    );
    let conversation = &trace.files[0].conversations[0];
    assert_eq!(
        conversation
            .contributor
            .as_ref()
            .expect("conversation contributor")
            .kind,
        ContributorType::Ai
    );
    assert_eq!(
        conversation
            .contributor
            .as_ref()
            .expect("conversation contributor")
            .model_id
            .as_deref(),
        Some(AUTHOR_MODEL)
    );
    let related: Vec<_> = conversation
        .related
        .as_ref()
        .expect("related sessions")
        .iter()
        .map(|related| related.url.clone())
        .collect();
    assert_eq!(
        related,
        vec![crate::services::agent_trace::agent_trace_session_url(
            AUTHOR_SESSION
        )]
    );
}

#[tokio::test]
async fn ai_edits_to_a_deleted_and_recreated_file_keep_mutation_provenance() {
    let repo = TreeRepo::new();
    let original = repo.tree(Some(BASE));
    let deleted = repo.tree(None);
    let recreated = repo.tree(Some(BASE));
    let commit = repo.tree(Some(WITH_LINE));
    let snapshot = repo.snapshot().await;
    let committed = committed_patch(&snapshot, &recreated, &commit).await;

    let attribution = resolve(
        &repo,
        vec![
            event(1, &original, &deleted, false),
            event(2, &deleted, &recreated, false),
            event(3, &recreated, &commit, true),
        ],
        &committed,
        &commit,
        Some(3),
    )
    .await;

    assert_eq!(attribution.barrier, None);
    assert_eq!(attribution.gap_resets, 0);
    assert_eq!(
        added_line_numbers(&attribution.result.mutation_ai_patch),
        vec![3]
    );
    assert!(added_line_numbers(&attribution.result.unresolved_patch).is_empty());
}

#[tokio::test]
async fn an_unobserved_tail_keeps_the_line_unknown() {
    let repo = TreeRepo::new();
    let base = repo.tree(Some(BASE));
    let commit = repo.tree(Some(WITH_LINE));
    let snapshot = repo.snapshot().await;
    let committed = committed_patch(&snapshot, &base, &commit).await;

    let attribution = resolve(
        &repo,
        vec![event(1, &base, &base, true)],
        &committed,
        &commit,
        Some(1),
    )
    .await;

    assert_eq!(attribution.barrier, None);
    assert_barrier_keeps_line_unknown(&attribution, &committed);
}

#[tokio::test]
async fn a_history_gap_keeps_the_line_unknown() {
    let repo = TreeRepo::new();
    let base = repo.tree(Some(BASE));
    let observed = repo.tree(Some(WITH_LINE));
    let diverged = repo.tree(Some("a\nb\n  - Verify:\nz\n"));
    let snapshot = repo.snapshot().await;
    let committed = committed_patch(&snapshot, &base, &observed).await;

    let attribution = resolve(
        &repo,
        vec![
            event(1, &base, &observed, true),
            event(2, &diverged, &observed, false),
        ],
        &committed,
        &observed,
        Some(2),
    )
    .await;

    assert_eq!(attribution.gap_resets, 1);
    assert_barrier_keeps_line_unknown(&attribution, &committed);
}

#[tokio::test]
async fn an_event_outside_the_replay_window_keeps_the_line_unknown() {
    let repo = TreeRepo::new();
    let base = repo.tree(Some(BASE));
    let commit = repo.tree(Some(WITH_LINE));
    let snapshot = repo.snapshot().await;
    let committed = committed_patch(&snapshot, &base, &commit).await;

    let newest = MAX_MUTATION_ATTRIBUTION_EVENTS as u64 + 1;
    let mut rows = vec![event(1, &base, &commit, true)];
    rows.extend((2..=newest).map(|revision| event(revision, &commit, &commit, false)));

    let attribution = resolve(&repo, rows, &committed, &commit, Some(newest)).await;

    assert_eq!(attribution.loaded_rows, MAX_MUTATION_ATTRIBUTION_EVENTS);
    assert_barrier_keeps_line_unknown(&attribution, &committed);
}

#[tokio::test]
async fn an_event_beyond_the_revision_ceiling_keeps_the_line_unknown() {
    let repo = TreeRepo::new();
    let base = repo.tree(Some(BASE));
    let commit = repo.tree(Some(WITH_LINE));
    let snapshot = repo.snapshot().await;
    let committed = committed_patch(&snapshot, &base, &commit).await;

    let attribution = resolve(
        &repo,
        vec![
            event(1, &base, &base, false),
            event(2, &base, &commit, true),
        ],
        &committed,
        &commit,
        Some(1),
    )
    .await;

    assert_eq!(attribution.loaded_rows, 1);
    assert_barrier_keeps_line_unknown(&attribution, &committed);
}

fn removed_line_numbers(patch: &ParsedPatch) -> Vec<u64> {
    patch
        .files
        .iter()
        .flat_map(|file| file.hunks.iter())
        .flat_map(|hunk| hunk.lines.iter())
        .filter(|line| line.kind == TouchedLineKind::Removed)
        .map(|line| line.line_number)
        .collect()
}

fn removed_in(patch: &ParsedPatch, old_path: &str) -> Vec<u64> {
    removed_line_numbers(&ParsedPatch {
        files: patch
            .files
            .iter()
            .filter(|file| file.old_path == old_path)
            .cloned()
            .collect(),
    })
}

const ABC: &str = "a\nb\nc\n";
const AC: &str = "a\nc\n";

struct RemovalCase {
    repo: TreeRepo,
    parent: TreeId,
    commit: TreeId,
    committed: ParsedPatch,
}

async fn removal_case(parent: &str, commit: &str) -> RemovalCase {
    let repo = TreeRepo::new();
    let parent = repo.tree(Some(parent));
    let commit = repo.tree(Some(commit));
    let committed = committed_patch(&repo.snapshot().await, &parent, &commit).await;
    RemovalCase {
        repo,
        parent,
        commit,
        committed,
    }
}

impl RemovalCase {
    async fn resolve(&self, rows: Vec<MutationEventPageRow>) -> BoundedMutationAttribution {
        let ceiling = rows.iter().map(|row| row.revision).max();
        resolve_from(
            &self.repo,
            rows,
            &self.committed,
            &self.commit,
            Some(&self.parent),
            ceiling,
        )
        .await
    }
}

fn assert_removal_unresolved(attribution: &BoundedMutationAttribution, expected: &[u64]) {
    assert!(removed_line_numbers(&attribution.result.mutation_ai_patch).is_empty());
    assert!(removed_line_numbers(&attribution.result.resolved_non_ai_patch).is_empty());
    assert_eq!(
        removed_line_numbers(&attribution.result.unresolved_patch),
        expected
    );
}

#[tokio::test]
async fn a_proven_ai_exclusive_removal_is_attributed_with_scope_provenance() {
    let case = removal_case(ABC, AC).await;
    let attribution = case
        .resolve(vec![event(1, &case.parent, &case.commit, true)])
        .await;

    assert!(attribution.removal_replay_anchored);
    assert_eq!(
        removed_line_numbers(&attribution.result.mutation_ai_patch),
        vec![2]
    );
    let hunk = &attribution.result.mutation_ai_patch.files[0].hunks[0];
    assert_eq!(hunk.lines[0].content, "b");
    assert_eq!(hunk.lines[0].session_id.as_deref(), Some(AUTHOR_SESSION));
    assert_eq!(hunk.model_id.as_deref(), Some(AUTHOR_MODEL));
    assert!(removed_line_numbers(&attribution.result.unresolved_patch).is_empty());

    let trace = build_trace(&case.committed, &attribution.result, &[]);
    assert!(trace.files.is_empty(), "deleted lines produce no ranges");
    assert_eq!(trace.metadata.sce.line_changes.ai.removed, 1);
    assert_eq!(trace.metadata.sce.line_changes.unknown.removed, 0);
}

#[tokio::test]
async fn a_proven_non_exclusive_removal_is_non_exclusive() {
    let case = removal_case(ABC, AC).await;
    let attribution = case
        .resolve(vec![event(1, &case.parent, &case.commit, false)])
        .await;

    assert!(removed_line_numbers(&attribution.result.mutation_ai_patch).is_empty());
    assert_eq!(
        removed_line_numbers(&attribution.result.resolved_non_ai_patch),
        vec![2]
    );
    let trace = build_trace(&case.committed, &attribution.result, &[]);
    assert_eq!(trace.metadata.sce.line_changes.non_exclusive.removed, 1);
}

#[tokio::test]
async fn history_without_the_parent_tree_leaves_removals_unresolved() {
    let case = removal_case(ABC, AC).await;
    let elsewhere = case.repo.tree(Some("a\nb\nc\nz\n"));
    let attribution = case
        .resolve(vec![event(1, &elsewhere, &case.commit, true)])
        .await;

    assert!(!attribution.removal_replay_anchored);
    assert_removal_unresolved(&attribution, &[2]);
}

#[tokio::test]
async fn a_history_gap_after_the_parent_anchor_leaves_removals_unresolved() {
    let case = removal_case(ABC, AC).await;
    let diverged = case.repo.tree(Some("a\nc\nx\n"));
    let attribution = case
        .resolve(vec![
            event(1, &case.parent, &case.commit, true),
            event(2, &diverged, &case.commit, true),
        ])
        .await;

    assert!(!attribution.removal_replay_anchored);
    assert_removal_unresolved(&attribution, &[2]);
}

#[tokio::test]
async fn a_parent_transition_outside_the_replay_window_leaves_removals_unresolved() {
    let case = removal_case(ABC, AC).await;
    let newest = MAX_MUTATION_ATTRIBUTION_EVENTS as u64 + 1;
    let mut rows = vec![event(1, &case.parent, &case.commit, true)];
    rows.extend((2..=newest).map(|revision| event(revision, &case.commit, &case.commit, true)));
    let attribution = case.resolve(rows).await;

    assert_eq!(attribution.loaded_rows, MAX_MUTATION_ATTRIBUTION_EVENTS);
    assert!(!attribution.removal_replay_anchored);
    assert_removal_unresolved(&attribution, &[2]);
}

#[tokio::test]
async fn an_unobserved_tail_removal_stays_unresolved() {
    let case = removal_case(ABC, AC).await;
    let attribution = case
        .resolve(vec![event(1, &case.parent, &case.parent, true)])
        .await;

    assert!(attribution.removal_replay_anchored);
    assert_removal_unresolved(&attribution, &[2]);
}

#[tokio::test]
async fn an_unobserved_tail_only_distrusts_the_paths_it_touches() {
    let repo = TreeRepo::new();
    let parent = repo.trees_with(&[(PLAN_PATH, Some(ABC)), ("other.txt", Some("o\n"))]);
    let observed = repo.trees_with(&[(PLAN_PATH, Some(AC))]);
    let commit = repo.trees_with(&[("other.txt", Some("o2\n"))]);
    let committed = committed_patch(&repo.snapshot().await, &parent, &commit).await;

    let attribution = resolve_from(
        &repo,
        vec![event(1, &parent, &observed, true)],
        &committed,
        &commit,
        Some(&parent),
        Some(1),
    )
    .await;

    assert_eq!(
        removed_in(&attribution.result.mutation_ai_patch, PLAN_PATH),
        vec![2]
    );
    assert_eq!(
        removed_in(&attribution.result.unresolved_patch, "other.txt"),
        vec![1]
    );
}

#[tokio::test]
async fn a_partially_staged_removal_is_not_attributed_from_the_larger_worktree_change() {
    let case = removal_case("A\nB\nC\n", "A\nC\n").await;
    let worktree = case.repo.tree(Some("A\n"));
    let attribution = case
        .resolve(vec![event(1, &case.parent, &worktree, true)])
        .await;

    assert!(attribution.removal_replay_anchored);
    assert_removal_unresolved(&attribution, &[2]);
}

#[tokio::test]
async fn a_restored_then_redeleted_line_keeps_no_stale_ai_removal() {
    let case = removal_case(ABC, AC).await;
    let restored = case.repo.tree(Some("a\nc\nb\n"));
    let attribution = case
        .resolve(vec![
            event(1, &case.parent, &case.commit, true),
            event(2, &case.commit, &restored, false),
            event(3, &restored, &case.commit, false),
        ])
        .await;

    assert!(attribution.removal_replay_anchored);
    assert_removal_unresolved(&attribution, &[2]);
}

#[tokio::test]
async fn an_exact_return_to_the_parent_tree_reanchors_at_the_latest_removal() {
    let case = removal_case(ABC, AC).await;
    let attribution = case
        .resolve(vec![
            event(1, &case.parent, &case.commit, true),
            event(2, &case.commit, &case.parent, false),
            event(3, &case.parent, &case.commit, false),
        ])
        .await;

    assert!(removed_line_numbers(&attribution.result.mutation_ai_patch).is_empty());
    assert_eq!(
        removed_line_numbers(&attribution.result.resolved_non_ai_patch),
        vec![2]
    );
}

#[tokio::test]
async fn a_duplicate_removal_maps_to_its_own_parent_line() {
    let case = removal_case("x\na\nx\n", "x\na\n").await;
    let attribution = case
        .resolve(vec![event(1, &case.parent, &case.commit, true)])
        .await;

    let removed: Vec<_> = attribution.result.mutation_ai_patch.files[0].hunks[0]
        .lines
        .iter()
        .map(|line| (line.line_number, line.content.as_str()))
        .collect();
    assert_eq!(removed, vec![(3, "x")]);
}

#[tokio::test]
async fn removed_and_added_lines_resolve_independently() {
    let case = removal_case(ABC, "a\nB\nc\nnew\n").await;
    let attribution = case
        .resolve(vec![event(1, &case.parent, &case.commit, true)])
        .await;

    assert_eq!(
        removed_line_numbers(&attribution.result.mutation_ai_patch),
        vec![2]
    );
    assert_eq!(
        added_line_numbers(&attribution.result.mutation_ai_patch),
        vec![2, 4]
    );

    let without_parent = resolve(
        &case.repo,
        vec![event(1, &case.parent, &case.commit, true)],
        &case.committed,
        &case.commit,
        Some(1),
    )
    .await;
    assert_eq!(
        added_line_numbers(&without_parent.result.mutation_ai_patch),
        vec![2, 4],
        "added lines resolve the same without a parent anchor"
    );
    assert_eq!(
        removed_line_numbers(&without_parent.result.unresolved_patch),
        vec![2]
    );
}

#[tokio::test]
async fn a_renamed_file_resolves_removals_under_its_old_path() {
    let repo = TreeRepo::new();
    let parent = repo.trees_with(&[("old.md", Some("one\ntwo\nthree\nfour\nfive\n"))]);
    let commit = repo.trees_with(&[("old.md", None), ("new.md", Some("one\ntwo\nfour\nfive\n"))]);
    let committed = committed_patch(&repo.snapshot().await, &parent, &commit).await;
    assert_eq!(committed.files[0].old_path, "old.md");
    assert_eq!(committed.files[0].new_path, "new.md");

    let attribution = resolve_from(
        &repo,
        vec![event(1, &parent, &commit, true)],
        &committed,
        &commit,
        Some(&parent),
        Some(1),
    )
    .await;
    assert_eq!(
        removed_in(&attribution.result.mutation_ai_patch, "old.md"),
        vec![3]
    );
}

#[tokio::test]
async fn committed_trees_follow_the_captured_commit_after_head_moves() {
    let repo = TreeRepo::new();
    let parent = repo.tree(Some(ABC));
    repo.commit("parent");
    let commit = repo.tree(Some(AC));
    let commit_oid = repo.commit("commit");
    repo.tree(Some("a\n"));
    repo.commit("later");
    assert_ne!(TreeId(repo.git(&["rev-parse", "HEAD^{tree}"])), commit);

    let snapshot = repo.snapshot().await;
    let trees = snapshot
        .committed_trees(&commit_oid)
        .await
        .expect("committed trees");
    assert_eq!(trees.commit_tree, commit);
    assert_eq!(trees.parent_tree.as_ref(), Some(&parent));

    let committed = committed_patch(&snapshot, &parent, &commit).await;
    let attribution = resolve_from(
        &repo,
        vec![event(1, &parent, &commit, true)],
        &committed,
        &trees.commit_tree,
        trees.parent_tree.as_ref(),
        Some(1),
    )
    .await;
    assert_eq!(
        removed_line_numbers(&attribution.result.mutation_ai_patch),
        vec![2]
    );
}

#[tokio::test]
async fn root_and_merge_commits_have_no_parent_tree() {
    let repo = TreeRepo::new();
    repo.tree(Some(ABC));
    let root = repo.commit("root");
    let snapshot = repo.snapshot().await;
    assert_eq!(
        snapshot
            .committed_trees(&root)
            .await
            .expect("root trees")
            .parent_tree,
        None
    );

    repo.git(&["checkout", "-q", "-b", "side"]);
    repo.tree(Some("a\nb\nc\nside\n"));
    repo.commit("side");
    repo.git(&["checkout", "-q", "main"]);
    repo.tree(Some("main\na\nb\nc\n"));
    repo.commit("main");
    repo.git(&[
        "-c",
        "user.name=Attribution Test",
        "-c",
        "user.email=attribution@example.invalid",
        "merge",
        "-q",
        "--no-ff",
        "--no-edit",
        "side",
    ]);
    let merge = repo.git(&["rev-parse", "HEAD"]);
    let trees = snapshot.committed_trees(&merge).await.expect("merge trees");
    assert_eq!(trees.parent_tree, None);
    assert_eq!(
        trees.commit_tree,
        TreeId(repo.git(&["rev-parse", "HEAD^{tree}"]))
    );
    assert!(snapshot.committed_trees("HEAD").await.is_err());
}

const CUTOFF_UNIX_SECONDS: i64 = 1_731_067_204;
const CUTOFF_MS: i64 = CUTOFF_UNIX_SECONDS * 1_000;

fn cutoff() -> CommitTimeCutoff {
    CommitTimeCutoff::from_commit_time_ms(CUTOFF_MS).expect("valid cutoff")
}

fn created_at(time_ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(time_ms)
        .expect("valid created_at")
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

fn fake_tree(revision: u64) -> String {
    format!("{revision:040x}")
}

async fn open_event_db() -> (tempfile::TempDir, RepositoryAgentTraceDb) {
    let dir = tempfile::tempdir().expect("db dir");
    let db = RepositoryAgentTraceDb::new_at(dir.path().join("agent-trace.db"))
        .await
        .expect("open agent trace db");
    (dir, db)
}

async fn insert_timed_event(db: &RepositoryAgentTraceDb, revision: u64, created_ms: i64) {
    db.execute(
        "INSERT INTO mutation_trace_events (worktree_id, revision, before_tree, after_tree, tainted, failure_kind, attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id, created_at) VALUES (?1, ?2, ?3, ?4, 0, 'healthy', 'ineligible_unscoped', NULL, 'flush', NULL, NULL, ?5)",
        (
            WORKTREE,
            revision.to_be_bytes().as_slice(),
            fake_tree(revision - 1),
            fake_tree(revision),
            created_at(created_ms),
        ),
    )
    .await
    .expect("insert timed mutation event");
}

fn revisions(rows: &[MutationEventPageRow]) -> Vec<u64> {
    rows.iter().map(|row| row.revision).collect()
}

#[tokio::test]
async fn commit_bounded_event_pages_exclude_the_whole_commit_second() {
    let (_dir, mut db) = open_event_db().await;
    let created = [
        CUTOFF_MS - 1,
        CUTOFF_MS,
        CUTOFF_MS + 1,
        CUTOFF_MS + 999,
        CUTOFF_MS + 1_000,
    ];
    for (index, created_ms) in created.into_iter().enumerate() {
        insert_timed_event(&db, index as u64 + 1, created_ms).await;
    }
    let store = MutationTraceStore::new(&mut db);
    let worktree = WorktreeId(WORKTREE.to_owned());

    let bounded = store
        .load_mutation_event_page_before_commit(&worktree, None, 32, cutoff())
        .await
        .expect("bounded page");
    assert_eq!(revisions(&bounded), vec![1]);

    let unbounded = store
        .load_mutation_event_page(&worktree, None, 32)
        .await
        .expect("unbounded page");
    assert_eq!(revisions(&unbounded), vec![5, 4, 3, 2, 1]);
}

#[tokio::test]
async fn late_events_never_consume_the_bounded_event_budget() {
    let (_dir, mut db) = open_event_db().await;
    let eligible = 150_u64;
    let late = 140_u64;
    for revision in 1..=eligible {
        insert_timed_event(&db, revision, CUTOFF_MS - 60_000 + revision.cast_signed()).await;
    }
    for revision in eligible + 1..=eligible + late {
        insert_timed_event(&db, revision, CUTOFF_MS + revision.cast_signed()).await;
    }
    let store = MutationTraceStore::new(&mut db);
    let worktree = WorktreeId(WORKTREE.to_owned());

    let first = store
        .load_mutation_event_page_before_commit(&worktree, None, 32, cutoff())
        .await
        .expect("first page");
    assert_eq!(revisions(&first), (119..=150).rev().collect::<Vec<_>>());
    let second = store
        .load_mutation_event_page_before_commit(&worktree, Some(119), 32, cutoff())
        .await
        .expect("second page");
    assert_eq!(revisions(&second), (87..=118).rev().collect::<Vec<_>>());

    let source = CommitBoundedEvents {
        store: &store,
        cutoff: cutoff(),
    };
    let mut state = ReplayState {
        loaded_pages: 0,
        loaded_rows: 0,
        inspected_events: 0,
        reconstructed_events: 0,
        gap_resets: 0,
        barrier: None,
    };
    let window = load_event_window(&source, &worktree, Some(eligible + late), &mut state).await;
    assert_eq!(revisions(&window), (23..=150).collect::<Vec<_>>());
    assert_eq!(window.len(), MAX_MUTATION_ATTRIBUTION_EVENTS);
    assert_eq!(state.loaded_pages, 4);
    assert_eq!(state.loaded_rows, MAX_MUTATION_ATTRIBUTION_EVENTS);
    assert_eq!(state.barrier, None);
}

#[tokio::test]
async fn a_short_eligible_history_behind_many_late_events_is_loaded_completely() {
    let (_dir, mut db) = open_event_db().await;
    for revision in 1..=40_u64 {
        insert_timed_event(&db, revision, CUTOFF_MS - 60_000 + revision.cast_signed()).await;
    }
    for revision in 41..=180_u64 {
        insert_timed_event(&db, revision, CUTOFF_MS + revision.cast_signed()).await;
    }
    let store = MutationTraceStore::new(&mut db);
    let source = CommitBoundedEvents {
        store: &store,
        cutoff: cutoff(),
    };
    let mut state = ReplayState {
        loaded_pages: 0,
        loaded_rows: 0,
        inspected_events: 0,
        reconstructed_events: 0,
        gap_resets: 0,
        barrier: None,
    };
    let window = load_event_window(
        &source,
        &WorktreeId(WORKTREE.to_owned()),
        Some(180),
        &mut state,
    )
    .await;
    assert_eq!(revisions(&window), (1..=40).collect::<Vec<_>>());
    assert_eq!(state.loaded_pages, 2);
    assert_eq!(state.loaded_rows, 40);
}
