//! Resolves Codex `apply_patch` paths against the event cwd and the real Git
//! repository root.
//!
//! Codex invokes command hooks with the event's cwd, while the SCE dispatcher
//! may be launched from any directory in the checkout. This module keeps that
//! distinction explicit and only emits repository-relative, UTF-8 paths that
//! can be represented losslessly in SCE's patch format.

use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

use super::parser::{CodexFileOperation, CodexPatch};

/// Resolves every path that can contribute Codex patch evidence in `patch`.
///
/// The Git root and event cwd are validated once per event. Source and move
/// destination paths are then resolved independently from the event cwd.
pub(crate) fn resolve_codex_patch_paths(
    repository_root: &Path,
    event_cwd: &str,
    patch: &mut CodexPatch,
) -> Result<()> {
    let git_root = resolve_git_root(repository_root)?;
    let event_cwd = resolve_event_cwd(&git_root, event_cwd)?;

    for operation in &mut patch.operations {
        match operation {
            CodexFileOperation::Add { path, .. } | CodexFileOperation::Delete { path } => {
                *path = resolve_path_from_cwd(&git_root, &event_cwd, path)?;
            }
            CodexFileOperation::Update {
                old_path, new_path, ..
            } => {
                *old_path = resolve_path_from_cwd(&git_root, &event_cwd, old_path)?;
                if let Some(new_path) = new_path {
                    *new_path = resolve_path_from_cwd(&git_root, &event_cwd, new_path)?;
                }
            }
        }
    }

    Ok(())
}

/// Resolves one Codex path to a repository-relative path.
///
/// This public seam intentionally performs the same Git-root and cwd checks
/// as the event-level resolver, making the path contract independently
/// testable without invoking the hook dispatcher or opening the Agent Trace
/// database.
#[allow(dead_code)]
pub(crate) fn resolve_codex_patch_path(
    repository_root: &Path,
    event_cwd: &str,
    codex_path: &str,
) -> Result<String> {
    let git_root = resolve_git_root(repository_root)?;
    let event_cwd = resolve_event_cwd(&git_root, event_cwd)?;
    resolve_path_from_cwd(&git_root, &event_cwd, codex_path)
}

fn resolve_git_root(repository_root: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(repository_root)
        .output()
        .with_context(|| {
            format!(
                "failed to discover Git root from '{}'.",
                repository_root.display()
            )
        })?;

    if !output.status.success() {
        bail!(
            "git rev-parse --show-toplevel failed from '{}': {}",
            repository_root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let reported_root = String::from_utf8(output.stdout)
        .context("git rev-parse --show-toplevel emitted invalid UTF-8")?
        .trim()
        .to_string();
    if reported_root.is_empty() || reported_root.contains('\0') {
        bail!("git rev-parse --show-toplevel returned an invalid root.");
    }

    let reported_root = PathBuf::from(reported_root);
    let root_path = if reported_root.is_absolute() {
        reported_root
    } else {
        repository_root.join(reported_root)
    };
    let root = std::fs::canonicalize(&root_path).with_context(|| {
        format!(
            "failed to canonicalize the Git root '{}'.",
            root_path.display()
        )
    })?;
    if !root.is_dir() {
        bail!("resolved Git root '{}' is not a directory.", root.display());
    }

    Ok(root)
}

fn resolve_event_cwd(git_root: &Path, event_cwd: &str) -> Result<PathBuf> {
    if event_cwd.trim().is_empty() || event_cwd.contains('\0') {
        bail!("Codex hook event cwd is missing or malformed.");
    }

    let cwd = Path::new(event_cwd);
    if !cwd.is_absolute() {
        bail!("Codex hook event cwd must be an absolute path.");
    }

    let lexical_cwd = normalize_absolute_path(cwd)?;
    let canonical_cwd = std::fs::canonicalize(&lexical_cwd).with_context(|| {
        format!(
            "failed to resolve Codex hook event cwd '{}'.",
            cwd.display()
        )
    })?;
    if !canonical_cwd.is_dir() {
        bail!(
            "Codex hook event cwd '{}' is not a directory.",
            cwd.display()
        );
    }
    if !canonical_cwd.starts_with(git_root) {
        bail!(
            "Codex hook event cwd '{}' is outside Git repository '{}'.",
            cwd.display(),
            git_root.display()
        );
    }

    Ok(lexical_cwd)
}

fn resolve_path_from_cwd(git_root: &Path, event_cwd: &Path, codex_path: &str) -> Result<String> {
    if codex_path.trim().is_empty() || codex_path.contains('\0') {
        bail!("Codex apply_patch path is empty or malformed.");
    }

    let codex_path = Path::new(codex_path);
    let candidate = if codex_path.is_absolute() {
        codex_path.to_path_buf()
    } else {
        event_cwd.join(codex_path)
    };
    let lexical_target = normalize_absolute_path(&candidate)?;
    let resolved = resolve_candidate_inside_repository(git_root, &lexical_target)?;
    path_to_utf8_slash_path(
        resolved
            .strip_prefix(git_root)
            .map_err(|_| anyhow!("repository-relative path is outside the Git root."))?,
    )
}

/// Resolve a lexically normalized absolute path while preserving filesystem
/// semantics for existing components. Lexical normalization must happen before
/// this function so a symlink component removed by `..` is never inspected.
fn resolve_candidate_inside_repository(git_root: &Path, candidate: &Path) -> Result<PathBuf> {
    let (existing, suffix) = nearest_existing_prefix(candidate)?;
    let canonical_existing = canonicalize_inside_repository(git_root, &existing, candidate)?;
    let resolved = append_path_lexically(&canonical_existing, &suffix)?;
    if !resolved.starts_with(git_root) {
        bail!(
            "Codex apply_patch path '{}' resolves outside Git repository '{}'.",
            candidate.display(),
            git_root.display()
        );
    }
    Ok(resolved)
}

/// Normalize an absolute path using Codex's lexical `PathUri::join` semantics:
/// `.` is removed, `..` removes the preceding lexical component, and parent
/// traversal at the filesystem root is clamped rather than treated as an error.
fn normalize_absolute_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("Codex path must be absolute after joining with the event cwd.");
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(std::path::MAIN_SEPARATOR.to_string()),
            Component::CurDir => {}
            Component::Normal(value) => normalized.push(value),
            Component::ParentDir => {
                let _ = normalized.pop();
            }
        }
    }

    Ok(normalized)
}

