use anyhow::{bail, Context, Result};
use inquire::{InquireError, Select};
use lexopt::{Arg, ValueExt};
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::services::security::{ensure_directory_is_writable, redact_sensitive_text};

pub const NAME: &str = "setup";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupTarget {
    OpenCode,
    Claude,
    Both,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmbeddedAsset {
    pub relative_path: &'static str,
    pub bytes: &'static [u8],
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequiredHookAsset {
    PreCommit,
    CommitMsg,
    PostCommit,
}

include!(concat!(env!("OUT_DIR"), "/setup_embedded_assets.rs"));

pub fn iter_required_hook_assets() -> std::slice::Iter<'static, EmbeddedAsset> {
    HOOK_EMBEDDED_ASSETS.iter()
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn get_required_hook_asset(hook: RequiredHookAsset) -> Option<&'static EmbeddedAsset> {
    let hook_name = match hook {
        RequiredHookAsset::PreCommit => "pre-commit",
        RequiredHookAsset::CommitMsg => "commit-msg",
        RequiredHookAsset::PostCommit => "post-commit",
    };

    HOOK_EMBEDDED_ASSETS
        .iter()
        .find(|asset| asset.relative_path == hook_name)
}

pub enum EmbeddedAssetSelectionIter {
    One(std::slice::Iter<'static, EmbeddedAsset>),
    Both(
        std::iter::Chain<
            std::slice::Iter<'static, EmbeddedAsset>,
            std::slice::Iter<'static, EmbeddedAsset>,
        >,
    ),
}

impl Iterator for EmbeddedAssetSelectionIter {
    type Item = &'static EmbeddedAsset;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::One(iter) => iter.next(),
            Self::Both(iter) => iter.next(),
        }
    }
}

