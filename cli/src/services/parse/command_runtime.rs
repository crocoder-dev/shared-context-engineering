use std::path::PathBuf;

use crate::{cli_schema, command_surface, services};
use services::command_registry::{CommandRegistry, RuntimeCommand};
use services::error::{ClassifiedError, FailureClass};
use services::observability::traits::Logger as LoggerTrait;

pub fn parse_runtime_command<I>(
    args: I,
    registry: &CommandRegistry,
    logger: Option<&dyn LoggerTrait>,
) -> Result<RuntimeCommand, ClassifiedError>
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
) -> Result<RuntimeCommand, ClassifiedError> {
    if error.kind() == clap::error::ErrorKind::DisplayHelp {
        if let Some((name, text)) = render_subcommand_help_from_args(args) {
            return Ok(RuntimeCommand::HelpText(
                services::help::command::HelpTextCommand { name, text },
            ));
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
) -> Result<RuntimeCommand, ClassifiedError> {
    if !registry.contains(name) {
        return Err(ClassifiedError::runtime(format!(
            "Command '{name}' is not registered. Try: run 'sce --help' to see available commands."
        )));
    }

    services::command_registry::default_runtime_command(name).ok_or_else(|| {
        ClassifiedError::runtime(format!(
            "Command '{name}' is not registered. Try: run 'sce --help' to see available commands."
        ))
    })
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

fn render_missing_subcommand_help(args: &[String]) -> Option<RuntimeCommand> {
    let command_name = args.get(1)?.as_str();

    match command_name {
        services::auth_command::NAME => Some(RuntimeCommand::HelpText(
            services::help::command::HelpTextCommand {
                name: services::auth_command::NAME.to_string(),
                text: cli_schema::auth_help_text(),
            },
        )),
        services::config::NAME => Some(RuntimeCommand::HelpText(
            services::help::command::HelpTextCommand {
                name: services::config::NAME.to_string(),
                text: cli_schema::render_help_for_path(&[services::config::NAME])?,
            },
        )),
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

fn convert_clap_command(command: cli_schema::Commands) -> Result<RuntimeCommand, ClassifiedError> {
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
        cli_schema::Commands::Doctor { fix, format } => Ok(convert_doctor_command(fix, format)),
        cli_schema::Commands::Hooks { subcommand } => convert_hooks_subcommand(subcommand),
        cli_schema::Commands::Policy { subcommand } => Ok(convert_policy_subcommand(&subcommand)),
        cli_schema::Commands::Version { format } => Ok(RuntimeCommand::Version(
            services::version::command::VersionCommand {
                request: services::version::VersionRequest { format },
            },
        )),
        cli_schema::Commands::Completion { shell } => Ok(RuntimeCommand::Completion(
            services::completion::command::CompletionCommand {
                request: services::completion::CompletionRequest {
                    shell: convert_completion_shell(shell),
                },
            },
        )),
        cli_schema::Commands::Trace { subcommand } => Ok(convert_trace_subcommand(subcommand)),
    }
}

#[allow(clippy::needless_pass_by_value)]
fn convert_trace_subcommand(subcommand: cli_schema::TraceSubcommand) -> RuntimeCommand {
    let request = match subcommand {
        cli_schema::TraceSubcommand::Db { subcommand } => match subcommand {
            cli_schema::TraceDbSubcommand::List { format } => services::trace::TraceRequest {
                subcommand: services::trace::TraceSubcommandRequest::DbList { format },
            },
            cli_schema::TraceDbSubcommand::Shell { identifier } => services::trace::TraceRequest {
                subcommand: services::trace::TraceSubcommandRequest::DbShell { identifier },
            },
        },
        cli_schema::TraceSubcommand::Status { all, format } => services::trace::TraceRequest {
            subcommand: services::trace::TraceSubcommandRequest::Status { all, format },
        },
    };

    RuntimeCommand::Trace(services::trace::command::TraceCommand { request })
}

fn convert_doctor_command(
    fix: bool,
    format: services::output_format::OutputFormat,
) -> RuntimeCommand {
    let request = services::doctor::DoctorRequest {
        mode: if fix {
            services::doctor::DoctorMode::Fix
        } else {
            services::doctor::DoctorMode::Diagnose
        },
        format,
    };

    RuntimeCommand::Doctor(services::doctor::command::DoctorCommand { request })
}

fn convert_policy_subcommand(subcommand: &cli_schema::PolicySubcommand) -> RuntimeCommand {
    let request = match subcommand {
        cli_schema::PolicySubcommand::Bash { input, output } => {
            services::bash_policy::BashPolicyRequest {
                input: convert_policy_input_mode(*input),
                output: convert_policy_output_mode(*output),
            }
        }
    };

    RuntimeCommand::Policy(services::bash_policy::command::PolicyCommand { request })
}

fn convert_policy_input_mode(
    input: cli_schema::PolicyInputMode,
) -> services::bash_policy::PolicyInputMode {
    match input {
        cli_schema::PolicyInputMode::ClaudePreToolUse => {
            services::bash_policy::PolicyInputMode::ClaudePreToolUse
        }
        cli_schema::PolicyInputMode::Normalized => {
            services::bash_policy::PolicyInputMode::Normalized
        }
    }
}

fn convert_policy_output_mode(
    output: cli_schema::PolicyOutputMode,
) -> services::bash_policy::PolicyOutputMode {
    match output {
        cli_schema::PolicyOutputMode::ClaudeHook => {
            services::bash_policy::PolicyOutputMode::ClaudeHook
        }
        cli_schema::PolicyOutputMode::Json => services::bash_policy::PolicyOutputMode::Json,
    }
}

#[allow(clippy::unnecessary_wraps, clippy::needless_pass_by_value)]
fn convert_auth_subcommand(
    subcommand: cli_schema::AuthSubcommand,
) -> Result<RuntimeCommand, ClassifiedError> {
    let subcommand = match subcommand {
        cli_schema::AuthSubcommand::Login { format } => {
            services::auth_command::AuthSubcommand::Login { format }
        }
        cli_schema::AuthSubcommand::Renew { format, force } => {
            services::auth_command::AuthSubcommand::Renew { format, force }
        }
        cli_schema::AuthSubcommand::Logout { format } => {
            services::auth_command::AuthSubcommand::Logout { format }
        }
        cli_schema::AuthSubcommand::Status { format } => {
            services::auth_command::AuthSubcommand::Status { format }
        }
    };

    Ok(RuntimeCommand::Auth(
        services::auth_command::command::AuthCommand {
            request: services::auth_command::AuthRequest { subcommand },
        },
    ))
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
) -> Result<RuntimeCommand, ClassifiedError> {
    match subcommand {
        cli_schema::ConfigSubcommand::Show {
            format,
            config,
            log_level,
            timeout_ms,
        } => Ok(RuntimeCommand::Config(
            services::config::command::ConfigCommand {
                subcommand: services::config::ConfigSubcommand::Show(
                    services::config::ConfigRequest {
                        report_format: format,
                        config_path: config,
                        log_level,
                        timeout_ms,
                    },
                ),
            },
        )),
        cli_schema::ConfigSubcommand::Validate {
            format,
            config,
            log_level,
            timeout_ms,
        } => Ok(RuntimeCommand::Config(
            services::config::command::ConfigCommand {
                subcommand: services::config::ConfigSubcommand::Validate(
                    services::config::ConfigRequest {
                        report_format: format,
                        config_path: config,
                        log_level,
                        timeout_ms,
                    },
                ),
            },
        )),
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
) -> Result<RuntimeCommand, ClassifiedError> {
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

    Ok(RuntimeCommand::Setup(
        services::setup::command::SetupCommand { request },
    ))
}

#[allow(clippy::unnecessary_wraps)]
fn convert_hooks_subcommand(
    subcommand: cli_schema::HooksSubcommand,
) -> Result<RuntimeCommand, ClassifiedError> {
    let subcommand = convert_hooks_subcommand_request(subcommand)?;

    Ok(RuntimeCommand::Hooks(
        services::hooks::command::HooksCommand { subcommand },
    ))
}

fn convert_hooks_subcommand_request(
    subcommand: cli_schema::HooksSubcommand,
) -> Result<services::hooks::HookSubcommand, ClassifiedError> {
    match subcommand {
        cli_schema::HooksSubcommand::PreCommit => Ok(services::hooks::HookSubcommand::PreCommit),
        cli_schema::HooksSubcommand::CommitMsg { message_file } => {
            Ok(services::hooks::HookSubcommand::CommitMsg { message_file })
        }
        cli_schema::HooksSubcommand::PostCommit { vcs, remote_url } => {
            let vcs_type = parse_optional_hook_vcs_type(vcs.as_deref())
                .map_err(ClassifiedError::validation)?;
            let remote_url =
                parse_optional_hook_remote_url(remote_url).map_err(ClassifiedError::validation)?;

            Ok(services::hooks::HookSubcommand::PostCommit {
                vcs_type,
                remote_url: Some(remote_url),
            })
        }
        cli_schema::HooksSubcommand::PostRewrite { rewrite_method } => {
            Ok(services::hooks::HookSubcommand::PostRewrite { rewrite_method })
        }
        cli_schema::HooksSubcommand::DiffTrace => Ok(services::hooks::HookSubcommand::DiffTrace),
        cli_schema::HooksSubcommand::ConversationTrace => {
            Ok(services::hooks::HookSubcommand::ConversationTrace)
        }
    }
}

fn parse_optional_hook_vcs_type(
    vcs: Option<&str>,
) -> Result<Option<services::agent_trace::AgentTraceVcsType>, String> {
    let Some(vcs) = vcs else {
        return Ok(None);
    };

    let normalized = vcs.trim().to_ascii_lowercase();

    match normalized.as_str() {
        "git" => Ok(Some(services::agent_trace::AgentTraceVcsType::Git)),
        "jj" => Ok(Some(services::agent_trace::AgentTraceVcsType::Jj)),
        "hg" => Ok(Some(services::agent_trace::AgentTraceVcsType::Hg)),
        "svn" => Ok(Some(services::agent_trace::AgentTraceVcsType::Svn)),
        _ => Err(format!(
            "Unsupported value for '--vcs': '{vcs}'. Supported values: git, jj, hg, svn."
        )),
    }
}

fn parse_optional_hook_remote_url(remote_url: Option<String>) -> Result<String, String> {
    match remote_url {
        Some(url) if !url.trim().is_empty() => Ok(url),
        _ => Err("Missing required option '--remote-url' for 'sce hooks post-commit'.".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> RuntimeCommand {
        parse_runtime_command(
            args.iter().map(|arg| (*arg).to_string()),
            &CommandRegistry::default(),
            None,
        )
        .expect("command should parse")
    }

    #[test]
    fn trace_db_shell_parses_to_trace_shell_request() {
        let command = parse(&["sce", "trace", "db", "shell", "agent_trace_0"]);

        let RuntimeCommand::Trace(command) = command else {
            panic!("expected trace command");
        };

        assert_eq!(
            command.request.subcommand,
            services::trace::TraceSubcommandRequest::DbShell {
                identifier: String::from("agent_trace_0"),
            }
        );
    }

    #[test]
    fn trace_db_help_lists_shell_subcommand() {
        let command = parse(&["sce", "trace", "db", "--help"]);

        let RuntimeCommand::HelpText(command) = command else {
            panic!("expected help text command");
        };

        assert!(command.text.contains("shell"));
        assert!(command
            .text
            .contains("Open an embedded SQL shell for a discovered Agent Trace database"));
    }
}
