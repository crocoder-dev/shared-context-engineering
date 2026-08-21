use crate::services::error::{CliError, UserError};
use crate::services::version;

pub struct VersionCommand {
    pub request: version::VersionRequest,
}

impl VersionCommand {
    pub fn execute<C>(&self, _context: &C) -> Result<String, CliError> {
        version::render_version(self.request)
            .map_err(|source| CliError::user_with_source(UserError::UnexpectedFailure, source))
    }
}
