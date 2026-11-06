use anyhow::Result;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_db::CommitTimeCutoff;
use crate::services::mutation_trace::attribution::{
    logical_path, patch_for_locations, patch_for_locations_with_provenance,
    MutationAttributionResult, PatchLineLocation, PatchLineProvenance,
};
use crate::services::mutation_trace::lineage::{LineProvenance, MutationLineage, TransitionOrigin};
use crate::services::mutation_trace::store::{
    AttributionKind, MutationEventPageRow, MutationTraceStore, ScopeProvenance,
    MUTATION_ATTRIBUTION_PAGE_SIZE,
};
use crate::services::mutation_trace::types::{FailureKind, ScopeId, TreeId, WorktreeId};
use crate::services::patch::{parse_patch, ParsedPatch, PatchFileChange, TouchedLineKind};

use super::git_snapshot::{
    resolve_git_dir, resolve_worktree_id, CommittedTrees, GitSnapshotService,
};
use super::worktree_lock::WorktreeLock;

pub const MAX_MUTATION_ATTRIBUTION_EVENTS: usize = 128;

const REVISION_CUT_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

pub trait MutationEventPageSource {
    async fn load_mutation_event_page(
        &self,
        worktree: &WorktreeId,
        revision_cursor: Option<u64>,
        requested_limit: usize,
    ) -> Result<Vec<MutationEventPageRow>>;

    async fn load_scope_provenance(&self, scope_id: &ScopeId) -> Result<Option<ScopeProvenance>> {
        let _ = scope_id;
        Ok(None)
    }
}

impl MutationEventPageSource for MutationTraceStore<'_> {
    async fn load_mutation_event_page(
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
        .await
    }

    async fn load_scope_provenance(&self, scope_id: &ScopeId) -> Result<Option<ScopeProvenance>> {
        MutationTraceStore::load_scope_provenance(self, scope_id).await
    }
}

struct CommitBoundedEvents<'s, 'db> {
    store: &'s MutationTraceStore<'db>,
    cutoff: CommitTimeCutoff,
}

impl MutationEventPageSource for CommitBoundedEvents<'_, '_> {
    async fn load_mutation_event_page(
        &self,
        worktree: &WorktreeId,
        revision_cursor: Option<u64>,
        requested_limit: usize,
    ) -> Result<Vec<MutationEventPageRow>> {
        self.store
            .load_mutation_event_page_before_commit(
                worktree,
                revision_cursor,
                requested_limit,
                self.cutoff,
            )
            .await
    }

    async fn load_scope_provenance(&self, scope_id: &ScopeId) -> Result<Option<ScopeProvenance>> {
        self.store.load_scope_provenance(scope_id).await
    }
}

pub trait TreeReadSource {
    async fn diff_trees(&self, before: &TreeId, after: &TreeId) -> Result<String>;
    async fn file_at_tree(&self, tree: &TreeId, path: &str) -> Result<Option<String>>;
}

impl TreeReadSource for GitSnapshotService {
    async fn diff_trees(&self, before: &TreeId, after: &TreeId) -> Result<String> {
        GitSnapshotService::diff_trees(self, before, after).await
    }

