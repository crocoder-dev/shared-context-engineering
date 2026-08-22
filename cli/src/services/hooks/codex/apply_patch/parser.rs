//! Grammar parser for Codex's `apply_patch` custom patch text.
//!
//! The grammar implemented here follows the official Lark grammar documented
//! in `openai/codex`'s `codex-rs/apply-patch/src/parser.rs` (checked against
//! that source directly for this task; see plan
//! `context/plans/codex-cli-integration.md` Assumptions):
//!
//! ```text
//! start: begin_patch environment_id? hunk+ end_patch
//! begin_patch: "*** Begin Patch" LF
//! environment_id: "*** Environment ID: " filename LF
//! end_patch: "*** End Patch" LF?
//!
//! hunk: add_hunk | delete_hunk | update_hunk
//! add_hunk: "*** Add File: " filename LF add_line+
//! delete_hunk: "*** Delete File: " filename LF
//! update_hunk: "*** Update File: " filename LF change_move? change?
//! filename: /(.+)/
//! add_line: "+" /(.+)/ LF -> line
//!
//! change_move: "*** Move to: " filename LF
//! change: (change_context | change_line)+ eof_line?
//! change_context: ("@@" | "@@ " /(.+)/) LF
//! change_line: ("+" | "-" | " ") /(.+)/ LF
//! eof_line: "*** End of File" LF
//! ```
//!
//! Upstream Codex itself accepts absolute hunk paths (resolving them against
//! the tool's own `cwd` later). This parser is deliberately more
//! conservative than upstream, per this task's own scope: it rejects
//! absolute paths and `..` traversal segments outright, since SCE has no
//! equivalent downstream resolution step and normalized evidence must stay
//! anchored inside the repository working tree.

use std::path::{Component, Path};

const BEGIN_PATCH_MARKER: &str = "*** Begin Patch";
const END_PATCH_MARKER: &str = "*** End Patch";
const ENVIRONMENT_ID_MARKER: &str = "*** Environment ID: ";
const ADD_FILE_MARKER: &str = "*** Add File: ";
const DELETE_FILE_MARKER: &str = "*** Delete File: ";
const UPDATE_FILE_MARKER: &str = "*** Update File: ";
const MOVE_TO_MARKER: &str = "*** Move to: ";
const END_OF_FILE_MARKER: &str = "*** End of File";
const CHANGE_CONTEXT_MARKER: &str = "@@";
const CHANGE_CONTEXT_MARKER_WITH_TEXT: &str = "@@ ";

/// One fully parsed Codex `apply_patch` payload: an ordered list of the file
/// operations it declares. Order is preserved from the source text.
#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexPatch {
    pub(crate) operations: Vec<CodexFileOperation>,
}

/// A single `*** Add File:` / `*** Update File:` / `*** Delete File:` block.
#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CodexFileOperation {
    Add {
        path: String,
        lines: Vec<String>,
    },
    Update {
        old_path: String,
        new_path: Option<String>,
        hunks: Vec<CodexHunk>,
    },
    Delete {
        path: String,
    },
}

/// One contiguous change region within an `*** Update File:` block, started
/// either by an explicit `@@` context marker or implicitly by the first
/// change line when no `@@` marker precedes it.
#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexHunk {
    pub(crate) context: Option<String>,
    pub(crate) lines: Vec<CodexHunkLine>,
    pub(crate) is_end_of_file: bool,
}

/// A single line within a [`CodexHunk`], without its leading marker
/// character.
#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CodexHunkLine {
    Context(String),
    Added(String),
    Removed(String),
}

/// Error produced when raw `apply_patch` text does not conform to the
/// grammar above, or violates this parser's own conservative path
/// validation.
#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexPatchParseError {
    pub(crate) message: String,
}

impl std::fmt::Display for CodexPatchParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "codex apply_patch parse error: {}", self.message)
    }
}

impl std::error::Error for CodexPatchParseError {}

