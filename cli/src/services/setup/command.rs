use std::future::Future;

use anyhow::Context;

use crate::app::ContextWithRepoRoot;
use crate::services::error::{CliError, UserError};
use crate::services::lifecycle::{
    lifecycle_providers, RequiredHookInstallStatus, RequiredHooksInstallOutcome,
};
use crate::services::managed_telemetry::claude_helper::repository_id_for;
use crate::services::managed_telemetry::{run_with_deadline, CREDENTIAL_LOOKUP_DEADLINE};
use crate::services::telemetry_credential_storage::{
    bind_repository_telemetry, RepositoryBindingMode,
};
use crate::services::token_storage::TokenStorageError;
use crate::services::{config, setup};

pub struct SetupCommand {
    pub request: setup::SetupRequest,
}

impl SetupCommand {
    pub async fn execute<C: ContextWithRepoRoot>(&self, context: &C) -> Result<String, CliError> {
        let setup_start_path = match &self.request.hooks_repo_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .context("Failed to determine current directory")
                .map_err(unexpected_failure)?,
        };

        let repository_root = resolve_setup_repository(&setup_start_path)?;

        let setup_dispatch = if self.request.context_only {
            None
        } else if let Some(mode) = self.request.config_mode {
            let optional_workflow_defaults = match &self.request.optional_workflows {
                Some(selection) => selection.clone(),
                None => setup::persisted_optional_workflows(&repository_root),
            };

            match setup::resolve_setup_dispatch(
                mode,
                &setup::InquireSetupTargetPrompter,
                &optional_workflow_defaults,
                &setup::persisted_integration_targets(&repository_root),
                setup::persisted_setup_confirmations(&repository_root),
            )
            .map_err(unexpected_failure)?
            {
                setup::SetupDispatch::Cancelled => {
                    return Ok(setup::setup_cancelled_text());
                }
                dispatch @ setup::SetupDispatch::Proceed { .. } => Some(dispatch),
            }
        } else {
            None
        };

        let claude_telemetry_selected = dispatch_targets_claude(setup_dispatch.as_ref());

        let mut sections = Vec::new();

        let context_message =
            setup::bootstrap_context_baseline(&repository_root).map_err(unexpected_failure)?;
        sections.push(context_message);

        if self.request.context_only {
            return Ok(sections.join("\n\n"));
        }

        let ctx = context.with_repo_root(repository_root.clone());

        let providers = lifecycle_providers(self.request.install_hooks);

        for provider in &providers {
            let outcome = provider.setup(&ctx).await.map_err(unexpected_failure)?;

            sections.extend(outcome.messages);

            if let Some(ref hooks_outcome) = outcome.required_hooks_install {
                sections.push(setup::format_required_hook_install_success_message(
                    &setup_required_hooks_outcome_from_lifecycle(hooks_outcome),
                ));
            }
        }

        if let Some(setup::SetupDispatch::Proceed {
            targets,
            target_persistence,
            optional_workflows: prompted_optional_workflows,
            agent_trace_auto_sync,
            attribution_hooks_enabled,
        }) = setup_dispatch
        {
            let optional_workflows = prompted_optional_workflows
                .as_deref()
                .or(self.request.optional_workflows.as_deref());

            let setup_message = setup::run_setup_for_targets(
                &repository_root,
                &targets,
                target_persistence,
                optional_workflows,
                agent_trace_auto_sync,
                attribution_hooks_enabled,
            )
            .map_err(unexpected_failure)?;
            sections.push(setup_message);

            if claude_telemetry_selected {
                sections.push(claude_telemetry_report(&repository_root).await);
            }
        }

        Ok(sections.join("\n\n"))
    }
}

fn dispatch_targets_claude(dispatch: Option<&setup::SetupDispatch>) -> bool {
    matches!(
        dispatch,
        Some(setup::SetupDispatch::Proceed { targets, .. })
            if targets.contains(&setup::ConcreteSetupTarget::Claude)
    )
}

