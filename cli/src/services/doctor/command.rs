use std::borrow::Cow;

use crate::app::AppContext;
use crate::services::command_registry::{RuntimeCommand, RuntimeCommandHandle};
use crate::services::doctor;
use crate::services::error::ClassifiedError;

pub struct DoctorCommand {
    pub request: doctor::DoctorRequest,
}

impl RuntimeCommand for DoctorCommand {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(doctor::NAME)
    }

    fn execute(&self, _context: &AppContext) -> Result<String, ClassifiedError> {
        doctor::run_doctor(self.request)
            .map_err(|error| ClassifiedError::runtime(error.to_string()))
    }
}

/// Construct a `DoctorCommand` with a default diagnose-text request (used by the registry).
///
/// This default constructor is available for registry-based dispatch.
/// The parse layer constructs `DoctorCommand` with the user's chosen mode and format.
#[allow(dead_code)]
pub fn make_doctor_command() -> RuntimeCommandHandle {
    Box::new(DoctorCommand {
        request: doctor::DoctorRequest {
            mode: doctor::DoctorMode::Diagnose,
            format: doctor::DoctorFormat::Text,
        },
    })
}
