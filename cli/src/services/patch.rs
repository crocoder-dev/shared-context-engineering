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
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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

#[allow(dead_code)]
struct DiffPaths {
    old_path: String,
    new_path: String,
}

/// Parse a `---` or `+++` path line, stripping prefixes and trailing whitespace.
#[allow(dead_code)]
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
#[allow(dead_code)]
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
    let range_part = &rest[..header_end].trim();

    let (old_start, old_count) = parse_range_part(range_part, '-')?;
    let plus_pos = range_part.find('+').ok_or_else(|| ParseError {
        message: format!("invalid hunk header: missing '+' range in {range_part:?}"),
    })?;
    let new_range = &range_part[plus_pos..];
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
#[allow(dead_code)]
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
mod tests {
    use super::*;

    fn sample_added_file_change() -> PatchFileChange {
        PatchFileChange {
            old_path: String::new(),
            new_path: "poem.md".to_string(),
            kind: FileChangeKind::Added,
            hunks: vec![PatchHunk {
                old_start: 0,
                old_count: 0,
                new_start: 1,
                new_count: 9,
                lines: vec![
                    TouchedLine {
                        kind: TouchedLineKind::Added,
                        line_number: 1,
                        content: "Morning leans on the windowsill,".to_string(),
                    },
                    TouchedLine {
                        kind: TouchedLineKind::Added,
                        line_number: 2,
                        content: "Dust turns slow in borrowed gold.".to_string(),
                    },
                    TouchedLine {
                        kind: TouchedLineKind::Added,
                        line_number: 3,
                        content: "The kettle hums, the street is still,".to_string(),
                    },
                ],
            }],
        }
    }

    fn sample_modified_file_change() -> PatchFileChange {
        PatchFileChange {
            old_path: "poem.md".to_string(),
            new_path: "poem.md".to_string(),
            kind: FileChangeKind::Modified,
            hunks: vec![PatchHunk {
                old_start: 3,
                old_count: 7,
                new_start: 3,
                new_count: 7,
                lines: vec![
                    TouchedLine {
                        kind: TouchedLineKind::Removed,
                        line_number: 5,
                        content: "Small hours gather into name,".to_string(),
                    },
                    TouchedLine {
                        kind: TouchedLineKind::Added,
                        line_number: 5,
                        content: "Smqll hours gather into name,".to_string(),
                    },
                    TouchedLine {
                        kind: TouchedLineKind::Removed,
                        line_number: 8,
                        content: "But something kind is waiting there.".to_string(),
                    },
                    TouchedLine {
                        kind: TouchedLineKind::Added,
                        line_number: 8,
                        content: "But something kind is wqiting there.".to_string(),
                    },
                ],
            }],
        }
    }

    fn sample_multi_file_patch() -> ParsedPatch {
        ParsedPatch {
            files: vec![
                PatchFileChange {
                    old_path: "poem.md".to_string(),
                    new_path: "poem.md".to_string(),
                    kind: FileChangeKind::Modified,
                    hunks: vec![PatchHunk {
                        old_start: 1,
                        old_count: 4,
                        new_start: 1,
                        new_count: 6,
                        lines: vec![
                            TouchedLine {
                                kind: TouchedLineKind::Added,
                                line_number: 1,
                                content: "hello, nerds".to_string(),
                            },
                            TouchedLine {
                                kind: TouchedLineKind::Added,
                                line_number: 2,
                                content: String::new(),
                            },
                        ],
                    }],
                },
                PatchFileChange {
                    old_path: String::new(),
                    new_path: "poem-2.md".to_string(),
                    kind: FileChangeKind::Added,
                    hunks: vec![PatchHunk {
                        old_start: 0,
                        old_count: 0,
                        new_start: 1,
                        new_count: 9,
                        lines: vec![
                            TouchedLine {
                                kind: TouchedLineKind::Added,
                                line_number: 1,
                                content: "Evening settles in the hall,".to_string(),
                            },
                            TouchedLine {
                                kind: TouchedLineKind::Added,
                                line_number: 2,
                                content: "Shadows fold the edges near.".to_string(),
                            },
                        ],
                    }],
                },
            ],
        }
    }

    #[test]
    fn parsed_patch_roundtrip_json() {
        let patch = sample_multi_file_patch();
        let json = serde_json::to_string(&patch).expect("serialize to JSON");
        let deserialized: ParsedPatch = serde_json::from_str(&json).expect("deserialize from JSON");
        assert_eq!(patch, deserialized);
    }

    #[test]
    fn file_change_added_roundtrip() {
        let change = sample_added_file_change();
        let json = serde_json::to_string(&change).expect("serialize to JSON");
        let deserialized: PatchFileChange =
            serde_json::from_str(&json).expect("deserialize from JSON");
        assert_eq!(change, deserialized);
    }

    #[test]
    fn file_change_modified_roundtrip() {
        let change = sample_modified_file_change();
        let json = serde_json::to_string(&change).expect("serialize to JSON");
        let deserialized: PatchFileChange =
            serde_json::from_str(&json).expect("deserialize from JSON");
        assert_eq!(change, deserialized);
    }