async fn claude_telemetry_report(repository_root: &std::path::Path) -> String {
    let repository_id = repository_id_for(repository_root);
    render_claude_telemetry_section(
        setup::claude_telemetry::resolve_claude_settings_path().as_deref(),
        &setup::claude_telemetry::managed_claude_settings_path(),
        async move {
            match repository_id {
                Some(repository_id) => {
                    bind_repository_telemetry(repository_id, RepositoryBindingMode::Enroll).await
                }
                None => Ok(false),
            }
        },
    )
    .await
}

async fn render_claude_telemetry_section<Fut>(
    settings_path: Option<&std::path::Path>,
    managed_settings_path: &std::path::Path,
    repository_enrollment: Fut,
) -> String
where
    Fut: Future<Output = Result<bool, TokenStorageError>> + Send + 'static,
{
    use setup::claude_telemetry::{
        format_claude_telemetry_report, install_claude_telemetry, ClaudeTelemetryInstallOutcome,
    };

    let installed = match settings_path {
        Some(settings_path) => install_claude_telemetry(settings_path, managed_settings_path)
            .unwrap_or(ClaudeTelemetryInstallOutcome::SkippedInvalid),
        None => ClaudeTelemetryInstallOutcome::SkippedInvalid,
    };
    let authentication_pending = installed == ClaudeTelemetryInstallOutcome::Configured
        && !run_with_deadline(CREDENTIAL_LOOKUP_DEADLINE, repository_enrollment)
            .await
            .and_then(Result::ok)
            .unwrap_or(false);

    format_claude_telemetry_report(&installed, authentication_pending)
}

fn resolve_setup_repository(start_path: &std::path::Path) -> Result<std::path::PathBuf, CliError> {
    let repository_root = setup::ensure_git_repository(start_path).map_err(|source| {
        let user_error = match source {
            setup::GitRepositoryResolutionError::NotGitRepository(_) => UserError::NotGitRepository,
            setup::GitRepositoryResolutionError::Unexpected(_) => UserError::UnexpectedFailure,
        };

        CliError::user_with_source(user_error, source)
    })?;

    let storage_config = config::resolve_agent_trace_storage_runtime_config(&repository_root)
        .map_err(unexpected_failure)?;

    setup::ensure_git_remote(&repository_root, &storage_config.repository_remote).map_err(
        |source| {
            if setup::is_missing_git_remote_error(&source) {
                CliError::user_with_source(UserError::NotGitRemote, source)
            } else {
                unexpected_failure(source)
            }
        },
    )?;

    Ok(repository_root)
}

