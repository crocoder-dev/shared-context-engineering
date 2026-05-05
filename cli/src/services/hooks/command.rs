use std::borrow::Cow;

use crate::app::AppContext;
use crate::services::command_registry::{RuntimeCommand, RuntimeCommandHandle};
use crate::services::error::ClassifiedError;
use crate::services::hooks;

pub struct HooksCommand {
    pub subcommand: hooks::HookSubcommand,
}

impl RuntimeCommand for HooksCommand {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(hooks::NAME)
    }

    fn execute(&self, context: &AppContext) -> Result<String, ClassifiedError> {
        hooks::run_hooks_subcommand(&self.subcommand, Some(context.logger()))
            .map_err(|error| ClassifiedError::runtime(error.to_string()))
    }
}

/// Build the default hooks command for registry dispatch.
pub fn make_hooks_command() -> RuntimeCommandHandle {
    Box::new(HooksCommand {
        subcommand: hooks::HookSubcommand::PreCommit,
    })
}
