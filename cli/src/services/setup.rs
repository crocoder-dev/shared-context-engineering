use anyhow::{bail, Result};
use lexopt::{Arg, ValueExt};

pub const NAME: &str = "setup";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupTarget {
    OpenCode,
    Claude,
    Both,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupMode {
    Interactive,
    NonInteractive(SetupTarget),
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

    Ok(format!(
        "TODO: '{NAME}' is planned and not implemented yet. Setup mode '{mode_label}' accepted; setup plan scaffolded with {} deferred step(s).",
        plan.tasks.len(),
    ))
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
    use anyhow::Result;

    use super::{
        parse_setup_cli_options, resolve_setup_mode, run_placeholder_setup_for_mode,
        setup_usage_text, PlaceholderSetupService, SetupCliOptions, SetupMode, SetupRequest,
        SetupService, SetupTarget,
    };

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
}