    async fn file_at_tree(&self, tree: &TreeId, path: &str) -> Result<Option<String>> {
        GitSnapshotService::file_at_tree(self, tree, path).await
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
    /// Whether an uninterrupted replay from the exact parent tree reached the
    /// last loaded event. Removed lines resolve only when it did, and only in
    /// paths that also reconciled with the commit tree.
    pub removal_replay_anchored: bool,
}

impl BoundedMutationAttribution {
    fn empty() -> Self {
        BoundedMutationAttribution {
            result: empty_attribution(),
            loaded_pages: 0,
            loaded_rows: 0,
            inspected_events: 0,
            reconstructed_events: 0,
            gap_resets: 0,
            barrier: None,
            removal_replay_anchored: false,
        }
    }
}

fn empty_patch() -> ParsedPatch {
    ParsedPatch { files: Vec::new() }
}

fn empty_attribution() -> MutationAttributionResult {
    MutationAttributionResult {
        mutation_ai_patch: empty_patch(),
        resolved_non_ai_patch: empty_patch(),
        unresolved_patch: empty_patch(),
    }
}

/// Resolves committed touched lines from bounded mutation history.
///
/// Added lines use the existing gap-tolerant replay. Removed lines resolve only
/// through a separate, stricter replay anchored at `parent_tree`; without a
/// parent tree (root or merge commits) every removed line stays unresolved.
#[allow(clippy::too_many_arguments)]
pub async fn resolve_bounded_mutation_attribution<P, R>(
    page_source: &P,
    tree_source: &R,
    worktree: &WorktreeId,
    committed_patch: &ParsedPatch,
    commit_tree: &TreeId,
    parent_tree: Option<&TreeId>,
    revision_ceiling: Option<u64>,
) -> BoundedMutationAttribution
where
    P: MutationEventPageSource + ?Sized,
    R: TreeReadSource,
{
    let target_paths = target_logical_paths(committed_patch);
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

    let events = load_event_window(page_source, worktree, revision_ceiling, &mut state).await;
    let lineages = if events.is_empty() {
        None
    } else {
        let removal_anchor = parent_tree.map(|parent_tree| RemovalAnchor {
            parent_tree,
            paths: removal_parent_paths(committed_patch),
        });
        Some(
            replay(
                &events,
                &target_paths,
                removal_anchor,
                commit_tree,
                tree_source,
                &mut state,
            )
            .await,
        )
    };
    finish(committed_patch, lineages.as_ref(), &state, page_source).await
}

struct ReplayState {
    loaded_pages: usize,
    loaded_rows: usize,
    inspected_events: usize,
    reconstructed_events: usize,
    gap_resets: usize,
    barrier: Option<MutationAttributionBarrier>,
}

struct ReplayedLineages {
    added: MutationLineage,
    removals: Option<MutationLineage>,
}

async fn finish<P>(
    target: &ParsedPatch,
    lineages: Option<&ReplayedLineages>,
    state: &ReplayState,
    page_source: &P,
) -> BoundedMutationAttribution
where
    P: MutationEventPageSource + ?Sized,
{
    let result = match lineages {
        Some(lineages) => {
            project(
                target,
                &lineages.added,
                lineages.removals.as_ref(),
                page_source,
            )
            .await
        }
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
        removal_replay_anchored: lineages.is_some_and(|lineages| lineages.removals.is_some()),
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

/// Every path whose parent lines a committed removal can name, plus the path
/// that holds them in the commit.
fn removal_parent_paths(target: &ParsedPatch) -> BTreeSet<String> {
    target
        .files
        .iter()
        .filter(|file| {
            file.hunks.iter().any(|hunk| {
                hunk.lines
                    .iter()
                    .any(|line| line.kind == TouchedLineKind::Removed)
            })
        })
        .flat_map(|file| [file.old_path.clone(), file.new_path.clone()])
        .filter(|path| !path.is_empty())
        .collect()
}

struct RemovalAnchor<'a> {
    parent_tree: &'a TreeId,
    paths: BTreeSet<String>,
}

/// The removal replay: it starts at the last event leaving the exact parent
/// tree and is abandoned, never reset, at the first gap or failed transition.
struct RemovalReplay {
    anchor: usize,
    lineage: MutationLineage,
}

async fn start_removal_replay<R>(
    events: &[MutationEventPageRow],
    anchor: Option<RemovalAnchor<'_>>,
    tree_source: &R,
) -> Option<RemovalReplay>
where
    R: TreeReadSource,
{
    let anchor_spec = anchor?;
    let anchor = events
        .iter()
        .rposition(|row| row.before_tree == *anchor_spec.parent_tree)?;
    let mut baseline = BTreeMap::new();
    for path in &anchor_spec.paths {
        let content = tree_source
            .file_at_tree(anchor_spec.parent_tree, path)
            .await
            .ok()?;
        baseline.insert(path.clone(), content);
    }
    Some(RemovalReplay {
        anchor,
        lineage: MutationLineage::from_parent(&baseline),
    })
}

fn advance_removal_replay(
    replay: &mut Option<RemovalReplay>,
    index: usize,
    row: &MutationEventPageRow,
    prev_after: &TreeId,
    reconstructed: Option<&ParsedPatch>,
    origin: &TransitionOrigin,
) {
    let Some(active) = replay.as_mut() else {
        return;
    };
    if index < active.anchor {
        return;
    }
    let contiguous = index == active.anchor || row.before_tree == *prev_after;
    let applied = contiguous
        && reconstructed.is_some_and(|patch| active.lineage.apply(patch, origin).is_ok());
    if !applied {
        *replay = None;
    }
}

/// Reconciles the removal replay with the commit tree, path by path. A path
/// keeps its removal claims only when it holds exactly its committed lines and
/// any unobserved tail between the last event and the commit left it
/// byte-identical; otherwise (for example a partially staged file) every
/// removal claim the path bears on becomes unknown.
async fn settle_removal_replay<R>(
    replay: Option<RemovalReplay>,
    last_after: &TreeId,
    commit_tree: &TreeId,
    tree_source: &R,
) -> Option<MutationLineage>
where
    R: TreeReadSource,
{
    let mut lineage = replay?.lineage;
    let paths: Vec<String> = lineage.tracked_paths().map(ToOwned::to_owned).collect();
    for path in &paths {
        let committed = tree_source.file_at_tree(commit_tree, path).await.ok()?;
        let tail_touches_path = last_after != commit_tree
            && tree_source.file_at_tree(last_after, path).await.ok()? != committed;
        if tail_touches_path || !lineage.matches_content(path, committed.as_deref()) {
            lineage.distrust_removals(path);
        }
    }
    Some(lineage)
}

async fn load_event_window<P>(
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

        let Ok(page) = page_source
            .load_mutation_event_page(worktree, cursor, want)
            .await
        else {
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

async fn replay<R>(
    events: &[MutationEventPageRow],
    target_paths: &BTreeSet<String>,
    removal_anchor: Option<RemovalAnchor<'_>>,
    commit_tree: &TreeId,
    tree_source: &R,
    state: &mut ReplayState,
) -> ReplayedLineages
where
    R: TreeReadSource,
{
    let mut removal_replay = start_removal_replay(events, removal_anchor, tree_source).await;
    let mut lineage = MutationLineage::from_baseline(
        &load_baseline(tree_source, &events[0].before_tree, target_paths).await,
    );
    let mut prev_after = events[0].before_tree.clone();

    for (index, row) in events.iter().enumerate() {
        state.inspected_events += 1;

        if row.before_tree != prev_after {
            lineage.reset_all(&load_baseline(tree_source, &row.before_tree, target_paths).await);
            state.gap_resets += 1;
        }

        let reconstructed = tree_source
            .diff_trees(&row.before_tree, &row.after_tree)
            .await
            .ok()
            .and_then(|text| parse_patch(&text, None).ok());
        if reconstructed.is_some() {
            state.reconstructed_events += 1;
        }

        let origin = transition_origin(row);
        advance_removal_replay(
            &mut removal_replay,
            index,
            row,
            &prev_after,
            reconstructed.as_ref(),
            &origin,
        );
        if apply_or_reset(
            &mut lineage,
            reconstructed.as_ref(),
            &origin,
            &row.after_tree,
            target_paths,
            tree_source,
        )
        .await
        {
            state.barrier = Some(MutationAttributionBarrier::EventReconstruction);
            state.gap_resets += 1;
        }

        prev_after = row.after_tree.clone();
    }

    let removals =
        settle_removal_replay(removal_replay, &prev_after, commit_tree, tree_source).await;

    if prev_after != *commit_tree {
        let tail = tree_source
            .diff_trees(&prev_after, commit_tree)
            .await
            .ok()
            .and_then(|text| parse_patch(&text, None).ok());
        if apply_or_reset(
            &mut lineage,
            tail.as_ref(),
            &TransitionOrigin::Unobserved,
            commit_tree,
            target_paths,
            tree_source,
        )
        .await
        {
            state.barrier = Some(MutationAttributionBarrier::Tail);
            state.gap_resets += 1;
        }
    }

    ReplayedLineages {
        added: lineage,
        removals,
    }
}

async fn apply_or_reset<R>(
    lineage: &mut MutationLineage,
    patch: Option<&ParsedPatch>,
    origin: &TransitionOrigin,
    fallback_tree: &TreeId,
    target_paths: &BTreeSet<String>,
    tree_source: &R,
) -> bool
where
    R: TreeReadSource,
{
    let applied = patch.is_some_and(|patch| lineage.apply(patch, origin).is_ok());
    if !applied {
        lineage.reset_all(&load_baseline(tree_source, fallback_tree, target_paths).await);
    }
    !applied
}

async fn load_baseline<R>(
    tree_source: &R,
    tree: &TreeId,
    target_paths: &BTreeSet<String>,
) -> std::collections::BTreeMap<String, Option<String>>
where
    R: TreeReadSource,
{
    let mut baseline = std::collections::BTreeMap::new();
    for path in target_paths {
        let content = tree_source.file_at_tree(tree, path).await.unwrap_or(None);
        baseline.insert(path.clone(), content);
    }
    baseline
}

fn transition_origin(row: &MutationEventPageRow) -> TransitionOrigin {
    let healthy = row.failure_kind == FailureKind::Healthy;
    match (&row.attribution_kind, &row.attribution_scope_id) {
        (AttributionKind::AiExclusive, Some(scope_id)) if healthy => {
            TransitionOrigin::MutationAi(scope_id.clone())
        }
        _ => TransitionOrigin::MutationNonAi,
    }
}

#[derive(Default)]
struct ProjectedLines {
    ai: BTreeMap<PatchLineLocation, ScopeId>,
    non_ai: BTreeSet<PatchLineLocation>,
    unresolved: BTreeSet<PatchLineLocation>,
}

impl ProjectedLines {
    fn assign(&mut self, location: PatchLineLocation, provenance: LineProvenance) {
        match provenance {
            LineProvenance::MutationAi { scope_id } => {
                self.ai.insert(location, scope_id);
            }
            LineProvenance::MutationNonAi => {
                self.non_ai.insert(location);
            }
            LineProvenance::Unknown => {
                self.unresolved.insert(location);
            }
        }
    }
}

/// Removed lines are resolved by their canonical old-side identity: the
/// file's old path and old line number, never new-file coordinates.
fn removed_line_provenance(
    file: &PatchFileChange,
    removals: Option<&MutationLineage>,
    removed: &[(u64, &str)],
) -> Vec<LineProvenance> {
    match removals {
        Some(lineage) if !file.old_path.is_empty() => {
            lineage.committed_removals(logical_path(file), &file.old_path, removed)
        }
        _ => vec![LineProvenance::Unknown; removed.len()],
    }
}

async fn project<P>(
    target: &ParsedPatch,
    lineage: &MutationLineage,
    removals: Option<&MutationLineage>,
    page_source: &P,
) -> MutationAttributionResult
where
    P: MutationEventPageSource + ?Sized,
{
    let mut projected = ProjectedLines::default();

    for (file_index, file) in target.files.iter().enumerate() {
        let path = logical_path(file);
        let mut removed_locations = Vec::new();
        let mut removed_lines = Vec::new();
        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            for (line_index, line) in hunk.lines.iter().enumerate() {
                let location = PatchLineLocation {
                    file_index,
                    hunk_index,
                    line_index,
                };
                match line.kind {
                    TouchedLineKind::Added => projected.assign(
                        location,
                        lineage.provenance_at(path, line.line_number, &line.content),
                    ),
                    TouchedLineKind::Removed => {
                        removed_locations.push(location);
                        removed_lines.push((line.line_number, line.content.as_str()));
                    }
                }
            }
        }
        let provenances = removed_line_provenance(file, removals, &removed_lines);
        for (location, provenance) in removed_locations.into_iter().zip(provenances) {
            projected.assign(location, provenance);
        }
    }
    let ProjectedLines {
        ai,
        non_ai,
        unresolved,
    } = projected;

    let mut scope_provenance = BTreeMap::new();
    for scope_id in ai.values().collect::<BTreeSet<_>>() {
        let provenance = page_source
            .load_scope_provenance(scope_id)
            .await
            .ok()
            .flatten();
        scope_provenance.insert(scope_id.clone(), provenance);
    }
    let ai_with_provenance = ai
        .into_iter()
        .map(|(location, scope_id)| {
            let provenance = scope_provenance.get(&scope_id).and_then(Option::as_ref);
            (
                location,
                PatchLineProvenance {
                    session_id: provenance.map(|value| value.session_id.clone()),
                    model_id: provenance.and_then(|value| value.model_id.clone()),
                },
            )
        })
        .collect();

    MutationAttributionResult {
        mutation_ai_patch: patch_for_locations_with_provenance(target, &ai_with_provenance),
        resolved_non_ai_patch: patch_for_locations(target, &non_ai),
        unresolved_patch: patch_for_locations(target, &unresolved),
    }
}

/// Resolves mutation attribution for the commit captured as `commit_oid`.
///
/// Both the commit tree and its first-parent tree are resolved from that exact
/// object ID, never from a mutable `HEAD`, so a later `HEAD` move cannot
/// change which revisions removed lines are anchored to.
pub(crate) async fn resolve_post_commit_mutation_attribution(
    repository_root: &Path,
    db: &mut RepositoryAgentTraceDb,
    commit_oid: &str,
    committed_patch: &ParsedPatch,
    cutoff: Option<CommitTimeCutoff>,
) -> MutationAttributionResult {
    let Some(cutoff) = cutoff else {
        return empty_attribution();
    };
    let Ok(git_dir) = resolve_git_dir(repository_root).await else {
        return empty_attribution();
    };
    let Ok(worktree) = resolve_worktree_id(repository_root).await else {
        return empty_attribution();
    };
    let Ok(snapshot) = GitSnapshotService::new(repository_root).await else {
        return empty_attribution();
    };
    let Ok(CommittedTrees {
        commit_tree,
        parent_tree,
    }) = snapshot.committed_trees(commit_oid).await
    else {
        return empty_attribution();
    };
    let store = MutationTraceStore::new(db);

    let Some(revision_ceiling) = capture_revision_cut(&git_dir, &store, &worktree).await else {
        return empty_attribution();
    };

    resolve_bounded_mutation_attribution(
        &CommitBoundedEvents {
            store: &store,
            cutoff,
        },
        &snapshot,
        &worktree,
        committed_patch,
        &commit_tree,
        parent_tree.as_ref(),
        Some(revision_ceiling),
    )
    .await
    .result
}

async fn capture_revision_cut(
    git_dir: &Path,
    store: &MutationTraceStore<'_>,
    worktree: &WorktreeId,
) -> Option<u64> {
    let _lock = WorktreeLock::acquire_async(git_dir, REVISION_CUT_LOCK_TIMEOUT)
        .await
        .ok()?;
    store
        .latest_mutation_event_revision(worktree)
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
#[path = "mutation_attribution/tests.rs"]
mod tests;
