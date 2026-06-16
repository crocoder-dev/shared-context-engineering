use crate::services::error::ClassifiedError;
use crate::services::version;

pub struct VersionCommand {
    pub request: version::VersionRequest,
}

impl VersionCommand {
    pub fn execute<C>(&self, _context: &C) -> Result<String, ClassifiedError> {
        version::render_version(self.request)
            .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))
    }
}
