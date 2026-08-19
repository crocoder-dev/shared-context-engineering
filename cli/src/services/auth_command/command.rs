use crate::services::auth_command;
use crate::services::error::CliError;

pub struct AuthCommand {
    pub request: auth_command::AuthRequest,
}

impl AuthCommand {
    pub fn execute<C>(&self, _context: &C) -> Result<String, CliError> {
        auth_command::run_auth_subcommand(self.request).map_err(CliError::runtime)
    }
}
