use crate::app::AppContext;
use crate::services::doctor;
use crate::services::error::ClassifiedError;

pub struct DoctorCommand {
    pub request: doctor::DoctorRequest,
}

impl DoctorCommand {
    pub fn execute(&self, context: &AppContext) -> Result<String, ClassifiedError> {
        doctor::run_doctor_with_context(self.request, context)
            .map_err(|error| ClassifiedError::runtime(error.to_string()))
    }
}
