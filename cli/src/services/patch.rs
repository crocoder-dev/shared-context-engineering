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

#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
pub fn load_patch_from_json(input: &str) -> Result<ParsedPatch, PatchLoadError> {
    serde_json::from_str(input).map_err(|e| PatchLoadError {
        message: format!("invalid patch JSON: {e}"),
    })
}

/// Load a `ParsedPatch` from JSON bytes previously produced by serializing
/// a `ParsedPatch`.
///
/// This is the bytes-oriented counterpart to [`load_patch_from_json`],
/// convenient when the caller has raw bytes (for example, from a database
/// BLOB column or a file read) rather than a UTF-8 string.
///
/// # Errors
///
/// Returns `PatchLoadError` with an actionable message when the input is
/// not valid JSON or does not match the expected `ParsedPatch` structure.
#[allow(dead_code)]
pub fn load_patch_from_json_bytes(input: &[u8]) -> Result<ParsedPatch, PatchLoadError> {
    serde_json::from_slice(input).map_err(|e| PatchLoadError {
        message: format!("invalid patch JSON: {e}"),
    })
}

/// Compute the touched-line intersection of two patches.
///
/// Returns a `ParsedPatch` containing only the touched lines from
/// `post_commit_patch` that are also represented in `constructed_patch` for the
/// same logical file. Files are matched by their post-change path identity:
/// exact `new_path` equality, or an absolute path whose normalized path segments
/// end with the same relative path segments.
///
/// Matching prefers exact touched-line identity (`kind`, `line_number`, and
/// `content`). When no exact match exists, it falls back to historical
/// reconstruction matching by `kind` and `content` only, which lets callers
/// compare a canonical post-commit patch against earlier incremental diffs even
/// when line numbers drift across intermediate edits.
///
/// Files with no overlapping touched lines are excluded from the result.
/// Within matched files, hunks are reconstructed from the overlapping lines in
/// `post_commit_patch`, preserving `post_commit_patch`'s hunk metadata so the
/// result can be compared directly to the canonical target patch. The output is
/// deterministic: the same inputs always produce the same result.
///
/// # Examples
///
/// ```
/// use sce::services::patch::{intersect_patches, parse_patch};
///
/// let constructed_patch = parse_patch("...")?;
/// let post_commit_patch = parse_patch("...")?;
/// let overlap = intersect_patches(&constructed_patch, &post_commit_patch);
/// ```
#[allow(dead_code)]
pub fn intersect_patches(
    constructed_patch: &ParsedPatch,
    post_commit_patch: &ParsedPatch,
) -> ParsedPatch {
    let mut result_files: Vec<PatchFileChange> = Vec::new();

    for post_commit_file in &post_commit_patch.files {
        // Only consider files that also appear in `constructed_patch` by equivalent post-change path.
        let Some(constructed_file) = constructed_patch.files.iter().find(|constructed_file| {
            paths_refer_to_same_file(&constructed_file.new_path, &post_commit_file.new_path)
        }) else {
            continue;
        };

        let available_lines: Vec<&TouchedLine> = constructed_file
            .hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .collect();
        let mut used_lines = vec![false; available_lines.len()];

        // Filter hunks in `post_commit_file` to only include lines that are also
        // represented in `constructed_file`, preferring exact line-number matches
        // and falling back to same-kind/same-content historical matches when line
        // numbers have drifted.
        let mut result_hunks: Vec<PatchHunk> = Vec::new();
        for post_commit_hunk in &post_commit_file.hunks {
            let overlapping_lines: Vec<TouchedLine> = post_commit_hunk
                .lines
                .iter()
                .filter(|line| {
                    if let Some(index) = find_available_line_match(
                        &available_lines,
                        &used_lines,
                        line,
                        touched_lines_match_exact,
                    ) {
                        used_lines[index] = true;
                        return true;
                    }

                    if let Some(index) = find_available_line_match(
                        &available_lines,
                        &used_lines,
                        line,
                        touched_lines_match_historical,
                    ) {
                        used_lines[index] = true;
                        return true;
                    }

                    false
                })
                .cloned()
                .collect();

            if overlapping_lines.is_empty() {
                continue;
            }

            result_hunks.push(PatchHunk {
                old_start: post_commit_hunk.old_start,
                old_count: post_commit_hunk.old_count,
                new_start: post_commit_hunk.new_start,
                new_count: post_commit_hunk.new_count,
                lines: overlapping_lines,
            });
        }

        if result_hunks.is_empty() {
            continue;
        }

        result_files.push(PatchFileChange {
            old_path: post_commit_file.old_path.clone(),
            new_path: post_commit_file.new_path.clone(),
            kind: post_commit_file.kind,
            hunks: result_hunks,
        });
    }

    ParsedPatch {
        files: result_files,
    }
}

fn find_available_line_match(
    available_lines: &[&TouchedLine],
    used_lines: &[bool],
    target: &TouchedLine,
    matcher: fn(&TouchedLine, &TouchedLine) -> bool,
) -> Option<usize> {
    available_lines
        .iter()
        .enumerate()
        .find_map(|(index, candidate)| {
            (!used_lines[index] && matcher(candidate, target)).then_some(index)
        })
}

fn touched_lines_match_exact(candidate: &TouchedLine, target: &TouchedLine) -> bool {
    candidate.kind == target.kind
        && candidate.line_number == target.line_number
        && candidate.content == target.content
}

