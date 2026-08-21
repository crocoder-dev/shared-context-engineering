use crate::app::ContextWithRepoRoot;
use crate::services::doctor;
use crate::services::error::{CliError, UserError};

pub struct DoctorCommand {
    pub request: doctor::DoctorRequest,
}

impl DoctorCommand {
    pub fn execute<C: ContextWithRepoRoot>(&self, context: &C) -> Result<String, CliError> {
        doctor::run_doctor_with_context(self.request, context)
            .map_err(|source| CliError::user_with_source(UserError::UnexpectedFailure, source))
    }
}
