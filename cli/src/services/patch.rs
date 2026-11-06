//! Patch domain model and parser for in-memory parsed patch representation.
//!
//! This module defines the core types for representing parsed unified-diff
//! patches as structured data, capturing only touched lines (added/removed)
//! plus the minimal per-file/per-hunk metadata needed to interpret them.
//!
//! Non-hunk headers and unchanged context lines are intentionally excluded.
//!
//! The types are `serde`-serializable and deserializable so they can round-trip
//! through a structured representation (e.g., JSON) and be loaded back into
//! the same struct shape.
//!
//! The parser supports both `Index:` (SVN-style) and `diff --git` (git-style)
//! unified-diff formats and produces deterministic `ParsedPatch` structs from
//! raw patch text.

use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

/// Top-level parsed patch containing one or more file changes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ParsedPatch {
    pub files: Vec<PatchFileChange>,
}

/// A single file's changes within a patch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PatchFileChange {
    /// Path of the file before the change. Empty string for new files.
    pub old_path: String,
    /// Path of the file after the change. Empty string for deleted files.
    pub new_path: String,
    /// Kind of file change.
    pub kind: FileChangeKind,
    /// Hunks within this file change.
    pub hunks: Vec<PatchHunk>,
}

/// Kind of change applied to a file.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    /// File was newly created.
    Added,
    /// File was modified in place.
    Modified,
    /// File was deleted.
    Deleted,
    /// File was renamed (path changed, content may or may not have changed).
    Renamed,
}

/// A single hunk within a file change, containing touched lines.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PatchHunk {
    /// Starting line number in the old file (0 for new-file hunks).
    pub old_start: u64,
    /// Number of lines in the old file context for this hunk (0 for new-file hunks).
    pub old_count: u64,
    /// Starting line number in the new file (0 for deleted-file hunks).
    pub new_start: u64,
    /// Number of lines in the new file context for this hunk (0 for deleted-file hunks).
    pub new_count: u64,
    /// Optional model provenance associated with this hunk.
    #[serde(default)]
    pub model_id: Option<String>,
    /// Touched lines within this hunk (added and removed lines only;
    /// unchanged context lines are excluded).
    pub lines: Vec<TouchedLine>,
}

/// A single touched line within a hunk.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TouchedLine {
    /// Kind of line change.
    pub kind: TouchedLineKind,
    /// Line number in the new file for added lines, or in the old file
    /// for removed lines.
    pub line_number: u64,
    /// Content of the line (without the leading `+`/`-` prefix).
    pub content: String,
    /// Optional session identifier associated with this touched line.
    #[serde(default)]
    pub session_id: Option<String>,
}

/// Kind of touched line.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TouchedLineKind {
    /// Line was added.
    Added,
    /// Line was removed.
    Removed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseError {
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "patch parse error: {}", self.message)
    }
}

impl std::error::Error for ParseError {}

/// Error produced when loading a `ParsedPatch` from serialized JSON fails.
///
/// `PatchLoadError` carries an actionable message describing why the JSON
/// payload could not be reconstructed into a valid `ParsedPatch`. Common
/// causes include malformed JSON syntax, missing required fields, or type
/// mismatches in the serialized structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatchLoadError {
    pub message: String,
}

impl std::fmt::Display for PatchLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "patch load error: {}", self.message)
    }
}

impl std::error::Error for PatchLoadError {}

/// Load a `ParsedPatch` from a JSON string previously produced by
/// serializing a `ParsedPatch`.
///
/// This is the primary storage-agnostic entrypoint for reconstructing a
/// parsed patch from serialized JSON content. Callers who have already read
/// the JSON from a database, file, or any other source can pass the string
/// directly.
///
/// # Errors
///
/// Returns `PatchLoadError` with an actionable message when the input is
/// not valid JSON or does not match the expected `ParsedPatch` structure.
pub fn load_patch_from_json(input: &str) -> Result<ParsedPatch, PatchLoadError> {
    serde_json::from_str(input).map_err(|e| PatchLoadError {
        message: format!("invalid patch JSON: {e}"),
    })
}

pub(crate) fn paths_refer_to_same_file(path_a: &str, path_b: &str) -> bool {
    if path_a == path_b {
        return true;
    }

    let a_components = normalized_path_components(path_a);
    let b_components = normalized_path_components(path_b);

    if a_components.is_empty() || b_components.is_empty() {
        return false;
    }

    path_has_relative_suffix(&a_components, &b_components)
        || path_has_relative_suffix(&b_components, &a_components)
}

