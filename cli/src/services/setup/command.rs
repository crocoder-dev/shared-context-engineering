use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::app::ContextWithRepoRoot;
use crate::services::config::{AuthMode, ResolvedAccessPolicy, ValueSource};
use crate::services::error::{AuthPolicyConfigFile, CliError, UserError};
use crate::services::lifecycle::{
    lifecycle_providers, RequiredHookInstallStatus, RequiredHooksInstallOutcome,
};
use crate::services::setup::access_preferences::{
    persist_global_access_preferences, read_global_access_preferences,
    GlobalAccessPreferencesError, GlobalWriteOutcome,
};
use crate::services::{config, managed_telemetry, setup};

const CLAUDE_TELEMETRY_DISABLED_BY_CONFIG_LINE: &str =
    "Claude telemetry: skipped; SCE telemetry is disabled by config.";
const SCE_ACCESS_HEADING: &str = "SCE access:";
const SAVED_ACCESS_SCOPE: &str = "applies to all repositories on this machine";
const AUTH_LOGOUT_HINT_LINE: &str =
    "- SCE login: stored SCE credentials remain on this machine; run 'sce auth logout' to remove them.";
const GLOBAL_ACCESS_WRITE_REMEDIATION: &str = "No repository files were changed. Run 'sce config validate' to check the file, then rerun 'sce setup'.";

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

        let (setup_dispatch, interactive_access) = match self.prepare_dispatch(&repository_root)? {
            PreparedSetup::Cancelled => return Ok(setup::setup_cancelled_text()),
            PreparedSetup::Ready {
                dispatch,
                interactive_access,
            } => (dispatch, interactive_access),
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
            ..
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

            if let Some((access_save, auth_db_present)) = &interactive_access {
                sections.push(setup_access_report(
                    &repository_root,
                    access_save,
                    resolve_access_policy,
                    *auth_db_present,
                )?);
            }

            if claude_telemetry_selected {
                sections.push(claude_telemetry_report(&repository_root));
            }
        }

        Ok(sections.join("\n\n"))
    }

    fn prepare_dispatch(&self, repository_root: &Path) -> Result<PreparedSetup, CliError> {
        if self.request.context_only {
            return Ok(PreparedSetup::Ready {
                dispatch: None,
                interactive_access: None,
            });
        }
        let Some(mode) = self.request.config_mode else {
            return Ok(PreparedSetup::Ready {
                dispatch: None,
                interactive_access: None,
            });
        };

        let optional_workflow_defaults = match &self.request.optional_workflows {
            Some(selection) => selection.clone(),
            None => setup::persisted_optional_workflows(repository_root),
        };

        if mode != setup::SetupMode::Interactive {
            let dispatch = setup::resolve_setup_dispatch(
                mode,
                &setup::InquireSetupTargetPrompter,
                &optional_workflow_defaults,
                &setup::persisted_integration_targets(repository_root),
                setup::persisted_setup_confirmations(repository_root),
                setup::SetupAccessGate::default(),
            )
            .map_err(unexpected_failure)?;

            return Ok(match dispatch {
                setup::SetupDispatch::Cancelled => PreparedSetup::Cancelled,
                dispatch @ setup::SetupDispatch::Proceed { .. } => PreparedSetup::Ready {
                    dispatch: Some(dispatch),
                    interactive_access: None,
                },
            });
        }

        let global_config_path =
            config::resolver::resolve_default_global_config_path().map_err(unexpected_failure)?;
        let auth_db_present = managed_telemetry::auth_db_present();

        Ok(
            match prepare_interactive_setup(
                repository_root,
                &setup::InquireSetupTargetPrompter,
                &optional_workflow_defaults,
                &global_config_path,
                resolve_access_policy,
                auth_db_present,
            )? {
                InteractiveSetup::Cancelled => PreparedSetup::Cancelled,
                InteractiveSetup::Ready {
                    dispatch,
                    access_save,
                } => PreparedSetup::Ready {
                    dispatch: Some(dispatch),
                    interactive_access: Some((access_save, auth_db_present)),
                },
            },
        )
    }
}

