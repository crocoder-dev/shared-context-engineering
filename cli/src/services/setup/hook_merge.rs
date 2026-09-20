use anyhow::{bail, Result};

pub const MANAGED_BLOCK_START: &str = "# >>> sce managed block (do not edit) >>>";

pub const MANAGED_BLOCK_END: &str = "# <<< sce managed block <<<";

const LEGACY_GUIDANCE_URL: &str = "https://sce.crocoder.dev/docs/getting-started#install-cli";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookMergeKind {
    Created,
    ManagedBlockReplaced,
    AppendedToForeign,
    AlreadyCurrent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HookMerge {
    pub bytes: Vec<u8>,
    pub kind: HookMergeKind,
    pub unreachable_block_advisory: bool,
}

pub fn merge_or_create_hook(
    existing: Option<&[u8]>,
    canonical: &[u8],
    hook_name: &str,
) -> Result<HookMerge> {
    let Some(existing) = existing else {
        return Ok(HookMerge {
            bytes: canonical.to_vec(),
            kind: HookMergeKind::Created,
            unreachable_block_advisory: false,
        });
    };

    let canonical_block = match locate_block(canonical) {
        BlockLocation::Balanced(start, end) => &canonical[start..end],
        _ => bail!(
            "Canonical template for hook '{hook_name}' must contain exactly one complete SCE managed block"
        ),
    };

    match locate_block(existing) {
        BlockLocation::Balanced(start, end) => {
            let existing_block = &existing[start..end];
            if existing_block == canonical_block {
                Ok(HookMerge {
                    bytes: existing.to_vec(),
                    kind: HookMergeKind::AlreadyCurrent,
                    unreachable_block_advisory: false,
                })
            } else {
                let mut bytes = Vec::with_capacity(existing.len());
                bytes.extend_from_slice(&existing[..start]);
                bytes.extend_from_slice(canonical_block);
                bytes.extend_from_slice(&existing[end..]);
                Ok(HookMerge {
                    bytes,
                    kind: HookMergeKind::ManagedBlockReplaced,
                    unreachable_block_advisory: false,
                })
            }
        }
        BlockLocation::Unbalanced => {
            bail!("Hook '{hook_name}' contains an unbalanced or partial SCE managed block marker")
        }
        BlockLocation::Absent => {
            let existing_text = String::from_utf8_lossy(existing);
            if existing_text.contains(LEGACY_GUIDANCE_URL) {
                Ok(HookMerge {
                    bytes: canonical.to_vec(),
                    kind: HookMergeKind::ManagedBlockReplaced,
                    unreachable_block_advisory: false,
                })
            } else {
                let advisory = ends_with_unreachable_control_flow(&existing_text);

                let mut bytes = existing.to_vec();
                if !bytes.ends_with(b"\n") {
                    bytes.push(b'\n');
                }
                bytes.push(b'\n');
                bytes.extend_from_slice(canonical_block);

                Ok(HookMerge {
                    bytes,
                    kind: HookMergeKind::AppendedToForeign,
                    unreachable_block_advisory: advisory,
                })
            }
        }
    }
}

enum BlockLocation {
    Absent,
    Balanced(usize, usize),
    Unbalanced,
}

fn locate_block(bytes: &[u8]) -> BlockLocation {
    let start = locate_marker_line(bytes, MANAGED_BLOCK_START);
    let end = locate_marker_line(bytes, MANAGED_BLOCK_END);
    match (start, end) {
        (None, None) => BlockLocation::Absent,
        (Some((start, _)), Some((_, end))) if start < end => BlockLocation::Balanced(start, end),
        _ => BlockLocation::Unbalanced,
    }
}

fn locate_marker_line(bytes: &[u8], marker: &str) -> Option<(usize, usize)> {
    let mut offset = 0;
    for line in bytes.split_inclusive(|&byte| byte == b'\n') {
        let content = line.strip_suffix(b"\n").unwrap_or(line);
        let content = content.strip_suffix(b"\r").unwrap_or(content);
        if content == marker.as_bytes() {
            return Some((offset, offset + line.len()));
        }
        offset += line.len();
    }
    None
}

fn ends_with_unreachable_control_flow(text: &str) -> bool {
    let Some(line) = text.lines().rev().find(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty() && !trimmed.starts_with('#')
    }) else {
        return false;
    };

    if line.starts_with(' ') || line.starts_with('\t') {
        return false;
    }

    line == "exit" || line.starts_with("exit ") || line == "exec" || line.starts_with("exec ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical_template() -> Vec<u8> {
        format!(
            "#!/bin/sh\nset -eu\n\n{MANAGED_BLOCK_START}\nsce hooks pre-commit \"$@\"\nstatus=$?\nexit \"$status\"\n{MANAGED_BLOCK_END}\n"
        )
        .into_bytes()
    }

    fn canonical_block_bytes() -> Vec<u8> {
        format!("{MANAGED_BLOCK_START}\nsce hooks pre-commit \"$@\"\nstatus=$?\nexit \"$status\"\n{MANAGED_BLOCK_END}\n").into_bytes()
    }

    #[test]
    fn creates_from_absent() {
        let canonical = canonical_template();
        let merge = merge_or_create_hook(None, &canonical, "pre-commit").unwrap();

        assert_eq!(merge.bytes, canonical);
        assert_eq!(merge.kind, HookMergeKind::Created);
        assert!(!merge.unreachable_block_advisory);
    }

    #[test]
    fn replaces_block_in_place_preserving_surrounding_foreign_content() {
        let canonical = canonical_template();
        let existing = format!(
            "#!/bin/sh\n# husky-style guard\nrun-linter\n\n{MANAGED_BLOCK_START}\nsce hooks pre-commit \"$@\" # stale\n{MANAGED_BLOCK_END}\n\n# trailer\necho done\n"
        )
        .into_bytes();

        let merge = merge_or_create_hook(Some(&existing), &canonical, "pre-commit").unwrap();

        assert_eq!(merge.kind, HookMergeKind::ManagedBlockReplaced);
        let text = String::from_utf8(merge.bytes.clone()).unwrap();
        assert!(text.starts_with("#!/bin/sh\n# husky-style guard\nrun-linter\n\n"));
        assert!(text.ends_with("\n\n# trailer\necho done\n"));
        assert!(text.contains(&String::from_utf8(canonical_block_bytes()).unwrap()));
        assert!(!merge.unreachable_block_advisory);
    }

    #[test]
    fn appends_to_foreign_hook_preserving_original_bytes_as_exact_prefix() {
        let canonical = canonical_template();
        let existing = b"#!/bin/sh\necho foreign-hook\n".to_vec();

        let merge = merge_or_create_hook(Some(&existing), &canonical, "pre-commit").unwrap();

        assert_eq!(merge.kind, HookMergeKind::AppendedToForeign);
        assert!(merge.bytes.starts_with(&existing));
        let block = canonical_block_bytes();
        let block_text = String::from_utf8(block).unwrap();
        assert!(String::from_utf8(merge.bytes)
            .unwrap()
            .ends_with(&block_text));
        assert!(!merge.unreachable_block_advisory);
    }

    #[test]
    fn legacy_pre_marker_payload_is_replaced_wholesale() {
        let canonical = canonical_template();
        let legacy = format!(
            "#!/bin/sh\nset -eu\nif ! command -v sce >/dev/null 2>&1; then\n  echo 'Install: {LEGACY_GUIDANCE_URL}'\n  exit 0\nfi\nexec sce hooks pre-commit \"$@\"\n"
        )
        .into_bytes();

        let merge = merge_or_create_hook(Some(&legacy), &canonical, "pre-commit").unwrap();

        assert_eq!(merge.kind, HookMergeKind::ManagedBlockReplaced);
        assert_eq!(merge.bytes, canonical);
        assert!(!merge.unreachable_block_advisory);
    }

    #[test]
    fn already_current_block_is_returned_unchanged() {
        let canonical = canonical_template();
        let existing = canonical.clone();

        let merge = merge_or_create_hook(Some(&existing), &canonical, "pre-commit").unwrap();

        assert_eq!(merge.kind, HookMergeKind::AlreadyCurrent);
        assert_eq!(merge.bytes, existing);
    }

    #[test]
    fn is_idempotent_across_two_merges_for_foreign_and_replace_shapes() {
        let canonical = canonical_template();

        let foreign = b"#!/bin/sh\necho foreign-hook\n".to_vec();
        let once = merge_or_create_hook(Some(&foreign), &canonical, "pre-commit").unwrap();
        let twice = merge_or_create_hook(Some(&once.bytes), &canonical, "pre-commit").unwrap();
        assert_eq!(once.bytes, twice.bytes);
        assert_eq!(twice.kind, HookMergeKind::AlreadyCurrent);

        let legacy =
            format!("#!/bin/sh\n{LEGACY_GUIDANCE_URL}\nexec sce hooks pre-commit \"$@\"\n")
                .into_bytes();
        let once = merge_or_create_hook(Some(&legacy), &canonical, "pre-commit").unwrap();
        let twice = merge_or_create_hook(Some(&once.bytes), &canonical, "pre-commit").unwrap();
        assert_eq!(once.bytes, twice.bytes);
        assert_eq!(twice.kind, HookMergeKind::AlreadyCurrent);
    }

    #[test]
    fn unbalanced_marker_fails_with_deterministic_error_naming_the_hook() {
        let canonical = canonical_template();
        let existing =
            format!("#!/bin/sh\n{MANAGED_BLOCK_START}\necho no-closing-marker\n").into_bytes();

        let error = merge_or_create_hook(Some(&existing), &canonical, "commit-msg").unwrap_err();

        assert!(error.to_string().contains("commit-msg"));
        assert!(error.to_string().contains("unbalanced"));
    }

    #[test]
    fn partial_end_only_marker_fails_with_deterministic_error_naming_the_hook() {
        let canonical = canonical_template();
        let existing =
            format!("#!/bin/sh\necho no-opening-marker\n{MANAGED_BLOCK_END}\n").into_bytes();

        let error = merge_or_create_hook(Some(&existing), &canonical, "post-commit").unwrap_err();

        assert!(error.to_string().contains("post-commit"));
    }

    #[test]
    fn advisory_fires_on_trailing_zero_indent_exec() {
        let canonical = canonical_template();
        let existing = b"#!/bin/sh\nexec some-tool \"$@\"\n".to_vec();

        let merge = merge_or_create_hook(Some(&existing), &canonical, "pre-commit").unwrap();

        assert!(merge.unreachable_block_advisory);
    }

    #[test]
    fn advisory_fires_on_trailing_zero_indent_exit() {
        let canonical = canonical_template();
        let existing = b"#!/bin/sh\nrun-linter\nexit 1\n".to_vec();

        let merge = merge_or_create_hook(Some(&existing), &canonical, "pre-commit").unwrap();

        assert!(merge.unreachable_block_advisory);
    }

    #[test]
    fn advisory_does_not_fire_on_indented_exec() {
        let canonical = canonical_template();
        let existing =
            b"#!/bin/sh\nif [ -f .foo ]; then\n  exec some-tool \"$@\"\nfi\necho done\n".to_vec();

        let merge = merge_or_create_hook(Some(&existing), &canonical, "pre-commit").unwrap();

        assert!(!merge.unreachable_block_advisory);
    }

    #[test]
    fn advisory_does_not_fire_on_ordinary_final_command() {
        let canonical = canonical_template();
        let existing = b"#!/bin/sh\necho done\n".to_vec();

        let merge = merge_or_create_hook(Some(&existing), &canonical, "pre-commit").unwrap();

        assert!(!merge.unreachable_block_advisory);
    }

    #[test]
    fn advisory_ignores_trailing_comment_and_blank_lines() {
        let canonical = canonical_template();
        let existing = b"#!/bin/sh\nexec some-tool \"$@\"\n\n# trailing comment\n".to_vec();

        let merge = merge_or_create_hook(Some(&existing), &canonical, "pre-commit").unwrap();

        assert!(merge.unreachable_block_advisory);
    }
}
