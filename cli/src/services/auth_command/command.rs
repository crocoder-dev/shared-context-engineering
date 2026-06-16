use crate::services::auth_command;
use crate::services::error::ClassifiedError;

pub struct AuthCommand {
    pub request: auth_command::AuthRequest,
}

impl AuthCommand {
    pub fn execute<C>(&self, _context: &C) -> Result<String, ClassifiedError> {
        auth_command::run_auth_subcommand(self.request)
            .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))
    }
}
