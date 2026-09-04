use anyhow::Context;

use crate::app::ContextWithRepoRoot;
use crate::services::error::{CliError, UserError};
use crate::services::lifecycle::{
    lifecycle_providers, RequiredHookInstallStatus, RequiredHooksInstallOutcome,
};
use crate::services::{config, setup};

pub struct SetupCommand {
    pub request: setup::SetupRequest,
}

impl SetupCommand {
    pub fn execute<C: ContextWithRepoRoot>(&self, context: &C) -> Result<String, CliError> {
        let setup_start_path = match &self.request.hooks_repo_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .context("Failed to determine current directory")
                .map_err(CliError::runtime)?,
        };

        // The repository root is resolved before any prompt so the interactive
        // optional-workflow prompt can pre-check the persisted selection.
        let repository_root = resolve_setup_repository(&setup_start_path)?;

        let setup_dispatch = if self.request.context_only {
            None
        } else if let Some(mode) = self.request.config_mode {
            // A supplied `--workflow` selection seeds the prompt; otherwise the
            // repository's persisted selection does.
            let optional_workflow_defaults = match &self.request.optional_workflows {
                Some(selection) => selection.clone(),
                None => setup::persisted_optional_workflows(&repository_root),
            };

            match setup::resolve_setup_dispatch(
                mode,
                &setup::InquireSetupTargetPrompter,
                &optional_workflow_defaults,
            )
            .map_err(CliError::runtime)?
            {
                setup::SetupDispatch::Proceed {
                    mode: resolved_mode,
                    optional_workflows,
                } => Some((resolved_mode, optional_workflows)),
                setup::SetupDispatch::Cancelled => {
                    return Ok(setup::setup_cancelled_text());
                }
            }
        } else {
            None
        };

        let mut sections = Vec::new();

        // Every successful setup path ensures the durable-context baseline exists.
        let context_message =
            setup::bootstrap_context_baseline(&repository_root).map_err(CliError::runtime)?;
        sections.push(context_message);

        if self.request.context_only {
            return Ok(sections.join("\n\n"));
        }

        // Scope the runtime AppContext to the resolved repository root for lifecycle providers.
        let ctx = context.with_repo_root(repository_root.clone());

        // Aggregate setup steps from lifecycle providers in order:
        // config → local_db → auth_db → agent_trace_db → hooks (when requested).
        let providers = lifecycle_providers(self.request.install_hooks);

        for provider in &providers {
            let outcome = provider.setup(&ctx).map_err(CliError::runtime)?;

            sections.extend(outcome.messages);

            if let Some(ref hooks_outcome) = outcome.required_hooks_install {
                sections.push(setup::format_required_hook_install_success_message(
                    &setup_required_hooks_outcome_from_lifecycle(hooks_outcome),
                ));
            }
        }

        // Handle config target installation (OpenCode/Claude assets).
        if let Some((resolved_mode, prompted_optional_workflows)) = setup_dispatch {
            // A prompted selection is authoritative for the run; without one the
            // `--workflow` selection (or, absent that, the persisted one) applies.
            let optional_workflows = prompted_optional_workflows
                .as_deref()
                .or(self.request.optional_workflows.as_deref());

            let setup_message =
                setup::run_setup_for_mode(&repository_root, resolved_mode, optional_workflows)
                    .map_err(CliError::runtime)?;
            sections.push(setup_message);
        }

        Ok(sections.join("\n\n"))
    }
}

fn resolve_setup_repository(start_path: &std::path::Path) -> Result<std::path::PathBuf, CliError> {
    let repository_root = setup::ensure_git_repository(start_path).map_err(|source| {
        if setup::is_not_git_repository_error(&source) {
            CliError::user_with_source(UserError::NotGitRepository, source)
        } else {
            CliError::runtime(source)
        }
    })?;
    let storage_config = config::resolve_agent_trace_storage_runtime_config(&repository_root)
        .map_err(CliError::runtime)?;

    setup::ensure_git_remote(&repository_root, &storage_config.repository_remote).map_err(
        |source| {
            if setup::is_missing_git_remote_error(&source) {
                CliError::user_with_source(
                    UserError::NotGitRemote {
                        remote_name: storage_config.repository_remote.clone(),
                    },
                    source,
                )
            } else {
                CliError::runtime(source)
            }
        },
    )?;

    Ok(repository_root)
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
                unreachable_block_advisory: result.unreachable_block_advisory,
            })
            .collect(),
    }
}
