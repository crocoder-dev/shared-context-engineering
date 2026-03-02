use anyhow::{bail, Result};
use inquire::{InquireError, Select};
use lexopt::{Arg, ValueExt};

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupRequest {
    pub repository_root: String,
    pub mode: SetupMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupPlan {
    pub tasks: Vec<&'static str>,
    pub ready_for_execution: bool,
}

pub trait SetupService {
    fn plan(&self, request: &SetupRequest) -> SetupPlan;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PlaceholderSetupService;

impl SetupService for PlaceholderSetupService {
    fn plan(&self, _request: &SetupRequest) -> SetupPlan {
        SetupPlan {
            tasks: vec![
                "Validate repository shape",
                "Initialize local development prerequisites",
                "Persist setup state for future runs",
            ],
            ready_for_execution: false,
        }
    }
}

pub fn run_placeholder_setup_for_mode(mode: SetupMode) -> Result<String> {
    let service = PlaceholderSetupService;
    let request = SetupRequest {
        repository_root: ".".to_string(),
        mode,
    };
    let plan = service.plan(&request);

    let mode_label = match mode {
        SetupMode::Interactive => "interactive selection",
        SetupMode::NonInteractive(SetupTarget::OpenCode) => "--opencode",
        SetupMode::NonInteractive(SetupTarget::Claude) => "--claude",
        SetupMode::NonInteractive(SetupTarget::Both) => "--both",
    };

    let selected_target = match mode {
        SetupMode::Interactive => SetupTarget::Both,
        SetupMode::NonInteractive(target) => target,
    };
    let embedded_asset_count = iter_embedded_assets_for_setup_target(selected_target).count();

    Ok(format!(
        "TODO: '{NAME}' is planned and not implemented yet. Setup mode '{mode_label}' accepted; setup plan scaffolded with {} deferred step(s). Embedded asset manifest is ready with {embedded_asset_count} file(s).",
        plan.tasks.len()
    ))
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
        fs,
        path::{Path, PathBuf},
    };

    use anyhow::Result;

    use super::{
        iter_embedded_assets_for_setup_target, parse_setup_cli_options, resolve_setup_dispatch,
        resolve_setup_mode, run_placeholder_setup_for_mode, setup_usage_text,
        PlaceholderSetupService, SetupCliOptions, SetupDispatch, SetupMode, SetupRequest,
        SetupService, SetupTarget,
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
    fn setup_placeholder_service_exposes_deferred_plan() {
        let service = PlaceholderSetupService;
        let plan = service.plan(&SetupRequest {
            repository_root: ".".to_string(),
            mode: SetupMode::Interactive,
        });

        assert_eq!(plan.tasks.len(), 3);
        assert!(!plan.ready_for_execution);
    }

    #[test]
    fn setup_placeholder_message_mentions_scaffolded_plan() -> Result<()> {
        let message = run_placeholder_setup_for_mode(SetupMode::Interactive)?;
        assert!(message.contains("setup plan scaffolded"));
        Ok(())
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
    fn setup_placeholder_message_mentions_flag_mode() -> Result<()> {
        let message = run_placeholder_setup_for_mode(SetupMode::NonInteractive(SetupTarget::Both))?;
        assert!(message.contains("Setup mode '--both' accepted"));
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
}