fn error(message: impl Into<String>) -> CodexPatchParseError {
    CodexPatchParseError {
        message: message.into(),
    }
}

/// Parses raw Codex `apply_patch` `tool_input.command` text into a
/// [`CodexPatch`]. Performs no normalization to SCE unified-diff form and no
/// filesystem access; see the `apply_patch` module's own scope boundary.
#[allow(dead_code)]
pub(crate) fn parse_codex_apply_patch(raw: &str) -> Result<CodexPatch, CodexPatchParseError> {
    let trimmed = raw.trim();
    let lines: Vec<&str> = trimmed.lines().collect();

    if lines.first().map(|line| line.trim()) != Some(BEGIN_PATCH_MARKER) {
        return Err(error(format!(
            "Codex apply_patch text must start with '{BEGIN_PATCH_MARKER}'."
        )));
    }
    if lines.last().map(|line| line.trim()) != Some(END_PATCH_MARKER) {
        return Err(error(format!(
            "Codex apply_patch text must end with '{END_PATCH_MARKER}'."
        )));
    }

    let mut body: &[&str] = &lines[1..lines.len() - 1];

    if let Some(first) = body.first() {
        if let Some(raw_id) = first.strip_prefix(ENVIRONMENT_ID_MARKER) {
            if raw_id.trim().is_empty() {
                return Err(error("Codex apply_patch environment id cannot be empty."));
            }
            body = &body[1..];
        }
    }

    let mut operations = Vec::new();
    let mut index = 0;

    while index < body.len() {
        let line = body[index];

        if let Some(path) = line.strip_prefix(ADD_FILE_MARKER) {
            let path = validate_path(path.trim())?;
            index += 1;

            let mut added_lines = Vec::new();
            while index < body.len() && !is_top_level_marker(body[index]) {
                let content_line = body[index];
                match content_line.strip_prefix('+') {
                    Some(content) => added_lines.push(content.to_string()),
                    None => {
                        return Err(error(format!(
                            "Codex apply_patch Add File '{path}' has an unrecognized line {}: '{content_line}'.",
                            index + 1
                        )));
                    }
                }
                index += 1;
            }

            if added_lines.is_empty() {
                return Err(error(format!(
                    "Codex apply_patch Add File '{path}' has no added lines."
                )));
            }

            operations.push(CodexFileOperation::Add {
                path,
                lines: added_lines,
            });
        } else if let Some(path) = line.strip_prefix(DELETE_FILE_MARKER) {
            let path = validate_path(path.trim())?;
            operations.push(CodexFileOperation::Delete { path });
            index += 1;
        } else if let Some(path) = line.strip_prefix(UPDATE_FILE_MARKER) {
            let old_path = validate_path(path.trim())?;
            index += 1;

            let mut new_path = None;
            if index < body.len() {
                if let Some(destination) = body[index].strip_prefix(MOVE_TO_MARKER) {
                    new_path = Some(validate_path(destination.trim())?);
                    index += 1;
                }
            }

            let (hunks, consumed) = parse_update_hunks(&old_path, &body[index..])?;
            index += consumed;

            if hunks.is_empty() && new_path.is_none() {
                return Err(error(format!(
                    "Codex apply_patch Update File '{old_path}' has no move and no changes."
                )));
            }

            operations.push(CodexFileOperation::Update {
                old_path,
                new_path,
                hunks,
            });
        } else {
            return Err(error(format!(
                "Unrecognized Codex apply_patch operation line {}: '{line}'.",
                index + 1
            )));
        }
    }

    Ok(CodexPatch { operations })
}

fn is_top_level_marker(line: &str) -> bool {
    line.starts_with(ADD_FILE_MARKER)
        || line.starts_with(DELETE_FILE_MARKER)
        || line.starts_with(UPDATE_FILE_MARKER)
}

