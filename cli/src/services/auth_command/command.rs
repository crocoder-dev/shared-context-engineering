use std::borrow::Cow;

use crate::app::AppContext;
use crate::services::auth_command;
use crate::services::command_registry::{RuntimeCommand, RuntimeCommandHandle};
use crate::services::error::ClassifiedError;

pub struct AuthCommand {
    pub request: auth_command::AuthRequest,
}

impl RuntimeCommand for AuthCommand {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(auth_command::NAME)
    }

    fn execute(&self, _context: &AppContext) -> Result<String, ClassifiedError> {
        auth_command::run_auth_subcommand(self.request)
            .map_err(|error| ClassifiedError::runtime(error.to_string()))
    }
}

/// Construct an `AuthCommand` with a default login request (used by the registry).
///
/// This default constructor is available for registry-based dispatch.
/// The parse layer constructs `AuthCommand` with the user's chosen subcommand.
#[allow(dead_code)]
pub fn make_auth_command() -> RuntimeCommandHandle {
    Box::new(AuthCommand {
        request: auth_command::AuthRequest {
            subcommand: auth_command::AuthSubcommand::Status {
                format: auth_command::AuthFormat::Text,
            },
        },
    })
}
