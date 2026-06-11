use std::borrow::Cow;

use crate::app::AppContext;
use crate::services;
use crate::services::error::ClassifiedError;

const DEFAULT_COMMAND_NAMES: &[&str] = &[
    services::auth_command::NAME,
    services::completion::NAME,
    services::config::NAME,
    services::doctor::NAME,
    services::help::NAME,
    services::hooks::NAME,
    services::setup::NAME,
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
    Version(services::version::command::VersionCommand),
    Completion(services::completion::command::CompletionCommand),
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
            Self::Version(_) => Cow::Borrowed(services::version::NAME),
            Self::Completion(_) => Cow::Borrowed(services::completion::NAME),
        }
    }

    pub fn execute(&self, context: &AppContext) -> Result<String, ClassifiedError> {
        match self {
            Self::Help(_) => Ok(services::help::help_text()),
            Self::HelpText(command) => Ok(command.execute(context)),
            Self::Auth(command) => command.execute(context),
            Self::Config(command) => command.execute(context),
            Self::Setup(command) => command.execute(context),
            Self::Doctor(command) => command.execute(context),
            Self::Hooks(command) => command.execute(context),
            Self::Version(command) => command.execute(context),
            Self::Completion(command) => Ok(command.execute(context)),
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
                    subcommand: services::auth_command::AuthSubcommand::Status {
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
                        timeout_ms: None,
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
                "setup",
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
        assert!(!registry.contains("sync"));
    }

    #[test]
    fn default_runtime_commands_have_expected_names() {
        for name in DEFAULT_COMMAND_NAMES {
            let command = default_runtime_command(name).expect("command should exist");
            assert_eq!(command.name(), *name);
        }
        assert!(default_runtime_command("sync").is_none());
    }
}
