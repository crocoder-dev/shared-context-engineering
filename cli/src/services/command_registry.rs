use std::borrow::Cow;
use std::io::Write;

use crate::app::{ContextWithRepoRoot, HasLogger};
use crate::services;
use crate::services::error::CliError;

const DEFAULT_COMMAND_NAMES: &[&str] = &[
    services::auth_command::NAME,
    services::completion::NAME,
    services::config::NAME,
    services::doctor::NAME,
    services::help::NAME,
    services::hooks::NAME,
    services::bash_policy::NAME,
    services::setup::NAME,
    services::sync::NAME,
    services::version::NAME,
];

/// Static runtime command dispatcher for all known CLI commands.
///
/// Parsed command requests are represented as enum variants instead of boxed
/// trait objects. Each variant delegates to the same service-owned command
/// implementation used before the static-dispatch migration.
pub enum RuntimeCommand {
    Help(services::help::command::HelpCommand),
    HelpText(services::help::command::HelpTextCommand),
    Auth(services::auth_command::command::AuthCommand),
    Config(services::config::command::ConfigCommand),
    Setup(services::setup::command::SetupCommand),
    Doctor(services::doctor::command::DoctorCommand),
    Hooks(services::hooks::command::HooksCommand),
    Policy(services::bash_policy::command::PolicyCommand),
    Version(services::version::command::VersionCommand),
    Completion(services::completion::command::CompletionCommand),
    Sync(services::sync::command::SyncCommand),
}

impl RuntimeCommand {
    pub fn name(&self) -> Cow<'_, str> {
        match self {
            Self::Help(_) => Cow::Borrowed(services::help::NAME),
            Self::HelpText(command) => command.name(),
            Self::Auth(_) => Cow::Borrowed(services::auth_command::NAME),
            Self::Config(_) => Cow::Borrowed(services::config::NAME),
            Self::Setup(_) => Cow::Borrowed(services::setup::NAME),
            Self::Doctor(_) => Cow::Borrowed(services::doctor::NAME),
            Self::Hooks(_) => Cow::Borrowed(services::hooks::NAME),
            Self::Policy(_) => Cow::Borrowed(services::bash_policy::NAME),
            Self::Version(_) => Cow::Borrowed(services::version::NAME),
            Self::Completion(_) => Cow::Borrowed(services::completion::NAME),
            Self::Sync(_) => Cow::Borrowed(services::sync::NAME),
        }
    }

    #[allow(dead_code)]
    pub fn execute<C>(&self, context: &C) -> Result<String, CliError>
    where
        C: HasLogger + ContextWithRepoRoot,
    {
        let mut stderr = std::io::sink();
        self.execute_with_stderr(context, &mut stderr)
    }

    pub fn execute_with_stderr<C, W>(&self, context: &C, stderr: &mut W) -> Result<String, CliError>
    where
        C: HasLogger + ContextWithRepoRoot,
        W: Write,
    {
        match self {
            Self::Help(_) => Ok(services::help::help_text()),
            Self::HelpText(command) => Ok(command.execute(context)),
            Self::Auth(command) => command.execute(context),
            Self::Config(command) => command.execute(context),
            Self::Setup(command) => command.execute(context),
            Self::Doctor(command) => command.execute(context),
            Self::Hooks(command) => command.execute(context),
            Self::Policy(command) => command.execute(),
            Self::Version(command) => command.execute(context),
            Self::Completion(command) => Ok(command.execute(context)),
            Self::Sync(command) => command.execute_with_stderr(context, stderr),
        }
    }
}

/// Statically populated command catalog.
///
/// The catalog owns deterministic command-name lookup only. Per-invocation
/// command payloads are built by the parse layer as [`RuntimeCommand`] variants.
pub struct CommandRegistry {
    names: &'static [&'static str],
}

impl CommandRegistry {
    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(&name)
    }

    #[cfg(test)]
    pub fn command_names(&self) -> Vec<&'static str> {
        let mut names = self.names.to_vec();
        names.sort_unstable();
        names
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        build_default_registry()
    }
}

