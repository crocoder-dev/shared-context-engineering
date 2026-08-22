//! Normalizes a parsed Codex `apply_patch` payload ([`CodexPatch`]) into SCE
//! `Index:`-form unified-diff text that `crate::services::patch::parse_patch`
//! already accepts.
//!
//! Positions are deterministic and event-scoped: each `Update File` operation
//! numbers only the touched (`+`/`-`) lines it actually emits, from a bounded
//! range derived from the stable `tool_use_id`. Local offsets are allocated
//! across every emitted operation, hunk, and file. Codex's own unchanged
//! context lines are dropped, not persisted as evidence, and contribute no
//! positional weight. These positions are evidence identities, never real
//! filesystem line numbers. The
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

use sha2::{Digest, Sha256};

use super::{CodexFileOperation, CodexHunk, CodexHunkLine, CodexPatch};

const PATCH_INDEX_SEPARATOR: &str =
    "===================================================================";
const CODEX_SYNTHETIC_LINE_ID_DOMAIN: &[u8] = b"sce-codex-apply-patch-line-id-v1\0";
const SYNTHETIC_EVENT_RANGE_SIZE: u64 = 1 << 31;
const SYNTHETIC_BASE_OFFSET: u64 = 2;

/// Error produced when Codex apply-patch evidence cannot be assigned safe,
/// event-scoped synthetic line identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexPatchNormalizeError {
    message: String,
}

impl std::fmt::Display for CodexPatchNormalizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "codex apply_patch normalization error: {}", self.message)
    }
}

impl std::error::Error for CodexPatchNormalizeError {}

fn normalize_error(message: impl Into<String>) -> CodexPatchNormalizeError {
    CodexPatchNormalizeError {
        message: message.into(),
    }
}

/// Normalizes every `Add`/`Update` file operation in `patch` into one
/// combined SCE `Index:`-form unified-diff string, in operation order.
///
/// The synthetic line identities are deterministic for `tool_use_id`, and
/// local offsets are allocated across the entire patch rather than restarting
/// for each file. They are evidence identities, not source line numbers.
#[allow(dead_code)]
pub(crate) fn normalize_codex_patch(
    patch: &CodexPatch,
    tool_use_id: &str,
) -> Result<String, CodexPatchNormalizeError> {
    let base = synthetic_base(tool_use_id)?;
    normalize_codex_patch_with_base(patch, base)
}

fn synthetic_base(tool_use_id: &str) -> Result<u64, CodexPatchNormalizeError> {
    let identity = tool_use_id.trim();
    if identity.is_empty() || identity != tool_use_id {
        return Err(normalize_error(
            "Codex apply_patch tool_use_id must be present, trimmed, and non-empty.",
        ));
    }

    let mut hasher = Sha256::new();
    hasher.update(CODEX_SYNTHETIC_LINE_ID_DOMAIN);
    hasher.update(identity.as_bytes());
    let digest = hasher.finalize();
    let mut bucket_bytes = [0_u8; 4];
    bucket_bytes.copy_from_slice(&digest[..4]);
    let bucket = u64::from(u32::from_be_bytes(bucket_bytes));

    bucket
        .checked_mul(SYNTHETIC_EVENT_RANGE_SIZE)
        .and_then(|value| value.checked_add(SYNTHETIC_BASE_OFFSET))
        .ok_or_else(|| normalize_error("Codex apply_patch synthetic base overflowed."))
}

fn normalize_codex_patch_with_base(
    patch: &CodexPatch,
    base: u64,
) -> Result<String, CodexPatchNormalizeError> {
    let mut allocator = SyntheticLineAllocator {
        base,
        next_offset: 0,
    };

    patch
        .operations
        .iter()
        .try_fold(String::new(), |mut output, operation| {
            if let Some(normalized) = normalize_operation(operation, &mut allocator)? {
                output.push_str(&normalized);
            }
            Ok(output)
        })
}

struct SyntheticLineAllocator {
    base: u64,
    next_offset: u64,
}

impl SyntheticLineAllocator {
    fn allocate(&mut self, count: u64) -> Result<u64, CodexPatchNormalizeError> {
        if count == 0 {
            return Err(normalize_error(
                "Codex apply_patch cannot allocate an empty synthetic range.",
            ));
        }

        let start = self.base.checked_add(self.next_offset).ok_or_else(|| {
            normalize_error("Codex apply_patch synthetic line identity overflowed.")
        })?;
        let next_offset = self
            .next_offset
            .checked_add(count)
            .ok_or_else(|| normalize_error("Codex apply_patch synthetic offset overflowed."))?;
        if next_offset > SYNTHETIC_EVENT_RANGE_SIZE {
            return Err(normalize_error(
                "Codex apply_patch synthetic line range was exhausted.",
            ));
        }
        self.base.checked_add(next_offset - 1).ok_or_else(|| {
            normalize_error("Codex apply_patch synthetic line identity overflowed.")
        })?;
        self.next_offset = next_offset;
        Ok(start)
    }
}

