use crate::app::AppContext;
use crate::services::error::ClassifiedError;
use crate::services::hooks;

pub struct HooksCommand {
    pub subcommand: hooks::HookSubcommand,
}

impl HooksCommand {
    pub fn execute(&self, context: &AppContext) -> Result<String, ClassifiedError> {
        hooks::run_hooks_subcommand(&self.subcommand, Some(context.logger()))
            .map_err(|error| ClassifiedError::runtime(error.to_string()))
    }
}