    #[test]
    fn hunk_roundtrip() {
        let hunk = PatchHunk {
            old_start: 3,
            old_count: 7,
            new_start: 3,
            new_count: 7,
            lines: vec![
                TouchedLine {
                    kind: TouchedLineKind::Removed,
                    line_number: 5,
                    content: "Small hours gather into name,".to_string(),
                },
                TouchedLine {
                    kind: TouchedLineKind::Added,
                    line_number: 5,
                    content: "Smqll hours gather into name,".to_string(),
                },
            ],
        };
        let json = serde_json::to_string(&hunk).expect("serialize to JSON");
        let deserialized: PatchHunk = serde_json::from_str(&json).expect("deserialize from JSON");
        assert_eq!(hunk, deserialized);
    }

    #[test]
    fn touched_line_roundtrip() {
        let line = TouchedLine {
            kind: TouchedLineKind::Added,
            line_number: 1,
            content: "Morning leans on the windowsill,".to_string(),
        };
        let json = serde_json::to_string(&line).expect("serialize to JSON");
        let deserialized: TouchedLine = serde_json::from_str(&json).expect("deserialize from JSON");
        assert_eq!(line, deserialized);
    }

    #[test]
    fn file_change_kind_serde_variants() {
        for kind in [
            FileChangeKind::Added,
            FileChangeKind::Modified,
            FileChangeKind::Deleted,
            FileChangeKind::Renamed,
        ] {
            let json = serde_json::to_string(&kind).expect("serialize FileChangeKind");
            let deserialized: FileChangeKind =
                serde_json::from_str(&json).expect("deserialize FileChangeKind");
            assert_eq!(kind, deserialized, "round-trip failed for {kind:?}");
        }
    }

    #[test]
    fn touched_line_kind_serde_variants() {
        for kind in [TouchedLineKind::Added, TouchedLineKind::Removed] {
            let json = serde_json::to_string(&kind).expect("serialize TouchedLineKind");
            let deserialized: TouchedLineKind =
                serde_json::from_str(&json).expect("deserialize TouchedLineKind");
            assert_eq!(kind, deserialized, "round-trip failed for {kind:?}");
        }
    }

    #[test]
    fn empty_patch_roundtrip() {
        let patch = ParsedPatch { files: vec![] };
        let json = serde_json::to_string(&patch).expect("serialize empty patch");
        let deserialized: ParsedPatch =
            serde_json::from_str(&json).expect("deserialize empty patch");
        assert_eq!(patch, deserialized);
    }

    #[test]
    fn file_change_with_empty_hunks_roundtrip() {
        let change = PatchFileChange {
            old_path: "deleted.txt".to_string(),
            new_path: String::new(),
            kind: FileChangeKind::Deleted,
            hunks: vec![],
        };
        let json = serde_json::to_string(&change).expect("serialize");
        let deserialized: PatchFileChange = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(change, deserialized);
    }

    #[test]
    fn serde_json_field_names_are_snake_case() {
        let patch = ParsedPatch {
            files: vec![PatchFileChange {
                old_path: "a.txt".to_string(),
                new_path: "b.txt".to_string(),
                kind: FileChangeKind::Renamed,
                hunks: vec![PatchHunk {
                    old_start: 1,
                    old_count: 3,
                    new_start: 1,
                    new_count: 3,
                    lines: vec![TouchedLine {
                        kind: TouchedLineKind::Added,
                        line_number: 2,
                        content: "new line".to_string(),
                    }],
                }],
            }],
        };
        let json = serde_json::to_string_pretty(&patch).expect("serialize");
        assert!(
            json.contains("\"old_path\""),
            "expected snake_case field 'old_path' in JSON"
        );
        assert!(
            json.contains("\"new_path\""),
            "expected snake_case field 'new_path' in JSON"
        );
        assert!(
            json.contains("\"old_start\""),
            "expected snake_case field 'old_start' in JSON"
        );
        assert!(
            json.contains("\"line_number\""),
            "expected snake_case field 'line_number' in JSON"
        );
        assert!(
            json.contains("\"snake_case\""),
            "expected snake_case enum variant 'snake_case' in JSON for FileChangeKind::Renamed"
        );
    }

    #[test]
    fn parse_index_style_new_file_absolute_path() {
        // files/1/diff.1 — Index: with absolute path, new file
        let input = "\
Index: /home/davidabram/repos/shared-context-engineering/master/poem.md
===================================================================
--- /home/davidabram/repos/shared-context-engineering/master/poem.md
+++ /home/davidabram/repos/shared-context-engineering/master/poem.md
@@ -0,0 +1,9 @@
+Morning leans on the windowsill,
+Dust turns slow in borrowed gold.
+The kettle hums, the street is still,
+And day begins by growing bold.
+
+Small hours gather into name,
+Birdsong stitches light to air.
+Nothing ends the way it came,
+But something kind is waiting there.";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 1);
        let file = &patch.files[0];
        // The ---/+++ lines have absolute paths; the parser uses them as-is
        // (stripping only /dev/null and a/b/ prefixes).
        assert_eq!(file.kind, FileChangeKind::Added);
        assert_eq!(file.hunks.len(), 1);
        let hunk = &file.hunks[0];
        assert_eq!(hunk.old_start, 0);
        assert_eq!(hunk.old_count, 0);
        assert_eq!(hunk.new_start, 1);
        assert_eq!(hunk.new_count, 9);
        // All lines are added (no context lines in a new-file hunk)
        assert!(hunk.lines.iter().all(|l| l.kind == TouchedLineKind::Added));
        assert_eq!(hunk.lines.len(), 9);
        assert_eq!(hunk.lines[0].content, "Morning leans on the windowsill,");
        assert_eq!(hunk.lines[0].line_number, 1);
        // Line 5 is an empty line (the blank line in the poem)
        assert_eq!(hunk.lines[4].content, "");
        assert_eq!(hunk.lines[4].line_number, 5);
    }

