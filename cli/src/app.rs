use std::io::{self, Write};
use std::process::ExitCode;

use crate::{command_surface, services};
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
    code: &'static str,
    message: String,
}

impl ClassifiedError {
    fn parse(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Parse,
            code: "SCE-ERR-PARSE",
            message: message.into(),
        }
    }

    fn validation(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Validation,
            code: "SCE-ERR-VALIDATION",
            message: message.into(),
        }
    }

    fn runtime(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Runtime,
            code: "SCE-ERR-RUNTIME",
            message: message.into(),
        }
    }

    fn dependency(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Dependency,
            code: "SCE-ERR-DEPENDENCY",
            message: message.into(),
        }
    }
}

impl FailureClass {
    fn default_try_guidance(self) -> &'static str {
        match self {
            Self::Parse => "run 'sce --help' to see valid usage.",
            Self::Validation => {
                "run the command-specific '--help' usage shown in the error and retry."
            }
            Self::Runtime => "inspect the runtime diagnostic details, then retry.",
            Self::Dependency => {
                "verify required runtime dependencies and environment setup, then retry."
            }
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
    Completion(services::completion::CompletionRequest),
    CompletionHelp,
    Config(services::config::ConfigSubcommand),
    Setup(services::setup::SetupRequest),
    SetupHelp,
    Doctor(services::doctor::DoctorRequest),
    DoctorHelp,
    Mcp(services::mcp::McpRequest),
    McpHelp,
    Hooks(services::hooks::HookSubcommand),
    HooksHelp,
    Sync(services::sync::SyncRequest),
    SyncHelp,
    Version(services::version::VersionRequest),
    VersionHelp,
}

impl Command {
    fn name(&self) -> &'static str {
        match self {
            Self::Help => "help",
            Self::Completion(_) | Self::CompletionHelp => services::completion::NAME,
            Self::Config(_) => services::config::NAME,
            Self::Setup(_) | Self::SetupHelp => services::setup::NAME,
            Self::Doctor(_) | Self::DoctorHelp => services::doctor::NAME,
            Self::Mcp(_) | Self::McpHelp => services::mcp::NAME,
            Self::Hooks(_) | Self::HooksHelp => services::hooks::NAME,
            Self::Sync(_) | Self::SyncHelp => services::sync::NAME,
            Self::Version(_) | Self::VersionHelp => services::version::NAME,
        }
    }
}

pub fn run<I>(args: I) -> ExitCode
where
    I: IntoIterator<Item = String>,
{
    run_with_dependency_check(args, || Ok(()))
}

fn run_with_dependency_check<I, F>(args: I, dependency_check: F) -> ExitCode
where
    I: IntoIterator<Item = String>,
    F: FnOnce() -> anyhow::Result<()>,
{
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    run_with_dependency_check_and_streams(args, dependency_check, &mut stdout, &mut stderr)
}

