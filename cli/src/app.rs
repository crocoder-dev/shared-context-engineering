use std::process::ExitCode;

use crate::{command_surface, dependency_contract, services};
use anyhow::Context;
use lexopt::ValueExt;

const EXIT_CODE_PARSE_FAILURE: u8 = 2;
const EXIT_CODE_VALIDATION_FAILURE: u8 = 3;
const EXIT_CODE_RUNTIME_FAILURE: u8 = 4;
const EXIT_CODE_DEPENDENCY_FAILURE: u8 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FailureClass {
    Parse,
    Validation,
    Runtime,
    Dependency,
}

impl FailureClass {
    fn exit_code(self) -> u8 {
        match self {
            Self::Parse => EXIT_CODE_PARSE_FAILURE,
            Self::Validation => EXIT_CODE_VALIDATION_FAILURE,
            Self::Runtime => EXIT_CODE_RUNTIME_FAILURE,
            Self::Dependency => EXIT_CODE_DEPENDENCY_FAILURE,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::Validation => "validation",
            Self::Runtime => "runtime",
            Self::Dependency => "dependency",
        }
    }
}

#[derive(Debug)]
struct ClassifiedError {
    class: FailureClass,
    message: String,
}

impl ClassifiedError {
    fn parse(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Parse,
            message: message.into(),
        }
    }

    fn validation(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Validation,
            message: message.into(),
        }
    }

    fn runtime(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Runtime,
            message: message.into(),
        }
    }

    fn dependency(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Dependency,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ClassifiedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ClassifiedError {}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Command {
    Help,
    Config(services::config::ConfigSubcommand),
    Setup(services::setup::SetupMode),
    SetupHooks(Option<std::path::PathBuf>),
    SetupHelp,
    Doctor,
    Mcp,
    Hooks(services::hooks::HookSubcommand),
    Sync,
}

impl Command {
    fn name(&self) -> &'static str {
        match self {
            Self::Help => "help",
            Self::Config(_) => services::config::NAME,
            Self::Setup(_) | Self::SetupHooks(_) | Self::SetupHelp => services::setup::NAME,
            Self::Doctor => services::doctor::NAME,
            Self::Mcp => services::mcp::NAME,
            Self::Hooks(_) => services::hooks::NAME,
            Self::Sync => services::sync::NAME,
        }
    }
}

pub fn run<I>(args: I) -> ExitCode
where
    I: IntoIterator<Item = String>,
{
    run_with_dependency_check(args, || {
        dependency_contract::dependency_contract_snapshot().0
    })
}

fn run_with_dependency_check<I, F>(args: I, dependency_check: F) -> ExitCode
where
    I: IntoIterator<Item = String>,
    F: FnOnce() -> anyhow::Result<()>,
{
    match try_run_with_dependency_check(args, dependency_check) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::from(error.class.exit_code())
        }
    }
}

fn try_run_with_dependency_check<I, F>(args: I, dependency_check: F) -> Result<(), ClassifiedError>
where
    I: IntoIterator<Item = String>,
    F: FnOnce() -> anyhow::Result<()>,
{
    dependency_check().map_err(|error| {
        ClassifiedError::dependency(format!("Failed to initialize dependency contract: {error}"))
    })?;

    let logger = services::observability::Logger::from_env().map_err(|error| {
        ClassifiedError::validation(format!("Invalid observability configuration: {error}"))
    })?;
    let telemetry = services::observability::TelemetryRuntime::from_env().map_err(|error| {
        ClassifiedError::validation(format!("Invalid observability configuration: {error}"))
    })?;

    telemetry.with_default_subscriber(|| {
        logger.info(
            "sce.app.start",
            "Starting command dispatch",
            &[("component", services::observability::NAME)],
        );

        let command = match parse_command(args) {
            Ok(command) => command,
            Err(error) => {
                logger.error(
                    "sce.command.parse_failed",
                    "Command parse failed",
                    &[("failure_class", error.class.as_str())],
                );
                return Err(error);
            }
        };

        logger.info(
            "sce.command.parsed",
            "Command parsed",
            &[("command", command.name())],
        );

        match dispatch(&command) {
            Ok(()) => {
                logger.info(
                    "sce.command.completed",
                    "Command completed",
                    &[("command", command.name())],
                );
                Ok(())
            }
            Err(error) => {
                logger.error(
                    "sce.command.failed",
                    "Command failed",
                    &[
                        ("command", command.name()),
                        ("failure_class", error.class.as_str()),
                    ],
                );
                Err(error)
            }
        }
    })
}

