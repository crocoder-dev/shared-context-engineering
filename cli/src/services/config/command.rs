use crate::app::AppContext;
use crate::services::config;
use crate::services::error::ClassifiedError;

pub struct ConfigCommand {
    pub subcommand: config::ConfigSubcommand,
}

impl ConfigCommand {
    pub fn execute(&self, _context: &AppContext) -> Result<String, ClassifiedError> {
        config::run_config_subcommand(self.subcommand.clone())
            .map_err(|error| ClassifiedError::runtime(error.to_string()))
    }
}
