use crate::app::AppContext;
use crate::services::error::ClassifiedError;
use crate::services::version;

pub struct VersionCommand {
    pub request: version::VersionRequest,
}

impl VersionCommand {
    pub fn execute(&self, _context: &AppContext) -> Result<String, ClassifiedError> {
        version::render_version(self.request)
            .map_err(|error| ClassifiedError::runtime(error.to_string()))
    }
}
