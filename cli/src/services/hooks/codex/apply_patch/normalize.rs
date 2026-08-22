//! Normalizes a parsed Codex `apply_patch` payload ([`CodexPatch`]) into SCE
//! `Index:`-form unified-diff text that `crate::services::patch::parse_patch`
//! already accepts.
//!
//! Positions are deterministic and patch-local: each `Update File` operation
//! numbers only the touched (`+`/`-`) lines it actually emits, starting from
//! line 1, ignoring Codex's own unchanged context lines entirely (they are
//! dropped, not persisted as evidence, and contribute no positional weight).
//! These positions are never claimed to be real filesystem line numbers. The
//! existing, unmodified `intersect_patches`
//! historical `kind`+`content` fallback is what lets this synthetic-line
//! evidence still attribute correctly once a real commit lands at different
//! real line numbers (see plan `context/plans/codex-cli-integration.md`
//! T11/AC15) — this module does not touch that fallback.
//!
//! `Delete File` operations, and `Update File` + `Move to` operations with no
//! changed lines, contribute no evidence and are silently dropped: an
//! `apply_patch` producing no provable evidence normalizes to an empty
//! string.

use std::fmt::Write as _;

use super::{CodexFileOperation, CodexHunk, CodexHunkLine, CodexPatch};

const PATCH_INDEX_SEPARATOR: &str =
    "===================================================================";

/// Normalizes every `Add`/`Update` file operation in `patch` into one
/// combined SCE `Index:`-form unified-diff string, in operation order.
#[allow(dead_code)]
pub(crate) fn normalize_codex_patch(patch: &CodexPatch) -> String {
    patch
        .operations
        .iter()
        .filter_map(normalize_operation)
        .collect()
}

fn normalize_operation(operation: &CodexFileOperation) -> Option<String> {
    match operation {
        CodexFileOperation::Add { path, lines } => Some(normalize_add(path, lines)),
        CodexFileOperation::Update {
            old_path,
            new_path,
            hunks,
        } => normalize_update(old_path, new_path.as_deref(), hunks),
        CodexFileOperation::Delete { .. } => None,
    }
}

fn normalize_add(path: &str, lines: &[String]) -> String {
    let mut body = format!("@@ -0,0 +1,{} @@\n", lines.len());
    for line in lines {
        body.push('+');
        body.push_str(line);
        body.push('\n');
    }
    render_file_section(path, path, &body)
}

fn normalize_update(old_path: &str, new_path: Option<&str>, hunks: &[CodexHunk]) -> Option<String> {
    let mut body = String::new();
    let mut old_pos: u64 = 1;
    let mut new_pos: u64 = 1;
    let mut has_changes = false;

    for hunk in hunks {
        let hunk_old_start = old_pos;
        let hunk_new_start = new_pos;
        let mut hunk_body = String::new();
        let mut removed_count: u64 = 0;
        let mut added_count: u64 = 0;

        for line in &hunk.lines {
            match line {
                // Codex's unchanged context is dropped, not persisted as
                // evidence, and does not affect synthetic positions.
                CodexHunkLine::Context(_) => {}
                CodexHunkLine::Removed(content) => {
                    hunk_body.push('-');
                    hunk_body.push_str(content);
                    hunk_body.push('\n');
                    old_pos += 1;
                    removed_count += 1;
                }
                CodexHunkLine::Added(content) => {
                    hunk_body.push('+');
                    hunk_body.push_str(content);
                    hunk_body.push('\n');
                    new_pos += 1;
                    added_count += 1;
                }
            }
        }

        if removed_count > 0 || added_count > 0 {
            let _ = writeln!(
                body,
                "@@ -{hunk_old_start},{removed_count} +{hunk_new_start},{added_count} @@"
            );
            body.push_str(&hunk_body);
            has_changes = true;
        }
    }

    if !has_changes {
        return None;
    }

    let destination = new_path.unwrap_or(old_path);
    Some(render_file_section(old_path, destination, &body))
}

fn render_file_section(old_path: &str, new_path: &str, body: &str) -> String {
    format!("Index: {new_path}\n{PATCH_INDEX_SEPARATOR}\n--- {old_path}\n+++ {new_path}\n{body}")
}

#[cfg(test)]
mod tests {
    use super::super::parser::parse_codex_apply_patch;
    use super::*;
    use crate::services::patch::{
        intersect_patches, parse_patch, FileChangeKind, ParsedPatch, PatchFileChange, PatchHunk,
        TouchedLine, TouchedLineKind,
    };

