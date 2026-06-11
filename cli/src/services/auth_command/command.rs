use crate::app::AppContext;
use crate::services::auth_command;
use crate::services::error::ClassifiedError;

pub struct AuthCommand {
    pub request: auth_command::AuthRequest,
}

impl AuthCommand {
    pub fn execute(&self, _context: &AppContext) -> Result<String, ClassifiedError> {
        auth_command::run_auth_subcommand(self.request)
            .map_err(|error| ClassifiedError::runtime(error.to_string()))
    }
}