    #[test]
    fn parse_index_style_new_file_relative_path() {
        // files/1/diff.2 — Index: with relative path, new file
        let input = "\
Index: poem.md
===================================================================
--- poem.md\t
+++ poem.md\t
@@ -0,0 +1,9 @@
+Morning leans on the windowsill,
+Dust turns slow in borrowed gold.
+The kettle hums, the street is still,
+And day begins by growing bold.
+
+Small hours gather into name,
+Birdsong stitches light to air.
+Nothing ends the way it came,
+But something kind is waiting there.";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 1);
        let file = &patch.files[0];
        assert_eq!(file.kind, FileChangeKind::Added);
        assert_eq!(file.hunks.len(), 1);
        assert_eq!(file.hunks[0].lines.len(), 9);
    }

    #[test]
    fn parse_git_style_new_file() {
        // files/1/diff.6 — git-style new file
        let input = "\
diff --git a/poem.md b/poem.md
new file mode 100644
index 0000000..b827922
--- /dev/null
+++ b/poem.md
@@ -0,0 +1,9 @@
+Morning leans on the windowsill,
+Dust turns slow in borrowed gold.
+The kettle hums, the street is still,
+And day begins by growing bold.
+
+Small hours gather into name,
+Birdsong stitches light to air.
+Nothing ends the way it came,
+But something kind is waiting there.";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 1);
        let file = &patch.files[0];
        assert_eq!(file.old_path, "/dev/null");
        assert_eq!(file.new_path, "poem.md");
        assert_eq!(file.kind, FileChangeKind::Added);
        assert_eq!(file.hunks.len(), 1);
        assert_eq!(file.hunks[0].lines.len(), 9);
        assert_eq!(file.hunks[0].old_start, 0);
        assert_eq!(file.hunks[0].old_count, 0);
        assert_eq!(file.hunks[0].new_start, 1);
        assert_eq!(file.hunks[0].new_count, 9);
    }

    #[test]
    fn parse_index_style_modified_file_with_removed_lines() {
        // files/2/diff.1 — Index: with absolute path, modified file
        let input = "\
Index: /home/davidabram/repos/shared-context-engineering/master/poem.md
===================================================================
--- /home/davidabram/repos/shared-context-engineering/master/poem.md
+++ /home/davidabram/repos/shared-context-engineering/master/poem.md
@@ -2,8 +2,8 @@
 Dust turns slow in borrowed gold.
 The kettle hums, the street is still,
 And day begins by growing bold.

-Small hours gather into name,
+Smqll hours gather into name,
 Birdsong stitches light to air.
 Nothing ends the way it came,
-But something kind is waiting there.
+But something kind is wqiting there.";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 1);
        let file = &patch.files[0];
        assert_eq!(file.kind, FileChangeKind::Modified);
        assert_eq!(file.hunks.len(), 1);
        let hunk = &file.hunks[0];
        assert_eq!(hunk.old_start, 2);
        assert_eq!(hunk.old_count, 8);
        assert_eq!(hunk.new_start, 2);
        assert_eq!(hunk.new_count, 8);
        // Only touched lines: 2 removed + 2 added = 4
        assert_eq!(hunk.lines.len(), 4);
        // First removed line
        assert_eq!(hunk.lines[0].kind, TouchedLineKind::Removed);
        assert_eq!(hunk.lines[0].line_number, 5);
        assert_eq!(hunk.lines[0].content, "Small hours gather into name,");
        // First added line
        assert_eq!(hunk.lines[1].kind, TouchedLineKind::Added);
        assert_eq!(hunk.lines[1].line_number, 5);
        assert_eq!(hunk.lines[1].content, "Smqll hours gather into name,");
        // Second removed line
        assert_eq!(hunk.lines[2].kind, TouchedLineKind::Removed);
        assert_eq!(hunk.lines[2].line_number, 8);
        assert_eq!(
            hunk.lines[2].content,
            "But something kind is waiting there."
        );
        // Second added line
        assert_eq!(hunk.lines[3].kind, TouchedLineKind::Added);
        assert_eq!(hunk.lines[3].line_number, 8);
        assert_eq!(
            hunk.lines[3].content,
            "But something kind is wqiting there."
        );
    }

    #[test]
    fn parse_git_style_modified_file() {
        // files/2/diff.6 — git-style modified file
        let input = "\
diff --git a/poem.md b/poem.md
index b827922..71b993e 100644
--- a/poem.md
+++ b/poem.md
@@ -3,7 +3,7 @@ Dust turns slow in borrowed gold.
 The kettle hums, the street is still,
 And day begins by growing bold.

-Small hours gather into name,
+Smqll hours gather into name,
 Birdsong stitches light to air.
 Nothing ends the way it came,
-But something kind is waiting there.
+But something kind is wqiting there.";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 1);
        let file = &patch.files[0];
        assert_eq!(file.old_path, "poem.md");
        assert_eq!(file.new_path, "poem.md");
        assert_eq!(file.kind, FileChangeKind::Modified);
        assert_eq!(file.hunks.len(), 1);
        let hunk = &file.hunks[0];
        assert_eq!(hunk.old_start, 3);
        assert_eq!(hunk.old_count, 7);
        assert_eq!(hunk.new_start, 3);
        assert_eq!(hunk.new_count, 7);
        assert_eq!(hunk.lines.len(), 4);
        assert_eq!(hunk.lines[0].kind, TouchedLineKind::Removed);
        assert_eq!(hunk.lines[0].content, "Small hours gather into name,");
        assert_eq!(hunk.lines[1].kind, TouchedLineKind::Added);
        assert_eq!(hunk.lines[1].content, "Smqll hours gather into name,");
    }