fn run_with_dependency_check_and_streams<I, F, StdoutW, StderrW>(
    args: I,
    dependency_check: F,
    stdout: &mut StdoutW,
    stderr: &mut StderrW,
) -> ExitCode
where
    I: IntoIterator<Item = String>,
    F: FnOnce() -> anyhow::Result<()>,
    StdoutW: Write,
    StderrW: Write,
{
    match try_run_with_dependency_check(args, dependency_check) {
        Ok(payload) => {
            if let Err(error) = write_stdout_payload(stdout, &payload) {
                write_error_diagnostic(stderr, &error);
                ExitCode::from(error.class.exit_code())
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            write_error_diagnostic(stderr, &error);
            ExitCode::from(error.class.exit_code())
        }
    }
}

fn write_stdout_payload<W>(writer: &mut W, payload: &str) -> Result<(), ClassifiedError>
where
    W: Write,
{
    writeln!(writer, "{payload}").map_err(|error| {
        ClassifiedError::runtime(format!("Failed to write command output to stdout: {error}"))
    })
}

fn write_error_diagnostic<W>(writer: &mut W, error: &ClassifiedError)
where
    W: Write,
{
    let rendered = if error.message.contains("Try:") {
        error.message.clone()
    } else {
        format!(
            "{} Try: {}",
            error.message,
            error.class.default_try_guidance()
        )
    };

    let _ = writeln!(
        writer,
        "Error [{}]: {}",
        error.code,
        services::security::redact_sensitive_text(&rendered)
    );
}

fn try_run_with_dependency_check<I, F>(
    args: I,
    dependency_check: F,
) -> Result<String, ClassifiedError>
where
    I: IntoIterator<Item = String>,
    F: FnOnce() -> anyhow::Result<()>,
{
    dependency_check().map_err(|error| {
        ClassifiedError::dependency(format!("Failed to initialize dependency checks: {error}"))
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
            Ok(payload) => {
                logger.info(
                    "sce.command.completed",
                    "Command completed",
                    &[("command", command.name())],
                );
                Ok(payload)
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
            "Failed to parse arguments: {error}. Try: run 'sce --help' to list valid commands, then retry with a supported form such as 'sce version' or 'sce setup --help'."
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
                    "Failed to parse command token: {error}. Try: run 'sce --help' to list valid commands, then rerun with one of them."
                ))
            })?;
            parse_subcommand(subcommand, tail_args.into_iter().skip(1).collect())
        }
        None => Ok(Command::Help),
    }
}

fn unknown_option_message(option: &str) -> String {
    format!(
        "Unknown option '{}'. Try: run 'sce --help' to see top-level usage, or use 'sce <command> --help' for command-specific options.",
        option
    )
}

fn parse_subcommand(value: String, tail_args: Vec<String>) -> Result<Command, ClassifiedError> {
    match value.as_str() {
        "help" => Ok(Command::Help),
        "completion" => parse_completion_subcommand(tail_args),
        "config" => parse_config_subcommand(tail_args),
        "setup" => parse_setup_subcommand(tail_args),
        "doctor" => parse_doctor_subcommand(tail_args),
        "mcp" => parse_mcp_subcommand(tail_args),
        "hooks" => parse_hooks_subcommand(tail_args),
        "sync" => parse_sync_subcommand(tail_args),
        "version" => parse_version_subcommand(tail_args),
        _ => {
            if command_surface::is_known_command(&value) {
                return Err(ClassifiedError::parse(format!(
                    "Command '{}' is currently unavailable in this build. Try: run 'sce --help' to see available commands in this build.",
                    value,
                )));
            }

            Err(ClassifiedError::parse(format!(
                "Unknown command '{}'. Try: run 'sce --help' to list valid commands, then rerun with a valid command such as 'sce version' or 'sce setup --help'.",
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

fn parse_completion_subcommand(args: Vec<String>) -> Result<Command, ClassifiedError> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return Ok(Command::CompletionHelp);
    }

    let request = services::completion::parse_completion_request(args)
        .map_err(|error| ClassifiedError::validation(error.to_string()))?;
    Ok(Command::Completion(request))
}

fn parse_setup_subcommand(args: Vec<String>) -> Result<Command, ClassifiedError> {
    let options = services::setup::parse_setup_cli_options(args)
        .map_err(|error| ClassifiedError::validation(error.to_string()))?;

    if options.help {
        return Ok(Command::SetupHelp);
    }

    let request = services::setup::resolve_setup_request(options)
        .map_err(|error| ClassifiedError::validation(error.to_string()))?;
    Ok(Command::Setup(request))
}

fn parse_doctor_subcommand(args: Vec<String>) -> Result<Command, ClassifiedError> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return Ok(Command::DoctorHelp);
    }

    let request = services::doctor::parse_doctor_request(args)
        .map_err(|error| ClassifiedError::validation(error.to_string()))?;
    Ok(Command::Doctor(request))
}

fn parse_mcp_subcommand(args: Vec<String>) -> Result<Command, ClassifiedError> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return Ok(Command::McpHelp);
    }

    let request = services::mcp::parse_mcp_request(args)
        .map_err(|error| ClassifiedError::validation(error.to_string()))?;
    Ok(Command::Mcp(request))
}