    fn parse(raw: &str) -> CodexPatch {
        parse_codex_apply_patch(raw).expect("fixture patch should parse")
    }

    #[test]
    fn normalizes_add_file_into_a_parseable_added_hunk() {
        let patch = parse(
            "*** Begin Patch\n\
             *** Add File: foo.txt\n\
             +line one\n\
             +line two\n\
             *** End Patch",
        );

        let normalized = normalize_codex_patch(&patch);

        assert_eq!(
            normalized,
            "Index: foo.txt\n\
             ===================================================================\n\
             --- foo.txt\n\
             +++ foo.txt\n\
             @@ -0,0 +1,2 @@\n\
             +line one\n\
             +line two\n"
        );

        let parsed = parse_patch(&normalized, Some("cx_test")).expect("should parse");
        assert_eq!(parsed.files.len(), 1);
        let file = &parsed.files[0];
        assert_eq!(file.kind, FileChangeKind::Added);
        assert_eq!(file.hunks.len(), 1);
        assert_eq!(
            file.hunks[0].lines,
            vec![
                TouchedLine {
                    kind: TouchedLineKind::Added,
                    line_number: 1,
                    content: "line one".to_string(),
                    session_id: Some("cx_test".to_string()),
                },
                TouchedLine {
                    kind: TouchedLineKind::Added,
                    line_number: 2,
                    content: "line two".to_string(),
                    session_id: Some("cx_test".to_string()),
                },
            ]
        );
    }

    #[test]
    fn normalizes_update_file_dropping_context_lines() {
        let patch = parse(
            "*** Begin Patch\n\
             *** Update File: src/lib.rs\n\
             @@ fn main() {\n\
             \x20   unchanged\n\
             -    old_line\n\
             +    new_line\n\
             *** End Patch",
        );

        let normalized = normalize_codex_patch(&patch);

        // The context line ("    unchanged") is dropped entirely and
        // contributes no positional weight.
        assert_eq!(
            normalized,
            "Index: src/lib.rs\n\
             ===================================================================\n\
             --- src/lib.rs\n\
             +++ src/lib.rs\n\
             @@ -1,1 +1,1 @@\n\
             -    old_line\n\
             +    new_line\n"
        );

        let parsed = parse_patch(&normalized, Some("cx_test")).expect("should parse");
        assert_eq!(parsed.files.len(), 1);
        let file = &parsed.files[0];
        assert_eq!(file.kind, FileChangeKind::Modified);
        assert_eq!(
            file.hunks[0].lines,
            vec![
                TouchedLine {
                    kind: TouchedLineKind::Removed,
                    line_number: 1,
                    content: "    old_line".to_string(),
                    session_id: Some("cx_test".to_string()),
                },
                TouchedLine {
                    kind: TouchedLineKind::Added,
                    line_number: 1,
                    content: "    new_line".to_string(),
                    session_id: Some("cx_test".to_string()),
                },
            ]
        );
    }

    #[test]
    fn normalizes_update_with_move_and_changed_lines() {
        let patch = parse(
            "*** Begin Patch\n\
             *** Update File: old_name.txt\n\
             *** Move to: new_name.txt\n\
             @@\n\
             -old\n\
             +new\n\
             *** End Patch",
        );

        let normalized = normalize_codex_patch(&patch);

        assert_eq!(
            normalized,
            "Index: new_name.txt\n\
             ===================================================================\n\
             --- old_name.txt\n\
             +++ new_name.txt\n\
             @@ -1,1 +1,1 @@\n\
             -old\n\
             +new\n"
        );

        let parsed = parse_patch(&normalized, Some("cx_test")).expect("should parse");
        assert_eq!(parsed.files.len(), 1);
        let file = &parsed.files[0];
        assert_eq!(file.old_path, "old_name.txt");
        assert_eq!(file.new_path, "new_name.txt");
        assert_eq!(file.kind, FileChangeKind::Renamed);
    }

    #[test]
    fn drops_pure_rename_with_no_changed_lines() {
        let patch = parse(
            "*** Begin Patch\n\
             *** Update File: old_name.txt\n\
             *** Move to: new_name.txt\n\
             *** End Patch",
        );

        assert_eq!(normalize_codex_patch(&patch), "");
    }

    #[test]
    fn normalizes_delete_only_patch_to_empty_string() {
        let patch = parse(
            "*** Begin Patch\n\
             *** Delete File: obsolete.txt\n\
             *** End Patch",
        );

        assert_eq!(normalize_codex_patch(&patch), "");
    }