fn normalize_operation(
    operation: &CodexFileOperation,
    allocator: &mut SyntheticLineAllocator,
) -> Result<Option<String>, CodexPatchNormalizeError> {
    match operation {
        CodexFileOperation::Add { path, lines } => normalize_add(path, lines, allocator),
        CodexFileOperation::Update {
            old_path,
            new_path,
            hunks,
        } => normalize_update(old_path, new_path.as_deref(), hunks, allocator),
        CodexFileOperation::Delete { .. } => Ok(None),
    }
}

fn normalize_add(
    path: &str,
    lines: &[String],
    allocator: &mut SyntheticLineAllocator,
) -> Result<Option<String>, CodexPatchNormalizeError> {
    if lines.is_empty() {
        return Ok(None);
    }
    let start = allocator.allocate(line_count(lines.len())?)?;
    let mut body = format!("@@ -0,0 +{start},{} @@\n", lines.len());
    for line in lines {
        body.push('+');
        body.push_str(line);
        body.push('\n');
    }
    Ok(Some(render_file_section(path, path, &body)))
}

fn normalize_update(
    old_path: &str,
    new_path: Option<&str>,
    hunks: &[CodexHunk],
    allocator: &mut SyntheticLineAllocator,
) -> Result<Option<String>, CodexPatchNormalizeError> {
    let mut body = String::new();
    let mut has_changes = false;

    for hunk in hunks {
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
                    removed_count = removed_count.checked_add(1).ok_or_else(|| {
                        normalize_error("Codex apply_patch removed-line count overflowed.")
                    })?;
                }
                CodexHunkLine::Added(content) => {
                    hunk_body.push('+');
                    hunk_body.push_str(content);
                    hunk_body.push('\n');
                    added_count = added_count.checked_add(1).ok_or_else(|| {
                        normalize_error("Codex apply_patch added-line count overflowed.")
                    })?;
                }
            }
        }

        if removed_count > 0 || added_count > 0 {
            let local_count = removed_count.max(added_count);
            let start = allocator.allocate(local_count)?;
            let _ = writeln!(
                body,
                "@@ -{start},{removed_count} +{start},{added_count} @@"
            );
            body.push_str(&hunk_body);
            has_changes = true;
        }
    }

    if !has_changes {
        return Ok(None);
    }

    let destination = new_path.unwrap_or(old_path);
    Ok(Some(render_file_section(old_path, destination, &body)))
}

fn line_count(count: usize) -> Result<u64, CodexPatchNormalizeError> {
    u64::try_from(count)
        .map_err(|_| normalize_error("Codex apply_patch line count does not fit in u64."))
}

fn render_file_section(old_path: &str, new_path: &str, body: &str) -> String {
    format!("Index: {new_path}\n{PATCH_INDEX_SEPARATOR}\n--- {old_path}\n+++ {new_path}\n{body}")
}

#[cfg(test)]
mod tests {
    use super::super::parser::parse_codex_apply_patch;
    use super::*;
    use crate::services::patch::{
        combine_patches, intersect_patches, parse_patch, FileChangeKind, ParsedPatch,
        PatchFileChange, PatchHunk, TouchedLine, TouchedLineKind,
    };

    fn parse(raw: &str) -> CodexPatch {
        parse_codex_apply_patch(raw).expect("fixture patch should parse")
    }

    fn normalized(raw: &str) -> String {
        normalize_codex_patch(&parse(raw), "tool-1").expect("fixture should normalize")
    }

