use anyhow::{bail, Context, Result};
use inquire::{InquireError, Select};
use lexopt::{Arg, ValueExt};
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
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

include!(concat!(env!("OUT_DIR"), "/setup_embedded_assets.rs"));

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
        time::{SystemTime, UNIX_EPOCH},
    };

    use anyhow::Result;

    use super::{
        install_embedded_setup_assets, install_embedded_setup_assets_with_rename,
        iter_embedded_assets_for_setup_target, parse_setup_cli_options, resolve_setup_dispatch,
        resolve_setup_mode, run_setup_for_mode, setup_usage_text, SetupCliOptions, SetupDispatch,
        SetupMode, SetupTarget,
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
        let temp = TestTempDir::new().expect("temp dir should be created");
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
        let temp = TestTempDir::new()?;
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
        let temp = TestTempDir::new()?;
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
    fn install_engine_replaces_existing_target_with_backup() -> Result<()> {
        let temp = TestTempDir::new()?;
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
        let temp = TestTempDir::new()?;

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
        let temp = TestTempDir::new()?;
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

    #[derive(Debug)]
    struct TestTempDir {
        path: PathBuf,
    }

    impl TestTempDir {
        fn new() -> Result<Self> {
            let epoch_nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "sce-setup-install-tests-{}-{}",
                std::process::id(),
                epoch_nanos
            ));
            fs::create_dir_all(&path)?;
            Ok(Self { path })
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestTempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
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
}