fn touched_lines_match_historical(candidate: &TouchedLine, target: &TouchedLine) -> bool {
    candidate.kind == target.kind && candidate.content == target.content
}

fn paths_refer_to_same_file(path_a: &str, path_b: &str) -> bool {
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

/// Combine multiple patches into one deterministic result.
///
/// Merges all file changes from the input patches, grouped by `new_path`.
/// When multiple patches touch the same file, touched-line entries are
/// deduplicated by identity (`kind`, `line_number`, `content`), with later
/// patches winning over earlier ones for the same identity.
///
/// File metadata (`old_path`, `kind`) is also taken from the last patch
/// that contributed to each file. Hunk metadata is preserved from the
/// last patch that contributed each surviving touched line.
///
/// Files appear in the result in the order they are first encountered
/// across the input patches. Within each file, hunks are ordered by
/// `old_start` and lines within each hunk are ordered by `line_number`
/// with `Removed` lines before `Added` lines at the same position.
///
/// The result is deterministic: the same inputs in the same order always
/// produce the same output.
///
/// # Examples
///
/// ```
/// use sce::services::patch::combine_patches;
///
/// let combined = combine_patches(&[patch_a, patch_b]);
/// ```
#[allow(dead_code)]
pub fn combine_patches(patches: &[ParsedPatch]) -> ParsedPatch {
    use std::collections::HashMap;

    /// Touched-line identity key: (`kind`, `line_number`, `content`).
    type LineKey = (TouchedLineKind, u64, String);
    /// Hunk metadata key: (`old_start`, `old_count`, `new_start`, `new_count`).
    type HunkMeta = (u64, u64, u64, u64);

    #[allow(clippy::type_complexity)]
    struct FileAcc {
        old_path: String,
        kind: FileChangeKind,
        lines: HashMap<LineKey, (TouchedLine, HunkMeta)>,
    }

    let mut file_order: Vec<String> = Vec::new();
    let mut files: HashMap<String, FileAcc> = HashMap::new();

    for patch in patches {
        for file in &patch.files {
            let acc = files.entry(file.new_path.clone()).or_insert_with(|| {
                file_order.push(file.new_path.clone());
                FileAcc {
                    old_path: file.old_path.clone(),
                    kind: file.kind,
                    lines: HashMap::new(),
                }
            });
            // Later patch wins for file metadata.
            acc.old_path.clone_from(&file.old_path);
            acc.kind = file.kind;

            for hunk in &file.hunks {
                let hunk_meta: HunkMeta = (
                    hunk.old_start,
                    hunk.old_count,
                    hunk.new_start,
                    hunk.new_count,
                );
                for line in &hunk.lines {
                    let line_key = (line.kind, line.line_number, line.content.clone());
                    acc.lines.insert(line_key, (line.clone(), hunk_meta));
                }
            }
        }
    }

    let mut result_files = Vec::new();

    for path in file_order {
        let acc = files.remove(&path).unwrap();

        // Group surviving lines by their hunk metadata.
        let mut hunk_groups: HashMap<HunkMeta, Vec<TouchedLine>> = HashMap::new();
        for (_line_key, (line, hunk_meta)) in acc.lines {
            hunk_groups.entry(hunk_meta).or_default().push(line);
        }

        // Sort hunk groups by old_start for deterministic output.
        let mut sorted_hunks: Vec<_> = hunk_groups.into_iter().collect();
        sorted_hunks.sort_by_key(|(meta, _)| meta.0);

        let mut hunks = Vec::new();
        for (meta, mut lines) in sorted_hunks {
            // Sort lines within each hunk: by line_number, then Removed before
            // Added, then by content for full determinism.
            lines.sort_by(|a, b| {
                a.line_number
                    .cmp(&b.line_number)
                    .then_with(|| {
                        let a_order = match a.kind {
                            TouchedLineKind::Removed => 0,
                            TouchedLineKind::Added => 1,
                        };
                        let b_order = match b.kind {
                            TouchedLineKind::Removed => 0,
                            TouchedLineKind::Added => 1,
                        };
                        a_order.cmp(&b_order)
                    })
                    .then_with(|| a.content.cmp(&b.content))
            });
            hunks.push(PatchHunk {
                old_start: meta.0,
                old_count: meta.1,
                new_start: meta.2,
                new_count: meta.3,
                lines,
            });
        }

        result_files.push(PatchFileChange {
            old_path: acc.old_path,
            new_path: path,
            kind: acc.kind,
            hunks,
        });
    }

    ParsedPatch {
        files: result_files,
    }
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
#[allow(dead_code)]
pub fn parse_patch(input: &str) -> Result<ParsedPatch, ParseError> {
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
                let hunk = parse_hunk_header_and_body(rest, &mut lines)?;
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
        let kind = self
            .kind
            .unwrap_or_else(|| determine_file_kind(&self.old_path, &self.new_path));
        PatchFileChange {
            old_path: self.old_path,
            new_path: self.new_path,
            kind,
            hunks: self.hunks,
        }
    }
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
            });
            new_line_num += 1;
        } else if let Some(content) = line.strip_prefix('-') {
            // Removed line
            touched_lines.push(TouchedLine {
                kind: TouchedLineKind::Removed,
                line_number: old_line_num,
                content: content.to_string(),
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

#[cfg(test)]
#[path = "patch/tests.rs"]
mod tests;