fn parse_command<I>(args: I) -> Result<Command, ClassifiedError>
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
    match parser.next().map_err(|error| {
        ClassifiedError::parse(format!(
            "Failed to parse arguments: {error}. Run 'sce --help' to see valid usage."
        ))
    })? {
        Some(lexopt::Arg::Long("help")) => {
            if tail_args.len() == 1 {
                Ok(Command::Help)
            } else {
                Err(ClassifiedError::parse(unknown_option_message("--help")))
            }
        }
        Some(lexopt::Arg::Short('h')) => {
            if tail_args.len() == 1 {
                Ok(Command::Help)
            } else {
                Err(ClassifiedError::parse(unknown_option_message("-h")))
            }
        }
        Some(lexopt::Arg::Long(option)) => Err(ClassifiedError::parse(unknown_option_message(
            &format!("--{option}"),
        ))),
        Some(lexopt::Arg::Short(option)) => Err(ClassifiedError::parse(unknown_option_message(
            &format!("-{option}"),
        ))),
        Some(lexopt::Arg::Value(value)) => {
            let subcommand = value.string().map_err(|error| {
                ClassifiedError::parse(format!(
                    "Failed to parse command token: {error}. Run 'sce --help' to see valid usage."
                ))
            })?;
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

fn parse_subcommand(value: String, tail_args: Vec<String>) -> Result<Command, ClassifiedError> {
    match value.as_str() {
        "help" => Ok(Command::Help),
        "config" => parse_config_subcommand(tail_args),
        "setup" => parse_setup_subcommand(tail_args),
        "doctor" => parse_non_setup_subcommand(Command::Doctor, tail_args),
        "mcp" => parse_non_setup_subcommand(Command::Mcp, tail_args),
        "hooks" => parse_hooks_subcommand(tail_args),
        "sync" => parse_non_setup_subcommand(Command::Sync, tail_args),
        _ => {
            if command_surface::is_known_command(&value) {
                return Err(ClassifiedError::parse(format!(
                    "Command '{}' is currently unavailable in this build.",
                    value,
                )));
            }

            Err(ClassifiedError::parse(format!(
                "Unknown command '{}'. Run 'sce --help' to see the current command surface.",
                value,
            )))
        }
    }
}

fn parse_config_subcommand(args: Vec<String>) -> Result<Command, ClassifiedError> {
    let subcommand = services::config::parse_config_subcommand(args)
        .map_err(|error| ClassifiedError::validation(error.to_string()))?;
    Ok(Command::Config(subcommand))
}

fn parse_setup_subcommand(args: Vec<String>) -> Result<Command, ClassifiedError> {
    let options = services::setup::parse_setup_cli_options(args)
        .map_err(|error| ClassifiedError::validation(error.to_string()))?;

    if options.help {
        return Ok(Command::SetupHelp);
    }

    if options.hooks {
        let repo_path = services::setup::resolve_setup_hooks_repository(&options)
            .map_err(|error| ClassifiedError::validation(error.to_string()))?;
        return Ok(Command::SetupHooks(repo_path));
    }

    services::setup::resolve_setup_hooks_repository(&options)
        .map_err(|error| ClassifiedError::validation(error.to_string()))?;

    let mode = services::setup::resolve_setup_mode(options)
        .map_err(|error| ClassifiedError::validation(error.to_string()))?;
    Ok(Command::Setup(mode))
}

fn parse_non_setup_subcommand(
    command: Command,
    tail_args: Vec<String>,
) -> Result<Command, ClassifiedError> {
    if tail_args.is_empty() {
        return Ok(command);
    }

    Err(ClassifiedError::validation(format!(
        "Unexpected extra argument '{}'. Run 'sce --help' to see valid usage.",
        tail_args[0]
    )))
}

fn parse_hooks_subcommand(args: Vec<String>) -> Result<Command, ClassifiedError> {
    let subcommand = services::hooks::parse_hooks_subcommand(args)
        .map_err(|error| ClassifiedError::validation(error.to_string()))?;
    Ok(Command::Hooks(subcommand))
}

fn dispatch(command: &Command) -> Result<(), ClassifiedError> {
    match command {
        Command::Help => println!("{}", command_surface::help_text()),
        Command::Config(subcommand) => {
            println!(
                "{}",
                services::config::run_config_subcommand(subcommand.clone())
                    .map_err(|error| ClassifiedError::runtime(error.to_string()))?
            );
        }
        Command::Setup(mode) => {
            let dispatch = services::setup::resolve_setup_dispatch(
                *mode,
                &services::setup::InquireSetupTargetPrompter,
            )
            .map_err(|error| ClassifiedError::runtime(error.to_string()))?;

            match dispatch {
                services::setup::SetupDispatch::Proceed(mode) => {
                    let repository_root = std::env::current_dir()
                        .context("Failed to determine current directory")
                        .map_err(|error| ClassifiedError::runtime(error.to_string()))?;
                    println!(
                        "{}",
                        services::setup::run_setup_for_mode(&repository_root, mode)
                            .map_err(|error| ClassifiedError::runtime(error.to_string()))?
                    );
                }
                services::setup::SetupDispatch::Cancelled => {
                    println!("{}", services::setup::setup_cancelled_text());
                }
            }
        }
        Command::SetupHooks(repo_path) => {
            let current_dir = std::env::current_dir()
                .context("Failed to determine current directory")
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?;
            let repository_root = repo_path.as_deref().unwrap_or(current_dir.as_path());
            println!(
                "{}",
                services::setup::run_setup_hooks(repository_root)
                    .map_err(|error| ClassifiedError::runtime(error.to_string()))?
            );
        }
        Command::SetupHelp => println!("{}", services::setup::setup_usage_text()),
        Command::Doctor => println!(
            "{}",
            services::doctor::run_doctor()
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?
        ),
        Command::Mcp => println!(
            "{}",
            services::mcp::run_placeholder_mcp()
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?
        ),
        Command::Hooks(subcommand) => {
            println!(
                "{}",
                services::hooks::run_hooks_subcommand(subcommand.clone())
                    .map_err(|error| ClassifiedError::runtime(error.to_string()))?
            )
        }
        Command::Sync => println!(
            "{}",
            services::sync::run_placeholder_sync()
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?
        ),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::ExitCode;

    use crate::services::setup::{SetupMode, SetupTarget};

    use super::{
        parse_command, run, run_with_dependency_check, Command, EXIT_CODE_DEPENDENCY_FAILURE,
        EXIT_CODE_PARSE_FAILURE, EXIT_CODE_RUNTIME_FAILURE, EXIT_CODE_VALIDATION_FAILURE,
    };

    #[test]
    fn help_path_exits_success() {
        let code = run(vec!["sce".to_string(), "--help".to_string()]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn hooks_command_without_subcommand_exits_non_zero() {
        let code = run(vec!["sce".to_string(), "hooks".to_string()]);
        assert_eq!(code, ExitCode::from(EXIT_CODE_VALIDATION_FAILURE));
    }

    #[test]
    fn doctor_command_exits_success() {
        let code = run(vec!["sce".to_string(), "doctor".to_string()]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn config_show_command_exits_success() {
        let code = run(vec![
            "sce".to_string(),
            "config".to_string(),
            "show".to_string(),
        ]);
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
        assert_eq!(code, ExitCode::from(EXIT_CODE_PARSE_FAILURE));
    }

    #[test]
    fn setup_validation_failure_uses_validation_exit_code() {
        let code = run(vec![
            "sce".to_string(),
            "setup".to_string(),
            "--repo".to_string(),
            "../demo-repo".to_string(),
        ]);
        assert_eq!(code, ExitCode::from(EXIT_CODE_VALIDATION_FAILURE));
    }

    #[test]
    fn runtime_failure_uses_runtime_exit_code() {
        let code = run(vec![
            "sce".to_string(),
            "hooks".to_string(),
            "commit-msg".to_string(),
            "/definitely/missing/COMMIT_EDITMSG".to_string(),
        ]);
        assert_eq!(code, ExitCode::from(EXIT_CODE_RUNTIME_FAILURE));
    }

    #[test]
    fn dependency_failure_uses_dependency_exit_code() {
        let code = run_with_dependency_check(vec!["sce".to_string(), "--help".to_string()], || {
            anyhow::bail!("simulated dependency check failure")
        });
        assert_eq!(code, ExitCode::from(EXIT_CODE_DEPENDENCY_FAILURE));
    }

    #[test]
    fn parser_defaults_to_help_without_command() {
        let command = parse_command(vec!["sce".to_string()]).expect("command should parse");
        assert_eq!(command, Command::Help);
    }

    #[test]
    fn parser_routes_hooks_pre_commit_subcommand() {
        let command = parse_command(vec![
            "sce".to_string(),
            "hooks".to_string(),
            "pre-commit".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(
            command,
            Command::Hooks(crate::services::hooks::HookSubcommand::PreCommit)
        );
    }

    #[test]
    fn parser_routes_hooks_commit_msg_subcommand_with_path() {
        let command = parse_command(vec![
            "sce".to_string(),
            "hooks".to_string(),
            "commit-msg".to_string(),
            ".git/COMMIT_EDITMSG".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(
            command,
            Command::Hooks(crate::services::hooks::HookSubcommand::CommitMsg {
                message_file: std::path::PathBuf::from(".git/COMMIT_EDITMSG"),
            })
        );
    }

    #[test]
    fn parser_rejects_hooks_unknown_subcommand() {
        let error = parse_command(vec![
            "sce".to_string(),
            "hooks".to_string(),
            "unknown".to_string(),
        ])
        .expect_err("unknown hook subcommand should fail");
        assert_eq!(
            error.to_string(),
            "Unknown hook subcommand 'unknown'. Run 'sce hooks --help' to see valid usage."
        );
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

    #[test]
    fn parser_routes_config_show_subcommand() {
        let command = parse_command(vec![
            "sce".to_string(),
            "config".to_string(),
            "show".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(
            command,
            Command::Config(crate::services::config::ConfigSubcommand::Show(
                crate::services::config::ConfigRequest {
                    report_format: crate::services::config::ReportFormat::Text,
                    config_path: None,
                    log_level: None,
                    timeout_ms: None,
                }
            ))
        );
    }
}
