//! Structured editor-hook patch derivation.
//!
//! This module converts supported structured tool payloads into the CLI's
//! canonical [`ParsedPatch`](crate::services::patch::ParsedPatch) domain model
//! without going through rendered unified-diff text. The first supported source
//! is Claude `PostToolUse` payloads for `Write` creates and `Edit` structured
//! patches.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::services::patch::{
    FileChangeKind, ParsedPatch, PatchFileChange, PatchHunk, TouchedLine, TouchedLineKind,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaudeStructuredPatch {
    pub session_id: String,
    pub patch: ParsedPatch,
    pub time: u64,
    pub tool_name: String,
    pub tool_version: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaudeStructuredPatchDerivationResult {
    Derived(ClaudeStructuredPatch),
    Skipped(ClaudeStructuredPatchSkipReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaudeStructuredPatchSkipReason {
    UnsupportedEvent,
    EventWithoutDiffTrace,
    InvalidPayload,
    EventNameMismatch,
    UnsupportedTool,
    UnsupportedWritePayload,
    MissingFilePath,
    MissingFileContent,
    UnsupportedEditPayload,
    MissingSessionId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PatchBuildResult {
    Built(ParsedPatch),
    Skipped(ClaudeStructuredPatchSkipReason),
}

const CLAUDE_TOOL_NAME: &str = "claude";

pub fn derive_claude_structured_patch(
    event_name: &str,
    payload: &Value,
    time: u64,
    tool_version: Option<&str>,
) -> ClaudeStructuredPatchDerivationResult {
    match event_name {
        "SessionStart" | "UserPromptSubmit" | "PostToolUse" | "Stop" => {}
        _ => return skipped(ClaudeStructuredPatchSkipReason::UnsupportedEvent),
    }

    if event_name != "PostToolUse" {
        return skipped(ClaudeStructuredPatchSkipReason::EventWithoutDiffTrace);
    }

    let Some(payload_object) = payload.as_object() else {
        return skipped(ClaudeStructuredPatchSkipReason::InvalidPayload);
    };

    if let Some(payload_event_name) = string_field(payload_object, &["hook_event_name"]) {
        if payload_event_name != event_name {
            return skipped(ClaudeStructuredPatchSkipReason::EventNameMismatch);
        }
    }

    let patch = match build_claude_post_tool_use_patch(payload_object) {
        PatchBuildResult::Built(patch) => patch,
        PatchBuildResult::Skipped(reason) => return skipped(reason),
    };

    let Some(session_id) = string_field(payload_object, &["session_id", "sessionID"]) else {
        return skipped(ClaudeStructuredPatchSkipReason::MissingSessionId);
    };

    ClaudeStructuredPatchDerivationResult::Derived(ClaudeStructuredPatch {
        session_id,
        patch,
        time,
        tool_name: CLAUDE_TOOL_NAME.to_string(),
        tool_version: extract_claude_tool_version(tool_version, payload_object),
    })
}

fn build_claude_post_tool_use_patch(payload: &Map<String, Value>) -> PatchBuildResult {
    match string_field(payload, &["tool_name"]).as_deref() {
        Some("Write") => build_write_create_patch(payload),
        Some("Edit") => build_edit_structured_patch(payload),
        _ => skipped_build(ClaudeStructuredPatchSkipReason::UnsupportedTool),
    }
}

fn build_write_create_patch(payload: &Map<String, Value>) -> PatchBuildResult {
    let Some(tool_input) = object_field(payload, "tool_input") else {
        return skipped_build(ClaudeStructuredPatchSkipReason::UnsupportedWritePayload);
    };
    let Some(tool_response) = object_field(payload, "tool_response") else {
        return skipped_build(ClaudeStructuredPatchSkipReason::UnsupportedWritePayload);
    };

    if value_field(tool_response, &["originalFile", "original_file"]) != Some(&Value::Null) {
        return skipped_build(ClaudeStructuredPatchSkipReason::UnsupportedWritePayload);
    }

    let file_path = normalize_patch_path(
        string_field(tool_input, &["file_path", "filePath"])
            .or_else(|| string_field(tool_response, &["file_path", "filePath"]))
            .as_deref(),
        string_field(payload, &["cwd"]).as_deref(),
    );
    let Some(file_path) = file_path else {
        return skipped_build(ClaudeStructuredPatchSkipReason::MissingFilePath);
    };

    let Some(content) = string_value_field(tool_input, &["content", "newFile", "new_file"]) else {
        return skipped_build(ClaudeStructuredPatchSkipReason::MissingFileContent);
    };

    PatchBuildResult::Built(write_create_patch(file_path, &content))
}

fn build_edit_structured_patch(payload: &Map<String, Value>) -> PatchBuildResult {
    let Some(tool_input) = object_field(payload, "tool_input") else {
        return skipped_build(ClaudeStructuredPatchSkipReason::UnsupportedEditPayload);
    };
    let Some(tool_response) = object_field(payload, "tool_response") else {
        return skipped_build(ClaudeStructuredPatchSkipReason::UnsupportedEditPayload);
    };

    let Some(structured_patch) =
        value_field(tool_response, &["structuredPatch", "structured_patch"])
    else {
        return skipped_build(ClaudeStructuredPatchSkipReason::UnsupportedEditPayload);
    };
    if structured_patch.is_null() {
        return skipped_build(ClaudeStructuredPatchSkipReason::UnsupportedEditPayload);
    }

    let file_path = normalize_patch_path(
        string_field(tool_input, &["file_path", "filePath"])
            .or_else(|| {
                structured_patch
                    .as_object()
                    .and_then(|patch| string_field(patch, &["file_path", "filePath", "path"]))
            })
            .as_deref(),
        string_field(payload, &["cwd"]).as_deref(),
    );
    let Some(file_path) = file_path else {
        return skipped_build(ClaudeStructuredPatchSkipReason::MissingFilePath);
    };

    let hunks: Vec<PatchHunk> = structured_patch_hunks(structured_patch)
        .into_iter()
        .filter_map(parse_structured_patch_hunk)
        .collect();

    if hunks.is_empty() {
        return skipped_build(ClaudeStructuredPatchSkipReason::UnsupportedEditPayload);
    }

    PatchBuildResult::Built(ParsedPatch {
        files: vec![PatchFileChange {
            old_path: file_path.clone(),
            new_path: file_path,
            kind: FileChangeKind::Modified,
            hunks,
        }],
    })
}

fn write_create_patch(file_path: String, content: &str) -> ParsedPatch {
    let content_lines = split_file_content(content);
    let lines = content_lines
        .into_iter()
        .enumerate()
        .map(|(index, content)| TouchedLine {
            kind: TouchedLineKind::Added,
            line_number: u64::try_from(index + 1).expect("line index should fit in u64"),
            content,
            session_id: None,
        })
        .collect::<Vec<_>>();
    let new_count = u64::try_from(lines.len()).expect("line count should fit in u64");

    ParsedPatch {
        files: vec![PatchFileChange {
            old_path: String::new(),
            new_path: file_path,
            kind: FileChangeKind::Added,
            hunks: (!lines.is_empty())
                .then_some(PatchHunk {
                    old_start: 0,
                    old_count: 0,
                    new_start: 1,
                    new_count,
                    model_id: None,
                    lines,
                })
                .into_iter()
                .collect(),
        }],
    }
}

fn parse_structured_patch_hunk(hunk_value: &Value) -> Option<PatchHunk> {
    let hunk = hunk_value.as_object()?;
    let raw_lines = array_field(hunk, &["lines", "body", "changes"])?;
    let old_start = numeric_field(hunk, &["oldStart", "old_start", "oldLine", "old_line"])?;
    let new_start = numeric_field(hunk, &["newStart", "new_start", "newLine", "new_line"])?;
    let old_count = numeric_field(hunk, &["oldCount", "old_count", "oldLines", "old_lines"])
        .unwrap_or_else(|| count_old_hunk_lines(raw_lines));
    let new_count = numeric_field(hunk, &["newCount", "new_count", "newLines", "new_lines"])
        .unwrap_or_else(|| count_new_hunk_lines(raw_lines));

    let mut old_line_number = old_start;
    let mut new_line_number = new_start;
    let mut touched_lines = Vec::new();

    for raw_line in raw_lines {
        match structured_patch_line(raw_line) {
            Some(StructuredPatchLine::Context) => {
                old_line_number += 1;
                new_line_number += 1;
            }
            Some(StructuredPatchLine::Added(content)) => {
                touched_lines.push(TouchedLine {
                    kind: TouchedLineKind::Added,
                    line_number: new_line_number,
                    content,
                    session_id: None,
                });
                new_line_number += 1;
            }
            Some(StructuredPatchLine::Removed(content)) => {
                touched_lines.push(TouchedLine {
                    kind: TouchedLineKind::Removed,
                    line_number: old_line_number,
                    content,
                    session_id: None,
                });
                old_line_number += 1;
            }
            Some(StructuredPatchLine::NoNewlineMarker) | None => {}
        }
    }

    (!touched_lines.is_empty()).then_some(PatchHunk {
        old_start,
        old_count,
        new_start,
        new_count,
        model_id: None,
        lines: touched_lines,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum StructuredPatchLine {
    Context,
    Added(String),
    Removed(String),
    NoNewlineMarker,
}

fn structured_patch_line(line_value: &Value) -> Option<StructuredPatchLine> {
    if let Some(line) = line_value.as_str() {
        if line.starts_with('\\') {
            return Some(StructuredPatchLine::NoNewlineMarker);
        }
        if let Some(content) = line.strip_prefix('+') {
            return Some(StructuredPatchLine::Added(content.to_string()));
        }
        if let Some(content) = line.strip_prefix('-') {
            return Some(StructuredPatchLine::Removed(content.to_string()));
        }
        return Some(StructuredPatchLine::Context);
    }

    let line = line_value.as_object()?;
    let content = string_value_field(line, &["content", "text", "value"])?;
    match string_field(line, &["kind", "type", "operation", "change"]).as_deref() {
        Some("context" | "unchanged" | "equal" | " ") => Some(StructuredPatchLine::Context),
        Some("added" | "add" | "insert" | "+") => Some(StructuredPatchLine::Added(content)),
        Some("removed" | "remove" | "delete" | "-") => Some(StructuredPatchLine::Removed(content)),
        _ => None,
    }
}

fn structured_patch_hunks(structured_patch: &Value) -> Vec<&Value> {
    if let Some(hunks) = structured_patch.as_array() {
        return hunks.iter().collect();
    }

    let Some(patch) = structured_patch.as_object() else {
        return Vec::new();
    };

    if let Some(hunks) = array_field(patch, &["hunks", "changes"]) {
        return hunks.iter().collect();
    }

    if array_field(patch, &["lines", "body"]).is_some() {
        return vec![structured_patch];
    }

    Vec::new()
}

fn split_file_content(content: &str) -> Vec<String> {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    if normalized.is_empty() {
        return Vec::new();
    }

    normalized
        .strip_suffix('\n')
        .unwrap_or(&normalized)
        .split('\n')
        .map(ToString::to_string)
        .collect()
}

fn count_old_hunk_lines(lines: &[Value]) -> u64 {
    count_lines(lines, |line| !matches!(line, StructuredPatchLine::Added(_)))
}

fn count_new_hunk_lines(lines: &[Value]) -> u64 {
    count_lines(lines, |line| {
        !matches!(line, StructuredPatchLine::Removed(_))
    })
}

fn count_lines(lines: &[Value], include: fn(&StructuredPatchLine) -> bool) -> u64 {
    let count = lines
        .iter()
        .filter_map(structured_patch_line)
        .filter(|line| !matches!(line, StructuredPatchLine::NoNewlineMarker) && include(line))
        .count();

    u64::try_from(count).expect("line count should fit in u64")
}

fn extract_claude_tool_version(
    input_tool_version: Option<&str>,
    payload: &Map<String, Value>,
) -> Option<String> {
    if let Some(version) = input_tool_version
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(version.to_string());
    }

    for key in ["tool_version", "claude_version", "version"] {
        match normalize_optional_version(payload.get(key)) {
            VersionField::Present(version) => return version,
            VersionField::Missing => {}
        }
    }

    None
}

enum VersionField {
    Missing,
    Present(Option<String>),
}

fn normalize_optional_version(value: Option<&Value>) -> VersionField {
    match value {
        None => VersionField::Missing,
        Some(Value::String(version)) => {
            let normalized = version.trim();
            VersionField::Present((!normalized.is_empty()).then(|| normalized.to_string()))
        }
        Some(Value::Null | _) => VersionField::Present(None),
    }
}

fn normalize_patch_path(file_path: Option<&str>, cwd: Option<&str>) -> Option<String> {
    let mut normalized = file_path?.trim().to_string();
    if normalized.is_empty() {
        return None;
    }

    if let Some(cwd) = cwd.map(str::trim).filter(|value| !value.is_empty()) {
        let path = Path::new(&normalized);
        let cwd_path = Path::new(cwd);
        if path.is_absolute() && cwd_path.is_absolute() {
            if let Ok(relative_path) = path.strip_prefix(cwd_path) {
                if !relative_path.as_os_str().is_empty() {
                    normalized = path_to_forward_slashes(relative_path);
                }
            }
        }
    }

    normalized = normalized.replace('\\', "/");
    while let Some(stripped) = normalized.strip_prefix("./") {
        normalized = stripped.to_string();
    }

    if normalized.is_empty() || normalized == "." {
        None
    } else {
        Some(normalized)
    }
}

fn path_to_forward_slashes(path: &Path) -> String {
    path.components()
        .collect::<PathBuf>()
        .to_string_lossy()
        .replace('\\', "/")
}

fn object_field<'a>(payload: &'a Map<String, Value>, key: &str) -> Option<&'a Map<String, Value>> {
    payload.get(key)?.as_object()
}

fn string_field(payload: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let value = payload.get(*key)?.as_str()?.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn string_value_field(payload: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| payload.get(*key)?.as_str().map(ToString::to_string))
}

fn numeric_field(payload: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| payload.get(*key)?.as_u64())
}

fn array_field<'a>(payload: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Vec<Value>> {
    keys.iter().find_map(|key| payload.get(*key)?.as_array())
}

fn value_field<'a>(payload: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| payload.get(*key))
}

fn skipped(reason: ClaudeStructuredPatchSkipReason) -> ClaudeStructuredPatchDerivationResult {
    ClaudeStructuredPatchDerivationResult::Skipped(reason)
}

fn skipped_build(reason: ClaudeStructuredPatchSkipReason) -> PatchBuildResult {
    PatchBuildResult::Skipped(reason)
}

#[cfg(test)]
#[path = "structured_patch/tests.rs"]
mod tests;
