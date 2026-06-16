use crate::services::config;
use crate::services::error::ClassifiedError;

pub struct ConfigCommand {
    pub subcommand: config::ConfigSubcommand,
}

impl ConfigCommand {
    pub fn execute<C>(&self, _context: &C) -> Result<String, ClassifiedError> {
        config::run_config_subcommand(self.subcommand.clone())
            .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))
    }
}
