use anyhow::{bail, Context, Result};
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::services::codex_hook_config;
use crate::services::default_paths::InstallTargetPaths;
use crate::services::security::{ensure_directory_is_writable, redact_sensitive_text};

use super::config_merge;
use super::hook_merge;
use super::{
    classify_git_exit, cleanup_path_if_exists, concrete_targets_for,
    embedded_assets_for_concrete_target, hook_install_recovery_guidance,
    iter_embedded_assets_for_setup_target_with_selection, iter_required_hook_assets,
    setup_install_recovery_guidance, EmbeddedAsset, GitExitKind, GitRepositoryResolutionError,
    RequiredHookInstallResult, RequiredHookInstallStatus, RequiredHooksInstallOutcome,
    SetupInstallOutcome, SetupInstallTargetResult, SetupTarget,
};
use crate::services::default_paths;
use crate::services::default_paths::claude_asset;

pub(super) fn prepare_setup_hooks_repository(repository_root: &Path) -> Result<PathBuf> {
    let normalized_repository_root = normalize_user_repository_path(repository_root)?;
    Ok(resolve_git_repository_root(&normalized_repository_root)?)
}

pub(super) fn ensure_git_repository(
    directory: &Path,
) -> Result<PathBuf, GitRepositoryResolutionError> {
    resolve_git_repository_root(directory)
}

pub(super) fn install_required_git_hooks(
    repository_root: &Path,
) -> Result<RequiredHooksInstallOutcome> {
    install_required_git_hooks_with_rename(repository_root, |from, to| fs::rename(from, to))
}

pub(super) fn install_required_git_hooks_with_rename<F>(
    repository_root: &Path,
    rename_fn: F,
) -> Result<RequiredHooksInstallOutcome>
where
    F: FnMut(&Path, &Path) -> io::Result<()>,
{
    let resolved_repository_root = prepare_setup_hooks_repository(repository_root)?;
    install_required_git_hooks_in_resolved_repository(&resolved_repository_root, rename_fn)
}

pub(super) fn install_embedded_setup_assets(
    repository_root: &Path,
    target: SetupTarget,
    selected_optional_workflows: &[String],
) -> Result<SetupInstallOutcome> {
    install_embedded_setup_assets_with_rename(
        repository_root,
        target,
        selected_optional_workflows,
        |from, to| fs::rename(from, to),
    )
}

pub(super) fn repair_merge_target_asset(
    repository_root: &Path,
    target: SetupTarget,
    relative_path: &str,
) -> Result<()> {
    let asset = embedded_assets_for_concrete_target(target)
        .iter()
        .find(|asset| asset.relative_path == relative_path)
        .with_context(|| {
            format!("No embedded asset named '{relative_path}' for target {target:?}")
        })?;

    let install_targets = InstallTargetPaths::new(repository_root);
    let destination_root = match target {
        SetupTarget::OpenCode => install_targets.opencode_target_dir(),
        SetupTarget::Claude => install_targets.claude_target_dir(),
        SetupTarget::Pi => install_targets.pi_target_dir(),
        SetupTarget::Codex => install_targets.codex_target_dir(),
        SetupTarget::All => unreachable!("meta targets are expanded into concrete targets"),
    };

    install_single_asset_with_rename(target, &destination_root, asset, &mut |from, to| {
        fs::rename(from, to)
    })
}

fn install_required_git_hooks_in_resolved_repository<F>(
    resolved_repository_root: &Path,
    mut rename_fn: F,
) -> Result<RequiredHooksInstallOutcome>
where
    F: FnMut(&Path, &Path) -> io::Result<()>,
{
    ensure_directory_is_writable(resolved_repository_root, "repository root")?;
    let hooks_directory = resolve_git_hooks_directory(resolved_repository_root)?;
    fs::create_dir_all(&hooks_directory).with_context(|| {
        format!(
            "Failed to create git hooks directory '{}'",
            hooks_directory.display()
        )
    })?;
    ensure_directory_is_writable(&hooks_directory, "git hooks directory")?;

    let mut hook_results = Vec::new();
    for hook_asset in iter_required_hook_assets() {
        let hook_result =
            install_single_required_hook_with_rename(&hooks_directory, hook_asset, &mut rename_fn)?;
        hook_results.push(hook_result);
    }

    Ok(RequiredHooksInstallOutcome {
        repository_root: resolved_repository_root.to_path_buf(),
        hooks_directory,
        hook_results,
    })
}

