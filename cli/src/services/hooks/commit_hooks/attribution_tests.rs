use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::services::agent_trace::{
    agent_trace_session_url, validate_agent_trace_value, AgentTrace, AgentTraceVcsType,
    ContributorType, DirectEvidenceSource, LineChangeAttribution, LineChangeCounts, RepositoryRoot,
};
use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_db::{
    AgentTraceInsert, DiffTraceInsert, DiffTracePathOrigin, ParsedDiffTracePatch,
    PostCommitPatchIntersectionInsert, RecentDiffTracePatches, PAYLOAD_TYPE_PATCH,
    PAYLOAD_TYPE_STRUCTURED,
};
use crate::services::mutation_trace::attribution::MutationAttributionResult;
use crate::services::mutation_trace::runtime::GitSnapshotService;
use crate::services::mutation_trace::runtime::{
    resolve_bounded_mutation_attribution, resolve_post_commit_mutation_attribution,
    MutationEventPageSource,
};
use crate::services::mutation_trace::store::{
    AttributionKind, MutationEventPageRow, MutationTraceStore, ScopeProvenance,
    MUTATION_ATTRIBUTION_PAGE_SIZE,
};
use crate::services::mutation_trace::types::{FailureKind, ScopeId, TreeId, WorktreeId};
use crate::services::patch::{parse_patch, ParsedPatch, TouchedLineKind};

use super::{
    capture_post_commit_patch_from_git, run_post_commit_agent_trace_flow_with,
    run_post_commit_intersection_flow_with, staged_diff_has_ai_overlap_with,
    PostCommitIntersectionFlowResult, StagedDiffAiOverlapResult,
};

const PLAN_PATH: &str = "context/plans/plan.md";
const BOILERPLATE: [&str; 3] = ["  - Completed: 2026-11-06", "  - Result:", "  - Verify:"];
const WORKTREE: &str = "main";
const REMOTE_URL: &str = "https://example.invalid/repo.git";

const BASE_TIME_S: i64 = 1_793_960_000;
const COMMIT_A_TIME_S: i64 = BASE_TIME_S + 1_000;
const COMMIT_B_TIME_S: i64 = BASE_TIME_S + 2_000;
const COMMIT_C_TIME_S: i64 = BASE_TIME_S + 3_000;

const SESSION_A: &str = "cc_session-a";
const SESSION_B: &str = "cc_session-b";
const SESSION_C: &str = "cc_session-c";
const SESSION_X: &str = "cc_session-x";
const MODEL_A: &str = "model-a";
const MODEL_B: &str = "model-b";
const MODEL_C: &str = "model-c";
const MODEL_X: &str = "model-x";
const SCOPE_A: &str = "scope-a";
const SCOPE_B: &str = "scope-b";
const SCOPE_C: &str = "scope-c";
const CLAUDE_TOOL: &str = "claude";
const OPENCODE_TOOL: &str = "opencode";

const fn ms(seconds: i64) -> i64 {
    seconds * 1_000
}

fn plan(done: [bool; 3]) -> String {
    let mut text = String::from("# Plan\n");
    for (index, section_done) in done.iter().enumerate() {
        writeln!(text, "- T0{}", index + 1).expect("write plan section");
        if *section_done {
            for line in BOILERPLATE {
                text.push_str(line);
                text.push('\n');
            }
        }
    }
    text
}

fn boilerplate_patch(old_start: u64, new_start: u64) -> String {
    boilerplate_patch_at(PLAN_PATH, old_start, new_start)
}

fn boilerplate_patch_at(path: &str, old_start: u64, new_start: u64) -> String {
    let mut text = format!(
        "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -{old_start},0 +{new_start},3 @@\n"
    );
    for line in BOILERPLATE {
        text.push('+');
        text.push_str(line);
        text.push('\n');
    }
    text
}

const EXACT_C_POSITIONS: (u64, u64) = (10, 11);
const COMMIT_B_POSITIONS: (u64, u64) = (6, 7);

struct HistoryRepo {
    dir: tempfile::TempDir,
    trees: [TreeId; 4],
}

impl HistoryRepo {
    fn new() -> Self {
        Self::with_plan_paths(&[PLAN_PATH])
    }

    /// Every path receives the same plan content in every commit, so the
    /// committed touched lines are identical at identical positions.
    fn with_plan_paths(paths: &[&str]) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        git(root, &["init", "-q", "-b", "main"], None);
        git(
            root,
            &["config", "user.email", "attribution@example.invalid"],
            None,
        );
        git(root, &["config", "user.name", "Attribution Test"], None);
        git(root, &["config", "commit.gpgsign", "false"], None);
        for path in paths {
            std::fs::create_dir_all(root.join(path).parent().expect("plan parent"))
                .expect("plan dir");
        }

        let mut trees = Vec::new();
        for (done, time_s) in [
            ([false, false, false], BASE_TIME_S),
            ([true, false, false], COMMIT_A_TIME_S),
            ([true, true, false], COMMIT_B_TIME_S),
            ([true, true, true], COMMIT_C_TIME_S),
        ] {
            for path in paths {
                std::fs::write(root.join(path), plan(done)).expect("write plan");
            }
            git(root, &["add", "-A"], None);
            git(root, &["commit", "-q", "-m", "plan"], Some(time_s));
            trees.push(TreeId(git(root, &["rev-parse", "HEAD^{tree}"], None)));
        }

        HistoryRepo {
            dir,
            trees: trees.try_into().expect("four trees"),
        }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn commit_tree(&self) -> &TreeId {
        &self.trees[3]
    }

    fn parent_tree(&self) -> &TreeId {
        &self.trees[2]
    }

    fn mutation_rows(&self, final_kind: AttributionKind) -> Vec<MutationEventPageRow> {
        let final_scope = (final_kind == AttributionKind::AiExclusive).then_some(SCOPE_C);
        [Some(SCOPE_A), Some(SCOPE_B), final_scope]
            .into_iter()
            .enumerate()
            .map(|(index, scope)| MutationEventPageRow {
                revision: index as u64 + 1,
                before_tree: self.trees[index].clone(),
                after_tree: self.trees[index + 1].clone(),
                failure_kind: FailureKind::Healthy,
                attribution_kind: if scope.is_some() {
                    AttributionKind::AiExclusive
                } else {
                    final_kind
                },
                attribution_scope_id: scope.map(|scope| ScopeId(scope.to_owned())),
            })
            .collect()
    }
}

fn git(root: &Path, args: &[&str], time_s: Option<i64>) -> String {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    if let Some(time_s) = time_s {
        let date = format!("@{time_s} +0000");
        command
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date);
    }
    let output = command.output().expect("git command should run");
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

struct VecPages {
    rows: Vec<MutationEventPageRow>,
}

fn scope_provenance(scope_id: &ScopeId) -> Option<ScopeProvenance> {
    let (session_id, model_id) = match scope_id.0.as_str() {
        SCOPE_A => (SESSION_A, MODEL_A),
        SCOPE_B => (SESSION_B, MODEL_B),
        SCOPE_C => (SESSION_C, MODEL_C),
        _ => return None,
    };
    Some(ScopeProvenance {
        scope_id: scope_id.clone(),
        session_id: session_id.to_owned(),
        model_id: Some(model_id.to_owned()),
    })
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
        Ok(scope_provenance(scope_id))
    }
}

struct DiffTraceRow {
    time_ms: i64,
    session_id: &'static str,
    model_id: &'static str,
    tool_name: &'static str,
    patch: String,
}

fn stale_rows() -> Vec<DiffTraceRow> {
    vec![
        DiffTraceRow {
            time_ms: ms(COMMIT_A_TIME_S) - 500,
            session_id: SESSION_A,
            model_id: MODEL_A,
            tool_name: CLAUDE_TOOL,
            patch: boilerplate_patch(EXACT_C_POSITIONS.0, EXACT_C_POSITIONS.1),
        },
        DiffTraceRow {
            time_ms: ms(COMMIT_B_TIME_S) - 500,
            session_id: SESSION_B,
            model_id: MODEL_B,
            tool_name: CLAUDE_TOOL,
            patch: boilerplate_patch(COMMIT_B_POSITIONS.0, COMMIT_B_POSITIONS.1),
        },
    ]
}

