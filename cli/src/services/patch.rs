use serde::{Deserialize, Serialize};

mod git_path;

use git_path::{decode_marker_path, decode_path, split_diff_header, GitPathError};

const GIT_SOURCE_PREFIX: &str = "a/";
const GIT_DESTINATION_PREFIX: &str = "b/";
const DEV_NULL: &str = "/dev/null";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ParsedPatch {
    pub files: Vec<PatchFileChange>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PatchFileChange {
    pub old_path: String,
    pub new_path: String,
    pub kind: FileChangeKind,
    pub hunks: Vec<PatchHunk>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PatchHunk {
    pub old_start: u64,
    pub old_count: u64,
    pub new_start: u64,
    pub new_count: u64,
    #[serde(default)]
    pub model_id: Option<String>,
    pub lines: Vec<TouchedLine>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TouchedLine {
    pub kind: TouchedLineKind,
    pub line_number: u64,
    pub content: String,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TouchedLineKind {
    Added,
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

pub fn load_patch_from_json(input: &str) -> Result<ParsedPatch, PatchLoadError> {
    serde_json::from_str(input).map_err(|e| PatchLoadError {
        message: format!("invalid patch JSON: {e}"),
    })
}

pub fn parse_patch(input: &str, session_id: Option<&str>) -> Result<ParsedPatch, ParseError> {
    let mut files: Vec<PatchFileChange> = Vec::new();
    let mut current_file: Option<FileBuilder> = None;

    let mut lines = input.lines().peekable();

    while let Some(line) = lines.next() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if let Some(fb) = current_file.take() {
                files.push(fb.build()?);
            }
            let paths = parse_git_diff_header(rest).map_err(|error| path_error(line, &error))?;
            current_file = Some(match paths {
                Some(paths) => FileBuilder::new(paths.old_path, paths.new_path),
                None => FileBuilder::unresolved(line),
            });
            continue;
        }

        if let Some(rest) = line.strip_prefix("Index: ") {
            if let Some(fb) = current_file.take() {
                files.push(fb.build()?);
            }
            let index_path = rest.trim().to_string();
            current_file = Some(FileBuilder::new(index_path.clone(), index_path));
            continue;
        }

        if line.starts_with("===") {
            continue;
        }

        if let Some(rest) = line.strip_prefix("--- ") {
            let fb = current_file.as_mut().ok_or_else(|| ParseError {
                message: format!(
                    "encountered '---' line without a preceding file header: {line:?}"
                ),
            })?;
            fb.set_old_path(
                parse_diff_path(rest, GIT_SOURCE_PREFIX)
                    .map_err(|error| path_error(line, &error))?,
            );
            continue;
        }

        if let Some(rest) = line.strip_prefix("+++ ") {
            let fb = current_file.as_mut().ok_or_else(|| ParseError {
                message: format!(
                    "encountered '+++' line without a preceding file header: {line:?}"
                ),
            })?;
            fb.set_new_path(
                parse_diff_path(rest, GIT_DESTINATION_PREFIX)
                    .map_err(|error| path_error(line, &error))?,
            );
            continue;
        }

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
            if let Some(fb) = current_file.as_mut() {
                if line.starts_with("new file mode ") {
                    fb.mark_added();
                } else if line.starts_with("deleted file mode ") {
                    fb.mark_deleted();
                } else if line.starts_with("rename from ") || line.starts_with("rename to ") {
                    fb.mark_renamed();
                }
                apply_metadata_path(fb, line)?;
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("@@ ") {
            if let Some(fb) = current_file.as_mut() {
                let hunk = parse_hunk_header_and_body(rest, &mut lines, session_id)?;
                fb.add_hunk(hunk);
            }
        }
    }

    if let Some(fb) = current_file.take() {
        files.push(fb.build()?);
    }

    Ok(ParsedPatch { files })
}

struct FileBuilder {
    header: Option<String>,
    old_path: Option<String>,
    new_path: Option<String>,
    kind: Option<FileChangeKind>,
    hunks: Vec<PatchHunk>,
}

impl FileBuilder {
    fn new(old_path: String, new_path: String) -> Self {
        Self {
            header: None,
            old_path: Some(old_path),
            new_path: Some(new_path),
            kind: None,
            hunks: Vec::new(),
        }
    }

    fn unresolved(header: &str) -> Self {
        Self {
            header: Some(header.to_owned()),
            old_path: None,
            new_path: None,
            kind: None,
            hunks: Vec::new(),
        }
    }

    fn set_old_path(&mut self, path: String) {
        self.old_path = Some(path);
    }

    fn set_new_path(&mut self, path: String) {
        self.new_path = Some(path);
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

    fn build(self) -> Result<PatchFileChange, ParseError> {
        let (Some(old_path), Some(new_path)) = (self.old_path, self.new_path) else {
            return Err(ParseError {
                message: format!(
                    "cannot determine both paths of ambiguous header {:?}",
                    self.header.unwrap_or_default()
                ),
            });
        };
        let inferred_kind = self.kind.unwrap_or_else(|| {
            infer_file_kind_from_hunks(&self.hunks)
                .unwrap_or_else(|| determine_file_kind(&old_path, &new_path))
        });
        let (old_path, new_path) = normalize_paths_for_kind(old_path, new_path, inferred_kind);

        Ok(PatchFileChange {
            old_path,
            new_path,
            kind: inferred_kind,
            hunks: self.hunks,
        })
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

fn parse_git_diff_header(rest: &str) -> Result<Option<DiffPaths>, GitPathError> {
    Ok(split_diff_header(rest)?.map(|(old, new)| DiffPaths {
        old_path: strip_git_prefix(old, GIT_SOURCE_PREFIX),
        new_path: strip_git_prefix(new, GIT_DESTINATION_PREFIX),
    }))
}

struct DiffPaths {
    old_path: String,
    new_path: String,
}

fn parse_diff_path(rest: &str, prefix: &str) -> Result<String, GitPathError> {
    if !rest.starts_with('"') && rest.split('\t').next() == Some(DEV_NULL) {
        return Ok(String::new());
    }
    decode_marker_path(rest).map(|path| strip_git_prefix(path, prefix))
}

fn strip_git_prefix(path: String, prefix: &str) -> String {
    match path.strip_prefix(prefix) {
        Some(stripped) => stripped.to_owned(),
        None => path,
    }
}

fn apply_metadata_path(fb: &mut FileBuilder, line: &str) -> Result<(), ParseError> {
    let decode = |rest: &str| decode_path(rest).map_err(|error| path_error(line, &error));
    if let Some(rest) = line
        .strip_prefix("rename from ")
        .or_else(|| line.strip_prefix("copy from "))
    {
        fb.set_old_path(decode(rest)?);
    } else if let Some(rest) = line
        .strip_prefix("rename to ")
        .or_else(|| line.strip_prefix("copy to "))
    {
        fb.set_new_path(decode(rest)?);
    }
    Ok(())
}

fn path_error(line: &str, error: &GitPathError) -> ParseError {
    ParseError {
        message: format!("invalid path in {line:?}: {error}"),
    }
}

fn parse_hunk_header_and_body<'a, I>(
    rest: &str,
    lines: &mut std::iter::Peekable<I>,
    session_id: Option<&str>,
) -> Result<PatchHunk, ParseError>
where
    I: Iterator<Item = &'a str>,
{
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

    let mut touched_lines: Vec<TouchedLine> = Vec::new();
    let mut old_line_num = old_start;
    let mut new_line_num = new_start;

    while let Some(&line) = lines.peek() {
        if line.starts_with("diff --git ") || line.starts_with("Index: ") || line.starts_with("===")
        {
            break;
        }
        if line.starts_with("@@ ") {
            break;
        }
        if line.starts_with("--- ") || line.starts_with("+++ ") {
            break;
        }
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

        let line = lines.next().unwrap();

        if let Some(content) = line.strip_prefix('+') {
            touched_lines.push(TouchedLine {
                kind: TouchedLineKind::Added,
                line_number: new_line_num,
                content: content.to_string(),
                session_id: session_id.map(str::to_string),
            });
            new_line_num += 1;
        } else if let Some(content) = line.strip_prefix('-') {
            touched_lines.push(TouchedLine {
                kind: TouchedLineKind::Removed,
                line_number: old_line_num,
                content: content.to_string(),
                session_id: session_id.map(str::to_string),
            });
            old_line_num += 1;
        } else if line.starts_with(' ') || line.starts_with('\t') || line.is_empty() {
            old_line_num += 1;
            new_line_num += 1;
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
mod tests;