fn normalized_path_components(path: &str) -> Vec<&str> {
    Path::new(path)
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect()
}

fn path_has_relative_suffix<'a>(full_path: &[&'a str], suffix_candidate: &[&'a str]) -> bool {
    full_path.len() > suffix_candidate.len()
        && full_path.ends_with(suffix_candidate)
        && !suffix_candidate.is_empty()
}

/// Parse raw unified-diff text into a `ParsedPatch`.
///
/// Supports both `Index:` (SVN-style) and `diff --git` (git-style) patch
/// formats. Context lines (space-prefixed) are excluded from the output;
/// only added (`+`) and removed (`-`) lines are captured as touched lines.
///
/// # Errors
///
/// Returns `ParseError` with an actionable message when the input is malformed,
/// such as an invalid hunk header or a `---`/`+++` line that cannot be parsed.
pub fn parse_patch(input: &str, session_id: Option<&str>) -> Result<ParsedPatch, ParseError> {
    let mut files: Vec<PatchFileChange> = Vec::new();
    let mut current_file: Option<FileBuilder> = None;

    let mut lines = input.lines().peekable();

    while let Some(line) = lines.next() {
        // Detect file boundary: git-style diff header
        if let Some(rest) = line.strip_prefix("diff --git ") {
            // Finalize any in-progress file before starting a new one.
            if let Some(fb) = current_file.take() {
                files.push(fb.build());
            }
            let paths = parse_git_diff_header(rest);
            current_file = Some(FileBuilder::new(paths.old_path, paths.new_path));
            continue;
        }

        // Detect file boundary: Index: (SVN-style) header
        if let Some(rest) = line.strip_prefix("Index: ") {
            // Finalize any in-progress file before starting a new one.
            if let Some(fb) = current_file.take() {
                files.push(fb.build());
            }
            // The Index: line gives us the file path, but we'll also see ---/+++
            // lines that may refine it. For now, store a placeholder.
            let index_path = rest.trim().to_string();
            current_file = Some(FileBuilder::new(index_path.clone(), index_path));
            continue;
        }

        // Skip separator lines after Index:
        if line.starts_with("===") {
            continue;
        }

        // Handle --- and +++ lines
        if let Some(rest) = line.strip_prefix("--- ") {
            let fb = current_file.as_mut().ok_or_else(|| ParseError {
                message: format!(
                    "encountered '---' line without a preceding file header: {line:?}"
                ),
            })?;
            fb.set_old_path(parse_diff_path(rest));
            continue;
        }

        if let Some(rest) = line.strip_prefix("+++ ") {
            let fb = current_file.as_mut().ok_or_else(|| ParseError {
                message: format!(
                    "encountered '+++' line without a preceding file header: {line:?}"
                ),
            })?;
            fb.set_new_path(parse_diff_path(rest));
            continue;
        }

        // Skip git-style metadata lines between diff --git and the first hunk
        if line.starts_with("new file mode ")
            || line.starts_with("deleted file mode ")
            || line.starts_with("old mode ")
            || line.starts_with("new mode ")
            || line.starts_with("index ")
            || line.starts_with("similarity index ")
            || line.starts_with("rename from ")
            || line.starts_with("rename to ")
            || line.starts_with("copy from ")
            || line.starts_with("copy to ")
        {
            // Track file kind from metadata
            if let Some(fb) = current_file.as_mut() {
                if line.starts_with("new file mode ") {
                    fb.mark_added();
                } else if line.starts_with("deleted file mode ") {
                    fb.mark_deleted();
                } else if line.starts_with("rename from ") || line.starts_with("rename to ") {
                    fb.mark_renamed();
                }
            }
            continue;
        }

        // Parse hunk header: @@ -old_start[,old_count] +new_start[,new_count] @@
        if let Some(rest) = line.strip_prefix("@@ ") {
            if let Some(fb) = current_file.as_mut() {
                let hunk = parse_hunk_header_and_body(rest, &mut lines, session_id)?;
                fb.add_hunk(hunk);
            }
        }

        // Skip any other header or unrecognized lines between file sections
    }

    // Finalize the last file
    if let Some(fb) = current_file.take() {
        files.push(fb.build());
    }

    Ok(ParsedPatch { files })
}

