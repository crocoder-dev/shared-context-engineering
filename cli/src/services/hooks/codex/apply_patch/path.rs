use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

use super::parser::{CodexFileOperation, CodexPatch};

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
