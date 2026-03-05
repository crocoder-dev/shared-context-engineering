use std::process::ExitCode;

use crate::{command_surface, dependency_contract, services};
use anyhow::{bail, Context, Result};
use lexopt::ValueExt;

#[derive(Clone, Debug, Eq, PartialEq)]
enum Command {
    Help,
    Setup(services::setup::SetupMode),
    SetupHooks(Option<std::path::PathBuf>),
    SetupHelp,
    Doctor,
    Mcp,
    Hooks,
    Sync,
}

pub fn run<I>(args: I) -> ExitCode
where
    I: IntoIterator<Item = String>,
{
    match try_run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::from(2)
        }
    }
}

fn try_run<I>(args: I) -> Result<()>
where
    I: IntoIterator<Item = String>,
{
    let _ = dependency_contract::dependency_contract_snapshot();
    let command = parse_command(args)?;
    dispatch(command)
}

fn parse_command<I>(args: I) -> Result<Command>
where
    I: IntoIterator<Item = String>,
{
    let mut argv = args.into_iter();
    let Some(_program) = argv.next() else {
        return Ok(Command::Help);
    };

    let tail_args: Vec<String> = argv.collect();
    if tail_args.is_empty() {
        return Ok(Command::Help);
    }

    let mut parser = lexopt::Parser::from_args(tail_args.iter().map(String::as_str));
    match parser.next()? {
        Some(lexopt::Arg::Long("help")) => {
            if tail_args.len() == 1 {
                Ok(Command::Help)
            } else {
                bail!("{}", unknown_option_message("--help"))
            }
        }
        Some(lexopt::Arg::Short('h')) => {
            if tail_args.len() == 1 {
                Ok(Command::Help)
            } else {
                bail!("{}", unknown_option_message("-h"))
            }
        }
        Some(lexopt::Arg::Long(option)) => {
            bail!("{}", unknown_option_message(&format!("--{option}")))
        }
        Some(lexopt::Arg::Short(option)) => {
            bail!("{}", unknown_option_message(&format!("-{option}")))
        }
        Some(lexopt::Arg::Value(value)) => {
            let subcommand = value.string()?;
            parse_subcommand(subcommand, tail_args.into_iter().skip(1).collect())
        }
        None => Ok(Command::Help),
    }
}

fn unknown_option_message(option: &str) -> String {
    format!(
        "Unknown option '{}'. Run 'sce --help' to see valid usage.",
        option
    )
}

fn parse_subcommand(value: String, tail_args: Vec<String>) -> Result<Command> {
    match value.as_str() {
        "help" => Ok(Command::Help),
        "setup" => parse_setup_subcommand(tail_args),
        "doctor" => parse_non_setup_subcommand(Command::Doctor, tail_args),
        "mcp" => parse_non_setup_subcommand(Command::Mcp, tail_args),
        "hooks" => parse_non_setup_subcommand(Command::Hooks, tail_args),
        "sync" => parse_non_setup_subcommand(Command::Sync, tail_args),
        _ => {
            if command_surface::is_known_command(&value) {
                bail!(
                    "Command '{}' is currently unavailable in this build.",
                    value,
                );
            }

            bail!(
                "Unknown command '{}'. Run 'sce --help' to see the current command surface.",
                value,
            );
        }
    }
}

fn parse_setup_subcommand(args: Vec<String>) -> Result<Command> {
    let options = services::setup::parse_setup_cli_options(args)?;

    if options.help {
        return Ok(Command::SetupHelp);
    }

    if options.hooks {
        let repo_path = services::setup::resolve_setup_hooks_repository(&options)?;
        return Ok(Command::SetupHooks(repo_path));
    }

    services::setup::resolve_setup_hooks_repository(&options)?;

    let mode = services::setup::resolve_setup_mode(options)?;
    Ok(Command::Setup(mode))
}

fn parse_non_setup_subcommand(command: Command, tail_args: Vec<String>) -> Result<Command> {
    if tail_args.is_empty() {
        return Ok(command);
    }

    bail!(
        "Unexpected extra argument '{}'. Run 'sce --help' to see valid usage.",
        tail_args[0]
    );
}