/// Builder for `PatchFileChange` that tracks path and kind information
/// progressively as header lines are parsed.
struct FileBuilder {
    old_path: String,
    new_path: String,
    kind: Option<FileChangeKind>,
    hunks: Vec<PatchHunk>,
}

impl FileBuilder {
    fn new(old_path: String, new_path: String) -> Self {
        Self {
            old_path,
            new_path,
            kind: None,
            hunks: Vec::new(),
        }
    }

    fn set_old_path(&mut self, path: String) {
        self.old_path = path;
    }

    fn set_new_path(&mut self, path: String) {
        self.new_path = path;
    }

    fn mark_added(&mut self) {
        self.kind = Some(FileChangeKind::Added);
    }

    fn mark_deleted(&mut self) {
        self.kind = Some(FileChangeKind::Deleted);
    }

    fn mark_renamed(&mut self) {
        self.kind = Some(FileChangeKind::Renamed);
    }

    fn add_hunk(&mut self, hunk: PatchHunk) {
        self.hunks.push(hunk);
    }

    fn build(self) -> PatchFileChange {
        let inferred_kind = self.kind.unwrap_or_else(|| {
            infer_file_kind_from_hunks(&self.hunks)
                .unwrap_or_else(|| determine_file_kind(&self.old_path, &self.new_path))
        });
        let (old_path, new_path) =
            normalize_paths_for_kind(self.old_path, self.new_path, inferred_kind);

        PatchFileChange {
            old_path,
            new_path,
            kind: inferred_kind,
            hunks: self.hunks,
        }
    }
}

fn infer_file_kind_from_hunks(hunks: &[PatchHunk]) -> Option<FileChangeKind> {
    if hunks.is_empty() {
        return None;
    }

    if hunks
        .iter()
        .all(|hunk| hunk.old_start == 0 && hunk.old_count == 0)
    {
        return Some(FileChangeKind::Added);
    }

    if hunks
        .iter()
        .all(|hunk| hunk.new_start == 0 && hunk.new_count == 0)
    {
        return Some(FileChangeKind::Deleted);
    }

    None
}

fn normalize_paths_for_kind(
    mut old_path: String,
    mut new_path: String,
    kind: FileChangeKind,
) -> (String, String) {
    match kind {
        FileChangeKind::Added => old_path.clear(),
        FileChangeKind::Deleted => new_path.clear(),
        _ => {}
    }

    (old_path, new_path)
}

/// Determine file change kind from old/new paths when no explicit metadata
/// was found.
fn determine_file_kind(old_path: &str, new_path: &str) -> FileChangeKind {
    if old_path == "/dev/null" || old_path.is_empty() {
        FileChangeKind::Added
    } else if new_path == "/dev/null" || new_path.is_empty() {
        FileChangeKind::Deleted
    } else if old_path != new_path {
        FileChangeKind::Renamed
    } else {
        FileChangeKind::Modified
    }
}

/// Parse a `diff --git a/old b/new` header line (after stripping the prefix).
fn parse_git_diff_header(rest: &str) -> DiffPaths {
    // Format: "a/old_path b/new_path"
    // The paths can contain spaces, so we need to split on " b/" carefully.
    // Git format: diff --git a/path b/path
    if let Some(idx) = rest.find(" b/") {
        let old = rest[..idx]
            .strip_prefix("a/")
            .unwrap_or(&rest[..idx])
            .to_string();
        let new = rest[idx + 3..]
            .strip_prefix("b/")
            .unwrap_or(&rest[idx + 3..])
            .to_string();
        DiffPaths {
            old_path: old,
            new_path: new,
        }
    } else {
        // Fallback: treat the whole thing as both paths
        let path = rest.trim().to_string();
        DiffPaths {
            old_path: path.clone(),
            new_path: path,
        }
    }
}

struct DiffPaths {
    old_path: String,
    new_path: String,
}

/// Parse a `---` or `+++` path line, stripping prefixes and trailing whitespace.
fn parse_diff_path(rest: &str) -> String {
    let trimmed = rest.trim_end();
    // Strip common prefixes: a/ for git-style, /dev/null for new/deleted files
    if trimmed == "/dev/null" {
        return String::new();
    }
    if let Some(stripped) = trimmed.strip_prefix("a/") {
        return stripped.to_string();
    }
    if let Some(stripped) = trimmed.strip_prefix("b/") {
        return stripped.to_string();
    }
    // Strip absolute path prefix if present (Index:-style diffs sometimes
    // use absolute paths like /home/user/repo/file)
    trimmed.to_string()
}

