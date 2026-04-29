use std::borrow::Cow;

use crate::app::AppContext;
use crate::services::command_registry::{RuntimeCommand, RuntimeCommandHandle};
use crate::services::completion;
use crate::services::error::ClassifiedError;

pub struct CompletionCommand {
    pub request: completion::CompletionRequest,
}

impl RuntimeCommand for CompletionCommand {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(completion::NAME)
    }

    fn execute(&self, _context: &AppContext) -> Result<String, ClassifiedError> {
        Ok(completion::render_completion(self.request))
    }
}

/// Construct a `CompletionCommand` with Bash shell (used by the registry).
///
/// This default constructor is available for registry-based dispatch.
/// The parse layer constructs `CompletionCommand` with the user's chosen shell.
#[allow(dead_code)]
pub fn make_completion_command() -> RuntimeCommandHandle {
    Box::new(CompletionCommand {
        request: completion::CompletionRequest {
            shell: completion::CompletionShell::Bash,
        },
    })
}