fn dispatch(command: Command) -> Result<()> {
    match command {
        Command::Help => println!("{}", command_surface::help_text()),
        Command::Setup(mode) => {
            let dispatch = services::setup::resolve_setup_dispatch(
                mode,
                &services::setup::InquireSetupTargetPrompter,
            )?;

            match dispatch {
                services::setup::SetupDispatch::Proceed(mode) => {
                    let repository_root =
                        std::env::current_dir().context("Failed to determine current directory")?;
                    println!(
                        "{}",
                        services::setup::run_setup_for_mode(&repository_root, mode)?
                    );
                }
                services::setup::SetupDispatch::Cancelled => {
                    println!("{}", services::setup::setup_cancelled_text());
                }
            }
        }
        Command::SetupHooks(repo_path) => {
            let current_dir =
                std::env::current_dir().context("Failed to determine current directory")?;
            let repository_root = repo_path.as_deref().unwrap_or(current_dir.as_path());
            println!("{}", services::setup::run_setup_hooks(repository_root)?);
        }
        Command::SetupHelp => println!("{}", services::setup::setup_usage_text()),
        Command::Doctor => println!("{}", services::doctor::run_doctor()?),
        Command::Mcp => println!("{}", services::mcp::run_placeholder_mcp()?),
        Command::Hooks => println!("{}", services::hooks::run_placeholder_hooks()?),
        Command::Sync => println!("{}", services::sync::run_placeholder_sync()?),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::ExitCode;

    use crate::services::setup::{SetupMode, SetupTarget};

    use super::{parse_command, run, Command};

    #[test]
    fn help_path_exits_success() {
        let code = run(vec!["sce".to_string(), "--help".to_string()]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn hooks_command_exits_success() {
        let code = run(vec!["sce".to_string(), "hooks".to_string()]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn doctor_command_exits_success() {
        let code = run(vec!["sce".to_string(), "doctor".to_string()]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn setup_help_exits_success() {
        let code = run(vec![
            "sce".to_string(),
            "setup".to_string(),
            "--help".to_string(),
        ]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn sync_command_exits_success() {
        let code = run(vec!["sce".to_string(), "sync".to_string()]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn unknown_command_exits_non_zero() {
        let code = run(vec!["sce".to_string(), "does-not-exist".to_string()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn parser_defaults_to_help_without_command() {
        let command = parse_command(vec!["sce".to_string()]).expect("command should parse");
        assert_eq!(command, Command::Help);
    }

    #[test]
    fn parser_routes_placeholder_command() {
        let command = parse_command(vec!["sce".to_string(), "hooks".to_string()])
            .expect("command should parse");
        assert_eq!(command, Command::Hooks);
    }

    #[test]
    fn parser_routes_setup_opencode_flag_to_non_interactive_mode() {
        let command = parse_command(vec![
            "sce".to_string(),
            "setup".to_string(),
            "--opencode".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(
            command,
            Command::Setup(SetupMode::NonInteractive(SetupTarget::OpenCode,))
        );
    }

    #[test]
    fn parser_routes_setup_claude_flag_to_non_interactive_mode() {
        let command = parse_command(vec![
            "sce".to_string(),
            "setup".to_string(),
            "--claude".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(
            command,
            Command::Setup(SetupMode::NonInteractive(SetupTarget::Claude,))
        );
    }

    #[test]
    fn parser_routes_setup_both_flag_to_non_interactive_mode() {
        let command = parse_command(vec![
            "sce".to_string(),
            "setup".to_string(),
            "--both".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(
            command,
            Command::Setup(SetupMode::NonInteractive(SetupTarget::Both,))
        );
    }

    #[test]
    fn parser_routes_setup_without_flags_to_interactive_mode() {
        let command = parse_command(vec!["sce".to_string(), "setup".to_string()])
            .expect("command should parse");
        assert_eq!(command, Command::Setup(SetupMode::Interactive));
    }

    #[test]
    fn parser_routes_setup_hooks_without_repo() {
        let command = parse_command(vec![
            "sce".to_string(),
            "setup".to_string(),
            "--hooks".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(command, Command::SetupHooks(None));
    }

    #[test]
    fn parser_routes_setup_hooks_with_repo() {
        let command = parse_command(vec![
            "sce".to_string(),
            "setup".to_string(),
            "--hooks".to_string(),
            "--repo".to_string(),
            "../demo-repo".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(
            command,
            Command::SetupHooks(Some(std::path::PathBuf::from("../demo-repo")))
        );
    }

    #[test]
    fn parser_rejects_setup_mutually_exclusive_flags() {
        let error = parse_command(vec![
            "sce".to_string(),
            "setup".to_string(),
            "--opencode".to_string(),
            "--claude".to_string(),
        ])
        .expect_err("mutually exclusive flags should fail");
        assert_eq!(
            error.to_string(),
            "Options '--opencode', '--claude', and '--both' are mutually exclusive. Choose exactly one target flag or none for interactive mode."
        );
    }

    #[test]
    fn parser_rejects_setup_repo_without_hooks() {
        let error = parse_command(vec![
            "sce".to_string(),
            "setup".to_string(),
            "--repo".to_string(),
            "../demo-repo".to_string(),
        ])
        .expect_err("--repo without --hooks should fail");
        assert_eq!(
            error.to_string(),
            "Option '--repo' requires '--hooks'. Run 'sce setup --help' to see valid usage."
        );
    }

    #[test]
    fn parser_rejects_hooks_with_target_flag() {
        let error = parse_command(vec![
            "sce".to_string(),
            "setup".to_string(),
            "--hooks".to_string(),
            "--opencode".to_string(),
        ])
        .expect_err("--hooks with target flag should fail");
        assert_eq!(
            error.to_string(),
            "Option '--hooks' cannot be combined with '--opencode', '--claude', or '--both'. Run 'sce setup --help' to see valid usage."
        );
    }

    #[test]
    fn parser_rejects_unknown_command() {
        let error = parse_command(vec!["sce".to_string(), "nope".to_string()])
            .expect_err("unknown command should fail");
        assert_eq!(
            error.to_string(),
            "Unknown command 'nope'. Run 'sce --help' to see the current command surface."
        );
    }

    #[test]
    fn parser_rejects_unknown_option() {
        let error = parse_command(vec!["sce".to_string(), "--verbose".to_string()])
            .expect_err("unknown option should fail");
        assert_eq!(
            error.to_string(),
            "Unknown option '--verbose'. Run 'sce --help' to see valid usage."
        );
    }

    #[test]
    fn parser_rejects_extra_arguments() {
        let error = parse_command(vec![
            "sce".to_string(),
            "setup".to_string(),
            "extra".to_string(),
        ])
        .expect_err("extra argument should fail");
        assert_eq!(
            error.to_string(),
            "Unexpected setup argument 'extra'. Run 'sce setup --help' to see valid usage."
        );
    }
}