fn unexpected_failure(source: impl Into<anyhow::Error>) -> CliError {
    CliError::user_with_source(UserError::UnexpectedFailure, source)
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use anyhow::Result;
    use serde_json::Value;

    use super::*;
    use crate::services::setup::{
        resolve_setup_dispatch, resolve_setup_request, ConcreteSetupTarget,
        PersistedSetupConfirmations, SetupCliOptions, SetupDispatch, SetupMode, SetupTarget,
        SetupTargetPrompter,
    };

    struct FakePrompter {
        targets: Option<Vec<ConcreteSetupTarget>>,
    }

    impl SetupTargetPrompter for FakePrompter {
        fn prompt_targets(
            &self,
            _configured: &[ConcreteSetupTarget],
        ) -> Result<Option<Vec<ConcreteSetupTarget>>> {
            Ok(self.targets.clone())
        }

        fn prompt_optional_workflows(&self, _defaults: &[String]) -> Result<Option<Vec<String>>> {
            Ok(Some(Vec::new()))
        }

        fn prompt_agent_trace_auto_sync(&self) -> Result<Option<bool>> {
            Ok(Some(false))
        }

        fn prompt_attribution_hooks_enabled(&self) -> Result<Option<bool>> {
            Ok(Some(false))
        }
    }

    fn dispatch_for(mode: SetupMode, prompted: Option<Vec<ConcreteSetupTarget>>) -> SetupDispatch {
        resolve_setup_dispatch(
            mode,
            &FakePrompter { targets: prompted },
            &[],
            &[],
            PersistedSetupConfirmations::default(),
        )
        .expect("dispatch should resolve")
    }

    fn run_claude_telemetry_stage(dispatch: Option<&SetupDispatch>, root: &Path) -> Option<String> {
        dispatch_targets_claude(dispatch).then(|| {
            block_on_current_thread(render_claude_telemetry_section(
                Some(&root.join("settings.json")),
                &root.join("managed.json"),
                std::future::ready(Ok(false)),
            ))
        })
    }

    fn block_on_current_thread<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[tokio::test]
    async fn setup_claude_telemetry_skips_repository_enrollment_when_install_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        fs::write(&settings, r#"{"otelHeadersHelper":"my-helper"}"#).unwrap();
        let looked_up = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = looked_up.clone();

        let report = render_claude_telemetry_section(
            Some(&settings),
            &dir.path().join("managed.json"),
            async move {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(false)
            },
        )
        .await;

        assert!(report.contains("skipped"));
        assert!(!looked_up.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn setup_claude_telemetry_reports_pending_when_repository_enrollment_stalls() {
        let dir = tempfile::tempdir().unwrap();
        let started = std::time::Instant::now();

        let report = render_claude_telemetry_section(
            Some(&dir.path().join("settings.json")),
            &dir.path().join("managed.json"),
            std::future::pending(),
        )
        .await;

        assert!(report.contains("authentication pending"));
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[tokio::test]
    async fn setup_claude_telemetry_reports_ready_only_when_repository_is_enrolled() {
        for (enrolled, pending) in [(Ok(true), false), (Ok(false), true), (Err(storage()), true)] {
            let dir = tempfile::tempdir().unwrap();
            let report = render_claude_telemetry_section(
                Some(&dir.path().join("settings.json")),
                &dir.path().join("managed.json"),
                std::future::ready(enrolled),
            )
            .await;
            assert_eq!(
                report.contains("authentication pending"),
                pending,
                "{report}"
            );
        }
    }

    fn storage() -> TokenStorageError {
        TokenStorageError::Database("unavailable".to_string())
    }

    fn assert_installed(root: &Path) {
        let parsed: Value =
            serde_json::from_slice(&fs::read(root.join("settings.json")).unwrap()).unwrap();
        assert_eq!(parsed["otelHeadersHelper"], "sce auth telemetry-headers");
        assert!(parsed["env"].is_object());
    }

    fn assert_untouched(root: &Path) {
        assert!(!root.join("settings.json").exists());
    }

    #[test]
    fn setup_claude_telemetry_installs_when_claude_is_selected() {
        let interactive = dispatch_for(
            SetupMode::Interactive,
            Some(vec![ConcreteSetupTarget::Claude]),
        );
        for dispatch in [
            dispatch_for(SetupMode::NonInteractive(SetupTarget::Claude), None),
            dispatch_for(SetupMode::NonInteractive(SetupTarget::All), None),
            interactive,
        ] {
            let dir = tempfile::tempdir().unwrap();

            assert!(run_claude_telemetry_stage(Some(&dispatch), dir.path()).is_some());
            assert_installed(dir.path());
        }
    }

    #[test]
    fn setup_claude_telemetry_leaves_settings_untouched_when_claude_is_not_installed() {
        let hooks_only = resolve_setup_request(SetupCliOptions {
            hooks: true,
            ..SetupCliOptions::default()
        })
        .unwrap();
        let bootstrap = resolve_setup_request(SetupCliOptions {
            bootstrap_context: true,
            ..SetupCliOptions::default()
        })
        .unwrap();
        assert_eq!(hooks_only.config_mode, None);
        assert!(bootstrap.context_only);

        let dispatches = [
            Some(dispatch_for(
                SetupMode::NonInteractive(SetupTarget::OpenCode),
                None,
            )),
            Some(dispatch_for(SetupMode::Interactive, None)),
            None,
        ];
        for dispatch in &dispatches {
            let dir = tempfile::tempdir().unwrap();

            assert!(run_claude_telemetry_stage(dispatch.as_ref(), dir.path()).is_none());
            assert_untouched(dir.path());
        }
    }
}