    #[test]
    fn parse_index_style_modified_with_full_context() {
        // files/2/diff.2 — Index: with relative path, modified file with full context
        let input = "\
Index: poem.md
===================================================================
--- poem.md\t
+++ poem.md\t
@@ -1,9 +1,9 @@
 Morning leans on the windowsill,
 Dust turns slow in borrowed gold.
 The kettle hums, the street is still,
 And day begins by growing bold.

-Small hours gather into name,
+Smqll hours gather into name,
 Birdsong stitches light to air.
 Nothing ends the way it came,
-But something kind is waiting there.
+But something kind is wqiting there.";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 1);
        let file = &patch.files[0];
        assert_eq!(file.kind, FileChangeKind::Modified);
        assert_eq!(file.hunks.len(), 1);
        let hunk = &file.hunks[0];
        assert_eq!(hunk.old_start, 1);
        assert_eq!(hunk.old_count, 9);
        assert_eq!(hunk.new_start, 1);
        assert_eq!(hunk.new_count, 9);
        // Context lines are excluded, only 4 touched lines
        assert_eq!(hunk.lines.len(), 4);
    }

    #[test]
    fn parse_multi_file_index_style() {
        // files/3/diff.1 — multi-file Index: patch
        let input = "\
Index: /home/davidabram/repos/shared-context-engineering/master/poem.md
===================================================================
--- /home/davidabram/repos/shared-context-engineering/master/poem.md
+++ /home/davidabram/repos/shared-context-engineering/master/poem.md
@@ -1,4 +1,6 @@
+hello, nerds
+
 Morning leans on the windowsill,
 Dust turns slow in borrowed gold.
 The kettle hums, the street is still,
 And day begins by growing bold.

Index: /home/davidabram/repos/shared-context-engineering/master/poem-2.md
===================================================================
--- /home/davidabram/repos/shared-context-engineering/master/poem-2.md
+++ /home/davidabram/repos/shared-context-engineering/master/poem-2.md
@@ -0,0 +1,9 @@
+Evening settles in the hall,
+Shadows fold the edges near.
+Radiators click behind the wall,
+Like quiet thoughts we almost hear.
+
+Soft lamps gather amber light,
+Pages rest in patient hands.
+Night arrives without a fight,
+And leaves its hush across the lands.";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 2);

        // First file: modified poem.md with added lines
        let file1 = &patch.files[0];
        assert_eq!(file1.kind, FileChangeKind::Modified);
        assert_eq!(file1.hunks.len(), 1);
        let hunk1 = &file1.hunks[0];
        assert_eq!(hunk1.old_start, 1);
        assert_eq!(hunk1.old_count, 4);
        assert_eq!(hunk1.new_start, 1);
        assert_eq!(hunk1.new_count, 6);
        // Only the 2 added lines, context lines excluded
        assert_eq!(hunk1.lines.len(), 2);
        assert_eq!(hunk1.lines[0].kind, TouchedLineKind::Added);
        assert_eq!(hunk1.lines[0].content, "hello, nerds");
        assert_eq!(hunk1.lines[0].line_number, 1);
        assert_eq!(hunk1.lines[1].kind, TouchedLineKind::Added);
        assert_eq!(hunk1.lines[1].content, "");
        assert_eq!(hunk1.lines[1].line_number, 2);

        // Second file: new poem-2.md
        let file2 = &patch.files[1];
        assert_eq!(file2.kind, FileChangeKind::Added);
        assert_eq!(file2.hunks.len(), 1);
        let hunk2 = &file2.hunks[0];
        assert_eq!(hunk2.old_start, 0);
        assert_eq!(hunk2.old_count, 0);
        assert_eq!(hunk2.new_start, 1);
        assert_eq!(hunk2.new_count, 9);
        assert_eq!(hunk2.lines.len(), 9);
        assert_eq!(hunk2.lines[0].content, "Evening settles in the hall,");
    }

    #[test]
    fn parse_index_style_new_file_relative() {
        // files/3/diff.4 — Index: with relative path, new file
        let input = "\
Index: poem-2.md
===================================================================
--- poem-2.md\t
+++ poem-2.md\t
@@ -0,0 +1,9 @@
+Evening settles in the hall,
+Shadows fold the edges near.
+Radiators click behind the wall,
+Like quiet thoughts we almost hear.
+
+Soft lamps gather amber light,
+Pages rest in patient hands.
+Night arrives without a fight,
+And leaves its hush across the lands.";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 1);
        let file = &patch.files[0];
        assert_eq!(file.kind, FileChangeKind::Added);
        assert_eq!(file.hunks.len(), 1);
        assert_eq!(file.hunks[0].lines.len(), 9);
    }