enum PreparedSetup {
    Cancelled,
    Ready {
        dispatch: Option<setup::SetupDispatch>,
        interactive_access: Option<(SetupAccessSave, bool)>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SetupAccessSave {
    global_config_path: PathBuf,
    auth_mode: Option<AuthMode>,
    telemetry_enabled: Option<bool>,
    outcome: GlobalWriteOutcome,
}

#[derive(Debug)]
enum InteractiveSetup {
    Cancelled,
    Ready {
        dispatch: setup::SetupDispatch,
        access_save: SetupAccessSave,
    },
}

fn resolve_access_policy(repository_root: &Path) -> anyhow::Result<ResolvedAccessPolicy> {
    config::resolve_auth_runtime_config(repository_root).map(|config| config.access_policy)
}

fn prepare_interactive_setup<P, R>(
    repository_root: &Path,
    prompter: &P,
    optional_workflow_defaults: &[String],
    global_config_path: &Path,
    resolve_policy: R,
    auth_db_present: bool,
) -> Result<InteractiveSetup, CliError>
where
    P: setup::SetupTargetPrompter,
    R: Fn(&Path) -> anyhow::Result<ResolvedAccessPolicy>,
{
    let policy = resolve_policy(repository_root).map_err(unexpected_failure)?;
    let global_preferences =
        read_global_access_preferences(global_config_path).map_err(unexpected_failure)?;
    let persisted_confirmations = setup::persisted_setup_confirmations(repository_root)
        .with_global_access_preferences(global_preferences);

    let dispatch = setup::resolve_setup_dispatch(
        setup::SetupMode::Interactive,
        prompter,
        optional_workflow_defaults,
        &setup::persisted_integration_targets(repository_root),
        persisted_confirmations,
        setup::SetupAccessGate::from_policy(&policy, auth_db_present),
    )
    .map_err(unexpected_failure)?;

    let (auth_mode, telemetry_enabled) = match &dispatch {
        setup::SetupDispatch::Cancelled => return Ok(InteractiveSetup::Cancelled),
        setup::SetupDispatch::Proceed {
            auth_mode,
            telemetry_enabled,
            ..
        } => (*auth_mode, *telemetry_enabled),
    };

    let access_save =
        persist_setup_access_preferences(global_config_path, auth_mode, telemetry_enabled)?;

    Ok(InteractiveSetup::Ready {
        dispatch,
        access_save,
    })
}

fn persist_setup_access_preferences(
    global_config_path: &Path,
    auth_mode: Option<AuthMode>,
    telemetry_enabled: Option<bool>,
) -> Result<SetupAccessSave, CliError> {
    let outcome =
        persist_global_access_preferences(global_config_path, auth_mode, telemetry_enabled)
            .map_err(|source| global_access_write_failure(global_config_path, &source))?;

    Ok(SetupAccessSave {
        global_config_path: global_config_path.to_path_buf(),
        auth_mode,
        telemetry_enabled,
        outcome,
    })
}

fn global_access_write_failure(
    global_config_path: &Path,
    source: &GlobalAccessPreferencesError,
) -> CliError {
    CliError::runtime(anyhow::anyhow!(
        "Failed to save SCE access preferences to the global SCE config '{}'.\n{source}\n{GLOBAL_ACCESS_WRITE_REMEDIATION}",
        global_config_path.display()
    ))
}

fn setup_access_report<R>(
    repository_root: &Path,
    access_save: &SetupAccessSave,
    resolve_policy: R,
    auth_db_present: bool,
) -> Result<String, CliError>
where
    R: Fn(&Path) -> anyhow::Result<ResolvedAccessPolicy>,
{
    let policy = resolve_policy(repository_root).map_err(unexpected_failure)?;

    Ok(render_setup_access_section(
        access_save,
        &policy,
        auth_db_present,
    ))
}

fn render_setup_access_section(
    access_save: &SetupAccessSave,
    policy: &ResolvedAccessPolicy,
    auth_db_present: bool,
) -> String {
    let global_config_path = access_save.global_config_path.display();
    let mut lines = vec![SCE_ACCESS_HEADING.to_string()];

    let saved_values = [
        access_save
            .auth_mode
            .map(|mode| ("auth.mode", mode.as_str().to_string())),
        access_save
            .telemetry_enabled
            .map(|enabled| ("telemetry.enabled", enabled.to_string())),
    ];
    if access_save.outcome == GlobalWriteOutcome::Written {
        lines.extend(saved_values.into_iter().flatten().map(|(key, saved)| {
            format!(
                "- {key}: {saved} (saved to global SCE config '{global_config_path}'; {SAVED_ACCESS_SCOPE})"
            )
        }));
    } else {
        lines.push(format!(
            "- saved access preferences: unchanged (global SCE config '{global_config_path}')"
        ));
    }

    lines.push(format!(
        "- effective auth.mode: {} ({})",
        policy.auth_mode.value.as_str(),
        format_policy_source(policy.auth_mode.source)
    ));
    lines.push(match policy.telemetry.disabled_reason() {
        Some(reason) => format!(
            "- effective telemetry: disabled (reason: {})",
            reason.as_str()
        ),
        None => "- effective telemetry: enabled".to_string(),
    });

    if let Some(file) = setup::SetupAccessGate::from_policy(policy, auth_db_present).auth_locked_by
    {
        lines.push(format!("- {}", auth_locked_by_line(file)));
    }
    if !policy.auth_allowed() && auth_db_present {
        lines.push(AUTH_LOGOUT_HINT_LINE.to_string());
    }

    lines.join("\n")
}

fn format_policy_source(source: ValueSource) -> String {
    match source.config_source() {
        Some(config_source) => format!(
            "source: {}, config_source: {}",
            source.as_str(),
            config_source.as_str()
        ),
        None => format!("source: {}", source.as_str()),
    }
}

fn auth_locked_by_line(file: AuthPolicyConfigFile) -> &'static str {
    let message = UserError::AuthDisabledByConfig(file).message();
    message.lines().next().unwrap_or(message)
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
    use crate::services::config::AuthMode;
    use crate::services::setup::{
        resolve_setup_dispatch, resolve_setup_request, ConcreteSetupTarget,
        PersistedSetupConfirmations, SetupAccessGate, SetupCliOptions, SetupDispatch, SetupMode,
        SetupTarget, SetupTargetPrompter,
    };

    struct FakePrompter {
        targets: Option<Vec<ConcreteSetupTarget>>,
        auth_mode: Option<AuthMode>,
        telemetry_consent: bool,
    }

    impl FakePrompter {
        fn with_targets(targets: Option<Vec<ConcreteSetupTarget>>) -> Self {
            Self {
                targets,
                auth_mode: None,
                telemetry_consent: false,
            }
        }

        fn answering(auth_mode: AuthMode, telemetry_consent: bool) -> Self {
            Self {
                targets: Some(vec![ConcreteSetupTarget::OpenCode]),
                auth_mode: Some(auth_mode),
                telemetry_consent,
            }
        }
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

        fn prompt_agent_trace_auto_sync(&self, _default: bool) -> Result<Option<bool>> {
            Ok(Some(false))
        }

        fn prompt_attribution_hooks_enabled(&self, _default: bool) -> Result<Option<bool>> {
            Ok(Some(false))
        }

        fn prompt_auth_mode(&self, default: AuthMode) -> Result<Option<AuthMode>> {
            Ok(Some(self.auth_mode.unwrap_or(default)))
        }

        fn prompt_telemetry_consent(&self, _default: bool) -> Result<Option<bool>> {
            Ok(Some(self.telemetry_consent))
        }

        fn prompt_login(&self) -> Result<Option<bool>> {
            Ok(Some(false))
        }
    }

    fn dispatch_for(mode: SetupMode, prompted: Option<Vec<ConcreteSetupTarget>>) -> SetupDispatch {
        resolve_setup_dispatch(
            mode,
            &FakePrompter::with_targets(prompted),
            &[],
            &[],
            PersistedSetupConfirmations::default(),
            SetupAccessGate::default(),
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

    struct AccessFixture {
        _dir: tempfile::TempDir,
        repo: std::path::PathBuf,
        global: std::path::PathBuf,
    }

    impl AccessFixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let repo = dir.path().join("repo");
            fs::create_dir_all(repo.join(".git")).unwrap();
            let global = dir.path().join("global").join("config.json");
            fs::create_dir_all(global.parent().unwrap()).unwrap();
            Self {
                _dir: dir,
                repo,
                global,
            }
        }

        fn write_repo_config(&self, json: &str) {
            fs::create_dir_all(self.repo.join(".sce")).unwrap();
            fs::write(self.repo.join(".sce").join("config.json"), json).unwrap();
        }

        fn resolve_policy(&self) -> impl Fn(&Path) -> Result<ResolvedAccessPolicy> + '_ {
            move |root| {
                config::resolver::resolve_auth_runtime_config_with(
                    root,
                    |_| None,
                    |path| fs::read_to_string(path).map_err(anyhow::Error::from),
                    Path::exists,
                    || Ok(self.global.clone()),
                )
                .map(|config| config.access_policy)
            }
        }

        fn prepare(&self, prompter: &FakePrompter) -> Result<InteractiveSetup, CliError> {
            prepare_interactive_setup(
                &self.repo,
                prompter,
                &[],
                &self.global,
                self.resolve_policy(),
                false,
            )
        }

        fn report(&self, access_save: &SetupAccessSave, auth_db_present: bool) -> String {
            setup_access_report(
                &self.repo,
                access_save,
                self.resolve_policy(),
                auth_db_present,
            )
            .unwrap()
        }
    }

