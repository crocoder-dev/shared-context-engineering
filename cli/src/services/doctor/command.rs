use crate::app::ContextWithRepoRoot;
use crate::services::doctor;
use crate::services::error::{CliError, UserError};

pub struct DoctorCommand {
    pub request: doctor::DoctorRequest,
}

impl DoctorCommand {
    pub async fn execute<C: ContextWithRepoRoot + crate::app::HasGit>(
        &self,
        context: &C,
    ) -> Result<String, CliError> {
        doctor::run_doctor_with_context(self.request, context)
            .await
            .map_err(|source| CliError::user_with_source(UserError::UnexpectedFailure, source))
    }
}
