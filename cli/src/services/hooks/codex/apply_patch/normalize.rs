use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use super::{CodexFileOperation, CodexHunk, CodexHunkLine, CodexPatch};

const PATCH_INDEX_SEPARATOR: &str =
    "===================================================================";
const CODEX_SYNTHETIC_LINE_ID_DOMAIN: &[u8] = b"sce-codex-apply-patch-line-id-v1\0";
const SYNTHETIC_EVENT_RANGE_SIZE: u64 = 1 << 31;
const SYNTHETIC_BASE_OFFSET: u64 = 2;

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
