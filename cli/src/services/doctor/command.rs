use crate::app::ContextWithRepoRoot;
use crate::services::doctor;
use crate::services::error::CliError;

pub struct DoctorCommand {
    pub request: doctor::DoctorRequest,
}

impl DoctorCommand {
    pub fn execute<C: ContextWithRepoRoot>(&self, context: &C) -> Result<String, CliError> {
        doctor::run_doctor_with_context(self.request, context)
            .map_err(|error| CliError::runtime(anyhow::Error::msg(format!("{error:#}"))))
    }
}
