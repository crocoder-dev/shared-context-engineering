use std::process::ExitCode;

use anyhow::{bail, Result};
use lexopt::{Arg, ValueExt};

use crate::{command_surface, dependency_contract, services};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Command {
    Help,
    Setup,
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
    let mut args = args.into_iter();
    let _program = args.next();

    let mut parser = lexopt::Parser::from_args(args);
    let mut command = None;

    while let Some(arg) = parser.next()? {
        match arg {
            Arg::Long("help") | Arg::Short('h') => {
                if command.is_some() {
                    bail!("'--help' must be used by itself. Run 'sce --help'.");
                }
                command = Some(Command::Help);
            }
            Arg::Value(value) => {
                let value = value.string()?;

                if command.is_some() {
                    bail!(
                        "Unexpected extra argument '{}'. Run 'sce --help' to see valid usage.",
                        value
                    );
                }

                command = Some(parse_subcommand(&value)?);
            }
            Arg::Long(option) => {
                bail!(
                    "Unknown option '--{}'. Run 'sce --help' to see valid usage.",
                    option
                );
            }
            Arg::Short(option) => {
                bail!(
                    "Unknown option '-{}'. Run 'sce --help' to see valid usage.",
                    option
                );
            }
        }
    }

    Ok(command.unwrap_or(Command::Help))
}

fn parse_subcommand(value: &str) -> Result<Command> {
    match value {
        "help" => Ok(Command::Help),
        "setup" => Ok(Command::Setup),
        "mcp" => Ok(Command::Mcp),
        "hooks" => Ok(Command::Hooks),
        "sync" => Ok(Command::Sync),
        _ => {
            if command_surface::is_known_command(value) {
                bail!(
                    "Command '{}' is currently unavailable in this build.",
                    value
                );
            }

            bail!(
                "Unknown command '{}'. Run 'sce --help' to see the current command surface.",
                value
            );
        }
    }
}

fn dispatch(command: Command) -> Result<()> {
    match command {
        Command::Help => println!("{}", command_surface::help_text()),
        Command::Setup => println!("TODO: 'setup' is planned and not implemented yet."),
        Command::Mcp => println!("TODO: 'mcp' is planned and not implemented yet."),
        Command::Hooks => println!("TODO: 'hooks' is planned and not implemented yet."),
        Command::Sync => println!("{}", services::sync::run_placeholder_sync()?),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::ExitCode;

    use super::{parse_command, run, Command};

    #[test]
    fn help_path_exits_success() {
        let code = run(vec!["sce".to_string(), "--help".to_string()]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn placeholder_command_exits_success() {
        let code = run(vec!["sce".to_string(), "setup".to_string()]);
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
            "Unexpected extra argument 'extra'. Run 'sce --help' to see valid usage."
        );
    }
}
