use std::borrow::Cow;
use std::io::Write;

use crate::app::{ContextWithRepoRoot, HasLogger};
use crate::services;
use crate::services::error::CliError;

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

    pub async fn execute_with_stderr<C, W>(
        &self,
        context: &C,
        stderr: &mut W,
    ) -> Result<String, CliError>
    where
        C: HasLogger + ContextWithRepoRoot + crate::app::HasGit + crate::app::HasFs,
        W: Write,
    {
        match self {
            Self::Help(_) => Ok(services::help::help_text()),
            Self::HelpText(command) => Ok(command.execute(context)),
            Self::Auth(command) => command.execute(context).await,
            Self::Config(command) => command.execute(context),
            Self::Setup(command) => command.execute(context).await,
            Self::Doctor(command) => command.execute(context).await,
            Self::Hooks(command) => command.execute(context).await,
            Self::Policy(command) => command.execute(),
            Self::Version(command) => command.execute(context),
            Self::Completion(command) => Ok(command.execute(context)),
            Self::Sync(command) => command.execute_with_stderr(context, stderr).await,
        }
    }
}
