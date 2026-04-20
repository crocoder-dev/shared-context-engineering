//! Patch domain model for in-memory parsed patch representation.
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

use serde::{Deserialize, Serialize};

/// Top-level parsed patch containing one or more file changes.
#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ParsedPatch {
    pub files: Vec<PatchFileChange>,
}

/// A single file's changes within a patch.
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TouchedLineKind {
    /// Line was added.
    Added,
    /// Line was removed.
    Removed,
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
        // Verify snake_case field names appear in the JSON output
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
}
