use std::io::{self, Write};
use std::process::ExitCode;

use crate::{cli_schema, command_surface, services};
use anyhow::Context;

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

/// Internal command representation after clap parsing and adapter conversion.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Command {
    Help,
    Auth(services::auth_command::AuthRequest),
    Completion(services::completion::CompletionRequest),
    Config(services::config::ConfigSubcommand),
    Setup(services::setup::SetupRequest),
    Doctor(services::doctor::DoctorRequest),
    Mcp(services::mcp::McpRequest),
    Hooks(services::hooks::HookSubcommand),
    Sync(services::sync::SyncRequest),
    Version(services::version::VersionRequest),
}

impl Command {
    fn name(&self) -> &'static str {
        match self {
            Self::Help => "help",
            Self::Auth(_) => services::auth_command::NAME,
            Self::Completion(_) => services::completion::NAME,
            Self::Config(_) => services::config::NAME,
            Self::Setup(_) => services::setup::NAME,
            Self::Doctor(_) => services::doctor::NAME,
            Self::Mcp(_) => services::mcp::NAME,
            Self::Hooks(_) => services::hooks::NAME,
            Self::Sync(_) => services::sync::NAME,
            Self::Version(_) => services::version::NAME,
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
    let args_vec: Vec<String> = args.into_iter().collect();

    // Handle empty args (just program name) -> Help
    if args_vec.len() <= 1 {
        return Ok(Command::Help);
    }

    // Use clap to parse
    let cli = match cli_schema::Cli::try_parse_from(&args_vec) {
        Ok(cli) => cli,
        Err(error) => {
            // Handle --help specially - user explicitly requested help
            if error.kind() == clap::error::ErrorKind::DisplayHelp {
                // Return Help command for successful output
                return Ok(Command::Help);
            }
            // Handle missing subcommand as validation error, not help display
            if error.kind() == clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand {
                // This means a required subcommand was not provided
                return Err(ClassifiedError::parse(
                    "Missing required subcommand. Try: run 'sce --help' to see valid commands.",
                ));
            }
            if error.kind() == clap::error::ErrorKind::DisplayVersion {
                // Return version command for --version
                return Ok(Command::Version(services::version::VersionRequest {
                    format: services::version::VersionFormat::Text,
                }));
            }
            return Err(classify_clap_error(&error));
        }
    };

    // No subcommand -> Help
    let Some(command) = cli.command else {
        return Ok(Command::Help);
    };

    // Convert clap command to internal command
    convert_clap_command(command)
}

/// Classify a clap error into our `ClassifiedError` taxonomy.
fn classify_clap_error(error: &clap::Error) -> ClassifiedError {
    use clap::error::ErrorKind;

    let message = error.to_string();

    // Determine error class based on clap error kind
    // Note: Many clap error kinds map to Parse failures
    let class = match error.kind() {
        // Validation errors: missing required arguments, argument conflicts
        ErrorKind::MissingRequiredArgument | ErrorKind::ArgumentConflict | ErrorKind::NoEquals => {
            FailureClass::Validation
        }

        // All other errors (parse errors, display errors, etc.) map to Parse
        _ => FailureClass::Parse,
    };

    // Clean up clap's error message to match our style
    let cleaned_message = clean_clap_error_message(&message, error.kind());

    match class {
        FailureClass::Validation => ClassifiedError::validation(cleaned_message),
        _ => ClassifiedError::parse(cleaned_message),
    }
}

/// Clean up clap error messages to match our error message style.
fn clean_clap_error_message(message: &str, kind: clap::error::ErrorKind) -> String {
    use clap::error::ErrorKind;

    // Remove the "error: " prefix that clap adds
    let message = message.strip_prefix("error: ").unwrap_or(message);

    match kind {
        ErrorKind::InvalidSubcommand => {
            // Extract the invalid subcommand name and provide helpful guidance
            if let Some(subcommand) = extract_quoted_value(message) {
                if command_surface::is_known_command(&subcommand) {
                    format!(
                        "Command '{subcommand}' is currently unavailable in this build. Try: run 'sce --help' to see available commands in this build."
                    )
                } else {
                    format!(
                        "Unknown command '{subcommand}'. Try: run 'sce --help' to list valid commands, then rerun with a valid command such as 'sce version' or 'sce setup --help'."
                    )
                }
            } else {
                format!("{message}. Try: run 'sce --help' to see valid usage.")
            }
        }
        ErrorKind::UnknownArgument => {
            // Extract the unknown argument and provide helpful guidance
            if let Some(arg) = extract_quoted_value(message) {
                format!(
                    "Unknown option '{arg}'. Try: run 'sce --help' to see top-level usage, or use 'sce <command> --help' for command-specific options."
                )
            } else {
                format!("{message}. Try: run 'sce --help' to see valid usage.")
            }
        }
        ErrorKind::MissingRequiredArgument => {
            // Clean up clap's message for missing required arguments
            if message.contains("required") {
                format!("{message}. Try: run 'sce --help' to see required arguments.")
            } else {
                format!("{message}. Try: run 'sce --help' to see valid usage.")
            }
        }
        ErrorKind::ArgumentConflict => {
            // Handle mutually exclusive arguments
            if message.contains("cannot be used with") || message.contains("conflicts with") {
                format!("{message}. Try: use only one of the conflicting options.")
            } else {
                format!("{message}. Try: run 'sce --help' to see valid usage.")
            }
        }
        _ => {
            // Default cleanup: ensure message ends with guidance
            if message.contains("Try:") {
                message.to_string()
            } else {
                format!("{message}. Try: run 'sce --help' to see valid usage.")
            }
        }
    }
}

/// Extract a single-quoted value from an error message.
fn extract_quoted_value(message: &str) -> Option<String> {
    // Clap uses single quotes for values in error messages
    let start = message.find('\'')?;
    let end = message[start + 1..].find('\'')?;
    Some(message[start + 1..start + 1 + end].to_string())
}

/// Convert a clap command to our internal command representation.
fn convert_clap_command(command: cli_schema::Commands) -> Result<Command, ClassifiedError> {
    match command {
        cli_schema::Commands::Config { subcommand } => convert_config_subcommand(subcommand),
        cli_schema::Commands::Auth { subcommand } => convert_auth_subcommand(subcommand),
        cli_schema::Commands::Setup {
            opencode,
            claude,
            both,
            non_interactive,
            hooks,
            repo,
        } => convert_setup_command(opencode, claude, both, non_interactive, hooks, repo),
        cli_schema::Commands::Doctor { format } => {
            Ok(Command::Doctor(services::doctor::DoctorRequest {
                format: convert_output_format(format),
            }))
        }
        cli_schema::Commands::Mcp { format } => Ok(Command::Mcp(services::mcp::McpRequest {
            format: convert_output_format(format),
        })),
        cli_schema::Commands::Hooks { subcommand } => convert_hooks_subcommand(subcommand),
        cli_schema::Commands::Sync { format } => Ok(Command::Sync(services::sync::SyncRequest {
            format: convert_output_format(format),
        })),
        cli_schema::Commands::Version { format } => {
            Ok(Command::Version(services::version::VersionRequest {
                format: convert_output_format(format),
            }))
        }
        cli_schema::Commands::Completion { shell } => Ok(Command::Completion(
            services::completion::CompletionRequest {
                shell: convert_completion_shell(shell),
            },
        )),
    }
}

#[allow(clippy::unnecessary_wraps, clippy::needless_pass_by_value)]
fn convert_auth_subcommand(
    subcommand: cli_schema::AuthSubcommand,
) -> Result<Command, ClassifiedError> {
    let subcommand = match subcommand {
        cli_schema::AuthSubcommand::Login { format } => {
            services::auth_command::AuthSubcommand::Login {
                format: convert_output_format(format),
            }
        }
        cli_schema::AuthSubcommand::Logout { format } => {
            services::auth_command::AuthSubcommand::Logout {
                format: convert_output_format(format),
            }
        }
        cli_schema::AuthSubcommand::Status { format } => {
            services::auth_command::AuthSubcommand::Status {
                format: convert_output_format(format),
            }
        }
    };

    Ok(Command::Auth(services::auth_command::AuthRequest {
        subcommand,
    }))
}

/// Convert clap output format to service output format.
fn convert_output_format(
    format: cli_schema::OutputFormat,
) -> services::output_format::OutputFormat {
    match format {
        cli_schema::OutputFormat::Text => services::output_format::OutputFormat::Text,
        cli_schema::OutputFormat::Json => services::output_format::OutputFormat::Json,
    }
}

/// Convert clap completion shell to service completion shell.
fn convert_completion_shell(
    shell: cli_schema::CompletionShell,
) -> services::completion::CompletionShell {
    match shell {
        cli_schema::CompletionShell::Bash => services::completion::CompletionShell::Bash,
        cli_schema::CompletionShell::Zsh => services::completion::CompletionShell::Zsh,
        cli_schema::CompletionShell::Fish => services::completion::CompletionShell::Fish,
    }
}

/// Convert clap config subcommand to service config subcommand.
#[allow(clippy::unnecessary_wraps)]
fn convert_config_subcommand(
    subcommand: cli_schema::ConfigSubcommand,
) -> Result<Command, ClassifiedError> {
    match subcommand {
        cli_schema::ConfigSubcommand::Show {
            format,
            config,
            log_level,
            timeout_ms,
        } => Ok(Command::Config(services::config::ConfigSubcommand::Show(
            services::config::ConfigRequest {
                report_format: convert_output_format(format),
                config_path: config,
                log_level: log_level.map(convert_log_level),
                timeout_ms,
            },
        ))),
        cli_schema::ConfigSubcommand::Validate {
            format,
            config,
            log_level,
            timeout_ms,
        } => Ok(Command::Config(
            services::config::ConfigSubcommand::Validate(services::config::ConfigRequest {
                report_format: convert_output_format(format),
                config_path: config,
                log_level: log_level.map(convert_log_level),
                timeout_ms,
            }),
        )),
    }
}

/// Convert clap log level to service log level.
fn convert_log_level(level: cli_schema::LogLevel) -> services::config::LogLevel {
    match level {
        cli_schema::LogLevel::Error => services::config::LogLevel::Error,
        cli_schema::LogLevel::Warn => services::config::LogLevel::Warn,
        cli_schema::LogLevel::Info => services::config::LogLevel::Info,
        cli_schema::LogLevel::Debug => services::config::LogLevel::Debug,
    }
}

/// Convert setup command flags to `SetupRequest`.
#[allow(clippy::fn_params_excessive_bools)]
fn convert_setup_command(
    opencode: bool,
    claude: bool,
    both: bool,
    non_interactive: bool,
    hooks: bool,
    repo: Option<std::path::PathBuf>,
) -> Result<Command, ClassifiedError> {
    // Build SetupCliOptions and use the existing resolve_setup_request
    let options = services::setup::SetupCliOptions {
        help: false,
        non_interactive,
        opencode,
        claude,
        both,
        hooks,
        repo_path: repo,
    };

    let request = services::setup::resolve_setup_request(options)
        .map_err(|error| ClassifiedError::validation(error.to_string()))?;

    Ok(Command::Setup(request))
}

/// Convert clap hooks subcommand to service hooks subcommand.
#[allow(clippy::unnecessary_wraps)]
fn convert_hooks_subcommand(
    subcommand: cli_schema::HooksSubcommand,
) -> Result<Command, ClassifiedError> {
    match subcommand {
        cli_schema::HooksSubcommand::PreCommit => {
            Ok(Command::Hooks(services::hooks::HookSubcommand::PreCommit))
        }
        cli_schema::HooksSubcommand::CommitMsg { message_file } => {
            Ok(Command::Hooks(services::hooks::HookSubcommand::CommitMsg {
                message_file,
            }))
        }
        cli_schema::HooksSubcommand::PostCommit => {
            Ok(Command::Hooks(services::hooks::HookSubcommand::PostCommit))
        }
        cli_schema::HooksSubcommand::PostRewrite { rewrite_method } => Ok(Command::Hooks(
            services::hooks::HookSubcommand::PostRewrite { rewrite_method },
        )),
    }
}

fn dispatch(command: &Command) -> Result<String, ClassifiedError> {
    match command {
        Command::Help => Ok(command_surface::help_text()),
        Command::Auth(request) => services::auth_command::run_auth_subcommand(*request)
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
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
        Command::Doctor(request) => services::doctor::run_doctor(*request)
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
        Command::Mcp(request) => services::mcp::run_placeholder_mcp(*request)
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
        Command::Hooks(subcommand) => services::hooks::run_hooks_subcommand(subcommand.clone())
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
        Command::Sync(request) => services::sync::run_placeholder_sync(*request)
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
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
        assert!(stderr.contains("Error [SCE-ERR-PARSE]:"));
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

        let first_stderr = String::from_utf8(first_stderr).expect("stderr should be utf-8");
        let second_stderr = String::from_utf8(second_stderr).expect("stderr should be utf-8");
        assert_eq!(first_stderr, second_stderr);
        assert!(first_stderr.contains("Unknown command 'does-not-exist'"));
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
        assert_eq!(code, ExitCode::from(EXIT_CODE_PARSE_FAILURE));
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
        assert!(error.to_string().contains("unknown"));
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
    fn parser_routes_auth_login_subcommand() {
        let command = parse_command(vec![
            "sce".to_string(),
            "auth".to_string(),
            "login".to_string(),
        ])
        .expect("auth login should parse");
        assert_eq!(
            command,
            Command::Auth(crate::services::auth_command::AuthRequest {
                subcommand: crate::services::auth_command::AuthSubcommand::Login {
                    format: crate::services::auth_command::AuthFormat::Text,
                },
            })
        );
    }

    #[test]
    fn parser_routes_auth_status_json_subcommand() {
        let command = parse_command(vec![
            "sce".to_string(),
            "auth".to_string(),
            "status".to_string(),
            "--format".to_string(),
            "json".to_string(),
        ])
        .expect("auth status json should parse");
        assert_eq!(
            command,
            Command::Auth(crate::services::auth_command::AuthRequest {
                subcommand: crate::services::auth_command::AuthSubcommand::Status {
                    format: crate::services::auth_command::AuthFormat::Json,
                },
            })
        );
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
    fn parser_rejects_setup_mutually_exclusive_flags() {
        let error = parse_command(vec![
            "sce".to_string(),
            "setup".to_string(),
            "--opencode".to_string(),
            "--claude".to_string(),
        ])
        .expect_err("mutually exclusive flags should fail");
        assert!(
            error.to_string().contains("cannot be used with")
                || error.to_string().contains("conflicts")
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
        // clap enforces this via the requires attribute
        assert!(error.to_string().contains("--repo") || error.to_string().contains("--hooks"));
    }

    #[test]
    fn parser_rejects_setup_non_interactive_without_target() {
        let error = parse_command(vec![
            "sce".to_string(),
            "setup".to_string(),
            "--non-interactive".to_string(),
        ])
        .expect_err("--non-interactive without a target should fail");
        assert!(error.to_string().contains("--non-interactive"));
    }

    #[test]
    fn parser_rejects_unknown_command() {
        let error = parse_command(vec!["sce".to_string(), "nope".to_string()])
            .expect_err("unknown command should fail");
        assert!(error.to_string().contains("Unknown command 'nope'"));
    }

    #[test]
    fn parser_rejects_unknown_option() {
        let error = parse_command(vec!["sce".to_string(), "--verbose".to_string()])
            .expect_err("unknown option should fail");
        assert!(error.to_string().contains("Unknown option"));
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
}