    #[test]
    fn parse_index_style_modified_with_added_lines() {
        // files/3/diff.5 — Index: with relative path, modified file with added lines
        let input = "\
Index: poem.md
===================================================================
--- poem.md\t
+++ poem.md\t
@@ -1,9 +1,11 @@
+hello, nerds
+
 Morning leans on the windowsill,
 Dust turns slow in borrowed gold.
 The kettle hums, the street is still,
 And day begins by growing bold.

 Smqll hours gather into name,
 Birdsong stitches light to air.
 Nothing ends the way it came,
 But something kind is wqiting there.";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 1);
        let file = &patch.files[0];
        assert_eq!(file.kind, FileChangeKind::Modified);
        assert_eq!(file.hunks.len(), 1);
        let hunk = &file.hunks[0];
        assert_eq!(hunk.old_start, 1);
        assert_eq!(hunk.old_count, 9);
        assert_eq!(hunk.new_start, 1);
        assert_eq!(hunk.new_count, 11);
        // Only the 2 added lines at the top; context lines excluded
        assert_eq!(hunk.lines.len(), 2);
        assert_eq!(hunk.lines[0].kind, TouchedLineKind::Added);
        assert_eq!(hunk.lines[0].content, "hello, nerds");
        assert_eq!(hunk.lines[1].kind, TouchedLineKind::Added);
        assert_eq!(hunk.lines[1].content, "");
    }

    #[test]
    fn parse_empty_input() {
        let patch = parse_patch("").expect("empty input should parse");
        assert!(patch.files.is_empty());
    }