    fn ready(setup: Result<InteractiveSetup, CliError>) -> (SetupDispatch, SetupAccessSave) {
        match setup.expect("interactive setup should prepare") {
            InteractiveSetup::Ready {
                dispatch,
                access_save,
            } => (dispatch, access_save),
            InteractiveSetup::Cancelled => panic!("interactive setup should not cancel"),
        }
    }

    fn read_json_file(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn interactive_setup_saves_access_answers_only_to_the_global_config() {
        let fixture = AccessFixture::new();
        fs::write(&fixture.global, "{\"log_level\": \"debug\"}\n").unwrap();

        let (dispatch, access_save) =
            ready(fixture.prepare(&FakePrompter::answering(AuthMode::Mandatory, true)));

        let global = read_json_file(&fixture.global);
        assert_eq!(global["auth"]["mode"], "mandatory");
        assert_eq!(global["telemetry"]["enabled"], true);
        assert_eq!(global["log_level"], "debug");
        assert_eq!(access_save.outcome, GlobalWriteOutcome::Written);

        let SetupDispatch::Proceed {
            targets,
            target_persistence,
            agent_trace_auto_sync,
            attribution_hooks_enabled,
            ..
        } = dispatch
        else {
            panic!("dispatch should proceed");
        };
        setup::persist_integration_targets(
            &fixture.repo,
            &targets,
            target_persistence,
            &[],
            agent_trace_auto_sync,
            attribution_hooks_enabled,
        )
        .unwrap();

        let repository = read_json_file(&fixture.repo.join(".sce").join("config.json"));
        let repository = repository.as_object().unwrap();
        assert!(!repository.contains_key("auth"));
        assert!(!repository.contains_key("telemetry"));
    }

    #[test]
    fn interactive_setup_rerun_leaves_the_global_config_bytes_unchanged() {
        let fixture = AccessFixture::new();
        let prompter = FakePrompter::answering(AuthMode::Optional, false);

        ready(fixture.prepare(&prompter));
        let first = fs::read(&fixture.global).unwrap();

        let (_, rerun) = ready(fixture.prepare(&prompter));
        assert_eq!(
            rerun,
            SetupAccessSave {
                global_config_path: fixture.global.clone(),
                auth_mode: None,
                telemetry_enabled: None,
                outcome: GlobalWriteOutcome::Unchanged,
            }
        );
        assert_eq!(fs::read(&fixture.global).unwrap(), first);

        let repeated = persist_setup_access_preferences(
            &fixture.global,
            Some(AuthMode::Optional),
            Some(false),
        )
        .unwrap();
        assert_eq!(repeated.outcome, GlobalWriteOutcome::Unchanged);
        assert_eq!(fs::read(&fixture.global).unwrap(), first);
    }

    #[test]
    fn interactive_setup_skips_access_writes_when_the_global_config_is_invalid() {
        let fixture = AccessFixture::new();
        let invalid = b"{\"auth\": {\"mode\": \"sometimes\"}}\n";
        fs::write(&fixture.global, invalid).unwrap();

        let (dispatch, access_save) =
            ready(fixture.prepare(&FakePrompter::answering(AuthMode::Mandatory, true)));

        assert!(matches!(
            dispatch,
            SetupDispatch::Proceed {
                auth_mode: None,
                telemetry_enabled: None,
                ..
            }
        ));
        assert_eq!(access_save.outcome, GlobalWriteOutcome::Unchanged);
        assert_eq!(fs::read(&fixture.global).unwrap(), invalid);
    }

    #[cfg(unix)]
    #[test]
    fn interactive_setup_aborts_before_repository_writes_when_the_global_config_is_read_only() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = AccessFixture::new();
        let global_dir = fixture.global.parent().unwrap().to_path_buf();
        fs::set_permissions(&global_dir, fs::Permissions::from_mode(0o555)).unwrap();

        let result = fixture.prepare(&FakePrompter::answering(AuthMode::Optional, true));
        fs::set_permissions(&global_dir, fs::Permissions::from_mode(0o755)).unwrap();

        let error = result.expect_err("read-only global config should abort setup");
        let message = error.to_string();
        assert!(message.starts_with(&format!(
            "Failed to save SCE access preferences to the global SCE config '{}'.\n",
            fixture.global.display()
        )));
        assert!(message.ends_with(GLOBAL_ACCESS_WRITE_REMEDIATION));
        assert_eq!(error.code(), "SCE-ERR-RUNTIME");
        assert!(!fixture.global.exists());
        assert!(!fixture.repo.join("context").exists());
        assert!(!fixture.repo.join(".sce").exists());
    }