/// Parses the `change_move? change?` tail of an `*** Update File:` block
/// (with any `*** Move to:` line already consumed by the caller), returning
/// the resulting hunks plus how many lines of `lines` were consumed.
fn parse_update_hunks(
    path: &str,
    lines: &[&str],
) -> Result<(Vec<CodexHunk>, usize), CodexPatchParseError> {
    let mut hunks: Vec<CodexHunk> = Vec::new();
    let mut consumed = 0;

    while consumed < lines.len() && !is_top_level_marker(lines[consumed]) {
        let line = lines[consumed];

        if line == CHANGE_CONTEXT_MARKER || line.starts_with(CHANGE_CONTEXT_MARKER_WITH_TEXT) {
            let context = line
                .strip_prefix(CHANGE_CONTEXT_MARKER_WITH_TEXT)
                .map(str::to_string);
            hunks.push(CodexHunk {
                context,
                lines: Vec::new(),
                is_end_of_file: false,
            });
            consumed += 1;
            continue;
        }

        if line.trim() == END_OF_FILE_MARKER {
            match hunks.last_mut() {
                Some(hunk) => hunk.is_end_of_file = true,
                None => hunks.push(CodexHunk {
                    context: None,
                    lines: Vec::new(),
                    is_end_of_file: true,
                }),
            }
            consumed += 1;
            continue;
        }

        let hunk_line = if line.is_empty() {
            CodexHunkLine::Context(String::new())
        } else {
            let mut chars = line.chars();
            let marker = chars.next();
            let rest = chars.as_str();
            match marker {
                Some('+') => CodexHunkLine::Added(rest.to_string()),
                Some('-') => CodexHunkLine::Removed(rest.to_string()),
                Some(' ') => CodexHunkLine::Context(rest.to_string()),
                _ => {
                    return Err(error(format!(
                        "Codex apply_patch Update File '{path}' has an unrecognized change line {}: '{line}'.",
                        consumed + 1
                    )));
                }
            }
        };

        if hunks.is_empty() {
            hunks.push(CodexHunk {
                context: None,
                lines: Vec::new(),
                is_end_of_file: false,
            });
        }
        hunks
            .last_mut()
            .expect("a hunk was just ensured present above")
            .lines
            .push(hunk_line);
        consumed += 1;
    }

    Ok((hunks, consumed))
}

