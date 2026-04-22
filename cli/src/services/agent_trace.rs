//! Minimal agent-trace domain model and hunk classification contract.
//!
//! This module defines the domain types for producing a minimal agent-trace
//! JSON payload from patch data. The classification logic computes the
//! intersection of a constructed patch and a post-commit patch
//! (`intersection_patch = intersect_patches(constructed_patch, post_commit_patch)`),
//! then compares `intersection_patch` against `post_commit_patch` hunk by hunk,
//! anchored to `post_commit_patch` as the canonical source of truth.
//!
//! Classification rules:
//! - **exact** line-by-line match between `intersection_patch` and `post_commit_patch` hunk => `ai`
//! - same hunk slot in `post_commit_patch` but not exact line-by-line match => `mixed`
//! - hunk present in `post_commit_patch` but missing from `intersection_patch` => `unknown`

use serde::{Deserialize, Serialize};

use super::patch::{intersect_patches, ParsedPatch, PatchHunk};

/// Classification of a single hunk's origin relative to the AI candidate patch.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HunkContributor {
    /// All touched lines in the `post_commit_patch` hunk are present identically
    /// in `intersection_patch`.
    Ai,
    /// The `post_commit_patch` hunk has a corresponding slot in `intersection_patch`
    /// but the content differs.
    Mixed,
    /// The `post_commit_patch` hunk has no corresponding slot in `intersection_patch`
    /// (missing from AI candidate).
    Unknown,
}

/// A single conversation entry derived from one `post_commit_patch` hunk.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Conversation {
    /// Classification of this hunk's origin.
    pub contributor: Contributor,
    /// Line ranges in the new file, derived from the `post_commit_patch` hunk metadata.
    pub ranges: Vec<LineRange>,
}

/// Nested contributor object for a conversation entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Contributor {
    /// Classification of this hunk's origin.
    #[serde(rename = "type")]
    pub kind: HunkContributor,
}

/// A single line range in the new file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct LineRange {
    pub start_line: u64,
    pub end_line: u64,
}

/// A file-level entry in the minimal agent-trace payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TraceFile {
    /// Post-change file path (from `post_commit_patch`).
    pub path: String,
    /// One conversation per `post_commit_patch` hunk, in `post_commit_patch` hunk order.
    pub conversations: Vec<Conversation>,
}

/// Top-level minimal agent-trace payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentTrace {
    /// File-level trace entries, one per file present in `post_commit_patch`.
    pub files: Vec<TraceFile>,
}

/// Classify a single `post_commit_patch` hunk against the corresponding
/// `intersection_patch` hunk (if any).
///
/// Two hunks correspond when they share the same `old_start` value within the
/// same file. This is the slot-matching rule that aligns `intersection_patch`
/// hunks to `post_commit_patch` hunks for comparison.
///
/// Returns:
/// - `HunkContributor::Ai` when the `intersection_patch` hunk exists and its
///   touched lines match the `post_commit_patch` hunk's touched lines exactly
///   (same count, same kind, same `line_number`, same content, in the same order).
/// - `HunkContributor::Mixed` when the `intersection_patch` hunk exists but its
///   touched lines differ from the `post_commit_patch` hunk's touched lines.
/// - `HunkContributor::Unknown` when no `intersection_patch` hunk with the same
///   `old_start` exists for this `post_commit_patch` hunk.
pub fn classify_hunk(
    post_commit_hunk: &PatchHunk,
    intersection_hunks: &[PatchHunk],
) -> HunkContributor {
    let Some(intersection_hunk) = intersection_hunks
        .iter()
        .find(|h| h.old_start == post_commit_hunk.old_start)
    else {
        return HunkContributor::Unknown;
    };

    if hunks_match_exactly(post_commit_hunk, intersection_hunk) {
        HunkContributor::Ai
    } else {
        HunkContributor::Mixed
    }
}

/// Check whether two hunks have identical touched lines in the same order.
fn hunks_match_exactly(left: &PatchHunk, right: &PatchHunk) -> bool {
    if left.lines.len() != right.lines.len() {
        return false;
    }
    left.lines.iter().zip(right.lines.iter()).all(|(ll, rl)| {
        ll.kind == rl.kind && ll.line_number == rl.line_number && ll.content == rl.content
    })
}

fn line_range_from_hunk(hunk: &PatchHunk) -> LineRange {
    let start_line = hunk.new_start;
    let end_line = start_line.saturating_add(hunk.new_count.saturating_sub(1));

    LineRange {
        start_line,
        end_line,
    }
}

/// Build the minimal agent-trace payload from two patches.
///
/// Computes `intersection_patch = intersect_patches(constructed_patch, post_commit_patch)`,
/// then iterates over `post_commit_patch`'s files and hunks to classify each hunk
/// against `intersection_patch`. The output contains one `TraceFile` per file in
/// `post_commit_patch` and one `Conversation` per hunk in `post_commit_patch`,
/// preserving `post_commit_patch`'s file and hunk ordering.
///
/// Files in `post_commit_patch` that have no corresponding file in
/// `intersection_patch` still appear in the output with all hunks classified
/// as `Unknown`.
#[allow(dead_code)]
pub fn build_agent_trace(
    constructed_patch: &ParsedPatch,
    post_commit_patch: &ParsedPatch,
) -> AgentTrace {
    let intersection_patch = intersect_patches(constructed_patch, post_commit_patch);

    let files = post_commit_patch
        .files
        .iter()
        .map(|post_commit_file| {
            let intersection_file = intersection_patch
                .files
                .iter()
                .find(|ifile| ifile.new_path == post_commit_file.new_path);

            let conversations = post_commit_file
                .hunks
                .iter()
                .map(|post_commit_hunk| {
                    let contributor = match intersection_file {
                        Some(ifile) => classify_hunk(post_commit_hunk, &ifile.hunks),
                        None => HunkContributor::Unknown,
                    };
                    Conversation {
                        contributor: Contributor { kind: contributor },
                        ranges: vec![line_range_from_hunk(post_commit_hunk)],
                    }
                })
                .collect();

            TraceFile {
                path: post_commit_file.new_path.clone(),
                conversations,
            }
        })
        .collect();

    AgentTrace { files }
}

#[cfg(test)]
mod tests;