fn install_single_required_hook_with_rename<F>(
    hooks_directory: &Path,
    hook_asset: &EmbeddedAsset,
    rename_fn: &mut F,
) -> Result<RequiredHookInstallResult>
where
    F: FnMut(&Path, &Path) -> io::Result<()>,
{
    validate_embedded_relative_path(hook_asset.relative_path)?;

    let hook_path = hooks_directory.join(hook_asset.relative_path);
    let existing_metadata = fs::metadata(&hook_path).ok();

    let existing_bytes =
        if existing_metadata
            .as_ref()
            .is_some_and(std::fs::Metadata::is_file)
        {
            Some(fs::read(&hook_path).with_context(|| {
                format!("Failed to read existing hook '{}'", hook_path.display())
            })?)
        } else if existing_metadata.is_some() {
            bail!(
                "Existing hook target '{}' is not a file",
                hook_path.display()
            );
        } else {
            None
        };

    let merge = hook_merge::merge_or_create_hook(
        existing_bytes.as_deref(),
        hook_asset.bytes,
        hook_asset.relative_path,
    )?;

    if let Some(existing_bytes) = existing_bytes.as_deref() {
        let executable = is_executable_file(&hook_path)?;
        if merge.bytes == existing_bytes && executable {
            return Ok(RequiredHookInstallResult {
                hook_name: hook_asset.relative_path.to_string(),
                hook_path,
                status: RequiredHookInstallStatus::Skipped,
                unreachable_block_advisory: merge.unreachable_block_advisory,
            });
        }
    }

    let had_existing_hook = existing_metadata.is_some();

    let hook_staging_path = create_hook_staging_path(hooks_directory, hook_asset.relative_path)?;
    if let Err(error) = write_hook_payload_to_staging(&hook_staging_path, &merge.bytes) {
        cleanup_path_if_exists(&hook_staging_path);
        return Err(error);
    }

    let action = if had_existing_hook {
        "update"
    } else {
        "install"
    };
    if let Err(error) = rename_fn(&hook_staging_path, &hook_path).with_context(|| {
        format!(
            "Failed to {action} required hook '{}' at '{}'",
            hook_asset.relative_path,
            hook_path.display()
        )
    }) {
        cleanup_path_if_exists(&hook_staging_path);
        let error = if had_existing_hook {
            error.context(hook_install_recovery_guidance(&hook_path))
        } else {
            error
        };
        return Err(error);
    }

    Ok(RequiredHookInstallResult {
        hook_name: hook_asset.relative_path.to_string(),
        hook_path,
        status: if had_existing_hook {
            RequiredHookInstallStatus::Updated
        } else {
            RequiredHookInstallStatus::Installed
        },
        unreachable_block_advisory: merge.unreachable_block_advisory,
    })
}

fn write_hook_payload_to_staging(staging_path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(staging_path, bytes).with_context(|| {
        format!(
            "Failed to write staged hook payload '{}'",
            staging_path.display()
        )
    })?;
    ensure_executable_permissions(staging_path)?;
    Ok(())
}

fn create_hook_staging_path(hooks_directory: &Path, hook_name: &str) -> Result<PathBuf> {
    let epoch_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System clock is before UNIX_EPOCH")?
        .as_nanos();
    let sanitized_hook_name = hook_name.replace('/', "-");

    for attempt in 0..1000_u16 {
        let candidate = hooks_directory.join(format!(
            ".sce-hook-staging-{sanitized_hook_name}-{epoch_nanos}-{}-{attempt}",
            std::process::id()
        ));

        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Failed to allocate hook staging file '{}'",
                        candidate.display()
                    )
                });
            }
        }
    }

    bail!(
        "Could not allocate a unique hook staging file under '{}'",
        hooks_directory.display()
    )
}

fn normalize_user_repository_path(repository_root: &Path) -> Result<PathBuf> {
    if repository_root.as_os_str().is_empty() {
        bail!("Option '--repo' must not be empty. Try: pass a path to an existing git repository.");
    }

    let canonical_repository_root = fs::canonicalize(repository_root).with_context(|| {
        format!(
            "Failed to resolve repository path '{}'. Try: pass a path to an existing git repository.",
            repository_root.display()
        )
    })?;

    let metadata = fs::metadata(&canonical_repository_root).with_context(|| {
        format!(
            "Failed to inspect repository path '{}'.",
            canonical_repository_root.display()
        )
    })?;

    if !metadata.is_dir() {
        bail!(
            "Repository path '{}' is not a directory. Try: pass a path to an existing git repository.",
            canonical_repository_root.display()
        );
    }

    Ok(canonical_repository_root)
}