    #[test]
    fn setup_access_write_refuses_an_invalid_global_config_with_a_fixed_message() {
        let fixture = AccessFixture::new();
        fs::write(&fixture.global, "{not json").unwrap();

        let error = persist_setup_access_preferences(&fixture.global, None, Some(true))
            .expect_err("invalid global config should not be overwritten");

        let message = error.to_string();
        assert!(message.starts_with(&format!(
            "Failed to save SCE access preferences to the global SCE config '{}'.\nGlobal SCE config '{}' is invalid and was left unchanged: ",
            fixture.global.display(),
            fixture.global.display()
        )));
        assert!(message.ends_with(&format!("\n{GLOBAL_ACCESS_WRITE_REMEDIATION}")));
        assert_eq!(fs::read(&fixture.global).unwrap(), b"{not json");
    }

    #[test]
    fn setup_access_section_reports_repository_opt_out_after_global_consent() {
        let fixture = AccessFixture::new();
        fixture.write_repo_config(r#"{"telemetry":{"enabled":false}}"#);

        let (_, access_save) =
            ready(fixture.prepare(&FakePrompter::answering(AuthMode::Optional, true)));

        assert_eq!(
            fixture.report(&access_save, false),
            format!(
                "SCE access:\n\
                 - auth.mode: optional (saved to global SCE config '{path}'; applies to all repositories on this machine)\n\
                 - telemetry.enabled: true (saved to global SCE config '{path}'; applies to all repositories on this machine)\n\
                 - effective auth.mode: optional (source: config_file, config_source: default_discovered_global)\n\
                 - effective telemetry: disabled (reason: repository_opt_out)",
                path = fixture.global.display()
            )
        );
    }

    #[test]
    fn setup_access_section_renders_exact_lines() {
        let unchanged = |fixture: &AccessFixture| SetupAccessSave {
            global_config_path: fixture.global.clone(),
            auth_mode: None,
            telemetry_enabled: None,
            outcome: GlobalWriteOutcome::Unchanged,
        };

        let fixture = AccessFixture::new();
        fs::write(
            &fixture.global,
            r#"{"auth":{"mode":"optional"},"telemetry":{"enabled":true}}"#,
        )
        .unwrap();
        assert_eq!(
            fixture.report(&unchanged(&fixture), true),
            format!(
                "SCE access:\n\
                 - saved access preferences: unchanged (global SCE config '{}')\n\
                 - effective auth.mode: optional (source: config_file, config_source: default_discovered_global)\n\
                 - effective telemetry: enabled",
                fixture.global.display()
            )
        );

        let fixture = AccessFixture::new();
        let access_save =
            persist_setup_access_preferences(&fixture.global, Some(AuthMode::Disabled), None)
                .unwrap();
        assert_eq!(
            fixture.report(&access_save, true),
            format!(
                "SCE access:\n\
                 - auth.mode: disabled (saved to global SCE config '{}'; applies to all repositories on this machine)\n\
                 - effective auth.mode: disabled (source: config_file, config_source: default_discovered_global)\n\
                 - effective telemetry: disabled (reason: auth_disabled)\n\
                 {AUTH_LOGOUT_HINT_LINE}",
                fixture.global.display()
            )
        );
        assert!(!fixture
            .report(&access_save, false)
            .contains("sce auth logout"));

        let fixture = AccessFixture::new();
        fixture.write_repo_config(r#"{"auth":{"mode":"disabled"}}"#);
        assert_eq!(
            fixture.report(&unchanged(&fixture), false),
            format!(
                "SCE access:\n\
                 - saved access preferences: unchanged (global SCE config '{}')\n\
                 - effective auth.mode: disabled (source: config_file, config_source: default_discovered_local)\n\
                 - effective telemetry: disabled (reason: auth_disabled)\n\
                 - Authentication is disabled by `auth.mode: disabled` in the repository SCE config (`.sce/config.json`).",
                fixture.global.display()
            )
        );
    }
}
