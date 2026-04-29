use std::borrow::Cow;
use std::sync::Arc;

use anyhow::Context;

use crate::app::AppContext;
use crate::services::command_registry::{RuntimeCommand, RuntimeCommandHandle};
use crate::services::error::ClassifiedError;
use crate::services::lifecycle::ServiceLifecycle;
use crate::services::observability::traits::{NoopLogger, Telemetry};
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

        // Build an AppContext with the resolved repository root for lifecycle providers.
        let ctx = AppContext::new(
            Arc::new(NoopLogger),
            Arc::new(NullTelemetry),
            Arc::new(crate::services::capabilities::StdFsOps),
            Arc::new(crate::services::capabilities::ProcessGitOps),
            Some(repository_root.clone()),
        );

        // Aggregate setup steps from lifecycle providers in order:
        // config → local_db → hooks (when requested).
        let providers = setup_lifecycle_providers(&self.request);
        let mut sections = Vec::new();

        for provider in &providers {
            let outcome = provider
                .setup(&ctx)
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?;

            if let Some(ref hooks_outcome) = outcome.required_hooks_install {
                sections.push(setup::format_required_hook_install_success_message(
                    hooks_outcome,
                ));
            }
        }

        // Handle config target installation (OpenCode/Claude assets).
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

        Ok(sections.join("\n\n"))
    }
}

/// Returns the ordered list of lifecycle providers for setup aggregation.
///
/// Providers are returned in execution order: config → `local_db` → hooks (when requested).
fn setup_lifecycle_providers(request: &setup::SetupRequest) -> Vec<Box<dyn ServiceLifecycle>> {
    let mut providers: Vec<Box<dyn ServiceLifecycle>> = vec![
        Box::new(crate::services::config::lifecycle::ConfigLifecycle),
        Box::new(crate::services::local_db::lifecycle::LocalDbLifecycle),
    ];

    if request.install_hooks {
        providers.push(Box::new(crate::services::hooks::lifecycle::HooksLifecycle));
    }

    providers
}

/// Minimal telemetry used during setup lifecycle aggregation.
struct NullTelemetry;

impl Telemetry for NullTelemetry {
    fn with_default_subscriber(
        &self,
        action: &mut dyn FnMut() -> Result<String, ClassifiedError>,
    ) -> Result<String, ClassifiedError> {
        action()
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