fn resolve_git_repository_root(
    repository_root: &Path,
) -> Result<PathBuf, GitRepositoryResolutionError> {
    let repository_root_output = run_git_command_in_directory(
        repository_root,
        &["rev-parse", "--show-toplevel"],
        "Failed to resolve repository root. Ensure '--repo' points to an accessible git repository.",
    )
    .map_err(map_setup_repository_resolution_error)?;
    Ok(PathBuf::from(repository_root_output))
}

fn map_setup_repository_resolution_error(error: GitCommandError) -> GitRepositoryResolutionError {
    let is_not_repository = matches!(
        &error,
        GitCommandError::NonZeroExit {
            kind: GitExitKind::NotRepository,
            ..
        }
    );
    let source = anyhow::Error::new(error);

    if is_not_repository {
        GitRepositoryResolutionError::NotGitRepository(source)
    } else {
        GitRepositoryResolutionError::Unexpected(source)
    }
}

fn resolve_git_hooks_directory(repository_root: &Path) -> Result<PathBuf> {
    let hooks_directory_output = run_git_command_in_directory(
        repository_root,
        &["rev-parse", "--git-path", "hooks"],
        "Failed to resolve effective git hooks path.",
    )?;

    let hooks_directory = PathBuf::from(&hooks_directory_output);
    if hooks_directory.is_absolute() {
        return Ok(hooks_directory);
    }

    Ok(repository_root.join(hooks_directory))
}

#[derive(Debug)]
enum GitCommandError {
    Spawn {
        context: String,
        directory: PathBuf,
        source: std::io::Error,
    },
    NonZeroExit {
        context: String,
        directory: PathBuf,
        status: std::process::ExitStatus,
        kind: GitExitKind,
        diagnostic: String,
    },
    InvalidUtf8 {
        context: String,
        source: std::string::FromUtf8Error,
    },
    EmptyOutput {
        context: String,
        directory: PathBuf,
    },
}

impl std::fmt::Display for GitCommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn {
                context,
                directory,
                source,
            } => write!(
                f,
                "{context} (directory: '{}'): {source}",
                directory.display()
            ),
            Self::NonZeroExit {
                context,
                directory,
                status,
                diagnostic,
                ..
            } => write!(
                f,
                "{context} (directory: '{}', status: {status:?}) {diagnostic}",
                directory.display()
            ),
            Self::InvalidUtf8 { context, source } => {
                write!(
                    f,
                    "{context}: git command output contained invalid UTF-8: {source}"
                )
            }
            Self::EmptyOutput { context, directory } => write!(
                f,
                "{context} (directory: '{}'): git command returned empty output",
                directory.display()
            ),
        }
    }
}

impl std::error::Error for GitCommandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } => Some(source),
            Self::InvalidUtf8 { source, .. } => Some(source),
            Self::NonZeroExit { .. } | Self::EmptyOutput { .. } => None,
        }
    }
}

fn run_git_command_in_directory(
    repository_root: &Path,
    args: &[&str],
    context_message: &str,
) -> std::result::Result<String, GitCommandError> {
    let output = Command::new("git")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env_remove("LANGUAGE")
        .args(args)
        .current_dir(repository_root)
        .output()
        .map_err(|source| GitCommandError::Spawn {
            context: context_message.to_string(),
            directory: repository_root.to_path_buf(),
            source,
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let kind = classify_git_exit(&stderr);
        let diagnostic = if stderr.is_empty() {
            String::from("git command exited with a non-zero status")
        } else {
            redact_sensitive_text(&stderr)
        };
        return Err(GitCommandError::NonZeroExit {
            context: context_message.to_string(),
            directory: repository_root.to_path_buf(),
            status: output.status,
            kind,
            diagnostic,
        });
    }

    let stdout =
        String::from_utf8(output.stdout).map_err(|source| GitCommandError::InvalidUtf8 {
            context: context_message.to_string(),
            source,
        })?;
    let stdout = stdout.trim().to_string();
    if stdout.is_empty() {
        return Err(GitCommandError::EmptyOutput {
            context: context_message.to_string(),
            directory: repository_root.to_path_buf(),
        });
    }

    Ok(stdout)
}

#[cfg(unix)]
fn ensure_executable_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path)
        .with_context(|| format!("Failed to read metadata for '{}'", path.display()))?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    fs::set_permissions(path, permissions).with_context(|| {
        format!(
            "Failed to set executable permissions for '{}'",
            path.display()
        )
    })?;
    Ok(())
}

