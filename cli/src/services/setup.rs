use anyhow::{bail, Context, Result};
use inquire::{InquireError, Select};
use lexopt::{Arg, ValueExt};
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SetupCliOptions {
    pub help: bool,
    pub opencode: bool,
    pub claude: bool,
    pub both: bool,
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
    let hooks_directory = resolve_git_hooks_directory(&resolved_repository_root)?;
    fs::create_dir_all(&hooks_directory).with_context(|| {
        format!(
            "Failed to create git hooks directory '{}'",
            hooks_directory.display()
        )
    })?;

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
            stderr
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
                "Interactive setup requires a TTY. Re-run with '--opencode', '--claude', or '--both' for non-interactive automation."
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
    "Usage: sce setup [--opencode|--claude|--both]\n\nWithout a target flag, setup defaults to interactive target selection.\nTarget flags are mutually exclusive and intended for non-interactive automation."
}

pub fn parse_setup_cli_options<I>(args: I) -> Result<SetupCliOptions>
where
    I: IntoIterator<Item = String>,
{
    let mut parser = lexopt::Parser::from_args(args);
    let mut options = SetupCliOptions::default();

    while let Some(arg) = parser.next()? {
        match arg {
            Arg::Long("opencode") => options.opencode = true,
            Arg::Long("claude") => options.claude = true,
            Arg::Long("both") => options.both = true,
            Arg::Long("help") | Arg::Short('h') => options.help = true,
            Arg::Long(option) => {
                bail!(
                    "Unknown setup option '--{}'. Run 'sce setup --help' to see valid usage.",
                    option
                );
            }
            Arg::Short(option) => {
                bail!(
                    "Unknown setup option '-{}'. Run 'sce setup --help' to see valid usage.",
                    option
                );
            }
            Arg::Value(value) => {
                let value = value.string()?;
                bail!(
                    "Unexpected setup argument '{}'. Run 'sce setup --help' to see valid usage.",
                    value
                );
            }
        }
    }

    Ok(options)
}