fn parse_sync_subcommand(args: Vec<String>) -> Result<Command, ClassifiedError> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return Ok(Command::SyncHelp);
    }

    let request = services::sync::parse_sync_request(args)
        .map_err(|error| ClassifiedError::validation(error.to_string()))?;
    Ok(Command::Sync(request))
}

fn parse_hooks_subcommand(args: Vec<String>) -> Result<Command, ClassifiedError> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return Ok(Command::HooksHelp);
    }

    let subcommand = services::hooks::parse_hooks_subcommand(args)
        .map_err(|error| ClassifiedError::validation(error.to_string()))?;
    Ok(Command::Hooks(subcommand))
}

fn parse_version_subcommand(args: Vec<String>) -> Result<Command, ClassifiedError> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return Ok(Command::VersionHelp);
    }

    let request = services::version::parse_version_request(args)
        .map_err(|error| ClassifiedError::validation(error.to_string()))?;
    Ok(Command::Version(request))
}

fn dispatch(command: &Command) -> Result<String, ClassifiedError> {
    match command {
        Command::Help => Ok(command_surface::help_text()),
        Command::CompletionHelp => Ok(services::completion::completion_usage_text().to_string()),
        Command::Completion(request) => Ok(services::completion::render_completion(*request)),
        Command::Config(subcommand) => services::config::run_config_subcommand(subcommand.clone())
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
        Command::Setup(request) => {
            let current_dir = std::env::current_dir()
                .context("Failed to determine current directory")
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?;

            let mut sections = Vec::new();

            if let Some(mode) = request.config_mode {
                let dispatch = services::setup::resolve_setup_dispatch(
                    mode,
                    &services::setup::InquireSetupTargetPrompter,
                )
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?;

                match dispatch {
                    services::setup::SetupDispatch::Proceed(resolved_mode) => {
                        let setup_message =
                            services::setup::run_setup_for_mode(&current_dir, resolved_mode)
                                .map_err(|error| ClassifiedError::runtime(error.to_string()))?;
                        sections.push(setup_message);
                    }
                    services::setup::SetupDispatch::Cancelled => {
                        return Ok(services::setup::setup_cancelled_text().to_string());
                    }
                }
            }

            if request.install_hooks {
                let repository_root = request
                    .hooks_repo_path
                    .as_deref()
                    .unwrap_or(current_dir.as_path());
                let hooks_message = services::setup::run_setup_hooks(repository_root)
                    .map_err(|error| ClassifiedError::runtime(error.to_string()))?;
                sections.push(hooks_message);
            }

            Ok(sections.join("\n\n"))
        }
        Command::SetupHelp => Ok(services::setup::setup_usage_text().to_string()),
        Command::DoctorHelp => Ok(services::doctor::doctor_usage_text().to_string()),
        Command::Doctor(request) => services::doctor::run_doctor(*request)
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
        Command::McpHelp => Ok(services::mcp::mcp_usage_text().to_string()),
        Command::Mcp(request) => services::mcp::run_placeholder_mcp(*request)
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
        Command::HooksHelp => Ok(services::hooks::hooks_usage_text().to_string()),
        Command::Hooks(subcommand) => services::hooks::run_hooks_subcommand(subcommand.clone())
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
        Command::SyncHelp => Ok(services::sync::sync_usage_text().to_string()),
        Command::Sync(request) => services::sync::run_placeholder_sync(*request)
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
        Command::VersionHelp => Ok(services::version::version_usage_text().to_string()),
        Command::Version(request) => services::version::render_version(*request)
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use std::process::ExitCode;

    use crate::services::setup::{SetupMode, SetupRequest, SetupTarget};

    use super::{
        parse_command, run, run_with_dependency_check, run_with_dependency_check_and_streams,
        Command, EXIT_CODE_DEPENDENCY_FAILURE, EXIT_CODE_PARSE_FAILURE, EXIT_CODE_RUNTIME_FAILURE,
        EXIT_CODE_VALIDATION_FAILURE,
    };

    #[test]
    fn successful_output_is_written_to_stdout() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_with_dependency_check_and_streams(
            vec!["sce".to_string(), "--help".to_string()],
            || Ok(()),
            &mut stdout,
            &mut stderr,
        );
        assert_eq!(code, ExitCode::SUCCESS);

        let stdout = String::from_utf8(stdout).expect("stdout should be utf-8");
        assert!(stdout.contains("Usage:"));
    }

    #[test]
    fn parse_failure_keeps_stdout_empty_and_reports_stderr() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_with_dependency_check_and_streams(
            vec!["sce".to_string(), "does-not-exist".to_string()],
            || Ok(()),
            &mut stdout,
            &mut stderr,
        );
        assert_eq!(code, ExitCode::from(EXIT_CODE_PARSE_FAILURE));
        assert!(stdout.is_empty());

        let stderr = String::from_utf8(stderr).expect("stderr should be utf-8");
        assert!(stderr.contains("Error [SCE-ERR-PARSE]: Unknown command 'does-not-exist'."));
        assert!(stderr.contains("Try:"));
    }

    #[test]
    fn parse_failure_stderr_contract_is_exact_and_deterministic() {
        let mut first_stdout = Vec::new();
        let mut first_stderr = Vec::new();
        let first_code = run_with_dependency_check_and_streams(
            vec!["sce".to_string(), "does-not-exist".to_string()],
            || Ok(()),
            &mut first_stdout,
            &mut first_stderr,
        );
        assert_eq!(first_code, ExitCode::from(EXIT_CODE_PARSE_FAILURE));
        assert!(first_stdout.is_empty());

        let mut second_stdout = Vec::new();
        let mut second_stderr = Vec::new();
        let second_code = run_with_dependency_check_and_streams(
            vec!["sce".to_string(), "does-not-exist".to_string()],
            || Ok(()),
            &mut second_stdout,
            &mut second_stderr,
        );
        assert_eq!(second_code, ExitCode::from(EXIT_CODE_PARSE_FAILURE));
        assert!(second_stdout.is_empty());

        let expected = "Error [SCE-ERR-PARSE]: Unknown command 'does-not-exist'. Try: run 'sce --help' to list valid commands, then rerun with a valid command such as 'sce version' or 'sce setup --help'.\n";
        let first_stderr = String::from_utf8(first_stderr).expect("stderr should be utf-8");
        let second_stderr = String::from_utf8(second_stderr).expect("stderr should be utf-8");
        assert_eq!(first_stderr, expected);
        assert_eq!(second_stderr, expected);
    }

    #[test]
    fn dependency_failure_reports_stable_error_code_and_try_guidance() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_with_dependency_check_and_streams(
            vec!["sce".to_string(), "--help".to_string()],
            || anyhow::bail!("simulated dependency check failure"),
            &mut stdout,
            &mut stderr,
        );
        assert_eq!(code, ExitCode::from(EXIT_CODE_DEPENDENCY_FAILURE));
        assert!(stdout.is_empty());

        let stderr = String::from_utf8(stderr).expect("stderr should be utf-8");
        assert!(stderr.contains("Error [SCE-ERR-DEPENDENCY]:"));
        assert!(stderr.contains("Try:"));
    }

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
    fn completion_command_exits_success() {
        let code = run(vec![
            "sce".to_string(),
            "completion".to_string(),
            "--shell".to_string(),
            "bash".to_string(),
        ]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn completion_help_exits_success() {
        let code = run(vec![
            "sce".to_string(),
            "completion".to_string(),
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
    fn doctor_help_exits_success() {
        let code = run(vec![
            "sce".to_string(),
            "doctor".to_string(),
            "--help".to_string(),
        ]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn mcp_help_exits_success() {
        let code = run(vec![
            "sce".to_string(),
            "mcp".to_string(),
            "--help".to_string(),
        ]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn hooks_help_exits_success() {
        let code = run(vec![
            "sce".to_string(),
            "hooks".to_string(),
            "--help".to_string(),
        ]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn sync_help_exits_success() {
        let code = run(vec![
            "sce".to_string(),
            "sync".to_string(),
            "--help".to_string(),
        ]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn version_command_exits_success() {
        let code = run(vec!["sce".to_string(), "version".to_string()]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn version_help_exits_success() {
        let code = run(vec![
            "sce".to_string(),
            "version".to_string(),
            "--help".to_string(),
        ]);
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
            "Unknown hook subcommand 'unknown'. Try: run 'sce hooks --help' and use one of 'pre-commit', 'commit-msg', 'post-commit', or 'post-rewrite'."
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
            Command::Setup(SetupRequest {
                config_mode: Some(SetupMode::NonInteractive(SetupTarget::OpenCode,)),
                install_hooks: false,
                hooks_repo_path: None,
            })
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
            Command::Setup(SetupRequest {
                config_mode: Some(SetupMode::NonInteractive(SetupTarget::Claude,)),
                install_hooks: false,
                hooks_repo_path: None,
            })
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
            Command::Setup(SetupRequest {
                config_mode: Some(SetupMode::NonInteractive(SetupTarget::Both,)),
                install_hooks: false,
                hooks_repo_path: None,
            })
        );
    }

    #[test]
    fn parser_routes_setup_without_flags_to_interactive_mode() {
        let command = parse_command(vec!["sce".to_string(), "setup".to_string()])
            .expect("command should parse");
        assert_eq!(
            command,
            Command::Setup(SetupRequest {
                config_mode: Some(SetupMode::Interactive),
                install_hooks: true,
                hooks_repo_path: None,
            })
        );
    }

    #[test]
    fn parser_routes_setup_target_with_non_interactive_flag() {
        let command = parse_command(vec![
            "sce".to_string(),
            "setup".to_string(),
            "--opencode".to_string(),
            "--non-interactive".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(
            command,
            Command::Setup(SetupRequest {
                config_mode: Some(SetupMode::NonInteractive(SetupTarget::OpenCode,)),
                install_hooks: false,
                hooks_repo_path: None,
            })
        );
    }

    #[test]
    fn parser_routes_setup_hooks_without_repo() {
        let command = parse_command(vec![
            "sce".to_string(),
            "setup".to_string(),
            "--hooks".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(
            command,
            Command::Setup(SetupRequest {
                config_mode: None,
                install_hooks: true,
                hooks_repo_path: None,
            })
        );
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
            Command::Setup(SetupRequest {
                config_mode: None,
                install_hooks: true,
                hooks_repo_path: Some(std::path::PathBuf::from("../demo-repo")),
            })
        );
    }

    #[test]
    fn parser_routes_setup_target_plus_hooks_in_single_request() {
        let command = parse_command(vec![
            "sce".to_string(),
            "setup".to_string(),
            "--opencode".to_string(),
            "--hooks".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(
            command,
            Command::Setup(SetupRequest {
                config_mode: Some(SetupMode::NonInteractive(SetupTarget::OpenCode,)),
                install_hooks: true,
                hooks_repo_path: None,
            })
        );
    }

    #[test]
    fn parser_routes_doctor_help() {
        let command = parse_command(vec![
            "sce".to_string(),
            "doctor".to_string(),
            "--help".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(command, Command::DoctorHelp);
    }

    #[test]
    fn parser_routes_doctor_json_format() {
        let command = parse_command(vec![
            "sce".to_string(),
            "doctor".to_string(),
            "--format".to_string(),
            "json".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(
            command,
            Command::Doctor(crate::services::doctor::DoctorRequest {
                format: crate::services::doctor::DoctorFormat::Json,
            })
        );
    }

    #[test]
    fn parser_routes_mcp_help() {
        let command = parse_command(vec![
            "sce".to_string(),
            "mcp".to_string(),
            "--help".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(command, Command::McpHelp);
    }

    #[test]
    fn parser_routes_mcp_json_format() {
        let command = parse_command(vec![
            "sce".to_string(),
            "mcp".to_string(),
            "--format".to_string(),
            "json".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(
            command,
            Command::Mcp(crate::services::mcp::McpRequest {
                format: crate::services::mcp::McpFormat::Json,
            })
        );
    }

    #[test]
    fn parser_routes_hooks_help() {
        let command = parse_command(vec![
            "sce".to_string(),
            "hooks".to_string(),
            "--help".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(command, Command::HooksHelp);
    }

    #[test]
    fn parser_routes_sync_help() {
        let command = parse_command(vec![
            "sce".to_string(),
            "sync".to_string(),
            "--help".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(command, Command::SyncHelp);
    }

    #[test]
    fn parser_routes_sync_json_format() {
        let command = parse_command(vec![
            "sce".to_string(),
            "sync".to_string(),
            "--format".to_string(),
            "json".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(
            command,
            Command::Sync(crate::services::sync::SyncRequest {
                format: crate::services::sync::SyncFormat::Json,
            })
        );
    }

    #[test]
    fn parser_routes_version_text_by_default() {
        let command = parse_command(vec!["sce".to_string(), "version".to_string()])
            .expect("command should parse");
        assert_eq!(
            command,
            Command::Version(crate::services::version::VersionRequest {
                format: crate::services::version::VersionFormat::Text,
            })
        );
    }

    #[test]
    fn parser_routes_version_json_format() {
        let command = parse_command(vec![
            "sce".to_string(),
            "version".to_string(),
            "--format".to_string(),
            "json".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(
            command,
            Command::Version(crate::services::version::VersionRequest {
                format: crate::services::version::VersionFormat::Json,
            })
        );
    }

    #[test]
    fn parser_routes_version_help() {
        let command = parse_command(vec![
            "sce".to_string(),
            "version".to_string(),
            "--help".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(command, Command::VersionHelp);
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
            "Options '--opencode', '--claude', and '--both' are mutually exclusive. Try: choose exactly one target flag (for example 'sce setup --opencode --non-interactive') or omit all target flags for interactive mode."
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
            "Option '--repo' requires '--hooks'. Try: run 'sce setup --hooks --repo <path>' or remove '--repo'."
        );
    }

    #[test]
    fn parser_rejects_setup_non_interactive_without_target() {
        let error = parse_command(vec![
            "sce".to_string(),
            "setup".to_string(),
            "--non-interactive".to_string(),
        ])
        .expect_err("--non-interactive without a target should fail");
        assert_eq!(
            error.to_string(),
            "Option '--non-interactive' requires a target flag. Try: 'sce setup --opencode --non-interactive', 'sce setup --claude --non-interactive', or 'sce setup --both --non-interactive'."
        );
    }

    #[test]
    fn parser_rejects_unknown_command() {
        let error = parse_command(vec!["sce".to_string(), "nope".to_string()])
            .expect_err("unknown command should fail");
        assert_eq!(
            error.to_string(),
            "Unknown command 'nope'. Try: run 'sce --help' to list valid commands, then rerun with a valid command such as 'sce version' or 'sce setup --help'."
        );
    }

    #[test]
    fn parser_rejects_unknown_option() {
        let error = parse_command(vec!["sce".to_string(), "--verbose".to_string()])
            .expect_err("unknown option should fail");
        assert_eq!(
            error.to_string(),
            "Unknown option '--verbose'. Try: run 'sce --help' to see top-level usage, or use 'sce <command> --help' for command-specific options."
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
            "Unexpected setup argument 'extra'. Try: remove the extra argument and use 'sce setup --help' for supported forms."
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

    #[test]
    fn parser_routes_completion_bash_shell() {
        let command = parse_command(vec![
            "sce".to_string(),
            "completion".to_string(),
            "--shell".to_string(),
            "bash".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(
            command,
            Command::Completion(crate::services::completion::CompletionRequest {
                shell: crate::services::completion::CompletionShell::Bash,
            })
        );
    }

    #[test]
    fn parser_routes_completion_help() {
        let command = parse_command(vec![
            "sce".to_string(),
            "completion".to_string(),
            "--help".to_string(),
        ])
        .expect("command should parse");
        assert_eq!(command, Command::CompletionHelp);
    }
}
