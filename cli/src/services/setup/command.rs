use anyhow::Context;

use crate::app::ContextWithRepoRoot;
use crate::services::error::{CliError, UserError};
use crate::services::lifecycle::{
    lifecycle_providers, RequiredHookInstallStatus, RequiredHooksInstallOutcome,
};
use crate::services::{config, setup};

const CLAUDE_TELEMETRY_DISABLED_BY_CONFIG_LINE: &str =
    "Claude telemetry: skipped; SCE telemetry is disabled by config.";

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
                sections.push(claude_telemetry_report(&repository_root));
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

fn claude_telemetry_report(repository_root: &std::path::Path) -> String {
    let telemetry_enabled = config::resolve_auth_runtime_config(repository_root)
        .is_ok_and(|config| config.access_policy.telemetry_enabled());

    render_claude_telemetry_section(
        telemetry_enabled,
        setup::claude_telemetry::resolve_claude_settings_path().as_deref(),
        &setup::claude_telemetry::managed_claude_settings_path(),
    )
}

fn render_claude_telemetry_section(
    telemetry_enabled: bool,
    settings_path: Option<&std::path::Path>,
    managed_settings_path: &std::path::Path,
) -> String {
    use setup::claude_telemetry::{
        format_claude_telemetry_report, install_claude_telemetry, ClaudeTelemetryInstallOutcome,
    };

    if !telemetry_enabled {
        return CLAUDE_TELEMETRY_DISABLED_BY_CONFIG_LINE.to_string();
    }

    let installed = match settings_path {
        Some(settings_path) => install_claude_telemetry(settings_path, managed_settings_path)
            .unwrap_or(ClaudeTelemetryInstallOutcome::SkippedInvalid),
        None => ClaudeTelemetryInstallOutcome::SkippedInvalid,
    };
    let authentication_pending = installed == ClaudeTelemetryInstallOutcome::Configured;

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

    fn run_claude_telemetry_stage(
        dispatch: Option<&SetupDispatch>,
        telemetry_enabled: bool,
        root: &Path,
    ) -> Option<String> {
        dispatch_targets_claude(dispatch).then(|| {
            render_claude_telemetry_section(
                telemetry_enabled,
                Some(&root.join("settings.json")),
                &root.join("managed.json"),
            )
        })
    }

    #[test]
    fn setup_claude_telemetry_reports_skip_without_repository_enrollment() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        fs::write(&settings, r#"{"otelHeadersHelper":"my-helper"}"#).unwrap();

        let report = render_claude_telemetry_section(
            true,
            Some(&settings),
            &dir.path().join("managed.json"),
        );

        assert!(report.contains("skipped"));
        assert!(!report.contains("authentication pending"));
    }

    #[test]
    fn setup_claude_telemetry_defers_repository_enrollment_to_auth_login() {
        let dir = tempfile::tempdir().unwrap();

        let report = render_claude_telemetry_section(
            true,
            Some(&dir.path().join("settings.json")),
            &dir.path().join("managed.json"),
        );

        assert!(report.contains("authentication pending"));
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

            assert!(run_claude_telemetry_stage(Some(&dispatch), true, dir.path()).is_some());
            assert_installed(dir.path());
        }
    }

    #[test]
    fn setup_claude_telemetry_skips_install_when_telemetry_is_disabled_by_config() {
        let dispatch = dispatch_for(SetupMode::NonInteractive(SetupTarget::Claude), None);

        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            run_claude_telemetry_stage(Some(&dispatch), false, dir.path()).as_deref(),
            Some(CLAUDE_TELEMETRY_DISABLED_BY_CONFIG_LINE)
        );
        assert_untouched(dir.path());

        let dir = tempfile::tempdir().unwrap();
        let existing = br#"{"env":{"FOO":"bar"},"model":"opus"}"#;
        fs::write(dir.path().join("settings.json"), existing).unwrap();
        assert_eq!(
            run_claude_telemetry_stage(Some(&dispatch), false, dir.path()).as_deref(),
            Some("Claude telemetry: skipped; SCE telemetry is disabled by config.")
        );
        assert_eq!(
            fs::read(dir.path().join("settings.json")).unwrap(),
            existing
        );

        let report = run_claude_telemetry_stage(Some(&dispatch), true, dir.path()).unwrap();
        assert_eq!(
            report,
            "Claude telemetry: configured in user-level Claude settings for native OpenTelemetry export.\nClaude telemetry: authentication pending for this repository; run 'sce auth login' in this repository to start exporting."
        );
        assert_installed(dir.path());
        let parsed: Value =
            serde_json::from_slice(&fs::read(dir.path().join("settings.json")).unwrap()).unwrap();
        assert_eq!(parsed["model"], "opus");
        assert_eq!(parsed["env"]["FOO"], "bar");
    }

    #[test]
    fn setup_claude_telemetry_policy_follows_layered_config() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        let global = dir.path().join("global.json");
        let local_dir = dir.path().join(".sce");
        fs::create_dir_all(&local_dir).unwrap();
        let local = local_dir.join("config.json");
        let enabled = |global_json: &str, local_json: Option<&str>| {
            fs::write(&global, global_json).unwrap();
            match local_json {
                Some(json) => fs::write(&local, json).unwrap(),
                None => {
                    let _ = fs::remove_file(&local);
                }
            }
            config::resolver::resolve_auth_runtime_config_with(
                dir.path(),
                |_| None,
                |path| fs::read_to_string(path).map_err(anyhow::Error::from),
                Path::exists,
                || Ok(global.clone()),
            )
            .is_ok_and(|config| config.access_policy.telemetry_enabled())
        };

        assert!(enabled(r#"{"telemetry":{"enabled":true}}"#, None));
        assert!(!enabled("{}", None));
        assert!(!enabled("{}", Some(r#"{"telemetry":{"enabled":true}}"#)));
        assert!(!enabled(
            r#"{"telemetry":{"enabled":true}}"#,
            Some(r#"{"telemetry":{"enabled":false}}"#)
        ));
        assert!(!enabled(
            r#"{"telemetry":{"enabled":true}}"#,
            Some(r#"{"auth":{"mode":"disabled"}}"#)
        ));
        assert!(!enabled(
            r#"{"telemetry":{"enabled":true}}"#,
            Some(r#"{"telemetry":{"enabled":"yes"}}"#)
        ));
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

            assert!(run_claude_telemetry_stage(dispatch.as_ref(), true, dir.path()).is_none());
            assert_untouched(dir.path());
        }
    }
}
