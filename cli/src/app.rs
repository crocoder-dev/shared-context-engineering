use std::io::{self, Write};
use std::process::ExitCode;

use crate::{cli_schema, command_surface, services};
use anyhow::Context;
use services::error::{ClassifiedError, FailureClass};

#[derive(Clone, Debug, Eq, PartialEq)]
enum Command {
    Help,
    HelpText { name: String, text: String },
    Auth(services::auth_command::AuthRequest),
    Completion(services::completion::CompletionRequest),
    Config(services::config::ConfigSubcommand),
    Setup(services::setup::SetupRequest),
    Doctor(services::doctor::DoctorRequest),
    Hooks(services::hooks::HookSubcommand),
    Trace(services::trace::TraceRequest),
    Sync(services::sync::SyncRequest),
    Version(services::version::VersionRequest),
}

impl Command {
    fn name(&self) -> &str {
        match self {
            Self::Help => "help",
            Self::HelpText { name, .. } => name.as_str(),
            Self::Auth(_) => services::auth_command::NAME,
            Self::Completion(_) => services::completion::NAME,
            Self::Config(_) => services::config::NAME,
            Self::Setup(_) => services::setup::NAME,
            Self::Doctor(_) => services::doctor::NAME,
            Self::Hooks(_) => services::hooks::NAME,
            Self::Trace(_) => services::trace::NAME,
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
    let (result, logger) = try_run_with_dependency_check(args, dependency_check);
    match result {
        Ok(payload) => {
            if let Err(error) = write_stdout_payload(stdout, &payload) {
                if let Some(ref log) = logger {
                    log.log_classified_error(&error);
                }
                write_error_diagnostic(stderr, &error);
                ExitCode::from(error.class().exit_code())
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            if let Some(ref log) = logger {
                log.log_classified_error(&error);
            }
            write_error_diagnostic(stderr, &error);
            ExitCode::from(error.class().exit_code())
        }
    }
}

fn write_stdout_payload<W>(writer: &mut W, payload: &str) -> Result<(), ClassifiedError>
where
    W: Write,
{
    if payload.is_empty() {
        return Ok(());
    }

    writeln!(writer, "{payload}").map_err(|error| {
        ClassifiedError::runtime(format!("Failed to write command output to stdout: {error}"))
    })
}

fn write_error_diagnostic<W>(writer: &mut W, error: &ClassifiedError)
where
    W: Write,
{
    let rendered = if error.message().contains("Try:") {
        error.message().to_string()
    } else {
        format!(
            "{} Try: {}",
            error.message(),
            error.class().default_try_guidance()
        )
    };

    let styled_code = services::style::error_code(error.code());
    let styled_heading = services::style::heading("Error");
    let styled_message =
        services::style::error_text(&services::security::redact_sensitive_text(&rendered));

    writeln!(writer, "{styled_heading} [{styled_code}]: {styled_message}")
        .expect("writing error diagnostic to writer should not fail");
}

#[allow(clippy::too_many_lines)]
fn try_run_with_dependency_check<I, F>(
    args: I,
    dependency_check: F,
) -> (
    Result<String, ClassifiedError>,
    Option<services::observability::Logger>,
)
where
    I: IntoIterator<Item = String>,
    F: FnOnce() -> anyhow::Result<()>,
{
    if let Err(error) = dependency_check() {
        return (
            Err(ClassifiedError::dependency(format!(
                "Failed to initialize dependency checks: {error}"
            ))),
            None,
        );
    }

    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(error) => {
            return (
                Err(ClassifiedError::runtime(format!(
                    "Failed to determine current directory for observability config resolution: {error}"
                ))),
                None,
            );
        }
    };

    let observability_config = match services::config::resolve_observability_runtime_config(&cwd) {
        Ok(config) => config,
        Err(error) => {
            return (
                Err(ClassifiedError::validation(format!(
                    "Invalid observability configuration: {error}"
                ))),
                None,
            );
        }
    };

    let logger = match services::observability::Logger::from_resolved_config(&observability_config)
    {
        Ok(log) => log,
        Err(error) => {
            return (
                Err(ClassifiedError::validation(format!(
                    "Invalid observability configuration: {error}"
                ))),
                None,
            );
        }
    };

    // Log discovered config files at debug level
    for loaded_path in &observability_config.loaded_config_paths {
        logger.debug(
            "sce.config.file_discovered",
            "Config file discovered",
            &[
                ("path", loaded_path.path.to_string_lossy().as_ref()),
                ("source", loaded_path.source.as_str()),
            ],
        );
    }

    let telemetry = match services::observability::TelemetryRuntime::from_resolved_config(
        &observability_config,
    ) {
        Ok(tel) => tel,
        Err(error) => {
            return (
                Err(ClassifiedError::validation(format!(
                    "Invalid observability configuration: {error}"
                ))),
                None,
            );
        }
    };

    let result = telemetry.with_default_subscriber(|| {
        logger.info(
            "sce.app.start",
            "Starting command dispatch",
            &[("component", services::observability::NAME)],
        );

        let command = match parse_command(args, Some(&logger)) {
            Ok(command) => command,
            Err(error) => {
                return Err(error);
            }
        };

        logger.info(
            "sce.command.parsed",
            "Command parsed",
            &[("command", command.name())],
        );

        logger.debug(
            "sce.command.dispatch_start",
            "Dispatching command",
            &[("command", command.name())],
        );

        let dispatch_result = dispatch(&command, &logger);

        if dispatch_result.is_ok() {
            logger.debug(
                "sce.command.dispatch_end",
                "Command dispatch completed",
                &[("command", command.name())],
            );
        }

        match dispatch_result {
            Ok(payload) => {
                logger.info(
                    "sce.command.completed",
                    "Command completed",
                    &[("command", command.name())],
                );
                Ok(payload)
            }
            Err(error) => Err(error),
        }
    });

    (result, Some(logger))
}

fn parse_command<I>(
    args: I,
    logger: Option<&services::observability::Logger>,
) -> Result<Command, ClassifiedError>
where
    I: IntoIterator<Item = String>,
{
    let args_vec: Vec<String> = args.into_iter().collect();

    // Log raw args at debug level (redacted for security)
    if let Some(log) = logger {
        let args_summary = if args_vec.len() <= 1 {
            args_vec.join(" ")
        } else {
            format!("{} ...", args_vec[0])
        };
        log.debug(
            "sce.command.raw_args",
            "Parsing command arguments",
            &[("args_summary", &args_summary)],
        );
    }

    if args_vec.len() <= 1 {
        return Ok(Command::Help);
    }

    let cli = match cli_schema::Cli::try_parse_from(&args_vec) {
        Ok(cli) => cli,
        Err(error) => {
            if error.kind() == clap::error::ErrorKind::DisplayHelp {
                if let Some((name, text)) = render_subcommand_help_from_args(&args_vec) {
                    return Ok(Command::HelpText { name, text });
                }

                return Ok(Command::Help);
            }
            if error.kind() == clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand {
                if let Some(help_text) = render_missing_subcommand_help(&args_vec) {
                    return Ok(help_text);
                }

                return Err(ClassifiedError::parse(
                    "Missing required subcommand. Try: run 'sce --help' to see valid commands.",
                ));
            }
            if error.kind() == clap::error::ErrorKind::DisplayVersion {
                return Ok(Command::Version(services::version::VersionRequest {
                    format: services::version::VersionFormat::Text,
                }));
            }
            return Err(classify_clap_error(&error));
        }
    };

    let Some(command) = cli.command else {
        return Ok(Command::Help);
    };

    convert_clap_command(command)
}

fn classify_clap_error(error: &clap::Error) -> ClassifiedError {
    use clap::error::ErrorKind;

    let message = error.to_string();

    let class = match error.kind() {
        ErrorKind::MissingRequiredArgument | ErrorKind::ArgumentConflict | ErrorKind::NoEquals => {
            FailureClass::Validation
        }

        _ => FailureClass::Parse,
    };

    let cleaned_message = clean_clap_error_message(&message, error.kind());

    match class {
        FailureClass::Validation => ClassifiedError::validation(cleaned_message),
        _ => ClassifiedError::parse(cleaned_message),
    }
}

fn render_subcommand_help_from_args(args: &[String]) -> Option<(String, String)> {
    let command_name = args.get(1)?.to_owned();
    let command_path = args[1..]
        .iter()
        .take_while(|arg| !arg.starts_with('-'))
        .map(String::as_str)
        .collect::<Vec<_>>();

    if command_path.is_empty() {
        return None;
    }

    if command_path.as_slice() == [services::auth_command::NAME] {
        return Some((command_name, cli_schema::auth_help_text()));
    }

    cli_schema::render_help_for_path(&command_path).map(|text| (command_name, text))
}

fn render_missing_subcommand_help(args: &[String]) -> Option<Command> {
    let command_name = args.get(1)?.as_str();

    match command_name {
        services::auth_command::NAME => Some(Command::HelpText {
            name: services::auth_command::NAME.to_string(),
            text: cli_schema::auth_help_text(),
        }),
        services::config::NAME => Some(Command::HelpText {
            name: services::config::NAME.to_string(),
            text: cli_schema::render_help_for_path(&[services::config::NAME])?,
        }),
        _ => None,
    }
}

fn clean_clap_error_message(message: &str, kind: clap::error::ErrorKind) -> String {
    use clap::error::ErrorKind;

    let message = message.strip_prefix("error: ").unwrap_or(message);

    match kind {
        ErrorKind::InvalidSubcommand => {
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
            if let Some(arg) = extract_quoted_value(message) {
                format!(
                    "Unknown option '{arg}'. Try: run 'sce --help' to see top-level usage, or use 'sce <command> --help' for command-specific options."
                )
            } else {
                format!("{message}. Try: run 'sce --help' to see valid usage.")
            }
        }
        ErrorKind::MissingRequiredArgument => {
            if message.contains("required") {
                format!("{message}. Try: run 'sce --help' to see required arguments.")
            } else {
                format!("{message}. Try: run 'sce --help' to see valid usage.")
            }
        }
        ErrorKind::ArgumentConflict => {
            if message.contains("cannot be used with") || message.contains("conflicts with") {
                format!("{message}. Try: use only one of the conflicting options.")
            } else {
                format!("{message}. Try: run 'sce --help' to see valid usage.")
            }
        }
        _ => {
            if message.contains("Try:") {
                message.to_string()
            } else {
                format!("{message}. Try: run 'sce --help' to see valid usage.")
            }
        }
    }
}

fn extract_quoted_value(message: &str) -> Option<String> {
    let start = message.find('\'')?;
    let end = message[start + 1..].find('\'')?;
    Some(message[start + 1..start + 1 + end].to_string())
}

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
        cli_schema::Commands::Doctor { fix, format } => {
            Ok(Command::Doctor(services::doctor::DoctorRequest {
                mode: if fix {
                    services::doctor::DoctorMode::Fix
                } else {
                    services::doctor::DoctorMode::Diagnose
                },
                format: convert_output_format(format),
            }))
        }
        cli_schema::Commands::Hooks { subcommand } => convert_hooks_subcommand(subcommand),
        cli_schema::Commands::Trace { subcommand } => convert_trace_subcommand(subcommand),
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
        cli_schema::AuthSubcommand::Renew { format, force } => {
            services::auth_command::AuthSubcommand::Renew {
                format: convert_output_format(format),
                force,
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

fn convert_output_format(
    format: cli_schema::OutputFormat,
) -> services::output_format::OutputFormat {
    match format {
        cli_schema::OutputFormat::Text => services::output_format::OutputFormat::Text,
        cli_schema::OutputFormat::Json => services::output_format::OutputFormat::Json,
    }
}

fn convert_completion_shell(
    shell: cli_schema::CompletionShell,
) -> services::completion::CompletionShell {
    match shell {
        cli_schema::CompletionShell::Bash => services::completion::CompletionShell::Bash,
        cli_schema::CompletionShell::Zsh => services::completion::CompletionShell::Zsh,
        cli_schema::CompletionShell::Fish => services::completion::CompletionShell::Fish,
    }
}

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

fn convert_log_level(level: cli_schema::LogLevel) -> services::config::LogLevel {
    match level {
        cli_schema::LogLevel::Error => services::config::LogLevel::Error,
        cli_schema::LogLevel::Warn => services::config::LogLevel::Warn,
        cli_schema::LogLevel::Info => services::config::LogLevel::Info,
        cli_schema::LogLevel::Debug => services::config::LogLevel::Debug,
    }
}

#[allow(clippy::fn_params_excessive_bools)]
fn convert_setup_command(
    opencode: bool,
    claude: bool,
    both: bool,
    non_interactive: bool,
    hooks: bool,
    repo: Option<std::path::PathBuf>,
) -> Result<Command, ClassifiedError> {
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

#[allow(clippy::unnecessary_wraps)]
fn convert_trace_subcommand(
    subcommand: cli_schema::TraceSubcommand,
) -> Result<Command, ClassifiedError> {
    match subcommand {
        cli_schema::TraceSubcommand::Prompts {
            commit_sha,
            format,
            json,
        } => Ok(Command::Trace(services::trace::TraceRequest {
            subcommand: services::trace::TraceSubcommand::Prompts(
                services::trace::TracePromptsRequest {
                    commit_sha,
                    format: if json {
                        services::trace::TraceFormat::Json
                    } else {
                        convert_output_format(format)
                    },
                },
            ),
        })),
    }
}

fn dispatch(
    command: &Command,
    _logger: &services::observability::Logger,
) -> Result<String, ClassifiedError> {
    match command {
        Command::Help => Ok(command_surface::help_text()),
        Command::HelpText { text, .. } => Ok(text.to_owned()),
        Command::Auth(request) => services::auth_command::run_auth_subcommand(*request)
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
        Command::Completion(request) => Ok(services::completion::render_completion(*request)),
        // Clone required: run_config_subcommand takes ownership of ConfigSubcommand
        Command::Config(subcommand) => services::config::run_config_subcommand(subcommand.clone())
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
        Command::Setup(request) => {
            let current_dir = std::env::current_dir()
                .context("Failed to determine current directory")
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?;

            let preflight_hooks_repository = if request.install_hooks {
                let repository_root = request
                    .hooks_repo_path
                    .as_deref()
                    .unwrap_or(current_dir.as_path());
                Some(
                    services::setup::prepare_setup_hooks_repository(repository_root)
                        .map_err(|error| ClassifiedError::runtime(error.to_string()))?,
                )
            } else {
                None
            };

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
                        return Ok(services::setup::setup_cancelled_text());
                    }
                }
            }

            if request.install_hooks {
                let repository_root = preflight_hooks_repository
                    .as_deref()
                    .expect("hook repository preflight should exist when install_hooks is true");
                let hooks_message = services::setup::run_setup_hooks(repository_root)
                    .map_err(|error| ClassifiedError::runtime(error.to_string()))?;
                sections.push(hooks_message);
            }

            Ok(sections.join("\n\n"))
        }
        Command::Doctor(request) => services::doctor::run_doctor(*request)
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
        // Clone required: run_hooks_subcommand takes ownership of HookSubcommand
        Command::Hooks(subcommand) => services::hooks::run_hooks_subcommand(subcommand.clone())
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
        // Clone required: run_trace_subcommand takes ownership of TraceRequest
        Command::Trace(request) => services::trace::run_trace_subcommand(request.clone())
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
        Command::Sync(request) => services::sync::run_placeholder_sync(*request)
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
        Command::Version(request) => services::version::render_version(*request)
            .map_err(|error| ClassifiedError::runtime(error.to_string())),
    }
}