    fn test_base() -> u64 {
        synthetic_base("tool-1").expect("test identity should hash")
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

        let normalized = normalize_codex_patch(&patch, "tool-1").expect("should normalize");
        let base = test_base();

        assert_eq!(
            normalized,
            format!(
                "Index: foo.txt\n\
                 ===================================================================\n\
                 --- foo.txt\n\
                 +++ foo.txt\n\
                 @@ -0,0 +{base},2 @@\n\
                 +line one\n\
                 +line two\n"
            )
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
                    line_number: base,
                    content: "line one".to_string(),
                    session_id: Some("cx_test".to_string()),
                },
                TouchedLine {
                    kind: TouchedLineKind::Added,
                    line_number: base + 1,
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

        let normalized = normalize_codex_patch(&patch, "tool-1").expect("should normalize");
        let base = test_base();

        // The context line ("    unchanged") is dropped entirely and
        // contributes no positional weight.
        assert_eq!(
            normalized,
            format!(
                "Index: src/lib.rs\n\
                 ===================================================================\n\
                 --- src/lib.rs\n\
                 +++ src/lib.rs\n\
                 @@ -{base},1 +{base},1 @@\n\
                 -    old_line\n\
                 +    new_line\n"
            )
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
                    line_number: base,
                    content: "    old_line".to_string(),
                    session_id: Some("cx_test".to_string()),
                },
                TouchedLine {
                    kind: TouchedLineKind::Added,
                    line_number: base,
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

        let normalized = normalize_codex_patch(&patch, "tool-1").expect("should normalize");
        let base = test_base();

        assert_eq!(
            normalized,
            format!(
                "Index: new_name.txt\n\
                 ===================================================================\n\
                 --- old_name.txt\n\
                 +++ new_name.txt\n\
                 @@ -{base},1 +{base},1 @@\n\
                 -old\n\
                 +new\n"
            )
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

        assert_eq!(
            normalize_codex_patch(&patch, "tool-1").expect("should normalize"),
            ""
        );
    }

    #[test]
    fn normalizes_delete_only_patch_to_empty_string() {
        let patch = parse(
            "*** Begin Patch\n\
             *** Delete File: obsolete.txt\n\
             *** End Patch",
        );

        assert_eq!(
            normalize_codex_patch(&patch, "tool-1").expect("should normalize"),
            ""
        );
    }

    #[test]
    fn mixed_patch_keeps_only_add_and_update_evidence() {
        let normalized = normalized(
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

        let normalized = normalize_codex_patch(&patch, "tool-1").expect("should normalize");
        let base = test_base();

        assert_eq!(
            normalized,
            format!(
                "Index: d.txt\n\
                 ===================================================================\n\
                 --- d.txt\n\
                 +++ d.txt\n\
                 @@ -{base},1 +{base},1 @@\n\
                 -a\n\
                 +b\n\
                 @@ -{next},1 +{next},1 @@\n\
                 -c\n\
                 +d\n",
                next = base + 1
            )
        );

        parse_patch(&normalized, Some("cx_test")).expect("should parse");
    }

    #[test]
    fn event_scoped_normalization_is_deterministic_and_allocates_across_files() {
        let patch = parse(
            "*** Begin Patch\n\
             *** Add File: first.txt\n\
             +first\n\
             +second\n\
             *** Update File: second.txt\n\
             @@\n\
             -old\n\
             +new\n\
             *** Add File: third.txt\n\
             +third\n\
             *** End Patch",
        );

        let first = normalize_codex_patch(&patch, "event-1").expect("should normalize");
        let repeated = normalize_codex_patch(&patch, "event-1").expect("should normalize");
        assert_eq!(first, repeated);

        let parsed = parse_patch(&first, None).expect("normalized patch should parse");
        let line_numbers: Vec<u64> = parsed
            .files
            .iter()
            .flat_map(|file| file.hunks.iter())
            .flat_map(|hunk| hunk.lines.iter())
            .map(|line| line.line_number)
            .collect();
        assert_eq!(line_numbers.len(), 5);
        assert!(line_numbers.iter().all(|line| *line > 1));
        assert_eq!(
            line_numbers,
            vec![
                test_base_for("event-1"),
                test_base_for("event-1") + 1,
                test_base_for("event-1") + 2,
                test_base_for("event-1") + 2,
                test_base_for("event-1") + 3,
            ]
        );
    }

    #[test]
    fn different_event_ids_use_separate_synthetic_ranges() {
        let patch = parse(
            "*** Begin Patch\n\
             *** Add File: same.txt\n\
             +same content\n\
             *** End Patch",
        );

        let first = normalize_codex_patch(&patch, "event-1").expect("should normalize");
        let second = normalize_codex_patch(&patch, "event-2").expect("should normalize");
        assert_ne!(synthetic_base("event-1"), synthetic_base("event-2"));
        assert_ne!(first, second);

        let first_line = parse_patch(&first, None)
            .expect("first patch should parse")
            .files[0]
            .hunks[0]
            .lines[0]
            .line_number;
        let second_line = parse_patch(&second, None)
            .expect("second patch should parse")
            .files[0]
            .hunks[0]
            .lines[0]
            .line_number;
        assert_ne!(first_line, second_line);
    }

    #[test]
    fn rejects_faulted_identity_inputs_and_checked_range_overflow() {
        let patch = parse(
            "*** Begin Patch\n\
             *** Add File: file.txt\n\
             +one\n\
             +two\n\
             *** End Patch",
        );

        assert!(normalize_codex_patch(&patch, "").is_err());
        assert!(normalize_codex_patch(&patch, "   ").is_err());
        assert!(normalize_codex_patch(&patch, " event-1").is_err());
        assert!(normalize_codex_patch_with_base(&patch, u64::MAX).is_err());
    }

    #[test]
    fn same_content_events_survive_combination_and_match_two_commit_additions() {
        let patch = parse(
            "*** Begin Patch\n\
             *** Add File: same.txt\n\
             +same content\n\
             *** End Patch",
        );
        let first = parse_patch(
            &normalize_codex_patch(&patch, "event-1").expect("first event should normalize"),
            Some("cx_event-1"),
        )
        .expect("first normalized patch should parse");
        let second = parse_patch(
            &normalize_codex_patch(&patch, "event-2").expect("second event should normalize"),
            Some("cx_event-2"),
        )
        .expect("second normalized patch should parse");

        let combined = combine_patches(&[first, second]);
        let combined_lines: Vec<&TouchedLine> = combined.files[0]
            .hunks
            .iter()
            .flat_map(|hunk| hunk.lines.iter())
            .collect();
        assert_eq!(combined_lines.len(), 2);
        assert_ne!(combined_lines[0].line_number, combined_lines[1].line_number);

        let post_commit = ParsedPatch {
            files: vec![PatchFileChange {
                old_path: String::new(),
                new_path: "same.txt".to_string(),
                kind: FileChangeKind::Added,
                hunks: vec![PatchHunk {
                    old_start: 0,
                    old_count: 0,
                    new_start: 40,
                    new_count: 2,
                    model_id: None,
                    lines: vec![
                        TouchedLine {
                            kind: TouchedLineKind::Added,
                            line_number: 40,
                            content: "same content".to_string(),
                            session_id: None,
                        },
                        TouchedLine {
                            kind: TouchedLineKind::Added,
                            line_number: 41,
                            content: "same content".to_string(),
                            session_id: None,
                        },
                    ],
                }],
            }],
        };

        let overlap = intersect_patches(&combined, &post_commit);
        let overlap_lines: Vec<&TouchedLine> = overlap.files[0]
            .hunks
            .iter()
            .flat_map(|hunk| hunk.lines.iter())
            .collect();
        assert_eq!(overlap_lines.len(), 2);
        assert_eq!(overlap_lines[0].line_number, 40);
        assert_eq!(overlap_lines[1].line_number, 41);
    }

    #[test]
    fn repeated_identical_content_remains_physically_ambiguous_without_line_ranges() {
        let patch = parse(
            "*** Begin Patch\n\
             *** Update File: repeated.txt\n\
             @@\n\
             -before\n\
             +same\n\
             @@\n\
             -before\n\
             +same\n\
             *** End Patch",
        );
        let first = parse_patch(
            &normalize_codex_patch(&patch, "event-1").expect("first event should normalize"),
            Some("cx_event-1"),
        )
        .expect("first normalized patch should parse");
        let second = parse_patch(
            &normalize_codex_patch(&patch, "event-2").expect("second event should normalize"),
            Some("cx_event-2"),
        )
        .expect("second normalized patch should parse");

        let combined = combine_patches(&[first, second]);
        let post_commit = ParsedPatch {
            files: vec![PatchFileChange {
                old_path: "repeated.txt".to_string(),
                new_path: "repeated.txt".to_string(),
                kind: FileChangeKind::Modified,
                hunks: vec![PatchHunk {
                    old_start: 20,
                    old_count: 2,
                    new_start: 20,
                    new_count: 2,
                    model_id: None,
                    lines: vec![
                        TouchedLine {
                            kind: TouchedLineKind::Removed,
                            line_number: 20,
                            content: "before".to_string(),
                            session_id: None,
                        },
                        TouchedLine {
                            kind: TouchedLineKind::Added,
                            line_number: 20,
                            content: "same".to_string(),
                            session_id: None,
                        },
                    ],
                }],
            }],
        };

        let overlap = intersect_patches(&combined, &post_commit);
        let lines: Vec<&TouchedLine> = overlap.files[0]
            .hunks
            .iter()
            .flat_map(|hunk| hunk.lines.iter())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].session_id.as_deref(), Some("cx_event-1"));
        assert_eq!(
            lines[1].session_id.as_deref(),
            Some("cx_event-1"),
            "without true line ranges, both matching physical lines are attributed to the first available event"
        );
    }

    fn test_base_for(tool_use_id: &str) -> u64 {
        synthetic_base(tool_use_id).expect("test identity should hash")
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
        let normalized = normalize_codex_patch(&codex_patch, "tool-1").expect("should normalize");
        let constructed_patch =
            parse_patch(&normalized, Some("cx_test")).expect("constructed patch should parse");

        // A realistic post-commit patch where the same touched lines sit at
        // different real line numbers than the event-scoped synthetic ones,
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