fn fresh_row(
    session_id: &'static str,
    model_id: &'static str,
    positions: (u64, u64),
) -> DiffTraceRow {
    DiffTraceRow {
        time_ms: ms(COMMIT_C_TIME_S) - 500,
        session_id,
        model_id,
        tool_name: CLAUDE_TOOL,
        patch: boilerplate_patch(positions.0, positions.1),
    }
}

fn fresh_row_at(
    path: &str,
    session_id: &'static str,
    model_id: &'static str,
    tool_name: &'static str,
) -> DiffTraceRow {
    DiffTraceRow {
        tool_name,
        patch: boilerplate_patch_at(path, EXACT_C_POSITIONS.0, EXACT_C_POSITIONS.1),
        ..fresh_row(session_id, model_id, EXACT_C_POSITIONS)
    }
}

fn parsed_row(id: i64, row: &DiffTraceRow) -> ParsedDiffTracePatch {
    let mut patch = parse_patch(&row.patch, Some(row.session_id)).expect("diff trace parses");
    for file in &mut patch.files {
        for hunk in &mut file.hunks {
            hunk.model_id = Some(row.model_id.to_owned());
        }
    }
    ParsedDiffTracePatch {
        id,
        time_ms: row.time_ms,
        session_id: row.session_id.to_owned(),
        patch,
        tool_name: Some(row.tool_name.to_owned()),
        tool_version: None,
        payload_type: String::from(PAYLOAD_TYPE_PATCH),
        path_origin: DiffTracePathOrigin::RepositoryRelative,
    }
}

fn rows_in_window(rows: &[DiffTraceRow], cutoff_ms: i64, end_ms: i64) -> RecentDiffTracePatches {
    RecentDiffTracePatches {
        patches: rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.time_ms >= cutoff_ms && row.time_ms <= end_ms)
            .map(|(index, row)| parsed_row(i64::try_from(index).expect("row index fits") + 1, row))
            .collect(),
        skipped: Vec::new(),
    }
}

struct PersistedIntersection {
    loaded: i64,
    patch: ParsedPatch,
}

async fn intersection_flow(
    repo: &HistoryRepo,
    rows: &[DiffTraceRow],
) -> (PostCommitIntersectionFlowResult, PersistedIntersection) {
    let persisted = RefCell::new(None);
    let flow = run_post_commit_intersection_flow_with(
        repo.root(),
        capture_post_commit_patch_from_git,
        || Ok(ms(COMMIT_C_TIME_S) + 500),
        async |cutoff_ms, end_ms| Ok(rows_in_window(rows, cutoff_ms, end_ms)),
        async |insert: PostCommitPatchIntersectionInsert<'_>| -> Result<()> {
            *persisted.borrow_mut() = Some(PersistedIntersection {
                loaded: insert.loaded_diff_trace_count,
                patch: serde_json::from_str(insert.intersection_patch).expect("intersection JSON"),
            });
            Ok(())
        },
    )
    .await
    .expect("intersection flow");
    (
        flow,
        persisted.into_inner().expect("intersection persisted"),
    )
}

async fn mutation_attribution(
    repo: &HistoryRepo,
    rows: Vec<MutationEventPageRow>,
    committed: &ParsedPatch,
) -> MutationAttributionResult {
    let ceiling = rows.iter().map(|row| row.revision).max();
    resolve_bounded_mutation_attribution(
        &VecPages { rows },
        &GitSnapshotService::new(repo.root())
            .await
            .expect("snapshot"),
        &WorktreeId(WORKTREE.to_owned()),
        committed,
        repo.commit_tree(),
        Some(repo.parent_tree()),
        ceiling,
    )
    .await
    .result
}

fn no_mutation() -> MutationAttributionResult {
    let empty = ParsedPatch { files: Vec::new() };
    MutationAttributionResult {
        mutation_ai_patch: empty.clone(),
        resolved_non_ai_patch: empty.clone(),
        unresolved_patch: empty,
    }
}

async fn agent_trace(
    flow: &PostCommitIntersectionFlowResult,
    mutation: &MutationAttributionResult,
) -> AgentTrace {
    run_post_commit_agent_trace_flow_with(
        flow,
        None,
        REMOTE_URL,
        mutation,
        None,
        |value| validate_agent_trace_value(value).map_err(|error| anyhow!(error.to_string())),
        async |_insert: AgentTraceInsert<'_>| -> Result<()> { Ok(()) },
    )
    .await
    .expect("agent trace flow")
}

fn related_sessions(trace: &AgentTrace) -> Vec<String> {
    trace.files[0].conversations[0]
        .related
        .iter()
        .flatten()
        .map(|related| related.url.clone())
        .collect()
}

fn assert_contains_none(json: &str, needles: &[&str]) {
    for needle in needles {
        assert!(!json.contains(needle), "unexpected {needle} in {json}");
    }
}

fn touched_line_count(patch: &ParsedPatch) -> usize {
    patch
        .files
        .iter()
        .flat_map(|file| file.hunks.iter())
        .map(|hunk| hunk.lines.len())
        .sum()
}

#[tokio::test]
async fn stale_sessions_from_already_committed_history_do_not_credit_a_later_commit() {
    let repo = HistoryRepo::new();
    let (flow, persisted) = intersection_flow(&repo, &stale_rows()).await;

    assert_eq!(persisted.loaded, 0);
    assert_eq!(touched_line_count(&persisted.patch), 0);

    let mutation = mutation_attribution(
        &repo,
        repo.mutation_rows(AttributionKind::AiExclusive),
        &flow.post_commit_data.parsed_patch,
    )
    .await;
    let trace = agent_trace(&flow, &mutation).await;
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
        Some(MODEL_C)
    );
    assert_eq!(
        related_sessions(&trace),
        vec![agent_trace_session_url(SESSION_C)]
    );
    assert_contains_none(
        &serde_json::to_string(&trace).expect("trace serializes"),
        &[SESSION_A, SESSION_B, MODEL_A, MODEL_B],
    );
}

#[tokio::test]
async fn causal_mutation_lineage_overrides_a_fresh_exact_direct_match_from_another_session() {
    let repo = HistoryRepo::new();
    let rows = vec![fresh_row(SESSION_X, MODEL_X, EXACT_C_POSITIONS)];
    let (flow, persisted) = intersection_flow(&repo, &rows).await;

    assert_eq!(touched_line_count(&persisted.patch), BOILERPLATE.len());

    let mutation = mutation_attribution(
        &repo,
        repo.mutation_rows(AttributionKind::AiExclusive),
        &flow.post_commit_data.parsed_patch,
    )
    .await;
    assert_eq!(
        touched_line_count(&mutation.mutation_ai_patch),
        BOILERPLATE.len()
    );

    let trace = agent_trace(&flow, &mutation).await;
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
        Some(MODEL_C)
    );
    assert_eq!(
        related_sessions(&trace),
        vec![agent_trace_session_url(SESSION_C)]
    );
    assert_eq!(trace.tool, None);
    assert_contains_none(
        &serde_json::to_string(&trace).expect("trace serializes"),
        &[SESSION_X, MODEL_X],
    );
}

#[tokio::test]
async fn non_exclusive_mutation_lineage_is_not_overridden_by_a_direct_match() {
    let repo = HistoryRepo::new();
    let rows = vec![fresh_row(SESSION_X, MODEL_X, EXACT_C_POSITIONS)];
    let (flow, _) = intersection_flow(&repo, &rows).await;

    let mutation = mutation_attribution(
        &repo,
        repo.mutation_rows(AttributionKind::IneligibleUnscoped),
        &flow.post_commit_data.parsed_patch,
    )
    .await;
    assert_eq!(
        touched_line_count(&mutation.resolved_non_ai_patch),
        BOILERPLATE.len()
    );

    let trace = agent_trace(&flow, &mutation).await;
    assert_eq!(
        trace.files[0].conversations[0]
            .contributor
            .as_ref()
            .expect("conversation contributor")
            .kind,
        ContributorType::Unknown
    );
    assert!(related_sessions(&trace).is_empty());
    let line_changes = trace.metadata.sce.line_changes;
    assert_eq!(line_changes.ai, LineChangeCounts::default());
    assert_eq!(
        line_changes.non_exclusive,
        LineChangeCounts {
            added: BOILERPLATE.len() as u64,
            removed: 0,
        }
    );
    assert_eq!(line_changes.unknown, LineChangeCounts::default());
}

