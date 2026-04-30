use std::borrow::Cow;

use crate::app::AppContext;
use crate::services::command_registry::{RuntimeCommand, RuntimeCommandHandle};
use crate::services::error::ClassifiedError;
use crate::services::version;

pub struct VersionCommand {
    pub request: version::VersionRequest,
}

impl RuntimeCommand for VersionCommand {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(version::NAME)
    }

    fn execute(&self, _context: &AppContext) -> Result<String, ClassifiedError> {
        version::render_version(self.request)
            .map_err(|error| ClassifiedError::runtime(error.to_string()))
    }
}

/// Construct a `VersionCommand` with text format (used by the registry).
///
/// This default constructor is available for registry-based dispatch.
/// The parse layer constructs `VersionCommand` with the user's chosen format.
#[allow(dead_code)]
pub fn make_version_command() -> RuntimeCommandHandle {
    Box::new(VersionCommand {
        request: version::VersionRequest {
            format: version::VersionFormat::Text,
        },
    })
}