fn nearest_existing_prefix(path: &Path) -> Result<(PathBuf, PathBuf)> {
    let mut existing = path.to_path_buf();
    loop {
        match std::fs::symlink_metadata(&existing) {
            Ok(_) => {
                let suffix = path
                    .strip_prefix(&existing)
                    .map_err(|_| anyhow!("Codex apply_patch path has an invalid prefix."))?
                    .to_path_buf();
                return Ok((existing, suffix));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                existing = existing.parent().map(Path::to_path_buf).ok_or_else(|| {
                    anyhow!(
                        "Codex apply_patch path '{}' has no existing repository prefix.",
                        path.display()
                    )
                })?;
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to inspect Codex apply_patch path prefix '{}'.",
                        existing.display()
                    )
                });
            }
        }
    }
}

fn canonicalize_inside_repository(
    git_root: &Path,
    existing: &Path,
    candidate: &Path,
) -> Result<PathBuf> {
    let resolved = std::fs::canonicalize(existing).with_context(|| {
        format!(
            "failed to resolve existing Codex apply_patch path prefix '{}'.",
            existing.display()
        )
    })?;
    if !resolved.starts_with(git_root) {
        bail!(
            "Codex apply_patch path '{}' resolves outside Git repository '{}'.",
            candidate.display(),
            git_root.display()
        );
    }
    Ok(resolved)
}

fn append_path_lexically(base: &Path, suffix: &Path) -> Result<PathBuf> {
    let mut result = base.to_path_buf();
    for component in suffix.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => result.push(value),
            Component::ParentDir => {
                if !result.pop() {
                    bail!("Codex apply_patch path traverses above the filesystem root.");
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!("Codex apply_patch path has an invalid suffix.");
            }
        }
    }
    Ok(result)
}

