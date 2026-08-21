use crate::services::config;
use crate::services::error::{CliError, UserError};

pub struct ConfigCommand {
    pub subcommand: config::ConfigSubcommand,
}

impl ConfigCommand {
    pub fn execute<C>(&self, _context: &C) -> Result<String, CliError> {
        config::run_config_subcommand(self.subcommand.clone())
            .map_err(|source| CliError::user_with_source(UserError::UnexpectedFailure, source))
    }
}