    #[test]
    fn mixed_patch_keeps_only_add_and_update_evidence() {
        let patch = parse(
            "*** Begin Patch\n\
             *** Add File: a.txt\n\
             +hello\n\
             *** Delete File: b.txt\n\
             *** Update File: c.txt\n\
             @@\n\
             -old\n\
             +new\n\
             *** End Patch",
        );

        let normalized = normalize_codex_patch(&patch);
        let parsed = parse_patch(&normalized, Some("cx_test")).expect("should parse");

        assert_eq!(parsed.files.len(), 2);
        assert_eq!(parsed.files[0].new_path, "a.txt");
        assert_eq!(parsed.files[0].kind, FileChangeKind::Added);
        assert_eq!(parsed.files[1].new_path, "c.txt");
        assert_eq!(parsed.files[1].kind, FileChangeKind::Modified);
    }

    #[test]
    fn multiple_hunks_advance_positions_cumulatively() {
        let patch = parse(
            "*** Begin Patch\n\
             *** Update File: d.txt\n\
             @@ fn one() {\n\
             -a\n\
             +b\n\
             @@ fn two() {\n\
             -c\n\
             +d\n\
             *** End Patch",
        );

        let normalized = normalize_codex_patch(&patch);

        assert_eq!(
            normalized,
            "Index: d.txt\n\
             ===================================================================\n\
             --- d.txt\n\
             +++ d.txt\n\
             @@ -1,1 +1,1 @@\n\
             -a\n\
             +b\n\
             @@ -2,1 +2,1 @@\n\
             -c\n\
             +d\n"
        );

        parse_patch(&normalized, Some("cx_test")).expect("should parse");
    }

    /// AC15: synthetic patch-local line numbers must still attribute
    /// correctly through the existing, unmodified `intersect_patches`
    /// historical `kind`+`content` fallback once the real commit lands the
    /// same touched lines at different real line numbers, while an unrelated
    /// committed line does not intersect.
    #[test]
    fn intersect_patches_matches_synthetic_lines_via_historical_fallback() {
        let codex_patch = parse(
            "*** Begin Patch\n\
             *** Update File: src/lib.rs\n\
             @@\n\
             -old_line\n\
             +new_line\n\
             *** End Patch",
        );
        let normalized = normalize_codex_patch(&codex_patch);
        let constructed_patch =
            parse_patch(&normalized, Some("cx_test")).expect("constructed patch should parse");

        // A realistic post-commit patch where the same touched lines sit at
        // different real line numbers than the synthetic ones above (1/1),
        // plus one unrelated line that should not intersect.
        let post_commit_patch = real_commit_patch();

        let overlap = intersect_patches(&constructed_patch, &post_commit_patch);

        assert_eq!(overlap.files.len(), 1);
        let file = &overlap.files[0];
        assert_eq!(file.new_path, "src/lib.rs");
        assert_eq!(file.hunks.len(), 1);
        assert_eq!(
            file.hunks[0].lines,
            vec![
                TouchedLine {
                    kind: TouchedLineKind::Removed,
                    line_number: 42,
                    content: "old_line".to_string(),
                    session_id: Some("cx_test".to_string()),
                },
                TouchedLine {
                    kind: TouchedLineKind::Added,
                    line_number: 42,
                    content: "new_line".to_string(),
                    session_id: Some("cx_test".to_string()),
                },
            ]
        );
    }

    fn real_commit_patch() -> ParsedPatch {
        ParsedPatch {
            files: vec![PatchFileChange {
                old_path: "src/lib.rs".to_string(),
                new_path: "src/lib.rs".to_string(),
                kind: FileChangeKind::Modified,
                hunks: vec![PatchHunk {
                    old_start: 42,
                    old_count: 1,
                    new_start: 42,
                    new_count: 2,
                    model_id: None,
                    lines: vec![
                        TouchedLine {
                            kind: TouchedLineKind::Removed,
                            line_number: 42,
                            content: "old_line".to_string(),
                            session_id: None,
                        },
                        TouchedLine {
                            kind: TouchedLineKind::Added,
                            line_number: 42,
                            content: "new_line".to_string(),
                            session_id: None,
                        },
                        TouchedLine {
                            kind: TouchedLineKind::Added,
                            line_number: 43,
                            content: "unrelated_line".to_string(),
                            session_id: None,
                        },
                    ],
                }],
            }],
        }
    }
}