    #[test]
    fn parse_error_on_hunk_without_file_header() {
        let input = "@@ -1,3 +1,3 @@\n-old line\n+new line\n context\n";
        let result = parse_patch(input);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().message.contains("'---'"),
            "error should mention missing --- header"
        );
    }

    #[test]
    fn parse_error_on_invalid_hunk_header() {
        let input = "\
Index: test.txt
===================================================================
--- test.txt
+++ test.txt
@@ invalid @@";
        let result = parse_patch(input);
        assert!(result.is_err());
    }

    #[test]
    fn parse_error_on_missing_closing_at_at() {
        let input = "\
Index: test.txt
===================================================================
--- test.txt
+++ test.txt
@@ -1,3 +1,3";
        let result = parse_patch(input);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().message.contains("@@"),
            "error should mention missing closing @@"
        );
    }

    #[test]
    fn parse_git_style_dev_null_old_path() {
        // Verify /dev/null in --- line produces empty old_path and Added kind
        let input = "\
diff --git a/newfile.txt b/newfile.txt
new file mode 100644
index 0000000..abc1234
--- /dev/null
+++ b/newfile.txt
@@ -0,0 +1,3 @@
+line one
+line two
+line three";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 1);
        let file = &patch.files[0];
        assert_eq!(file.old_path, "/dev/null");
        assert_eq!(file.new_path, "newfile.txt");
        assert_eq!(file.kind, FileChangeKind::Added);
        assert_eq!(file.hunks[0].lines.len(), 3);
    }

    #[test]
    fn parse_git_style_deleted_file() {
        // Verify /dev/null in +++ line produces empty new_path and Deleted kind
        let input = "\
diff --git a/oldfile.txt b/oldfile.txt
deleted file mode 100644
index abc1234..0000000
--- a/oldfile.txt
+++ /dev/null
@@ -1,3 +0,0 @@
-line one
-line two
-line three";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 1);
        let file = &patch.files[0];
        assert_eq!(file.old_path, "oldfile.txt");
        assert_eq!(file.new_path, "/dev/null");
        assert_eq!(file.kind, FileChangeKind::Deleted);
        assert_eq!(file.hunks[0].lines.len(), 3);
        assert!(file.hunks[0]
            .lines
            .iter()
            .all(|l| l.kind == TouchedLineKind::Removed));
    }

    #[test]
    fn parse_no_newline_at_end_of_file() {
        // Verify "\ No newline at end of file" is skipped
        let input = "\
Index: test.txt
===================================================================
--- test.txt
+++ test.txt
@@ -1 +1 @@
-old line
\\ No newline at end of file
+new line
\\ No newline at end of file";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 1);
        let hunk = &patch.files[0].hunks[0];
        assert_eq!(hunk.lines.len(), 2);
        assert_eq!(hunk.lines[0].kind, TouchedLineKind::Removed);
        assert_eq!(hunk.lines[0].content, "old line");
        assert_eq!(hunk.lines[1].kind, TouchedLineKind::Added);
        assert_eq!(hunk.lines[1].content, "new line");
    }

    #[test]
    fn parse_multiple_hunks_in_single_file() {
        let input = "\
Index: test.txt
===================================================================
--- test.txt
+++ test.txt
@@ -1,3 +1,3 @@
 context1
-old line 1
+new line 1
 context2
@@ -10,3 +10,3 @@
 context3
-old line 2
+new line 2
 context4";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 1);
        let file = &patch.files[0];
        assert_eq!(file.hunks.len(), 2);
        assert_eq!(file.hunks[0].lines.len(), 2);
        assert_eq!(file.hunks[1].lines.len(), 2);
        assert_eq!(file.hunks[0].old_start, 1);
        assert_eq!(file.hunks[1].old_start, 10);
    }

    #[test]
    fn parse_line_number_tracking_for_removed_and_added() {
        // Verify line numbers track correctly through mixed context/removed/added
        let input = "\
Index: test.txt
===================================================================
--- test.txt
+++ test.txt
@@ -3,7 +3,7 @@
 line3
-line4_old
+line4_new
 line5
 line6
-line7_old
+line7_new
 line8";

        let patch = parse_patch(input).expect("parse should succeed");
        let hunk = &patch.files[0].hunks[0];
        assert_eq!(hunk.lines.len(), 4);
        // Removed line at old line 4
        assert_eq!(hunk.lines[0].kind, TouchedLineKind::Removed);
        assert_eq!(hunk.lines[0].line_number, 4);
        // Added line at new line 4
        assert_eq!(hunk.lines[1].kind, TouchedLineKind::Added);
        assert_eq!(hunk.lines[1].line_number, 4);
        // Removed line at old line 7
        assert_eq!(hunk.lines[2].kind, TouchedLineKind::Removed);
        assert_eq!(hunk.lines[2].line_number, 7);
        // Added line at new line 7
        assert_eq!(hunk.lines[3].kind, TouchedLineKind::Added);
        assert_eq!(hunk.lines[3].line_number, 7);
    }

    #[test]
    fn parse_hunk_header_without_count_defaults_to_one() {
        // @@ -1 +1 @@ means count of 1 for both sides
        let input = "\
Index: test.txt
===================================================================
--- test.txt
+++ test.txt
@@ -1 +1 @@
-old line
+new line";

        let patch = parse_patch(input).expect("parse should succeed");
        let hunk = &patch.files[0].hunks[0];
        assert_eq!(hunk.old_start, 1);
        assert_eq!(hunk.old_count, 1);
        assert_eq!(hunk.new_start, 1);
        assert_eq!(hunk.new_count, 1);
    }

    #[test]
    fn parse_git_style_renamed_file() {
        let input = "\
diff --git a/old_name.txt b/new_name.txt
similarity index 80%
rename from old_name.txt
rename to new_name.txt
--- a/old_name.txt
+++ b/new_name.txt
@@ -1,3 +1,3 @@
 line1
-old line
+new line
 line3";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 1);
        let file = &patch.files[0];
        assert_eq!(file.old_path, "old_name.txt");
        assert_eq!(file.new_path, "new_name.txt");
        assert_eq!(file.kind, FileChangeKind::Renamed);
        assert_eq!(file.hunks.len(), 1);
    }

    // --- T03: Multi-file and deletion-oriented coverage tests ---

    #[test]
    fn parse_git_style_multi_file_patch() {
        // Git-style patch with two files: one modified, one new
        let input = "\
diff --git a/readme.md b/readme.md
index abc1234..def5678 100644
--- a/readme.md
+++ b/readme.md
@@ -1,3 +1,3 @@
 line1
-old line
+new line
 line3
diff --git a/newfile.txt b/newfile.txt
new file mode 100644
index 0000000..1234567
--- /dev/null
+++ b/newfile.txt
@@ -0,0 +1,2 @@
+first line
+second line";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 2);

        // First file: modified readme.md
        let file1 = &patch.files[0];
        assert_eq!(file1.old_path, "readme.md");
        assert_eq!(file1.new_path, "readme.md");
        assert_eq!(file1.kind, FileChangeKind::Modified);
        assert_eq!(file1.hunks.len(), 1);
        assert_eq!(file1.hunks[0].lines.len(), 2);
        assert_eq!(file1.hunks[0].lines[0].kind, TouchedLineKind::Removed);
        assert_eq!(file1.hunks[0].lines[0].content, "old line");
        assert_eq!(file1.hunks[0].lines[1].kind, TouchedLineKind::Added);
        assert_eq!(file1.hunks[0].lines[1].content, "new line");

        // Second file: new file
        let file2 = &patch.files[1];
        assert_eq!(file2.old_path, "/dev/null");
        assert_eq!(file2.new_path, "newfile.txt");
        assert_eq!(file2.kind, FileChangeKind::Added);
        assert_eq!(file2.hunks.len(), 1);
        assert_eq!(file2.hunks[0].lines.len(), 2);
        assert_eq!(file2.hunks[0].lines[0].kind, TouchedLineKind::Added);
        assert_eq!(file2.hunks[0].lines[0].content, "first line");
        assert_eq!(file2.hunks[0].lines[1].kind, TouchedLineKind::Added);
        assert_eq!(file2.hunks[0].lines[1].content, "second line");
    }

    #[test]
    fn parse_index_style_deleted_file() {
        // Index-style patch where a file is entirely deleted
        let input = "\
Index: obsolete.txt
===================================================================
--- obsolete.txt
+++ /dev/null\t
@@ -1,4 +0,0 @@
-line one
-line two
-line three
-line four";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 1);
        let file = &patch.files[0];
        assert_eq!(file.old_path, "obsolete.txt");
        assert_eq!(file.new_path, ""); // /dev/null -> empty string
        assert_eq!(file.kind, FileChangeKind::Deleted);
        assert_eq!(file.hunks.len(), 1);
        let hunk = &file.hunks[0];
        assert_eq!(hunk.old_start, 1);
        assert_eq!(hunk.old_count, 4);
        assert_eq!(hunk.new_start, 0);
        assert_eq!(hunk.new_count, 0);
        // All lines are removed, none added
        assert_eq!(hunk.lines.len(), 4);
        assert!(hunk
            .lines
            .iter()
            .all(|l| l.kind == TouchedLineKind::Removed));
        assert_eq!(hunk.lines[0].content, "line one");
        assert_eq!(hunk.lines[0].line_number, 1);
        assert_eq!(hunk.lines[3].content, "line four");
        assert_eq!(hunk.lines[3].line_number, 4);
    }

    #[test]
    fn parse_multi_file_with_deleted_file() {
        // Multi-file Index-style patch including a deleted file
        let input = "\
Index: poem.md
===================================================================
--- poem.md\t
+++ poem.md\t
@@ -1,4 +1,6 @@
+hello, nerds
+
 Morning leans on the windowsill,
 Dust turns slow in borrowed gold.,
 The kettle hums, the street is still,
 And day begins by growing bold.

Index: obsolete.txt
===================================================================
--- obsolete.txt
+++ /dev/null\t
@@ -1,3 +0,0 @@
-line one
-line two
-line three";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 2);

        // First file: modified poem.md with added lines
        let file1 = &patch.files[0];
        assert_eq!(file1.kind, FileChangeKind::Modified);
        assert_eq!(file1.hunks.len(), 1);
        assert_eq!(file1.hunks[0].lines.len(), 2);
        assert_eq!(file1.hunks[0].lines[0].kind, TouchedLineKind::Added);
        assert_eq!(file1.hunks[0].lines[0].content, "hello, nerds");

        // Second file: deleted file
        let file2 = &patch.files[1];
        assert_eq!(file2.old_path, "obsolete.txt");
        assert_eq!(file2.new_path, "");
        assert_eq!(file2.kind, FileChangeKind::Deleted);
        assert_eq!(file2.hunks.len(), 1);
        assert_eq!(file2.hunks[0].lines.len(), 3);
        assert!(file2.hunks[0]
            .lines
            .iter()
            .all(|l| l.kind == TouchedLineKind::Removed));
    }

    #[test]
    fn parse_hunk_with_only_removed_lines() {
        // Index-style patch where a hunk has only removed lines, no additions
        let input = "\
Index: config.txt
===================================================================
--- config.txt\t
+++ config.txt\t
@@ -2,4 +2,2 @@
 context line
-removed line one
-removed line two
 context line";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 1);
        let file = &patch.files[0];
        assert_eq!(file.kind, FileChangeKind::Modified);
        assert_eq!(file.hunks.len(), 1);
        let hunk = &file.hunks[0];
        assert_eq!(hunk.old_start, 2);
        assert_eq!(hunk.old_count, 4);
        assert_eq!(hunk.new_start, 2);
        assert_eq!(hunk.new_count, 2);
        // Only removed lines, no added lines
        assert_eq!(hunk.lines.len(), 2);
        assert_eq!(hunk.lines[0].kind, TouchedLineKind::Removed);
        assert_eq!(hunk.lines[0].content, "removed line one");
        assert_eq!(hunk.lines[0].line_number, 3);
        assert_eq!(hunk.lines[1].kind, TouchedLineKind::Removed);
        assert_eq!(hunk.lines[1].content, "removed line two");
        assert_eq!(hunk.lines[1].line_number, 4);
    }

    #[test]
    fn parse_git_style_multi_hunk_multi_file() {
        // Git-style patch with multiple hunks across two files
        let input = "\
diff --git a/file1.txt b/file1.txt
index abc1234..def5678 100644
--- a/file1.txt
+++ b/file1.txt
@@ -1,3 +1,3 @@
 line1
-old line 1
+new line 1
 line3
@@ -10,3 +10,3 @@
 line10
-old line 10
+new line 10
 line12
diff --git a/file2.txt b/file2.txt
new file mode 100644
index 0000000..1234567
--- /dev/null
+++ b/file2.txt
@@ -0,0 +1,3 @@
+alpha
+beta
+gamma";

        let patch = parse_patch(input).expect("parse should succeed");
        assert_eq!(patch.files.len(), 2);

        // First file: modified with two hunks
        let file1 = &patch.files[0];
        assert_eq!(file1.old_path, "file1.txt");
        assert_eq!(file1.new_path, "file1.txt");
        assert_eq!(file1.kind, FileChangeKind::Modified);
        assert_eq!(file1.hunks.len(), 2);

        let hunk1 = &file1.hunks[0];
        assert_eq!(hunk1.old_start, 1);
        assert_eq!(hunk1.old_count, 3);
        assert_eq!(hunk1.new_start, 1);
        assert_eq!(hunk1.new_count, 3);
        assert_eq!(hunk1.lines.len(), 2);
        assert_eq!(hunk1.lines[0].kind, TouchedLineKind::Removed);
        assert_eq!(hunk1.lines[0].content, "old line 1");
        assert_eq!(hunk1.lines[1].kind, TouchedLineKind::Added);
        assert_eq!(hunk1.lines[1].content, "new line 1");

        let hunk2 = &file1.hunks[1];
        assert_eq!(hunk2.old_start, 10);
        assert_eq!(hunk2.lines.len(), 2);
        assert_eq!(hunk2.lines[0].kind, TouchedLineKind::Removed);
        assert_eq!(hunk2.lines[0].content, "old line 10");
        assert_eq!(hunk2.lines[1].kind, TouchedLineKind::Added);
        assert_eq!(hunk2.lines[1].content, "new line 10");

        // Second file: new file with one hunk
        let file2 = &patch.files[1];
        assert_eq!(file2.kind, FileChangeKind::Added);
        assert_eq!(file2.hunks.len(), 1);
        assert_eq!(file2.hunks[0].lines.len(), 3);
        assert!(file2.hunks[0]
            .lines
            .iter()
            .all(|l| l.kind == TouchedLineKind::Added));
    }

    #[test]
    fn parse_roundtrip_after_parse() {
        // Parse a git-style patch, serialize to JSON, deserialize, and verify equality
        let input = "\
diff --git a/poem.md b/poem.md
index b827922..71b993e 100644
--- a/poem.md
+++ b/poem.md
@@ -3,7 +3,7 @@ Dust turns slow in borrowed gold.
 The kettle hums, the street is still,
 And day begins by growing bold.

-Small hours gather into name,
+Smqll hours gather into name,
 Birdsong stitches light to air.
 Nothing ends the way it came,
-But something kind is waiting there.
+But something kind is wqiting there.";

        let patch = parse_patch(input).expect("parse should succeed");
        let json = serde_json::to_string_pretty(&patch).expect("serialize");
        let roundtripped: ParsedPatch = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(patch, roundtripped);
    }

    // --- T01: JSON load helper tests ---

    #[test]
    fn json_load_roundtrip_from_string() {
        let patch = sample_multi_file_patch();
        let json = serde_json::to_string(&patch).expect("serialize");
        let loaded = load_patch_from_json(&json).expect("load from JSON string");
        assert_eq!(patch, loaded);
    }

    #[test]
    fn json_load_roundtrip_from_bytes() {
        let patch = sample_multi_file_patch();
        let json_bytes = serde_json::to_vec(&patch).expect("serialize to bytes");
        let loaded = load_patch_from_json_bytes(&json_bytes).expect("load from JSON bytes");
        assert_eq!(patch, loaded);
    }

    #[test]
    fn json_load_empty_patch() {
        let patch = ParsedPatch { files: vec![] };
        let json = serde_json::to_string(&patch).expect("serialize empty patch");
        let loaded = load_patch_from_json(&json).expect("load empty patch");
        assert!(loaded.files.is_empty());
    }

    #[test]
    fn json_load_single_file_patch() {
        let patch = ParsedPatch {
            files: vec![sample_added_file_change()],
        };
        let json = serde_json::to_string(&patch).expect("serialize");
        let loaded = load_patch_from_json(&json).expect("load");
        assert_eq!(patch, loaded);
    }

    #[test]
    fn json_load_error_on_invalid_json_syntax() {
        let result = load_patch_from_json("not json at all");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("invalid patch JSON"),
            "error message should contain 'invalid patch JSON', got: {}",
            err.message
        );
    }

    #[test]
    fn json_load_error_on_valid_json_but_wrong_structure() {
        // Valid JSON but not a ParsedPatch shape
        let result = load_patch_from_json("{\"not\": \"a patch\"}");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("invalid patch JSON"),
            "error message should contain 'invalid patch JSON', got: {}",
            err.message
        );
    }

    #[test]
    fn json_load_error_on_missing_files_field() {
        // Valid JSON object but missing the required "files" field
        let result = load_patch_from_json("{}");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("invalid patch JSON"),
            "error message should contain 'invalid patch JSON', got: {}",
            err.message
        );
    }

    #[test]
    fn json_load_bytes_error_on_invalid_utf8_json() {
        // Bytes that are not valid UTF-8 and not valid JSON
        let result = load_patch_from_json_bytes(&[0xff, 0xfe, 0x00]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("invalid patch JSON"),
            "error message should contain 'invalid patch JSON', got: {}",
            err.message
        );
    }

    #[test]
    fn json_load_bytes_error_on_wrong_structure() {
        // Valid JSON bytes but wrong structure
        let json_bytes = b"{\"not\": \"a patch\"}";
        let result = load_patch_from_json_bytes(json_bytes);
        assert!(result.is_err());
    }

    #[test]
    fn json_load_preserves_all_field_change_kinds() {
        for kind in [
            FileChangeKind::Added,
            FileChangeKind::Modified,
            FileChangeKind::Deleted,
            FileChangeKind::Renamed,
        ] {
            let patch = ParsedPatch {
                files: vec![PatchFileChange {
                    old_path: "a.txt".to_string(),
                    new_path: "b.txt".to_string(),
                    kind,
                    hunks: vec![],
                }],
            };
            let json = serde_json::to_string(&patch).expect("serialize");
            let loaded = load_patch_from_json(&json).expect("load");
            assert_eq!(patch, loaded, "round-trip failed for {kind:?}");
        }
    }

    #[test]
    fn json_load_preserves_touched_line_kinds() {
        for line_kind in [TouchedLineKind::Added, TouchedLineKind::Removed] {
            let patch = ParsedPatch {
                files: vec![PatchFileChange {
                    old_path: "test.txt".to_string(),
                    new_path: "test.txt".to_string(),
                    kind: FileChangeKind::Modified,
                    hunks: vec![PatchHunk {
                        old_start: 1,
                        old_count: 1,
                        new_start: 1,
                        new_count: 1,
                        lines: vec![TouchedLine {
                            kind: line_kind,
                            line_number: 1,
                            content: "test content".to_string(),
                        }],
                    }],
                }],
            };
            let json = serde_json::to_string(&patch).expect("serialize");
            let loaded = load_patch_from_json(&json).expect("load");
            assert_eq!(patch, loaded, "round-trip failed for {line_kind:?}");
        }
    }

    #[test]
    fn json_load_after_parse_roundtrip() {
        // End-to-end: parse raw diff text, serialize to JSON, load back via helper
        let input = "\
diff --git a/readme.md b/readme.md
index abc1234..def5678 100644
--- a/readme.md
+++ b/readme.md
@@ -1,3 +1,3 @@
 line1
-old line
+new line
 line3";
        let patch = parse_patch(input).expect("parse should succeed");
        let json = serde_json::to_string(&patch).expect("serialize");
        let loaded = load_patch_from_json(&json).expect("load from JSON");
        assert_eq!(patch, loaded);
    }
}