#[tokio::test]
async fn a_historical_content_match_at_other_positions_does_not_attribute_without_lineage() {
    let repo = HistoryRepo::new();
    let rows = vec![fresh_row(SESSION_X, MODEL_X, COMMIT_B_POSITIONS)];
    let (flow, persisted) = intersection_flow(&repo, &rows).await;

    assert_eq!(persisted.loaded, 1);
    assert_eq!(touched_line_count(&persisted.patch), 0);

    let trace = agent_trace(&flow, &no_mutation()).await;
    assert_eq!(
        trace.files[0].conversations[0]
            .contributor
            .as_ref()
            .expect("conversation contributor")
            .kind,
        ContributorType::Unknown
    );
    assert_contains_none(
        &serde_json::to_string(&trace).expect("trace serializes"),
        &[SESSION_X, MODEL_X],
    );
}

#[tokio::test]
async fn fresh_exact_direct_evidence_still_attributes_without_mutation_history() {
    let repo = HistoryRepo::new();
    let mut rows = stale_rows();
    rows.push(fresh_row(SESSION_C, MODEL_C, EXACT_C_POSITIONS));
    let (flow, _) = intersection_flow(&repo, &rows).await;

    let trace = agent_trace(&flow, &no_mutation()).await;
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
        Some(MODEL_C)
    );
    assert_eq!(
        related_sessions(&trace),
        vec![agent_trace_session_url(SESSION_C)]
    );
    assert_eq!(
        trace.tool.as_ref().and_then(|tool| tool.name.as_deref()),
        Some("claude")
    );
    assert_contains_none(
        &serde_json::to_string(&trace).expect("trace serializes"),
        &[SESSION_A, SESSION_B],
    );
}

#[tokio::test]
async fn conflicting_fresh_direct_sessions_fail_closed_without_lineage() {
    let repo = HistoryRepo::new();
    let rows = vec![
        fresh_row(SESSION_C, MODEL_C, EXACT_C_POSITIONS),
        fresh_row(SESSION_X, MODEL_X, EXACT_C_POSITIONS),
    ];
    let (flow, persisted) = intersection_flow(&repo, &rows).await;

    assert_eq!(touched_line_count(&persisted.patch), 0);
    let trace = agent_trace(&flow, &no_mutation()).await;
    assert_eq!(
        trace.files[0].conversations[0]
            .contributor
            .as_ref()
            .expect("conversation contributor")
            .kind,
        ContributorType::Unknown
    );
    assert!(related_sessions(&trace).is_empty());
}

#[test]
fn one_direct_source_line_covers_only_its_own_duplicate_occurrence() {
    let committed = parse_patch(
        "diff --git a/plan.md b/plan.md\n--- a/plan.md\n+++ b/plan.md\n@@ -1,0 +2,1 @@\n+  - Verify:\n@@ -20,0 +22,1 @@\n+  - Verify:\n",
        None,
    )
    .expect("committed patch parses");
    let direct = parse_patch(
        "diff --git a/plan.md b/plan.md\n--- a/plan.md\n+++ b/plan.md\n@@ -1,0 +2,1 @@\n+  - Verify:\n",
        Some(SESSION_C),
    )
    .expect("direct patch parses");

    let trace = crate::services::agent_trace::build_agent_trace_from_evidence(
        crate::services::agent_trace::AgentTraceEvidence {
            repository_root: &RepositoryRoot::new(Path::new("/workspace/project")),
            direct_sources: &[DirectEvidenceSource::repository_relative(&direct)],
            mutation_ai_patch: &ParsedPatch { files: Vec::new() },
            mutation_non_ai_patch: &ParsedPatch { files: Vec::new() },
        },
        &committed,
        crate::services::agent_trace::AgentTraceMetadataInput {
            commit_timestamp: "2026-11-06T12:23:44Z",
            commit_revision: "0000000000000000000000000000000000000001",
            vcs_type: None,
        },
        &crate::services::agent_trace::UnverifiedCommittedFiles,
    )
    .expect("trace builds");

    let kinds: Vec<_> = trace.files[0]
        .conversations
        .iter()
        .map(|conversation| {
            conversation
                .contributor
                .as_ref()
                .expect("conversation contributor")
                .kind
        })
        .collect();
    assert_eq!(kinds, vec![ContributorType::Ai, ContributorType::Unknown]);
    assert_eq!(trace.metadata.sce.line_changes.ai.added, 1);
    assert_eq!(trace.metadata.sce.line_changes.unknown.added, 1);
}

async fn staged_overlap(
    rows: &[DiffTraceRow],
    head_time_s: Option<i64>,
) -> StagedDiffAiOverlapResult {
    staged_overlap_in(Path::new("."), rows, head_time_s).await
}

async fn staged_overlap_in(
    root: &Path,
    rows: &[DiffTraceRow],
    head_time_s: Option<i64>,
) -> StagedDiffAiOverlapResult {
    let staged = parse_patch(
        &boilerplate_patch(EXACT_C_POSITIONS.0, EXACT_C_POSITIONS.1),
        None,
    )
    .expect("staged patch parses");
    staged_diff_has_ai_overlap_with(
        root,
        |_| Ok(staged),
        |_| Ok(head_time_s.map(ms)),
        || Ok(ms(COMMIT_C_TIME_S)),
        async |cutoff_ms, end_ms| Ok(rows_in_window(rows, cutoff_ms, end_ms)),
    )
    .await
}

#[tokio::test]
async fn staged_coauthor_check_ignores_stale_and_historical_only_diff_traces() {
    assert_eq!(
        staged_overlap(&stale_rows(), Some(COMMIT_B_TIME_S)).await,
        StagedDiffAiOverlapResult::NoOverlap
    );
    assert_eq!(
        staged_overlap(
            &[fresh_row(SESSION_X, MODEL_X, COMMIT_B_POSITIONS)],
            Some(COMMIT_B_TIME_S)
        )
        .await,
        StagedDiffAiOverlapResult::NoOverlap
    );
}

#[tokio::test]
async fn staged_coauthor_check_keeps_fresh_exact_direct_evidence() {
    assert_eq!(
        staged_overlap(
            &[fresh_row(SESSION_C, MODEL_C, EXACT_C_POSITIONS)],
            Some(COMMIT_B_TIME_S)
        )
        .await,
        StagedDiffAiOverlapResult::Overlap
    );
    assert_eq!(
        staged_overlap(&stale_rows(), None).await,
        StagedDiffAiOverlapResult::Overlap
    );
}

async fn insert_mutation_rows(db: &RepositoryAgentTraceDb, rows: &[MutationEventPageRow]) {
    for row in rows {
        let scope_id = row
            .attribution_scope_id
            .as_ref()
            .expect("persisted rows are AI-exclusive");
        let provenance = scope_provenance(scope_id).expect("known scope");
        db.execute(
            "INSERT INTO mutation_trace_scopes (scope_id, worktree_id, actor_kind, status) VALUES (?1, ?2, 'claude_code', 'closed')",
            (scope_id.0.as_str(), WORKTREE),
        )
        .await
        .expect("insert scope");
        db.execute(
            "INSERT INTO mutation_trace_scope_provenance (scope_id, session_id, model_id) VALUES (?1, ?2, ?3)",
            (
                scope_id.0.as_str(),
                provenance.session_id.as_str(),
                provenance.model_id.as_deref(),
            ),
        )
        .await
        .expect("insert scope provenance");
        let revision = row.revision.to_be_bytes();
        db.execute(
            "INSERT INTO mutation_trace_events (worktree_id, revision, before_tree, after_tree, tainted, failure_kind, attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id) VALUES (?1, ?2, ?3, ?4, 0, 'healthy', 'ai_exclusive', ?5, 'close', ?5, ?6)",
            (
                WORKTREE,
                revision.as_slice(),
                row.before_tree.0.as_str(),
                row.after_tree.0.as_str(),
                scope_id.0.as_str(),
                format!("event-{}", row.revision),
            ),
        )
        .await
        .expect("insert mutation event");
    }
}

