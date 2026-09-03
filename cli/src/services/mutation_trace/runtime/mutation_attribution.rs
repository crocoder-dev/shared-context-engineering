use anyhow::Result;

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::checkout::{read_checkout_id, resolve_git_dir};
use crate::services::mutation_trace::attribution::{
    exclude_direct_coverage, logical_path, patch_for_locations, MutationAttributionResult,
    PatchLineLocation,
};
use crate::services::mutation_trace::lineage::{LineProvenance, MutationLineage, TransitionOrigin};
use crate::services::mutation_trace::store::{
    AttributionKind, MutationEventPageRow, MutationTraceStore, MUTATION_ATTRIBUTION_PAGE_SIZE,
};
use crate::services::mutation_trace::types::{FailureKind, TreeId, WorktreeId};
use crate::services::patch::{parse_patch, ParsedPatch, TouchedLineKind};

use super::git_snapshot::GitSnapshotService;
use super::worktree_lock::WorktreeLock;

pub const MAX_MUTATION_ATTRIBUTION_EVENTS: usize = 128;

const REVISION_CUT_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

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

pub trait TreeReadSource {
    fn diff_trees(&self, before: &TreeId, after: &TreeId) -> Result<String>;
    fn file_at_tree(&self, tree: &TreeId, path: &str) -> Result<Option<String>>;
}

impl TreeReadSource for GitSnapshotService {
    fn diff_trees(&self, before: &TreeId, after: &TreeId) -> Result<String> {
        GitSnapshotService::diff_trees(self, before, after)
    }

