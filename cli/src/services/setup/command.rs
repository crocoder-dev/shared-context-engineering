use std::borrow::Cow;

use anyhow::Context;

use crate::app::AppContext;
use crate::services::command_registry::{RuntimeCommand, RuntimeCommandHandle};
use crate::services::error::ClassifiedError;
use crate::services::lifecycle::{
    lifecycle_providers, RequiredHookInstallStatus, RequiredHooksInstallOutcome,
};
use crate::services::setup;

pub struct SetupCommand {
    pub request: setup::SetupRequest,
}

impl RuntimeCommand for SetupCommand {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(setup::NAME)
    }

    fn execute(&self, context: &AppContext) -> Result<String, ClassifiedError> {
        let setup_dispatch = if let Some(mode) = self.request.config_mode {
            match setup::resolve_setup_dispatch(mode, &setup::InquireSetupTargetPrompter)
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?
            {
                setup::SetupDispatch::Proceed(resolved_mode) => Some(resolved_mode),
                setup::SetupDispatch::Cancelled => {
                    return Ok(setup::setup_cancelled_text());
                }
            }
        } else {
            None
        };

        let setup_start_path = match &self.request.hooks_repo_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .context("Failed to determine current directory")
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?,
        };

        let repository_root = setup::ensure_git_repository(&setup_start_path)
            .map_err(|error| ClassifiedError::runtime(error.to_string()))?;

        // Scope the runtime AppContext to the resolved repository root for lifecycle providers.
        let ctx = context.with_repo_root(repository_root.clone());

        // Aggregate setup steps from lifecycle providers in order:
        // config → local_db → hooks (when requested).
        let providers = lifecycle_providers(self.request.install_hooks);
        let mut sections = Vec::new();

        for provider in &providers {
            let outcome = provider
                .setup(&ctx)
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?;

            if let Some(ref hooks_outcome) = outcome.required_hooks_install {
                sections.push(setup::format_required_hook_install_success_message(
                    &setup_required_hooks_outcome_from_lifecycle(hooks_outcome),
                ));
            }
        }

        // Handle config target installation (OpenCode/Claude assets).
        if let Some(resolved_mode) = setup_dispatch {
            let setup_message = setup::run_setup_for_mode(&repository_root, resolved_mode)
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?;
            sections.push(setup_message);
        }

        Ok(sections.join("\n\n"))
    }
}

fn setup_required_hooks_outcome_from_lifecycle(
    outcome: &RequiredHooksInstallOutcome,
) -> setup::RequiredHooksInstallOutcome {
    setup::RequiredHooksInstallOutcome {
        repository_root: outcome.repository_root.clone(),
        hooks_directory: outcome.hooks_directory.clone(),
        hook_results: outcome
            .hook_results
            .iter()
            .map(|result| setup::RequiredHookInstallResult {
                hook_name: result.hook_name.clone(),
                hook_path: result.hook_path.clone(),
                status: match result.status {
                    RequiredHookInstallStatus::Installed => {
                        setup::RequiredHookInstallStatus::Installed
                    }
                    RequiredHookInstallStatus::Updated => setup::RequiredHookInstallStatus::Updated,
                    RequiredHookInstallStatus::Skipped => setup::RequiredHookInstallStatus::Skipped,
                },
            })
            .collect(),
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