pub fn iter_embedded_assets_for_setup_target(target: SetupTarget) -> EmbeddedAssetSelectionIter {
    match target {
        SetupTarget::OpenCode => EmbeddedAssetSelectionIter::One(OPENCODE_EMBEDDED_ASSETS.iter()),
        SetupTarget::Claude => EmbeddedAssetSelectionIter::One(CLAUDE_EMBEDDED_ASSETS.iter()),
        SetupTarget::Both => EmbeddedAssetSelectionIter::Both(
            OPENCODE_EMBEDDED_ASSETS
                .iter()
                .chain(CLAUDE_EMBEDDED_ASSETS.iter()),
        ),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupMode {
    Interactive,
    NonInteractive(SetupTarget),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupDispatch {
    Proceed(SetupMode),
    Cancelled,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SetupCliOptions {
    pub help: bool,
    pub non_interactive: bool,
    pub opencode: bool,
    pub claude: bool,
    pub both: bool,
    pub hooks: bool,
    pub repo_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupRequest {
    pub config_mode: Option<SetupMode>,
    pub install_hooks: bool,
    pub hooks_repo_path: Option<PathBuf>,
}

pub fn resolve_setup_request(options: SetupCliOptions) -> Result<SetupRequest> {
    if options.repo_path.is_some() && !options.hooks {
        bail!(
            "Option '--repo' requires '--hooks'. Try: run 'sce setup --hooks --repo <path>' or remove '--repo'."
        );
    }

    let mut selected_targets = Vec::new();

    if options.opencode {
        selected_targets.push(SetupTarget::OpenCode);
    }
    if options.claude {
        selected_targets.push(SetupTarget::Claude);
    }
    if options.both {
        selected_targets.push(SetupTarget::Both);
    }

    if selected_targets.len() > 1 {
        bail!(
            "Options '--opencode', '--claude', and '--both' are mutually exclusive. Try: choose exactly one target flag (for example 'sce setup --opencode --non-interactive') or omit all target flags for interactive mode."
        );
    }

    if options.non_interactive && selected_targets.is_empty() && !options.hooks {
        bail!(
            "Option '--non-interactive' requires a target flag. Try: 'sce setup --opencode --non-interactive', 'sce setup --claude --non-interactive', or 'sce setup --both --non-interactive'."
        );
    }

    let config_mode = match selected_targets.as_slice() {
        [target] => Some(SetupMode::NonInteractive(*target)),
        [] if options.hooks => None,
        [] => Some(SetupMode::Interactive),
        _ => unreachable!("target count already validated"),
    };

    let install_hooks = options.hooks || (config_mode == Some(SetupMode::Interactive));

    Ok(SetupRequest {
        config_mode,
        install_hooks,
        hooks_repo_path: options.repo_path,
    })
}

pub fn run_setup_for_mode(repository_root: &Path, mode: SetupMode) -> Result<String> {
    let target = match mode {
        SetupMode::Interactive => {
            bail!("Interactive setup mode must be resolved before installation")
        }
        SetupMode::NonInteractive(target) => target,
    };

    let outcome = install_embedded_setup_assets(repository_root, target).with_context(|| {
        format!(
            "Setup installation failed for {}",
            setup_target_label(target)
        )
    })?;

    Ok(format_setup_install_success_message(&outcome))
}

pub fn run_setup_hooks(repository_root: &Path) -> Result<String> {
    let normalized_repository_root = normalize_user_repository_path(repository_root)?;
    let outcome = install_required_git_hooks(&normalized_repository_root)
        .context("Hook setup failed while installing required git hooks")?;
    Ok(format_required_hook_install_success_message(&outcome))
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

fn format_setup_install_success_message(outcome: &SetupInstallOutcome) -> String {
    let selected_targets = outcome
        .target_results
        .iter()
        .map(|result| setup_target_label(result.target))
        .collect::<Vec<_>>()
        .join(", ");

    let mut lines = vec![
        "Setup completed successfully.".to_string(),
        format!("Selected target(s): {selected_targets}"),
    ];

    for result in &outcome.target_results {
        lines.push(format!(
            "- {}: installed {} file(s) to '{}'",
            setup_target_label(result.target),
            result.installed_file_count,
            result.destination_root.display()
        ));

        match result.backup_root.as_ref() {
            Some(backup_root) => lines.push(format!(
                "  backup: existing target moved to '{}'",
                backup_root.display()
            )),
            None => lines.push("  backup: not needed (no existing target)".to_string()),
        }
    }

    lines.join("\n")
}

fn format_required_hook_install_success_message(outcome: &RequiredHooksInstallOutcome) -> String {
    let mut lines = vec![
        "Hook setup completed successfully.".to_string(),
        format!("Repository root: '{}'", outcome.repository_root.display()),
        format!("Hooks directory: '{}'", outcome.hooks_directory.display()),
    ];

    for result in &outcome.hook_results {
        lines.push(format!(
            "- {}: {} at '{}'",
            result.hook_name,
            required_hook_status_label(result.status),
            result.hook_path.display()
        ));

        match result.backup_path.as_ref() {
            Some(backup_path) => lines.push(format!("  backup: '{}'", backup_path.display())),
            None => lines.push("  backup: not needed".to_string()),
        }
    }

    lines.join("\n")
}

fn required_hook_status_label(status: RequiredHookInstallStatus) -> &'static str {
    match status {
        RequiredHookInstallStatus::Installed => "installed",
        RequiredHookInstallStatus::Updated => "updated",
        RequiredHookInstallStatus::Skipped => "skipped",
    }
}

fn setup_target_label(target: SetupTarget) -> &'static str {
    match target {
        SetupTarget::OpenCode => "OpenCode",
        SetupTarget::Claude => "Claude",
        SetupTarget::Both => "Both",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupInstallTargetResult {
    pub target: SetupTarget,
    pub destination_root: PathBuf,
    pub backup_root: Option<PathBuf>,
    pub installed_file_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupInstallOutcome {
    pub target_results: Vec<SetupInstallTargetResult>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequiredHookInstallStatus {
    Installed,
    Updated,
    Skipped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequiredHookInstallResult {
    pub hook_name: String,
    pub hook_path: PathBuf,
    pub status: RequiredHookInstallStatus,
    pub backup_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequiredHooksInstallOutcome {
    pub repository_root: PathBuf,
    pub hooks_directory: PathBuf,
    pub hook_results: Vec<RequiredHookInstallResult>,
}

pub fn install_required_git_hooks(repository_root: &Path) -> Result<RequiredHooksInstallOutcome> {
    install_required_git_hooks_with_rename(repository_root, |from, to| fs::rename(from, to))
}

fn install_required_git_hooks_with_rename<F>(
    repository_root: &Path,
    mut rename_fn: F,
) -> Result<RequiredHooksInstallOutcome>
where
    F: FnMut(&Path, &Path) -> io::Result<()>,
{
    let resolved_repository_root = resolve_git_repository_root(repository_root)?;
    ensure_directory_is_writable(&resolved_repository_root, "repository root")?;
    let hooks_directory = resolve_git_hooks_directory(&resolved_repository_root)?;
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
        repository_root: resolved_repository_root,
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

    if existing_metadata
        .as_ref()
        .is_some_and(|metadata| metadata.is_file())
    {
        let existing_bytes = fs::read(&hook_path)
            .with_context(|| format!("Failed to read existing hook '{}'", hook_path.display()))?;
        let executable = is_executable_file(&hook_path)?;

        if existing_bytes == hook_asset.bytes && executable {
            return Ok(RequiredHookInstallResult {
                hook_name: hook_asset.relative_path.to_string(),
                hook_path,
                status: RequiredHookInstallStatus::Skipped,
                backup_path: None,
            });
        }
    } else if existing_metadata.is_some() {
        bail!(
            "Existing hook target '{}' is not a file",
            hook_path.display()
        );
    }

    let hook_staging_path = create_hook_staging_path(hooks_directory, hook_asset.relative_path)?;
    if let Err(error) = write_hook_payload_to_staging(&hook_staging_path, hook_asset.bytes) {
        cleanup_path_if_exists(&hook_staging_path);
        return Err(error);
    }

    if existing_metadata.is_none() {
        if let Err(error) = rename_fn(&hook_staging_path, &hook_path).with_context(|| {
            format!(
                "Failed to install required hook '{}' at '{}'",
                hook_asset.relative_path,
                hook_path.display()
            )
        }) {
            cleanup_path_if_exists(&hook_staging_path);
            return Err(error);
        }

        return Ok(RequiredHookInstallResult {
            hook_name: hook_asset.relative_path.to_string(),
            hook_path,
            status: RequiredHookInstallStatus::Installed,
            backup_path: None,
        });
    }

    let backup_path = next_backup_path(&hook_path)?;
    rename_fn(&hook_path, &backup_path).with_context(|| {
        format!(
            "Failed to back up existing hook '{}' to '{}'",
            hook_path.display(),
            backup_path.display()
        )
    })?;

    if let Err(error) = rename_fn(&hook_staging_path, &hook_path).with_context(|| {
        format!(
            "Failed to update required hook '{}' at '{}'",
            hook_asset.relative_path,
            hook_path.display()
        )
    }) {
        cleanup_path_if_exists(&hook_staging_path);

        if !hook_path.exists() {
            if let Err(restore_error) = rename_fn(&backup_path, &hook_path) {
                return Err(error.context(format!(
                    "Rollback failed while restoring hook '{}' from backup '{}': {}",
                    hook_path.display(),
                    backup_path.display(),
                    restore_error
                )));
            }
        }

        return Err(error);
    }

    Ok(RequiredHookInstallResult {
        hook_name: hook_asset.relative_path.to_string(),
        hook_path,
        status: RequiredHookInstallStatus::Updated,
        backup_path: Some(backup_path),
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
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
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

fn resolve_git_repository_root(repository_root: &Path) -> Result<PathBuf> {
    let repository_root_output = run_git_command_in_directory(
        repository_root,
        &["rev-parse", "--show-toplevel"],
        "Failed to resolve repository root. Ensure '--repo' points to an accessible git repository.",
    )?;
    Ok(PathBuf::from(repository_root_output))
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

fn run_git_command_in_directory(
    repository_root: &Path,
    args: &[&str],
    context_message: &str,
) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository_root)
        .output()
        .with_context(|| {
            format!(
                "{} (directory: '{}')",
                context_message,
                repository_root.display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let diagnostic = if stderr.is_empty() {
            "git command exited with a non-zero status".to_string()
        } else {
            redact_sensitive_text(&stderr)
        };
        bail!("{} {}", context_message, diagnostic);
    }

    let stdout = String::from_utf8(output.stdout)
        .context("git command output contained invalid UTF-8")?
        .trim()
        .to_string();
    if stdout.is_empty() {
        bail!("{} git command returned empty output", context_message);
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

pub fn install_embedded_setup_assets(
    repository_root: &Path,
    target: SetupTarget,
) -> Result<SetupInstallOutcome> {
    install_embedded_setup_assets_with_rename(repository_root, target, |from, to| {
        fs::rename(from, to)
    })
}

fn install_embedded_setup_assets_with_rename<F>(
    repository_root: &Path,
    target: SetupTarget,
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
            iter_embedded_assets_for_setup_target(concrete_target).collect();
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
    let destination_root = repository_root.join(target_install_directory_name(target));
    let staging_root = create_staging_root(repository_root, target)?;

    if let Err(error) = write_assets_to_staging(&staging_root, assets) {
        cleanup_path_if_exists(&staging_root);
        return Err(error);
    }

    let mut backup_root = None;

    if destination_root.exists() {
        let backup_path = next_backup_path(&destination_root)?;
        rename_fn(&destination_root, &backup_path).with_context(|| {
            format!(
                "Failed to move existing target '{}' to backup '{}'",
                destination_root.display(),
                backup_path.display()
            )
        })?;
        backup_root = Some(backup_path);
    }

    if let Err(error) = rename_fn(&staging_root, &destination_root).with_context(|| {
        format!(
            "Failed to swap staged install '{}' into destination '{}'",
            staging_root.display(),
            destination_root.display()
        )
    }) {
        cleanup_path_if_exists(&staging_root);

        if let Some(backup_path) = backup_root.as_ref() {
            if !destination_root.exists() {
                if let Err(restore_error) = rename_fn(backup_path, &destination_root) {
                    return Err(error.context(format!(
                        "Rollback failed while restoring '{}' from backup '{}': {}",
                        destination_root.display(),
                        backup_path.display(),
                        restore_error
                    )));
                }
            }
        }

        return Err(error);
    }

    Ok(SetupInstallTargetResult {
        target,
        destination_root,
        backup_root,
        installed_file_count: assets.len(),
    })
}

fn write_assets_to_staging(staging_root: &Path, assets: &[&'static EmbeddedAsset]) -> Result<()> {
    for asset in assets {
        validate_embedded_relative_path(asset.relative_path)?;
        let destination = staging_root.join(asset.relative_path);
        let parent = destination
            .parent()
            .context("Embedded asset destination should have a parent directory")?;

        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create staged parent directory '{}'",
                parent.display()
            )
        })?;

        fs::write(&destination, asset.bytes).with_context(|| {
            format!(
                "Failed to write staged embedded asset '{}'",
                destination.display()
            )
        })?;
    }

    Ok(())
}

fn validate_embedded_relative_path(relative_path: &str) -> Result<()> {
    let path = Path::new(relative_path);

    if path.is_absolute() {
        bail!(
            "Embedded asset path '{}' must be relative, not absolute",
            relative_path
        );
    }

    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => {
                bail!(
                    "Embedded asset path '{}' contains disallowed component",
                    relative_path
                );
            }
        }
    }

    Ok(())
}

fn create_staging_root(repository_root: &Path, target: SetupTarget) -> Result<PathBuf> {
    let target_label = target_install_directory_name(target).trim_start_matches('.');
    let epoch_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System clock is before UNIX_EPOCH")?
        .as_nanos();

    for attempt in 0..1000_u16 {
        let candidate = repository_root.join(format!(
            ".sce-setup-staging-{target_label}-{epoch_nanos}-{}-{attempt}",
            std::process::id()
        ));

        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Failed to create staging directory '{}'",
                        candidate.display()
                    )
                });
            }
        }
    }

    bail!(
        "Could not allocate a unique staging directory under '{}'",
        repository_root.display()
    )
}

fn next_backup_path(destination_root: &Path) -> Result<PathBuf> {
    let base_name = destination_root
        .file_name()
        .and_then(|name| name.to_str())
        .context("Target destination root should have a valid UTF-8 file name")?;

    for suffix in std::iter::once(String::new()).chain((1_u16..).map(|i| format!(".{i}"))) {
        let candidate = destination_root.with_file_name(format!("{base_name}.backup{suffix}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    unreachable!("backup suffix iterator is unbounded")
}

fn target_install_directory_name(target: SetupTarget) -> &'static str {
    match target {
        SetupTarget::OpenCode => ".opencode",
        SetupTarget::Claude => ".claude",
        SetupTarget::Both => unreachable!("both is expanded into concrete targets"),
    }
}

fn concrete_targets_for(target: SetupTarget) -> &'static [SetupTarget] {
    match target {
        SetupTarget::OpenCode => &[SetupTarget::OpenCode],
        SetupTarget::Claude => &[SetupTarget::Claude],
        SetupTarget::Both => &[SetupTarget::OpenCode, SetupTarget::Claude],
    }
}

fn cleanup_path_if_exists(path: &Path) {
    let cleanup_result = if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };

    let _ = cleanup_result;
}

pub trait SetupTargetPrompter {
    fn prompt_target(&self) -> Result<SetupDispatch>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct InquireSetupTargetPrompter;

impl SetupTargetPrompter for InquireSetupTargetPrompter {
    fn prompt_target(&self) -> Result<SetupDispatch> {
        let options = vec![
            SetupPromptTarget::OpenCode,
            SetupPromptTarget::Claude,
            SetupPromptTarget::Both,
        ];

        let selection = Select::new("Select setup target", options).prompt();

        match selection {
            Ok(SetupPromptTarget::OpenCode) => Ok(SetupDispatch::Proceed(SetupMode::NonInteractive(
                SetupTarget::OpenCode,
            ))),
            Ok(SetupPromptTarget::Claude) => Ok(SetupDispatch::Proceed(SetupMode::NonInteractive(
                SetupTarget::Claude,
            ))),
            Ok(SetupPromptTarget::Both) => {
                Ok(SetupDispatch::Proceed(SetupMode::NonInteractive(SetupTarget::Both)))
            }
            Err(InquireError::OperationCanceled) | Err(InquireError::OperationInterrupted) => {
                Ok(SetupDispatch::Cancelled)
            }
            Err(InquireError::NotTTY) => bail!(
                "Interactive setup requires a TTY. Re-run with '--non-interactive' and one of '--opencode', '--claude', or '--both'."
            ),
            Err(error) => Err(error.into()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetupPromptTarget {
    OpenCode,
    Claude,
    Both,
}

impl std::fmt::Display for SetupPromptTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::OpenCode => "OpenCode",
            Self::Claude => "Claude",
            Self::Both => "Both",
        };

        write!(f, "{label}")
    }
}

pub fn resolve_setup_dispatch<P>(mode: SetupMode, prompter: &P) -> Result<SetupDispatch>
where
    P: SetupTargetPrompter,
{
    match mode {
        SetupMode::Interactive => prompter.prompt_target(),
        SetupMode::NonInteractive(target) => {
            Ok(SetupDispatch::Proceed(SetupMode::NonInteractive(target)))
        }
    }
}

pub fn setup_cancelled_text() -> &'static str {
    "Setup cancelled. No files were changed."
}

pub fn setup_usage_text() -> &'static str {
    "Usage:\n  sce setup [--opencode|--claude|--both] [--non-interactive] [--hooks] [--repo <path>]\n\nExamples:\n  sce setup\n  sce setup --opencode --non-interactive --hooks\n  sce setup --both --non-interactive\n  sce setup --hooks\n  sce setup --hooks --repo ../demo-repo\n  sce setup --opencode --non-interactive --hooks && sce doctor --format json\n\nWithout a target flag, setup defaults to interactive target selection.\nDefault interactive setup installs selected config assets and required hooks in one run.\nUse '--non-interactive' to fail fast instead of prompting; it requires '--opencode', '--claude', or '--both' when running config setup.\nTarget flags are mutually exclusive and intended for non-interactive automation.\n'--hooks' installs required git hooks for the current repository by default, or for '--repo <path>' when provided.\nLegacy one-purpose invocations remain supported: target-only runs install config assets, and '--hooks' without a target installs hooks only."
}

pub fn parse_setup_cli_options<I>(args: I) -> Result<SetupCliOptions>
where
    I: IntoIterator<Item = String>,
{
    let mut parser = lexopt::Parser::from_args(args);
    let mut options = SetupCliOptions::default();

    while let Some(arg) = parser.next()? {
        match arg {
            Arg::Long("non-interactive") => options.non_interactive = true,
            Arg::Long("opencode") => options.opencode = true,
            Arg::Long("claude") => options.claude = true,
            Arg::Long("both") => options.both = true,
            Arg::Long("hooks") => options.hooks = true,
            Arg::Long("repo") => {
                let value = parser
                    .value()
                    .context(
                        "Option '--repo' requires a path value. Try: 'sce setup --hooks --repo ../demo-repo'.",
                    )?;
                if options.repo_path.is_some() {
                    bail!(
                        "Option '--repo' may only be provided once. Try: keep a single '--repo <path>' value and rerun."
                    );
                }
                options.repo_path = Some(PathBuf::from(value.string()?));
            }
            Arg::Long("help") | Arg::Short('h') => options.help = true,
            Arg::Long(option) => {
                bail!(
                    "Unknown setup option '--{}'. Try: run 'sce setup --help' to see supported setup options.",
                    option
                );
            }
            Arg::Short(option) => {
                bail!(
                    "Unknown setup option '-{}'. Try: run 'sce setup --help' to see supported setup options.",
                    option
                );
            }
            Arg::Value(value) => {
                let value = value.string()?;
                bail!(
                    "Unexpected setup argument '{}'. Try: remove the extra argument and use 'sce setup --help' for supported forms.",
                    value
                );
            }
        }
    }

    Ok(options)
}

#[cfg(test)]
mod tests;