    fn file_at_tree(&self, tree: &TreeId, path: &str) -> Result<Option<String>> {
        GitSnapshotService::file_at_tree(self, tree, path)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationAttributionBarrier {
    PageQuery,
    EventReconstruction,
    Tail,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedMutationAttribution {
    pub result: MutationAttributionResult,
    pub loaded_pages: usize,
    pub loaded_rows: usize,
    pub inspected_events: usize,
    pub reconstructed_events: usize,
    pub gap_resets: usize,
    pub barrier: Option<MutationAttributionBarrier>,
}

impl BoundedMutationAttribution {
    fn empty() -> Self {
        BoundedMutationAttribution {
            result: MutationAttributionResult {
                mutation_ai_patch: empty_patch(),
                resolved_non_ai_patch: empty_patch(),
                unresolved_patch: empty_patch(),
            },
            loaded_pages: 0,
            loaded_rows: 0,
            inspected_events: 0,
            reconstructed_events: 0,
            gap_resets: 0,
            barrier: None,
        }
    }
}

fn empty_patch() -> ParsedPatch {
    ParsedPatch { files: Vec::new() }
}

#[allow(clippy::too_many_arguments)]
pub fn resolve_bounded_mutation_attribution<P, R>(
    page_source: &P,
    tree_source: &R,
    worktree: &WorktreeId,
    direct_coverage: &ParsedPatch,
    committed_patch: &ParsedPatch,
    commit_tree: &TreeId,
    revision_ceiling: Option<u64>,
) -> BoundedMutationAttribution
where
    P: MutationEventPageSource + ?Sized,
    R: TreeReadSource + ?Sized,
{
    let target = exclude_direct_coverage(committed_patch, direct_coverage);
    let target_paths = target_logical_paths(&target);
    if target_paths.is_empty() {
        return BoundedMutationAttribution::empty();
    }

    let mut state = ReplayState {
        loaded_pages: 0,
        loaded_rows: 0,
        inspected_events: 0,
        reconstructed_events: 0,
        gap_resets: 0,
        barrier: None,
    };

    let events = load_event_window(page_source, worktree, revision_ceiling, &mut state);
    let lineage = if events.is_empty() {
        None
    } else {
        Some(replay(
            &events,
            &target_paths,
            commit_tree,
            tree_source,
            &mut state,
        ))
    };
    finish(&target, lineage.as_ref(), &state)
}

struct ReplayState {
    loaded_pages: usize,
    loaded_rows: usize,
    inspected_events: usize,
    reconstructed_events: usize,
    gap_resets: usize,
    barrier: Option<MutationAttributionBarrier>,
}

fn finish(
    target: &ParsedPatch,
    lineage: Option<&MutationLineage>,
    state: &ReplayState,
) -> BoundedMutationAttribution {
    let result = match lineage {
        Some(lineage) => project(target, lineage),
        None => MutationAttributionResult {
            mutation_ai_patch: empty_patch(),
            resolved_non_ai_patch: empty_patch(),
            unresolved_patch: target.clone(),
        },
    };

    BoundedMutationAttribution {
        result,
        loaded_pages: state.loaded_pages,
        loaded_rows: state.loaded_rows,
        inspected_events: state.inspected_events,
        reconstructed_events: state.reconstructed_events,
        gap_resets: state.gap_resets,
        barrier: state.barrier,
    }
}

fn target_logical_paths(target: &ParsedPatch) -> BTreeSet<String> {
    target
        .files
        .iter()
        .filter(|file| file.hunks.iter().any(|hunk| !hunk.lines.is_empty()))
        .map(|file| logical_path(file).to_owned())
        .collect()
}

fn load_event_window<P>(
    page_source: &P,
    worktree: &WorktreeId,
    revision_ceiling: Option<u64>,
    state: &mut ReplayState,
) -> Vec<MutationEventPageRow>
where
    P: MutationEventPageSource + ?Sized,
{
    let mut rows: Vec<MutationEventPageRow> = Vec::new();
    let mut cursor: Option<u64> = revision_ceiling.and_then(|ceiling| ceiling.checked_add(1));

    loop {
        if rows.len() >= MAX_MUTATION_ATTRIBUTION_EVENTS {
            break;
        }
        let want = MUTATION_ATTRIBUTION_PAGE_SIZE.min(MAX_MUTATION_ATTRIBUTION_EVENTS - rows.len());

        let Ok(page) = page_source.load_mutation_event_page(worktree, cursor, want) else {
            state.barrier = Some(MutationAttributionBarrier::PageQuery);
            break;
        };
        if page.is_empty() {
            break;
        }

        state.loaded_pages += 1;
        state.loaded_rows += page.len();
        let short = page.len() < want;
        cursor = Some(page[page.len() - 1].revision);
        rows.extend(page);

        if short {
            break;
        }
    }

    rows.reverse();
    rows
}

fn replay<R>(
    events: &[MutationEventPageRow],
    target_paths: &BTreeSet<String>,
    commit_tree: &TreeId,
    tree_source: &R,
    state: &mut ReplayState,
) -> MutationLineage
where
    R: TreeReadSource + ?Sized,
{
    let mut lineage = MutationLineage::from_baseline(&load_baseline(
        tree_source,
        &events[0].before_tree,
        target_paths,
    ));
    let mut prev_after = events[0].before_tree.clone();

    for row in events {
        state.inspected_events += 1;

        if row.before_tree != prev_after {
            lineage.reset_all(&load_baseline(tree_source, &row.before_tree, target_paths));
            state.gap_resets += 1;
        }

        let reconstructed = tree_source
            .diff_trees(&row.before_tree, &row.after_tree)
            .ok()
            .and_then(|text| parse_patch(&text, None).ok());
        if reconstructed.is_some() {
            state.reconstructed_events += 1;
        }

        let origin = transition_origin(row);
        if apply_or_reset(
            &mut lineage,
            reconstructed.as_ref(),
            &origin,
            &row.after_tree,
            target_paths,
            tree_source,
        ) {
            state.barrier = Some(MutationAttributionBarrier::EventReconstruction);
            state.gap_resets += 1;
        }

        prev_after = row.after_tree.clone();
    }

    if prev_after != *commit_tree {
        let tail = tree_source
            .diff_trees(&prev_after, commit_tree)
            .ok()
            .and_then(|text| parse_patch(&text, None).ok());
        if apply_or_reset(
            &mut lineage,
            tail.as_ref(),
            &TransitionOrigin::Unobserved,
            commit_tree,
            target_paths,
            tree_source,
        ) {
            state.barrier = Some(MutationAttributionBarrier::Tail);
            state.gap_resets += 1;
        }
    }

    lineage
}

fn apply_or_reset<R>(
    lineage: &mut MutationLineage,
    patch: Option<&ParsedPatch>,
    origin: &TransitionOrigin,
    fallback_tree: &TreeId,
    target_paths: &BTreeSet<String>,
    tree_source: &R,
) -> bool
where
    R: TreeReadSource + ?Sized,
{
    let applied = patch.is_some_and(|patch| lineage.apply(patch, origin).is_ok());
    if !applied {
        lineage.reset_all(&load_baseline(tree_source, fallback_tree, target_paths));
    }
    !applied
}

fn load_baseline<R>(
    tree_source: &R,
    tree: &TreeId,
    target_paths: &BTreeSet<String>,
) -> std::collections::BTreeMap<String, Option<String>>
where
    R: TreeReadSource + ?Sized,
{
    target_paths
        .iter()
        .map(|path| {
            let content = tree_source.file_at_tree(tree, path).unwrap_or(None);
            (path.clone(), content)
        })
        .collect()
}

fn transition_origin(row: &MutationEventPageRow) -> TransitionOrigin {
    let healthy = !row.tainted && row.failure_kind == FailureKind::Healthy;
    match (&row.attribution_kind, &row.attribution_scope_id) {
        (AttributionKind::AiExclusive, Some(scope_id)) if healthy => {
            TransitionOrigin::MutationAi(scope_id.clone())
        }
        _ => TransitionOrigin::MutationNonAi,
    }
}

fn project(target: &ParsedPatch, lineage: &MutationLineage) -> MutationAttributionResult {
    let mut ai = BTreeSet::new();
    let mut non_ai = BTreeSet::new();
    let mut unresolved = BTreeSet::new();

    for (file_index, file) in target.files.iter().enumerate() {
        let path = logical_path(file);
        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            for (line_index, line) in hunk.lines.iter().enumerate() {
                let location = PatchLineLocation {
                    file_index,
                    hunk_index,
                    line_index,
                };
                if line.kind != TouchedLineKind::Added {
                    unresolved.insert(location);
                    continue;
                }
                match lineage.provenance_at(path, line.line_number, &line.content) {
                    LineProvenance::MutationAi { .. } => {
                        ai.insert(location);
                    }
                    LineProvenance::MutationNonAi => {
                        non_ai.insert(location);
                    }
                    LineProvenance::Unknown => {
                        unresolved.insert(location);
                    }
                }
            }
        }
    }

    MutationAttributionResult {
        mutation_ai_patch: patch_for_locations(target, &ai),
        resolved_non_ai_patch: patch_for_locations(target, &non_ai),
        unresolved_patch: patch_for_locations(target, &unresolved),
    }
}

pub(crate) fn resolve_post_commit_mutation_ai_patch(
    repository_root: &Path,
    db: &RepositoryAgentTraceDb,
    direct_coverage: &ParsedPatch,
    committed_patch: &ParsedPatch,
) -> ParsedPatch {
    let Ok(git_dir) = resolve_git_dir(repository_root) else {
        return empty_patch();
    };
    let Ok(Some(checkout_id)) = read_checkout_id(&git_dir) else {
        return empty_patch();
    };
    let Ok(snapshot) = GitSnapshotService::new(repository_root) else {
        return empty_patch();
    };
    let Ok(commit_tree) = snapshot.head_tree() else {
        return empty_patch();
    };
    let store = MutationTraceStore::new(db);
    let worktree = WorktreeId(checkout_id);

    let Some(revision_ceiling) = capture_revision_cut(&git_dir, &store, &worktree) else {
        return empty_patch();
    };

    resolve_bounded_mutation_attribution(
        &store,
        &snapshot,
        &worktree,
        direct_coverage,
        committed_patch,
        &commit_tree,
        Some(revision_ceiling),
    )
    .result
    .mutation_ai_patch
}

fn capture_revision_cut(
    git_dir: &Path,
    store: &MutationTraceStore<'_>,
    worktree: &WorktreeId,
) -> Option<u64> {
    let _lock = WorktreeLock::acquire(git_dir, REVISION_CUT_LOCK_TIMEOUT).ok()?;
    store
        .latest_mutation_event_revision(worktree)
        .ok()
        .flatten()
}

#[cfg(test)]
#[path = "mutation_attribution/tests.rs"]
mod tests;
