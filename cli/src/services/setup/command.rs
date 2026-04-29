use std::borrow::Cow;

use anyhow::Context;

use crate::app::AppContext;
use crate::services::command_registry::{RuntimeCommand, RuntimeCommandHandle};
use crate::services::error::ClassifiedError;
use crate::services::setup;

pub struct SetupCommand {
    pub request: setup::SetupRequest,
}

impl RuntimeCommand for SetupCommand {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(setup::NAME)
    }

    fn execute(&self, _context: &AppContext) -> Result<String, ClassifiedError> {
        let current_dir = std::env::current_dir()
            .context("Failed to determine current directory")
            .map_err(|error| ClassifiedError::runtime(error.to_string()))?;

        let repository_root = setup::ensure_git_repository(&current_dir)
            .map_err(|error| ClassifiedError::runtime(error.to_string()))?;

        setup::bootstrap_repo_local_config(&repository_root)
            .map_err(|error| ClassifiedError::runtime(error.to_string()))?;

        setup::bootstrap_local_db().map_err(|error| ClassifiedError::runtime(error.to_string()))?;

        let preflight_hooks_repository = if self.request.install_hooks {
            let hooks_repository = self
                .request
                .hooks_repo_path
                .as_deref()
                .unwrap_or(repository_root.as_path());
            Some(
                setup::prepare_setup_hooks_repository(hooks_repository)
                    .map_err(|error| ClassifiedError::runtime(error.to_string()))?,
            )
        } else {
            None
        };

        let mut sections = Vec::new();

        if let Some(mode) = self.request.config_mode {
            let dispatch = setup::resolve_setup_dispatch(mode, &setup::InquireSetupTargetPrompter)
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?;

            match dispatch {
                setup::SetupDispatch::Proceed(resolved_mode) => {
                    let setup_message = setup::run_setup_for_mode(&repository_root, resolved_mode)
                        .map_err(|error| ClassifiedError::runtime(error.to_string()))?;
                    sections.push(setup_message);
                }
                setup::SetupDispatch::Cancelled => {
                    return Ok(setup::setup_cancelled_text());
                }
            }
        }

        if self.request.install_hooks {
            let repository_root = preflight_hooks_repository
                .as_deref()
                .expect("hook repository preflight should exist when install_hooks is true");
            let hooks_message = setup::run_setup_hooks(repository_root)
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?;
            sections.push(hooks_message);
        }

        Ok(sections.join("\n\n"))
    }
}

/// Construct a `SetupCommand` with a default interactive setup request (used by the registry).
///
/// This default constructor is available for registry-based dispatch.
/// The parse layer constructs `SetupCommand` with the user's chosen options.
#[allow(dead_code)]
pub fn make_setup_command() -> RuntimeCommandHandle {
    Box::new(SetupCommand {
        request: setup::SetupRequest {
            config_mode: Some(setup::SetupMode::Interactive),
            install_hooks: true,
            hooks_repo_path: None,
        },
    })
}
