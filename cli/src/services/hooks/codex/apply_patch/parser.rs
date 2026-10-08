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

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexPatch {
    pub(crate) operations: Vec<CodexFileOperation>,
}

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

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexHunk {
    pub(crate) context: Option<String>,
    pub(crate) lines: Vec<CodexHunkLine>,
    pub(crate) is_end_of_file: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CodexHunkLine {
    Context(String),
    Added(String),
    Removed(String),
}

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

#[allow(dead_code)]
pub(crate) fn normalize_outer_apply_patch_input(raw: &str) -> Result<String, CodexPatchParseError> {
    let trimmed = raw.trim();
    let lines: Vec<&str> = trimmed.lines().collect();

    if has_canonical_boundaries(&lines) {
        return Ok(raw.to_string());
    }

    let Some(first_line) = lines.first().copied() else {
        return Err(error("Codex apply_patch input cannot be empty."));
    };
    let Some(last_line) = lines.last().copied() else {
        return Err(error("Codex apply_patch input cannot be empty."));
    };

    let is_supported_heredoc = matches!(first_line, "<<EOF" | "<<'EOF'" | "<<\"EOF\"");
    if !is_supported_heredoc || last_line != "EOF" || lines.len() < 4 {
        return Err(error(
            "Codex apply_patch input must be a canonical patch or one of the supported EOF wrappers.",
        ));
    }

    let inner_lines = &lines[1..lines.len() - 1];
    if !has_canonical_boundaries(inner_lines) {
        return Err(error(
            "Codex apply_patch heredoc must contain exactly one canonical patch.",
        ));
    }

    Ok(inner_lines.join("\n"))
}

fn has_canonical_boundaries(lines: &[&str]) -> bool {
    lines.first().map(|line| line.trim()) == Some(BEGIN_PATCH_MARKER)
        && lines.last().map(|line| line.trim()) == Some(END_PATCH_MARKER)
}

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

fn validate_path(path: &str) -> Result<String, CodexPatchParseError> {
    if path.is_empty() {
        return Err(error("Codex apply_patch path cannot be empty."));
    }

    Ok(path.to_string())
}