/// Conservative path validation: rejects absolute paths and any `..`
/// traversal segment. Deliberately stricter than upstream Codex, which
/// accepts and resolves absolute hunk paths itself (see this module's own
/// doc comment).
fn validate_path(path: &str) -> Result<String, CodexPatchParseError> {
    if path.is_empty() {
        return Err(error("Codex apply_patch path cannot be empty."));
    }

    let candidate = Path::new(path);

    if candidate.is_absolute() {
        return Err(error(format!(
            "Codex apply_patch path '{path}' must not be absolute."
        )));
    }

    if candidate
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(error(format!(
            "Codex apply_patch path '{path}' must not contain '..' traversal segments."
        )));
    }

    Ok(path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_add_file_operation() {
        let patch = "*** Begin Patch\n\
             *** Add File: foo.txt\n\
             +line one\n\
             +line two\n\
             *** End Patch";

        let parsed = parse_codex_apply_patch(patch).expect("valid Add File patch should parse");

        assert_eq!(
            parsed.operations,
            vec![CodexFileOperation::Add {
                path: "foo.txt".to_string(),
                lines: vec!["line one".to_string(), "line two".to_string()],
            }]
        );
    }

    #[test]
    fn parses_update_file_single_hunk() {
        let patch = "*** Begin Patch\n\
             *** Update File: src/lib.rs\n\
             @@ fn main() {\n\
             -    old_line\n\
             +    new_line\n\
             *** End Patch";

        let parsed = parse_codex_apply_patch(patch).expect("valid Update File patch should parse");

        assert_eq!(
            parsed.operations,
            vec![CodexFileOperation::Update {
                old_path: "src/lib.rs".to_string(),
                new_path: None,
                hunks: vec![CodexHunk {
                    context: Some("fn main() {".to_string()),
                    lines: vec![
                        CodexHunkLine::Removed("    old_line".to_string()),
                        CodexHunkLine::Added("    new_line".to_string()),
                    ],
                    is_end_of_file: false,
                }],
            }]
        );
    }

    #[test]
    fn parses_delete_file_operation() {
        let patch = "*** Begin Patch\n\
             *** Delete File: obsolete.txt\n\
             *** End Patch";

        let parsed = parse_codex_apply_patch(patch).expect("valid Delete File patch should parse");

        assert_eq!(
            parsed.operations,
            vec![CodexFileOperation::Delete {
                path: "obsolete.txt".to_string(),
            }]
        );
    }

    #[test]
    fn parses_update_file_with_move_to_and_changes() {
        let patch = "*** Begin Patch\n\
             *** Update File: old_name.txt\n\
             *** Move to: new_name.txt\n\
             @@\n\
             -old\n\
             +new\n\
             *** End Patch";

        let parsed =
            parse_codex_apply_patch(patch).expect("valid Update File + Move to patch should parse");

        assert_eq!(
            parsed.operations,
            vec![CodexFileOperation::Update {
                old_path: "old_name.txt".to_string(),
                new_path: Some("new_name.txt".to_string()),
                hunks: vec![CodexHunk {
                    context: None,
                    lines: vec![
                        CodexHunkLine::Removed("old".to_string()),
                        CodexHunkLine::Added("new".to_string()),
                    ],
                    is_end_of_file: false,
                }],
            }]
        );
    }

    #[test]
    fn parses_pure_rename_with_no_changed_lines() {
        let patch = "*** Begin Patch\n\
             *** Update File: old_name.txt\n\
             *** Move to: new_name.txt\n\
             *** End Patch";

        let parsed =
            parse_codex_apply_patch(patch).expect("a move with no changes should still parse");

        assert_eq!(
            parsed.operations,
            vec![CodexFileOperation::Update {
                old_path: "old_name.txt".to_string(),
                new_path: Some("new_name.txt".to_string()),
                hunks: Vec::new(),
            }]
        );
    }

    #[test]
    fn parses_multiple_operations_in_one_patch() {
        let patch = "*** Begin Patch\n\
             *** Add File: a.txt\n\
             +hello\n\
             *** Delete File: b.txt\n\
             *** Update File: c.txt\n\
             @@\n\
             -old\n\
             +new\n\
             *** End Patch";

        let parsed = parse_codex_apply_patch(patch).expect("multi-operation patch should parse");

        assert_eq!(
            parsed.operations,
            vec![
                CodexFileOperation::Add {
                    path: "a.txt".to_string(),
                    lines: vec!["hello".to_string()],
                },
                CodexFileOperation::Delete {
                    path: "b.txt".to_string(),
                },
                CodexFileOperation::Update {
                    old_path: "c.txt".to_string(),
                    new_path: None,
                    hunks: vec![CodexHunk {
                        context: None,
                        lines: vec![
                            CodexHunkLine::Removed("old".to_string()),
                            CodexHunkLine::Added("new".to_string()),
                        ],
                        is_end_of_file: false,
                    }],
                },
            ]
        );
    }

    #[test]
    fn parses_multiple_hunks_within_one_update_file() {
        let patch = "*** Begin Patch\n\
             *** Update File: d.txt\n\
             @@ fn one() {\n\
             -a\n\
             +b\n\
             @@ fn two() {\n\
             -c\n\
             +d\n\
             *** End Patch";

        let parsed =
            parse_codex_apply_patch(patch).expect("multi-hunk Update File patch should parse");

        assert_eq!(
            parsed.operations,
            vec![CodexFileOperation::Update {
                old_path: "d.txt".to_string(),
                new_path: None,
                hunks: vec![
                    CodexHunk {
                        context: Some("fn one() {".to_string()),
                        lines: vec![
                            CodexHunkLine::Removed("a".to_string()),
                            CodexHunkLine::Added("b".to_string()),
                        ],
                        is_end_of_file: false,
                    },
                    CodexHunk {
                        context: Some("fn two() {".to_string()),
                        lines: vec![
                            CodexHunkLine::Removed("c".to_string()),
                            CodexHunkLine::Added("d".to_string()),
                        ],
                        is_end_of_file: false,
                    },
                ],
            }]
        );
    }

    #[test]
    fn sets_end_of_file_flag_on_trailing_marker() {
        let patch = "*** Begin Patch\n\
             *** Update File: e.txt\n\
             @@\n\
             +tail line\n\
             *** End of File\n\
             *** End Patch";

        let parsed = parse_codex_apply_patch(patch).expect("End of File marker should parse");

        assert_eq!(
            parsed.operations,
            vec![CodexFileOperation::Update {
                old_path: "e.txt".to_string(),
                new_path: None,
                hunks: vec![CodexHunk {
                    context: None,
                    lines: vec![CodexHunkLine::Added("tail line".to_string())],
                    is_end_of_file: true,
                }],
            }]
        );
    }

    #[test]
    fn accepts_environment_id_preamble() {
        let patch = "*** Begin Patch\n\
             *** Environment ID: remote\n\
             *** Add File: hello.txt\n\
             +hello\n\
             *** End Patch";

        let parsed = parse_codex_apply_patch(patch).expect("Environment ID preamble should parse");

        assert_eq!(
            parsed.operations,
            vec![CodexFileOperation::Add {
                path: "hello.txt".to_string(),
                lines: vec!["hello".to_string()],
            }]
        );
    }

    #[test]
    fn rejects_empty_environment_id() {
        let patch = "*** Begin Patch\n\
             *** Environment ID:   \n\
             *** Add File: hello.txt\n\
             +hello\n\
             *** End Patch";

        let error = parse_codex_apply_patch(patch).expect_err("empty environment id is invalid");

        assert!(error.message.contains("environment id cannot be empty"));
    }

    #[test]
    fn rejects_missing_begin_marker() {
        let patch = "not a patch\n*** End Patch";

        let error =
            parse_codex_apply_patch(patch).expect_err("missing Begin Patch marker is invalid");

        assert!(error.message.contains("must start with"));
    }

    #[test]
    fn rejects_missing_end_marker() {
        let patch = "*** Begin Patch\n*** Add File: a.txt\n+x";

        let error =
            parse_codex_apply_patch(patch).expect_err("missing End Patch marker is invalid");

        assert!(error.message.contains("must end with"));
    }

    #[test]
    fn rejects_malformed_operation_line() {
        let patch = "*** Begin Patch\n*** Bogus Section\n*** End Patch";

        let error =
            parse_codex_apply_patch(patch).expect_err("unrecognized operation line is invalid");

        assert!(error
            .message
            .contains("Unrecognized Codex apply_patch operation line"));
    }

    #[test]
    fn accepts_nested_relative_path() {
        let patch = "*** Begin Patch\n\
             *** Add File: src/nested/file.txt\n\
             +content\n\
             *** End Patch";

        let parsed =
            parse_codex_apply_patch(patch).expect("a nested relative path should be accepted");

        assert_eq!(
            parsed.operations,
            vec![CodexFileOperation::Add {
                path: "src/nested/file.txt".to_string(),
                lines: vec!["content".to_string()],
            }]
        );
    }

    #[test]
    fn rejects_absolute_path() {
        let patch = "*** Begin Patch\n\
             *** Add File: /etc/passwd\n\
             +x\n\
             *** End Patch";

        let error = parse_codex_apply_patch(patch).expect_err("absolute path is rejected");

        assert!(error.message.contains("must not be absolute"));
    }

    #[test]
    fn rejects_traversal_path() {
        let patch = "*** Begin Patch\n\
             *** Add File: ../../etc/passwd\n\
             +x\n\
             *** End Patch";

        let error = parse_codex_apply_patch(patch).expect_err("traversal path is rejected");

        assert!(error.message.contains("traversal"));
    }
}