/// Convert a canonical or lexically resolved path into the slash-separated
/// UTF-8 form used by SCE patch text.
fn path_to_utf8_slash_path(path: &Path) -> Result<String> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => components.push(
                value
                    .to_str()
                    .ok_or_else(|| anyhow!("repository-relative path is not valid UTF-8"))?,
            ),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("repository-relative path is ambiguous or unsafe.");
            }
        }
    }

    if components.is_empty() {
        bail!("repository-relative path is empty.");
    }
    Ok(components.join("/"))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn temp_repo(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sce-codex-path-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temporary repository should be created");
        let output = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&root)
            .output()
            .expect("git init should run");
        assert!(output.status.success(), "git init failed: {output:?}");
        root
    }

    fn remove_repo(root: &Path) {
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resolves_a_root_cwd_path_to_repository_relative_form() {
        let root = temp_repo("root");
        let src = root.join("src");
        fs::create_dir(&src).expect("src directory should be created");
        let result = resolve_codex_patch_path(&root, &root.to_string_lossy(), "src/lib.rs")
            .expect("root cwd path should resolve");
        assert_eq!(result, "src/lib.rs");
        remove_repo(&root);
    }

    #[test]
    fn resolves_nested_cwd_parent_traversal_and_dot_components() {
        let root = temp_repo("nested");
        let cwd = root.join("src").join("lib");
        fs::create_dir_all(&cwd).expect("nested cwd should be created");

        let result = resolve_codex_patch_path(&root, &cwd.to_string_lossy(), "./../lib.rs")
            .expect("valid parent traversal should resolve");
        assert_eq!(result, "src/lib.rs");

        let result = resolve_codex_patch_path(&root, &cwd.to_string_lossy(), "../../root.rs")
            .expect("multiple valid parent traversals should resolve");
        assert_eq!(result, "root.rs");

        let result = resolve_codex_patch_path(&root, &cwd.to_string_lossy(), "./nested/file.rs")
            .expect("nested relative path should resolve");
        assert_eq!(result, "src/lib/nested/file.rs");
        remove_repo(&root);
    }

    #[test]
    fn accepts_absolute_inside_worktree_paths() {
        let root = temp_repo("absolute-inside");
        let cwd = root.join("src");
        fs::create_dir(&cwd).expect("src directory should be created");
        let target = cwd.join("../lib.rs");

        let result =
            resolve_codex_patch_path(&root, &cwd.to_string_lossy(), &target.to_string_lossy())
                .expect("absolute path inside the worktree should resolve");
        assert_eq!(result, "lib.rs");
        remove_repo(&root);
    }

    #[test]
    fn accepts_missing_add_targets_and_normalizes_their_parent_components() {
        let root = temp_repo("missing-target");
        let cwd = root.join("src").join("lib");
        fs::create_dir_all(&cwd).expect("nested cwd should be created");

        let result =
            resolve_codex_patch_path(&root, &cwd.to_string_lossy(), "../generated/new file.rs")
                .expect("missing add target should resolve");
        assert_eq!(result, "src/generated/new file.rs");

        let result = resolve_codex_patch_path(&root, &cwd.to_string_lossy(), "../../new.rs")
            .expect("missing target after parent traversal should resolve");
        assert_eq!(result, "new.rs");
        remove_repo(&root);
    }

    #[test]
    fn resolves_move_source_and_destination_independently() {
        let root = temp_repo("move");
        let cwd = root.join("src");
        fs::create_dir(&cwd).expect("src directory should be created");
        let source = resolve_codex_patch_path(&root, &cwd.to_string_lossy(), "old.rs")
            .expect("move source should resolve");
        let destination = resolve_codex_patch_path(&root, &cwd.to_string_lossy(), "new.rs")
            .expect("move destination should resolve");
        assert_eq!(source, "src/old.rs");
        assert_eq!(destination, "src/new.rs");
        remove_repo(&root);
    }

    #[test]
    fn rejects_repository_escape_absolute_and_malformed_paths() {
        let root = temp_repo("invalid-path");
        let outside = root
            .parent()
            .expect("temporary root should have a parent")
            .to_path_buf();

        for path in ["../outside.txt", "/etc/passwd", "./..", "", "bad\0path"] {
            let error = resolve_codex_patch_path(&root, &root.to_string_lossy(), path)
                .expect_err("unsafe path should be rejected");
            assert!(!error.to_string().is_empty());
        }

        let error = resolve_codex_patch_path(&root, &outside.to_string_lossy(), "file.rs")
            .expect_err("outside cwd should be rejected");
        assert!(error.to_string().contains("outside Git repository"));

        let error = resolve_codex_patch_path(&root, "", "file.rs")
            .expect_err("missing cwd should be rejected");
        assert!(error.to_string().contains("missing or malformed"));

        let error = resolve_codex_patch_path(&root, "relative/cwd", "file.rs")
            .expect_err("relative cwd should be rejected");
        assert!(error.to_string().contains("absolute"));
        remove_repo(&root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_existing_and_missing_paths_that_escape_through_symlinks() {
        use std::os::unix::fs::symlink;

        let root = temp_repo("symlink-escape");
        let outside = root
            .parent()
            .expect("temporary root should have a parent")
            .join(format!("sce-codex-path-outside-{}", std::process::id()));
        fs::create_dir_all(&outside).expect("outside directory should be created");
        fs::write(outside.join("existing.rs"), "outside").expect("outside file should be created");

        let alias = root.join("alias");
        let existing_link = root.join("existing-link");
        let missing_link = root.join("missing-link");
        symlink(&outside, &alias).expect("alias escape symlink should be created");
        symlink(&outside, &existing_link).expect("existing escape symlink should be created");
        symlink(&outside, &missing_link).expect("missing escape symlink should be created");

        let eliminated =
            resolve_codex_patch_path(&root, &root.to_string_lossy(), "alias/../foo.rs")
                .expect("a symlink removed by lexical parent traversal must not be inspected");
        assert_eq!(eliminated, "foo.rs");
        assert!(
            resolve_codex_patch_path(&root, &root.to_string_lossy(), "alias/foo.rs").is_err(),
            "an actually traversed escape symlink must be rejected"
        );
        assert!(resolve_codex_patch_path(
            &root,
            &root.to_string_lossy(),
            "existing-link/existing.rs"
        )
        .is_err());
        assert!(
            resolve_codex_patch_path(&root, &root.to_string_lossy(), "missing-link/new.rs")
                .is_err()
        );

        remove_repo(&root);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn clamps_excessive_parent_traversal_at_filesystem_root() {
        let root = temp_repo("root-clamp");
        let cwd = root.join("src");
        fs::create_dir(&cwd).expect("src directory should be created");
        let parent_path = root
            .parent()
            .expect("temporary repository should have a parent")
            .strip_prefix(Path::new("/"))
            .expect("temporary repository parent should be absolute")
            .to_string_lossy();
        let root_name = root
            .file_name()
            .expect("temporary repository should have a name")
            .to_str()
            .expect("temporary repository name should be UTF-8");
        // Overshoot past the filesystem root by a wide margin: `TMPDIR` depth
        // varies by platform (e.g. macOS's `/var/folders/xx/yyyy/T/` nests
        // deeper than Linux's `/tmp/`), so a fixed `..` count that clamps on
        // one platform can undershoot the root on another.
        let cwd_depth = cwd.components().count();
        let excess_traversal = "../".repeat(cwd_depth + 8);
        let path = format!("{excess_traversal}{parent_path}/{root_name}/clamped.rs");

        let result = resolve_codex_patch_path(&root, &cwd.to_string_lossy(), &path)
            .expect("excessive parent traversal should clamp at filesystem root");
        assert_eq!(result, "clamped.rs");
        remove_repo(&root);
    }

    #[test]
    fn preserves_spaces_in_repository_relative_paths() {
        let root = temp_repo("spaces");
        let cwd = root.join("folder with spaces");
        fs::create_dir(&cwd).expect("spaced cwd should be created");
        let result = resolve_codex_patch_path(&root, &cwd.to_string_lossy(), "file with spaces.rs")
            .expect("spaced paths should resolve");
        assert_eq!(result, "folder with spaces/file with spaces.rs");
        remove_repo(&root);
    }

    #[test]
    fn resolves_all_operation_paths_in_one_event() {
        let root = temp_repo("operations");
        let cwd = root.join("src");
        fs::create_dir(&cwd).expect("src directory should be created");
        let mut patch = CodexPatch {
            operations: vec![
                CodexFileOperation::Add {
                    path: "new.rs".to_string(),
                    lines: vec!["new".to_string()],
                },
                CodexFileOperation::Update {
                    old_path: "old.rs".to_string(),
                    new_path: Some("moved.rs".to_string()),
                    hunks: Vec::new(),
                },
                CodexFileOperation::Delete {
                    path: "gone.rs".to_string(),
                },
            ],
        };

        resolve_codex_patch_paths(&root, &cwd.to_string_lossy(), &mut patch)
            .expect("all operation paths should resolve");
        assert_eq!(
            patch.operations[0],
            CodexFileOperation::Add {
                path: "src/new.rs".to_string(),
                lines: vec!["new".to_string()],
            }
        );
        assert_eq!(
            patch.operations[1],
            CodexFileOperation::Update {
                old_path: "src/old.rs".to_string(),
                new_path: Some("src/moved.rs".to_string()),
                hunks: Vec::new(),
            }
        );
        assert_eq!(
            patch.operations[2],
            CodexFileOperation::Delete {
                path: "src/gone.rs".to_string(),
            }
        );
        remove_repo(&root);
    }
}