pub fn resolve_setup_mode(options: SetupCliOptions) -> Result<SetupMode> {
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

    match selected_targets.as_slice() {
        [] => Ok(SetupMode::Interactive),
        [target] => Ok(SetupMode::NonInteractive(*target)),
        _ => bail!(
            "Options '--opencode', '--claude', and '--both' are mutually exclusive. Choose exactly one target flag or none for interactive mode."
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        fs, io,
        path::{Path, PathBuf},
        process::Command,
    };

    use crate::test_support::TestTempDir;
    use anyhow::Result;

    use super::{
        get_required_hook_asset, install_embedded_setup_assets,
        install_embedded_setup_assets_with_rename, install_required_git_hooks,
        install_required_git_hooks_with_rename, iter_embedded_assets_for_setup_target,
        iter_required_hook_assets, parse_setup_cli_options, resolve_setup_dispatch,
        resolve_setup_mode, run_setup_for_mode, setup_usage_text, RequiredHookAsset,
        RequiredHookInstallStatus, SetupCliOptions, SetupDispatch, SetupMode, SetupTarget,
    };

    #[derive(Clone, Copy, Debug)]
    struct MockPrompter {
        response: SetupDispatch,
    }

    impl super::SetupTargetPrompter for MockPrompter {
        fn prompt_target(&self) -> Result<SetupDispatch> {
            Ok(self.response)
        }
    }

    #[test]
    fn run_setup_rejects_unresolved_interactive_mode() {
        let temp = TestTempDir::new("sce-setup-install-tests").expect("temp dir should be created");
        let error = run_setup_for_mode(temp.path(), SetupMode::Interactive)
            .expect_err("interactive mode should be resolved before install");
        assert_eq!(
            error.to_string(),
            "Interactive setup mode must be resolved before installation"
        );
    }

    #[test]
    fn setup_options_default_to_interactive_mode() -> Result<()> {
        let options = parse_setup_cli_options(Vec::<String>::new())?;
        let mode = resolve_setup_mode(options)?;
        assert_eq!(mode, SetupMode::Interactive);
        Ok(())
    }

    #[test]
    fn setup_options_parse_opencode_flag() -> Result<()> {
        let options = parse_setup_cli_options(vec!["--opencode".to_string()])?;
        let mode = resolve_setup_mode(options)?;
        assert_eq!(mode, SetupMode::NonInteractive(SetupTarget::OpenCode));
        Ok(())
    }

    #[test]
    fn setup_options_reject_mutually_exclusive_flags() {
        let error = resolve_setup_mode(SetupCliOptions {
            help: false,
            opencode: true,
            claude: true,
            both: false,
        })
        .expect_err("multiple target flags should fail");

        assert_eq!(
            error.to_string(),
            "Options '--opencode', '--claude', and '--both' are mutually exclusive. Choose exactly one target flag or none for interactive mode."
        );
    }

    #[test]
    fn setup_usage_contract_mentions_target_flags() {
        let usage = setup_usage_text();
        assert!(usage.contains("--opencode|--claude|--both"));
    }

    #[test]
    fn setup_help_option_sets_help_flag() -> Result<()> {
        let options = parse_setup_cli_options(vec!["--help".to_string()])?;
        assert!(options.help);
        Ok(())
    }

    #[test]
    fn run_setup_reports_selected_target_and_backup_status() -> Result<()> {
        let temp = TestTempDir::new("sce-setup-install-tests")?;
        fs::create_dir_all(temp.path().join(".opencode/legacy"))?;
        fs::write(temp.path().join(".opencode/legacy/config.txt"), b"legacy")?;

        let message = run_setup_for_mode(
            temp.path(),
            SetupMode::NonInteractive(SetupTarget::OpenCode),
        )?;
        assert!(message.contains("Setup completed successfully."));
        assert!(message.contains("Selected target(s): OpenCode"));
        assert!(message.contains("OpenCode: installed"));
        assert!(message.contains("backup: existing target moved to"));
        assert!(message.contains(".opencode.backup"));

        Ok(())
    }

    #[test]
    fn run_setup_reports_both_targets() -> Result<()> {
        let temp = TestTempDir::new("sce-setup-install-tests")?;
        let message =
            run_setup_for_mode(temp.path(), SetupMode::NonInteractive(SetupTarget::Both))?;
        assert!(message.contains("Selected target(s): OpenCode, Claude"));
        assert!(message.contains("OpenCode: installed"));
        assert!(message.contains("Claude: installed"));
        assert!(message.contains("backup: not needed (no existing target)"));
        Ok(())
    }

    #[test]
    fn interactive_dispatch_maps_selected_target() -> Result<()> {
        let dispatch = resolve_setup_dispatch(
            SetupMode::Interactive,
            &MockPrompter {
                response: SetupDispatch::Proceed(SetupMode::NonInteractive(SetupTarget::Claude)),
            },
        )?;

        assert_eq!(
            dispatch,
            SetupDispatch::Proceed(SetupMode::NonInteractive(SetupTarget::Claude))
        );
        Ok(())
    }

    #[test]
    fn interactive_dispatch_returns_cancelled_without_side_effects() -> Result<()> {
        let dispatch = resolve_setup_dispatch(
            SetupMode::Interactive,
            &MockPrompter {
                response: SetupDispatch::Cancelled,
            },
        )?;

        assert_eq!(dispatch, SetupDispatch::Cancelled);
        Ok(())
    }

    #[test]
    fn embedded_manifest_paths_are_sorted_and_normalized() {
        for target in [SetupTarget::OpenCode, SetupTarget::Claude] {
            let assets = assets_for_target(target);

            assert!(!assets.is_empty(), "embedded asset set should not be empty");

            let paths: Vec<&str> = assets.iter().map(|asset| asset.relative_path).collect();
            assert_eq!(paths.len(), assets.len());

            for asset in assets {
                assert!(!asset.relative_path.is_empty());
                assert!(!asset.relative_path.starts_with('/'));
                assert!(!asset.relative_path.contains('\\'));
                assert!(!asset.relative_path.starts_with("config/"));
                assert!(
                    !asset.bytes.is_empty(),
                    "embedded files should have content bytes"
                );
            }

            let mut sorted = paths.clone();
            sorted.sort_unstable();
            assert_eq!(
                paths, sorted,
                "embedded paths should be deterministic and sorted"
            );
        }
    }

    #[test]
    fn embedded_manifest_matches_runtime_config_tree() -> Result<()> {
        let opencode_expected =
            collect_runtime_relative_paths(runtime_target_root(SetupTarget::OpenCode))?;
        let claude_expected =
            collect_runtime_relative_paths(runtime_target_root(SetupTarget::Claude))?;

        let opencode_actual: Vec<String> = assets_for_target(SetupTarget::OpenCode)
            .iter()
            .map(|asset| asset.relative_path.to_string())
            .collect();
        let claude_actual: Vec<String> = assets_for_target(SetupTarget::Claude)
            .iter()
            .map(|asset| asset.relative_path.to_string())
            .collect();

        assert_eq!(opencode_actual, opencode_expected);
        assert_eq!(claude_actual, claude_expected);
        Ok(())
    }

    #[test]
    fn embedded_setup_target_iterator_scopes_assets_per_target() {
        let opencode_count = assets_for_target(SetupTarget::OpenCode).len();
        let claude_count = assets_for_target(SetupTarget::Claude).len();

        let iter_opencode_count =
            iter_embedded_assets_for_setup_target(SetupTarget::OpenCode).count();
        let iter_claude_count = iter_embedded_assets_for_setup_target(SetupTarget::Claude).count();
        let iter_both_count = iter_embedded_assets_for_setup_target(SetupTarget::Both).count();

        assert_eq!(iter_opencode_count, opencode_count);
        assert_eq!(iter_claude_count, claude_count);
        assert_eq!(iter_both_count, opencode_count + claude_count);
    }

    #[test]
    fn embedded_hook_manifest_is_complete_sorted_and_normalized() {
        let hooks: Vec<&super::EmbeddedAsset> = iter_required_hook_assets().collect();
        let paths: Vec<&str> = hooks.iter().map(|asset| asset.relative_path).collect();

        assert_eq!(paths, vec!["commit-msg", "post-commit", "pre-commit"]);

        for hook in hooks {
            assert!(!hook.relative_path.is_empty());
            assert!(!hook.relative_path.contains('/'));
            assert!(!hook.relative_path.contains('\\'));
            assert!(!hook.bytes.is_empty());
            assert!(
                hook.bytes.starts_with(b"#!/bin/sh\n"),
                "embedded hook should start with shell shebang"
            );
        }
    }

    #[test]
    fn required_hook_lookup_resolves_each_canonical_hook() {
        for hook in [
            RequiredHookAsset::PreCommit,
            RequiredHookAsset::CommitMsg,
            RequiredHookAsset::PostCommit,
        ] {
            let asset = get_required_hook_asset(hook).expect("required hook asset should exist");
            assert_eq!(asset.relative_path, hook_filename(hook));
            assert!(!asset.bytes.is_empty());
        }
    }

    #[test]
    fn install_engine_replaces_existing_target_with_backup() -> Result<()> {
        let temp = TestTempDir::new("sce-setup-install-tests")?;
        let existing_target = temp.path().join(".opencode");
        fs::create_dir_all(existing_target.join("legacy"))?;
        fs::write(existing_target.join("legacy/config.txt"), b"legacy")?;

        let outcome = install_embedded_setup_assets(temp.path(), SetupTarget::OpenCode)?;
        assert_eq!(outcome.target_results.len(), 1);

        let result = &outcome.target_results[0];
        assert_eq!(result.target, SetupTarget::OpenCode);
        assert_eq!(result.destination_root, temp.path().join(".opencode"));
        assert_eq!(
            result.installed_file_count,
            assets_for_target(SetupTarget::OpenCode).len()
        );

        let backup_root = result
            .backup_root
            .as_ref()
            .expect("existing target should have backup path");
        assert!(backup_root.exists());
        assert!(backup_root.join("legacy/config.txt").exists());

        let installed_paths = collect_runtime_relative_paths(result.destination_root.clone())?;
        let expected_paths: Vec<String> = assets_for_target(SetupTarget::OpenCode)
            .iter()
            .map(|asset| asset.relative_path.to_string())
            .collect();
        assert_eq!(installed_paths, expected_paths);
        Ok(())
    }

    #[test]
    fn install_engine_installs_both_targets() -> Result<()> {
        let temp = TestTempDir::new("sce-setup-install-tests")?;

        let outcome = install_embedded_setup_assets(temp.path(), SetupTarget::Both)?;
        assert_eq!(outcome.target_results.len(), 2);

        let opencode_paths = collect_runtime_relative_paths(temp.path().join(".opencode"))?;
        let claude_paths = collect_runtime_relative_paths(temp.path().join(".claude"))?;

        let expected_opencode: Vec<String> = assets_for_target(SetupTarget::OpenCode)
            .iter()
            .map(|asset| asset.relative_path.to_string())
            .collect();
        let expected_claude: Vec<String> = assets_for_target(SetupTarget::Claude)
            .iter()
            .map(|asset| asset.relative_path.to_string())
            .collect();

        assert_eq!(opencode_paths, expected_opencode);
        assert_eq!(claude_paths, expected_claude);
        Ok(())
    }

    #[test]
    fn install_engine_rolls_back_when_swap_fails() -> Result<()> {
        let temp = TestTempDir::new("sce-setup-install-tests")?;
        let destination = temp.path().join(".opencode");
        fs::create_dir_all(&destination)?;
        fs::write(destination.join("legacy.txt"), b"legacy")?;

        let rename_calls = Cell::new(0_u8);
        let error = install_embedded_setup_assets_with_rename(
            temp.path(),
            SetupTarget::OpenCode,
            |from, to| {
                rename_calls.set(rename_calls.get() + 1);
                if rename_calls.get() == 2 {
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        "injected swap failure",
                    ));
                }

                fs::rename(from, to)
            },
        )
        .expect_err("swap failure should bubble up as an error");

        assert!(error.to_string().contains("Failed to swap staged install"));
        assert!(destination.exists());
        assert!(destination.join("legacy.txt").exists());

        let backup = temp.path().join(".opencode.backup");
        assert!(!backup.exists(), "rollback should restore original path");

        for entry in fs::read_dir(temp.path())? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            assert!(
                !name.starts_with(".sce-setup-staging-opencode-"),
                "staging directory should be cleaned up after failure"
            );
        }

        Ok(())
    }

    #[test]
    fn required_hook_install_installs_missing_hooks_in_default_directory() -> Result<()> {
        let temp = TestTempDir::new("sce-setup-hook-install-tests")?;
        init_git_repo(temp.path())?;

        let outcome = install_required_git_hooks(temp.path())?;
        assert_eq!(outcome.repository_root, temp.path().to_path_buf());
        assert_eq!(outcome.hook_results.len(), 3);
        for hook in outcome.hook_results {
            assert_eq!(hook.status, RequiredHookInstallStatus::Installed);
            assert!(hook.hook_path.exists());
            assert!(hook.backup_path.is_none());
            assert_hook_is_executable(&hook.hook_path)?;
        }

        Ok(())
    }

    #[test]
    fn required_hook_install_rerun_reports_skipped_for_unchanged_hooks() -> Result<()> {
        let temp = TestTempDir::new("sce-setup-hook-install-tests")?;
        init_git_repo(temp.path())?;

        let first = install_required_git_hooks(temp.path())?;
        assert!(first
            .hook_results
            .iter()
            .all(|hook| hook.status == RequiredHookInstallStatus::Installed));

        let second = install_required_git_hooks(temp.path())?;
        assert!(second
            .hook_results
            .iter()
            .all(|hook| hook.status == RequiredHookInstallStatus::Skipped));
        assert!(second
            .hook_results
            .iter()
            .all(|hook| hook.backup_path.is_none()));

        Ok(())
    }

    #[test]
    fn required_hook_install_updates_noncanonical_hook_in_custom_hooks_path() -> Result<()> {
        let temp = TestTempDir::new("sce-setup-hook-install-tests")?;
        init_git_repo(temp.path())?;

        run_git_in_repo(temp.path(), &["config", "core.hooksPath", ".githooks"])?;

        let custom_hooks_directory = temp.path().join(".githooks");
        fs::create_dir_all(&custom_hooks_directory)?;
        let commit_msg_path = custom_hooks_directory.join("commit-msg");
        fs::write(&commit_msg_path, b"#!/bin/sh\necho legacy\n")?;
        set_test_file_mode(&commit_msg_path, 0o644)?;

        let outcome = install_required_git_hooks(temp.path())?;
        assert_eq!(outcome.hooks_directory, custom_hooks_directory);

        let updated = outcome
            .hook_results
            .iter()
            .find(|hook| hook.hook_name == "commit-msg")
            .expect("commit-msg result should exist");
        assert_eq!(updated.status, RequiredHookInstallStatus::Updated);
        let backup_path = updated
            .backup_path
            .as_ref()
            .expect("updated hook should retain backup path");
        assert!(backup_path.exists());
        assert_eq!(fs::read(backup_path)?, b"#!/bin/sh\necho legacy\n");
        assert_hook_is_executable(&updated.hook_path)?;

        Ok(())
    }

    #[test]
    fn required_hook_install_rolls_back_when_hook_swap_fails() -> Result<()> {
        let temp = TestTempDir::new("sce-setup-hook-install-tests")?;
        init_git_repo(temp.path())?;

        let hooks_directory = temp.path().join(".git/hooks");
        fs::create_dir_all(&hooks_directory)?;
        let commit_msg_path = hooks_directory.join("commit-msg");
        fs::write(&commit_msg_path, b"#!/bin/sh\necho legacy\n")?;

        let rename_calls = Cell::new(0_u8);
        let error = install_required_git_hooks_with_rename(temp.path(), |from, to| {
            rename_calls.set(rename_calls.get() + 1);
            if rename_calls.get() == 2 {
                return Err(io::Error::other("injected hook swap failure"));
            }

            fs::rename(from, to)
        })
        .expect_err("hook swap failure should bubble up");

        assert!(error
            .to_string()
            .contains("Failed to update required hook 'commit-msg'"));
        assert!(commit_msg_path.exists());
        assert_eq!(fs::read(&commit_msg_path)?, b"#!/bin/sh\necho legacy\n");
        assert!(!hooks_directory.join("commit-msg.backup").exists());

        for entry in fs::read_dir(&hooks_directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            assert!(
                !name.starts_with(".sce-hook-staging-"),
                "hook staging file should be cleaned up after failure"
            );
        }

        Ok(())
    }

    fn init_git_repo(repository_root: &Path) -> Result<()> {
        run_git_in_repo(repository_root, &["init", "-q"])?;
        Ok(())
    }

    fn run_git_in_repo(repository_root: &Path, args: &[&str]) -> Result<()> {
        let status = Command::new("git")
            .args(args)
            .current_dir(repository_root)
            .status()?;
        if !status.success() {
            anyhow::bail!("git command failed for test repository")
        }
        Ok(())
    }

    #[cfg(unix)]
    fn set_test_file_mode(path: &Path, mode: u32) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn set_test_file_mode(_path: &Path, _mode: u32) -> Result<()> {
        Ok(())
    }

    #[cfg(unix)]
    fn assert_hook_is_executable(path: &Path) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let metadata = fs::metadata(path)?;
        assert!(metadata.permissions().mode() & 0o111 != 0);
        Ok(())
    }

    #[cfg(not(unix))]
    fn assert_hook_is_executable(path: &Path) -> Result<()> {
        assert!(path.exists());
        Ok(())
    }

    fn runtime_target_root(target: SetupTarget) -> PathBuf {
        let target_relative = match target {
            SetupTarget::OpenCode => "config/.opencode",
            SetupTarget::Claude => "config/.claude",
            SetupTarget::Both => unreachable!("both is not a concrete filesystem root"),
        };

        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("cli crate should be nested under repository root")
            .join(target_relative)
    }

    fn assets_for_target(target: SetupTarget) -> &'static [super::EmbeddedAsset] {
        match target {
            SetupTarget::OpenCode => super::OPENCODE_EMBEDDED_ASSETS,
            SetupTarget::Claude => super::CLAUDE_EMBEDDED_ASSETS,
            SetupTarget::Both => unreachable!("both is not a single embedded target"),
        }
    }

    fn collect_runtime_relative_paths(root: PathBuf) -> Result<Vec<String>> {
        let mut files = Vec::new();
        collect_runtime_files(&root, &root, &mut files)?;

        files.sort_unstable();

        let stable_paths = files
            .into_iter()
            .map(|path| {
                path.to_str()
                    .expect("runtime config path should be UTF-8")
                    .replace('\\', "/")
            })
            .collect();

        Ok(stable_paths)
    }

    fn collect_runtime_files(
        base_root: &Path,
        current_dir: &Path,
        output: &mut Vec<PathBuf>,
    ) -> Result<()> {
        for entry in fs::read_dir(current_dir)? {
            let entry = entry?;
            let path = entry.path();

            if entry.file_type()?.is_dir() {
                collect_runtime_files(base_root, &path, output)?;
                continue;
            }

            let relative = path
                .strip_prefix(base_root)
                .expect("relative path should be under root")
                .to_path_buf();
            output.push(relative);
        }

        Ok(())
    }

    fn hook_filename(hook: RequiredHookAsset) -> &'static str {
        match hook {
            RequiredHookAsset::PreCommit => "pre-commit",
            RequiredHookAsset::CommitMsg => "commit-msg",
            RequiredHookAsset::PostCommit => "post-commit",
        }
    }
}