/// Build the default deterministic command catalog with all known commands.
pub fn build_default_registry() -> CommandRegistry {
    CommandRegistry {
        names: DEFAULT_COMMAND_NAMES,
    }
}

pub fn default_runtime_command(name: &str) -> Option<RuntimeCommand> {
    match name {
        services::help::NAME => Some(RuntimeCommand::Help(services::help::command::HelpCommand)),
        services::auth_command::NAME => Some(RuntimeCommand::Auth(
            services::auth_command::command::AuthCommand {
                request: services::auth_command::AuthRequest {
                    subcommand: services::auth_command::AuthSubcommand::Whoami {
                        format: services::auth_command::AuthFormat::Text,
                    },
                },
            },
        )),
        services::config::NAME => Some(RuntimeCommand::Config(
            services::config::command::ConfigCommand {
                subcommand: services::config::ConfigSubcommand::Show(
                    services::config::ConfigRequest {
                        report_format: services::config::ReportFormat::Text,
                        config_path: None,
                        log_level: None,
                    },
                ),
            },
        )),
        services::setup::NAME => Some(RuntimeCommand::Setup(
            services::setup::command::SetupCommand {
                request: services::setup::SetupRequest {
                    config_mode: Some(services::setup::SetupMode::Interactive),
                    install_hooks: true,
                    hooks_repo_path: None,
                    context_only: false,
                    optional_workflows: None,
                },
            },
        )),
        services::doctor::NAME => Some(RuntimeCommand::Doctor(
            services::doctor::command::DoctorCommand {
                request: services::doctor::DoctorRequest {
                    mode: services::doctor::DoctorMode::Diagnose,
                    format: services::doctor::DoctorFormat::Text,
                },
            },
        )),
        services::hooks::NAME => Some(RuntimeCommand::Hooks(
            services::hooks::command::HooksCommand {
                subcommand: services::hooks::HookSubcommand::PreCommit,
            },
        )),
        services::bash_policy::NAME => Some(RuntimeCommand::Policy(
            services::bash_policy::command::PolicyCommand {
                request: services::bash_policy::BashPolicyRequest {
                    input: services::bash_policy::PolicyInputMode::ClaudePreToolUse,
                    output: services::bash_policy::PolicyOutputMode::ClaudeHook,
                },
            },
        )),
        services::version::NAME => Some(RuntimeCommand::Version(
            services::version::command::VersionCommand {
                request: services::version::VersionRequest {
                    format: services::version::VersionFormat::Text,
                },
            },
        )),
        services::completion::NAME => Some(RuntimeCommand::Completion(
            services::completion::command::CompletionCommand {
                request: services::completion::CompletionRequest {
                    shell: services::completion::CompletionShell::Bash,
                },
            },
        )),
        services::sync::NAME => Some(RuntimeCommand::Sync(services::sync::command::SyncCommand {
            request: services::sync::SyncRequest {
                format: services::output_format::OutputFormat::Text,
            },
        })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_lists_all_commands_deterministically() {
        let registry = CommandRegistry::default();

        assert_eq!(
            registry.command_names(),
            vec![
                "auth",
                "completion",
                "config",
                "doctor",
                "help",
                "hooks",
                "policy",
                "setup",
                "sync",
                "version"
            ]
        );
    }

    #[test]
    fn default_registry_reports_known_command_names() {
        let registry = CommandRegistry::default();

        for name in DEFAULT_COMMAND_NAMES {
            assert!(registry.contains(name));
        }
        assert!(registry.contains("sync"));
    }

    #[test]
    fn default_runtime_commands_have_expected_names() {
        for name in DEFAULT_COMMAND_NAMES {
            let command = default_runtime_command(name).expect("command should exist");
            assert_eq!(command.name(), *name);
        }
        assert!(default_runtime_command("sync").is_some());
    }
}
