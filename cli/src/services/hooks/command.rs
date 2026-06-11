use crate::app::HasLogger;
use crate::services::error::ClassifiedError;
use crate::services::hooks;

pub struct HooksCommand {
    pub subcommand: hooks::HookSubcommand,
}

impl HooksCommand {
    pub fn execute<C: HasLogger>(&self, context: &C) -> Result<String, ClassifiedError> {
        hooks::run_hooks_subcommand(&self.subcommand, Some(context.logger()))
            .map_err(|error| ClassifiedError::runtime(error.to_string()))
    }
}
