use anyhow::Result;

use std::collections::HashMap;
use std::path::Path;

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::checkout::{read_checkout_id, resolve_git_dir};
use crate::services::mutation_trace::attribution::{
    exclude_direct_coverage, resolve_mutation_attribution, MutationAttributionResult,
    MutationPatchEvidence,
};
use crate::services::mutation_trace::store::{
    AttributionKind, MutationEventPageRow, MutationTraceStore, MUTATION_ATTRIBUTION_PAGE_SIZE,
};
use crate::services::mutation_trace::types::{Attribution, ScopeId, TreeId, WorktreeId};
use crate::services::patch::{
    parse_patch, FileChangeKind, ParsedPatch, PatchFileChange, PatchHunk, TouchedLine,
    TouchedLineKind,
};

use super::git_snapshot::GitSnapshotService;

pub const MAX_MUTATION_ATTRIBUTION_EVENTS: usize = 128;

pub trait MutationEventPageSource {
    fn load_mutation_event_page(
        &self,
        worktree: &WorktreeId,
        revision_cursor: Option<u64>,
        requested_limit: usize,
    ) -> Result<Vec<MutationEventPageRow>>;
}

impl MutationEventPageSource for MutationTraceStore<'_> {
    fn load_mutation_event_page(
        &self,
        worktree: &WorktreeId,
        revision_cursor: Option<u64>,
        requested_limit: usize,
    ) -> Result<Vec<MutationEventPageRow>> {
        MutationTraceStore::load_mutation_event_page(
            self,
            worktree,
            revision_cursor,
            requested_limit,
        )
    }
}

pub trait TreeDiffSource {
    fn diff_trees(&self, before: &TreeId, after: &TreeId) -> Result<String>;
}