async fn open_db() -> (tempfile::TempDir, RepositoryAgentTraceDb) {
    let db_dir = tempfile::tempdir().expect("db dir");
    let db = RepositoryAgentTraceDb::new_at(db_dir.path().join("agent-trace.db"))
        .await
        .expect("open agent trace db");
    (db_dir, db)
}

async fn insert_rows(db: &RepositoryAgentTraceDb, rows: &[DiffTraceRow]) {
    for row in rows {
        db.insert_diff_trace(DiffTraceInsert {
            time_ms: row.time_ms,
            session_id: row.session_id,
            patch: &row.patch,
            model_id: Some(row.model_id),
            tool_name: row.tool_name,
            tool_version: None,
            payload_type: PAYLOAD_TYPE_PATCH,
        })
        .await
        .expect("insert diff trace");
    }
}

struct PersistedTrace {
    json: String,
    value: Value,
    trace: AgentTrace,
    committed: ParsedPatch,
    intersection_touched_lines: usize,
}

async fn persist_trace(
    repo: &HistoryRepo,
    db: &mut RepositoryAgentTraceDb,
    mutation_rows: &[MutationEventPageRow],
) -> PersistedTrace {
    insert_mutation_rows(db, mutation_rows).await;

    let flow = run_post_commit_intersection_flow_with(
        repo.root(),
        capture_post_commit_patch_from_git,
        || Ok(ms(COMMIT_C_TIME_S) + 500),
        async |cutoff_ms, end_ms| db.recent_diff_trace_patches(cutoff_ms, end_ms).await,
        async |insert| {
            db.insert_post_commit_patch_intersection(insert)
                .await
                .map(|_| ())
        },
    )
    .await
    .expect("intersection flow");

    let mutation = {
        let store = MutationTraceStore::new(db);
        resolve_bounded_mutation_attribution(
            &store,
            &GitSnapshotService::new(repo.root())
                .await
                .expect("snapshot"),
            &WorktreeId(WORKTREE.to_owned()),
            &flow.post_commit_data.parsed_patch,
            repo.commit_tree(),
            Some(repo.parent_tree()),
            mutation_rows.iter().map(|row| row.revision).max(),
        )
        .await
        .result
    };

    let trace = run_post_commit_agent_trace_flow_with(
        &flow,
        None,
        REMOTE_URL,
        &mutation,
        None,
        |value| validate_agent_trace_value(value).map_err(|error| anyhow!(error.to_string())),
        async |insert| db.insert_agent_trace(insert).await.map(|_| ()),
    )
    .await
    .expect("agent trace flow");
    let intersection_touched_lines = touched_line_count(&flow.intersection_patch);

    let traces: Vec<String> = db
        .query_map("SELECT trace_json FROM agent_traces", (), |row| {
            Ok(row.get(0)?)
        })
        .await
        .expect("read agent traces");
    assert_eq!(traces.len(), 1);
    let value: Value = serde_json::from_str(&traces[0]).expect("trace JSON");
    validate_agent_trace_value(&value).expect("persisted trace validates");
    PersistedTrace {
        json: traces[0].clone(),
        value,
        trace,
        committed: flow.post_commit_data.parsed_patch,
        intersection_touched_lines,
    }
}