/// Parse a hunk header (the part after `@@ `) and then consume hunk body lines
/// until the next file boundary or end of input.
fn parse_hunk_header_and_body<'a, I>(
    rest: &str,
    lines: &mut std::iter::Peekable<I>,
    session_id: Option<&str>,
) -> Result<PatchHunk, ParseError>
where
    I: Iterator<Item = &'a str>,
{
    // Format: -old_start[,old_count] +new_start[,new_count] @@ [optional context]
    let header_end = rest.find("@@").ok_or_else(|| ParseError {
        message: format!("invalid hunk header: missing closing '@@' in {rest:?}"),
    })?;
    let range_part = rest[..header_end].trim();
    let mut ranges = range_part.split_whitespace();
    let old_range = ranges.next().ok_or_else(|| ParseError {
        message: format!("invalid hunk header: missing old range in {range_part:?}"),
    })?;
    let new_range = ranges.next().ok_or_else(|| ParseError {
        message: format!("invalid hunk header: missing new range in {range_part:?}"),
    })?;

    let (old_start, old_count) = parse_range_part(old_range, '-')?;
    let (new_start, new_count) = parse_range_part(new_range, '+')?;

    // Consume hunk body lines until we hit a line that starts a new file
    // section or another hunk header or end of input.
    let mut touched_lines: Vec<TouchedLine> = Vec::new();
    let mut old_line_num = old_start;
    let mut new_line_num = new_start;

    while let Some(&line) = lines.peek() {
        // Stop at file boundaries
        if line.starts_with("diff --git ") || line.starts_with("Index: ") || line.starts_with("===")
        {
            break;
        }
        // Stop at next hunk header
        if line.starts_with("@@ ") {
            break;
        }
        // Stop at ---/+++ headers (next file section in Index: format)
        if line.starts_with("--- ") || line.starts_with("+++ ") {
            break;
        }
        // Stop at git metadata lines
        if line.starts_with("new file mode ")
            || line.starts_with("deleted file mode ")
            || line.starts_with("old mode ")
            || line.starts_with("new mode ")
            || line.starts_with("index ")
            || line.starts_with("similarity index ")
            || line.starts_with("rename from ")
            || line.starts_with("rename to ")
            || line.starts_with("copy from ")
            || line.starts_with("copy to ")
        {
            break;
        }

        // Consume the line
        let line = lines.next().unwrap();

        if let Some(content) = line.strip_prefix('+') {
            // Added line
            touched_lines.push(TouchedLine {
                kind: TouchedLineKind::Added,
                line_number: new_line_num,
                content: content.to_string(),
                session_id: session_id.map(str::to_string),
            });
            new_line_num += 1;
        } else if let Some(content) = line.strip_prefix('-') {
            // Removed line
            touched_lines.push(TouchedLine {
                kind: TouchedLineKind::Removed,
                line_number: old_line_num,
                content: content.to_string(),
                session_id: session_id.map(str::to_string),
            });
            old_line_num += 1;
        } else if line.starts_with(' ') || line.starts_with('\t') {
            // Context line — skip but advance both counters
            old_line_num += 1;
            new_line_num += 1;
        } else if line.is_empty() {
            // Empty line within a hunk body — could be a context line with
            // no leading space (some diffs emit this). Treat as context.
            old_line_num += 1;
            new_line_num += 1;
        } else if line.starts_with('\\') {
            // "\ No newline at end of file" — skip
        } else {
            // Unknown line format within hunk — skip
        }
    }

    Ok(PatchHunk {
        old_start,
        old_count,
        new_start,
        new_count,
        model_id: None,
        lines: touched_lines,
    })
}

/// Parse a range part like `-3,7` or `+1,9` from a hunk header.
fn parse_range_part(s: &str, prefix: char) -> Result<(u64, u64), ParseError> {
    let s = s.strip_prefix(prefix).unwrap_or(s).trim();
    let parts: Vec<&str> = s.splitn(2, ',').collect();
    let start: u64 = parts[0].parse().map_err(|_| ParseError {
        message: format!("invalid hunk range start in {s:?}"),
    })?;
    let count: u64 = if parts.len() > 1 {
        parts[1].parse().map_err(|_| ParseError {
            message: format!("invalid hunk range count in {s:?}"),
        })?
    } else {
        1
    };
    Ok((start, count))
}