impl TreeDiffSource for GitSnapshotService {
    fn diff_trees(&self, before: &TreeId, after: &TreeId) -> Result<String> {
        GitSnapshotService::diff_trees(self, before, after)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationAttributionBarrier {
    PageQuery,
    EventReconstruction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedMutationAttribution {
    pub result: MutationAttributionResult,
    pub loaded_pages: usize,
    pub loaded_rows: usize,
    pub inspected_events: usize,
    pub reconstructed_events: usize,
    pub barrier: Option<MutationAttributionBarrier>,
}

pub fn resolve_bounded_mutation_attribution<P, D>(
    page_source: &P,
    diff_source: &D,
    worktree: &WorktreeId,
    direct_coverage: &ParsedPatch,
    unresolved_patch: &ParsedPatch,
) -> BoundedMutationAttribution
where
    P: MutationEventPageSource + ?Sized,
    D: TreeDiffSource + ?Sized,
{
    let mut loaded_pages = 0;
    let mut loaded_rows = 0;
    let mut inspected_events = 0;
    let mut reconstructed_events = 0;
    let mut barrier = None;

    let mut current_unresolved = exclude_direct_coverage(unresolved_patch, direct_coverage);
    let mut mutation_ai_parts: Vec<ParsedPatch> = Vec::new();
    let mut resolved_non_ai_parts: Vec<ParsedPatch> = Vec::new();

    let mut revision_cursor: Option<u64> = None;

    'traversal: while !patch_is_empty(&current_unresolved)
        && inspected_events < MAX_MUTATION_ATTRIBUTION_EVENTS
    {
        let requested_limit = requested_page_limit(
            MUTATION_ATTRIBUTION_PAGE_SIZE,
            MAX_MUTATION_ATTRIBUTION_EVENTS,
            inspected_events,
        );

        let Ok(page) =
            page_source.load_mutation_event_page(worktree, revision_cursor, requested_limit)
        else {
            barrier = Some(MutationAttributionBarrier::PageQuery);
            break 'traversal;
        };

        if page.is_empty() {
            break 'traversal;
        }

        loaded_pages += 1;
        loaded_rows += page.len();
        revision_cursor = Some(page[page.len() - 1].revision);
        let page_exhausts_history = page.len() < requested_limit;

        for row in page {
            if patch_is_empty(&current_unresolved)
                || inspected_events >= MAX_MUTATION_ATTRIBUTION_EVENTS
            {
                break 'traversal;
            }

            inspected_events += 1;

            let Ok(diff_text) = diff_source.diff_trees(&row.before_tree, &row.after_tree) else {
                barrier = Some(MutationAttributionBarrier::EventReconstruction);
                break 'traversal;
            };
            let Ok(event_patch) = parse_patch(&diff_text, None) else {
                barrier = Some(MutationAttributionBarrier::EventReconstruction);
                break 'traversal;
            };

            reconstructed_events += 1;

            let evidence = MutationPatchEvidence {
                patch: event_patch,
                tainted: row.tainted,
                failure_kind: row.failure_kind,
                attribution: attribution_from_row(row.attribution_kind, row.attribution_scope_id),
            };

            let step = resolve_mutation_attribution(
                direct_coverage,
                &current_unresolved,
                std::slice::from_ref(&evidence),
            );

            if !step.mutation_ai_patch.files.is_empty() {
                mutation_ai_parts.push(step.mutation_ai_patch);
            }
            if !step.resolved_non_ai_patch.files.is_empty() {
                resolved_non_ai_parts.push(step.resolved_non_ai_patch);
            }
            current_unresolved = step.unresolved_patch;
        }

        if page_exhausts_history {
            break 'traversal;
        }
    }

    BoundedMutationAttribution {
        result: MutationAttributionResult {
            mutation_ai_patch: combine_mutation_target_patches(&mutation_ai_parts),
            resolved_non_ai_patch: combine_mutation_target_patches(&resolved_non_ai_parts),
            unresolved_patch: current_unresolved,
        },
        loaded_pages,
        loaded_rows,
        inspected_events,
        reconstructed_events,
        barrier,
    }
}

fn requested_page_limit(page_size: usize, horizon: usize, inspected_events: usize) -> usize {
    page_size.min(horizon.saturating_sub(inspected_events))
}

/// Post-commit entry point: mutation-history AI coverage for the committed
/// patch's lines that direct evidence (`direct_coverage`, the post-commit-shaped
/// direct intersection) does not already cover, scoped to the invoking linked
/// worktree's existing checkout identity only.
///
/// Read-only and fail-open. An unresolvable git directory, an absent or
/// unreadable checkout identity, or an unavailable snapshot service each yield
/// an empty patch, so the caller falls back to direct-only Agent Trace
/// behavior. This path never creates checkout identity and never writes
/// mutation-cursor state.
pub(crate) fn resolve_post_commit_mutation_ai_patch(
    repository_root: &Path,
    db: &RepositoryAgentTraceDb,
    direct_coverage: &ParsedPatch,
    committed_patch: &ParsedPatch,
) -> ParsedPatch {
    let empty = || ParsedPatch { files: Vec::new() };

    let Ok(git_dir) = resolve_git_dir(repository_root) else {
        return empty();
    };
    let Ok(Some(checkout_id)) = read_checkout_id(&git_dir) else {
        return empty();
    };
    let Ok(snapshot) = GitSnapshotService::new(repository_root) else {
        return empty();
    };
    let store = MutationTraceStore::new(db);

    resolve_bounded_mutation_attribution(
        &store,
        &snapshot,
        &WorktreeId(checkout_id),
        direct_coverage,
        committed_patch,
    )
    .result
    .mutation_ai_patch
}

fn attribution_from_row(kind: AttributionKind, scope_id: Option<ScopeId>) -> Attribution {
    match (kind, scope_id) {
        (AttributionKind::AiExclusive, Some(scope_id)) => Attribution::AiExclusive(scope_id),
        (AttributionKind::AiContended, _) => Attribution::AiContended,
        (AttributionKind::AiExclusive | AttributionKind::IneligibleUnscoped, _) => {
            Attribution::IneligibleUnscoped
        }
    }
}

fn patch_is_empty(patch: &ParsedPatch) -> bool {
    patch
        .files
        .iter()
        .all(|file| file.hunks.iter().all(|hunk| hunk.lines.is_empty()))
}

fn logical_path(file: &PatchFileChange) -> &str {
    if file.new_path.is_empty() {
        &file.old_path
    } else {
        &file.new_path
    }
}

fn combine_mutation_target_patches(parts: &[ParsedPatch]) -> ParsedPatch {
    type LineKey = (TouchedLineKind, u64, String);
    type HunkMeta = (u64, u64, u64, u64, Option<String>);

    struct FileAcc {
        old_path: String,
        new_path: String,
        kind: FileChangeKind,
        hunk_order: Vec<HunkMeta>,
        hunks: HashMap<HunkMeta, HashMap<LineKey, TouchedLine>>,
    }

    let mut order: Vec<String> = Vec::new();
    let mut files: HashMap<String, FileAcc> = HashMap::new();

    for part in parts {
        for file in &part.files {
            let key = logical_path(file).to_owned();
            let acc = files.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                FileAcc {
                    old_path: file.old_path.clone(),
                    new_path: file.new_path.clone(),
                    kind: file.kind,
                    hunk_order: Vec::new(),
                    hunks: HashMap::new(),
                }
            });
            acc.old_path.clone_from(&file.old_path);
            acc.new_path.clone_from(&file.new_path);
            acc.kind = file.kind;

            for hunk in &file.hunks {
                let meta: HunkMeta = (
                    hunk.old_start,
                    hunk.old_count,
                    hunk.new_start,
                    hunk.new_count,
                    hunk.model_id.clone(),
                );
                if !acc.hunks.contains_key(&meta) {
                    acc.hunk_order.push(meta.clone());
                    acc.hunks.insert(meta.clone(), HashMap::new());
                }
                let lines = acc.hunks.get_mut(&meta).expect("hunk just inserted");
                for line in &hunk.lines {
                    lines.insert(
                        (line.kind, line.line_number, line.content.clone()),
                        line.clone(),
                    );
                }
            }
        }
    }

    let mut result_files = Vec::new();
    for path in order {
        let acc = files.remove(&path).expect("accumulator for ordered path");
        let mut hunks: Vec<PatchHunk> = acc
            .hunk_order
            .iter()
            .map(|meta| {
                let mut lines: Vec<TouchedLine> = acc.hunks[meta].values().cloned().collect();
                lines.sort_by(|a, b| {
                    a.line_number
                        .cmp(&b.line_number)
                        .then_with(|| line_kind_order(a.kind).cmp(&line_kind_order(b.kind)))
                        .then_with(|| a.content.cmp(&b.content))
                });
                PatchHunk {
                    old_start: meta.0,
                    old_count: meta.1,
                    new_start: meta.2,
                    new_count: meta.3,
                    model_id: meta.4.clone(),
                    lines,
                }
            })
            .collect();
        hunks.sort_by(|a, b| {
            (a.old_start, a.old_count, a.new_start, a.new_count)
                .cmp(&(b.old_start, b.old_count, b.new_start, b.new_count))
                .then_with(|| a.model_id.cmp(&b.model_id))
        });
        result_files.push(PatchFileChange {
            old_path: acc.old_path,
            new_path: acc.new_path,
            kind: acc.kind,
            hunks,
        });
    }

    ParsedPatch {
        files: result_files,
    }
}

fn line_kind_order(kind: TouchedLineKind) -> u8 {
    match kind {
        TouchedLineKind::Removed => 0,
        TouchedLineKind::Added => 1,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;
    use crate::services::mutation_trace::types::FailureKind;
    use crate::services::patch::{
        FileChangeKind, PatchFileChange, PatchHunk, TouchedLine, TouchedLineKind,
    };

    fn added_line(number: u64, content: &str) -> TouchedLine {
        TouchedLine {
            kind: TouchedLineKind::Added,
            line_number: number,
            content: content.to_owned(),
            session_id: None,
        }
    }

    fn target_patch(path: &str, lines: Vec<TouchedLine>) -> ParsedPatch {
        ParsedPatch {
            files: vec![PatchFileChange {
                old_path: path.to_owned(),
                new_path: path.to_owned(),
                kind: FileChangeKind::Modified,
                hunks: vec![PatchHunk {
                    old_start: 0,
                    old_count: 0,
                    new_start: 1,
                    new_count: lines.len() as u64,
                    model_id: None,
                    lines,
                }],
            }],
        }
    }

    fn added_line_diff(path: &str, number: u64, content: &str) -> String {
        format!(
            "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -{number},0 +{number},1 @@\n+{content}\n"
        )
    }

    fn page_row(
        revision: u64,
        before_tree: &str,
        after_tree: &str,
        attribution_kind: AttributionKind,
        attribution_scope_id: Option<&str>,
    ) -> MutationEventPageRow {
        MutationEventPageRow {
            revision,
            before_tree: TreeId(before_tree.to_owned()),
            after_tree: TreeId(after_tree.to_owned()),
            tainted: false,
            failure_kind: FailureKind::Healthy,
            attribution_kind,
            attribution_scope_id: attribution_scope_id.map(|id| ScopeId(id.to_owned())),
        }
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
                anyhow::bail!("injected page query failure on page {page_number}");
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

    struct FakeDiffSource {
        diffs: HashMap<(String, String), String>,
        fail_on_call: Option<usize>,
        calls: RefCell<usize>,
    }

    impl FakeDiffSource {
        fn new() -> Self {
            Self {
                diffs: HashMap::new(),
                fail_on_call: None,
                calls: RefCell::new(0),
            }
        }

        fn with(mut self, before: &str, after: &str, diff: String) -> Self {
            self.diffs
                .insert((before.to_owned(), after.to_owned()), diff);
            self
        }

        fn failing_on_call(mut self, call: usize) -> Self {
            self.fail_on_call = Some(call);
            self
        }

        fn call_count(&self) -> usize {
            *self.calls.borrow()
        }
    }

    impl TreeDiffSource for FakeDiffSource {
        fn diff_trees(&self, before: &TreeId, after: &TreeId) -> Result<String> {
            let call = {
                let mut calls = self.calls.borrow_mut();
                *calls += 1;
                *calls
            };
            if self.fail_on_call == Some(call) {
                anyhow::bail!("injected diff failure on call {call}");
            }
            self.diffs
                .get(&(before.0.clone(), after.0.clone()))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no canned diff for {before:?} -> {after:?}"))
        }
    }

    fn worktree() -> WorktreeId {
        WorktreeId("wt-current".to_owned())
    }

    fn empty_patch() -> ParsedPatch {
        ParsedPatch { files: vec![] }
    }

    fn ai_line_numbers(attribution: &BoundedMutationAttribution) -> Vec<u64> {
        attribution
            .result
            .mutation_ai_patch
            .files
            .iter()
            .flat_map(|file| file.hunks.iter())
            .flat_map(|hunk| hunk.lines.iter())
            .map(|line| line.line_number)
            .collect()
    }

    fn unresolved_line_numbers(attribution: &BoundedMutationAttribution) -> Vec<u64> {
        attribution
            .result
            .unresolved_patch
            .files
            .iter()
            .flat_map(|file| file.hunks.iter())
            .flat_map(|hunk| hunk.lines.iter())
            .map(|line| line.line_number)
            .collect()
    }

    #[test]
    fn requested_page_limit_is_capped_by_page_size_and_remaining_budget() {
        let limits: Vec<usize> = (0..5)
            .map(|page| requested_page_limit(32, 130, page * 32))
            .collect();
        assert_eq!(limits, vec![32, 32, 32, 32, 2]);

        let production: Vec<usize> = (0..4)
            .map(|page| requested_page_limit(32, 128, page * 32))
            .collect();
        assert_eq!(production, vec![32, 32, 32, 32]);
        assert_eq!(requested_page_limit(32, 128, 128), 0);
    }

    #[test]
    fn no_unresolved_lines_means_no_page_or_diff_work() {
        let page_source = FakePageSource::new(vec![page_row(
            5,
            "before-5",
            "after-5",
            AttributionKind::AiExclusive,
            Some("scope"),
        )]);
        let diff_source = FakeDiffSource::new();

        let attribution = resolve_bounded_mutation_attribution(
            &page_source,
            &diff_source,
            &worktree(),
            &empty_patch(),
            &empty_patch(),
        );

        assert!(page_source.requests().is_empty());
        assert_eq!(diff_source.call_count(), 0);
        assert_eq!(attribution.loaded_pages, 0);
        assert_eq!(attribution.inspected_events, 0);
        assert_eq!(attribution.barrier, None);
    }

    #[test]
    fn a_healthy_exclusive_event_contributes_mutation_ai_coverage() {
        let page_source = FakePageSource::new(vec![page_row(
            7,
            "before-7",
            "after-7",
            AttributionKind::AiExclusive,
            Some("scope-7"),
        )]);
        let diff_source = FakeDiffSource::new().with(
            "before-7",
            "after-7",
            added_line_diff("src/lib.rs", 2, "mutation"),
        );

        let attribution = resolve_bounded_mutation_attribution(
            &page_source,
            &diff_source,
            &worktree(),
            &empty_patch(),
            &target_patch("src/lib.rs", vec![added_line(2, "mutation")]),
        );

        assert_eq!(ai_line_numbers(&attribution), vec![2]);
        assert_eq!(unresolved_line_numbers(&attribution), Vec::<u64>::new());
        assert_eq!(attribution.loaded_pages, 1);
        assert_eq!(attribution.loaded_rows, 1);
        assert_eq!(attribution.inspected_events, 1);
        assert_eq!(attribution.reconstructed_events, 1);
        assert_eq!(attribution.barrier, None);
    }

    #[test]
    fn a_directly_covered_line_causes_zero_mutation_history_work() {
        let page_source = FakePageSource::new(vec![page_row(
            3,
            "before-3",
            "after-3",
            AttributionKind::AiExclusive,
            Some("scope-3"),
        )]);
        let diff_source = FakeDiffSource::new().with(
            "before-3",
            "after-3",
            added_line_diff("src/lib.rs", 1, "shared"),
        );

        let attribution = resolve_bounded_mutation_attribution(
            &page_source,
            &diff_source,
            &worktree(),
            &target_patch("src/lib.rs", vec![added_line(1, "shared")]),
            &target_patch("src/lib.rs", vec![added_line(1, "shared")]),
        );

        assert!(page_source.requests().is_empty());
        assert_eq!(attribution.loaded_pages, 0);
        assert_eq!(attribution.loaded_rows, 0);
        assert_eq!(attribution.inspected_events, 0);
        assert_eq!(attribution.reconstructed_events, 0);
        assert_eq!(diff_source.call_count(), 0);
        assert!(attribution.result.mutation_ai_patch.files.is_empty());
        assert!(attribution.result.resolved_non_ai_patch.files.is_empty());
        assert!(attribution.result.unresolved_patch.files.is_empty());
        assert_eq!(attribution.barrier, None);
    }

    #[test]
    fn only_the_lines_left_after_direct_coverage_reach_mutation_history() {
        let page_source = FakePageSource::new(vec![page_row(
            6,
            "before-6",
            "after-6",
            AttributionKind::AiExclusive,
            Some("scope-6"),
        )]);
        let diff_source = FakeDiffSource::new().with(
            "before-6",
            "after-6",
            added_line_diff("src/lib.rs", 2, "from-mutation"),
        );

        let attribution = resolve_bounded_mutation_attribution(
            &page_source,
            &diff_source,
            &worktree(),
            &target_patch("src/lib.rs", vec![added_line(1, "from-direct")]),
            &target_patch(
                "src/lib.rs",
                vec![added_line(1, "from-direct"), added_line(2, "from-mutation")],
            ),
        );

        assert_eq!(ai_line_numbers(&attribution), vec![2]);
        let ai_contents: Vec<String> = attribution
            .result
            .mutation_ai_patch
            .files
            .iter()
            .flat_map(|file| file.hunks.iter())
            .flat_map(|hunk| hunk.lines.iter())
            .map(|line| line.content.clone())
            .collect();
        assert_eq!(ai_contents, vec!["from-mutation".to_owned()]);
        assert_eq!(unresolved_line_numbers(&attribution), Vec::<u64>::new());
        assert_eq!(attribution.inspected_events, 1);
        assert_eq!(attribution.reconstructed_events, 1);
        assert_eq!(attribution.barrier, None);
    }

    #[test]
    fn newest_nonexclusive_match_blocks_the_older_exclusive_event() {
        let page_source = FakePageSource::new(vec![
            page_row(9, "before-9", "after-9", AttributionKind::AiContended, None),
            page_row(
                4,
                "before-4",
                "after-4",
                AttributionKind::AiExclusive,
                Some("scope-4"),
            ),
        ]);
        let diff_source = FakeDiffSource::new()
            .with(
                "before-9",
                "after-9",
                added_line_diff("src/lib.rs", 8, "same"),
            )
            .with(
                "before-4",
                "after-4",
                added_line_diff("src/lib.rs", 8, "same"),
            );

        let attribution = resolve_bounded_mutation_attribution(
            &page_source,
            &diff_source,
            &worktree(),
            &empty_patch(),
            &target_patch("src/lib.rs", vec![added_line(8, "same")]),
        );

        assert_eq!(ai_line_numbers(&attribution), Vec::<u64>::new());
        assert_eq!(
            attribution
                .result
                .resolved_non_ai_patch
                .files
                .iter()
                .flat_map(|file| file.hunks.iter())
                .flat_map(|hunk| hunk.lines.iter())
                .map(|line| line.line_number)
                .collect::<Vec<_>>(),
            vec![8]
        );
        assert_eq!(attribution.inspected_events, 1);
        assert_eq!(attribution.reconstructed_events, 1);
    }

    #[test]
    fn early_termination_leaves_loaded_rows_unreconstructed_and_requests_no_next_page() {
        let mut events = Vec::new();
        for revision in (1..=32).rev() {
            let scope = if revision == 29 {
                Some("scope-hit")
            } else {
                None
            };
            let kind = if revision == 29 {
                AttributionKind::AiExclusive
            } else {
                AttributionKind::IneligibleUnscoped
            };
            events.push(page_row(
                revision,
                &format!("before-{revision}"),
                &format!("after-{revision}"),
                kind,
                scope,
            ));
        }
        let mut diff_source = FakeDiffSource::new();
        for revision in (29..=32).rev() {
            let content = if revision == 29 { "target" } else { "noise" };
            let number = if revision == 29 { 5 } else { 999 };
            diff_source = diff_source.with(
                &format!("before-{revision}"),
                &format!("after-{revision}"),
                added_line_diff("src/lib.rs", number, content),
            );
        }
        let page_source = FakePageSource::new(events);

        let attribution = resolve_bounded_mutation_attribution(
            &page_source,
            &diff_source,
            &worktree(),
            &empty_patch(),
            &target_patch("src/lib.rs", vec![added_line(5, "target")]),
        );

        assert_eq!(ai_line_numbers(&attribution), vec![5]);
        assert_eq!(attribution.loaded_pages, 1);
        assert_eq!(attribution.loaded_rows, 32);
        assert_eq!(attribution.inspected_events, 4);
        assert_eq!(attribution.reconstructed_events, 4);
        assert_eq!(diff_source.call_count(), 4);
        assert_eq!(page_source.requests().len(), 1);
        assert_eq!(attribution.barrier, None);
    }

    #[test]
    fn traversal_is_current_worktree_only_and_revision_descending_with_an_exclusive_cursor() {
        let mut events = Vec::new();
        for revision in (1..=40).rev() {
            events.push(page_row(
                revision,
                &format!("before-{revision}"),
                &format!("after-{revision}"),
                AttributionKind::IneligibleUnscoped,
                None,
            ));
        }
        let mut diff_source = FakeDiffSource::new();
        for revision in 1..=40 {
            diff_source = diff_source.with(
                &format!("before-{revision}"),
                &format!("after-{revision}"),
                added_line_diff("src/lib.rs", 1, "noise"),
            );
        }
        let page_source = FakePageSource::new(events);

        let attribution = resolve_bounded_mutation_attribution(
            &page_source,
            &diff_source,
            &worktree(),
            &empty_patch(),
            &target_patch("src/lib.rs", vec![added_line(7, "unmatched")]),
        );

        let requests = page_source.requests();
        assert_eq!(
            requests,
            vec![
                PageRequest {
                    cursor: None,
                    limit: 32,
                },
                PageRequest {
                    cursor: Some(9),
                    limit: 32,
                },
            ]
        );
        assert_eq!(attribution.loaded_pages, 2);
        assert_eq!(attribution.loaded_rows, 40);
        assert_eq!(attribution.inspected_events, 40);
        assert_eq!(unresolved_line_numbers(&attribution), vec![7]);
        assert_eq!(attribution.barrier, None);
    }

    #[test]
    fn event_128_may_contribute_and_event_129_is_never_loaded_or_inspected() {
        let total = 200u64;
        let mut events = Vec::new();
        for revision in (1..=total).rev() {
            let matches = revision == total - 127;
            events.push(page_row(
                revision,
                &format!("before-{revision}"),
                &format!("after-{revision}"),
                if matches {
                    AttributionKind::AiExclusive
                } else {
                    AttributionKind::IneligibleUnscoped
                },
                matches.then_some("scope-128"),
            ));
        }
        let mut diff_source = FakeDiffSource::new();
        for revision in (1..=total).rev() {
            let matches = revision == total - 127;
            let (number, content) = if matches {
                (5, "target")
            } else {
                (999, "noise")
            };
            diff_source = diff_source.with(
                &format!("before-{revision}"),
                &format!("after-{revision}"),
                added_line_diff("src/lib.rs", number, content),
            );
        }
        let page_source = FakePageSource::new(events);

        let attribution = resolve_bounded_mutation_attribution(
            &page_source,
            &diff_source,
            &worktree(),
            &empty_patch(),
            &target_patch("src/lib.rs", vec![added_line(5, "target")]),
        );

        assert_eq!(ai_line_numbers(&attribution), vec![5]);
        assert_eq!(attribution.inspected_events, 128);
        assert_eq!(attribution.reconstructed_events, 128);
        assert_eq!(diff_source.call_count(), 128);
        assert_eq!(attribution.loaded_pages, 4);
        assert_eq!(attribution.loaded_rows, 128);
        for request in page_source.requests() {
            assert_eq!(request.limit, 32);
        }
        assert_eq!(attribution.barrier, None);
    }

    #[test]
    fn horizon_stops_traversal_when_no_line_ever_resolves() {
        let total = 200u64;
        let mut events = Vec::new();
        for revision in (1..=total).rev() {
            events.push(page_row(
                revision,
                &format!("before-{revision}"),
                &format!("after-{revision}"),
                AttributionKind::IneligibleUnscoped,
                None,
            ));
        }
        let mut diff_source = FakeDiffSource::new();
        for revision in (1..=total).rev() {
            diff_source = diff_source.with(
                &format!("before-{revision}"),
                &format!("after-{revision}"),
                added_line_diff("src/lib.rs", 1, "noise"),
            );
        }
        let page_source = FakePageSource::new(events);

        let attribution = resolve_bounded_mutation_attribution(
            &page_source,
            &diff_source,
            &worktree(),
            &empty_patch(),
            &target_patch("src/lib.rs", vec![added_line(7, "unmatched")]),
        );

        assert_eq!(attribution.inspected_events, 128);
        assert_eq!(attribution.reconstructed_events, 128);
        assert_eq!(attribution.loaded_pages, 4);
        assert_eq!(attribution.loaded_rows, 128);
        assert_eq!(page_source.requests().len(), 4);
        assert_eq!(unresolved_line_numbers(&attribution), vec![7]);
        assert_eq!(attribution.barrier, None);
    }

    #[test]
    fn a_page_query_failure_is_a_barrier_that_keeps_direct_and_newer_results() {
        let mut events = Vec::new();
        for revision in (1..=40).rev() {
            events.push(page_row(
                revision,
                &format!("before-{revision}"),
                &format!("after-{revision}"),
                AttributionKind::IneligibleUnscoped,
                None,
            ));
        }
        let mut diff_source = FakeDiffSource::new();
        for revision in (9..=40).rev() {
            diff_source = diff_source.with(
                &format!("before-{revision}"),
                &format!("after-{revision}"),
                added_line_diff("src/lib.rs", 1, "noise"),
            );
        }
        let page_source = FakePageSource::new(events).failing_on_page(2);

        let attribution = resolve_bounded_mutation_attribution(
            &page_source,
            &diff_source,
            &worktree(),
            &empty_patch(),
            &target_patch("src/lib.rs", vec![added_line(7, "unmatched")]),
        );

        assert_eq!(
            attribution.barrier,
            Some(MutationAttributionBarrier::PageQuery)
        );
        assert_eq!(attribution.loaded_pages, 1);
        assert_eq!(attribution.loaded_rows, 32);
        assert_eq!(attribution.inspected_events, 32);
        assert_eq!(attribution.reconstructed_events, 32);
        assert_eq!(page_source.requests().len(), 2);
        assert_eq!(unresolved_line_numbers(&attribution), vec![7]);
    }

    #[test]
    fn an_event_reconstruction_failure_is_a_barrier_that_inspects_nothing_older() {
        let page_source = FakePageSource::new(vec![
            page_row(
                9,
                "before-9",
                "after-9",
                AttributionKind::AiExclusive,
                Some("scope-9"),
            ),
            page_row(
                4,
                "before-4",
                "after-4",
                AttributionKind::AiExclusive,
                Some("scope-4"),
            ),
        ]);
        let diff_source = FakeDiffSource::new()
            .with(
                "before-9",
                "after-9",
                added_line_diff("src/lib.rs", 8, "eight"),
            )
            .failing_on_call(2);

        let attribution = resolve_bounded_mutation_attribution(
            &page_source,
            &diff_source,
            &worktree(),
            &empty_patch(),
            &target_patch(
                "src/lib.rs",
                vec![added_line(8, "eight"), added_line(9, "nine")],
            ),
        );

        assert_eq!(
            attribution.barrier,
            Some(MutationAttributionBarrier::EventReconstruction)
        );
        assert_eq!(ai_line_numbers(&attribution), vec![8]);
        assert_eq!(unresolved_line_numbers(&attribution), vec![9]);
        assert_eq!(attribution.loaded_rows, 2);
        assert_eq!(attribution.inspected_events, 2);
        assert_eq!(attribution.reconstructed_events, 1);
        assert_eq!(page_source.requests().len(), 1);
    }

    #[test]
    fn an_unparseable_reconstructed_diff_is_a_reconstruction_barrier() {
        let page_source = FakePageSource::new(vec![page_row(
            2,
            "before-2",
            "after-2",
            AttributionKind::AiExclusive,
            Some("scope-2"),
        )]);
        let diff_source = FakeDiffSource::new().with(
            "before-2",
            "after-2",
            "diff --git a/x b/x\n@@ this is not a hunk header @@\n".to_owned(),
        );

        let attribution = resolve_bounded_mutation_attribution(
            &page_source,
            &diff_source,
            &worktree(),
            &empty_patch(),
            &target_patch("src/lib.rs", vec![added_line(1, "x")]),
        );

        assert_eq!(
            attribution.barrier,
            Some(MutationAttributionBarrier::EventReconstruction)
        );
        assert_eq!(attribution.inspected_events, 1);
        assert_eq!(attribution.reconstructed_events, 0);
        assert_eq!(unresolved_line_numbers(&attribution), vec![1]);
    }

    #[test]
    fn irrelevant_events_still_count_toward_the_horizon() {
        let total = 130u64;
        let mut events = Vec::new();
        for revision in (1..=total).rev() {
            events.push(page_row(
                revision,
                &format!("before-{revision}"),
                &format!("after-{revision}"),
                AttributionKind::AiExclusive,
                Some("scope"),
            ));
        }
        let mut diff_source = FakeDiffSource::new();
        for revision in (1..=total).rev() {
            diff_source = diff_source.with(
                &format!("before-{revision}"),
                &format!("after-{revision}"),
                added_line_diff("other/file.rs", 1, "unrelated"),
            );
        }
        let page_source = FakePageSource::new(events);

        let attribution = resolve_bounded_mutation_attribution(
            &page_source,
            &diff_source,
            &worktree(),
            &empty_patch(),
            &target_patch("src/lib.rs", vec![added_line(3, "never-matched")]),
        );

        assert_eq!(attribution.inspected_events, 128);
        assert_eq!(attribution.reconstructed_events, 128);
        assert_eq!(diff_source.call_count(), 128);
        assert_eq!(unresolved_line_numbers(&attribution), vec![3]);
        assert_eq!(attribution.barrier, None);
    }

    fn removed_line(number: u64, content: &str) -> TouchedLine {
        TouchedLine {
            kind: TouchedLineKind::Removed,
            line_number: number,
            content: content.to_owned(),
            session_id: None,
        }
    }

    fn deleted_file_change(path: &str, lines: Vec<TouchedLine>) -> PatchFileChange {
        PatchFileChange {
            old_path: path.to_owned(),
            new_path: String::new(),
            kind: FileChangeKind::Deleted,
            hunks: vec![PatchHunk {
                old_start: 1,
                old_count: lines.len() as u64,
                new_start: 0,
                new_count: 0,
                model_id: None,
                lines,
            }],
        }
    }

    fn deleted_file_diff(path: &str, number: u64, content: &str) -> String {
        format!(
            "diff --git a/{path} b/{path}\ndeleted file mode 100644\n--- a/{path}\n+++ /dev/null\n@@ -{number},1 +0,0 @@\n-{content}\n"
        )
    }

    fn deleted_files_scenario(
        kind: AttributionKind,
    ) -> (FakePageSource, FakeDiffSource, ParsedPatch) {
        let scope = matches!(kind, AttributionKind::AiExclusive).then_some("scope-del");
        let page_source = FakePageSource::new(vec![
            page_row(2, "before-a", "after-a", kind, scope),
            page_row(1, "before-b", "after-b", kind, scope),
        ]);
        let diff_source = FakeDiffSource::new()
            .with("before-a", "after-a", deleted_file_diff("a.rs", 1, "alpha"))
            .with("before-b", "after-b", deleted_file_diff("b.rs", 1, "beta"));
        let target = ParsedPatch {
            files: vec![
                deleted_file_change("a.rs", vec![removed_line(1, "alpha")]),
                deleted_file_change("b.rs", vec![removed_line(1, "beta")]),
            ],
        };
        (page_source, diff_source, target)
    }

    #[test]
    fn two_deleted_files_stay_distinct_in_the_mutation_ai_result() {
        let (page_source, diff_source, target) =
            deleted_files_scenario(AttributionKind::AiExclusive);

        let attribution = resolve_bounded_mutation_attribution(
            &page_source,
            &diff_source,
            &worktree(),
            &empty_patch(),
            &target,
        );

        let files = &attribution.result.mutation_ai_patch.files;
        assert_eq!(
            files.len(),
            2,
            "deleted files must not collapse by empty new_path"
        );
        assert_eq!(files[0].old_path, "a.rs");
        assert_eq!(files[0].new_path, "");
        assert_eq!(files[1].old_path, "b.rs");
        assert_eq!(files[1].new_path, "");
        assert!(attribution.result.resolved_non_ai_patch.files.is_empty());
        assert!(attribution.result.unresolved_patch.files.is_empty());
        assert_eq!(attribution.barrier, None);
    }

    #[test]
    fn two_deleted_files_stay_distinct_in_the_resolved_non_ai_result() {
        let (page_source, diff_source, target) =
            deleted_files_scenario(AttributionKind::AiContended);

        let attribution = resolve_bounded_mutation_attribution(
            &page_source,
            &diff_source,
            &worktree(),
            &empty_patch(),
            &target,
        );

        let files = &attribution.result.resolved_non_ai_patch.files;
        assert_eq!(
            files.len(),
            2,
            "deleted files must not collapse by empty new_path"
        );
        assert_eq!(files[0].old_path, "a.rs");
        assert_eq!(files[0].new_path, "");
        assert_eq!(files[1].old_path, "b.rs");
        assert_eq!(files[1].new_path, "");
        assert!(attribution.result.mutation_ai_patch.files.is_empty());
        assert!(attribution.result.unresolved_patch.files.is_empty());
        assert_eq!(attribution.barrier, None);
    }

    #[test]
    fn real_store_and_git_snapshot_service_satisfy_the_consumer_seams() {
        use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
        use std::process::Command;

        let temp = tempfile::Builder::new()
            .prefix("sce-mutation-attribution-consumer-")
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

        let db_path = temp.path().join("agent-trace.db");
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("db opens");
        db.execute(
            "INSERT INTO mutation_trace_events
                (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
                 attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id)
             VALUES (?1, ?2, ?3, ?4, 0, 'healthy', 'ai_exclusive', 'scope-real', 'flush', NULL, NULL)",
            (
                "wt-real",
                crate::services::mutation_trace::store::encode_revision(1).as_slice(),
                before.0.as_str(),
                after.0.as_str(),
            ),
        )
        .expect("event insert");
        let store = MutationTraceStore::new(&db);

        let attribution = resolve_bounded_mutation_attribution(
            &store,
            &snapshot,
            &WorktreeId("wt-real".to_owned()),
            &empty_patch(),
            &target_patch("file.rs", vec![added_line(2, "two")]),
        );

        assert_eq!(ai_line_numbers(&attribution), vec![2]);
        assert_eq!(attribution.loaded_pages, 1);
        assert_eq!(attribution.inspected_events, 1);
        assert_eq!(attribution.reconstructed_events, 1);
        assert_eq!(attribution.barrier, None);
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
        git(&["commit", "--allow-empty", "--quiet", "-m", "init"]);
    }

    #[test]
    fn post_commit_entry_point_without_checkout_identity_yields_empty_patch_and_creates_none() {
        use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;

        let temp = tempfile::Builder::new()
            .prefix("sce-post-commit-attribution-no-identity-")
            .tempdir()
            .expect("temp dir");
        let repo_root = temp.path().join("repo");
        init_repo_with_commit(&repo_root);

        let git_dir = crate::services::checkout::resolve_git_dir(&repo_root).expect("git dir");
        let checkout_id_path = git_dir.join("sce").join("checkout-id");
        assert!(
            !checkout_id_path.exists(),
            "precondition: no checkout identity"
        );

        let db =
            RepositoryAgentTraceDb::new_at(temp.path().join("agent-trace.db")).expect("db opens");

        let result = resolve_post_commit_mutation_ai_patch(
            &repo_root,
            &db,
            &empty_patch(),
            &target_patch("file.rs", vec![added_line(2, "two")]),
        );

        assert!(
            result.files.is_empty(),
            "absent identity falls back to direct-only"
        );
        assert!(
            !checkout_id_path.exists(),
            "attribution lookup must not create checkout identity"
        );
    }

    #[test]
    fn post_commit_entry_point_resolves_current_worktree_mutation_ai_coverage() {
        use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;

        let temp = tempfile::Builder::new()
            .prefix("sce-post-commit-attribution-current-")
            .tempdir()
            .expect("temp dir");
        let repo_root = temp.path().join("repo");
        init_repo_with_commit(&repo_root);

        let git_dir = crate::services::checkout::resolve_git_dir(&repo_root).expect("git dir");
        let checkout_id =
            crate::services::checkout::get_or_create_checkout_id(&git_dir).expect("checkout id");

        let snapshot = GitSnapshotService::new(&repo_root).expect("snapshot service");
        std::fs::write(repo_root.join("file.rs"), b"one\n").expect("write");
        let before = snapshot.capture_tree().expect("capture before");
        std::fs::write(repo_root.join("file.rs"), b"one\ntwo\n").expect("write");
        let after = snapshot.capture_tree().expect("capture after");

        let db =
            RepositoryAgentTraceDb::new_at(temp.path().join("agent-trace.db")).expect("db opens");
        db.execute(
            "INSERT INTO mutation_trace_events
                (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
                 attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id)
             VALUES (?1, ?2, ?3, ?4, 0, 'healthy', 'ai_exclusive', 'scope-current', 'flush', NULL, NULL)",
            (
                checkout_id.as_str(),
                crate::services::mutation_trace::store::encode_revision(1).as_slice(),
                before.0.as_str(),
                after.0.as_str(),
            ),
        )
        .expect("event insert");
        // A foreign worktree's row for the same trees must never contribute.
        db.execute(
            "INSERT INTO mutation_trace_events
                (worktree_id, revision, before_tree, after_tree, tainted, failure_kind,
                 attribution_kind, attribution_scope_id, boundary_kind, boundary_scope_id, boundary_event_id)
             VALUES (?1, ?2, ?3, ?4, 0, 'healthy', 'ai_exclusive', 'scope-foreign', 'flush', NULL, NULL)",
            (
                "wt-foreign",
                crate::services::mutation_trace::store::encode_revision(2).as_slice(),
                before.0.as_str(),
                after.0.as_str(),
            ),
        )
        .expect("foreign event insert");

        let result = resolve_post_commit_mutation_ai_patch(
            &repo_root,
            &db,
            &empty_patch(),
            &target_patch("file.rs", vec![added_line(2, "two")]),
        );

        let ai_lines: Vec<u64> = result
            .files
            .iter()
            .flat_map(|file| file.hunks.iter())
            .flat_map(|hunk| hunk.lines.iter())
            .map(|line| line.line_number)
            .collect();
        assert_eq!(ai_lines, vec![2]);
    }
}