#[tokio::test]
async fn persisted_agent_trace_keeps_causal_session_and_drops_stale_sessions() {
    let repo = HistoryRepo::new();
    let (_db_dir, mut db) = open_db().await;

    let mut rows = stale_rows();
    rows.push(fresh_row(SESSION_C, MODEL_C, EXACT_C_POSITIONS));
    insert_rows(&db, &rows).await;
    let persisted = persist_trace(
        &repo,
        &mut db,
        &repo.mutation_rows(AttributionKind::AiExclusive),
    )
    .await;

    let intersections: Vec<(i64, String)> = db
        .query_map(
            "SELECT loaded_diff_trace_count, intersection_patch FROM post_commit_patch_intersections",
            (),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .await
        .expect("read intersections");
    assert_eq!(intersections.len(), 1);
    assert_eq!(intersections[0].0, 1);
    assert_contains_none(&intersections[0].1, &[SESSION_A, SESSION_B]);
    assert_contains_none(&persisted.json, &[SESSION_A, SESSION_B, MODEL_A, MODEL_B]);

    let conversation = &persisted.value["files"][0]["conversations"][0];
    assert_eq!(conversation["contributor"]["type"], "ai");
    assert_eq!(conversation["contributor"]["model_id"], MODEL_C);
    let related: BTreeMap<String, String> = conversation["related"]
        .as_array()
        .expect("related sessions")
        .iter()
        .map(|related| {
            (
                related["url"].as_str().unwrap().to_owned(),
                related["type"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        related.into_keys().collect::<Vec<_>>(),
        vec![agent_trace_session_url(SESSION_C)]
    );
    assert_eq!(persisted.value["tool"]["name"], CLAUDE_TOOL);
}

#[tokio::test]
async fn persisted_agent_trace_omits_tool_metadata_from_rejected_direct_evidence() {
    let repo = HistoryRepo::new();
    let (_db_dir, mut db) = open_db().await;

    insert_rows(
        &db,
        &[fresh_row_at(PLAN_PATH, SESSION_X, MODEL_X, OPENCODE_TOOL)],
    )
    .await;
    let persisted = persist_trace(
        &repo,
        &mut db,
        &repo.mutation_rows(AttributionKind::AiExclusive),
    )
    .await;

    let conversation = &persisted.value["files"][0]["conversations"][0];
    assert_eq!(conversation["contributor"]["type"], "ai");
    assert_eq!(conversation["contributor"]["model_id"], MODEL_C);
    assert!(persisted.value.get("tool").is_none());
    assert_contains_none(&persisted.json, &[OPENCODE_TOOL, SESSION_X, MODEL_X]);
}

#[tokio::test]
async fn staged_coauthor_check_rejects_conflicting_direct_sessions() {
    assert_eq!(
        staged_overlap(
            &[
                fresh_row(SESSION_C, MODEL_C, EXACT_C_POSITIONS),
                fresh_row(SESSION_X, MODEL_X, EXACT_C_POSITIONS),
            ],
            Some(COMMIT_B_TIME_S)
        )
        .await,
        StagedDiffAiOverlapResult::NoOverlap
    );
    assert_eq!(
        staged_overlap(
            &[
                fresh_row(SESSION_C, MODEL_C, EXACT_C_POSITIONS),
                fresh_row(SESSION_C, MODEL_C, EXACT_C_POSITIONS),
            ],
            Some(COMMIT_B_TIME_S)
        )
        .await,
        StagedDiffAiOverlapResult::Overlap
    );
}

fn file_identity_cases(repo: &HistoryRepo, outside: &Path) -> Vec<(String, bool)> {
    vec![
        (PLAN_PATH.to_owned(), true),
        (format!("{}/{PLAN_PATH}", repo.root().display()), true),
        (format!("./context/../{PLAN_PATH}"), true),
        (String::from("plans/plan.md"), false),
        (format!("vendor/{PLAN_PATH}"), false),
        (format!("{}/{PLAN_PATH}", outside.display()), false),
        (format!("../{PLAN_PATH}"), false),
    ]
}

#[tokio::test]
async fn staged_coauthor_check_requires_repository_file_identity() {
    let repo = HistoryRepo::new();
    let outside = tempfile::tempdir().expect("outside dir");
    for (path, expected) in file_identity_cases(&repo, outside.path()) {
        let rows = [fresh_row_at(&path, SESSION_C, MODEL_C, CLAUDE_TOOL)];
        let expected = if expected {
            StagedDiffAiOverlapResult::Overlap
        } else {
            StagedDiffAiOverlapResult::NoOverlap
        };
        assert_eq!(
            staged_overlap_in(repo.root(), &rows, Some(COMMIT_B_TIME_S)).await,
            expected,
            "{path}"
        );
    }
}

#[tokio::test]
async fn post_commit_direct_evidence_requires_repository_file_identity() {
    let repo = HistoryRepo::new();
    let outside = tempfile::tempdir().expect("outside dir");
    for (path, expected) in file_identity_cases(&repo, outside.path()) {
        let rows = [fresh_row_at(&path, SESSION_C, MODEL_C, CLAUDE_TOOL)];
        let (flow, persisted) = intersection_flow(&repo, &rows).await;
        let trace = agent_trace(&flow, &no_mutation()).await;

        let (kind, tool, lines) = if expected {
            (ContributorType::Ai, Some(CLAUDE_TOOL), BOILERPLATE.len())
        } else {
            (ContributorType::Unknown, None, 0)
        };
        assert_eq!(
            trace.files[0].conversations[0]
                .contributor
                .as_ref()
                .expect("conversation contributor")
                .kind,
            kind,
            "{path}"
        );
        assert_eq!(
            trace.tool.as_ref().and_then(|tool| tool.name.as_deref()),
            tool,
            "{path}"
        );
        assert_eq!(touched_line_count(&persisted.patch), lines, "{path}");
    }
}

fn claude_write_payload(cwd: &str, file_path: &str) -> String {
    claude_write_payload_with_cwd(Some(Value::from(cwd)), file_path)
}

fn claude_write_payload_with_cwd(cwd: Option<Value>, file_path: &str) -> String {
    let mut payload = serde_json::json!({
        "hook_event_name": "PostToolUse",
        "session_id": SESSION_C,
        "tool_name": "Write",
        "tool_input": {
            "file_path": file_path,
            "content": plan([true, true, true]),
        },
        "tool_response": { "originalFile": null },
    });
    if let Some(cwd) = cwd {
        payload["cwd"] = cwd;
    }
    payload.to_string()
}

async fn insert_structured_row(db: &RepositoryAgentTraceDb, payload: &str) {
    db.insert_diff_trace(DiffTraceInsert {
        time_ms: ms(COMMIT_C_TIME_S) - 500,
        session_id: SESSION_C,
        patch: payload,
        model_id: Some(MODEL_C),
        tool_name: CLAUDE_TOOL,
        tool_version: Some("2.0.0"),
        payload_type: PAYLOAD_TYPE_STRUCTURED,
    })
    .await
    .expect("insert structured diff trace");
}

#[tokio::test]
async fn claude_structured_rows_resolve_paths_against_their_recorded_cwd() {
    let repo = HistoryRepo::new();
    let root = repo.root().display().to_string();
    let cases = [
        (
            format!("{root}/context"),
            format!("{root}/{PLAN_PATH}"),
            true,
        ),
        (
            format!("{root}/context"),
            String::from("plans/plan.md"),
            true,
        ),
        (root.clone(), format!("{root}/{PLAN_PATH}"), true),
        (
            format!("{root}/other"),
            String::from("plans/plan.md"),
            false,
        ),
        (root.clone(), String::from("plans/plan.md"), false),
    ];
    for (cwd, file_path, expected) in cases {
        let (_db_dir, mut db) = open_db().await;
        db.insert_diff_trace(DiffTraceInsert {
            time_ms: ms(COMMIT_C_TIME_S) - 500,
            session_id: SESSION_C,
            patch: &claude_write_payload(&cwd, &file_path),
            model_id: Some(MODEL_C),
            tool_name: CLAUDE_TOOL,
            tool_version: Some("2.0.0"),
            payload_type: PAYLOAD_TYPE_STRUCTURED,
        })
        .await
        .expect("insert structured diff trace");

        let persisted = persist_trace(&repo, &mut db, &[]).await;
        let conversation = &persisted.value["files"][0]["conversations"][0];
        if expected {
            assert_eq!(
                conversation["contributor"]["type"], "ai",
                "{cwd} {file_path}"
            );
            assert_eq!(conversation["contributor"]["model_id"], MODEL_C);
            assert_eq!(persisted.value["tool"]["name"], CLAUDE_TOOL);
            assert_eq!(persisted.value["tool"]["version"], "2.0.0");
        } else {
            assert_eq!(
                conversation["contributor"]["type"], "unknown",
                "{cwd} {file_path}"
            );
            assert!(persisted.value.get("tool").is_none());
        }
    }
}

/// Claude actually edited this file from `cwd = <root>/cli`; the repository
/// root holds a twin at [`PLAN_PATH`] with identical committed lines.
const TWIN_PLAN_PATH: &str = "cli/context/plans/plan.md";

struct StructuredOutcome {
    contributors: BTreeMap<String, String>,
    tool: Option<Value>,
    intersection_lines: usize,
    staged: StagedDiffAiOverlapResult,
}

async fn structured_outcome(repo: &HistoryRepo, payload: &str) -> StructuredOutcome {
    let (_db_dir, mut db) = open_db().await;
    insert_structured_row(&db, payload).await;

    let parsed = db
        .recent_diff_trace_patches(ms(COMMIT_C_TIME_S) - 1_000, ms(COMMIT_C_TIME_S))
        .await
        .expect("read structured rows");
    assert_eq!(parsed.loaded_count(), 1, "structured row is retained");
    assert_eq!(parsed.skipped_count(), 0);

    let staged = parse_patch(
        &boilerplate_patch(EXACT_C_POSITIONS.0, EXACT_C_POSITIONS.1),
        None,
    )
    .expect("staged patch parses");
    let staged = staged_diff_has_ai_overlap_with(
        repo.root(),
        |_| Ok(staged),
        |_| Ok(Some(ms(COMMIT_B_TIME_S))),
        || Ok(ms(COMMIT_C_TIME_S)),
        async |cutoff_ms, end_ms| db.recent_diff_trace_patches(cutoff_ms, end_ms).await,
    )
    .await;

    let persisted = persist_trace(repo, &mut db, &[]).await;
    let intersections: Vec<String> = db
        .query_map(
            "SELECT intersection_patch FROM post_commit_patch_intersections",
            (),
            |row| Ok(row.get(0)?),
        )
        .await
        .expect("read intersections");
    assert_eq!(intersections.len(), 1);
    let intersection: ParsedPatch =
        serde_json::from_str(&intersections[0]).expect("intersection JSON");

    StructuredOutcome {
        contributors: persisted.value["files"]
            .as_array()
            .expect("trace files")
            .iter()
            .map(|file| {
                (
                    file["path"].as_str().expect("file path").to_owned(),
                    file["conversations"][0]["contributor"]["type"]
                        .as_str()
                        .expect("contributor type")
                        .to_owned(),
                )
            })
            .collect(),
        tool: persisted.value.get("tool").cloned(),
        intersection_lines: touched_line_count(&intersection),
        staged,
    }
}

fn contributors(root: &str, twin: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (PLAN_PATH.to_owned(), root.to_owned()),
        (TWIN_PLAN_PATH.to_owned(), twin.to_owned()),
    ])
}

#[tokio::test]
async fn claude_relative_paths_without_an_absolute_cwd_never_attribute_the_repository_root_twin() {
    let repo = HistoryRepo::with_plan_paths(&[PLAN_PATH, TWIN_PLAN_PATH]);
    let cases = [
        ("missing", None),
        ("relative", Some(Value::from("cli"))),
        ("empty", Some(Value::from(""))),
        ("blank", Some(Value::from("   "))),
        ("non-string", Some(Value::from(42))),
        ("null", Some(Value::Null)),
    ];
    for (label, cwd) in cases {
        let outcome =
            structured_outcome(&repo, &claude_write_payload_with_cwd(cwd, PLAN_PATH)).await;
        assert_eq!(
            outcome.contributors,
            contributors("unknown", "unknown"),
            "{label} cwd"
        );
        assert_eq!(outcome.tool, None, "{label} cwd");
        assert_eq!(outcome.intersection_lines, 0, "{label} cwd");
        assert_eq!(
            outcome.staged,
            StagedDiffAiOverlapResult::NoOverlap,
            "{label} cwd"
        );
    }
}

#[tokio::test]
async fn claude_structured_paths_attribute_only_the_file_their_coordinate_system_names() {
    let repo = HistoryRepo::with_plan_paths(&[PLAN_PATH, TWIN_PLAN_PATH]);
    let root = repo.root().display().to_string();
    let cli = format!("{root}/cli");
    let outside = tempfile::tempdir().expect("outside dir");
    let cases = [
        (Some(cli.as_str()), PLAN_PATH.to_owned(), "twin"),
        (Some(cli.as_str()), format!("../cli/./{PLAN_PATH}"), "twin"),
        (Some(cli.as_str()), format!("../{PLAN_PATH}"), "root"),
        (
            Some(cli.as_str()),
            format!(
                "../../{}/{PLAN_PATH}",
                repo.root().file_name().unwrap().to_str().unwrap()
            ),
            "root",
        ),
        (Some(cli.as_str()), format!("../../../{PLAN_PATH}"), "none"),
        (None, format!("{root}/{TWIN_PLAN_PATH}"), "twin"),
        (None, format!("{root}/{PLAN_PATH}"), "root"),
        (
            None,
            format!("{}/{PLAN_PATH}", outside.path().display()),
            "none",
        ),
        (
            Some(cli.as_str()),
            format!("{}/{PLAN_PATH}", outside.path().display()),
            "none",
        ),
    ];
    for (cwd, file_path, expected) in cases {
        let outcome = structured_outcome(
            &repo,
            &claude_write_payload_with_cwd(cwd.map(Value::from), &file_path),
        )
        .await;
        let (root_kind, twin_kind) = match expected {
            "root" => ("ai", "unknown"),
            "twin" => ("unknown", "ai"),
            _ => ("unknown", "unknown"),
        };
        let label = format!("{cwd:?} {file_path}");
        assert_eq!(
            outcome.contributors,
            contributors(root_kind, twin_kind),
            "{label}"
        );
        assert_eq!(outcome.tool.is_some(), expected != "none", "{label}");
        assert_eq!(
            outcome.intersection_lines,
            if expected == "none" {
                0
            } else {
                BOILERPLATE.len()
            },
            "{label}"
        );
        let staged = if expected == "root" {
            StagedDiffAiOverlapResult::Overlap
        } else {
            StagedDiffAiOverlapResult::NoOverlap
        };
        assert_eq!(outcome.staged, staged, "{label}");
    }
}

#[tokio::test]
async fn mutation_lineage_still_attributes_when_claude_direct_evidence_lacks_a_cwd() {
    let repo = HistoryRepo::new();
    let (_db_dir, mut db) = open_db().await;
    insert_structured_row(&db, &claude_write_payload_with_cwd(None, PLAN_PATH)).await;
    let persisted = persist_trace(
        &repo,
        &mut db,
        &repo.mutation_rows(AttributionKind::AiExclusive),
    )
    .await;

    let conversation = &persisted.value["files"][0]["conversations"][0];
    assert_eq!(conversation["contributor"]["type"], "ai");
    assert_eq!(conversation["contributor"]["model_id"], MODEL_C);
    assert!(persisted.value.get("tool").is_none());
}

async fn persisted_line_changes(rows: &[DiffTraceRow], lineage: bool) -> LineChangeAttribution {
    let repo = HistoryRepo::new();
    let (_db_dir, mut db) = open_db().await;
    insert_rows(&db, rows).await;
    let mutation_rows = if lineage {
        repo.mutation_rows(AttributionKind::AiExclusive)
    } else {
        Vec::new()
    };
    let persisted = persist_trace(&repo, &mut db, &mutation_rows).await;

    assert_eq!(touched_line_count(&persisted.committed), BOILERPLATE.len());
    assert_eq!(
        persisted.intersection_touched_lines,
        rows.len() * BOILERPLATE.len()
    );
    let persisted_counts: LineChangeAttribution =
        serde_json::from_value(persisted.value["metadata"]["sce"]["line_changes"].clone())
            .expect("persisted line_changes deserialize");
    assert_eq!(persisted_counts, persisted.trace.metadata.sce.line_changes);
    persisted_counts
}

const fn added(count: usize) -> LineChangeCounts {
    LineChangeCounts {
        added: count as u64,
        removed: 0,
    }
}

#[tokio::test]
async fn persisted_agent_trace_counts_causal_lineage_lines_as_ai() {
    let counts =
        persisted_line_changes(&[fresh_row(SESSION_X, MODEL_X, EXACT_C_POSITIONS)], true).await;
    assert_eq!(
        counts,
        LineChangeAttribution {
            ai: added(BOILERPLATE.len()),
            ..LineChangeAttribution::default()
        }
    );
}

#[tokio::test]
async fn persisted_agent_trace_counts_direct_only_lines_as_ai() {
    let counts =
        persisted_line_changes(&[fresh_row(SESSION_C, MODEL_C, EXACT_C_POSITIONS)], false).await;
    assert_eq!(
        counts,
        LineChangeAttribution {
            ai: added(BOILERPLATE.len()),
            ..LineChangeAttribution::default()
        }
    );
}

#[tokio::test]
async fn persisted_agent_trace_counts_unresolved_lines_as_unknown() {
    let counts = persisted_line_changes(&[], false).await;
    assert_eq!(
        counts,
        LineChangeAttribution {
            unknown: added(BOILERPLATE.len()),
            ..LineChangeAttribution::default()
        }
    );
}

const RANGE_APP_PATH: &str = "src/app.ts";
const RANGE_NOTES_PATH: &str = "notes.md";
const RANGE_BASE_APP: &str = "keep 1\r\nold line\r\nkeep 2\r\n";
const RANGE_COMMITTED_APP: &str =
    "keep 1\r\nai one ✓\r\nai two\r\nunresolved\r\nkeep 2\r\ntail without newline";
const RANGE_COMMITTED_NOTES: &str = "x\n\n  y\n";
const RANGE_DELETED_PATCH: &str =
    "diff --git a/notes.md b/notes.md\n--- a/notes.md\n+++ b/notes.md\n@@ -0,0 +1 @@\n+fabricated\n";

fn committed_blob(root: &Path, commit: &str, path: &str) -> Vec<u8> {
    let output = Command::new("git")
        .args(["cat-file", "blob", &format!("{commit}:{path}")])
        .current_dir(root)
        .output()
        .expect("git cat-file runs");
    assert!(output.status.success());
    output.stdout
}

fn independent_line_slice(bytes: &[u8], start_line: u64, end_line: u64) -> Option<Vec<u8>> {
    let lines: Vec<&[u8]> = bytes.split_inclusive(|byte| *byte == b'\n').collect();
    let start = usize::try_from(start_line).ok()?.checked_sub(1)?;
    let end = usize::try_from(end_line).ok()?;
    (start < end && end <= lines.len()).then(|| lines[start..end].concat())
}

fn murmur3_tag(bytes: &[u8]) -> String {
    let hash = murmur3::murmur3_32(&mut std::io::Cursor::new(bytes), 0).expect("murmur3");
    format!("murmur3:{hash:08x}")
}

fn commit_range_fixture(root: &Path) -> String {
    git(root, &["init", "-q", "-b", "main"], None);
    git(
        root,
        &["config", "user.email", "ranges@example.invalid"],
        None,
    );
    git(root, &["config", "user.name", "Ranges Test"], None);
    git(root, &["config", "commit.gpgsign", "false"], None);
    git(root, &["config", "core.autocrlf", "false"], None);
    std::fs::create_dir_all(root.join("src")).expect("src dir");
    std::fs::write(root.join(RANGE_APP_PATH), RANGE_BASE_APP).expect("write app");
    std::fs::write(root.join("gone.txt"), "a\nb\n").expect("write gone");
    std::fs::write(root.join("changes.patch"), RANGE_DELETED_PATCH).expect("write patch");
    git(root, &["add", "-A"], None);
    git(root, &["commit", "-q", "-m", "base"], Some(BASE_TIME_S));

    std::fs::write(root.join(RANGE_APP_PATH), RANGE_COMMITTED_APP).expect("write app");
    std::fs::write(root.join(RANGE_NOTES_PATH), RANGE_COMMITTED_NOTES).expect("write notes");
    std::fs::remove_file(root.join("gone.txt")).expect("remove gone");
    std::fs::remove_file(root.join("changes.patch")).expect("remove patch");
    git(root, &["add", "-A"], None);
    git(
        root,
        &["commit", "-q", "-m", "ranges"],
        Some(COMMIT_A_TIME_S),
    );
    git(root, &["rev-parse", "HEAD"], None)
}

async fn persist_range_trace(root: &Path, db: &RepositoryAgentTraceDb) -> (AgentTrace, String) {
    insert_rows(
        db,
        &[DiffTraceRow {
            time_ms: ms(COMMIT_A_TIME_S) - 500,
            session_id: SESSION_C,
            model_id: MODEL_C,
            tool_name: CLAUDE_TOOL,
            patch: format!(
                "diff --git a/{RANGE_APP_PATH} b/{RANGE_APP_PATH}\n--- a/{RANGE_APP_PATH}\n+++ b/{RANGE_APP_PATH}\n@@ -1,0 +2,2 @@\n+ai one ✓\n+ai two\n"
            ),
        }],
    )
    .await;
    let flow = run_post_commit_intersection_flow_with(
        root,
        capture_post_commit_patch_from_git,
        || Ok(ms(COMMIT_A_TIME_S) + 500),
        async |cutoff_ms, end_ms| db.recent_diff_trace_patches(cutoff_ms, end_ms).await,
        async |insert| {
            db.insert_post_commit_patch_intersection(insert)
                .await
                .map(|_| ())
        },
    )
    .await
    .expect("intersection flow");
    let trace = run_post_commit_agent_trace_flow_with(
        &flow,
        Some(AgentTraceVcsType::Git),
        REMOTE_URL,
        &no_mutation(),
        None,
        |value| validate_agent_trace_value(value).map_err(|error| anyhow!(error.to_string())),
        async |insert| db.insert_agent_trace(insert).await.map(|_| ()),
    )
    .await
    .expect("agent trace flow");

    let persisted: Vec<String> = db
        .query_map("SELECT trace_json FROM agent_traces", (), |row| {
            Ok(row.get(0)?)
        })
        .await
        .expect("read agent traces");
    assert_eq!(persisted.len(), 1);
    (trace, persisted[0].clone())
}

type ObservedRange = (
    String,
    u64,
    u64,
    String,
    ContributorType,
    Option<String>,
    Vec<String>,
);

fn observed_ranges(root: &Path, commit: &str, trace: &AgentTrace) -> Vec<ObservedRange> {
    let mut observed = Vec::new();
    for file in &trace.files {
        let blob = committed_blob(root, commit, &file.path);
        for conversation in &file.conversations {
            for range in &conversation.ranges {
                let bytes = independent_line_slice(&blob, range.start_line, range.end_line)
                    .expect("range exists at the recorded revision");
                assert_eq!(range.content_hash, Some(murmur3_tag(&bytes)));
                let contributor = range
                    .contributor
                    .as_ref()
                    .or(conversation.contributor.as_ref())
                    .expect("effective contributor");
                observed.push((
                    file.path.clone(),
                    range.start_line,
                    range.end_line,
                    String::from_utf8(bytes).expect("utf-8 range"),
                    contributor.kind,
                    contributor.model_id.clone(),
                    conversation
                        .related
                        .iter()
                        .flatten()
                        .map(|related| related.url.clone())
                        .collect(),
                ));
            }
        }
    }
    observed
}

fn unknown_range(path: &str, start: u64, end: u64, text: &str) -> ObservedRange {
    (
        path.to_owned(),
        start,
        end,
        text.to_owned(),
        ContributorType::Unknown,
        None,
        Vec::new(),
    )
}

#[tokio::test]
async fn persisted_agent_trace_ranges_reference_exact_committed_blobs() {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    let commit = commit_range_fixture(root);
    let (_db_dir, db) = open_db().await;
    let (trace, persisted) = persist_range_trace(root, &db).await;

    let persisted_value: Value = serde_json::from_str(&persisted).expect("trace JSON");
    validate_agent_trace_value(&persisted_value).expect("persisted trace validates");
    let persisted_trace: AgentTrace =
        serde_json::from_value(persisted_value).expect("persisted trace deserializes");
    assert_eq!(persisted_trace, trace);
    assert_eq!(
        persisted_trace
            .vcs
            .as_ref()
            .map(|vcs| vcs.revision.as_str()),
        Some(commit.as_str())
    );

    assert_eq!(
        observed_ranges(root, &commit, &persisted_trace),
        vec![
            unknown_range(RANGE_NOTES_PATH, 1, 3, RANGE_COMMITTED_NOTES),
            (
                RANGE_APP_PATH.to_owned(),
                2,
                3,
                "ai one ✓\r\nai two\r\n".to_owned(),
                ContributorType::Ai,
                Some(MODEL_C.to_owned()),
                vec![agent_trace_session_url(SESSION_C)]
            ),
            unknown_range(RANGE_APP_PATH, 4, 4, "unresolved\r\n"),
            unknown_range(RANGE_APP_PATH, 6, 6, "tail without newline"),
        ]
    );
    assert_eq!(
        persisted_trace.files[1].conversations.len(),
        2,
        "unresolved app ranges share one conversation"
    );
    assert_eq!(
        persisted_trace.tool.and_then(|tool| tool.name).as_deref(),
        Some(CLAUDE_TOOL)
    );
    let deleted_patch_lines = RANGE_DELETED_PATCH.lines().count() as u64;
    assert_eq!(
        persisted_trace.metadata.sce.line_changes,
        LineChangeAttribution {
            ai: LineChangeCounts {
                added: 2,
                removed: 0
            },
            non_exclusive: LineChangeCounts::default(),
            unknown: LineChangeCounts {
                added: 5,
                removed: 1 + 2 + deleted_patch_lines
            },
        }
    );
}

const DELETION_A_PATH: &str = "src/a.rs";
const DELETION_B_PATH: &str = "src/b.rs";
const DELETION_C_PATH: &str = "src/c.rs";
const DELETION_A_PARENT: &str = "a1\na2\na3\na4\na5\na6\n";
const DELETION_B_PARENT: &str = "b1\nb2\nb3\n";
const DELETION_C_PARENT: &str = "c1\nc2\nc3\n";

struct DeletionRepo {
    dir: tempfile::TempDir,
    parent_tree: TreeId,
}

impl DeletionRepo {
    fn new() -> Self {
        let mut repo = DeletionRepo {
            dir: tempfile::tempdir().expect("temp dir"),
            parent_tree: TreeId(String::new()),
        };
        let root = repo.root();
        git(root, &["init", "-q", "-b", "main"], None);
        git(
            root,
            &["config", "user.email", "deletions@example.invalid"],
            None,
        );
        git(root, &["config", "user.name", "Deletion Test"], None);
        git(root, &["config", "commit.gpgsign", "false"], None);
        std::fs::create_dir_all(root.join("src")).expect("src dir");
        repo.parent_tree = repo.stage(&[
            (DELETION_A_PATH, DELETION_A_PARENT),
            (DELETION_B_PATH, DELETION_B_PARENT),
            (DELETION_C_PATH, DELETION_C_PARENT),
        ]);
        repo.commit(BASE_TIME_S);
        repo
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn stage(&self, files: &[(&str, &str)]) -> TreeId {
        for (path, content) in files {
            std::fs::write(self.root().join(path), content).expect("write file");
        }
        git(self.root(), &["add", "-A"], None);
        TreeId(git(self.root(), &["write-tree"], None))
    }

    fn commit(&self, time_s: i64) {
        git(self.root(), &["commit", "-q", "-m", "change"], Some(time_s));
    }
}

fn deletion_event(
    revision: u64,
    before: &TreeId,
    after: &TreeId,
    scope: Option<&str>,
) -> MutationEventPageRow {
    MutationEventPageRow {
        revision,
        before_tree: before.clone(),
        after_tree: after.clone(),
        failure_kind: FailureKind::Healthy,
        attribution_kind: if scope.is_some() {
            AttributionKind::AiExclusive
        } else {
            AttributionKind::IneligibleUnscoped
        },
        attribution_scope_id: scope.map(|scope| ScopeId(scope.to_owned())),
    }
}

async fn insert_mixed_mutation_rows(db: &RepositoryAgentTraceDb, rows: &[MutationEventPageRow]) {
    let (ai, non_exclusive): (Vec<_>, Vec<_>) = rows
        .iter()
        .cloned()
        .partition(|row| row.attribution_scope_id.is_some());
    insert_mutation_rows(db, &ai).await;
    for row in non_exclusive {
        let revision = row.revision.to_be_bytes();
        db.execute(
            "INSERT INTO mutation_trace_events (worktree_id, revision, before_tree, after_tree, tainted, failure_kind, attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id) VALUES (?1, ?2, ?3, ?4, 0, 'healthy', 'ineligible_unscoped', NULL, 'flush', NULL, NULL)",
            (
                WORKTREE,
                revision.as_slice(),
                row.before_tree.0.as_str(),
                row.after_tree.0.as_str(),
            ),
        )
        .await
        .expect("insert non-exclusive mutation event");
    }
}

struct PersistedDeletionTrace {
    value: Value,
    trace: AgentTrace,
    mutation: MutationAttributionResult,
    committed: ParsedPatch,
}

/// Runs the real post-commit capture, the exact-OID mutation resolver over
/// Turso-persisted events, and Agent Trace persistence. `HEAD` moves before
/// the resolver runs, so it must use the captured commit, not `HEAD`.
async fn persist_deletion_trace(
    repo: &DeletionRepo,
    rows: &[MutationEventPageRow],
) -> PersistedDeletionTrace {
    let (_db_dir, mut db) = open_db().await;
    insert_mixed_mutation_rows(&db, rows).await;

    let flow = run_post_commit_intersection_flow_with(
        repo.root(),
        capture_post_commit_patch_from_git,
        || Ok(ms(COMMIT_A_TIME_S) + 500),
        async |cutoff_ms, end_ms| db.recent_diff_trace_patches(cutoff_ms, end_ms).await,
        async |insert| {
            db.insert_post_commit_patch_intersection(insert)
                .await
                .map(|_| ())
        },
    )
    .await
    .expect("intersection flow");

    repo.stage(&[(DELETION_A_PATH, "moved\n")]);
    repo.commit(COMMIT_B_TIME_S);

    let mutation = resolve_post_commit_mutation_attribution(
        repo.root(),
        &mut db,
        &flow.post_commit_data.commit_oid,
        &flow.post_commit_data.parsed_patch,
    )
    .await;

    let trace = run_post_commit_agent_trace_flow_with(
        &flow,
        Some(AgentTraceVcsType::Git),
        REMOTE_URL,
        &mutation,
        None,
        |value| validate_agent_trace_value(value).map_err(|error| anyhow!(error.to_string())),
        async |insert| db.insert_agent_trace(insert).await.map(|_| ()),
    )
    .await
    .expect("agent trace flow");

    let traces: Vec<String> = db
        .query_map("SELECT trace_json FROM agent_traces", (), |row| {
            Ok(row.get(0)?)
        })
        .await
        .expect("read agent traces");
    assert_eq!(traces.len(), 1);
    let value: Value = serde_json::from_str(&traces[0]).expect("trace JSON");
    validate_agent_trace_value(&value).expect("persisted trace validates");
    let persisted: AgentTrace =
        serde_json::from_value(value.clone()).expect("persisted trace deserializes");
    assert_eq!(persisted, trace);

    PersistedDeletionTrace {
        value,
        trace,
        mutation,
        committed: flow.post_commit_data.parsed_patch,
    }
}

fn removed_lines_by_path(patch: &ParsedPatch) -> Vec<(String, u64, Option<String>)> {
    patch
        .files
        .iter()
        .flat_map(|file| {
            file.hunks.iter().flat_map(move |hunk| {
                hunk.lines
                    .iter()
                    .filter(|line| line.kind == TouchedLineKind::Removed)
                    .map(move |line| {
                        (
                            file.old_path.clone(),
                            line.line_number,
                            line.session_id.clone(),
                        )
                    })
            })
        })
        .collect()
}

fn committed_removed_count(patch: &ParsedPatch) -> u64 {
    removed_lines_by_path(patch).len() as u64
}

#[tokio::test]
async fn persisted_deletion_only_commit_counts_each_removal_in_its_proven_bucket() {
    let repo = DeletionRepo::new();
    let after_ai = repo.stage(&[(DELETION_A_PATH, "a1\na5\na6\n")]);
    let after_non_exclusive = repo.stage(&[(DELETION_B_PATH, "b1\nb3\n")]);
    repo.stage(&[(DELETION_C_PATH, "c1\nc3\n")]);
    repo.commit(COMMIT_A_TIME_S);

    let persisted = persist_deletion_trace(
        &repo,
        &[
            deletion_event(1, &repo.parent_tree, &after_ai, Some(SCOPE_A)),
            deletion_event(2, &after_ai, &after_non_exclusive, None),
        ],
    )
    .await;

    let ai_removals = removed_lines_by_path(&persisted.mutation.mutation_ai_patch);
    assert_eq!(
        ai_removals,
        [2, 3, 4]
            .map(|line| (DELETION_A_PATH.to_owned(), line, Some(SESSION_A.to_owned())))
            .to_vec()
    );
    assert!(persisted
        .mutation
        .mutation_ai_patch
        .files
        .iter()
        .flat_map(|file| file.hunks.iter())
        .all(|hunk| hunk.model_id.as_deref() == Some(MODEL_A)));
    assert_eq!(
        removed_lines_by_path(&persisted.mutation.resolved_non_ai_patch),
        vec![(DELETION_B_PATH.to_owned(), 2, None)]
    );
    assert_eq!(
        removed_lines_by_path(&persisted.mutation.unresolved_patch),
        vec![(DELETION_C_PATH.to_owned(), 2, None)]
    );

    let line_changes = persisted.trace.metadata.sce.line_changes;
    assert_eq!(
        line_changes,
        LineChangeAttribution {
            ai: LineChangeCounts {
                added: 0,
                removed: 3
            },
            non_exclusive: LineChangeCounts {
                added: 0,
                removed: 1
            },
            unknown: LineChangeCounts {
                added: 0,
                removed: 1
            },
        }
    );
    assert_eq!(
        line_changes.ai.removed + line_changes.non_exclusive.removed + line_changes.unknown.removed,
        committed_removed_count(&persisted.committed)
    );
    let persisted_counts: LineChangeAttribution =
        serde_json::from_value(persisted.value["metadata"]["sce"]["line_changes"].clone())
            .expect("persisted line_changes deserialize");
    assert_eq!(persisted_counts, line_changes);
    assert_eq!(persisted.value["files"], serde_json::json!([]));
    assert!(persisted.trace.tool.is_none());
}

#[tokio::test]
async fn persisted_mixed_commit_keeps_added_line_attribution_and_adds_removals() {
    let repo = DeletionRepo::new();
    let after_ai = repo.stage(&[(DELETION_A_PATH, "a1\nA2\na4\na5\na6\na7\n")]);
    repo.commit(COMMIT_A_TIME_S);
    let rows = [deletion_event(
        1,
        &repo.parent_tree,
        &after_ai,
        Some(SCOPE_A),
    )];

    let persisted = persist_deletion_trace(&repo, &rows).await;

    assert_eq!(
        persisted.trace.metadata.sce.line_changes,
        LineChangeAttribution {
            ai: LineChangeCounts {
                added: 2,
                removed: 2
            },
            ..LineChangeAttribution::default()
        }
    );
    let file = &persisted.trace.files[0];
    assert_eq!(file.path, DELETION_A_PATH);
    let ranges: Vec<(u64, u64)> = file.conversations[0]
        .ranges
        .iter()
        .map(|range| (range.start_line, range.end_line))
        .collect();
    assert_eq!(ranges, vec![(2, 2), (6, 6)], "only added lines get ranges");
    assert_eq!(
        file.conversations[0]
            .contributor
            .as_ref()
            .and_then(|contributor| contributor.model_id.as_deref()),
        Some(MODEL_A)
    );

    let without_parent = resolve_bounded_mutation_attribution(
        &VecPages {
            rows: rows.to_vec(),
        },
        &GitSnapshotService::new(repo.root())
            .await
            .expect("snapshot"),
        &WorktreeId(WORKTREE.to_owned()),
        &persisted.committed,
        &after_ai,
        None,
        Some(1),
    )
    .await
    .result;
    let added_only = |patch: &ParsedPatch| {
        let mut patch = patch.clone();
        for file in &mut patch.files {
            for hunk in &mut file.hunks {
                hunk.lines
                    .retain(|line| line.kind == TouchedLineKind::Added);
            }
            file.hunks.retain(|hunk| !hunk.lines.is_empty());
        }
        patch.files.retain(|file| !file.hunks.is_empty());
        patch
    };
    assert_eq!(
        added_only(&persisted.mutation.mutation_ai_patch),
        added_only(&without_parent.mutation_ai_patch),
        "removal anchoring leaves added-line attribution unchanged"
    );
}
