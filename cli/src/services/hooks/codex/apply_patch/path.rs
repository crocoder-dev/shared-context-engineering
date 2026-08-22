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

    let cwd = std::fs::canonicalize(cwd).with_context(|| {
        format!(
            "failed to resolve Codex hook event cwd '{}'.",
            cwd.display()
        )
    })?;
    if !cwd.is_dir() {
        bail!(
            "Codex hook event cwd '{}' is not a directory.",
            cwd.display()
        );
    }
    if !cwd.starts_with(git_root) {
        bail!(
            "Codex hook event cwd '{}' is outside Git repository '{}'.",
            cwd.display(),
            git_root.display()
        );
    }

    Ok(cwd)
}

fn resolve_path_from_cwd(git_root: &Path, event_cwd: &Path, codex_path: &str) -> Result<String> {
    let relative_path = normalize_codex_relative_path(codex_path)?;
    let candidate = event_cwd.join(&relative_path);
    ensure_existing_prefix_is_inside_repository(git_root, &candidate)?;

    let cwd_relative = event_cwd
        .strip_prefix(git_root)
        .map_err(|_| anyhow!("Codex hook event cwd cannot be represented relative to Git root."))?;
    let repository_relative = cwd_relative.join(relative_path);
    path_to_utf8_slash_path(&repository_relative)
}

fn normalize_codex_relative_path(codex_path: &str) -> Result<PathBuf> {
    if codex_path.trim().is_empty() || codex_path.contains('\0') {
        bail!("Codex apply_patch path is empty or malformed.");
    }

    let path = Path::new(codex_path);
    if path.is_absolute() {
        bail!("Codex apply_patch path '{codex_path}' must not be absolute.");
    }

    let mut normalized = PathBuf::new();
    let mut has_normal_component = false;
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => {
                if value.to_str().is_none() {
                    bail!("Codex apply_patch path '{codex_path}' is not valid UTF-8.");
                }
                normalized.push(value);
                has_normal_component = true;
            }
            Component::ParentDir => {
                bail!("Codex apply_patch path '{codex_path}' must not escape the event cwd.");
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!("Codex apply_patch path '{codex_path}' is not relative.");
            }
        }
    }

    if !has_normal_component {
        bail!("Codex apply_patch path '{codex_path}' has no file component.");
    }

    Ok(normalized)
}

/// Check the nearest existing path prefix so a lexical path through a
/// symlink cannot silently map evidence outside the real repository. Missing
/// Add File targets are allowed; their existing parent prefix is checked.
fn ensure_existing_prefix_is_inside_repository(git_root: &Path, candidate: &Path) -> Result<()> {
    let mut existing = candidate;
    loop {
        match std::fs::symlink_metadata(existing) {
            Ok(_) => {
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
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                existing = existing.parent().ok_or_else(|| {
                    anyhow!(
                        "Codex apply_patch path '{}' has no existing repository prefix.",
                        candidate.display()
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
    fn resolves_nested_cwd_and_dot_components() {
        let root = temp_repo("nested");
        let cwd = root.join("src").join("lib");
        fs::create_dir_all(&cwd).expect("nested cwd should be created");
        let result =
            resolve_codex_patch_path(&root, &cwd.join(".").to_string_lossy(), "./../lib.rs")
                .expect_err("traversal must be rejected even when dot components are present");
        assert!(result.to_string().contains("must not escape"));

        let result = resolve_codex_patch_path(&root, &cwd.to_string_lossy(), "./nested/file.rs")
            .expect("nested relative path should resolve");
        assert_eq!(result, "src/lib/nested/file.rs");
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

        for path in ["../outside.txt", "/etc/passwd", "./..", ""] {
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