#[cfg(not(unix))]
fn ensure_executable_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> Result<bool> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path)
        .with_context(|| format!("Failed to read metadata for '{}'", path.display()))?;
    Ok(metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> Result<bool> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("Failed to read metadata for '{}'", path.display()))?;
    Ok(metadata.is_file())
}

pub(super) fn install_embedded_setup_assets_with_rename<F>(
    repository_root: &Path,
    target: SetupTarget,
    selected_optional_workflows: &[String],
    mut rename_fn: F,
) -> Result<SetupInstallOutcome>
where
    F: FnMut(&Path, &Path) -> io::Result<()>,
{
    ensure_directory_is_writable(repository_root, "setup repository root")?;

    let mut target_results = Vec::new();

    for concrete_target in concrete_targets_for(target) {
        let concrete_target = *concrete_target;
        let assets: Vec<&'static EmbeddedAsset> =
            iter_embedded_assets_for_setup_target_with_selection(
                concrete_target,
                selected_optional_workflows,
            )
            .collect();
        let result = install_assets_for_concrete_target_with_rename(
            repository_root,
            concrete_target,
            &assets,
            &mut rename_fn,
        )?;
        target_results.push(result);
    }

    Ok(SetupInstallOutcome { target_results })
}

fn install_assets_for_concrete_target_with_rename<F>(
    repository_root: &Path,
    target: SetupTarget,
    assets: &[&'static EmbeddedAsset],
    rename_fn: &mut F,
) -> Result<SetupInstallTargetResult>
where
    F: FnMut(&Path, &Path) -> io::Result<()>,
{
    let install_targets = InstallTargetPaths::new(repository_root);
    let destination_root = match target {
        SetupTarget::OpenCode => install_targets.opencode_target_dir(),
        SetupTarget::Claude => install_targets.claude_target_dir(),
        SetupTarget::Pi => install_targets.pi_target_dir(),
        SetupTarget::Codex => install_targets.codex_target_dir(),
        SetupTarget::All => {
            unreachable!("meta targets are expanded into concrete targets")
        }
    };

    for asset in assets {
        install_single_asset_with_rename(target, &destination_root, asset, rename_fn)?;
    }

    prune_stale_assets_for_concrete_target(&destination_root, target, assets)?;

    Ok(SetupInstallTargetResult {
        target,
        destination_root,
        installed_file_count: assets.len(),
    })
}

fn prune_stale_assets_for_concrete_target(
    destination_root: &Path,
    target: SetupTarget,
    installed_assets: &[&'static EmbeddedAsset],
) -> Result<()> {
    let installed_paths: std::collections::HashSet<&'static str> = installed_assets
        .iter()
        .map(|asset| asset.relative_path)
        .collect();

    for asset in embedded_assets_for_concrete_target(target) {
        if installed_paths.contains(asset.relative_path) {
            continue;
        }

        let destination = destination_root.join(asset.relative_path);
        if !destination.is_file() {
            continue;
        }

        fs::remove_file(&destination).with_context(|| {
            format!(
                "Failed to prune unselected setup asset '{}'",
                destination.display()
            )
        })?;

        remove_empty_ancestor_directories(destination_root, &destination);
    }

    Ok(())
}

fn remove_empty_ancestor_directories(destination_root: &Path, removed_file: &Path) {
    let mut current = removed_file.parent();
    while let Some(directory) = current {
        if directory == destination_root || !directory.starts_with(destination_root) {
            break;
        }
        if fs::remove_dir(directory).is_err() {
            break;
        }
        current = directory.parent();
    }
}

fn is_claude_settings_merge_target(target: SetupTarget, relative_path: &str) -> bool {
    target == SetupTarget::Claude && relative_path == claude_asset::SETTINGS_FILE
}

fn is_opencode_config_merge_target(target: SetupTarget, relative_path: &str) -> bool {
    target == SetupTarget::OpenCode && relative_path == default_paths::repo_file::OPENCODE_MANIFEST
}

fn is_codex_hooks_merge_target(target: SetupTarget, relative_path: &str) -> bool {
    target == SetupTarget::Codex && relative_path == ".codex/hooks.json"
}

fn install_single_asset_with_rename<F>(
    target: SetupTarget,
    destination_root: &Path,
    asset: &'static EmbeddedAsset,
    rename_fn: &mut F,
) -> Result<()>
where
    F: FnMut(&Path, &Path) -> io::Result<()>,
{
    validate_embedded_relative_path(asset.relative_path)?;
    let destination = destination_root.join(asset.relative_path);
    let parent = destination
        .parent()
        .context("Embedded asset destination should have a parent directory")?;

    fs::create_dir_all(parent).with_context(|| {
        format!(
            "Failed to create parent directory '{}' for setup asset",
            parent.display()
        )
    })?;

    if destination.is_dir() {
        bail!(
            "Setup asset destination '{}' is an existing directory, not a file. Try: remove or rename the directory and rerun 'sce setup'.",
            destination.display()
        );
    }

    let install_bytes: Vec<u8> = if is_claude_settings_merge_target(target, asset.relative_path) {
        let existing_bytes = if destination.is_file() {
            Some(fs::read(&destination).with_context(|| {
                format!(
                    "Failed to read existing setup asset '{}' for merge",
                    destination.display()
                )
            })?)
        } else {
            None
        };
        config_merge::merge_or_create_claude_settings(
            existing_bytes.as_deref(),
            asset.bytes,
            &destination.display().to_string(),
        )?
    } else if is_opencode_config_merge_target(target, asset.relative_path) {
        let existing_bytes = if destination.is_file() {
            Some(fs::read(&destination).with_context(|| {
                format!(
                    "Failed to read existing setup asset '{}' for merge",
                    destination.display()
                )
            })?)
        } else {
            None
        };
        config_merge::merge_or_create_opencode_config(
            existing_bytes.as_deref(),
            asset.bytes,
            &destination.display().to_string(),
        )?
    } else if is_codex_hooks_merge_target(target, asset.relative_path) {
        let existing_bytes = if destination.is_file() {
            Some(fs::read(&destination).with_context(|| {
                format!(
                    "Failed to read existing setup asset '{}' for merge",
                    destination.display()
                )
            })?)
        } else {
            None
        };
        codex_hook_config::merge_or_create(
            existing_bytes.as_deref(),
            asset.bytes,
            &destination.display().to_string(),
        )?
    } else {
        asset.bytes.to_vec()
    };

    let staging_path = create_asset_staging_path(parent, asset.relative_path)?;
    if let Err(error) = fs::write(&staging_path, &install_bytes).with_context(|| {
        format!(
            "Failed to write staged embedded asset '{}'",
            staging_path.display()
        )
    }) {
        cleanup_path_if_exists(&staging_path);
        return Err(error);
    }

    if let Err(error) = rename_fn(&staging_path, &destination).with_context(|| {
        format!(
            "Failed to install staged asset '{}' into destination '{}'",
            staging_path.display(),
            destination.display()
        )
    }) {
        cleanup_path_if_exists(&staging_path);
        return Err(error.context(setup_install_recovery_guidance(target, &destination)));
    }

    Ok(())
}

fn create_asset_staging_path(parent: &Path, relative_path: &str) -> Result<PathBuf> {
    let epoch_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System clock is before UNIX_EPOCH")?
        .as_nanos();
    let sanitized_name = relative_path.replace(['/', '\\'], "-");

    for attempt in 0..1000_u16 {
        let candidate = parent.join(format!(
            ".sce-setup-staging-{sanitized_name}-{epoch_nanos}-{}-{attempt}",
            std::process::id()
        ));

        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Failed to allocate staging file '{}'", candidate.display())
                });
            }
        }
    }

    bail!(
        "Could not allocate a unique staging file under '{}'",
        parent.display()
    )
}

fn validate_embedded_relative_path(relative_path: &str) -> Result<()> {
    let path = Path::new(relative_path);

    if path.is_absolute() {
        bail!("Embedded asset path '{relative_path}' must be relative, not absolute");
    }

    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => {
                bail!("Embedded asset path '{relative_path}' contains disallowed component");
            }
        }
    }

    Ok(())
}
