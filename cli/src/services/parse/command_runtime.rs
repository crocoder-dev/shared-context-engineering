use std::path::PathBuf;

use crate::{cli_schema, command_surface, services};
use services::command_registry::{CommandRegistry, RuntimeCommandHandle};
use services::error::{ClassifiedError, FailureClass};
use services::observability::traits::Logger as LoggerTrait;

pub fn parse_runtime_command<I>(
    args: I,
    registry: &CommandRegistry,
    logger: Option<&dyn LoggerTrait>,
) -> Result<RuntimeCommandHandle, ClassifiedError>
where
    I: IntoIterator<Item = String>,
{
    let args_vec: Vec<String> = args.into_iter().collect();

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
        return registry_command(registry, services::help::NAME);
    }

    let cli = match cli_schema::Cli::try_parse_from(&args_vec) {
        Ok(cli) => cli,
        Err(error) => return handle_clap_error(&args_vec, registry, &error),
    };

    let Some(command) = cli.command else {
        return registry_command(registry, services::help::NAME);
    };

    convert_clap_command(command)
}

fn handle_clap_error(
    args: &[String],
    registry: &CommandRegistry,
    error: &clap::Error,
) -> Result<RuntimeCommandHandle, ClassifiedError> {
    if error.kind() == clap::error::ErrorKind::DisplayHelp {
        if let Some((name, text)) = render_subcommand_help_from_args(args) {
            return Ok(Box::new(services::help::command::HelpTextCommand {
                name,
                text,
            }));
        }

        return registry_command(registry, services::help::NAME);
    }

    if error.kind() == clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand {
        if let Some(help_text) = render_missing_subcommand_help(args) {
            return Ok(help_text);
        }

        return Err(ClassifiedError::parse(
            "Missing required subcommand. Try: run 'sce --help' to see valid commands.",
        ));
    }

    if error.kind() == clap::error::ErrorKind::DisplayVersion {
        return registry_command(registry, services::version::NAME);
    }

    Err(classify_clap_error(error))
}

fn registry_command(
    registry: &CommandRegistry,
    name: &str,
) -> Result<RuntimeCommandHandle, ClassifiedError> {
    let constructor = registry.get(name).ok_or_else(|| {
        ClassifiedError::runtime(format!(
            "Command '{name}' is not registered. Try: run 'sce --help' to see available commands."
        ))
    })?;

    Ok(constructor())
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

fn render_missing_subcommand_help(args: &[String]) -> Option<RuntimeCommandHandle> {
    let command_name = args.get(1)?.as_str();

    match command_name {
        services::auth_command::NAME => Some(Box::new(services::help::command::HelpTextCommand {
            name: services::auth_command::NAME.to_string(),
            text: cli_schema::auth_help_text(),
        })),
        services::config::NAME => Some(Box::new(services::help::command::HelpTextCommand {
            name: services::config::NAME.to_string(),
            text: cli_schema::render_help_for_path(&[services::config::NAME])?,
        })),
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

fn convert_clap_command(
    command: cli_schema::Commands,
) -> Result<RuntimeCommandHandle, ClassifiedError> {
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
            Ok(Box::new(services::doctor::command::DoctorCommand {
                request: services::doctor::DoctorRequest {
                    mode: if fix {
                        services::doctor::DoctorMode::Fix
                    } else {
                        services::doctor::DoctorMode::Diagnose
                    },
                    format: convert_output_format(format),
                },
            }))
        }
        cli_schema::Commands::Hooks { subcommand } => convert_hooks_subcommand(subcommand),
        cli_schema::Commands::Version { format } => {
            Ok(Box::new(services::version::command::VersionCommand {
                request: services::version::VersionRequest {
                    format: convert_output_format(format),
                },
            }))
        }
        cli_schema::Commands::Completion { shell } => {
            Ok(Box::new(services::completion::command::CompletionCommand {
                request: services::completion::CompletionRequest {
                    shell: convert_completion_shell(shell),
                },
            }))
        }
    }
}

#[allow(clippy::unnecessary_wraps, clippy::needless_pass_by_value)]
fn convert_auth_subcommand(
    subcommand: cli_schema::AuthSubcommand,
) -> Result<RuntimeCommandHandle, ClassifiedError> {
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

    Ok(Box::new(services::auth_command::command::AuthCommand {
        request: services::auth_command::AuthRequest { subcommand },
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
) -> Result<RuntimeCommandHandle, ClassifiedError> {
    match subcommand {
        cli_schema::ConfigSubcommand::Show {
            format,
            config,
            log_level,
            timeout_ms,
        } => Ok(Box::new(services::config::command::ConfigCommand {
            subcommand: services::config::ConfigSubcommand::Show(services::config::ConfigRequest {
                report_format: convert_output_format(format),
                config_path: config,
                log_level: log_level.map(convert_log_level),
                timeout_ms,
            }),
        })),
        cli_schema::ConfigSubcommand::Validate {
            format,
            config,
            log_level,
            timeout_ms,
        } => Ok(Box::new(services::config::command::ConfigCommand {
            subcommand: services::config::ConfigSubcommand::Validate(
                services::config::ConfigRequest {
                    report_format: convert_output_format(format),
                    config_path: config,
                    log_level: log_level.map(convert_log_level),
                    timeout_ms,
                },
            ),
        })),
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
    repo: Option<PathBuf>,
) -> Result<RuntimeCommandHandle, ClassifiedError> {
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

    Ok(Box::new(services::setup::command::SetupCommand { request }))
}

#[allow(clippy::unnecessary_wraps)]
fn convert_hooks_subcommand(
    subcommand: cli_schema::HooksSubcommand,
) -> Result<RuntimeCommandHandle, ClassifiedError> {
    match subcommand {
        cli_schema::HooksSubcommand::PreCommit => {
            Ok(Box::new(services::hooks::command::HooksCommand {
                subcommand: services::hooks::HookSubcommand::PreCommit,
            }))
        }
        cli_schema::HooksSubcommand::CommitMsg { message_file } => {
            Ok(Box::new(services::hooks::command::HooksCommand {
                subcommand: services::hooks::HookSubcommand::CommitMsg { message_file },
            }))
        }
        cli_schema::HooksSubcommand::PostCommit => {
            Ok(Box::new(services::hooks::command::HooksCommand {
                subcommand: services::hooks::HookSubcommand::PostCommit,
            }))
        }
        cli_schema::HooksSubcommand::PostRewrite { rewrite_method } => {
            Ok(Box::new(services::hooks::command::HooksCommand {
                subcommand: services::hooks::HookSubcommand::PostRewrite { rewrite_method },
            }))
        }
        cli_schema::HooksSubcommand::DiffTrace => {
            Ok(Box::new(services::hooks::command::HooksCommand {
                subcommand: services::hooks::HookSubcommand::DiffTrace,
            }))
        }
    }
}
