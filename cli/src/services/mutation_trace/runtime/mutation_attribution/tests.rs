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
    resolve_bounded_mutation_attribution(
        &VecPages { rows },
        &repo.snapshot().await,
        &WorktreeId(WORKTREE.to_owned()),
        committed,
        commit_tree,
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
